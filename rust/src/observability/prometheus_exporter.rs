// APCore Protocol — Prometheus exporter & K8s health endpoints
// Spec reference: observability.md §1.6 K8s/Prometheus Integration Hooks

use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, Semaphore};
use tokio::task::JoinSet;
use tokio::time::{timeout, Duration};

use super::metrics::MetricsCollector;
use super::prometheus::render_prometheus_metrics;
use super::usage::UsageCollector;

/// Per-connection read deadline. A slow client that drip-feeds bytes is
/// treated as a slow-loris attempt; the request is dropped after this window.
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(10);
const RESPONSE_WRITE_TIMEOUT: Duration = Duration::from_secs(10);

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
    listener: Arc<parking_lot::Mutex<Option<RunningListener>>>,
}

#[derive(Debug)]
struct RunningListener {
    address: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
}

struct ListenerBinding(Arc<parking_lot::Mutex<Option<RunningListener>>>);

impl Drop for ListenerBinding {
    fn drop(&mut self) {
        self.0.lock().take();
    }
}

impl PrometheusExporter {
    /// Create a new exporter bound to the given collector.
    #[must_use]
    pub fn new(collector: MetricsCollector) -> Self {
        Self {
            collector,
            usage_collector: None,
            ready: Arc::new(AtomicBool::new(false)),
            listener: Arc::new(parking_lot::Mutex::new(None)),
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
        render_prometheus_metrics(&self.collector, self.usage_collector.as_ref())
    }

    /// Mark the application as ready to serve traffic. `/readyz` will return 200.
    pub fn mark_ready(&self) {
        self.ready.store(true, Ordering::SeqCst);
    }

    /// Mark the application as not ready (e.g. during shutdown).
    pub fn mark_not_ready(&self) {
        self.ready.store(false, Ordering::SeqCst);
    }

    /// Address of the listening socket, or `None` after it closes.
    #[must_use]
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.listener
            .lock()
            .as_ref()
            .map(|listener| listener.address)
    }

    /// Spawn a background HTTP server on the given port, serving metrics at `path`.
    ///
    /// The returned future resolves once the listener is bound. The server runs
    /// until `shutdown()` is called.
    ///
    /// # Errors
    /// Returns an error if the exporter is already listening or the port cannot be bound.
    pub async fn start(&self, port: u16, path: &str) -> io::Result<()> {
        let addr: SocketAddr = ([0, 0, 0, 0], port).into();
        let listener = TcpListener::bind(addr).await?;
        let address = listener.local_addr()?;
        let (shutdown, stop) = oneshot::channel();
        {
            let mut state = self.listener.lock();
            if state.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "PrometheusExporter is already listening",
                ));
            }
            *state = Some(RunningListener {
                address,
                shutdown: Some(shutdown),
            });
        }
        tokio::spawn(serve(
            listener,
            self.clone(),
            path.to_owned(),
            stop,
            ListenerBinding(Arc::clone(&self.listener)),
        ));
        Ok(())
    }

    /// Signal the running server to stop accepting new connections.
    pub fn shutdown(&self) {
        let shutdown = self
            .listener
            .lock()
            .as_mut()
            .and_then(|listener| listener.shutdown.take());
        if let Some(shutdown) = shutdown {
            let _ = shutdown.send(());
        }
    }
}

async fn serve(
    listener: TcpListener,
    exporter: PrometheusExporter,
    path: String,
    mut stop: oneshot::Receiver<()>,
    binding: ListenerBinding,
) {
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTIONS));
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _ = &mut stop => break,
            Some(_) = connections.join_next() => {},
            accept = listener.accept() => match accept {
                Ok((stream, _)) => {
                    let Ok(permit) = Arc::clone(&semaphore).try_acquire_owned() else {
                        tracing::warn!("PrometheusExporter: connection refused, max concurrent connections reached");
                        continue;
                    };
                    let exporter = exporter.clone();
                    let path = path.clone();
                    connections.spawn(async move {
                        let _permit = permit;
                        if let Err(error) = handle_connection(stream, exporter, &path).await {
                            tracing::debug!(%error, "PrometheusExporter: connection error");
                        }
                    });
                }
                Err(error) => tracing::warn!(%error, "PrometheusExporter: accept failed"),
            },
        }
    }

    drop(listener);
    drop(binding);
    if timeout(REQUEST_READ_TIMEOUT, async {
        while connections.join_next().await.is_some() {}
    })
    .await
    .is_err()
    {
        connections.abort_all();
    }
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
    timeout(RESPONSE_WRITE_TIMEOUT, async {
        stream.write_all(response.as_bytes()).await?;
        let _ = stream.shutdown().await;
        Ok(())
    })
    .await
    .unwrap_or_else(|_| {
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "PrometheusExporter: response write timed out",
        ))
    })
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

    #[tokio::test]
    async fn shutdown_before_listener_task_starts_releases_port_and_allows_restart() {
        let exporter = PrometheusExporter::new(MetricsCollector::new());
        exporter.start(0, "/metrics").await.unwrap();
        let bound = exporter.local_addr().unwrap();
        exporter.shutdown();

        timeout(Duration::from_secs(2), async {
            loop {
                match TcpListener::bind(bound).await {
                    Ok(_) => break,
                    Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    Err(error) => panic!("unexpected error rebinding listener: {error}"),
                }
            }
        })
        .await
        .expect("shutdown should release the port even before the listener task first polls");
        assert!(exporter.local_addr().is_none());

        exporter.start(bound.port(), "/metrics").await.unwrap();
        let mut socket = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, bound.port()))
            .await
            .unwrap();
        socket
            .write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        let mut response = String::new();
        timeout(Duration::from_secs(2), socket.read_to_string(&mut response))
            .await
            .unwrap()
            .unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        exporter.shutdown();
    }

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

    #[test]
    fn render_prometheus_metrics_matches_legacy_export_with_live_usage() {
        let metrics = MetricsCollector::new();
        let usage = UsageCollector::new();
        let exporter = PrometheusExporter::new(metrics.clone()).with_usage_collector(usage.clone());

        assert_eq!(
            crate::render_prometheus_metrics(&metrics, Some(&usage)),
            exporter.export()
        );
        metrics.increment_calls("sample.metric", "success");
        usage.record("sample.metric", Some("sample.caller"), 1.0, true);

        let snapshot = crate::render_prometheus_metrics(&metrics, Some(&usage));
        assert_eq!(snapshot, exporter.export());
        assert!(snapshot.contains("apcore_usage_"));
        assert!(snapshot.contains("sample.metric"));
    }
}
