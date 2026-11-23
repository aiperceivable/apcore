// APCore Protocol — Span processors (Simple + Batch)
// Spec reference: observability.md §1.2 BatchSpanProcessor for Non-Blocking OTEL Export

use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex as AsyncMutex};
use tokio::time::{interval, timeout, Duration};

use super::span::{Span, SpanExporter};
use crate::errors::ModuleError;

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
    rx: AsyncMutex<mpsc::Receiver<Span>>,
    queue_size: AtomicU64,
    spans_dropped: AtomicU64,
    config: BatchSpanProcessorConfig,
    exporter: Arc<dyn SpanExporter>,
    shutdown_tx: AsyncMutex<Option<mpsc::Sender<()>>>,
    /// Set when shutdown has been signalled (avoids double-signalling on Drop).
    shutdown_signalled: AtomicBool,
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
            rx: AsyncMutex::new(rx),
            queue_size: AtomicU64::new(0),
            spans_dropped: AtomicU64::new(0),
            config,
            exporter,
            shutdown_tx: AsyncMutex::new(Some(shutdown_tx)),
            shutdown_signalled: AtomicBool::new(false),
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
                            flush_batch(&bg_inner).await;
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
    /// calls. Safe to call on an already-shut-down processor — it only
    /// touches the queue and the exporter, neither of which `shutdown()`
    /// tears down.
    ///
    /// Mirrors apcore-python's `BatchSpanProcessor.force_flush`.
    pub async fn force_flush(&self, timeout_ms: u64) -> bool {
        let inner = self.inner.clone();
        let _ = timeout(Duration::from_millis(timeout_ms), async {
            loop {
                flush_batch(&inner).await;
                if inner.queue_size.load(Ordering::Relaxed) == 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await;
        self.inner.queue_size.load(Ordering::Relaxed) == 0
    }
}

/// Drain spans up to `max_export_batch_size` and export them.
/// Returns `true` when the receiver is permanently closed (all senders dropped),
/// which signals the bg loop that no further spans can ever arrive.
async fn flush_batch(inner: &BatchInner) -> bool {
    let mut rx = inner.rx.lock().await;
    let mut batch: Vec<Span> = Vec::with_capacity(inner.config.max_export_batch_size);
    let mut closed = false;
    while batch.len() < inner.config.max_export_batch_size {
        match rx.try_recv() {
            Ok(span) => {
                inner.queue_size.fetch_sub(1, Ordering::Relaxed);
                batch.push(span);
            }
            Err(mpsc::error::TryRecvError::Empty) => break,
            Err(mpsc::error::TryRecvError::Disconnected) => {
                closed = true;
                break;
            }
        }
    }
    drop(rx);

    for span in batch {
        if let Err(e) = inner.exporter.export(&span).await {
            tracing::warn!(error = %e.message, "BatchSpanProcessor: export failed");
        }
    }
    closed
}

#[async_trait]
impl SpanProcessor for BatchSpanProcessor {
    async fn on_span_end(&self, span: Span) {
        match self.tx.try_send(span) {
            Ok(()) => {
                self.inner.queue_size.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => {
                self.inner.spans_dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    async fn shutdown(&self) -> Result<(), ModuleError> {
        // Signal the background worker (if any) to flush and exit.
        // Idempotent: Option::take + AtomicBool guard prevent double-signalling
        // even when called concurrently with Drop.
        self.inner.shutdown_signalled.store(true, Ordering::SeqCst);
        let mut guard = self.inner.shutdown_tx.lock().await;
        if let Some(tx) = guard.take() {
            let _ = tx.send(()).await;
        }
        drop(guard);

        // Final drain up to export_timeout_ms.
        let timeout_dur = Duration::from_millis(self.inner.config.export_timeout_ms);
        let inner = self.inner.clone();
        let _ = timeout(timeout_dur, async move {
            // Drain any remaining spans the worker hasn't consumed.
            flush_batch(&inner).await;
        })
        .await;

        self.inner.exporter.shutdown().await
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
        // Avoid leaking the background task if the user drops every clone
        // without ever calling shutdown(). We can't await here, so we just
        // best-effort trigger the shutdown channel; the bg task observes it
        // on its next tick and exits. If shutdown() was already called,
        // shutdown_signalled is set and we skip.
        if self.inner.shutdown_signalled.swap(true, Ordering::SeqCst) {
            return;
        }
        // try_lock_owned is unstable; use blocking try_lock on the std-side
        // mutex equivalent: take the option via the async mutex's try_lock.
        if let Ok(mut guard) = self.inner.shutdown_tx.try_lock() {
            if let Some(tx) = guard.take() {
                let _ = tx.try_send(());
            }
        }
    }
}
