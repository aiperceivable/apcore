// APCore Protocol — Prometheus exporter & K8s health endpoints
// Spec reference: observability.md §1.6 K8s/Prometheus Integration Hooks

use std::fmt::Write as _;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, Semaphore};
use tokio::task::JoinSet;
use tokio::time::{timeout, Duration};

use super::metrics::{
    MetricsCollector, METRIC_CALLS_TOTAL, METRIC_DURATION_SECONDS, METRIC_ERRORS_TOTAL,
};
use super::usage::UsageCollector;

/// Per-connection read deadline. A slow client that drip-feeds bytes is
/// treated as a slow-loris attempt; the request is dropped after this window.
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Maximum simultaneously-open connections served by [`PrometheusExporter`].
/// Bounds spawn fan-out so a hostile client cannot exhaust the runtime.
const MAX_CONCURRENT_CONNECTIONS: usize = 64;

/// HTTP server that serves Prometheus text metrics plus `/healthz` and
/// `/readyz` endpoints.
///
/// Endpoints (observability.md §1.6):
///   GET /metrics  — Prometheus text exposition format
///   GET /healthz  — liveness, always 200 OK
///   GET /readyz   — readiness, 200 OK after `mark_ready()`, else 503
#[derive(Debug, Clone)]
pub struct PrometheusExporter {
    collector: MetricsCollector,
    usage_collector: Option<UsageCollector>,
    ready: Arc<AtomicBool>,
    shutdown: Arc<Notify>,
    bound_addr: Arc<parking_lot::Mutex<Option<SocketAddr>>>,
}

impl PrometheusExporter {
    /// Create a new exporter bound to the given collector.
    #[must_use]
    pub fn new(collector: MetricsCollector) -> Self {
        Self {
            collector,
            usage_collector: None,
            ready: Arc::new(AtomicBool::new(false)),
            shutdown: Arc::new(Notify::new()),
            bound_addr: Arc::new(parking_lot::Mutex::new(None)),
        }
    }

    /// Attach a `UsageCollector` whose `apcore_usage_*` metrics will be
    /// appended to every `/metrics` response (system-modules.md §1.3).
    #[must_use]
    pub fn with_usage_collector(mut self, usage: UsageCollector) -> Self {
        self.usage_collector = Some(usage);
        self
    }

    /// Return the current metrics in Prometheus text format.
    #[must_use]
    pub fn export(&self) -> String {
        let body = self.collector.export_prometheus();
        // Always include the three required apcore metric names so that scrape
        // discovery succeeds on a cold start before the first observation.
        // Spec reference: observability.md §1.6 normative rules.
        let mut out = String::new();
        ensure_metric_present(&mut out, &body, METRIC_CALLS_TOTAL, "counter");
        ensure_metric_present(&mut out, &body, METRIC_ERRORS_TOTAL, "counter");
        ensure_metric_present(&mut out, &body, METRIC_DURATION_SECONDS, "histogram");
        out.push_str(&body);
        // §1.3: append UsageCollector metrics if one is attached.
        if let Some(usage) = &self.usage_collector {
            out.push_str(&usage.export_prometheus());
        }
        out
    }

    /// Mark the application as ready to serve traffic. `/readyz` will return 200.
    pub fn mark_ready(&self) {
        self.ready.store(true, Ordering::SeqCst);
    }

    /// Mark the application as not ready (e.g. during shutdown).
    pub fn mark_not_ready(&self) {
        self.ready.store(false, Ordering::SeqCst);
    }

    /// Local address the server is currently bound to (after `start()` returns).
    #[must_use]
    pub fn local_addr(&self) -> Option<SocketAddr> {
        *self.bound_addr.lock()
    }

    /// Spawn a background HTTP server on the given port, serving metrics at `path`.
    ///
    /// The returned future resolves once the listener is bound. The server runs
    /// until `shutdown()` is called.
    ///
    /// # Errors
    /// Returns an error if the listener fails to bind to the requested port.
    pub async fn start(&self, port: u16, path: &str) -> io::Result<()> {
        let addr: SocketAddr = ([0, 0, 0, 0], port).into();
        let listener = TcpListener::bind(addr).await?;
        let bound = listener.local_addr()?;
        *self.bound_addr.lock() = Some(bound);

        let exporter = self.clone();
        let path = path.to_string();
        let shutdown = self.shutdown.clone();
        let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTIONS));
        tokio::spawn(async move {
            let mut conn_tasks: JoinSet<()> = JoinSet::new();
            loop {
                tokio::select! {
                    accept = listener.accept() => {
                        match accept {
                            Ok((stream, _)) => {
                                // Bounded fan-out: drop the connection when the
                                // semaphore is exhausted rather than queueing.
                                let Ok(permit) = semaphore.clone().try_acquire_owned() else {
                                    tracing::warn!(
                                        "PrometheusExporter: connection refused, max concurrent connections reached"
                                    );
                                    drop(stream);
                                    continue;
                                };
                                let exporter = exporter.clone();
                                let path = path.clone();
                                conn_tasks.spawn(async move {
                                    let _permit = permit;
                                    if let Err(e) = handle_connection(stream, exporter, &path).await {
                                        tracing::debug!(error = %e, "PrometheusExporter: connection error");
                                    }
                                });
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "PrometheusExporter: accept failed");
                            }
                        }
                    }
                    () = shutdown.notified() => {
                        tracing::debug!("PrometheusExporter: shutdown signal received");
                        break;
                    }
                    // Reap finished connection tasks so JoinSet does not grow
                    // unbounded under steady load.
                    Some(_) = conn_tasks.join_next() => {}
                }
            }

            // Graceful drain: give in-flight connection tasks up to the
            // request-read timeout to finish, then abort the remainder.
            let drain = timeout(REQUEST_READ_TIMEOUT, async {
                while conn_tasks.join_next().await.is_some() {}
            })
            .await;
            if drain.is_err() {
                conn_tasks.abort_all();
            }
        });

        Ok(())
    }

    /// Signal the running server to stop accepting new connections.
    pub fn shutdown(&self) {
        self.shutdown.notify_waiters();
    }
}

fn ensure_metric_present(out: &mut String, body: &str, name: &str, type_str: &str) {
    if body.contains(&format!("# TYPE {name} "))
        || body.contains(&format!("{name} "))
        || body.contains(&format!("{name}{{"))
        || body.contains(&format!("{name}_bucket"))
    {
        return;
    }
    let _ = writeln!(out, "# HELP {name} apcore standard metric");
    let _ = writeln!(out, "# TYPE {name} {type_str}");
}

async fn handle_connection(
    mut stream: TcpStream,
    exporter: PrometheusExporter,
    metrics_path: &str,
) -> io::Result<()> {
    // Bound the read so a slow-loris client cannot pin a task indefinitely.
    let request_line = match timeout(REQUEST_READ_TIMEOUT, read_request_line(&mut stream)).await {
        Ok(result) => result?,
        Err(_) => {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "PrometheusExporter: request read timed out",
            ));
        }
    };
    let target = parse_target(&request_line).unwrap_or_default();

    let (status, content_type, body) = if target == metrics_path {
        let body = exporter.export();
        ("200 OK", "text/plain; version=0.0.4; charset=utf-8", body)
    } else if target == "/healthz" {
        ("200 OK", "text/plain; charset=utf-8", "OK\n".to_string())
    } else if target == "/readyz" {
        if exporter.ready.load(Ordering::SeqCst) {
            ("200 OK", "text/plain; charset=utf-8", "OK\n".to_string())
        } else {
            (
                "503 Service Unavailable",
                "text/plain; charset=utf-8",
                "Not ready\n".to_string(),
            )
        }
    } else {
        (
            "404 Not Found",
            "text/plain; charset=utf-8",
            "Not Found\n".to_string(),
        )
    };

    let response = format!(
        "HTTP/1.1 {status}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {len}\r\n\
         Connection: close\r\n\r\n{body}",
        len = body.len(),
    );
    stream.write_all(response.as_bytes()).await?;
    let _ = stream.shutdown().await;
    Ok(())
}

async fn read_request_line(stream: &mut TcpStream) -> io::Result<String> {
    let mut buf = Vec::with_capacity(512);
    let mut tmp = [0u8; 256];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.len() > 8192 {
            break;
        }
    }
    let text = String::from_utf8_lossy(&buf);
    Ok(text.lines().next().unwrap_or_default().to_string())
}

fn parse_target(request_line: &str) -> Option<String> {
    let mut parts = request_line.split_whitespace();
    let _method = parts.next()?;
    let target = parts.next()?;
    Some(target.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_includes_required_metrics_when_empty() {
        let collector = MetricsCollector::new();
        let exporter = PrometheusExporter::new(collector);
        let body = exporter.export();
        assert!(body.contains("apcore_module_calls_total"));
        assert!(body.contains("apcore_module_errors_total"));
        assert!(body.contains("apcore_module_duration_seconds"));
    }

    #[test]
    fn export_includes_required_metrics_when_populated() {
        let collector = MetricsCollector::new();
        collector.increment_calls("mod.a", "success");
        collector.increment_errors("mod.a", "ERR");
        collector.observe_duration("mod.a", 0.1);
        let exporter = PrometheusExporter::new(collector);
        let body = exporter.export();
        assert!(body.contains("apcore_module_calls_total"));
        assert!(body.contains("apcore_module_errors_total"));
        assert!(body.contains("apcore_module_duration_seconds"));
    }
}
