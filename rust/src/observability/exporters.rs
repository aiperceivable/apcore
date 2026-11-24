// APCore Protocol — Span exporters
// Spec reference: Built-in span export destinations

use async_trait::async_trait;
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::Arc;

use super::span::{Span, SpanExporter};
use crate::errors::ModuleError;

/// Exports spans to stdout as JSON.
#[derive(Debug)]
pub struct StdoutExporter;

#[async_trait]
impl SpanExporter for StdoutExporter {
    async fn export(&self, span: &Span) -> Result<(), ModuleError> {
        let json = serde_json::to_string(span).map_err(|e| {
            ModuleError::new(
                crate::errors::ErrorCode::GeneralInternalError,
                format!("Failed to serialize span: {e}"),
            )
        })?;
        // Route through tracing so the span line integrates with the
        // application's tracing-subscriber configuration and does not
        // bypass log aggregation pipelines.
        tracing::info!(target: "apcore.span", span = %json);
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), ModuleError> {
        Ok(())
    }
}

/// Default maximum spans for `InMemoryExporter`, as in apcore-python and apcore-typescript.
const DEFAULT_MAX_SPANS: usize = 10_000;

/// Exports spans to an in-memory buffer for testing.
#[derive(Debug, Clone)]
pub struct InMemoryExporter {
    spans: Arc<Mutex<VecDeque<Span>>>,
    max_spans: usize,
}

impl InMemoryExporter {
    /// Create a new in-memory exporter with default capacity.
    #[must_use]
    pub fn new() -> Self {
        Self {
            spans: Arc::new(Mutex::new(VecDeque::new())),
            max_spans: DEFAULT_MAX_SPANS,
        }
    }

    /// Create with explicit max spans capacity.
    #[must_use]
    pub fn with_max_spans(max_spans: usize) -> Self {
        Self {
            spans: Arc::new(Mutex::new(VecDeque::new())),
            max_spans,
        }
    }

    /// Get all exported spans.
    #[must_use]
    pub fn get_spans(&self) -> Vec<Span> {
        self.spans.lock().iter().cloned().collect()
    }

    /// Clear all exported spans.
    pub fn clear(&self) {
        self.spans.lock().clear();
    }
}

impl Default for InMemoryExporter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SpanExporter for InMemoryExporter {
    async fn export(&self, span: &Span) -> Result<(), ModuleError> {
        let mut spans = self.spans.lock();
        spans.push_back(span.clone());
        while spans.len() > self.max_spans {
            spans.pop_front();
        }
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), ModuleError> {
        Ok(())
    }
}

/// Exports spans to an OTLP-compatible endpoint.
///
/// `endpoint` is the full OTLP/HTTP traces URL — `http://host:4318/v1/traces`,
/// the shape `observability.tracing.otlp_endpoint` and its default carry, and
/// the one apcore-python and apcore-typescript post to. It is used as given.
#[derive(Debug)]
pub struct OTLPExporter {
    pub endpoint: String,
}

impl OTLPExporter {
    /// Create a new OTLP exporter.
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
        }
    }
}

/// Convert a Span to OTLP JSON format.
#[cfg(feature = "events")]
fn span_to_otlp(span: &Span) -> serde_json::Value {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    // intentional: nanosecond timestamps fit in u64 for dates post-1970
    let start_ns = (span.start_time * 1_000_000_000.0) as u64;
    let mut otlp_span = serde_json::json!({
        "traceId": span.trace_id,
        "spanId": span.span_id,
        "name": span.name,
        "startTimeUnixNano": start_ns,
        "status": match span.status {
            super::span::SpanStatus::Ok => serde_json::json!({"code": 1}),
            super::span::SpanStatus::Error => serde_json::json!({"code": 2}),
            super::span::SpanStatus::Unset => serde_json::json!({"code": 0}),
        },
        "attributes": span.attributes.iter().map(|(k, v)| {
            serde_json::json!({
                "key": k,
                "value": { "stringValue": v.to_string() }
            })
        }).collect::<Vec<_>>(),
    });
    if let Some(ref parent_id) = span.parent_span_id {
        otlp_span["parentSpanId"] = serde_json::json!(parent_id);
    }
    if let Some(end_time) = span.end_time {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        // intentional: nanosecond timestamps fit in u64 for dates post-1970
        let end_ns = (end_time * 1_000_000_000.0) as u64;
        otlp_span["endTimeUnixNano"] = serde_json::json!(end_ns);
    }
    otlp_span
}

/// Returns whether the endpoint uses a loopback host.
#[cfg(feature = "events")]
fn is_loopback_endpoint(endpoint: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(endpoint) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    host.strip_suffix('.')
        .unwrap_or(host)
        .eq_ignore_ascii_case("localhost")
        || host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.to_canonical().is_loopback())
}

#[cfg(feature = "events")]
#[async_trait]
impl SpanExporter for OTLPExporter {
    async fn export(&self, span: &Span) -> Result<(), ModuleError> {
        let mut builder = reqwest::Client::builder();
        if is_loopback_endpoint(&self.endpoint) {
            // Avoid macOS system proxy discovery and keep local collectors local.
            // Remote endpoints retain reqwest's normal proxy configuration.
            builder = builder.no_proxy();
        }
        let client = builder.build().map_err(|e| {
            ModuleError::new(
                crate::errors::ErrorCode::GeneralInternalError,
                format!("OTLP HTTP client error: {e}"),
            )
        })?;
        let payload = serde_json::json!({
            "resourceSpans": [{
                "scopeSpans": [{
                    "spans": [span_to_otlp(span)]
                }]
            }]
        });
        client
            .post(&self.endpoint)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| {
                ModuleError::new(
                    crate::errors::ErrorCode::GeneralInternalError,
                    format!("OTLP export failed: {e}"),
                )
            })?;
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), ModuleError> {
        Ok(())
    }
}

#[cfg(not(feature = "events"))]
#[async_trait]
impl SpanExporter for OTLPExporter {
    async fn export(&self, _span: &Span) -> Result<(), ModuleError> {
        // Without the `events` feature, export is a silent no-op.
        // No network call is made; spans are discarded.
        tracing::warn!(
            "OTLPExporter::export called but the `events` feature is not enabled; span discarded"
        );
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), ModuleError> {
        Ok(())
    }
}

/// Composite span exporter that fan-outs each span to multiple wrapped
/// exporters with per-exporter error isolation. A failure in one wrapped
/// exporter is logged and the remaining exporters still receive the span,
/// matching apcore-python's `_CompositeExporter` (extensions.py:27-38).
///
/// Sync EXT-003: when ≥2 span exporters are registered via the
/// `ExtensionManager`, they are wrapped in this struct so all of them are
/// invoked instead of silently dropping the trailing entries.
pub struct CompositeExporter {
    inner: Vec<Arc<dyn SpanExporter>>,
}

impl std::fmt::Debug for CompositeExporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompositeExporter")
            .field("count", &self.inner.len())
            .finish()
    }
}

impl CompositeExporter {
    /// Wrap N span exporters into a single composite exporter that forwards
    /// each span to all of them.
    #[must_use]
    pub fn new(inner: Vec<Box<dyn SpanExporter>>) -> Self {
        Self::from_shared(inner.into_iter().map(Arc::from).collect())
    }

    /// Wrap N *shared* span exporters. Used by
    /// [`ExtensionManager::apply`](crate::extensions::ExtensionManager::apply),
    /// which keeps its registrations after wiring them (D-78) and so can only
    /// hand out clones of the handle.
    #[must_use]
    pub fn from_shared(inner: Vec<Arc<dyn SpanExporter>>) -> Self {
        Self { inner }
    }

    /// Returns the number of wrapped exporters.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns true if the composite has no wrapped exporters.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

#[async_trait]
impl SpanExporter for CompositeExporter {
    async fn export(&self, span: &Span) -> Result<(), ModuleError> {
        for (idx, exporter) in self.inner.iter().enumerate() {
            if let Err(e) = exporter.export(span).await {
                tracing::warn!(
                    exporter_index = idx,
                    error = %e.message,
                    "CompositeExporter: wrapped exporter failed; continuing with remaining exporters"
                );
            }
        }
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), ModuleError> {
        for (idx, exporter) in self.inner.iter().enumerate() {
            if let Err(e) = exporter.shutdown().await {
                tracing::warn!(
                    exporter_index = idx,
                    error = %e.message,
                    "CompositeExporter: wrapped exporter shutdown failed"
                );
            }
        }
        Ok(())
    }
}

#[cfg(all(test, feature = "events"))]
mod tests {
    use super::is_loopback_endpoint;

    #[test]
    fn test_proxy_bypass_rejects_multiple_trailing_dots() {
        for endpoint in [
            "http://localhost..:4318/v1/traces",
            "http://localhost...:4318/v1/traces",
        ] {
            assert!(!is_loopback_endpoint(endpoint), "{endpoint}");
        }
    }

    #[test]
    fn test_proxy_bypass_handles_ipv4_mapped_ipv6_addresses() {
        for (endpoint, expected) in [
            ("http://[::ffff:127.0.0.1]:4318/v1/traces", true),
            ("http://[::ffff:127.42.0.1]:4318/v1/traces", true),
            ("http://[::ffff:7f00:1]:4318/v1/traces", true),
            ("http://[::ffff:192.168.1.1]:4318/v1/traces", false),
            ("http://[::ffff:128.0.0.1]:4318/v1/traces", false),
            ("http://[::127.0.0.1]:4318/v1/traces", false),
        ] {
            assert_eq!(is_loopback_endpoint(endpoint), expected, "{endpoint}");
        }
    }

    #[test]
    fn test_proxy_bypass_is_limited_to_loopback_hosts() {
        for endpoint in [
            "http://127.0.0.1:4318/v1/traces",
            "http://127.42.0.1:4318/v1/traces",
            "http://[::1]:4318/v1/traces",
            "https://LOCALHOST:4318/v1/traces",
            "http://localhost.:4318/v1/traces",
        ] {
            assert!(is_loopback_endpoint(endpoint), "{endpoint}");
        }
        for endpoint in [
            "https://collector.example/v1/traces",
            "http://192.168.1.1:4318/v1/traces",
            "http://[2001:db8::1]:4318/v1/traces",
            "http://localhost.example/v1/traces",
            "http://127.0.0.1.example/v1/traces",
            "http://localhost@collector.example/v1/traces",
            "http://collector.example/v1/traces?host=127.0.0.1",
            "file:///v1/traces",
            "not a URL",
        ] {
            assert!(!is_loopback_endpoint(endpoint), "{endpoint}");
        }
    }
}
