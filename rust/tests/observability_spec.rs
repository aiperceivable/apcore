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
//!   3. `PrometheusExporter.export`— PRESENT and fully exercised.
//!
//! Each test fn name is the clause-id flattened to snake_case and carries the
//! verbatim clause-id in a leading `// clause:` comment so cross-language diffs
//! line up row-for-row with the Python and TypeScript suites.
//!
//! These tests are READ-ONLY contract verification — they never modify src/.

use apcore::observability::{MetricsCollector, PrometheusExporter};

// ---------------------------------------------------------------------------
// Helper: build an exporter over a collector carrying a single known counter.
//
// Note (Rust-actual): `PrometheusExporter::new(collector)` takes the collector
// BY VALUE. `MetricsCollector` is `Clone` and internally `Arc`-shared, so a
// clone retained before the move is a live handle onto the same state.
// ---------------------------------------------------------------------------
fn exporter_with_one_counter() -> PrometheusExporter {
    let collector = MetricsCollector::new();
    collector.increment_calls("math.add", "success");
    PrometheusExporter::new(collector)
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
// Contract 3: PrometheusExporter.export  (PRESENT — fully exercised)
//
// Rust-actual: the `collector` from the contract's ### Inputs is supplied at
// construction (`PrometheusExporter::new(collector)`), and `export()` takes no
// arguments and returns the Prometheus text `String`.
// ===========================================================================

// clause: observability.export.input.collector.required
#[test]
fn observability_export_input_collector_required() {
    // Rust enforces the required `collector` at compile time: `new()` cannot be
    // called without it (no runtime negative path exists). Positive: a supplied
    // collector's data is what `export()` renders.
    let collector = MetricsCollector::new();
    collector.increment_calls("math.add", "success");
    let exporter = PrometheusExporter::new(collector);
    let text = exporter.export();
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
    let exporter = PrometheusExporter::new(MetricsCollector::new());
    let text = exporter.export();
    // A `String` is always returned; the three required metric names are present
    // even on a cold/empty collector (spec §1.6).
    assert!(text.contains("apcore_module_calls_total"));
}

// clause: observability.export.returns.prometheus_text
#[test]
fn observability_export_returns_prometheus_text() {
    // ### Returns: Prometheus text exposition format, UTF-8. A Rust `String` is
    // UTF-8 by construction; assert the canonical HELP/TYPE comment lines.
    let exporter = exporter_with_one_counter();
    let text = exporter.export();
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
    let exporter = exporter_with_one_counter();
    let result: String = exporter.export();
    assert!(result.contains("apcore_module_calls_total"));
}

// clause: observability.export.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn observability_export_property_thread_safe() {
    // ### Properties: thread_safe: true. Launch N (>=8) concurrent exports over
    // a shared exporter; assert no panic and every snapshot is identical.
    use std::sync::Arc;
    let exporter = Arc::new(exporter_with_one_counter());

    let mut handles = Vec::new();
    for _ in 0..12 {
        let ex = exporter.clone();
        handles.push(tokio::spawn(async move { ex.export() }));
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
    // Rust-actual: the exporter takes the collector by value, but
    // `MetricsCollector` is `Clone` + `Arc`-shared, so a retained clone is a
    // live handle onto the same internal state.
    let collector = MetricsCollector::new();
    collector.increment_calls("math.add", "success");
    let live = collector.clone();
    let exporter = PrometheusExporter::new(collector);

    let before = live.snapshot();
    let first = exporter.export();
    let after = live.snapshot();
    // export() is a query: it does not mutate collector state.
    assert_eq!(before, after);
    assert!(first.contains("math.add"));

    // Live-state coupling: a new module appears on the next export.
    live.increment_calls("math.sub", "success");
    let second = exporter.export();
    assert!(second.contains("module_id=\"math.sub\""));
    assert!(!first.contains("module_id=\"math.sub\""));
}

// clause: observability.export.property.idempotent
#[test]
fn observability_export_property_idempotent() {
    // ### Properties: idempotent: true. Two successive calls with unchanged
    // collector state MUST produce identical output and leave state identical.
    let exporter = exporter_with_one_counter();
    let first = exporter.export();
    let second = exporter.export();
    assert_eq!(first, second);
    assert!(!first.is_empty());
}
