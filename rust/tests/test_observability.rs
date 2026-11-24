//! Tests for the observability subsystem:
//! error_history, exporters, metrics, span, and usage.

use std::collections::HashMap;

use apcore::context::{Context, Identity};
use apcore::errors::{ErrorCode, ModuleError};
use apcore::middleware::base::Middleware;
use apcore::observability::error_history::{ErrorEntry, ErrorHistory, ErrorHistoryMiddleware};
use apcore::observability::exporters::{InMemoryExporter, OTLPExporter, StdoutExporter};
use apcore::observability::metrics::{MetricsCollector, MetricsMiddleware};
use apcore::observability::span::{Span, SpanExporter, SpanStatus};
use apcore::observability::usage::{UsageCollector, UsageMiddleware, UsageStats};
use serde_json::{json, Value};

// ===========================================================================
// Helper: build a minimal context for middleware tests
// ===========================================================================

fn test_context() -> Context<Value> {
    Context::<Value>::new(Identity::new(
        "test-caller".into(),
        "test-caller".into(),
        vec![],
        HashMap::default(),
    ))
}

fn make_error(msg: &str) -> ModuleError {
    ModuleError::new(ErrorCode::ModuleExecuteError, msg)
}

// ===========================================================================
// ErrorHistory
// ===========================================================================

#[test]
fn test_error_history_new() {
    let history = ErrorHistory::new(50);
    assert!(history.get_all(None).is_empty());
}

#[test]
fn test_error_history_record_and_get() {
    let history = ErrorHistory::new(50);
    let err = make_error("something went wrong");
    history.record("mod.a", &err);

    let entries = history.get("mod.a", None);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].module_id, "mod.a");
    assert_eq!(entries[0].message, "something went wrong");
    assert_eq!(entries[0].count, 1);
}

#[test]
fn test_error_history_deduplication() {
    let history = ErrorHistory::new(50);
    let err = make_error("duplicate error");
    history.record("mod.a", &err);
    history.record("mod.a", &err);
    history.record("mod.a", &err);

    let entries = history.get("mod.a", None);
    assert_eq!(entries.len(), 1, "Duplicate errors should be merged");
    assert_eq!(entries[0].count, 3);
}

#[test]
fn test_error_history_different_messages_not_deduplicated() {
    let history = ErrorHistory::new(50);
    history.record("mod.a", &make_error("error one"));
    history.record("mod.a", &make_error("error two"));

    let entries = history.get("mod.a", None);
    assert_eq!(entries.len(), 2);
}

#[test]
fn test_error_history_different_modules() {
    let history = ErrorHistory::new(50);
    history.record("mod.a", &make_error("err"));
    history.record("mod.b", &make_error("err"));

    assert_eq!(history.get("mod.a", None).len(), 1);
    assert_eq!(history.get("mod.b", None).len(), 1);
    assert_eq!(history.get_all(None).len(), 2);
}

#[test]
fn test_error_history_get_nonexistent_module() {
    let history = ErrorHistory::new(50);
    assert!(history.get("nonexistent", None).is_empty());
}

#[test]
fn test_error_history_get_with_limit() {
    let history = ErrorHistory::new(50);
    for i in 0..10 {
        history.record("mod.a", &make_error(&format!("error {i}")));
    }

    let entries = history.get("mod.a", Some(3));
    assert_eq!(entries.len(), 3);
}

#[test]
fn test_error_history_get_all_with_limit() {
    let history = ErrorHistory::new(50);
    for i in 0..10 {
        history.record("mod.a", &make_error(&format!("error {i}")));
    }

    let entries = history.get_all(Some(5));
    assert_eq!(entries.len(), 5);
}

#[test]
fn test_error_history_per_module_eviction() {
    let history = ErrorHistory::new(3);
    for i in 0..5 {
        history.record("mod.a", &make_error(&format!("error {i}")));
    }

    let entries = history.get("mod.a", None);
    assert_eq!(entries.len(), 3, "Should evict oldest entries beyond limit");
}

#[test]
fn test_error_history_total_eviction() {
    // max 2 per module, max 3 total
    let history = ErrorHistory::with_limits(5, 3);
    history.record("mod.a", &make_error("a1"));
    history.record("mod.a", &make_error("a2"));
    history.record("mod.b", &make_error("b1"));
    history.record("mod.b", &make_error("b2"));

    let all = history.get_all(None);
    assert!(
        all.len() <= 3,
        "Total entries should not exceed max_total_entries"
    );
}

#[test]
fn test_error_history_clear_specific_module() {
    let history = ErrorHistory::new(50);
    history.record("mod.a", &make_error("err"));
    history.record("mod.b", &make_error("err"));

    history.clear(Some("mod.a"));
    assert!(history.get("mod.a", None).is_empty());
    assert_eq!(history.get("mod.b", None).len(), 1);
}

#[test]
fn test_error_history_clear_all() {
    let history = ErrorHistory::new(50);
    history.record("mod.a", &make_error("err"));
    history.record("mod.b", &make_error("err"));

    history.clear(None);
    assert!(history.get_all(None).is_empty());
}

#[test]
fn test_error_history_ai_guidance_preserved() {
    let history = ErrorHistory::new(50);
    let err = ModuleError::new(ErrorCode::ModuleExecuteError, "failure")
        .with_ai_guidance("Try retrying the request");
    history.record("mod.a", &err);

    let entries = history.get("mod.a", None);
    assert_eq!(
        entries[0].ai_guidance.as_deref(),
        Some("Try retrying the request")
    );
}

#[test]
fn test_error_entry_serialization_round_trip() {
    let history = ErrorHistory::new(50);
    history.record("mod.a", &make_error("test error"));

    let entries = history.get("mod.a", None);
    let json = serde_json::to_string(&entries[0]).unwrap();
    let restored: ErrorEntry = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.module_id, "mod.a");
    assert_eq!(restored.message, "test error");
    assert_eq!(restored.count, 1);
}

// ===========================================================================
// ErrorHistoryMiddleware
// ===========================================================================

#[tokio::test]
async fn test_error_history_middleware_name() {
    let mw = ErrorHistoryMiddleware::new(ErrorHistory::new(50));
    assert_eq!(mw.name(), "error_history");
}

#[tokio::test]
async fn test_error_history_middleware_before_is_noop() {
    let mw = ErrorHistoryMiddleware::new(ErrorHistory::new(50));
    let ctx = test_context();
    let result = mw.before("mod.a", json!({}), &ctx).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_error_history_middleware_after_is_noop() {
    let mw = ErrorHistoryMiddleware::new(ErrorHistory::new(50));
    let ctx = test_context();
    let result = mw.after("mod.a", json!({}), json!({}), &ctx).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_error_history_middleware_on_error_records() {
    let history = ErrorHistory::new(50);
    let mw = ErrorHistoryMiddleware::new(history.clone());
    let ctx = test_context();
    let err = make_error("middleware caught this");

    let result = mw.on_error("mod.a", json!({}), &err, &ctx).await.unwrap();
    assert!(result.is_none());

    let entries = mw.history().get("mod.a", None);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].message, "middleware caught this");
}

// ===========================================================================
// InMemoryExporter
// ===========================================================================

#[tokio::test]
async fn test_in_memory_exporter_new_empty() {
    let exporter = InMemoryExporter::new();
    assert!(exporter.get_spans().is_empty());
}

#[tokio::test]
async fn test_in_memory_exporter_export_and_get() {
    let exporter = InMemoryExporter::new();
    let span = Span::new("test-span", "trace-1");
    exporter.export(&span).await.unwrap();

    let spans = exporter.get_spans();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].name, "test-span");
    assert_eq!(spans[0].trace_id, "trace-1");
}

#[tokio::test]
async fn test_in_memory_exporter_capacity_eviction() {
    let exporter = InMemoryExporter::with_max_spans(3);
    for i in 0..5 {
        let span = Span::new(format!("span-{i}"), "trace-1");
        exporter.export(&span).await.unwrap();
    }

    let spans = exporter.get_spans();
    assert_eq!(spans.len(), 3);
    // Oldest should have been evicted
    assert_eq!(spans[0].name, "span-2");
    assert_eq!(spans[2].name, "span-4");
}

#[tokio::test]
async fn test_in_memory_exporter_clear() {
    let exporter = InMemoryExporter::new();
    exporter.export(&Span::new("s", "t")).await.unwrap();
    assert_eq!(exporter.get_spans().len(), 1);

    exporter.clear();
    assert!(exporter.get_spans().is_empty());
}

#[tokio::test]
async fn test_in_memory_exporter_default() {
    let exporter = InMemoryExporter::default();
    assert!(exporter.get_spans().is_empty());
}

#[tokio::test]
async fn test_in_memory_exporter_shutdown() {
    let exporter = InMemoryExporter::new();
    exporter.shutdown().await.unwrap();
}

// ===========================================================================
// StdoutExporter
// ===========================================================================

#[tokio::test]
async fn test_stdout_exporter_export_does_not_error() {
    let exporter = StdoutExporter;
    let span = Span::new("test-span", "trace-1");
    // Should succeed without error (output goes to stdout)
    exporter.export(&span).await.unwrap();
}

#[tokio::test]
async fn test_stdout_exporter_shutdown() {
    let exporter = StdoutExporter;
    exporter.shutdown().await.unwrap();
}

// ===========================================================================
// OTLPExporter
// ===========================================================================

#[tokio::test]
async fn test_otlp_exporter_creation() {
    let exporter = OTLPExporter::new("http://localhost:4317");
    assert_eq!(exporter.endpoint, "http://localhost:4317");
}

#[cfg(not(feature = "events"))]
#[tokio::test]
async fn test_otlp_exporter_export_placeholder() {
    // Without the `events` feature the exporter is a no-op that logs
    // and returns Ok. It must never perform a network call, so any
    // endpoint (including an unreachable one) is safe to pass.
    let exporter = OTLPExporter::new("http://127.0.0.1:1");
    let span = Span::new("test-span", "trace-1");
    exporter.export(&span).await.unwrap();
}

#[cfg(feature = "events")]
#[tokio::test]
async fn test_otlp_exporter_sends_span_to_endpoint() {
    // With the `events` feature the exporter performs a real HTTP POST.
    // Spin up a minimal TCP listener on an ephemeral port, accept one
    // connection, capture the raw HTTP request, and reply 200 OK. This
    // avoids any dependency on an external OTLP collector.
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let endpoint = format!("http://127.0.0.1:{port}/v1/traces");

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        // Read until we see the end of headers, then drain the content.
        let mut buf = Vec::with_capacity(8192);
        let mut tmp = [0u8; 1024];
        let mut headers_end: Option<usize> = None;
        let mut content_length: usize = 0;
        while headers_end.is_none() {
            let n = stream.read(&mut tmp).await.unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            if let Some(pos) = find_headers_end(&buf) {
                headers_end = Some(pos);
                content_length = parse_content_length(&buf[..pos]).unwrap_or(0);
            }
        }
        if let Some(pos) = headers_end {
            while buf.len() - pos < content_length {
                let n = stream.read(&mut tmp).await.unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
            }
        }
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        stream.write_all(response).await.unwrap();
        let _ = stream.shutdown().await;
        String::from_utf8_lossy(&buf).to_string()
    });

    let exporter = OTLPExporter::new(endpoint);
    let span = Span::new("otlp-capture-test", "trace-capture-1");
    let expected_name = span.name.clone();
    let result = exporter.export(&span).await;
    assert!(result.is_ok(), "export failed: {result:?}");

    let request = tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("server task timed out")
        .expect("server task panicked");

    assert!(
        request.starts_with("POST "),
        "expected POST request, got: {request}"
    );
    assert!(
        request.starts_with("POST /v1/traces HTTP/1.1"),
        "expected the endpoint's own path, got: {request}"
    );
    assert!(
        request
            .to_lowercase()
            .contains("content-type: application/json"),
        "expected JSON content type, got: {request}"
    );
    assert!(
        request.contains(&expected_name),
        "expected body to contain span name '{expected_name}', got: {request}"
    );
}

#[cfg(feature = "events")]
fn find_headers_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

#[cfg(feature = "events")]
fn parse_content_length(headers: &[u8]) -> Option<usize> {
    let text = std::str::from_utf8(headers).ok()?;
    for line in text.split("\r\n") {
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                return value.trim().parse().ok();
            }
        }
    }
    None
}

#[tokio::test]
async fn test_otlp_exporter_shutdown() {
    let exporter = OTLPExporter::new("http://localhost:4317");
    exporter.shutdown().await.unwrap();
}

// ===========================================================================
// MetricsCollector
// ===========================================================================

#[test]
fn test_metrics_collector_new() {
    let collector = MetricsCollector::new();
    let snap = collector.snapshot();
    assert_eq!(snap["counters"], json!({}));
    assert_eq!(snap["histograms"], json!({}));
}

#[test]
fn test_metrics_collector_default() {
    let collector = MetricsCollector::default();
    let snap = collector.snapshot();
    assert_eq!(snap["counters"], json!({}));
}

#[test]
fn test_metrics_collector_increment_counter() {
    let collector = MetricsCollector::new();
    collector.increment("requests", HashMap::new(), 1.0);
    collector.increment("requests", HashMap::new(), 1.0);

    let snap = collector.snapshot();
    assert_eq!(snap["counters"]["requests"], json!(2.0));
}

#[test]
fn test_metrics_collector_increment_with_labels() {
    let collector = MetricsCollector::new();
    let mut labels = HashMap::new();
    labels.insert("module_id".to_string(), "mod.a".to_string());
    collector.increment("calls", labels, 5.0);

    let snap = collector.snapshot();
    // The key format is "name|key=value"
    assert_eq!(snap["counters"]["calls|module_id=mod.a"], json!(5.0));
}

#[test]
fn test_metrics_collector_observe_histogram() {
    let collector = MetricsCollector::new();
    collector.observe("latency", HashMap::new(), 0.05);

    let snap = collector.snapshot();
    let hist = &snap["histograms"]["latency"];
    assert_eq!(hist["count"], json!(1));
    assert_eq!(hist["sum"], json!(0.05));
}

#[test]
fn test_metrics_collector_histogram_buckets() {
    let collector = MetricsCollector::new();
    // Observe a value that falls into the 0.1 bucket
    collector.observe("latency", HashMap::new(), 0.07);

    let snap = collector.snapshot();
    let buckets = snap["histograms"]["latency"]["buckets"].as_array().unwrap();

    // Buckets below 0.07 should have count 0, buckets >= 0.07 should have count 1
    for bucket in buckets {
        let count = bucket["count"].as_u64().unwrap();
        // D-106: the snapshot also carries the `+Inf` bucket, whose bound is the
        // string `"+Inf"` (JSON has no infinity literal). It holds every
        // observation, which is how a consumer tells "no data" from "all
        // overflow".
        let Some(le) = bucket["le"].as_f64() else {
            assert_eq!(bucket["le"].as_str(), Some("+Inf"));
            assert_eq!(count, 1, "the +Inf bucket counts every observation");
            continue;
        };
        if le >= 0.07 {
            assert_eq!(count, 1, "Bucket le={le} should contain the observation");
        } else {
            assert_eq!(count, 0, "Bucket le={le} should be empty");
        }
    }
}

#[test]
fn test_metrics_collector_multiple_observations() {
    let collector = MetricsCollector::new();
    collector.observe("duration", HashMap::new(), 0.5);
    collector.observe("duration", HashMap::new(), 1.5);
    collector.observe("duration", HashMap::new(), 3.0);

    let snap = collector.snapshot();
    let hist = &snap["histograms"]["duration"];
    assert_eq!(hist["count"], json!(3));
    let sum = hist["sum"].as_f64().unwrap();
    assert!((sum - 5.0).abs() < 1e-10);
}

#[test]
fn test_metrics_collector_increment_calls_convenience() {
    let collector = MetricsCollector::new();
    collector.increment_calls("mod.a", "success");
    collector.increment_calls("mod.a", "success");
    collector.increment_calls("mod.a", "error");

    let snap = collector.snapshot();
    assert_eq!(
        snap["counters"]["apcore_module_calls_total|module_id=mod.a,status=success"],
        json!(2.0)
    );
    assert_eq!(
        snap["counters"]["apcore_module_calls_total|module_id=mod.a,status=error"],
        json!(1.0)
    );
}

#[test]
fn test_metrics_collector_increment_errors_convenience() {
    let collector = MetricsCollector::new();
    collector.increment_errors("mod.a", "ModuleExecuteError");

    let snap = collector.snapshot();
    assert_eq!(
        snap["counters"]
            ["apcore_module_errors_total|error_code=ModuleExecuteError,module_id=mod.a"],
        json!(1.0)
    );
}

#[test]
fn test_metrics_collector_observe_duration_convenience() {
    let collector = MetricsCollector::new();
    collector.observe_duration("mod.a", 0.123);

    let snap = collector.snapshot();
    let hist = &snap["histograms"]["apcore_module_duration_seconds|module_id=mod.a"];
    assert_eq!(hist["count"], json!(1));
    assert_eq!(hist["sum"], json!(0.123));
}

#[test]
fn test_metrics_collector_reset() {
    let collector = MetricsCollector::new();
    collector.increment("counter", HashMap::new(), 1.0);
    collector.observe("hist", HashMap::new(), 0.5);

    collector.reset();
    let snap = collector.snapshot();
    assert_eq!(snap["counters"], json!({}));
    assert_eq!(snap["histograms"], json!({}));
}

#[test]
fn test_metrics_collector_export_prometheus_empty() {
    let collector = MetricsCollector::new();
    let output = collector.export_prometheus();
    assert!(output.is_empty());
}

#[test]
fn test_metrics_collector_export_prometheus_counter() {
    let collector = MetricsCollector::new();
    collector.increment("my_counter", HashMap::new(), 42.0);

    let output = collector.export_prometheus();
    assert!(output.contains("# TYPE my_counter counter"));
    assert!(output.contains("my_counter 42"));
}

#[test]
fn test_metrics_collector_export_prometheus_histogram() {
    let collector = MetricsCollector::new();
    collector.observe("my_hist", HashMap::new(), 0.5);

    let output = collector.export_prometheus();
    assert!(output.contains("# TYPE my_hist histogram"));
    assert!(output.contains("my_hist_bucket"));
    assert!(output.contains("my_hist_sum"));
    assert!(output.contains("my_hist_count"));
    assert!(output.contains("le=\"+Inf\""));
}

// [obs-help-text] Exported HELP lines must use the per-metric-name description
// table (matching apcore-python / apcore-typescript), not a generic
// "Counter metric" / "Histogram metric".
#[test]
fn test_metrics_collector_export_prometheus_uses_per_metric_help_text() {
    let collector = MetricsCollector::new();
    collector.increment_calls("mod.a", "success");
    collector.increment_errors("mod.a", "BOOM");
    collector.observe_duration("mod.a", 0.25);

    let output = collector.export_prometheus();
    assert!(
        output.contains("# HELP apcore_module_calls_total Total module calls"),
        "calls HELP must be 'Total module calls'; got:\n{output}"
    );
    assert!(
        output.contains("# HELP apcore_module_errors_total Total module errors"),
        "errors HELP must be 'Total module errors'; got:\n{output}"
    );
    assert!(
        output.contains("# HELP apcore_module_duration_seconds Module execution duration"),
        "duration HELP must be 'Module execution duration'; got:\n{output}"
    );
    // Unknown metric names fall back to the name itself, not a generic label.
    assert!(
        !output.contains("Counter metric") && !output.contains("Histogram metric"),
        "generic HELP text must no longer be emitted; got:\n{output}"
    );
}

#[test]
fn test_metrics_collector_export_prometheus_with_labels() {
    let collector = MetricsCollector::new();
    let mut labels = HashMap::new();
    labels.insert("env".to_string(), "prod".to_string());
    collector.increment("requests", labels, 10.0);

    let output = collector.export_prometheus();
    assert!(output.contains("env=\"prod\""));
}

// ===========================================================================
// MetricsMiddleware
// ===========================================================================

#[tokio::test]
async fn test_metrics_middleware_name() {
    let mw = MetricsMiddleware::new(MetricsCollector::new());
    assert_eq!(mw.name(), "metrics");
}

#[tokio::test]
async fn test_metrics_middleware_records_success() {
    let collector = MetricsCollector::new();
    let mw = MetricsMiddleware::new(collector.clone());
    let ctx = test_context();

    mw.before("mod.a", json!({}), &ctx).await.unwrap();
    mw.after("mod.a", json!({}), json!({"ok": true}), &ctx)
        .await
        .unwrap();

    let snap = mw.collector().snapshot();
    assert_eq!(
        snap["counters"]["apcore_module_calls_total|module_id=mod.a,status=success"],
        json!(1.0)
    );
    // Duration histogram should have 1 observation
    let hist = &snap["histograms"]["apcore_module_duration_seconds|module_id=mod.a"];
    assert_eq!(hist["count"], json!(1));
}

#[tokio::test]
async fn test_metrics_middleware_records_error() {
    let collector = MetricsCollector::new();
    let mw = MetricsMiddleware::new(collector.clone());
    let ctx = test_context();
    let err = make_error("fail");

    mw.before("mod.a", json!({}), &ctx).await.unwrap();
    mw.on_error("mod.a", json!({}), &err, &ctx).await.unwrap();

    let snap = mw.collector().snapshot();
    assert_eq!(
        snap["counters"]["apcore_module_calls_total|module_id=mod.a,status=error"],
        json!(1.0)
    );
    // Error counter should also be incremented
    let errors_key = snap["counters"]
        .as_object()
        .unwrap()
        .keys()
        .find(|k| k.starts_with("apcore_module_errors_total"));
    assert!(errors_key.is_some(), "Error counter should be recorded");
}

// ===========================================================================
// Span
// ===========================================================================

#[test]
fn test_span_new() {
    let span = Span::new("my-operation", "trace-abc");
    assert_eq!(span.name, "my-operation");
    assert_eq!(span.trace_id, "trace-abc");
    assert!(!span.span_id.is_empty());
    assert!(span.parent_span_id.is_none());
    assert!(span.end_time.is_none());
    assert!(span.attributes.is_empty());
    assert_eq!(span.status, SpanStatus::Unset);
    assert!(span.start_time > 0.0);
}

#[test]
fn test_span_unique_ids() {
    let s1 = Span::new("op", "t1");
    let s2 = Span::new("op", "t1");
    assert_ne!(s1.span_id, s2.span_id, "Each span should get a unique ID");
}

#[test]
fn test_span_end() {
    let mut span = Span::new("op", "t1");
    assert!(span.end_time.is_none());
    span.end();
    assert!(span.end_time.is_some());
    assert!(span.end_time.unwrap() >= span.start_time);
}

#[test]
fn test_span_set_attribute() {
    let mut span = Span::new("op", "t1");
    span.set_attribute("key1", json!("value1"));
    span.set_attribute("key2", json!(42));

    assert_eq!(span.attributes["key1"], json!("value1"));
    assert_eq!(span.attributes["key2"], json!(42));
}

#[test]
fn test_span_set_attribute_overwrite() {
    let mut span = Span::new("op", "t1");
    span.set_attribute("key", json!("old"));
    span.set_attribute("key", json!("new"));
    assert_eq!(span.attributes["key"], json!("new"));
}

#[test]
fn test_span_add_event() {
    let mut span = Span::new("op", "t1");
    span.add_event("checkpoint-1");
    span.add_event("checkpoint-2");

    // events is pub(crate), but we can verify via serialization
    let json_str = serde_json::to_string(&span).unwrap();
    assert!(json_str.contains("checkpoint-1"));
    assert!(json_str.contains("checkpoint-2"));
}

#[test]
fn test_span_add_event_with_attributes() {
    let mut span = Span::new("op", "t1");
    let mut attrs = HashMap::new();
    attrs.insert("detail".to_string(), json!("some info"));
    span.add_event_with_attributes("event-with-data", attrs);

    let json_str = serde_json::to_string(&span).unwrap();
    assert!(json_str.contains("event-with-data"));
    assert!(json_str.contains("some info"));
}

#[test]
fn test_span_serialization_round_trip() {
    let mut span = Span::new("my-span", "trace-123");
    span.set_attribute("foo", json!("bar"));
    span.end();

    let json = serde_json::to_string(&span).unwrap();
    let restored: Span = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.name, "my-span");
    assert_eq!(restored.trace_id, "trace-123");
    assert_eq!(restored.span_id, span.span_id);
    assert_eq!(restored.attributes["foo"], json!("bar"));
    assert!(restored.end_time.is_some());
}

#[test]
fn test_span_status_variants() {
    assert_eq!(SpanStatus::Unset, SpanStatus::Unset);
    assert_ne!(SpanStatus::Ok, SpanStatus::Error);

    // Serialization
    let json = serde_json::to_string(&SpanStatus::Ok).unwrap();
    assert_eq!(json, "\"ok\"");
    let json = serde_json::to_string(&SpanStatus::Error).unwrap();
    assert_eq!(json, "\"error\"");
    let json = serde_json::to_string(&SpanStatus::Unset).unwrap();
    assert_eq!(json, "\"unset\"");
}

// ===========================================================================
// UsageCollector
// ===========================================================================

#[test]
fn test_usage_collector_new() {
    let collector = UsageCollector::new();
    assert!(collector.get_all_summaries().is_empty());
}

#[test]
fn test_usage_collector_default() {
    let collector = UsageCollector::default();
    assert!(collector.get_all_summaries().is_empty());
}

// [obs-usage-window] export_prometheus must apply the same rolling 24h window
// as system.usage.module: records older than 24h are excluded from the exported
// counts and percentiles. Mirrors apcore-python.
#[test]
fn test_usage_export_prometheus_excludes_records_older_than_24h() {
    use chrono::{Duration as ChronoDuration, Utc};

    let collector = UsageCollector::new();
    let now = Utc::now();

    // One recent record (in window) with a small latency.
    collector.record_at(
        "mod.a",
        Some("c"),
        10.0,
        true,
        now - ChronoDuration::hours(1),
    );
    // One stale record (older than 24h) with a huge latency that would dominate
    // p99 if it were (incorrectly) included.
    collector.record_at(
        "mod.a",
        Some("c"),
        9_999.0,
        true,
        now - ChronoDuration::hours(30),
    );

    let output = collector.export_prometheus();

    // The stale record's latency must not appear in the exported p99.
    let p99_line = output
        .lines()
        .find(|l| l.starts_with("apcore_usage_p99_latency_ms{module_id=\"mod.a\"}"))
        .expect("p99 line present");
    assert!(
        !p99_line.contains("9999"),
        "stale (>24h) record must be excluded from exported p99; got: {p99_line}"
    );

    // Only the single in-window success should be counted.
    assert!(
        output.contains("apcore_usage_calls_total{module_id=\"mod.a\",status=\"success\"} 1"),
        "only the in-window record should be counted; got:\n{output}"
    );
}

#[test]
fn test_usage_collector_record_and_summary() {
    let collector = UsageCollector::new();
    collector.record("mod.a", Some("caller-1"), 10.0, true);
    collector.record("mod.a", Some("caller-1"), 20.0, true);

    let stats = collector.get_module_summary("mod.a").unwrap();
    assert_eq!(stats.module_id, "mod.a");
    assert_eq!(stats.call_count, 2);
    assert_eq!(stats.error_count, 0);
    assert!((stats.avg_latency_ms - 15.0).abs() < 1e-10);
    assert_eq!(stats.unique_callers, 1);
}

#[test]
fn test_usage_collector_error_counting() {
    let collector = UsageCollector::new();
    collector.record("mod.a", None, 5.0, true);
    collector.record("mod.a", None, 5.0, false);
    collector.record("mod.a", None, 5.0, false);

    let stats = collector.get_module_summary("mod.a").unwrap();
    assert_eq!(stats.call_count, 3);
    assert_eq!(stats.error_count, 2);
}

#[test]
fn test_usage_collector_unique_callers() {
    let collector = UsageCollector::new();
    collector.record("mod.a", Some("caller-1"), 1.0, true);
    collector.record("mod.a", Some("caller-2"), 1.0, true);
    collector.record("mod.a", Some("caller-1"), 1.0, true);
    collector.record("mod.a", None, 1.0, true);

    let stats = collector.get_module_summary("mod.a").unwrap();
    assert_eq!(
        stats.unique_callers, 2,
        "Only named callers count as unique"
    );
}

#[test]
fn test_usage_collector_nonexistent_module() {
    let collector = UsageCollector::new();
    assert!(collector.get_module_summary("nonexistent").is_none());
}

#[test]
fn test_usage_collector_multiple_modules() {
    let collector = UsageCollector::new();
    collector.record("mod.a", None, 1.0, true);
    collector.record("mod.b", None, 2.0, true);

    let summaries = collector.get_all_summaries();
    assert_eq!(summaries.len(), 2);
}

#[test]
fn test_usage_collector_reset() {
    let collector = UsageCollector::new();
    collector.record("mod.a", None, 1.0, true);
    collector.reset();
    assert!(collector.get_all_summaries().is_empty());
    assert!(collector.get_module_summary("mod.a").is_none());
}

#[test]
fn test_usage_collector_trend_default() {
    let collector = UsageCollector::new();
    collector.record("mod.a", None, 1.0, true);

    let stats = collector.get_module_summary("mod.a").unwrap();
    assert_eq!(stats.trend, "stable");
}

#[test]
fn test_usage_collector_p99_latency_empty() {
    let collector = UsageCollector::new();
    assert!((collector.get_p99_latency_ms("mod.a") - 0.0).abs() < f64::EPSILON);
}

#[test]
fn test_usage_collector_p99_latency_single_record() {
    let collector = UsageCollector::new();
    collector.record("mod.a", None, 42.0, true);

    let p99 = collector.get_p99_latency_ms("mod.a");
    assert!((p99 - 42.0).abs() < 1e-10);
}

#[test]
fn test_usage_collector_p99_latency_multiple_records() {
    let collector = UsageCollector::new();
    // Record 100 values: 1.0 through 100.0
    for i in 1..=100 {
        collector.record("mod.a", None, f64::from(i), true);
    }

    let p99 = collector.get_p99_latency_ms("mod.a");
    // p99 of 1..=100: ceil(100 * 0.99) = 99, index 98 = 99.0
    assert!((p99 - 99.0).abs() < 1e-10);
}

#[test]
fn test_usage_stats_serialization() {
    let stats = UsageStats {
        module_id: "mod.a".to_string(),
        call_count: 10,
        error_count: 2,
        avg_latency_ms: 15.5,
        unique_callers: 3,
        trend: "stable".to_string(),
    };

    let json = serde_json::to_string(&stats).unwrap();
    let restored: UsageStats = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.module_id, "mod.a");
    assert_eq!(restored.call_count, 10);
    assert_eq!(restored.error_count, 2);
}

// ===========================================================================
// UsageMiddleware
// ===========================================================================

#[tokio::test]
async fn test_usage_middleware_name() {
    let mw = UsageMiddleware::new(UsageCollector::new());
    assert_eq!(mw.name(), "usage");
}

#[tokio::test]
async fn test_usage_middleware_records_success() {
    let collector = UsageCollector::new();
    let mw = UsageMiddleware::new(collector.clone());
    let ctx = test_context();

    mw.before("mod.a", json!({}), &ctx).await.unwrap();
    mw.after("mod.a", json!({}), json!({}), &ctx).await.unwrap();

    let stats = mw.collector().get_module_summary("mod.a").unwrap();
    assert_eq!(stats.call_count, 1);
    assert_eq!(stats.error_count, 0);
}

#[tokio::test]
async fn test_usage_middleware_records_error() {
    let collector = UsageCollector::new();
    let mw = UsageMiddleware::new(collector.clone());
    let ctx = test_context();
    let err = make_error("fail");

    mw.before("mod.a", json!({}), &ctx).await.unwrap();
    mw.on_error("mod.a", json!({}), &err, &ctx).await.unwrap();

    let stats = mw.collector().get_module_summary("mod.a").unwrap();
    assert_eq!(stats.call_count, 1);
    assert_eq!(stats.error_count, 1);
}

#[tokio::test]
async fn test_usage_middleware_caller_id_from_context() {
    let collector = UsageCollector::new();
    let mw = UsageMiddleware::new(collector.clone());
    // Per Issue #66, top-level `Context::create` no longer accepts caller_id;
    // it is managed exclusively by `Context::child()`. To simulate an
    // Executor-set caller_id in this test we assign the field post-hoc — the
    // production pipeline never does this directly.
    let mut ctx = Context::<Value>::create(
        Some(Identity::new(
            "test-caller".into(),
            "test-caller".into(),
            vec![],
            HashMap::default(),
        )),
        None,
        None,
        None,
        Value::Null,
        None,
    );
    ctx.caller_id = Some("explicit-caller".to_string());

    mw.before("mod.a", json!({}), &ctx).await.unwrap();
    mw.after("mod.a", json!({}), json!({}), &ctx).await.unwrap();

    let stats = mw.collector().get_module_summary("mod.a").unwrap();
    assert_eq!(stats.unique_callers, 1);
}

#[tokio::test]
async fn test_usage_middleware_no_caller_id() {
    let collector = UsageCollector::new();
    let mw = UsageMiddleware::new(collector.clone());
    let ctx = test_context(); // caller_id is None

    mw.before("mod.a", json!({}), &ctx).await.unwrap();
    mw.after("mod.a", json!({}), json!({}), &ctx).await.unwrap();

    let stats = mw.collector().get_module_summary("mod.a").unwrap();
    assert_eq!(
        stats.unique_callers, 0,
        "None caller_id should not count as unique"
    );
}

// [obs-nested-timing] A nested call sharing the same trace_id must not clobber
// the outer call's start time: the outer call must record a non-zero latency.
#[tokio::test]
async fn test_usage_middleware_nested_same_trace_records_outer_latency() {
    let mw = UsageMiddleware::new(UsageCollector::new());
    let mut ctx = test_context();
    ctx.trace_id = "shared-trace".to_string();

    // Outer begins.
    mw.before("outer", json!({}), &ctx).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    // Nested call on the same trace.
    mw.before("inner", json!({}), &ctx).await.unwrap();
    mw.after("inner", json!({}), json!({}), &ctx).await.unwrap();
    // Outer completes.
    mw.after("outer", json!({}), json!({}), &ctx).await.unwrap();

    let outer = mw.collector().get_module_summary("outer").unwrap();
    assert!(
        outer.avg_latency_ms > 0.0,
        "nested same-trace call must not zero out the outer latency; got {}",
        outer.avg_latency_ms
    );
}
