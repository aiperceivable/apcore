// APCore Protocol — Usage exporter push trait + periodic driver.
// Spec reference: Issue #45 §3 — push-style usage export.
//
// `UsageExporter` is the language-Rust counterpart of Python's
// `UsageExporter` Protocol and TypeScript's `UsageExporter` interface. The
// concrete `PeriodicUsageExporter` polls a `UsageCollector` on a fixed
// interval and forwards a JSON snapshot of all module summaries to the
// configured exporter implementation.
//
// Behavioural contract (cross-language parity):
//   * `export(summary)` MUST be called exactly once per tick. Implementations
//     SHOULD treat individual export failures as recoverable — returning an
//     error logs a warning but does NOT abort the periodic loop.
//   * `shutdown()` is invoked at most once during `stop()` after the loop has
//     been cancelled, giving exporters a chance to flush in-flight buffers.
//   * `stop()` is idempotent — repeated calls are safe.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::errors::ModuleError;
use crate::observability::usage::UsageCollector;

/// Push-style usage exporter (#45 §3).
///
/// Implementations forward periodic JSON snapshots of usage stats to an
/// external sink (HTTP collector, OpenTelemetry exporter, custom stdout
/// formatter, …). The `summary` payload mirrors the JSON shape produced by
/// `serde_json::to_value(&UsageCollector::get_all_summaries())`.
#[async_trait]
pub trait UsageExporter: Send + Sync {
    /// Export a single summary snapshot. Errors are surfaced to callers but
    /// MUST NOT terminate the periodic driver.
    async fn export(&self, summary: &Value) -> Result<(), ModuleError>;

    /// Flush any buffered state and release resources. Called once during
    /// `PeriodicUsageExporter::stop()`.
    async fn shutdown(&self) -> Result<(), ModuleError>;
}

/// No-op exporter — useful as a default placeholder in tests and bootstrap
/// configurations.
pub struct NoopUsageExporter;

#[async_trait]
impl UsageExporter for NoopUsageExporter {
    async fn export(&self, _summary: &Value) -> Result<(), ModuleError> {
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), ModuleError> {
        Ok(())
    }
}

/// Periodic driver: spawns a background task that polls a `UsageCollector`
/// and pushes a JSON snapshot to the supplied exporter on each tick.
pub struct PeriodicUsageExporter {
    collector: Arc<UsageCollector>,
    exporter: Arc<dyn UsageExporter>,
    interval: Duration,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl PeriodicUsageExporter {
    /// Create a new periodic exporter. The driver is inert until
    /// [`Self::start`] is called.
    #[must_use]
    pub fn new(
        collector: Arc<UsageCollector>,
        exporter: Arc<dyn UsageExporter>,
        interval: Duration,
    ) -> Self {
        Self {
            collector,
            exporter,
            interval,
            handle: Mutex::new(None),
        }
    }

    /// Spawn the periodic export task. If the driver is already running this
    /// is a no-op.
    pub async fn start(&self) {
        let mut guard = self.handle.lock().await;
        if guard.is_some() {
            return;
        }
        let collector = self.collector.clone();
        let exporter = self.exporter.clone();
        let interval = self.interval;
        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            // Skip the immediate first tick so callers observe one full
            // interval between `start()` and the first export — matches
            // Python's `asyncio.sleep(interval)` loop semantics.
            ticker.tick().await;
            loop {
                ticker.tick().await;
                let summaries = collector.get_all_summaries();
                let payload = serde_json::to_value(&summaries).unwrap_or(Value::Null);
                if let Err(err) = exporter.export(&payload).await {
                    tracing::warn!(error = %err, "UsageExporter.export failed; continuing");
                }
            }
        });
        *guard = Some(handle);
    }

    /// Stop the periodic task and invoke `shutdown()` on the exporter.
    ///
    /// Idempotent: `shutdown()` fires only on the call that actually takes the
    /// running task handle. A second `stop()`, or a `stop()` on a driver that
    /// was never started, is a pure no-op and does NOT call `shutdown()` again.
    /// This matches `apcore-python`'s `PeriodicUsageExporter.stop`, which
    /// returns early when `self._task is None` ("only shutdown the exporter if
    /// it was actually started, mirroring `BatchSpanProcessor` semantics").
    pub async fn stop(&self) {
        let handle = {
            let mut guard = self.handle.lock().await;
            guard.take()
        };
        // No handle => never started, or already stopped. Returning here keeps
        // `shutdown()` exactly-once, which exporters rely on to flush buffers
        // and release resources without double-free/double-flush.
        let Some(handle) = handle else {
            return;
        };
        handle.abort();
        // Await termination; ignore the JoinError that aborting produces.
        let _ = handle.await;
        if let Err(err) = self.exporter.shutdown().await {
            tracing::warn!(error = %err, "UsageExporter.shutdown failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Counter {
        exports: AtomicUsize,
        shutdowns: AtomicUsize,
    }

    #[async_trait]
    impl UsageExporter for Counter {
        async fn export(&self, _summary: &Value) -> Result<(), ModuleError> {
            self.exports.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn shutdown(&self) -> Result<(), ModuleError> {
            self.shutdowns.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn noop_exporter_returns_ok() {
        let exporter = NoopUsageExporter;
        exporter.export(&Value::Null).await.unwrap();
        exporter.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn periodic_driver_calls_exporter() {
        let collector = Arc::new(UsageCollector::new());
        let counter = Arc::new(Counter::default());
        let exporter: Arc<dyn UsageExporter> = counter.clone();
        let driver = PeriodicUsageExporter::new(collector, exporter, Duration::from_millis(25));
        driver.start().await;
        tokio::time::sleep(Duration::from_millis(120)).await;
        driver.stop().await;
        assert!(counter.exports.load(Ordering::SeqCst) >= 2);
        assert_eq!(counter.shutdowns.load(Ordering::SeqCst), 1);
    }

    /// `stop()` twice MUST call `shutdown()` exactly once, and `stop()` on a
    /// never-started driver MUST NOT call it at all — matching apcore-python.
    #[tokio::test]
    async fn periodic_driver_double_stop_calls_shutdown_once() {
        let collector = Arc::new(UsageCollector::new());
        let counter = Arc::new(Counter::default());
        let exporter: Arc<dyn UsageExporter> = counter.clone();
        let driver = PeriodicUsageExporter::new(collector, exporter, Duration::from_millis(25));

        driver.start().await;
        driver.stop().await;
        driver.stop().await;
        driver.stop().await;
        assert_eq!(counter.shutdowns.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn periodic_driver_stop_without_start_does_not_shutdown() {
        let collector = Arc::new(UsageCollector::new());
        let counter = Arc::new(Counter::default());
        let exporter: Arc<dyn UsageExporter> = counter.clone();
        let driver = PeriodicUsageExporter::new(collector, exporter, Duration::from_millis(25));

        driver.stop().await;
        assert_eq!(counter.shutdowns.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn periodic_driver_double_start_is_idempotent() {
        let collector = Arc::new(UsageCollector::new());
        let counter = Arc::new(Counter::default());
        let exporter: Arc<dyn UsageExporter> = counter.clone();
        let driver = PeriodicUsageExporter::new(collector, exporter, Duration::from_millis(25));
        driver.start().await;
        driver.start().await; // must not spawn a second task
        driver.stop().await;
    }
}
