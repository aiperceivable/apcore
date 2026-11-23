// TDD tests for UsageExporter trait + Noop + Periodic exporter (#45 §3).
//
// Verifies:
//   - trait surface (export + shutdown returning Result<(), ModuleError>)
//   - NoopUsageExporter is a no-op
//   - PeriodicUsageExporter pushes summaries at the configured interval
//   - shutdown stops the periodic task cleanly

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
use tokio::sync::Mutex;

/// Test exporter that records every summary it receives.
#[derive(Default)]
struct RecordingExporter {
    calls: AtomicUsize,
    shutdowns: AtomicUsize,
    payloads: Mutex<Vec<Value>>,
}

#[async_trait]
impl UsageExporter for RecordingExporter {
    async fn export(&self, summary: &Value) -> Result<(), ModuleError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.payloads.lock().await.push(summary.clone());
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), ModuleError> {
        self.shutdowns.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
async fn noop_exporter_is_a_no_op() {
    let noop = NoopUsageExporter;
    let summary = serde_json::json!({"call_count": 0});
    noop.export(&summary).await.expect("noop export ok");
    noop.shutdown().await.expect("noop shutdown ok");
}

#[tokio::test]
async fn periodic_exporter_pushes_summary_at_intervals() {
    let collector = Arc::new(UsageCollector::new());
    collector.record("executor.test", Some("api.gw"), 12.3, true);

    let recorder = Arc::new(RecordingExporter::default());
    let exporter: Arc<dyn UsageExporter> = recorder.clone();

    let periodic =
        PeriodicUsageExporter::new(collector.clone(), exporter, Duration::from_millis(40));
    periodic.start().await;

    // Allow several ticks.
    tokio::time::sleep(Duration::from_millis(180)).await;

    periodic.stop().await;

    let calls = recorder.calls.load(Ordering::SeqCst);
    assert!(
        calls >= 2,
        "expected at least 2 export calls during 180ms with 40ms interval, got {calls}"
    );
    assert!(
        recorder.shutdowns.load(Ordering::SeqCst) >= 1,
        "shutdown must be invoked during stop()"
    );

    // The pushed payload should be a JSON value (array or object) containing
    // the recorded module's stats.
    let payloads = recorder.payloads.lock().await;
    assert!(!payloads.is_empty(), "no payloads captured");
    let first = &payloads[0];
    let serialized = serde_json::to_string(first).unwrap();
    assert!(
        serialized.contains("executor.test"),
        "payload should mention recorded module_id, got: {serialized}"
    );
}

#[tokio::test]
async fn periodic_exporter_stop_is_idempotent_and_clean() {
    let collector = Arc::new(UsageCollector::new());
    let recorder = Arc::new(RecordingExporter::default());
    let exporter: Arc<dyn UsageExporter> = recorder.clone();

    let periodic = PeriodicUsageExporter::new(collector, exporter, Duration::from_millis(20));
    periodic.start().await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    periodic.stop().await;

    // A second stop must not panic.
    periodic.stop().await;
}
