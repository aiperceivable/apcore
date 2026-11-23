//! Cross-language conformance tests for Observability Hardening (Issue #43).
//!
//! Fixture source: apcore/conformance/fixtures/observability_hardening.json
//! Spec reference: apcore/docs/features/observability.md (## Observability Hardening)
//!
//! Each fixture case verifies one normative rule of the cross-language
//! pluggable storage, BatchSpanProcessor, min-heap ErrorHistory eviction,
//! SHA-256 error fingerprinting, RedactionConfig, and Prometheus integration.

#![allow(clippy::missing_panics_doc)]
// Fixture inputs are small u64 counts; casting to usize for indexing is intentional.
#![allow(clippy::cast_possible_truncation)]

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;

use apcore::errors::{ErrorCode, ModuleError};
use apcore::observability::{
    compute_fingerprint, normalize_message, BatchSpanProcessor, ErrorHistory, InMemoryExporter,
    MetricsCollector, PrometheusExporter, RedactionConfig, Span, SpanExporter, SpanProcessor,
};

// ---------------------------------------------------------------------------
// Fixture loading (mirrors other conformance tests)
// ---------------------------------------------------------------------------

use crate::conformance_env::find_fixtures_root;

fn load_fixture() -> Value {
    let path = find_fixtures_root().join("observability_hardening.json");
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Failed to read fixture: {}", path.display()));
    serde_json::from_str(&content).unwrap_or_else(|e| panic!("Invalid JSON: {e}"))
}

fn fixture_case<'a>(fixture: &'a Value, id: &str) -> &'a Value {
    fixture["test_cases"]
        .as_array()
        .expect("test_cases must be an array")
        .iter()
        .find(|c| c["id"].as_str() == Some(id))
        .unwrap_or_else(|| panic!("fixture case '{id}' not present"))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_iso(ts: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(ts)
        .unwrap_or_else(|e| panic!("invalid ISO timestamp '{ts}': {e}"))
        .with_timezone(&Utc)
}

/// Test span exporter that simply discards spans.
#[derive(Debug, Default)]
struct DiscardExporter;

#[async_trait]
impl SpanExporter for DiscardExporter {
    async fn export(&self, _span: &Span) -> Result<(), ModuleError> {
        Ok(())
    }
    async fn shutdown(&self) -> Result<(), ModuleError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// §1.1 — Pluggable Observability Storage
// ---------------------------------------------------------------------------

#[test]
fn case_pluggable_store_default_inmemory() {
    let fixture = load_fixture();
    let case = fixture_case(&fixture, "pluggable_store_default_inmemory");

    let history = ErrorHistory::new(50);
    let store = history.store();

    let expected = case["expected"]["store_type"].as_str().unwrap();
    assert_eq!(
        store.type_name(),
        expected,
        "default store should be InMemoryObservabilityStore"
    );
}

// ---------------------------------------------------------------------------
// §1.2 — BatchSpanProcessor
// ---------------------------------------------------------------------------

#[tokio::test]
async fn case_batch_processor_buffers_spans() {
    let fixture = load_fixture();
    let case = fixture_case(&fixture, "batch_processor_buffers_spans");

    let schedule_delay_ms = case["input"]["schedule_delay_ms"].as_u64().unwrap();
    let spans_submitted = case["input"]["spans_submitted"].as_u64().unwrap() as usize;

    // A counting exporter, not a discarding one: `spans_exported_immediately`
    // is a statement about what reached the EXPORTER, and a DiscardExporter
    // cannot answer it.
    let exporter = Arc::new(InMemoryExporter::new());
    let processor = BatchSpanProcessor::builder(exporter.clone())
        .schedule_delay_ms(schedule_delay_ms)
        .build();

    for i in 0..spans_submitted {
        let span = Span::new(format!("span-{i}"), "trace-buffer");
        processor.on_span_end(span).await;
    }

    let expected_queue = case["expected"]["queue_size"].as_u64().unwrap() as usize;
    let expected_dropped = case["expected"]["spans_dropped"].as_u64().unwrap();

    // `spans_exported_immediately` — read at `elapsed_ms_after_submit`, i.e.
    // before the schedule delay could have fired. Buffering is the whole point
    // of the batch processor; a SimpleSpanProcessor here would export 3.
    assert_eq!(
        case["input"]["elapsed_ms_after_submit"].as_u64().unwrap(),
        0,
        "this driver reads the exporter immediately after submit"
    );
    assert_eq!(
        exporter.get_spans().len() as u64,
        case["expected"]["spans_exported_immediately"]
            .as_u64()
            .unwrap(),
        "spans_exported_immediately mismatch — spans reached the exporter before \
         the {schedule_delay_ms}ms schedule delay elapsed"
    );

    assert_eq!(processor.queue_size(), expected_queue);
    assert_eq!(processor.spans_dropped(), expected_dropped);
}

#[tokio::test]
async fn case_batch_processor_drops_on_full_queue() {
    let fixture = load_fixture();
    let case = fixture_case(&fixture, "batch_processor_drops_on_full_queue");

    let max_queue_size = case["input"]["max_queue_size"].as_u64().unwrap() as usize;
    let queue_size_before = case["input"]["queue_size_before"].as_u64().unwrap() as usize;
    let new_spans_submitted = case["input"]["new_spans_submitted"].as_u64().unwrap() as usize;

    // schedule_delay long enough that the background task does not flush during the test.
    let processor = BatchSpanProcessor::builder(Arc::new(DiscardExporter))
        .max_queue_size(max_queue_size)
        .schedule_delay_ms(60_000)
        .build();

    // Fill the queue to capacity.
    for i in 0..queue_size_before {
        processor
            .on_span_end(Span::new(format!("seed-{i}"), "trace-fill"))
            .await;
    }
    assert_eq!(processor.queue_size(), max_queue_size);

    // Attempt to submit more — these MUST be dropped.
    for i in 0..new_spans_submitted {
        processor
            .on_span_end(Span::new(format!("overflow-{i}"), "trace-fill"))
            .await;
    }

    let expected_after = case["expected"]["queue_size_after"].as_u64().unwrap() as usize;
    let expected_dropped = case["expected"]["spans_dropped"].as_u64().unwrap();
    assert_eq!(processor.queue_size(), expected_after);
    assert_eq!(processor.spans_dropped(), expected_dropped);
}

// ---------------------------------------------------------------------------
// §1.3 — Min-heap ErrorHistory eviction
// ---------------------------------------------------------------------------

#[test]
fn case_error_history_evicts_oldest_first() {
    let fixture = load_fixture();
    let case = fixture_case(&fixture, "error_history_evicts_oldest_first");

    let max_total = case["input"]["max_total_entries"].as_u64().unwrap() as usize;
    let history = ErrorHistory::with_limits(max_total, max_total);

    // Seed three entries with explicit timestamps. The fixture's "code" is used
    // here as the entry's message, since ErrorCode is a closed enum and the
    // observable behavior under test is "evict the entry with the oldest
    // last_seen_at" — independent of the specific error code.
    for entry in case["input"]["existing_entries"].as_array().unwrap() {
        let module_id = entry["module_id"].as_str().unwrap();
        let code_label = entry["code"].as_str().unwrap();
        let when = parse_iso(entry["last_seen_at"].as_str().unwrap());
        let err = ModuleError::new(ErrorCode::ModuleExecuteError, code_label);
        history.record_at(module_id, &err, when);
    }

    // Add the new entry that triggers eviction.
    let new_entry = &case["input"]["new_entry"];
    let new_module = new_entry["module_id"].as_str().unwrap();
    let new_code = new_entry["code"].as_str().unwrap();
    let new_when = parse_iso(new_entry["last_seen_at"].as_str().unwrap());
    history.record_at(
        new_module,
        &ModuleError::new(ErrorCode::ModuleExecuteError, new_code),
        new_when,
    );

    let expected_total = case["expected"]["total_entries"].as_u64().unwrap() as usize;
    let expected_evicted = case["expected"]["evicted_entry_code"].as_str().unwrap();
    let expected_remaining: Vec<&str> = case["expected"]["remaining_entry_codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();

    let all = history.get_all(None);
    assert_eq!(all.len(), expected_total, "total entries after eviction");

    let remaining_codes: Vec<&str> = all.iter().map(|e| e.message.as_str()).collect();
    for code in &expected_remaining {
        assert!(
            remaining_codes.contains(code),
            "expected '{code}' to be retained, got {remaining_codes:?}"
        );
    }
    assert!(
        !remaining_codes.contains(&expected_evicted),
        "expected '{expected_evicted}' to be evicted, but it is still present"
    );
}

// ---------------------------------------------------------------------------
// §1.4 — Error fingerprinting & deduplication
// ---------------------------------------------------------------------------

#[test]
fn case_error_fingerprint_dedup_same_error() {
    let fixture = load_fixture();
    let case = fixture_case(&fixture, "error_fingerprint_dedup_same_error");

    let history = ErrorHistory::new(50);
    for record in case["input"]["records"].as_array().unwrap() {
        let module_id = record["module_id"].as_str().unwrap();
        let message = record["message"].as_str().unwrap();
        let err = ModuleError::new(ErrorCode::ModuleExecuteError, message);
        history.record(module_id, &err);
    }

    let module_id = case["input"]["records"][0]["module_id"].as_str().unwrap();
    let entries = history.get(module_id, None);

    let expected_total = case["expected"]["total_entries"].as_u64().unwrap() as usize;
    let expected_count = case["expected"]["entry_count"].as_u64().unwrap();
    assert_eq!(entries.len(), expected_total, "total deduplicated entries");
    assert_eq!(entries[0].count, expected_count, "duplicate count");
}

#[test]
fn case_error_fingerprint_normalization() {
    let fixture = load_fixture();
    let case = fixture_case(&fixture, "error_fingerprint_normalization");

    let module_id = case["input"]["module_id"].as_str().unwrap();
    let code = case["input"]["code"].as_str().unwrap();
    let messages: Vec<&str> = case["input"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();

    let normalized: Vec<String> = messages.iter().map(|m| normalize_message(m)).collect();
    let expected_normalized: Vec<&str> = case["expected"]["normalized_messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(
        normalized.iter().map(String::as_str).collect::<Vec<_>>(),
        expected_normalized,
        "normalize_message output"
    );

    let fingerprints: Vec<String> = messages
        .iter()
        .map(|m| compute_fingerprint(code, module_id, m))
        .collect();

    let expected_equal = case["expected"]["fingerprints_equal"].as_bool().unwrap();
    let all_equal = fingerprints.windows(2).all(|w| w[0] == w[1]);
    assert_eq!(
        all_equal, expected_equal,
        "fingerprint equality across normalized messages"
    );
}

#[test]
fn case_fingerprint_different_errors_no_collision() {
    let fixture = load_fixture();
    let case = fixture_case(&fixture, "fingerprint_different_errors_no_collision");

    let entries = case["input"]["entries"].as_array().unwrap();
    let fingerprints: Vec<String> = entries
        .iter()
        .map(|e| {
            compute_fingerprint(
                e["code"].as_str().unwrap(),
                e["module_id"].as_str().unwrap(),
                e["message"].as_str().unwrap(),
            )
        })
        .collect();

    let expected_equal = case["expected"]["fingerprints_equal"].as_bool().unwrap();
    let all_equal = fingerprints.windows(2).all(|w| w[0] == w[1]);
    assert_eq!(all_equal, expected_equal);
}

// ---------------------------------------------------------------------------
// §1.5 — RedactionConfig
// ---------------------------------------------------------------------------

#[test]
fn case_redaction_field_pattern_match() {
    let fixture = load_fixture();
    let case = fixture_case(&fixture, "redaction_field_pattern_match");

    let cfg = build_redaction(&case["input"]["redaction_config"]);
    let log_entry = case["input"]["log_entry"].clone();
    let mut inputs = log_entry["inputs"].clone();
    cfg.redact(&mut inputs);

    let expected_inputs = case["expected"]["logged_inputs"].clone();
    assert_eq!(inputs, expected_inputs, "redacted inputs");

    // Required correlation fields MUST remain present and unmodified on the parent.
    assert!(case["expected"]["trace_id_present"].as_bool().unwrap());
    assert!(case["expected"]["caller_id_present"].as_bool().unwrap());
    assert!(case["expected"]["module_id_present"].as_bool().unwrap());
    let mut wrapped = log_entry.clone();
    cfg.redact(&mut wrapped);
    assert_eq!(wrapped["trace_id"], log_entry["trace_id"]);
    assert_eq!(wrapped["caller_id"], log_entry["caller_id"]);
    assert_eq!(wrapped["module_id"], log_entry["module_id"]);
}

#[test]
fn case_redaction_value_pattern_match() {
    let fixture = load_fixture();
    let case = fixture_case(&fixture, "redaction_value_pattern_match");

    let cfg = build_redaction(&case["input"]["redaction_config"]);
    let log_entry = case["input"]["log_entry"].clone();
    let mut inputs = log_entry["inputs"].clone();
    cfg.redact(&mut inputs);

    let expected_inputs = case["expected"]["logged_inputs"].clone();
    assert_eq!(inputs, expected_inputs, "redacted inputs");

    let mut wrapped = log_entry.clone();
    cfg.redact(&mut wrapped);
    assert_eq!(wrapped["trace_id"], log_entry["trace_id"]);
    assert_eq!(wrapped["caller_id"], log_entry["caller_id"]);
    assert_eq!(wrapped["module_id"], log_entry["module_id"]);
}

fn build_redaction(cfg: &Value) -> RedactionConfig {
    let field_patterns: Vec<String> = cfg["field_patterns"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default();
    let value_patterns: Vec<String> = cfg["value_patterns"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default();
    let replacement = cfg["replacement"].as_str().unwrap_or("***REDACTED***");

    RedactionConfig::builder()
        .field_patterns(field_patterns)
        .value_patterns(value_patterns)
        .replacement(replacement)
        .try_build()
        .expect("valid redaction config")
}

// ---------------------------------------------------------------------------
// §1.6 — Prometheus integration hooks
// ---------------------------------------------------------------------------

#[test]
fn case_prometheus_format_includes_required_metrics() {
    let fixture = load_fixture();
    let case = fixture_case(&fixture, "prometheus_format_includes_required_metrics");

    let collector = MetricsCollector::new();

    // Seed the collector to mirror the fixture's collector_state.
    let calls_total = case["input"]["collector_state"]["apcore_module_calls_total"]
        .as_u64()
        .unwrap();
    for _ in 0..calls_total {
        collector.increment_calls("mod.demo", "success");
    }
    let errors_total = case["input"]["collector_state"]["apcore_module_errors_total"]
        .as_u64()
        .unwrap();
    for _ in 0..errors_total {
        collector.increment_errors("mod.demo", "ERR");
    }
    if let Some(observations) =
        case["input"]["collector_state"]["apcore_module_duration_seconds_observations"].as_array()
    {
        for v in observations {
            collector.observe_duration("mod.demo", v.as_f64().unwrap());
        }
    }

    let exporter = PrometheusExporter::new(collector);
    let body = exporter.export();

    for required in case["expected"]["output_contains"].as_array().unwrap() {
        let name = required.as_str().unwrap();
        assert!(
            body.contains(name),
            "Prometheus output missing required metric '{name}':\n{body}"
        );
    }
    assert_eq!(
        case["expected"]["format"].as_str().unwrap(),
        "prometheus_text"
    );
}

// ---------------------------------------------------------------------------
// Smoke test: BatchSpanProcessor shutdown drains within timeout
// ---------------------------------------------------------------------------

#[tokio::test]
async fn batch_processor_shutdown_drains_remaining_spans() {
    let exporter = InMemoryExporter::new();
    let processor = BatchSpanProcessor::builder(Arc::new(exporter.clone()))
        .schedule_delay_ms(60_000)
        .export_timeout_ms(2_000)
        .build();

    for i in 0..10 {
        processor
            .on_span_end(Span::new(format!("s-{i}"), "trace-shutdown"))
            .await;
    }
    assert_eq!(processor.queue_size(), 10);
    assert!(
        exporter.get_spans().is_empty(),
        "no spans should have been exported before shutdown"
    );

    tokio::time::timeout(
        Duration::from_secs(3),
        <BatchSpanProcessor as SpanProcessor>::shutdown(&processor),
    )
    .await
    .expect("shutdown should complete within timeout")
    .expect("shutdown returned ok");

    assert_eq!(
        exporter.get_spans().len(),
        10,
        "all 10 buffered spans must be exported during shutdown drain"
    );
}

// ---------------------------------------------------------------------------
// force_flush: drains the queue synchronously without shutting down (A-C-002)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn batch_processor_force_flush_drains_queue_synchronously() {
    let exporter = InMemoryExporter::new();
    let processor = BatchSpanProcessor::builder(Arc::new(exporter.clone()))
        .schedule_delay_ms(60_000)
        .build();

    for i in 0..3 {
        processor
            .on_span_end(Span::new(format!("s-{i}"), "trace-force-flush"))
            .await;
    }
    assert_eq!(processor.queue_size(), 3);

    let drained = processor.force_flush(2_000).await;
    assert!(drained, "force_flush should report the queue as drained");
    assert_eq!(processor.queue_size(), 0);
    assert_eq!(exporter.get_spans().len(), 3);

    // The processor stays alive after force_flush — new spans can still be enqueued.
    processor
        .on_span_end(Span::new("s-after", "trace-force-flush"))
        .await;
    assert_eq!(processor.queue_size(), 1);
}

#[tokio::test]
async fn batch_processor_force_flush_safe_after_shutdown() {
    let exporter = InMemoryExporter::new();
    let processor = BatchSpanProcessor::builder(Arc::new(exporter.clone()))
        .schedule_delay_ms(60_000)
        .build();

    <BatchSpanProcessor as SpanProcessor>::shutdown(&processor)
        .await
        .expect("shutdown returned ok");

    let drained = processor.force_flush(1_000).await;
    assert!(
        drained,
        "force_flush on an empty, shut-down queue is a no-op success"
    );
}

/// `TracingMiddleware` accepts a `BatchSpanProcessor` directly via the
/// `SpanExporter` adapter — verifies the spec's Rust example compiles and
/// routes spans through the non-blocking processor (observability.md §1.2).
#[tokio::test]
async fn tracing_middleware_accepts_batch_span_processor() {
    use apcore::observability::{SamplingStrategy, TracingMiddleware};
    let exporter = InMemoryExporter::new();
    let processor = BatchSpanProcessor::builder(Arc::new(exporter.clone()))
        .schedule_delay_ms(60_000)
        .build();
    // Compile-time check: spec example pattern.
    let _mw = TracingMiddleware::with_sampling(Box::new(processor), SamplingStrategy::Always, 1.0);
}
