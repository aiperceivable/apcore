// APCore Protocol — Cancellation tokens
// Spec reference: Cooperative cancellation for module execution

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context as TaskContext, Poll};

use crate::observability::span::Span;

static NEXT_EXECUTION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
pub(crate) struct SpanLink {
    pub trace_id: String,
    pub span_id: String,
    pub sampled: bool,
}

#[derive(Debug, Default)]
struct ExecutionState {
    active: Mutex<HashMap<u64, ExecutionRecord>>,
    changed: tokio::sync::Notify,
    cancelled: tokio::sync::Notify,
    retirement_requested: AtomicBool,
    timeout_error: OnceLock<Box<ModuleError>>,
    middleware_timings: Mutex<HashMap<usize, std::time::Instant>>,
    tracing_spans: parking_lot::Mutex<Vec<(usize, Span, bool)>>,
    abort_requested: AtomicBool,
}

#[derive(Debug, Default)]
struct ExecutionRecord {
    abort: Option<tokio::task::AbortHandle>,
    abort_requested: bool,
}

struct ExecutionRegistration {
    id: u64,
    scopes: Vec<Arc<ExecutionState>>,
}

impl Drop for ExecutionRegistration {
    fn drop(&mut self) {
        for scope in &self.scopes {
            scope
                .active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&self.id);
            scope.changed.notify_waiters();
        }
    }
}

// Struct fields drop in declaration order: all module-owned resources must
// finish dropping before drain() can observe the registration as complete.
struct TrackedExecution<F> {
    future: Pin<Box<F>>,
    _registration: ExecutionRegistration,
}

impl<F: Future> Future for TrackedExecution<F> {
    type Output = F::Output;
    fn poll(self: Pin<&mut Self>, context: &mut TaskContext<'_>) -> Poll<Self::Output> {
        self.get_mut().future.as_mut().poll(context)
    }
}

use crate::errors::{ErrorCode, ModuleError};

/// Error raised when an execution is cancelled mid-flight.
///
/// Mirrors `apcore-python.ExecutionCancelledError(ModuleError)` and
/// `apcore-typescript ExecutionCancelledError extends ModuleError`. Carries
/// `message` (a human-readable cancellation reason) and, when the caller knew
/// it, `module_id` (the module that was running).
///
/// `module_id` is `Option` because a bare [`CancelToken::check`] has no module
/// in hand. It used to fabricate the sentinel `"@unknown"` — a string that
/// appears nowhere in `protocol-spec.md` — and write it into the error
/// `details`, so an external caller following the spec's own Rust example
/// produced a payload no other SDK produces (`check()` yields `details == {}`
/// in both peers). `check_for` still populates it.
#[derive(Debug, Clone, thiserror::Error)]
pub struct ExecutionCancelledError {
    /// ID of the module whose execution was cancelled, when known.
    pub module_id: Option<String>,
    /// Human-readable reason or description for the cancellation.
    pub message: String,
}

impl std::fmt::Display for ExecutionCancelledError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.module_id {
            Some(module_id) => write!(
                f,
                "ExecutionCancelledError: module '{}' — {}",
                module_id, self.message
            ),
            None => write!(f, "ExecutionCancelledError: {}", self.message),
        }
    }
}

impl ExecutionCancelledError {
    /// Build an `ExecutionCancelledError` with the given module ID and message.
    pub fn new(module_id: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            module_id: Some(module_id.into()),
            message: message.into(),
        }
    }

    /// Build an `ExecutionCancelledError` with no module ID — the shape a bare
    /// [`CancelToken::check`] produces, matching its Python and TypeScript
    /// counterparts, which carry no `module_id` detail either.
    pub fn without_module(message: impl Into<String>) -> Self {
        Self {
            module_id: None,
            message: message.into(),
        }
    }

    /// Convert into a generic [`ModuleError`] with code
    /// `ErrorCode::ExecutionCancelled`. Mirrors the `to_module_error()`
    /// helpers used by the other typed-error structs in `errors.rs`.
    ///
    /// `module_id` reaches `details` only when it is known; an unknown module
    /// leaves `details` empty rather than inventing a placeholder value.
    #[must_use]
    pub fn to_module_error(&self) -> ModuleError {
        let mut err = ModuleError::new(ErrorCode::ExecutionCancelled, &self.message);
        if let Some(module_id) = &self.module_id {
            err.details.insert(
                "module_id".to_string(),
                serde_json::Value::String(module_id.clone()),
            );
        }
        err
    }
}

impl From<ExecutionCancelledError> for ModuleError {
    fn from(value: ExecutionCancelledError) -> Self {
        value.to_module_error()
    }
}

/// Token used to signal cancellation to a running execution.
#[derive(Debug, Clone)]
pub struct CancelToken {
    cancelled: Arc<AtomicBool>,
    parent: Option<Arc<CancelToken>>,
    executions: Arc<ExecutionState>,
    parent_span: Option<SpanLink>,
}

impl CancelToken {
    /// Create a new cancel token in the non-cancelled state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            parent: None,
            executions: Arc::new(ExecutionState::default()),
            parent_span: None,
        }
    }

    pub(crate) fn invocation_scope(&self) -> std::sync::Weak<AtomicBool> {
        Arc::downgrade(&self.cancelled)
    }

    pub(crate) fn start_middleware_timing(&self, middleware: usize) {
        self.executions
            .middleware_timings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(middleware, std::time::Instant::now());
    }

    pub(crate) fn take_middleware_timing(&self, middleware: usize) -> Option<std::time::Instant> {
        self.executions
            .middleware_timings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&middleware)
    }

    fn active_span_link(&self, trace_id: Option<&str>) -> Option<SpanLink> {
        let spans = self.executions.tracing_spans.lock();
        let (_, span, sampled) = spans
            .iter()
            .rev()
            .find(|(_, span, _)| trace_id.is_none_or(|id| id == span.trace_id))?;
        Some(SpanLink {
            trace_id: span.trace_id.clone(),
            span_id: span.span_id.clone(),
            sampled: *sampled,
        })
    }

    pub(crate) fn span_link(&self, trace_id: &str) -> Option<SpanLink> {
        self.active_span_link(Some(trace_id)).or_else(|| {
            self.parent_span
                .as_ref()
                .filter(|span| span.trace_id == trace_id)
                .cloned()
        })
    }

    pub(crate) fn begin_span(&self, middleware: usize, span: Span, sampled: bool) {
        self.executions
            .tracing_spans
            .lock()
            .push((middleware, span, sampled));
    }

    pub(crate) fn take_span(&self, middleware: usize) -> Option<(Span, bool)> {
        let mut spans = self.executions.tracing_spans.lock();
        let index = spans.iter().rposition(|(id, _, _)| *id == middleware)?;
        let (_, span, sampled) = spans.remove(index);
        Some((span, sampled))
    }

    pub(crate) fn with_active_span(&self, trace_id: &str, apply: impl FnOnce(&mut Span)) -> bool {
        for scope in std::iter::successors(Some(self), |scope| scope.parent.as_deref()) {
            let mut spans = scope.executions.tracing_spans.lock();
            if let Some((_, span, _)) = spans
                .iter_mut()
                .rev()
                .find(|(_, span, _)| span.trace_id == trace_id)
            {
                apply(span);
                return true;
            }
        }
        false
    }

    /// Derive an independently cancellable token that observes its parent.
    #[must_use]
    pub fn child(&self) -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            parent: Some(Arc::new(self.clone())),
            executions: Arc::new(ExecutionState::default()),
            parent_span: self
                .active_span_link(None)
                .or_else(|| self.parent_span.clone()),
        }
    }

    /// Signal cancellation.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.executions.cancelled.notify_waiters();
    }

    /// Cancel the call directly admitted to a retiring registration.
    pub(crate) fn cancel_for_retirement(&self) {
        if !self.is_cancelled() {
            self.executions
                .retirement_requested
                .store(true, Ordering::Release);
            self.cancel();
        }
    }

    pub(crate) fn retirement_requested(&self) -> bool {
        self.executions.retirement_requested.load(Ordering::Acquire)
    }

    /// Publish the call's timeout before signalling its cooperative cancellation.
    pub(crate) fn cancel_after_timeout(&self, error: &ModuleError) {
        let _ = self.executions.timeout_error.set(Box::new(error.clone()));
        self.cancel();
    }

    /// Timeout belonging to this call, rather than an ancestor's cancellation.
    pub(crate) fn timeout_error(&self) -> Option<&ModuleError> {
        self.executions.timeout_error.get().map(Box::as_ref)
    }

    /// Wait until this invocation or one of its parents is cancelled.
    /// Wait for a cooperative cancellation signal from this token or its parent.
    pub async fn cancelled(&self) {
        let notifications: Vec<_> =
            std::iter::successors(Some(self), |token| token.parent.as_deref())
                .map(|token| Box::pin(token.executions.cancelled.notified()))
                .collect();
        // Register with each ancestor before rechecking the flag.
        if !self.is_cancelled() {
            futures_util::future::select_all(notifications).await;
        }
    }

    /// Wait for this invocation and its descendants to finish actual module
    /// execution, including cooperative cleanup after a reported timeout.
    /// This does not cancel or forcibly terminate any execution.
    pub async fn drain(&self) {
        loop {
            let changed = self.executions.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if !self.has_active_executions() {
                return;
            }
            changed.await;
        }
    }

    /// Whether this invocation still owns running module execution.
    #[must_use]
    pub fn has_active_executions(&self) -> bool {
        !self
            .executions
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    }

    pub(crate) fn spawn_tracked<F>(&self, future: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.spawn_tracked_on(&tokio::runtime::Handle::current(), future)
    }

    pub(crate) fn spawn_tracked_on<F>(
        &self,
        runtime: &tokio::runtime::Handle,
        future: F,
    ) -> tokio::task::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let registration = self.register_execution();
        let id = registration.id;
        let scopes = registration.scopes.clone();
        let invocation = runtime.spawn(TrackedExecution {
            future: Box::pin(future),
            _registration: registration,
        });
        Self::store_abort_handle(id, scopes, &invocation.abort_handle());
        invocation
    }

    pub(crate) fn spawn_blocking_tracked<F, Output>(
        &self,
        task: F,
    ) -> tokio::task::JoinHandle<Output>
    where
        F: FnOnce() -> Output + Send + 'static,
        Output: Send + 'static,
    {
        let registration = self.register_execution();
        let id = registration.id;
        let scopes = registration.scopes.clone();
        let invocation = tokio::task::spawn_blocking(move || {
            let _registration = registration;
            task()
        });
        Self::store_abort_handle(id, scopes, &invocation.abort_handle());
        invocation
    }

    fn register_execution(&self) -> ExecutionRegistration {
        let id = NEXT_EXECUTION.fetch_add(1, Ordering::Relaxed);
        let mut scopes = Vec::new();
        let mut current = Some(self);
        while let Some(token) = current {
            let mut active = token
                .executions
                .active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            active.insert(
                id,
                ExecutionRecord {
                    abort_requested: token.executions.abort_requested.load(Ordering::Acquire),
                    ..ExecutionRecord::default()
                },
            );
            drop(active);
            scopes.push(token.executions.clone());
            current = token.parent.as_deref();
        }
        ExecutionRegistration { id, scopes }
    }

    fn store_abort_handle(
        id: u64,
        scopes: Vec<Arc<ExecutionState>>,
        abort: &tokio::task::AbortHandle,
    ) {
        for scope in scopes {
            let mut active = scope
                .active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(record) = active.get_mut(&id) {
                record.abort = Some(abort.clone());
                if record.abort_requested {
                    abort.abort();
                }
            }
        }
    }

    /// Background task cancellation may interrupt only its own invocation tree.
    /// The public cancel() and executor deadlines remain cooperative.
    pub(crate) async fn abort_owned_executions(&self) {
        self.executions
            .abort_requested
            .store(true, Ordering::Release);
        let aborts: Vec<_> = self
            .executions
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values_mut()
            .filter_map(|record| {
                record.abort_requested = true;
                record.abort.clone()
            })
            .collect();
        for abort in aborts {
            abort.abort();
        }
        self.drain().await;
    }

    /// Check whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        let mut token = self;
        loop {
            if token.cancelled.load(Ordering::SeqCst) {
                return true;
            }
            match token.parent.as_deref() {
                Some(parent) => token = parent,
                None => return false,
            }
        }
    }

    /// Check if cancelled and return [`ExecutionCancelledError`] if so.
    ///
    /// Sync CANCEL-001 (BREAKING): the return type was previously
    /// `Result<(), ModuleError>`. The typed variant matches Python's
    /// `ExecutionCancelledError` subclass and TS's `extends ModuleError`
    /// hierarchy. Use `.into()` (or `?` against a `ModuleError` context)
    /// to widen back to `ModuleError`:
    ///
    /// ```rust,ignore
    /// fn run(token: &CancelToken) -> Result<(), ModuleError> {
    ///     token.check()?; // ExecutionCancelledError → ModuleError via From impl
    ///     Ok(())
    /// }
    /// ```
    pub fn check(&self) -> Result<(), ExecutionCancelledError> {
        if self.is_cancelled() {
            // No module is in hand here, so none is reported — see
            // `ExecutionCancelledError` for why no sentinel is substituted.
            Err(ExecutionCancelledError::without_module(
                "Execution was cancelled",
            ))
        } else {
            Ok(())
        }
    }

    /// Check if cancelled and return [`ExecutionCancelledError`] if so.
    ///
    /// The spec's canonical name for this check (`docs/features/cancellation.md`
    /// "## Contract: CancelToken.raise_if_cancelled") — identical behavior to
    /// [`Self::check`], which this crate added first and keeps: `check()` and
    /// `check_for()` are used internally throughout this crate and documented
    /// elsewhere in this file, so neither is deprecated or removed. Prefer this
    /// name when writing spec-traced or cross-language code; either name is
    /// equally correct.
    pub fn raise_if_cancelled(&self) -> Result<(), ExecutionCancelledError> {
        self.check()
    }

    /// Check with an explicit `module_id`. Returns the typed error so
    /// callers can match on cancellation specifically before widening.
    pub fn check_for(&self, module_id: &str) -> Result<(), ExecutionCancelledError> {
        if self.is_cancelled() {
            Err(ExecutionCancelledError::new(
                module_id,
                "Execution was cancelled",
            ))
        } else {
            Ok(())
        }
    }

    /// Reset the cancellation flag.
    pub fn reset(&self) {
        self.cancelled.store(false, Ordering::SeqCst);
    }
}

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod child_token_tests {
    use super::CancelToken;

    #[tokio::test]
    async fn test_drain_reaches_descendants_and_all_waiters_after_resource_drop() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        struct Cleanup(Arc<AtomicBool>);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let parent = CancelToken::new();
        let child = parent.child();
        let sibling = parent.child();
        let cleaned = Arc::new(AtomicBool::new(false));
        let cleanup = Cleanup(cleaned.clone());
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
        let invocation = child.spawn_tracked(async move {
            let _cleanup = cleanup;
            let _ = finish_rx.await;
        });
        assert!(parent.has_active_executions());
        assert!(!sibling.has_active_executions());
        let first = parent.clone();
        let second = parent.clone();
        let first_waiter = tokio::spawn(async move { first.drain().await });
        let second_waiter = tokio::spawn(async move { second.drain().await });
        finish_tx.send(()).unwrap();
        invocation.await.unwrap();
        first_waiter.await.unwrap();
        second_waiter.await.unwrap();
        assert!(cleaned.load(Ordering::Acquire));
        assert!(!parent.has_active_executions());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_tracked_execution_handles_early_finish_abort_and_panic() {
        let parent = CancelToken::new();
        for _ in 0..100 {
            let child = parent.child();
            child.spawn_tracked(async {}).await.unwrap();
            child.drain().await;
            assert!(!parent.has_active_executions());
        }
        let child = parent.child();
        let aborted = child.spawn_tracked(std::future::pending::<()>());
        child.abort_owned_executions().await;
        assert!(aborted.await.unwrap_err().is_cancelled());
        let child = parent.child();
        let panicked = child.spawn_tracked(async { panic!("tracked invocation panic") });
        assert!(panicked.await.unwrap_err().is_panic());
        parent.drain().await;
        assert!(!parent.is_cancelled());
    }

    #[test]
    fn test_child_cancellation_does_not_cancel_parent_or_sibling() {
        let parent = CancelToken::new();
        let child = parent.child();
        let sibling = parent.child();
        child.cancel();
        assert!(child.is_cancelled());
        assert!(!parent.is_cancelled());
        assert!(!sibling.is_cancelled());
    }

    #[test]
    fn test_parent_cancellation_reaches_all_descendants_and_clones() {
        let parent = CancelToken::new();
        let child = parent.child();
        let grandchild = child.child();
        let handle = parent.clone();
        handle.cancel();
        assert!(parent.is_cancelled());
        assert!(child.is_cancelled());
        assert!(grandchild.is_cancelled());
    }
}
