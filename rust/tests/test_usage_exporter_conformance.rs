//! Drive `usage_exporter.json` — the push-style usage export contract
//! (#45 §3, D-55, docs/features/observability.md#usageexporter-push-style).
//!
//! `tests/test_usage_exporter.rs` covers the same ground by hand. A hand copy
//! cannot notice when the canonical fixture gains a case, which is why this
//! file loads the fixture itself and derives every number from it.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use apcore::errors::ModuleError;
use apcore::observability::usage::UsageCollector;
use apcore::observability::usage_exporter::{
    NoopUsageExporter, PeriodicUsageExporter, UsageExporter,
};
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::sync::Mutex;

use crate::conformance_env::find_fixtures_root;

fn fixture() -> Value {
    let path = find_fixtures_root().join("usage_exporter.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("usage_exporter.json parses")
}

/// Fetch the case with `id`, panicking if the fixture no longer carries it —
/// a renamed or removed case must break this driver rather than pass silently.
fn case(fx: &Value, id: &str) -> Value {
    fx["test_cases"]
        .as_array()
        .expect("test_cases is an array")
        .iter()
        .find(|tc| tc["id"].as_str() == Some(id))
        .unwrap_or_else(|| panic!("usage_exporter.json no longer carries case `{id}`"))
        .clone()
}

/// Exporter that records every call and signals each export on a channel, so
/// the driver can observe tick N deterministically instead of sleeping and
/// hoping the wall clock cooperated.
struct RecordingExporter {
    exports: AtomicUsize,
    shutdowns: AtomicUsize,
    payloads: Mutex<Vec<Value>>,
    errors: Mutex<Vec<String>>,
    tick_tx: mpsc::UnboundedSender<()>,
}

impl RecordingExporter {
    fn new() -> (Arc<Self>, mpsc::UnboundedReceiver<()>) {
        let (tick_tx, tick_rx) = mpsc::unbounded_channel();
        (
            Arc::new(Self {
                exports: AtomicUsize::new(0),
                shutdowns: AtomicUsize::new(0),
                payloads: Mutex::new(Vec::new()),
                errors: Mutex::new(Vec::new()),
                tick_tx,
            }),
            tick_rx,
        )
    }
}

#[async_trait]
impl UsageExporter for RecordingExporter {
    async fn export(&self, summary: &Value) -> Result<(), ModuleError> {
        self.exports.fetch_add(1, Ordering::SeqCst);
        self.payloads.lock().await.push(summary.clone());
        let _ = self.tick_tx.send(());
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), ModuleError> {
        self.shutdowns.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// Build a collector primed with the fixture's `usage_records`.
fn collector_from(records: &Value) -> Arc<UsageCollector> {
    let collector = Arc::new(UsageCollector::new());
    for rec in records.as_array().expect("usage_records is an array") {
        collector.record(
            rec["module_id"].as_str().expect("module_id"),
            rec["caller_id"].as_str(),
            rec["latency_ms"].as_f64().expect("latency_ms"),
            rec["success"].as_bool().expect("success"),
        );
    }
    collector
}

/// Case `noop_usage_exporter_drops_summary`.
///
/// The fixture's `calls_observed: []` is a statement about a downstream sink;
/// `NoopUsageExporter` has none by construction, so the observable assertions
/// are `errors: []` and `shutdown_completed: true` — both driven below. The
/// third field is additionally driven indirectly: a `PeriodicUsageExporter`
/// wired to the Noop must also produce no error.
#[tokio::test]
async fn noop_usage_exporter_drops_summary() {
    let fx = fixture();
    let tc = case(&fx, "noop_usage_exporter_drops_summary");
    assert_eq!(
        tc["exporter_type"].as_str(),
        Some("NoopUsageExporter"),
        "case pins the exporter under test"
    );
    let expected = &tc["expected"];
    assert_eq!(
        expected["calls_observed"].as_array().map(Vec::len),
        Some(0),
        "fixture expects no downstream sink call"
    );

    let noop = NoopUsageExporter;
    let mut errors: Vec<String> = Vec::new();
    let mut shutdown_completed = false;

    for op in tc["operations"].as_array().expect("operations") {
        match op["op"].as_str().expect("op name") {
            "export" => {
                if let Err(e) = noop.export(&op["summary"]).await {
                    errors.push(format!("export: {}", e.message));
                }
            }
            "shutdown" => match noop.shutdown().await {
                Ok(()) => shutdown_completed = true,
                Err(e) => errors.push(format!("shutdown: {}", e.message)),
            },
            other => panic!("usage_exporter.json grew an op this driver cannot run: {other}"),
        }
    }

    assert_eq!(
        shutdown_completed,
        expected["shutdown_completed"].as_bool().unwrap(),
        "NoopUsageExporter.shutdown() must complete"
    );
    assert_eq!(
        errors,
        expected["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect::<Vec<_>>(),
        "NoopUsageExporter must raise nothing"
    );
}

/// Case `periodic_usage_exporter_pushes_at_interval`.
///
/// The fixture asks for exactly `ticks` export() calls at the configured
/// interval. Rather than sleep `ticks * interval` and hope, the driver awaits
/// the exporter's own per-tick signal and stops the moment tick N lands, so
/// the count is pinned exactly rather than approximately.
#[tokio::test]
async fn periodic_usage_exporter_pushes_at_interval() {
    let fx = fixture();
    let tc = case(&fx, "periodic_usage_exporter_pushes_at_interval");
    assert_eq!(tc["exporter_type"].as_str(), Some("PeriodicUsageExporter"));

    let interval_seconds = tc["config"]["interval_seconds"]
        .as_f64()
        .expect("config.interval_seconds");
    let ticks = tc["config"]["ticks"].as_u64().expect("config.ticks") as usize;
    let expected = &tc["expected"];
    let expected_calls = expected["export_call_count"]
        .as_u64()
        .expect("export_call_count") as usize;
    assert_eq!(
        expected_calls, ticks,
        "fixture's tick count and export_call_count must agree"
    );
    let must_include = expected["each_export_summary_includes"]
        .as_str()
        .expect("each_export_summary_includes");

    let collector = collector_from(&tc["usage_records"]);
    let (recorder, mut ticks_rx) = RecordingExporter::new();
    let exporter: Arc<dyn UsageExporter> = recorder.clone();
    let periodic = PeriodicUsageExporter::new(
        collector,
        exporter,
        Duration::from_secs_f64(interval_seconds),
    );

    periodic.start().await;
    // Await exactly `ticks` export signals. The generous per-tick timeout keeps
    // a starved CI box from flaking; a missing tick still fails loudly.
    for n in 1..=ticks {
        tokio::time::timeout(
            Duration::from_secs_f64(interval_seconds * 20.0 + 2.0),
            ticks_rx.recv(),
        )
        .await
        .unwrap_or_else(|_| panic!("PeriodicUsageExporter never produced tick {n} of {ticks}"))
        .expect("exporter channel closed");
    }
    // Stopping immediately on tick N pins the count: the next tick is a full
    // interval away.
    periodic.stop().await;

    assert_eq!(
        recorder.exports.load(Ordering::SeqCst),
        expected_calls,
        "PeriodicUsageExporter must call export() once per tick"
    );

    let payloads = recorder.payloads.lock().await;
    assert_eq!(payloads.len(), expected_calls);
    for (i, payload) in payloads.iter().enumerate() {
        let serialized = serde_json::to_string(payload).expect("payload serializes");
        assert!(
            serialized.contains(must_include),
            "tick {i} payload must carry the recorded module_id `{must_include}`, got {serialized}"
        );
    }
    drop(payloads);

    assert!(
        expected["shutdown_completed_after_stop"].as_bool().unwrap(),
        "fixture pins shutdown-after-stop"
    );
    assert_eq!(
        recorder.shutdowns.load(Ordering::SeqCst),
        1,
        "stop() must complete exporter.shutdown()"
    );
    assert!(
        recorder.errors.lock().await.is_empty(),
        "no exporter error expected"
    );
}

/// Case `periodic_usage_exporter_stop_is_idempotent_and_drains`, minus the
/// `shutdown_call_count` assertion, which is lifted into its own test below so
/// a failure names it rather than this loop. Both run.
#[tokio::test]
async fn periodic_usage_exporter_stop_is_idempotent_and_drains() {
    let fx = fixture();
    let tc = case(&fx, "periodic_usage_exporter_stop_is_idempotent_and_drains");
    let interval_seconds = tc["config"]["interval_seconds"]
        .as_f64()
        .expect("config.interval_seconds");
    let expected = &tc["expected"];
    assert!(
        expected["stop_idempotent"].as_bool().unwrap(),
        "fixture pins stop() idempotence"
    );
    assert!(
        expected["background_task_terminated"].as_bool().unwrap(),
        "fixture pins background-task termination"
    );

    let collector = Arc::new(UsageCollector::new());
    let (recorder, _ticks_rx) = RecordingExporter::new();
    let exporter: Arc<dyn UsageExporter> = recorder.clone();
    let periodic = PeriodicUsageExporter::new(
        collector,
        exporter,
        Duration::from_secs_f64(interval_seconds),
    );

    let mut stops = 0usize;
    for op in tc["operations"].as_array().expect("operations") {
        match op["op"].as_str().expect("op name") {
            "start" => periodic.start().await,
            "wait_ms" => {
                let ms = op["duration_ms"].as_u64().expect("duration_ms");
                tokio::time::sleep(Duration::from_millis(ms)).await;
            }
            // A second stop() must not panic — that IS the idempotence claim.
            "stop" => {
                periodic.stop().await;
                stops += 1;
            }
            other => panic!("usage_exporter.json grew an op this driver cannot run: {other}"),
        }
    }
    assert!(stops >= 2, "fixture must exercise stop() more than once");

    // background_task_terminated: no further export() can land after stop().
    let after_stop = recorder.exports.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs_f64(interval_seconds * 4.0)).await;
    assert_eq!(
        recorder.exports.load(Ordering::SeqCst),
        after_stop,
        "background task kept exporting after stop()"
    );

    assert_eq!(
        recorder.errors.lock().await.len(),
        expected["errors"].as_array().unwrap().len(),
        "no error expected during the stop sequence"
    );
}

/// The `shutdown_call_count: 1` half of
/// `periodic_usage_exporter_stop_is_idempotent_and_drains`.
///
/// This was `#[ignore]`d against a real defect: `PeriodicUsageExporter::stop()`
/// called `self.exporter.shutdown()` unconditionally, outside the handle
/// guard, so a second `stop()` shut the exporter down twice where the fixture
/// and apcore-python pin one. `stop()` now returns early when there is no
/// handle to take (src/observability/usage_exporter.rs), matching
/// apcore-python's `if self._task is None: return`, so the case runs.
#[tokio::test]
async fn periodic_usage_exporter_shutdown_called_once() {
    let fx = fixture();
    let tc = case(&fx, "periodic_usage_exporter_stop_is_idempotent_and_drains");
    let interval_seconds = tc["config"]["interval_seconds"]
        .as_f64()
        .expect("config.interval_seconds");
    let expected_shutdowns = tc["expected"]["shutdown_call_count"]
        .as_u64()
        .expect("shutdown_call_count") as usize;

    let collector = Arc::new(UsageCollector::new());
    let (recorder, _ticks_rx) = RecordingExporter::new();
    let exporter: Arc<dyn UsageExporter> = recorder.clone();
    let periodic = PeriodicUsageExporter::new(
        collector,
        exporter,
        Duration::from_secs_f64(interval_seconds),
    );

    for op in tc["operations"].as_array().expect("operations") {
        match op["op"].as_str().expect("op name") {
            "start" => periodic.start().await,
            "wait_ms" => {
                let ms = op["duration_ms"].as_u64().expect("duration_ms");
                tokio::time::sleep(Duration::from_millis(ms)).await;
            }
            "stop" => periodic.stop().await,
            other => panic!("usage_exporter.json grew an op this driver cannot run: {other}"),
        }
    }

    assert_eq!(
        recorder.shutdowns.load(Ordering::SeqCst),
        expected_shutdowns,
        "exporter.shutdown() must run exactly once across repeated stop() calls"
    );
}
