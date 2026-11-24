// APCore Protocol — Span processors (Simple + Batch)
// Spec reference: observability.md §1.2 BatchSpanProcessor for Non-Blocking OTEL Export

use async_trait::async_trait;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex as AsyncMutex};
use tokio::time::{interval, timeout, Duration};

use super::span::{Span, SpanExporter};
use crate::errors::{ErrorCode, ModuleError};

/// Span processor — receives finished spans and decides when/how to export them.
///
/// Implementations:
/// - [`SimpleSpanProcessor`] — synchronous, exports immediately on the calling task.
/// - [`BatchSpanProcessor`] — non-blocking, buffers spans and exports in background batches.
#[async_trait]
pub trait SpanProcessor: Send + Sync + std::fmt::Debug {
    /// Called when a span has finished. May queue, drop, or export immediately.
    async fn on_span_end(&self, span: Span);

    /// Flush + release resources. Called once when the processor is discarded.
    async fn shutdown(&self) -> Result<(), ModuleError>;
}

/// Synchronous span processor: exports each span immediately on the calling task.
///
/// Use in development and testing. Blocks the caller for the duration of `export()`.
#[derive(Debug)]
pub struct SimpleSpanProcessor {
    exporter: Arc<dyn SpanExporter>,
}

impl SimpleSpanProcessor {
    #[must_use]
    pub fn new(exporter: Arc<dyn SpanExporter>) -> Self {
        Self { exporter }
    }
}

#[async_trait]
impl SpanProcessor for SimpleSpanProcessor {
    async fn on_span_end(&self, span: Span) {
        if let Err(e) = self.exporter.export(&span).await {
            tracing::warn!(error = %e.message, "SimpleSpanProcessor: export failed");
        }
    }

    async fn shutdown(&self) -> Result<(), ModuleError> {
        self.exporter.shutdown().await
    }
}

/// Configuration for [`BatchSpanProcessor`].
#[derive(Debug, Clone, Copy)]
pub struct BatchSpanProcessorConfig {
    pub max_queue_size: usize,
    pub schedule_delay_ms: u64,
    pub max_export_batch_size: usize,
    pub export_timeout_ms: u64,
}

impl Default for BatchSpanProcessorConfig {
    fn default() -> Self {
        Self {
            max_queue_size: 2048,
            schedule_delay_ms: 5000,
            max_export_batch_size: 512,
            export_timeout_ms: 30000,
        }
    }
}

/// Builder for [`BatchSpanProcessor`].
#[derive(Debug)]
pub struct BatchSpanProcessorBuilder {
    exporter: Arc<dyn SpanExporter>,
    config: BatchSpanProcessorConfig,
}

impl BatchSpanProcessorBuilder {
    #[must_use]
    pub fn new(exporter: Arc<dyn SpanExporter>) -> Self {
        Self {
            exporter,
            config: BatchSpanProcessorConfig::default(),
        }
    }

    #[must_use]
    pub fn max_queue_size(mut self, n: usize) -> Self {
        self.config.max_queue_size = n;
        self
    }

    #[must_use]
    pub fn schedule_delay_ms(mut self, ms: u64) -> Self {
        self.config.schedule_delay_ms = ms;
        self
    }

    #[must_use]
    pub fn max_export_batch_size(mut self, n: usize) -> Self {
        self.config.max_export_batch_size = n;
        self
    }

    #[must_use]
    pub fn export_timeout_ms(mut self, ms: u64) -> Self {
        self.config.export_timeout_ms = ms;
        self
    }

    /// Build and start the background flush task.
    #[must_use]
    pub fn build(self) -> BatchSpanProcessor {
        BatchSpanProcessor::new(self.exporter, self.config)
    }
}

#[derive(Debug)]
struct BatchInner {
    buffer: AsyncMutex<ExportBuffer>,
    queue_size: AtomicU64,
    spans_dropped: AtomicU64,
    config: BatchSpanProcessorConfig,
    exporter: Arc<dyn SpanExporter>,
    // Taking the sender also closes admission to the span queue. The lock
    // orders shutdown with concurrent non-blocking enqueue operations.
    shutdown_tx: Mutex<Option<mpsc::Sender<()>>>,
    exporter_closed: AsyncMutex<bool>,
}

#[derive(Debug)]
struct ExportBuffer {
    rx: mpsc::Receiver<Span>,
    // Keep the current span in the buffer until export settles. A cancelled
    // flush can be retried without losing that span or the rest of the queue.
    exporting: Option<Span>,
}

impl BatchInner {
    fn signal_shutdown(&self) {
        if let Some(tx) = self.shutdown_tx.lock().take() {
            let _ = tx.try_send(());
        }
    }
}

/// Non-blocking span processor that buffers spans and exports in background batches.
///
/// Spans are enqueued on a bounded channel; when full, additional spans are
/// dropped and `spans_dropped` is incremented (observability.md §1.2). A
/// background task wakes on `schedule_delay_ms` and drains up to
/// `max_export_batch_size` spans per flush. `shutdown()` signals the worker
/// and waits up to `export_timeout_ms` for a final drain.
#[derive(Debug, Clone)]
pub struct BatchSpanProcessor {
    tx: mpsc::Sender<Span>,
    inner: Arc<BatchInner>,
    /// Shared by every clone and never by the worker, so its `Drop` runs when
    /// the LAST handle goes. The documented wiring hands a clone to the
    /// middleware and drops the original, and the worker must outlive that
    /// drop for as long as the middleware can still feed it.
    _last_handle: Arc<LastHandle>,
}

/// Signals the worker to flush and exit once no handle remains.
#[derive(Debug)]
struct LastHandle {
    inner: Arc<BatchInner>,
}

impl BatchSpanProcessor {
    /// Create a new processor with default configuration.
    #[must_use]
    pub fn builder(exporter: Arc<dyn SpanExporter>) -> BatchSpanProcessorBuilder {
        BatchSpanProcessorBuilder::new(exporter)
    }

    fn new(exporter: Arc<dyn SpanExporter>, config: BatchSpanProcessorConfig) -> Self {
        let (tx, rx) = mpsc::channel::<Span>(config.max_queue_size);
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        let inner = Arc::new(BatchInner {
            buffer: AsyncMutex::new(ExportBuffer {
                rx,
                exporting: None,
            }),
            queue_size: AtomicU64::new(0),
            spans_dropped: AtomicU64::new(0),
            config,
            exporter,
            shutdown_tx: Mutex::new(Some(shutdown_tx)),
            exporter_closed: AsyncMutex::new(false),
        });

        let bg_inner = inner.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let mut tick = interval(Duration::from_millis(bg_inner.config.schedule_delay_ms));
                // Skip the first immediate tick that interval emits.
                tick.tick().await;
                loop {
                    tokio::select! {
                        _ = tick.tick() => {
                            let closed = flush_batch(&bg_inner).await;
                            if closed && bg_inner.queue_size.load(Ordering::Relaxed) == 0 {
                                // Senders gone and queue is empty — exit cleanly
                                // so the task does not leak when the processor is
                                // dropped without an explicit shutdown.
                                break;
                            }
                        }
                        _ = shutdown_rx.recv() => {
                            let _ = timeout(
                                Duration::from_millis(bg_inner.config.export_timeout_ms),
                                drain_queue(&bg_inner),
                            ).await;
                            break;
                        }
                    }
                }
            });
        } else {
            tracing::warn!(
                "BatchSpanProcessor constructed outside a tokio runtime; \
                 spans will not be flushed until shutdown() is awaited inside one"
            );
        }

        let last_handle = Arc::new(LastHandle {
            inner: Arc::clone(&inner),
        });
        Self {
            tx,
            inner,
            _last_handle: last_handle,
        }
    }

    /// Number of spans currently held in the queue (best-effort, non-locking).
    #[must_use]
    pub fn queue_size(&self) -> usize {
        usize::try_from(self.inner.queue_size.load(Ordering::Relaxed)).unwrap_or(usize::MAX)
    }

    /// Number of spans dropped since construction.
    #[must_use]
    pub fn spans_dropped(&self) -> u64 {
        self.inner.spans_dropped.load(Ordering::Relaxed)
    }

    /// Maximum queue size configured at construction.
    #[must_use]
    pub fn max_queue_size(&self) -> usize {
        self.inner.config.max_queue_size
    }

    /// Synchronously (from the caller's point of view) drain the queue,
    /// exporting all currently buffered spans.
    ///
    /// Unlike [`shutdown`](SpanProcessor::shutdown), the processor stays
    /// alive after this call; new spans can continue to be enqueued.
    /// Returns `true` once the queue is empty, or `false` if `timeout_ms`
    /// elapses first. Delegates to the same [`flush_batch`] the background
    /// loop uses, repeatedly, since spans can arrive between successive
    /// calls. Once shutdown begins, reports whether its drain has completed.
    ///
    /// Mirrors apcore-python's `BatchSpanProcessor.force_flush`.
    pub async fn force_flush(&self, timeout_ms: u64) -> bool {
        if self.inner.shutdown_tx.lock().is_none() {
            return self.queue_size() == 0;
        }
        timeout(Duration::from_millis(timeout_ms), drain_queue(&self.inner))
            .await
            .is_ok()
    }
}

/// Drain spans up to `max_export_batch_size` and export them.
/// Returns `true` when the receiver is permanently closed (all senders dropped),
/// which signals the bg loop that no further spans can ever arrive.
async fn flush_batch(inner: &BatchInner) -> bool {
    let mut buffer = inner.buffer.lock().await;
    for _ in 0..inner.config.max_export_batch_size.max(1) {
        if buffer.exporting.is_none() {
            match buffer.rx.try_recv() {
                Ok(span) => buffer.exporting = Some(span),
                Err(mpsc::error::TryRecvError::Empty) => return false,
                Err(mpsc::error::TryRecvError::Disconnected) => return true,
            }
        }
        if let Some(span) = buffer.exporting.as_ref() {
            if let Err(error) = inner.exporter.export(span).await {
                tracing::warn!(error = %error.message, "BatchSpanProcessor: export failed");
            }
        }
        buffer.exporting = None;
        inner.queue_size.fetch_sub(1, Ordering::Relaxed);
    }
    false
}

async fn drain_queue(inner: &BatchInner) {
    loop {
        flush_batch(inner).await;
        if inner.queue_size.load(Ordering::Relaxed) == 0 {
            break;
        }
        tokio::task::yield_now().await;
    }
}

#[async_trait]
impl SpanProcessor for BatchSpanProcessor {
    async fn on_span_end(&self, span: Span) {
        let admission = self.inner.shutdown_tx.lock();
        if admission.is_some() {
            if let Ok(permit) = self.tx.try_reserve() {
                self.inner.queue_size.fetch_add(1, Ordering::Relaxed);
                permit.send(span);
                return;
            }
        }
        self.inner.spans_dropped.fetch_add(1, Ordering::Relaxed);
    }

    async fn shutdown(&self) -> Result<(), ModuleError> {
        let mut closed = self.inner.exporter_closed.lock().await;
        if *closed {
            return Ok(());
        }
        self.inner.signal_shutdown();
        let timeout_duration = Duration::from_millis(self.inner.config.export_timeout_ms);
        timeout(timeout_duration, drain_queue(&self.inner))
            .await
            .map_err(|_| ModuleError::new(ErrorCode::ModuleTimeout, "Timed out draining spans"))?;
        self.inner.exporter.shutdown().await?;
        *closed = true;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SpanExporter adapters
// ---------------------------------------------------------------------------
//
// `TracingMiddleware` is parameterised by `Box<dyn SpanExporter>`. Implementing
// `SpanExporter` for both processors lets users wire either one in via the same
// constructor, so the spec's Rust example
//
//   let processor = BatchSpanProcessor::builder(exporter).build();
//   let mw = TracingMiddleware::new(Box::new(processor));
//
// compiles and routes spans through the configured processor before they reach
// the exporter (observability.md §1.2 — non-blocking hot path).

#[async_trait]
impl SpanExporter for SimpleSpanProcessor {
    async fn export(&self, span: &Span) -> Result<(), ModuleError> {
        // SpanExporter has no on_span_end notion — forward the span directly.
        // This mirrors `<Self as SpanProcessor>::on_span_end`.
        self.exporter.export(span).await
    }

    async fn shutdown(&self) -> Result<(), ModuleError> {
        <Self as SpanProcessor>::shutdown(self).await
    }
}

#[async_trait]
impl SpanExporter for BatchSpanProcessor {
    /// Forward export calls to the non-blocking enqueue path.
    /// Errors are never returned: enqueue is fire-and-forget by spec; queue-full
    /// failures increment `spans_dropped` rather than surfacing a `Result::Err`.
    async fn export(&self, span: &Span) -> Result<(), ModuleError> {
        <Self as SpanProcessor>::on_span_end(self, span.clone()).await;
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), ModuleError> {
        <Self as SpanProcessor>::shutdown(self).await
    }
}

impl Drop for LastHandle {
    fn drop(&mut self) {
        self.inner.signal_shutdown();
    }
}
