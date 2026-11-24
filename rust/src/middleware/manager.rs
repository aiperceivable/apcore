// APCore Protocol — Middleware manager
// Spec reference: Middleware pipeline execution

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

use super::base::Middleware;
use crate::context::Context;
use crate::errors::{ErrorCode, ModuleError};

/// Options for registering a middleware with identity-based duplicate detection.
pub struct MiddlewareRegistration<M: Middleware + 'static> {
    /// The middleware instance to register.
    pub middleware: M,
    /// When `true`, skips the duplicate-identity warning entirely.
    /// Default: `false`.
    pub allow_duplicate: bool,
    /// Override the identity key used for duplicate detection.
    /// Default: `std::any::type_name::<M>()`.
    pub identity_key: Option<String>,
}

impl<M: Middleware + 'static> MiddlewareRegistration<M> {
    /// Create a new registration with default options (duplicate warning enabled,
    /// identity derived from the type name).
    pub fn new(middleware: M) -> Self {
        Self {
            middleware,
            allow_duplicate: false,
            identity_key: None,
        }
    }

    /// Set whether duplicate-identity warnings are suppressed.
    #[must_use]
    pub fn allow_duplicate(mut self, v: bool) -> Self {
        self.allow_duplicate = v;
        self
    }

    /// Override the identity key used for duplicate detection.
    #[must_use]
    pub fn identity_key(mut self, k: impl Into<String>) -> Self {
        self.identity_key = Some(k.into());
        self
    }
}

/// Manages an ordered pipeline of middleware.
///
/// Thread-safe: internal list is protected by a Mutex. All execution methods
/// clone the Arc pointers out of the mutex before iterating so the lock is
/// never held across `.await` points.
#[derive(Debug)]
pub struct MiddlewareManager {
    middlewares: Mutex<Vec<Arc<dyn Middleware>>>,
    /// Parallel to `middlewares`: the handle issued for each registration, so
    /// [`MiddlewareManager::remove_handle`] can remove exactly one even when
    /// several answer the same `name()`. Duplicate registration only WARNS —
    /// it never fails — so two instances sharing a name is a reachable state,
    /// and `remove(name)` drops the first in pipeline order rather than the one
    /// the caller holds (sync finding A-C-001).
    handles: Mutex<Vec<MiddlewareHandle>>,
    /// The duplicate-detection identity of each registration, parallel to
    /// `handles`, so `remove` / `remove_handle` can clear the right entry
    /// (D-114). The identity depends on an optional `identity_key` override
    /// that is an option of the call and is not recoverable from the instance.
    /// `None` for a registration made without duplicate detection.
    identities: Mutex<Vec<Option<String>>>,
    next_handle: AtomicU64,
    /// Tracks identity -> first registration location hint for duplicate detection.
    registered_identities: Mutex<HashMap<String, String>>,
}

/// The duplicate-detection half of one registration.
struct Registration {
    identity: String,
    allow_duplicate: bool,
    location: String,
}

/// Opaque token identifying one middleware registration, returned by
/// [`MiddlewareManager::add`] and consumed by
/// [`MiddlewareManager::remove_handle`].
///
/// The cross-language contract (`Contract: APCore.remove`) removes by IDENTITY:
/// apcore-python and apcore-typescript take the middleware object and compare
/// with `is` / `===`. Rust cannot take the object back — `use_middleware`
/// consumes the `Box` — so the handle is the identity the caller keeps. Mirrors
/// [`SubscriberHandle`](crate::events::SubscriberHandle), which exists for the
/// same reason on the event bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MiddlewareHandle(u64);

impl MiddlewareManager {
    /// Create a new empty middleware manager.
    #[must_use]
    pub fn new() -> Self {
        Self {
            middlewares: Mutex::new(vec![]),
            handles: Mutex::new(vec![]),
            identities: Mutex::new(vec![]),
            next_handle: AtomicU64::new(0),
            registered_identities: Mutex::new(HashMap::new()),
        }
    }

    /// Add a middleware to the pipeline.
    ///
    /// Middlewares are maintained in sorted order by priority (higher first).
    /// Among middlewares with the same priority, registration order is preserved.
    ///
    /// Returns an error if the middleware's priority exceeds 1000 (the spec-defined
    /// maximum from section 11.2).
    ///
    /// Takes `&self` — the internal list is protected by a `Mutex`, so mutation
    /// is possible through a shared reference. This allows `MiddlewareManager`
    /// to be held as `Arc<MiddlewareManager>` and mutated without `Arc::get_mut`
    /// hacks, even after the `Arc` has been cloned into pipeline contexts.
    pub fn add(&self, middleware: Box<dyn Middleware>) -> Result<MiddlewareHandle, ModuleError> {
        self.add_shared(Arc::from(middleware))
    }

    /// Add an already-shared middleware handle to the pipeline.
    ///
    /// Identical to [`Self::add`] except the caller keeps a reference. Used by
    /// [`ExtensionManager::apply`](crate::extensions::ExtensionManager::apply),
    /// which retains its registrations so a second `apply` wires the same set
    /// again (D-78).
    ///
    /// # Errors
    ///
    /// Returns [`ModuleError`] if the middleware's priority exceeds 1000.
    pub fn add_shared(
        &self,
        middleware: Arc<dyn Middleware>,
    ) -> Result<MiddlewareHandle, ModuleError> {
        self.insert(middleware, None)
    }

    /// Add a middleware, warning when one with the same `name()` is already
    /// registered — the duplicate detection apcore-python and
    /// apcore-typescript apply to every `use()`. Registration always succeeds.
    ///
    /// The identity is the middleware's `name()`: a `Box<dyn Middleware>` no
    /// longer names its concrete type. Use [`Self::add_with_opts`] with an
    /// `identity_key` to register two instances deliberately.
    ///
    /// # Errors
    ///
    /// Returns [`ModuleError`] if the middleware's priority exceeds 1000.
    #[track_caller]
    pub fn add_detecting_duplicates(
        &self,
        middleware: Box<dyn Middleware>,
    ) -> Result<MiddlewareHandle, ModuleError> {
        let identity = middleware.name().to_string();
        let registration = Registration {
            identity,
            allow_duplicate: false,
            location: std::panic::Location::caller().to_string(),
        };
        self.insert(Arc::from(middleware), Some(registration))
    }

    /// Insert in priority order, recording the registration's identity at the
    /// same position so `remove` / `remove_handle` clear the right entry.
    fn insert(
        &self,
        middleware: Arc<dyn Middleware>,
        registration: Option<Registration>,
    ) -> Result<MiddlewareHandle, ModuleError> {
        let priority = middleware.priority();
        if priority > 1000 {
            tracing::warn!(
                middleware = middleware.name(),
                priority = priority,
                "Middleware rejected: priority {} exceeds maximum 1000",
                priority,
            );
            return Err(ModuleError::new(
                ErrorCode::GeneralInvalidInput,
                format!(
                    "Middleware '{}' has priority {} which exceeds the maximum allowed value of 1000",
                    middleware.name(),
                    priority,
                ),
            ));
        }
        let identity = registration.map(|r| {
            self.note_identity(&r);
            r.identity
        });
        let mut mws = self.middlewares.lock();
        let mut handles = self.handles.lock();
        let mut identities = self.identities.lock();
        // Find the first position where existing priority is strictly less than
        // the new priority. Insert before that position to maintain stable
        // ordering (later registrations go after earlier ones at same priority).
        let pos = mws
            .iter()
            .position(|m| m.priority() < priority)
            .unwrap_or(mws.len());
        let handle = MiddlewareHandle(self.next_handle.fetch_add(1, Ordering::SeqCst));
        mws.insert(pos, middleware);
        handles.insert(pos, handle);
        identities.insert(pos, identity);
        Ok(handle)
    }

    /// Record an identity on first sight and warn on a later duplicate.
    ///
    /// A-D-020: the identity is recorded regardless of `allow_duplicate`; only
    /// the WARNING is suppressed by it, so a first registration made with
    /// `allow_duplicate(true)` still makes a later one detectable. Mirrors
    /// apcore-python (`if first_site is None: record`) and apcore-typescript
    /// (identity recorded even when allowDuplicate is true).
    fn note_identity(&self, registration: &Registration) {
        let mut ids = self.registered_identities.lock();
        match ids.get(&registration.identity) {
            Some(first_site) => {
                if !registration.allow_duplicate {
                    tracing::warn!(
                        identity = %registration.identity,
                        first_registration = %first_site,
                        duplicate_registration = %registration.location,
                        "duplicate middleware registration detected"
                    );
                }
            }
            None => {
                ids.insert(registration.identity.clone(), registration.location.clone());
            }
        }
    }

    /// Register a middleware with duplicate-detection options.
    ///
    /// If the same identity (type name or explicit key) is registered twice and
    /// `allow_duplicate` is `false`, a `tracing::warn!` is emitted. Registration
    /// always succeeds; the chain is not otherwise modified.
    ///
    /// The identity is captured generically via `std::any::type_name::<M>()` so
    /// two distinct instances of the same type are treated as duplicates by default.
    /// Use `identity_key` to distinguish them (e.g. `"auth-primary"` vs
    /// `"auth-secondary"`).
    #[track_caller]
    pub fn add_with_opts<M: Middleware + 'static>(
        &self,
        opts: MiddlewareRegistration<M>,
    ) -> Result<MiddlewareHandle, ModuleError> {
        let registration = Registration {
            identity: opts
                .identity_key
                .clone()
                .unwrap_or_else(|| std::any::type_name::<M>().to_string()),
            allow_duplicate: opts.allow_duplicate,
            location: std::panic::Location::caller().to_string(),
        };
        self.insert(Arc::new(opts.middleware), Some(registration))
    }

    /// Returns `true` if `identity` (an explicit `identity_key` or a type name)
    /// has been recorded by a prior `add_with_opts` call. Used to verify the
    /// A-D-020 contract: an identity is recorded on first sight regardless of
    /// `allow_duplicate`, so a later non-allow-duplicate registration is detected.
    #[must_use]
    pub fn identity_registered(&self, identity: &str) -> bool {
        self.registered_identities.lock().contains_key(identity)
    }

    /// Remove the first middleware whose `name()` matches `name`.
    ///
    /// **Name-based, NOT identity-based.** Unlike Python's `id()` /
    /// reference-equality removal, this method matches on the
    /// `Middleware::name()` string. Two middlewares registered with the
    /// same name **cannot be removed independently** — disambiguate them
    /// at registration time by giving each a unique name (sync finding
    /// A-D-401).
    ///
    /// Removes only the first match in pipeline order; returns `false` if
    /// no middleware with that name is registered.
    ///
    /// Takes `&self` — mutation goes through the internal `Mutex`.
    pub fn remove(&self, name: &str) -> bool {
        let mut mws = self.middlewares.lock();
        let Some(i) = mws.iter().position(|m| m.name() == name) else {
            return false;
        };
        mws.remove(i);
        self.handles.lock().remove(i);
        self.forget_identity_at(i);
        true
    }

    /// Drop the duplicate-identity entry for the registration at `index`,
    /// unless another registration still holds the same identity (D-114).
    ///
    /// The registry records the FIRST registration so a later duplicate can be
    /// traced back to it; leaving the entry behind corrupts that record in both
    /// directions. It names a registration that no longer exists, and
    /// `use` / `remove` / `use` — a legitimate swap — warns about a duplicate
    /// that is not one, which is how an operator learns to ignore the warning
    /// that will next fire for a real one.
    ///
    /// Duplicate registration warns but succeeds, so two instances sharing an
    /// identity is a reachable state: removing one must not make the survivor
    /// invisible to duplicate detection.
    fn forget_identity_at(&self, index: usize) {
        let mut identities = self.identities.lock();
        if index >= identities.len() {
            return;
        }
        let Some(identity) = identities.remove(index) else {
            return;
        };
        if !identities
            .iter()
            .any(|i| i.as_deref() == Some(identity.as_str()))
        {
            self.registered_identities.lock().remove(&identity);
        }
    }

    /// Remove exactly the middleware that [`Self::add`] returned `handle` for.
    ///
    /// Unambiguous regardless of `name()`, which is what the identity-based
    /// contract requires and what `remove(name)` cannot provide: duplicate
    /// registration warns but always succeeds, so two instances answering the
    /// same name is a reachable state and `remove` drops whichever comes first
    /// in pipeline order (sync finding A-C-001).
    ///
    /// Returns `false` when the handle names a middleware that is no longer
    /// registered — the same shape `remove` uses, and idempotent for the same
    /// reason.
    pub fn remove_handle(&self, handle: MiddlewareHandle) -> bool {
        let mut handles = self.handles.lock();
        let Some(i) = handles.iter().position(|h| *h == handle) else {
            return false;
        };
        handles.remove(i);
        self.middlewares.lock().remove(i);
        drop(handles);
        self.forget_identity_at(i);
        true
    }

    /// Return a snapshot of middleware names in pipeline order.
    pub fn snapshot(&self) -> Vec<String> {
        let mws = self.middlewares.lock();
        mws.iter().map(|m| m.name().to_string()).collect()
    }

    /// Return the first middleware in the chain whose `name()` matches, as a
    /// shared handle. Used by the extension-application path to locate an
    /// existing `TracingMiddleware` and reconfigure its exporter in place
    /// rather than appending a duplicate (sync finding A-D-18).
    pub fn find_by_name(&self, name: &str) -> Option<Arc<dyn Middleware>> {
        let mws = self.middlewares.lock();
        mws.iter().find(|m| m.name() == name).map(Arc::clone)
    }

    /// Run the before hooks for all middlewares in order.
    ///
    /// Returns the (possibly modified) input and the list of indices of
    /// middlewares that were successfully executed (used by `execute_on_error`
    /// for onion-model unwinding).
    ///
    /// If a middleware's `before()` fails, wraps the error as a
    /// `MiddlewareChainError` and returns it. The `executed` list allows
    /// callers to run `on_error` only on middlewares that actually ran.
    pub async fn execute_before(
        &self,
        module_id: &str,
        mut inputs: serde_json::Value,
        ctx: &Context<serde_json::Value>,
    ) -> Result<(serde_json::Value, Vec<usize>), ModuleError> {
        // Clone Arc pointers so the mutex is not held across .await points.
        let mws: Vec<Arc<dyn Middleware>> = {
            let guard = self.middlewares.lock();
            guard.iter().map(Arc::clone).collect()
        };
        let mut executed: Vec<usize> = Vec::new();
        for (i, mw) in mws.iter().enumerate() {
            // Push BEFORE calling before() so the failing middleware is included in
            // executed — enabling on_error self-heal (mirrors Python/TS behaviour).
            executed.push(i);
            match mw.before(module_id, inputs.clone(), ctx).await {
                Ok(Some(modified)) => {
                    inputs = modified;
                }
                Ok(None) => {
                    // No modification — keep current inputs
                }
                Err(e) => {
                    // Wrap as MiddlewareChainError but preserve the original
                    // typed error in details["inner_error"] so callers can
                    // recover it via ModuleError::unwrap_middleware_chain_error()
                    // (sync finding A-D-015 — match Python's
                    // `MiddlewareChainError.original` and TypeScript's
                    // `MiddlewareChainError.original`).
                    let mut details = std::collections::HashMap::new();
                    if let Ok(inner_json) = serde_json::to_value(&e) {
                        details.insert("inner_error".to_string(), inner_json);
                    }
                    // Carry the list of middlewares that ran (including the
                    // failing one) so the caller can run on_error over exactly
                    // those — mirrors Python's `MiddlewareChainError.executed_middlewares`
                    // and TS's `MiddlewareChainError.executedMiddlewares` (sync
                    // finding A-D-01).
                    details.insert(
                        "executed_middlewares".to_string(),
                        serde_json::json!(executed),
                    );
                    return Err(ModuleError::new(
                        ErrorCode::MiddlewareChainError,
                        e.message.clone(),
                    )
                    .with_details(details)
                    .with_cause(format!(
                        "Middleware '{}' before() failed: {}",
                        mw.name(),
                        e
                    )));
                }
            }
        }
        Ok((inputs, executed))
    }

    /// Run the after hooks for all middlewares in reverse order (onion model).
    ///
    /// If a middleware returns `Some(value)`, the output is replaced.
    /// If it returns `None`, the current output is kept unchanged.
    pub async fn execute_after(
        &self,
        module_id: &str,
        inputs: serde_json::Value,
        mut output: serde_json::Value,
        ctx: &Context<serde_json::Value>,
    ) -> Result<serde_json::Value, ModuleError> {
        // Clone Arc pointers so the mutex is not held across .await points.
        let mws: Vec<Arc<dyn Middleware>> = {
            let guard = self.middlewares.lock();
            guard.iter().map(Arc::clone).collect()
        };
        for mw in mws.iter().rev() {
            if let Some(modified) = mw
                .after(module_id, inputs.clone(), output.clone(), ctx)
                .await?
            {
                output = modified;
            } else {
                // No modification — keep current output
            }
        }
        Ok(output)
    }

    /// Run the `on_error` hooks in reverse order over the middlewares that
    /// were executed during `execute_before`.
    ///
    /// Returns `Some(recovery)` if any middleware provides a recovery value.
    /// Returns `None` if no handler recovers.
    ///
    /// Early-returns on the first recovery value — subsequent middlewares'
    /// `on_error` hooks are NOT called after recovery (mirrors Python/TS).
    pub async fn execute_on_error(
        &self,
        module_id: &str,
        inputs: serde_json::Value,
        error: &ModuleError,
        ctx: &Context<serde_json::Value>,
        executed: &[usize],
    ) -> Option<serde_json::Value> {
        // Backward-compatible variant: only returns recovery values, drops
        // any RetrySignal request. Use [`Self::execute_on_error_outcome`] to
        // observe RetrySignals from middleware (sync finding A-D-017).
        match self
            .execute_on_error_outcome(module_id, inputs, error, ctx, executed)
            .await
        {
            Some(crate::middleware::base::OnErrorOutcome::Recovery(v)) => Some(v),
            Some(crate::middleware::base::OnErrorOutcome::Retry(_)) | None => None,
        }
    }

    /// Run the `on_error_outcome` hooks in reverse order. Returns the first
    /// non-None outcome (Recovery or Retry) — Recovery becomes the call's
    /// return value; Retry tells the executor to re-run the pipeline.
    pub async fn execute_on_error_outcome(
        &self,
        module_id: &str,
        inputs: serde_json::Value,
        error: &ModuleError,
        ctx: &Context<serde_json::Value>,
        executed: &[usize],
    ) -> Option<crate::middleware::base::OnErrorOutcome> {
        // Clone Arc pointers so the mutex is not held across .await points.
        let mws: Vec<Arc<dyn Middleware>> = {
            let guard = self.middlewares.lock();
            guard.iter().map(Arc::clone).collect()
        };

        for &i in executed.iter().rev() {
            if let Some(mw) = mws.get(i) {
                match mw
                    .on_error_outcome(module_id, inputs.clone(), error, ctx)
                    .await
                {
                    Ok(Some(outcome)) => {
                        // Early-return on first non-None outcome (matches Python/TS).
                        return Some(outcome);
                    }
                    Ok(None) => {
                        // No recovery / retry from this middleware — continue
                    }
                    Err(e) => {
                        tracing::error!("Middleware '{}' on_error failed: {}", mw.name(), e);
                    }
                }
            }
        }

        None
    }
}

impl Default for MiddlewareManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Context;
    use crate::errors::ModuleError;
    use async_trait::async_trait;

    /// Simple test middleware with configurable name and priority.
    #[derive(Debug)]
    struct TestMiddleware {
        mw_name: String,
        mw_priority: u16,
    }

    impl TestMiddleware {
        fn new(name: &str, priority: u16) -> Self {
            Self {
                mw_name: name.to_string(),
                mw_priority: priority,
            }
        }

        fn boxed(name: &str, priority: u16) -> Box<Self> {
            Box::new(Self::new(name, priority))
        }
    }

    #[async_trait]
    impl Middleware for TestMiddleware {
        fn name(&self) -> &str {
            &self.mw_name
        }

        fn priority(&self) -> u16 {
            self.mw_priority
        }

        async fn before(
            &self,
            _module_id: &str,
            _inputs: serde_json::Value,
            _ctx: &Context<serde_json::Value>,
        ) -> Result<Option<serde_json::Value>, ModuleError> {
            Ok(None)
        }

        async fn after(
            &self,
            _module_id: &str,
            _inputs: serde_json::Value,
            _output: serde_json::Value,
            _ctx: &Context<serde_json::Value>,
        ) -> Result<Option<serde_json::Value>, ModuleError> {
            Ok(None)
        }

        async fn on_error(
            &self,
            _module_id: &str,
            _inputs: serde_json::Value,
            _error: &ModuleError,
            _ctx: &Context<serde_json::Value>,
        ) -> Result<Option<serde_json::Value>, ModuleError> {
            Ok(None)
        }
    }

    #[test]
    fn test_higher_priority_executes_first_in_before() {
        let mgr = MiddlewareManager::new();
        mgr.add(TestMiddleware::boxed("low", 1)).unwrap();
        mgr.add(TestMiddleware::boxed("high", 10)).unwrap();
        mgr.add(TestMiddleware::boxed("mid", 5)).unwrap();

        let names = mgr.snapshot();
        assert_eq!(names, vec!["high", "mid", "low"]);
    }

    #[test]
    fn test_equal_priority_preserves_registration_order() {
        let mgr = MiddlewareManager::new();
        mgr.add(TestMiddleware::boxed("first", 5)).unwrap();
        mgr.add(TestMiddleware::boxed("second", 5)).unwrap();
        mgr.add(TestMiddleware::boxed("third", 5)).unwrap();

        let names = mgr.snapshot();
        assert_eq!(names, vec!["first", "second", "third"]);
    }

    #[test]
    fn test_default_priority_orders_after_explicit_priority() {
        let mgr = MiddlewareManager::new();
        mgr.add(TestMiddleware::boxed("default_a", 0)).unwrap();
        mgr.add(TestMiddleware::boxed("explicit", 1)).unwrap();
        mgr.add(TestMiddleware::boxed("default_b", 0)).unwrap();

        let names = mgr.snapshot();
        assert_eq!(names, vec!["explicit", "default_a", "default_b"]);
    }

    #[test]
    fn test_snapshot_reflects_priority_sorted_order() {
        let mgr = MiddlewareManager::new();
        mgr.add(TestMiddleware::boxed("d", 0)).unwrap();
        mgr.add(TestMiddleware::boxed("a", 100)).unwrap();
        mgr.add(TestMiddleware::boxed("c", 5)).unwrap();
        mgr.add(TestMiddleware::boxed("b", 50)).unwrap();

        let names = mgr.snapshot();
        assert_eq!(names, vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn test_add_rejects_priority_above_1000() {
        let mgr = MiddlewareManager::new();
        let result = mgr.add(TestMiddleware::boxed("over_limit", 1001));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.message.contains("exceeds the maximum"),
            "Expected error about priority limit, got: {}",
            err.message,
        );
        // Pipeline should be empty — the middleware was not added.
        assert_eq!(mgr.snapshot(), Vec::<String>::new());
    }

    #[test]
    fn test_add_accepts_priority_at_1000() {
        let mgr = MiddlewareManager::new();
        mgr.add(TestMiddleware::boxed("at_limit", 1000)).unwrap();
        assert_eq!(mgr.snapshot(), vec!["at_limit"]);
    }
}
