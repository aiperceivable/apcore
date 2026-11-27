//! Spec-traced contract tests for the apcore Observability System (Rust SDK).
//!
//! Source spec: apcore/docs/features/observability.md
//! Canonical suite mirrored: apcore-python/tests/test_observability_spec.py
//!
//! Contracts under test:
//!   1. `Tracer.start_span` — MISSING SYMBOL in apcore-rust (no `Tracer`
//!      type; the SDK exposes `Span` / `TracingMiddleware` instead). Ignored.
//!   2. `MetricsEmitter.record` — MISSING SYMBOL in apcore-rust (no
//!      `MetricsEmitter` type; the SDK exposes `MetricsCollector` instead).
//!      Ignored.
//!   3. `render_prometheus_metrics` — PRESENT and fully exercised.
//!
//! Each test fn name is the clause-id flattened to snake_case and carries the
//! verbatim clause-id in a leading `// clause:` comment so cross-language diffs
//! line up row-for-row with the Python and TypeScript suites.
//!
//! These tests are READ-ONLY contract verification — they never modify src/.

use apcore::observability::{render_prometheus_metrics, MetricsCollector};

fn collector_with_one_counter() -> MetricsCollector {
    let collector = MetricsCollector::new();
    collector.increment_calls("math.add", "success");
    collector
}

// ===========================================================================
// Contract 1: Tracer.start_span  (MISSING SYMBOL — contract gap)
// ===========================================================================

// clause: observability.start_span.input.name.empty
#[test]
#[ignore = "observability.start_span.input.name.empty: missing symbol Tracer (contract gap)"]
fn observability_start_span_input_name_empty() {
    // `Tracer` is not shipped by apcore-rust (the SDK uses Span/TracingMiddleware).
    unreachable!("Tracer type does not exist in apcore-rust");
}

// clause: observability.start_span.property.async
#[test]
#[ignore = "observability.start_span.property.async: missing symbol Tracer (contract gap)"]
fn observability_start_span_property_async() {
    unreachable!("Tracer type does not exist in apcore-rust");
}

// clause: observability.start_span.property.thread_safe
#[test]
#[ignore = "observability.start_span.property.thread_safe: missing symbol Tracer (contract gap)"]
fn observability_start_span_property_thread_safe() {
    unreachable!("Tracer type does not exist in apcore-rust");
}

// clause: observability.start_span.property.pure
#[test]
#[ignore = "observability.start_span.property.pure: missing symbol Tracer (contract gap)"]
fn observability_start_span_property_pure() {
    unreachable!("Tracer type does not exist in apcore-rust");
}

// ===========================================================================
// Contract 2: MetricsEmitter.record  (MISSING SYMBOL — contract gap)
// ===========================================================================

// clause: observability.record.input.metric_name.registered
#[test]
#[ignore = "observability.record.input.metric_name.registered: missing symbol MetricsEmitter (contract gap)"]
fn observability_record_input_metric_name_registered() {
    // `MetricsEmitter` is not shipped; the SDK uses MetricsCollector.increment*.
    unreachable!("MetricsEmitter type does not exist in apcore-rust");
}

// clause: observability.record.property.async
#[test]
#[ignore = "observability.record.property.async: missing symbol MetricsEmitter (contract gap)"]
fn observability_record_property_async() {
    unreachable!("MetricsEmitter type does not exist in apcore-rust");
}

// clause: observability.record.property.thread_safe
#[test]
#[ignore = "observability.record.property.thread_safe: missing symbol MetricsEmitter (contract gap)"]
fn observability_record_property_thread_safe() {
    unreachable!("MetricsEmitter type does not exist in apcore-rust");
}

// clause: observability.record.property.pure
#[test]
#[ignore = "observability.record.property.pure: missing symbol MetricsEmitter (contract gap)"]
fn observability_record_property_pure() {
    unreachable!("MetricsEmitter type does not exist in apcore-rust");
}

// ===========================================================================
// Contract 3: Prometheus text rendering
// ===========================================================================

// clause: observability.export.input.collector.required
#[test]
fn observability_export_input_collector_required() {
    // The renderer reads metrics from the collector supplied by the caller.
    let collector = MetricsCollector::new();
    collector.increment_calls("math.add", "success");
    let text = render_prometheus_metrics(&collector, None);
    assert!(text.contains("apcore_module_calls_total"));
    assert!(text.contains("module_id=\"math.add\""));
    assert!(text.contains("status=\"success\""));
}

// clause: observability.export.error.none
#[test]
fn observability_export_error_none() {
    // ### Errors: None — export errors MUST NOT propagate. `export()` returns a
    // plain `String` (no `Result`); exporting over an empty collector must not
    // panic and must yield a string.
    let text = render_prometheus_metrics(&MetricsCollector::new(), None);
    // A `String` is always returned; the three required metric names are present
    // even on a cold/empty collector (spec §1.6).
    assert!(text.contains("apcore_module_calls_total"));
}

// clause: observability.export.returns.prometheus_text
#[test]
fn observability_export_returns_prometheus_text() {
    // ### Returns: Prometheus text exposition format, UTF-8. A Rust `String` is
    // UTF-8 by construction; assert the canonical HELP/TYPE comment lines.
    let collector = collector_with_one_counter();
    let text = render_prometheus_metrics(&collector, None);
    // Round-trips through UTF-8 without loss (String is valid UTF-8 by type).
    assert_eq!(String::from_utf8(text.clone().into_bytes()).unwrap(), text);
    assert!(text.contains("# HELP apcore_module_calls_total"));
    assert!(text.contains("# TYPE apcore_module_calls_total counter"));
}

// clause: observability.export.property.async
#[test]
fn observability_export_property_async() {
    // ### Properties: async: false. `export()` is a plain synchronous call
    // returning a concrete `String` (not a Future). If it were async this would
    // not type-check as a direct `String`.
    let collector = collector_with_one_counter();
    let result: String = render_prometheus_metrics(&collector, None);
    assert!(result.contains("apcore_module_calls_total"));
}

// clause: observability.export.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn observability_export_property_thread_safe() {
    // ### Properties: thread_safe: true. Launch N (>=8) concurrent exports over
    // a shared exporter; assert no panic and every snapshot is identical.
    use std::sync::Arc;
    let collector = Arc::new(collector_with_one_counter());

    let mut handles = Vec::new();
    for _ in 0..12 {
        let collector = Arc::clone(&collector);
        handles.push(tokio::spawn(async move {
            render_prometheus_metrics(&collector, None)
        }));
    }

    let mut results = Vec::new();
    for h in handles {
        results.push(h.await.expect("export task must not panic"));
    }

    assert_eq!(results.len(), 12);
    let first = &results[0];
    assert!(results.iter().all(|r| r == first));
    assert!(first.contains("apcore_module_calls_total"));
}

// clause: observability.export.property.pure
#[test]
fn observability_export_property_pure() {
    // ### Properties: pure: false (reads from live collector state). `export()`
    // must NOT mutate the collector (a query), yet MUST reflect subsequent
    // mutations of the live collector — proving it reads live state.
    //
    let collector = MetricsCollector::new();
    collector.increment_calls("math.add", "success");

    let before = collector.snapshot();
    let first = render_prometheus_metrics(&collector, None);
    let after = collector.snapshot();
    // export() is a query: it does not mutate collector state.
    assert_eq!(before, after);
    assert!(first.contains("math.add"));

    // Live-state coupling: a new module appears on the next export.
    collector.increment_calls("math.sub", "success");
    let second = render_prometheus_metrics(&collector, None);
    assert!(second.contains("module_id=\"math.sub\""));
    assert!(!first.contains("module_id=\"math.sub\""));
}

// clause: observability.export.property.idempotent
#[test]
fn observability_export_property_idempotent() {
    // ### Properties: idempotent: true. Two successive calls with unchanged
    // collector state MUST produce identical output and leave state identical.
    let collector = collector_with_one_counter();
    let first = render_prometheus_metrics(&collector, None);
    let second = render_prometheus_metrics(&collector, None);
    assert_eq!(first, second);
    assert!(!first.is_empty());
}
