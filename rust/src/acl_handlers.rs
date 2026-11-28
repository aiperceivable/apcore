// Built-in ACL condition handlers and handler trait.
//
// Defines the ACLConditionHandler trait, three basic handlers
// (identity_types, roles, max_call_depth), and two compound operators ($or, $not).

use async_trait::async_trait;
use parking_lot::RwLock;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, LazyLock};

use crate::context::Context;

/// The three outcomes of evaluating an ACL condition (PROTOCOL_SPEC §6.1.1,
/// spec v1.22.0, apcore#100).
///
/// A condition that **is false** and a condition that **cannot be evaluated**
/// are different outcomes, and the difference decides what a `deny` rule does.
/// Collapsing them into a `bool` is the defect §6.1.1 exists to prevent: it
/// made a `deny` rule carrying a misspelled condition key silently inert.
///
/// | Rule `effect` | `Unsatisfied` | `Unevaluable` |
/// |---|---|---|
/// | `allow` | rule does not match → continue | rule does not match → continue (MUST NOT grant) |
/// | `deny`  | rule does not match → continue | rule **takes effect** → the call is denied |
///
/// [`ConditionOutcome::Unevaluable`] means the implementation cannot answer the
/// condition **as written**. That is a principle, not a closed list; the cases
/// every implementation meets are: the condition key has no resolvable handler;
/// the handler panicked; the handler was asynchronous and could not be resolved
/// on the synchronous [`ACL::check`](crate::ACL::check) path; the value is
/// malformed for its key (`$or` that is not a list, `$not` that is not an
/// object); or `conditions` itself is not a mapping. A case outside the list is
/// classified by the principle, never defaulted to `Unsatisfied`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConditionOutcome {
    /// A registered handler ran to completion and returned `true`.
    Satisfied,
    /// A registered handler ran to completion and returned `false`. An
    /// ordinary non-match: evaluation continues with the next rule.
    Unsatisfied,
    /// No answer was obtainable at all (§6.1.1).
    Unevaluable,
}

impl ConditionOutcome {
    /// Map a handler's plain boolean answer onto the three-valued outcome.
    #[must_use]
    pub fn from_bool(value: bool) -> Self {
        if value {
            Self::Satisfied
        } else {
            Self::Unsatisfied
        }
    }

    /// `true` only for [`ConditionOutcome::Satisfied`].
    #[must_use]
    pub fn is_satisfied(self) -> bool {
        self == Self::Satisfied
    }

    /// `true` only for [`ConditionOutcome::Unevaluable`].
    #[must_use]
    pub fn is_unevaluable(self) -> bool {
        self == Self::Unevaluable
    }

    /// Three-valued (Kleene) AND, per §6.1.1's composition table:
    /// an outright `Unsatisfied` wins even against an unevaluable sibling;
    /// otherwise any `Unevaluable` child makes the conjunction unevaluable.
    #[must_use]
    pub fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::Unsatisfied, _) | (_, Self::Unsatisfied) => Self::Unsatisfied,
            (Self::Unevaluable, _) | (_, Self::Unevaluable) => Self::Unevaluable,
            (Self::Satisfied, Self::Satisfied) => Self::Satisfied,
        }
    }

    /// Three-valued (Kleene) OR, per §6.1.1's composition table:
    /// an outright `Satisfied` wins even against an unevaluable sibling;
    /// otherwise any `Unevaluable` child makes the disjunction unevaluable.
    #[must_use]
    pub fn or(self, other: Self) -> Self {
        match (self, other) {
            (Self::Satisfied, _) | (_, Self::Satisfied) => Self::Satisfied,
            (Self::Unevaluable, _) | (_, Self::Unevaluable) => Self::Unevaluable,
            (Self::Unsatisfied, Self::Unsatisfied) => Self::Unsatisfied,
        }
    }

    /// Three-valued (Kleene) NOT. `$not` of an unevaluable condition is
    /// **unevaluable**, never satisfied (§6.1.1) — negating "no answer" into
    /// "yes" would let a misspelled key inside a `$not` satisfy the very rule
    /// it was meant to gate.
    #[must_use]
    pub fn negate(self) -> Self {
        match self {
            Self::Satisfied => Self::Unsatisfied,
            Self::Unsatisfied => Self::Satisfied,
            Self::Unevaluable => Self::Unevaluable,
        }
    }
}

/// Per-call accumulation of unevaluable-condition diagnostics, keyed by the
/// offending condition key.
///
/// A `BTreeMap` rather than a `HashMap` on purpose: §6.1.1 rule 2 requires
/// `handler_error` to list every unevaluable condition **ordered
/// lexicographically by condition path**, not in evaluation order, because
/// evaluation order is not portable across SDKs — and by *path* rather than by
/// *key* because a key may occur at several positions in a nested `$or` /
/// `$not` tree, which leaves ordering by key undefined. `BTreeMap` iterates in
/// ascending key order, so the required ordering is a property of the
/// container we chose and not an accident of whatever map `serde_json`
/// happens to use. See [`join_handler_errors`].
type HandlerErrors = BTreeMap<String, String>;

/// Per-`check()` capture scope: the accumulated diagnostics, plus the
/// **condition path prefix** of the sub-tree currently being evaluated.
///
/// The prefix exists because paths (§6.1.4) are positional and the compound
/// operators recurse through the public [`ACLConditionHandler`] trait, whose
/// `evaluate_outcome(value, ctx)` has nowhere to carry one. `$or` / `$not`
/// push a segment around their recursion, so a handler that panics two levels
/// down still reports `$or[1].$not.k` rather than a bare `k`.
///
/// It lives in the *capture* scope rather than in a thread-local of its own so
/// that it inherits the scope's correctness on the async path: a tokio task
/// may migrate threads across an `.await`, so a save/restore pair around one
/// would be unsound, while the task-local travels with the task.
#[derive(Debug, Default)]
struct CaptureState {
    errors: HandlerErrors,
    path_prefix: String,
    /// The §6.1.8 governance projection of the call's arguments, when the
    /// check was entered through [`ACL::check_access`](crate::ACL::check_access)
    /// or [`ACL::async_check_access`](crate::ACL::async_check_access).
    ///
    /// It rides the capture scope rather than a pair of locals of its own for
    /// the reason the path prefix does: the scope is already correct on both
    /// evaluation paths — task-local for `async_check`, thread-local for
    /// `check` — and duplicating that machinery is how the two would drift.
    /// The [`ACLConditionHandler`] trait's `evaluate(value, ctx)` has nowhere
    /// to carry it, and widening the trait would make the projection reachable
    /// by every deployment-registered handler, which is precisely the
    /// unauditable host code §6.1.7 keeps out of a governance verdict.
    projection: Option<Arc<crate::acl::GovernanceProjection>>,
}

impl CaptureState {
    const fn new() -> Self {
        Self {
            errors: BTreeMap::new(),
            path_prefix: String::new(),
            projection: None,
        }
    }
}

/// Join accumulated diagnostics into the single `AuditEntry.handler_error`
/// string, ordered lexicographically by condition path and separated by `"; "`
/// (PROTOCOL_SPEC §6.1.1 rule 2). Returns `None` when nothing was reported.
fn join_handler_errors(errors: &HandlerErrors) -> Option<String> {
    if errors.is_empty() {
        return None;
    }
    // Explicit: collect the values in ascending-path order, then join. The sort
    // is `BTreeMap`'s ordering invariant, restated here so the rule is visible
    // at the point it is applied.
    let mut ordered: Vec<(&String, &String)> = errors.iter().collect();
    ordered.sort_by_key(|(path, _)| *path);
    Some(
        ordered
            .into_iter()
            .map(|(_, message)| message.as_str())
            .collect::<Vec<_>>()
            .join("; "),
    )
}

// Per-call slot for the latest handler error message. Mirrors Python's
// `_handler_error_var` ContextVar and TypeScript's `_lastHandlerError`
// — a handler that detects an internal failure can record it here, and
// `ACL::build_audit_entry` reads it back when emitting the audit record.
//
// Python uses a single `contextvars.ContextVar` that works on both its sync
// and async `check()` paths. Rust has no single primitive with that property,
// so we keep TWO scopes that the read/write helpers consult transparently:
//
//   * `HANDLER_ERROR` — a tokio task-local, used by the async `async_check()`
//     path. Concurrent ACL evaluations on different tokio tasks cannot see
//     each other's errors.
//   * `HANDLER_ERROR_SYNC` — a thread-local, used by the synchronous `check()`
//     path (A-D-002). Tokio task-locals are unavailable on a purely
//     synchronous call, so the async-only scope left `handler_error` always
//     null on the sync path even when a condition handler reported an error.
//
// `report_handler_error` writes to whichever scope is active; `take_*` /
// `current_*` reads mirror that. A sync scope is only "active" inside
// `with_handler_error_capture_sync`, so a stray report outside any check is a
// no-op (matching the async task-local's behavior).
tokio::task_local! {
    pub(crate) static HANDLER_ERROR: RefCell<CaptureState>;
}

thread_local! {
    // `(active_depth, state)`. The depth guards against a stray
    // `report_handler_error` leaking into an unrelated later read: writes and
    // reads only apply while a `with_handler_error_capture_sync` scope is open.
    static HANDLER_ERROR_SYNC: RefCell<(u32, CaptureState)> =
        const { RefCell::new((0, CaptureState::new())) };
}

/// Compose a condition path from the active capture scope's prefix and a local
/// segment (PROTOCOL_SPEC §6.1.4). At the root of `conditions` the prefix is
/// empty and the path is the bare key.
pub(crate) fn condition_path(local: &str) -> String {
    let prefix = current_path_prefix();
    if prefix.is_empty() {
        local.to_string()
    } else {
        format!("{prefix}.{local}")
    }
}

/// The path prefix of the sub-tree currently being evaluated.
fn current_path_prefix() -> String {
    if let Ok(prefix) = HANDLER_ERROR.try_with(|cell| cell.borrow().path_prefix.clone()) {
        return prefix;
    }
    HANDLER_ERROR_SYNC.with(|cell| cell.borrow().1.path_prefix.clone())
}

/// Replace the active path prefix, returning the previous value so a caller
/// can restore it. Used only through [`PathPrefixGuard`].
fn swap_path_prefix(next: String) -> String {
    let next_for_sync = next.clone();
    if let Ok(previous) = HANDLER_ERROR.try_with(move |cell| {
        let mut state = cell.borrow_mut();
        std::mem::replace(&mut state.path_prefix, next)
    }) {
        return previous;
    }
    HANDLER_ERROR_SYNC.with(|cell| {
        let mut state = cell.borrow_mut();
        if state.0 == 0 {
            return String::new();
        }
        std::mem::replace(&mut state.1.path_prefix, next_for_sync)
    })
}

/// Scope guard that extends the condition path prefix by one segment and
/// restores the previous prefix on drop.
///
/// A guard rather than a closure because `$or` / `$not` recurse across an
/// `.await`, and a `Drop` impl restores correctly on every exit path —
/// including a `?`, a `return`, and an unwind out of a panicking handler.
pub(crate) struct PathPrefixGuard {
    previous: String,
}

impl PathPrefixGuard {
    /// Enter `segment` relative to the current prefix. `$or[0]` under the
    /// prefix `$not` becomes `$not.$or[0]`.
    pub(crate) fn enter(segment: &str) -> Self {
        let previous = swap_path_prefix(condition_path(segment));
        Self { previous }
    }
}

impl Drop for PathPrefixGuard {
    fn drop(&mut self) {
        swap_path_prefix(std::mem::take(&mut self.previous));
    }
}

/// Record that the condition at `local` (relative to the active path prefix)
/// was **unevaluable** (PROTOCOL_SPEC §6.1.1).
///
/// Unlike [`report_handler_error`], the condition's position is supplied
/// separately so the §6.1.1 rule 2 ordering ("lexicographically by condition
/// path") is applied to the path itself rather than to a formatted message.
/// The stored message is `"{path}: {reason}"`, matching the format all three
/// SDKs emit.
pub(crate) fn report_condition_unevaluable(local: &str, reason: impl std::fmt::Display) {
    report_condition_unevaluable_at(&condition_path(local), reason);
}

/// [`report_condition_unevaluable`] for a caller that already holds the full
/// path — the §6.1.4 precheck, which walks the tree itself and never relies on
/// the evaluator's prefix.
pub(crate) fn report_condition_unevaluable_at(path: &str, reason: impl std::fmt::Display) {
    record_handler_error(path.to_string(), format!("{path}: {reason}"));
}

/// Insert one `(condition path, message)` pair into whichever capture scope is
/// active. Shared by [`report_handler_error`] and
/// [`report_condition_unevaluable_at`].
fn record_handler_error(path: String, message: String) {
    // Prefer the async task-local scope when present.
    let path_for_sync = path.clone();
    let message_for_sync = message.clone();
    if HANDLER_ERROR
        .try_with(move |cell| {
            cell.borrow_mut().errors.insert(path, message);
        })
        .is_ok()
    {
        return;
    }
    // Fall back to the synchronous thread-local scope, if one is active.
    HANDLER_ERROR_SYNC.with(|cell| {
        let mut state = cell.borrow_mut();
        if state.0 > 0 {
            state.1.errors.insert(path_for_sync, message_for_sync);
        }
    });
}

/// The condition paths reported as unevaluable in the active capture scope, in
/// ascending path order. Empty outside any scope.
///
/// Used by the ACL rule loop to name the offending paths in the §6.1.1 rule 3
/// warning, which must also carry the rule index and the rule's `effect` —
/// neither of which is known this far down the call stack.
pub(crate) fn reported_condition_paths() -> Vec<String> {
    if let Ok(paths) =
        HANDLER_ERROR.try_with(|cell| cell.borrow().errors.keys().cloned().collect::<Vec<_>>())
    {
        if !paths.is_empty() {
            return paths;
        }
    }
    HANDLER_ERROR_SYNC.with(|cell| cell.borrow().1.errors.keys().cloned().collect())
}

/// The governance projection (PROTOCOL_SPEC §6.1.8) of the call being checked,
/// or `None` when the check was entered without one.
///
/// Consults the async task-local scope first and the synchronous thread-local
/// scope second, in the same order as every other read on this scope. `None`
/// is a real answer, not a missing one: a caller with no call site (tooling
/// asking "may X reach Y?") supplies no projection, and the `arguments`
/// condition is then unevaluable rather than vacuously false.
pub(crate) fn current_governance_projection() -> Option<Arc<crate::acl::GovernanceProjection>> {
    if let Ok(projection) = HANDLER_ERROR.try_with(|cell| cell.borrow().projection.clone()) {
        return projection;
    }
    HANDLER_ERROR_SYNC.with(|cell| {
        let state = cell.borrow();
        if state.0 == 0 {
            return None;
        }
        state.1.projection.clone()
    })
}

/// Record a handler-evaluation error for the current ACL check.
///
/// Cross-language parity with apcore-python `_handler_error_var.set(...)` and
/// apcore-typescript `_lastHandlerError = ...`. Writes to the active async
/// task-local scope if one exists, otherwise to the active synchronous
/// thread-local scope. If called outside any active capture scope (i.e.
/// outside an ACL check entirely), the call is a no-op so handlers never panic
/// on a missing scope.
/// Messages are accumulated, not overwritten: §6.1.1 rule 2 requires
/// `handler_error` to report **every** unevaluable condition in one `check()`,
/// ordered lexicographically by condition **path**. This free-form entry point
/// has no separate path argument, so the sort key is taken from the
/// conventional `"{path}: {reason}"` message prefix (everything before the
/// first `':'`), falling back to the whole message when it carries no colon.
/// SDK-internal call sites use [`report_condition_unevaluable`], which knows
/// the condition's position in the tree.
pub fn report_handler_error(message: impl Into<String>) {
    let msg = message.into();
    let path = msg
        .split_once(':')
        .map_or_else(|| msg.clone(), |(prefix, _)| prefix.trim().to_string());
    record_handler_error(path, msg);
}

/// Read the handler error recorded for the current ACL check, if any.
///
/// Checks the async task-local scope first, then the synchronous thread-local
/// scope. Returns `None` outside any active capture scope. Used by
/// `ACL::build_audit_entry` so a handler error populates the audit entry on
/// both the sync and async paths (A-D-002).
pub(crate) fn current_handler_error() -> Option<String> {
    if let Ok(v) = HANDLER_ERROR.try_with(|cell| join_handler_errors(&cell.borrow().errors)) {
        if v.is_some() {
            return v;
        }
    }
    HANDLER_ERROR_SYNC.with(|cell| join_handler_errors(&cell.borrow().1.errors))
}

/// Run an async evaluation under a fresh handler-error capture scope.
///
/// The scope's final `Option<String>` is returned alongside the evaluation
/// result so callers can attach it to an `AuditEntry`. Mirrors the Python
/// `with _handler_error_var.set(None)` context-manager pattern.
pub async fn with_handler_error_capture<F, T>(fut: F) -> (T, Option<String>)
where
    F: std::future::Future<Output = T>,
{
    with_acl_evaluation_scope(None, fut).await
}

/// [`with_handler_error_capture`], additionally carrying the §6.1.8 governance
/// projection for the built-in `arguments` condition to read.
pub(crate) async fn with_acl_evaluation_scope<F, T>(
    projection: Option<Arc<crate::acl::GovernanceProjection>>,
    fut: F,
) -> (T, Option<String>)
where
    F: std::future::Future<Output = T>,
{
    let cell = RefCell::new(CaptureState {
        projection,
        ..CaptureState::new()
    });
    let result = HANDLER_ERROR.scope(cell, async move {
        let value = fut.await;
        let captured = HANDLER_ERROR.with(|c| join_handler_errors(&c.borrow().errors));
        (value, captured)
    });
    result.await
}

/// Run a synchronous evaluation under a fresh handler-error capture scope.
///
/// Synchronous analogue of [`with_handler_error_capture`] for the sync
/// `ACL::check()` path (A-D-002). Opens a thread-local scope for the duration
/// of `f`, restoring the previous slot on exit so nested checks are isolated.
/// Mirrors Python's `_handler_error_var.set(None)` / `.reset(token)` pairing.
pub fn with_handler_error_capture_sync<F, T>(f: F) -> (T, Option<String>)
where
    F: FnOnce() -> T,
{
    with_acl_evaluation_scope_sync(None, f)
}

/// [`with_handler_error_capture_sync`], additionally carrying the §6.1.8
/// governance projection for the built-in `arguments` condition to read.
pub(crate) fn with_acl_evaluation_scope_sync<F, T>(
    projection: Option<Arc<crate::acl::GovernanceProjection>>,
    f: F,
) -> (T, Option<String>)
where
    F: FnOnce() -> T,
{
    // Open a fresh slot, saving the previous one so a nested check restores it.
    let previous = HANDLER_ERROR_SYNC.with(|cell| {
        let mut state = cell.borrow_mut();
        state.0 += 1;
        let previous = std::mem::take(&mut state.1);
        state.1.projection = projection;
        previous
    });

    let value = f();

    let captured = HANDLER_ERROR_SYNC.with(|cell| {
        let mut state = cell.borrow_mut();
        let captured = join_handler_errors(&state.1.errors);
        state.0 -= 1;
        // Restore the enclosing scope's state (empty at the outermost level).
        state.1 = previous;
        captured
    });

    (value, captured)
}

/// Extract a human-readable message from a panic payload.
///
/// Shared by the sync and async ACL condition-evaluation paths (A-D-011) so a
/// panicking custom handler produces a consistent `handler_error` string.
pub(crate) fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

/// Trait for evaluating a single ACL condition.
#[async_trait]
pub trait ACLConditionHandler: Send + Sync {
    /// Answer the condition. `true` = satisfied, `false` = **unsatisfied**.
    ///
    /// A handler cannot report "unevaluable" through this method, and does not
    /// need to: PROTOCOL_SPEC §6.1.1's three unevaluable situations are all
    /// detected by the evaluator around the handler (no registration, a panic,
    /// or a future that is not ready on the synchronous path). Returning
    /// `false` therefore always means an ordinary non-match.
    async fn evaluate(&self, value: &Value, ctx: &Context<Value>) -> bool;

    /// Three-valued evaluation (PROTOCOL_SPEC §6.1.1).
    ///
    /// The default implementation maps [`Self::evaluate`] onto
    /// `Satisfied` / `Unsatisfied`, which is correct for every leaf handler.
    /// Only the compound operators `$or` and `$not` override it, because they
    /// have to *propagate* an unevaluable sub-condition rather than collapse
    /// it — `$not` of `Unevaluable` is `Unevaluable`, never `Satisfied`.
    ///
    /// Overriding is optional and additive: a handler written before spec
    /// v1.22.0 keeps working unchanged.
    async fn evaluate_outcome(&self, value: &Value, ctx: &Context<Value>) -> ConditionOutcome {
        ConditionOutcome::from_bool(self.evaluate(value, ctx).await)
    }
}

/// Global registry of condition handlers (sync entry point — consulted by
/// `ACL::check` and as the fallback for `async_check`).
pub static CONDITION_HANDLERS: LazyLock<RwLock<HashMap<String, Arc<dyn ACLConditionHandler>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Separate registry for handlers explicitly registered as async-only via
/// `ACL::register_async_condition`. `async_check` consults this map first
/// and falls back to [`CONDITION_HANDLERS`] when no async-specific handler
/// is registered for a key. Mirrors apcore-python `_async_condition_handlers`
/// and apcore-typescript `_asyncConditionHandlers` (closes A-D-ACL-002).
pub static ASYNC_CONDITION_HANDLERS: LazyLock<
    RwLock<HashMap<String, Arc<dyn ACLConditionHandler>>>,
> = LazyLock::new(|| RwLock::new(HashMap::new()));

/// Register a condition handler globally. Replaces any existing handler for the same key.
///
/// An explicit registration always wins over the built-in for that key, in
/// either order: built-ins are seeded only where the key is unclaimed (see
/// `register_builtin_handlers`).
///
/// See also [`ACL::register_condition`](crate::ACL::register_condition) for the convenience
/// static method on the ACL type, which delegates here.
pub fn register_condition(key: impl Into<String>, handler: Arc<dyn ACLConditionHandler>) {
    let mut map = CONDITION_HANDLERS.write();
    map.insert(key.into(), handler);
}

/// Register an async-only condition handler.
///
/// Handlers registered here are consulted by [`evaluate_conditions_async`]
/// *before* the sync registry, allowing async-only logic to override a sync
/// handler with the same key without affecting the sync `ACL::check` path.
/// Cross-language parity with apcore-python `register_async_condition` and
/// apcore-typescript `registerAsyncCondition` (closes A-D-ACL-002).
pub fn register_async_condition(key: impl Into<String>, handler: Arc<dyn ACLConditionHandler>) {
    let mut map = ASYNC_CONDITION_HANDLERS.write();
    map.insert(key.into(), handler);
}

/// Whether `key` is **resolvable on the synchronous**
/// [`ACL::check`](crate::ACL::check) path — i.e. whether it is present in
/// [`CONDITION_HANDLERS`].
///
/// PROTOCOL_SPEC §6.1.3 keeps the two registries deliberately separate: a key
/// registered only through [`register_async_condition`] is a working condition
/// under `async_check()` and an **unevaluable** one under `check()`.
#[must_use]
pub fn is_sync_resolvable(key: &str) -> bool {
    CONDITION_HANDLERS.read().contains_key(key)
}

/// Whether `key` is **resolvable on the asynchronous**
/// [`ACL::async_check`](crate::ACL::async_check) path.
///
/// `async_check()` consults [`ASYNC_CONDITION_HANDLERS`] first and falls back
/// to [`CONDITION_HANDLERS`], so this is the **union** of the two registries
/// and a sync-only registration is `async_resolvable` (PROTOCOL_SPEC §6.1.3
/// rule 2). The name says *resolvable* rather than *registered* for exactly
/// that reason: `async_registered` would read as a lookup in the async
/// registry and be false for every built-in leaf handler, all of which resolve
/// on both paths.
#[must_use]
pub fn is_async_resolvable(key: &str) -> bool {
    let has_async_handler = ASYNC_CONDITION_HANDLERS.read().contains_key(key);
    has_async_handler || is_sync_resolvable(key)
}

// ---------------------------------------------------------------------------
// Free async evaluator (used by compound operators and re-exported to ACL)
// ---------------------------------------------------------------------------

/// One condition key paired with the handler it resolved to (if any) and its
/// configured value. `None` in the middle slot is PROTOCOL_SPEC §6.1.1
/// case 1 — no resolvable handler, i.e. an unevaluable condition.
type ResolvedCondition = (String, Option<Arc<dyn ACLConditionHandler>>, Value);

/// Evaluate all conditions with AND logic using the handler registry.
///
/// Boolean façade over [`evaluate_conditions_async_outcome`], kept for callers
/// that only care whether the conditions were satisfied. An
/// [`ConditionOutcome::Unevaluable`] result maps to `false` here, which is
/// exactly the collapse PROTOCOL_SPEC §6.1.1 forbids the **rule loop** from
/// making — use the `_outcome` form anywhere the rule's `effect` matters.
pub async fn evaluate_conditions_async<S: ::std::hash::BuildHasher>(
    conditions: &HashMap<String, Value, S>,
    ctx: &Context<Value>,
) -> bool {
    evaluate_conditions_async_outcome(conditions, ctx)
        .await
        .is_satisfied()
}

/// Evaluate all conditions with three-valued AND logic (PROTOCOL_SPEC §6.1.1).
///
/// Resolves each condition key by consulting [`ASYNC_CONDITION_HANDLERS`]
/// first (async-only overrides), then falling back to [`CONDITION_HANDLERS`]
/// (the sync registry). A key with no handler in either registry is
/// **unevaluable**, not unsatisfied; so is a handler that panics. Handlers are
/// cloned out of the registries one lock at a time, before any `.await`.
///
/// **Every** child is evaluated — no short-circuit. §6.1.1 permits
/// short-circuiting AND on the first `Unsatisfied`, but explicitly allows an
/// implementation to evaluate every child instead "for deterministic
/// diagnostics", and that is what we do: `handler_error` must list *all*
/// unevaluable conditions (rule 2), and a key skipped by a short-circuit would
/// make the audit entry depend on map iteration order. The decision is
/// identical either way — an outright `Unsatisfied` still wins the conjunction.
pub async fn evaluate_conditions_async_outcome<S: ::std::hash::BuildHasher>(
    conditions: &HashMap<String, Value, S>,
    ctx: &Context<Value>,
) -> ConditionOutcome {
    use futures_util::FutureExt;

    // Resolve handlers first; an unresolved key is a leaf outcome of its own.
    // `None` in the middle slot is PROTOCOL_SPEC §6.1.1 case 1.
    let mut to_evaluate: Vec<ResolvedCondition> = Vec::with_capacity(conditions.len());
    {
        let async_handlers = ASYNC_CONDITION_HANDLERS.read();
        for (key, value) in conditions {
            let handler = async_handlers.get(key.as_str()).cloned();
            to_evaluate.push((key.clone(), handler, value.clone()));
        }
    }
    if to_evaluate.iter().any(|(_, handler, _)| handler.is_none()) {
        // A queued writer can block new readers. Release the async read
        // guard before acquiring the sync lock to avoid cyclic waits.
        let sync_handlers = CONDITION_HANDLERS.read();
        for (key, handler, _) in &mut to_evaluate {
            if handler.is_none() {
                *handler = sync_handlers.get(key.as_str()).cloned();
            }
        }
    }

    // AND over three-valued children, starting from the vacuous truth of an
    // empty `conditions` object.
    let mut outcome = ConditionOutcome::Satisfied;
    for (key, handler, value) in &to_evaluate {
        let Some(handler) = handler else {
            // §6.1.1 case 1: no resolvable handler.
            tracing::warn!(
                condition = %key,
                "Unknown ACL condition — unevaluable (PROTOCOL_SPEC §6.1.1)"
            );
            report_condition_unevaluable(key, "unknown ACL condition");
            outcome = outcome.and(ConditionOutcome::Unevaluable);
            continue;
        };
        // §6.1.1 case 2 (SECURITY, A-D-011): a panicking custom handler
        // must NOT unwind out of the ACL gate. Catch the panic, record it, and
        // report the condition unevaluable. Mirrors Python `try/except` and
        // TypeScript `try/catch` around handler.evaluate.
        let fut = std::panic::AssertUnwindSafe(handler.evaluate_outcome(value, ctx)).catch_unwind();
        let child = match fut.await {
            Ok(child) => child,
            Err(payload) => {
                let msg = panic_message(payload.as_ref());
                tracing::error!(
                    condition = %key,
                    panic = %msg,
                    "ACL condition handler panicked — unevaluable (PROTOCOL_SPEC §6.1.1)"
                );
                report_condition_unevaluable(key, format!("handler panicked: {msg}"));
                ConditionOutcome::Unevaluable
            }
        };
        outcome = outcome.and(child);
    }
    outcome
}

// ---------------------------------------------------------------------------
// Basic handlers
// ---------------------------------------------------------------------------

/// Check context.identity.type is in the allowed list.
pub struct IdentityTypesHandler;

#[async_trait]
impl ACLConditionHandler for IdentityTypesHandler {
    async fn evaluate(&self, value: &Value, ctx: &Context<Value>) -> bool {
        let Some(arr) = value.as_array() else {
            return false;
        };
        let Some(identity) = &ctx.identity else {
            return false;
        };
        arr.iter()
            .any(|v| v.as_str().is_some_and(|s| s == identity.identity_type()))
    }
}

/// Check at least one role overlaps between identity and required roles.
pub struct RolesHandler;

#[async_trait]
impl ACLConditionHandler for RolesHandler {
    async fn evaluate(&self, value: &Value, ctx: &Context<Value>) -> bool {
        let Some(arr) = value.as_array() else {
            return false;
        };
        let Some(identity) = &ctx.identity else {
            return false;
        };
        arr.iter().any(|v| {
            v.as_str()
                .is_some_and(|s| identity.roles().contains(&s.to_string()))
        })
    }
}

struct ScopesHandler;

#[async_trait]
impl ACLConditionHandler for ScopesHandler {
    async fn evaluate(&self, value: &Value, ctx: &Context<Value>) -> bool {
        scopes_match(value, ctx)
    }
}

fn scopes_match(value: &Value, ctx: &Context<Value>) -> bool {
    let Some(scopes) = ctx.identity.as_ref().and_then(crate::Identity::scopes) else {
        return false;
    };
    value.as_array().is_some_and(|required| {
        !required.is_empty()
            && required.iter().all(|value| {
                value.as_str().is_some_and(|scope| {
                    crate::context::valid_scope_literal(scope)
                        && scopes.iter().any(|held| held == scope)
                })
            })
    })
}

pub(crate) fn has_stock_scope_grant(
    conditions: Option<&Value>,
    ctx: &Context<Value>,
    path: PrecheckPath,
) -> bool {
    let Some(required) = conditions.and_then(|conditions| conditions.get("scopes")) else {
        return false;
    };
    let stock = match path {
        PrecheckPath::Sync => CONDITION_HANDLERS
            .read()
            .get("scopes")
            .is_some_and(|handler| Arc::ptr_eq(handler, &DISCOVERY_SCOPES)),
        PrecheckPath::Async => stock_discovery_handler("scopes").is_some(),
    };
    stock && scopes_match(required, ctx)
}

struct SubjectTypesHandler;

#[async_trait]
impl ACLConditionHandler for SubjectTypesHandler {
    async fn evaluate(&self, value: &Value, ctx: &Context<Value>) -> bool {
        ctx.identity
            .as_ref()
            .zip(value.as_array())
            .is_some_and(|(identity, types)| {
                let subject = identity.root_subject();
                types
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|kind| kind == subject.identity_type())
            })
    }
}

struct SubjectRolesHandler;

#[async_trait]
impl ACLConditionHandler for SubjectRolesHandler {
    async fn evaluate(&self, value: &Value, ctx: &Context<Value>) -> bool {
        ctx.identity
            .as_ref()
            .zip(value.as_array())
            .is_some_and(|(identity, roles)| {
                let subject = identity.root_subject();
                roles
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|role| subject.roles().iter().any(|held| held == role))
            })
    }
}

struct MaxDelegationDepthHandler;

#[async_trait]
impl ACLConditionHandler for MaxDelegationDepthHandler {
    async fn evaluate(&self, value: &Value, ctx: &Context<Value>) -> bool {
        ctx.identity
            .as_ref()
            .zip(delegation_threshold(value))
            .is_some_and(|(identity, max)| identity.delegation_depth() as u64 <= max)
    }
}

fn delegation_threshold(value: &Value) -> Option<u64> {
    integral_threshold(value)
        .filter(|threshold| *threshold <= crate::context::MAX_DELEGATION_DEPTH as u64)
}

fn identity_condition_fault(key: &str, value: &Value) -> Option<&'static str> {
    match key {
        "scopes" | "subject_roles" | "subject_types" => {
            let Some(items) = value.as_array().filter(|items| {
                !items.is_empty()
                    && items
                        .iter()
                        .all(|item| item.as_str().is_some_and(|item| !item.is_empty()))
            }) else {
                return Some("value must be a nonempty list of nonempty strings");
            };
            (key == "scopes"
                && !items
                    .iter()
                    .filter_map(Value::as_str)
                    .all(crate::context::valid_scope_literal))
            .then_some("scope strings must not contain whitespace")
        }
        "max_delegation_depth" if delegation_threshold(value).is_none() => {
            Some("value must be an integer from zero to eight")
        }
        _ => None,
    }
}

/// Extract a non-negative integer threshold from a JSON number, accepting
/// integral floats (`5.0` → `5`) but rejecting non-integral floats (`5.5`).
///
/// A-D-005: YAML/JSON configs frequently parse a bare `5` as a float (`5.0`).
/// `as_u64()` returns `None` for any float, which previously caused the handler
/// to fail-closed and reject a legitimate integral threshold. We now accept any
/// number whose value is a non-negative integer, matching apcore-typescript.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn integral_threshold(value: &Value) -> Option<u64> {
    let n = value.as_number()?;
    if let Some(u) = n.as_u64() {
        return Some(u);
    }
    let f = n.as_f64()?;
    // The `fract`/sign/range guards make the cast exact for accepted values;
    // precision loss only affects the `u64::MAX` bound comparison (harmless —
    // out-of-range values are rejected).
    if f.is_finite() && f.fract() == 0.0 && f >= 0.0 && f <= u64::MAX as f64 {
        return Some(f as u64);
    }
    None
}

/// Check call chain length does not exceed threshold.
///
/// Accepts both the bare-integer form `max_call_depth: 5` and the dict form
/// `max_call_depth: { lte: 5 }`, mirroring apcore-python and apcore-typescript
/// (sync finding A-D-024). Integral float thresholds (`5.0`) are accepted as the
/// equivalent integer (A-D-005). Non-integral floats and non-numeric forms are
/// rejected (fail-closed) per spec.
pub struct MaxCallDepthHandler;

#[async_trait]
impl ACLConditionHandler for MaxCallDepthHandler {
    async fn evaluate(&self, value: &Value, ctx: &Context<Value>) -> bool {
        let threshold = match value {
            Value::Number(_) => integral_threshold(value),
            Value::Object(map) => map.get("lte").and_then(integral_threshold),
            _ => None,
        };
        match threshold {
            Some(max) => (ctx.call_chain.len() as u64) <= max,
            None => false,
        }
    }
}

/// The three structure-only predicates of the built-in `arguments` condition
/// (PROTOCOL_SPEC §6.1.7). A key outside this set is unevaluable, not false.
const ARGUMENT_PREDICATES: &[&str] = &["has_key", "has_all_keys", "has_none_of"];

/// The built-in `arguments` condition (PROTOCOL_SPEC §6.1.7, spec v1.28.0,
/// apcore#108).
///
/// | Predicate | Passes when |
/// |---|---|
/// | `has_key` | **any** of the named keys is present in the call's arguments |
/// | `has_all_keys` | **every** named key is present |
/// | `has_none_of` | **none** of the named keys is present |
///
/// Several predicates in one `arguments` object are ANDed, like any other
/// sibling conditions, and an empty object is vacuously satisfied — the same
/// vacuous truth that keeps `$not: {}` fail-closed.
///
/// # No predicate reads a value
///
/// That is a design constraint, not a first cut. The argument view here is not
/// reliably redacted (redaction is driven by `x-sensitive` markers in the
/// module's input schema, so a module without one gets none), and the
/// arguments are unvalidated — the ACL check is Step 4 and input schema
/// validation is Step 7, so a value may be absent, of the wrong type, or
/// malformed. Key presence is the one question well-defined on what is
/// available, and it answers the driving requirement: "did this call carry
/// `--force`?" is a presence question. Value-level predicates are deliberately
/// unspecified; if they are ever added they carry a precondition that the
/// module declares an `input_schema`.
///
/// The type makes this structural: it reads a
/// [`GovernanceProjection`](crate::acl::GovernanceProjection), which has no
/// field that can hold a value.
///
/// # It requires no registration
///
/// It is seeded like every other built-in, through `seed_condition`, and there
/// is no separate extension hook for argument predicates: a *dedicated*
/// registration point for them would invite exactly the unauditable host code
/// §7.9.6 rule 2 exists to keep out of a governance verdict, and a fixed
/// vocabulary keeps the decision reproducible from the ACL document.
///
/// §6.1.7 means the condition *requires* no registration — **not** that
/// `register_condition` is forbidden from replacing it. That call can already
/// replace `roles`, which is equally consequential, and special-casing one
/// built-in would be arbitrary; `seed_condition` therefore treats `arguments`
/// exactly as it treats the others and never overwrites a claimed key.
///
/// Being built-in also means §6.1.4's precheck covers the key for free:
/// `argument:` written for `arguments:` is an unregistered condition key, so
/// the rule is unevaluable rather than silently inert.
pub struct ArgumentsHandler;

impl ArgumentsHandler {
    /// Read one predicate operand as a list of argument key names.
    ///
    /// `None` means the operand is malformed — §6.1.1 case 4 — and the caller
    /// reports the condition **unevaluable**, never unsatisfied. A malformed
    /// operand is the failure mode v1.25.0 widened §6.1.1 to cover: a `deny`
    /// rule carrying `has_key: "force"` (a bare string rather than a list)
    /// would otherwise go inert, which is the defect the section exists to
    /// prevent, reached through the operand rather than the key.
    fn key_names(value: &Value) -> Option<Vec<&str>> {
        value
            .as_array()?
            .iter()
            .map(serde_json::Value::as_str)
            .collect()
    }

    fn decide(value: &Value) -> ConditionOutcome {
        let Some(predicates) = value.as_object() else {
            report_condition_unevaluable(
                "arguments",
                "value must be an object of predicates (has_key / has_all_keys / has_none_of)",
            );
            return ConditionOutcome::Unevaluable;
        };

        // An empty predicate object is UNEVALUABLE, not vacuously satisfied
        // (§6.1.7). The reason §6.1 gives for `$not: {}` being fail-closed
        // applies unchanged: an operator who wrote `arguments: {}` asked
        // nothing, and reading "asked nothing" as "passes" turns the rule they
        // meant to restrict into an unconditional one.
        if predicates.is_empty() {
            report_condition_unevaluable(
                "arguments",
                format!(
                    "no predicate given; name at least one of {}",
                    ARGUMENT_PREDICATES.join(", ")
                ),
            );
            return ConditionOutcome::Unevaluable;
        }

        // `serde_json::Map` is a `BTreeMap` here (the `preserve_order` feature
        // is off), so predicates are visited in ascending key order and the
        // diagnostics are identical on every run. Every predicate is visited —
        // no short-circuit — so `handler_error` names all of the malformed
        // ones, matching the evaluator above.
        let mut outcome = ConditionOutcome::Satisfied;
        for (predicate, operand) in predicates {
            outcome = outcome.and(Self::decide_one(predicate, operand));
        }
        outcome
    }

    fn decide_one(predicate: &str, operand: &Value) -> ConditionOutcome {
        if !ARGUMENT_PREDICATES.contains(&predicate) {
            report_condition_unevaluable(
                &format!("arguments.{predicate}"),
                format!(
                    "unknown argument predicate; the vocabulary is closed ({})",
                    ARGUMENT_PREDICATES.join(", ")
                ),
            );
            return ConditionOutcome::Unevaluable;
        }
        let Some(names) = Self::key_names(operand) else {
            report_condition_unevaluable(
                &format!("arguments.{predicate}"),
                "predicate value must be a list of argument key names",
            );
            return ConditionOutcome::Unevaluable;
        };
        // The projection is resolved AFTER the operand is checked, so a
        // malformed rule is reported as malformed even on a check that carries
        // no call site at all.
        let Some(projection) = current_governance_projection() else {
            report_condition_unevaluable(
                &format!("arguments.{predicate}"),
                "no governance projection for this check (PROTOCOL_SPEC §6.1.8); \
                 the arguments condition is only answerable from a call site — \
                 use ACL::check_access / ACL::async_check_access",
            );
            return ConditionOutcome::Unevaluable;
        };
        let present = |key: &&str| projection.contains_key(key);
        ConditionOutcome::from_bool(match predicate {
            "has_key" => names.iter().any(present),
            "has_all_keys" => names.iter().all(present),
            "has_none_of" => !names.iter().any(present),
            // Unreachable: the vocabulary was checked above. Fail closed
            // rather than panic inside the governance gate.
            _ => return ConditionOutcome::Unevaluable,
        })
    }
}

#[async_trait]
impl ACLConditionHandler for ArgumentsHandler {
    async fn evaluate(&self, value: &Value, _ctx: &Context<Value>) -> bool {
        Self::decide(value).is_satisfied()
    }

    /// Overridden so a malformed predicate or a missing projection reaches the
    /// rule loop as `Unevaluable` rather than collapsing to "does not match" —
    /// the distinction that decides what a `deny` rule does (§6.1.1).
    async fn evaluate_outcome(&self, value: &Value, _ctx: &Context<Value>) -> ConditionOutcome {
        Self::decide(value)
    }
}

// ---------------------------------------------------------------------------
// Compound handlers
// ---------------------------------------------------------------------------

/// Which registry a compound operator resolves its sub-conditions from.
///
/// PROTOCOL_SPEC §6.1: sub-conditions MUST be evaluated in the same mode as the
/// enclosing call. A single instance cannot know how it was invoked (the trait
/// has one `async fn evaluate`), so two instances are registered per operator:
/// the `Sync` one into [`CONDITION_HANDLERS`] (reached by `ACL::check`) and the
/// `Async` one into [`ASYNC_CONDITION_HANDLERS`] (reached by
/// `ACL::async_check` / [`evaluate_conditions_async`], which consults the async
/// registry first).
///
/// Before this split, both operators were registered only in the sync registry
/// yet delegated to [`evaluate_conditions_async`], so a *sync* `ACL::check` on
/// `{$or: [{k: v}]}` resolved `k` from the ASYNC registry — where apcore-python
/// and apcore-typescript resolve it from the sync registry only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompoundMode {
    Sync,
    Async,
}

/// Evaluate a sub-condition set in the mode of the enclosing call, preserving
/// the three-valued outcome so `$or` / `$not` can propagate it (§6.1.1).
async fn evaluate_sub_conditions(
    mode: CompoundMode,
    map: &HashMap<String, Value>,
    ctx: &Context<Value>,
) -> ConditionOutcome {
    match mode {
        CompoundMode::Sync => crate::acl::ACL::evaluate_conditions(map, ctx),
        CompoundMode::Async => evaluate_conditions_async_outcome(map, ctx).await,
    }
}

fn sub_condition_map(obj: &serde_json::Map<String, Value>) -> HashMap<String, Value> {
    obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// $or: list of condition dicts. Returns true if ANY sub-set passes.
pub(crate) struct OrHandler {
    mode: CompoundMode,
}

impl OrHandler {
    pub(crate) fn new(mode: CompoundMode) -> Self {
        Self { mode }
    }
}

#[async_trait]
impl ACLConditionHandler for OrHandler {
    async fn evaluate(&self, value: &Value, ctx: &Context<Value>) -> bool {
        self.evaluate_outcome(value, ctx).await.is_satisfied()
    }

    /// §6.1.1: an outright `Satisfied` child wins even against an unevaluable
    /// sibling; otherwise any unevaluable child makes the whole `$or`
    /// unevaluable. Every child is evaluated (no short-circuit on the first
    /// `Satisfied`) so that `handler_error` reports every unevaluable sibling
    /// deterministically — the decision is unchanged either way.
    ///
    /// A `$or` whose value is **not a list** is §6.1.1 case 4: the operand is
    /// malformed for its key, so there is no question to answer and the result
    /// is `Unevaluable`, not `Unsatisfied`. Reporting it as a plain non-match
    /// would put a `deny` rule carrying `$or: "typo"` right back into the inert
    /// state §6.1.1 exists to end. §6.1.4's precheck normally catches this
    /// before any handler runs; this branch covers a direct call to
    /// [`ACL::evaluate_conditions`](crate::ACL::evaluate_conditions), which has
    /// no precheck in front of it.
    async fn evaluate_outcome(&self, value: &Value, ctx: &Context<Value>) -> ConditionOutcome {
        let Some(arr) = value.as_array() else {
            report_condition_unevaluable("$or", "value must be a list of condition objects");
            return ConditionOutcome::Unevaluable;
        };
        // OR over three-valued children, starting from the identity of an
        // empty `$or: []` (which stays a non-match, as before).
        let mut outcome = ConditionOutcome::Unsatisfied;
        for (index, sub) in arr.iter().enumerate() {
            // §6.1.4 paths are positional: a key `k` in the i-th branch is
            // `$or[i].k`. The guard restores the enclosing prefix on every exit
            // path, including an unwind out of a panicking handler.
            let _guard = PathPrefixGuard::enter(&format!("$or[{index}]"));
            let child = if let Some(obj) = sub.as_object() {
                evaluate_sub_conditions(self.mode, &sub_condition_map(obj), ctx).await
            } else {
                // A branch that is not an object is malformed for `$or`.
                report_condition_unevaluable_at(
                    &current_path_prefix(),
                    "$or branch must be a condition object",
                );
                ConditionOutcome::Unevaluable
            };
            outcome = outcome.or(child);
        }
        outcome
    }
}

/// $not: single condition dict. Returns true if the sub-set FAILS.
pub(crate) struct NotHandler {
    mode: CompoundMode,
}

impl NotHandler {
    pub(crate) fn new(mode: CompoundMode) -> Self {
        Self { mode }
    }
}

#[async_trait]
impl ACLConditionHandler for NotHandler {
    async fn evaluate(&self, value: &Value, ctx: &Context<Value>) -> bool {
        self.evaluate_outcome(value, ctx).await.is_satisfied()
    }

    /// §6.1.1: `$not` of an unevaluable sub-condition is **unevaluable**, never
    /// satisfied. Negating "no answer" into "yes" would let a misspelled key
    /// inside a `$not` satisfy the very rule it was meant to gate — the bypass
    /// this section exists to close, reintroduced one nesting level down.
    ///
    /// A `$not` whose value is **not an object** is §6.1.1 case 4 — malformed
    /// for its key, hence `Unevaluable`. See [`OrHandler::evaluate_outcome`].
    async fn evaluate_outcome(&self, value: &Value, ctx: &Context<Value>) -> ConditionOutcome {
        if let Some(obj) = value.as_object() {
            let _guard = PathPrefixGuard::enter("$not");
            evaluate_sub_conditions(self.mode, &sub_condition_map(obj), ctx)
                .await
                .negate()
        } else {
            report_condition_unevaluable("$not", "value must be a condition object");
            ConditionOutcome::Unevaluable
        }
    }
}

// ---------------------------------------------------------------------------
// §6.1.4 — structural and registry precheck
// ---------------------------------------------------------------------------

/// Which evaluation path a precheck is being run for. Resolvability differs
/// between the two (§6.1.3), so the precheck's answer does too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrecheckPath {
    /// `ACL::check` — consults the sync registry only.
    Sync,
    /// `ACL::async_check` — consults the async registry, falling back to sync.
    Async,
}

/// One structural or registry fault found by the §6.1.4 precheck.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuleFault {
    /// Position of the fault in the rule (§6.1.4 condition paths).
    pub path: String,
    /// The offending key, where one exists. `None` for a `conditions` value
    /// that is not a mapping at all — there is no key to name.
    pub key: Option<String>,
    /// Human-readable reason, used verbatim in `handler_error`.
    pub reason: String,
    /// Whether the condition resolves for `check()`.
    pub sync_resolvable: bool,
    /// Whether it resolves for `async_check()`.
    pub async_resolvable: bool,
}

/// Run the §6.1.4 precheck over one rule's `conditions` tree.
///
/// **Context-independent and handler-free**, by requirement: it examines
/// structure and the handler registries only, and never invokes a handler. It
/// therefore runs *before* §6.5's "conditions present but no context provided"
/// check, which is what closes the bypass where `conditions: {mispelled: true}`
/// on a `deny` rule passed traffic simply because the caller carried no
/// identity.
///
/// It **does not short-circuit** (§6.1.4 rule 3): it has no decisive outcome to
/// short-circuit on, and its completeness is what makes §6.1.1 rule 2's
/// deterministic `handler_error` achievable. Because the walk is exhaustive and
/// depends only on the rule and the registries, every conforming implementation
/// produces the same set of precheck-origin findings for the same rule.
///
/// Faults come back ordered lexicographically by path.
pub(crate) fn precheck_conditions(conditions: &Value, path: PrecheckPath) -> Vec<RuleFault> {
    let mut faults = Vec::new();
    collect_condition_faults(conditions, "", path, &mut faults);
    faults.sort_by(|a, b| a.path.cmp(&b.path));
    faults
}

/// Structural fault with no resolvable handler on either path — a malformed
/// operand or a non-mapping `conditions`. Both flags are false because the
/// condition as written cannot be answered on either evaluation path, which is
/// what the flags describe (§6.1.3 rule 2).
fn structural_fault(path: String, key: Option<&str>, reason: &str) -> RuleFault {
    RuleFault {
        path,
        key: key.map(str::to_string),
        reason: reason.to_string(),
        sync_resolvable: false,
        async_resolvable: false,
    }
}

fn join_path(prefix: &str, local: &str) -> String {
    if prefix.is_empty() {
        local.to_string()
    } else {
        format!("{prefix}.{local}")
    }
}

fn collect_condition_faults(
    conditions: &Value,
    prefix: &str,
    path: PrecheckPath,
    out: &mut Vec<RuleFault>,
) {
    let Some(obj) = conditions.as_object() else {
        // §6.1.1 case 5: `conditions` itself is not a mapping. At the root this
        // is path `$`; nested, it is the branch's own path (a `$or` element or
        // a `$not` operand that is not an object).
        let (path_str, reason) = if prefix.is_empty() {
            ("$".to_string(), "conditions must be a mapping")
        } else {
            (prefix.to_string(), "condition branch must be an object")
        };
        out.push(structural_fault(path_str, None, reason));
        return;
    };

    for (key, value) in obj {
        let key_path = join_path(prefix, key);

        // Every key, compound operators included, must resolve to a handler on
        // the path in use (§6.1.1 case 1).
        let sync_resolvable = is_sync_resolvable(key);
        let async_resolvable = is_async_resolvable(key);
        let resolvable = match path {
            PrecheckPath::Sync => sync_resolvable,
            PrecheckPath::Async => async_resolvable,
        };
        if !resolvable {
            out.push(RuleFault {
                path: key_path.clone(),
                key: Some(key.clone()),
                reason: "unknown ACL condition".to_string(),
                sync_resolvable,
                async_resolvable,
            });
            // An unresolvable key cannot have its operand shape checked against
            // a handler that does not exist. Nothing further to say about it.
            continue;
        }

        // §6.1.1 case 4: the operand must have the shape its key requires.
        match key.as_str() {
            "$or" => match value.as_array() {
                Some(arr) => {
                    for (index, sub) in arr.iter().enumerate() {
                        collect_condition_faults(sub, &format!("{key_path}[{index}]"), path, out);
                    }
                }
                None => out.push(structural_fault(
                    key_path,
                    Some(key),
                    "$or value must be a list of condition objects",
                )),
            },
            "$not" => {
                if value.is_object() {
                    collect_condition_faults(value, &key_path, path, out);
                } else {
                    out.push(structural_fault(
                        key_path,
                        Some(key),
                        "$not value must be a condition object",
                    ));
                }
            }
            // These closed operand contracts can be validated without
            // running a handler, even when there is no caller context.
            "arguments" => collect_argument_faults(value, &key_path, out),
            _ => {
                if let Some(reason) = identity_condition_fault(key, value) {
                    out.push(structural_fault(key_path, Some(key), reason));
                }
            }
        }
    }
}

/// Structural faults in an `arguments` operand (PROTOCOL_SPEC §6.1.7).
///
/// Three shapes are decidable here, all without a context and without running
/// a handler: the operand is not an object of predicates; it is an empty
/// object; or a predicate is unrecognised or carries something other than a
/// list of key names. Every one of them is UNEVALUABLE rather than
/// UNSATISFIED — the direction only shows on a `deny` rule, where "does not
/// match" means the call proceeds.
///
/// Does not short-circuit: every predicate is examined, so `handler_error` and
/// `validate_rules()` name all of them (§6.1.4 rule 3).
fn collect_argument_faults(value: &Value, key_path: &str, out: &mut Vec<RuleFault>) {
    let Some(predicates) = value.as_object() else {
        out.push(structural_fault(
            key_path.to_string(),
            Some("arguments"),
            "arguments value must be an object of predicates \
             (has_key / has_all_keys / has_none_of)",
        ));
        return;
    };
    if predicates.is_empty() {
        out.push(structural_fault(
            key_path.to_string(),
            Some("arguments"),
            "arguments carries no predicate; name at least one of \
             has_key, has_all_keys, has_none_of",
        ));
        return;
    }
    for (predicate, operand) in predicates {
        let path = format!("{key_path}.{predicate}");
        if !ARGUMENT_PREDICATES.contains(&predicate.as_str()) {
            out.push(structural_fault(
                path,
                Some("arguments"),
                &format!(
                    "unknown argument predicate '{predicate}'; the vocabulary is closed ({})",
                    ARGUMENT_PREDICATES.join(", ")
                ),
            ));
            continue;
        }
        if ArgumentsHandler::key_names(operand).is_none() {
            out.push(structural_fault(
                path,
                Some("arguments"),
                &format!("'{predicate}' value must be a list of argument key names"),
            ));
        }
    }
}

// Identity is significant here: a host can replace even a built-in key in
// either registry. Discovery only evaluates framework-owned pure handlers.
static DISCOVERY_IDENTITY_TYPES: LazyLock<Arc<dyn ACLConditionHandler>> =
    LazyLock::new(|| Arc::new(IdentityTypesHandler));
static DISCOVERY_ROLES: LazyLock<Arc<dyn ACLConditionHandler>> =
    LazyLock::new(|| Arc::new(RolesHandler));
static DISCOVERY_SCOPES: LazyLock<Arc<dyn ACLConditionHandler>> =
    LazyLock::new(|| Arc::new(ScopesHandler));
static DISCOVERY_SUBJECT_TYPES: LazyLock<Arc<dyn ACLConditionHandler>> =
    LazyLock::new(|| Arc::new(SubjectTypesHandler));
static DISCOVERY_SUBJECT_ROLES: LazyLock<Arc<dyn ACLConditionHandler>> =
    LazyLock::new(|| Arc::new(SubjectRolesHandler));
static DISCOVERY_MAX_DELEGATION_DEPTH: LazyLock<Arc<dyn ACLConditionHandler>> =
    LazyLock::new(|| Arc::new(MaxDelegationDepthHandler));
static DISCOVERY_MAX_CALL_DEPTH: LazyLock<Arc<dyn ACLConditionHandler>> =
    LazyLock::new(|| Arc::new(MaxCallDepthHandler));
static DISCOVERY_OR: LazyLock<Arc<dyn ACLConditionHandler>> =
    LazyLock::new(|| Arc::new(OrHandler::new(CompoundMode::Async)));
static DISCOVERY_NOT: LazyLock<Arc<dyn ACLConditionHandler>> =
    LazyLock::new(|| Arc::new(NotHandler::new(CompoundMode::Async)));

fn stock_discovery_handler(key: &str) -> Option<&'static Arc<dyn ACLConditionHandler>> {
    let stock = match key {
        "identity_types" => &*DISCOVERY_IDENTITY_TYPES,
        "roles" => &*DISCOVERY_ROLES,
        "scopes" => &*DISCOVERY_SCOPES,
        "subject_types" => &*DISCOVERY_SUBJECT_TYPES,
        "subject_roles" => &*DISCOVERY_SUBJECT_ROLES,
        "max_delegation_depth" => &*DISCOVERY_MAX_DELEGATION_DEPTH,
        "max_call_depth" => &*DISCOVERY_MAX_CALL_DEPTH,
        "$or" => &*DISCOVERY_OR,
        "$not" => &*DISCOVERY_NOT,
        _ => return None,
    };
    let async_handlers = ASYNC_CONDITION_HANDLERS.read();
    let handlers = CONDITION_HANDLERS.read();
    async_handlers
        .get(key)
        .or_else(|| handlers.get(key))
        .is_some_and(|handler| Arc::ptr_eq(handler, stock))
        .then_some(stock)
}

/// Evaluate a prechecked condition tree with no business arguments. Here
/// Unevaluable means a well-formed condition with an unknown call-time result.
pub(crate) fn discovery_conditions(conditions: &Value, ctx: &Context<Value>) -> ConditionOutcome {
    let Some(conditions) = conditions.as_object() else {
        return ConditionOutcome::Unevaluable;
    };
    conditions
        .iter()
        .fold(ConditionOutcome::Satisfied, |result, (key, value)| {
            result.and(discovery_condition(key, value, ctx))
        })
}

fn discovery_condition(key: &str, value: &Value, ctx: &Context<Value>) -> ConditionOutcome {
    let Some(handler) = stock_discovery_handler(key) else {
        return ConditionOutcome::Unevaluable;
    };
    match key {
        "$or" => {
            let Some(branches) = value.as_array() else {
                return ConditionOutcome::Unevaluable;
            };
            branches
                .iter()
                .fold(ConditionOutcome::Unsatisfied, |result, branch| {
                    result.or(discovery_conditions(branch, ctx))
                })
        }
        "$not" => discovery_conditions(value, ctx).negate(),
        _ => {
            use futures_util::FutureExt;
            handler
                .evaluate_outcome(value, ctx)
                .now_or_never()
                .unwrap_or(ConditionOutcome::Unevaluable)
        }
    }
}

/// Register all built-in handlers. Called once during initialization.
pub fn register_builtin_handlers() {
    seed_condition("identity_types", || Arc::clone(&DISCOVERY_IDENTITY_TYPES));
    seed_condition("roles", || Arc::clone(&DISCOVERY_ROLES));
    seed_condition("scopes", || Arc::clone(&DISCOVERY_SCOPES));
    seed_condition("subject_types", || Arc::clone(&DISCOVERY_SUBJECT_TYPES));
    seed_condition("subject_roles", || Arc::clone(&DISCOVERY_SUBJECT_ROLES));
    seed_condition("max_delegation_depth", || {
        Arc::clone(&DISCOVERY_MAX_DELEGATION_DEPTH)
    });
    seed_condition("max_call_depth", || Arc::clone(&DISCOVERY_MAX_CALL_DEPTH));
    // §6.1.7: `arguments` is part of the language §6.1 already defines, not a
    // mechanism beside it, so it is seeded here with the other built-ins and
    // has no registration point of its own.
    seed_condition("arguments", || Arc::new(ArgumentsHandler));
    // Mode-matched pairs — see `CompoundMode`.
    seed_condition("$or", || Arc::new(OrHandler::new(CompoundMode::Sync)));
    seed_condition("$not", || Arc::new(NotHandler::new(CompoundMode::Sync)));
    seed_async_condition("$or", || Arc::clone(&DISCOVERY_OR));
    seed_async_condition("$not", || Arc::clone(&DISCOVERY_NOT));
}

/// Install a built-in handler ONLY if `key` is unclaimed.
///
/// Built-ins are a floor, not an override. apcore-python and apcore-typescript
/// seed theirs at module load, so a deployment's `register_condition` call
/// always lands afterwards and wins. Rust seeds lazily, from the first
/// `ACL::new` — with an unconditional insert, a stricter handler registered at
/// startup was silently replaced by the permissive built-in the moment the
/// first ACL was constructed, and the ACL then evaluated `roles` against the
/// built-in rule the deployment had deliberately replaced. Same call after the
/// first ACL survived, so the failure was ordering-dependent and invisible
/// (sync finding A-D-010).
fn seed_condition(key: &str, build: impl FnOnce() -> Arc<dyn ACLConditionHandler>) {
    let mut map = CONDITION_HANDLERS.write();
    map.entry(key.to_string()).or_insert_with(build);
}

/// [`seed_condition`] for the async-only registry.
fn seed_async_condition(key: &str, build: impl FnOnce() -> Arc<dyn ACLConditionHandler>) {
    let mut map = ASYNC_CONDITION_HANDLERS.write();
    map.entry(key.to_string()).or_insert_with(build);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{Context, Identity};

    fn make_ctx(identity_type: &str, roles: Vec<&str>, call_depth: usize) -> Context<Value> {
        let identity = Identity::new(
            "test-id".to_string(),
            identity_type.to_string(),
            roles.into_iter().map(String::from).collect(),
            HashMap::new(),
        );
        let mut ctx = Context::new(identity);
        for i in 0..call_depth {
            ctx.call_chain.push(format!("module.{i}"));
        }
        ctx
    }

    fn anon_ctx() -> Context<Value> {
        Context::<Value>::anonymous()
    }

    // -------------------------------------------------------------------------
    // IdentityTypesHandler
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn identity_types_matches_correct_type() {
        let handler = IdentityTypesHandler;
        let ctx = make_ctx("user", vec![], 0);
        let value = serde_json::json!(["user", "service"]);
        assert!(handler.evaluate(&value, &ctx).await);
    }

    #[tokio::test]
    async fn identity_types_rejects_wrong_type() {
        let handler = IdentityTypesHandler;
        let ctx = make_ctx("agent", vec![], 0);
        let value = serde_json::json!(["user", "service"]);
        assert!(!handler.evaluate(&value, &ctx).await);
    }

    #[tokio::test]
    async fn identity_types_rejects_non_array_value() {
        let handler = IdentityTypesHandler;
        let ctx = make_ctx("user", vec![], 0);
        let value = serde_json::json!("user"); // not an array
        assert!(!handler.evaluate(&value, &ctx).await);
    }

    #[tokio::test]
    async fn identity_types_rejects_no_identity() {
        let handler = IdentityTypesHandler;
        let ctx = anon_ctx();
        let value = serde_json::json!(["user"]);
        assert!(!handler.evaluate(&value, &ctx).await);
    }

    // -------------------------------------------------------------------------
    // RolesHandler
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn roles_matches_overlapping_role() {
        let handler = RolesHandler;
        let ctx = make_ctx("user", vec!["admin", "viewer"], 0);
        let value = serde_json::json!(["admin"]);
        assert!(handler.evaluate(&value, &ctx).await);
    }

    #[tokio::test]
    async fn roles_rejects_no_overlap() {
        let handler = RolesHandler;
        let ctx = make_ctx("user", vec!["viewer"], 0);
        let value = serde_json::json!(["admin"]);
        assert!(!handler.evaluate(&value, &ctx).await);
    }

    #[tokio::test]
    async fn roles_rejects_no_identity() {
        let handler = RolesHandler;
        let ctx = anon_ctx();
        let value = serde_json::json!(["admin"]);
        assert!(!handler.evaluate(&value, &ctx).await);
    }

    // -------------------------------------------------------------------------
    // MaxCallDepthHandler
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn max_call_depth_allows_under_limit() {
        let handler = MaxCallDepthHandler;
        let ctx = make_ctx("user", vec![], 3);
        let value = serde_json::json!(5u64);
        assert!(handler.evaluate(&value, &ctx).await);
    }

    #[tokio::test]
    async fn max_call_depth_allows_at_limit() {
        let handler = MaxCallDepthHandler;
        let ctx = make_ctx("user", vec![], 5);
        let value = serde_json::json!(5u64);
        assert!(handler.evaluate(&value, &ctx).await);
    }

    #[tokio::test]
    async fn max_call_depth_rejects_over_limit() {
        let handler = MaxCallDepthHandler;
        let ctx = make_ctx("user", vec![], 6);
        let value = serde_json::json!(5u64);
        assert!(!handler.evaluate(&value, &ctx).await);
    }

    #[tokio::test]
    async fn max_call_depth_rejects_non_numeric_value() {
        let handler = MaxCallDepthHandler;
        let ctx = make_ctx("user", vec![], 0);
        let value = serde_json::json!("five"); // not a number
        assert!(!handler.evaluate(&value, &ctx).await);
    }

    #[tokio::test]
    async fn max_call_depth_accepts_integral_float_threshold() {
        // A-D-005: `max_call_depth: 5.0` is an integral float; it must be
        // treated as depth 5 (matches TS). Caller at depth 5 → ALLOW.
        let handler = MaxCallDepthHandler;
        let ctx = make_ctx("user", vec![], 5);
        let value = serde_json::json!(5.0);
        assert!(handler.evaluate(&value, &ctx).await);
    }

    #[tokio::test]
    async fn max_call_depth_integral_float_rejects_over_limit() {
        let handler = MaxCallDepthHandler;
        let ctx = make_ctx("user", vec![], 6);
        let value = serde_json::json!(5.0);
        assert!(!handler.evaluate(&value, &ctx).await);
    }

    #[tokio::test]
    async fn max_call_depth_rejects_non_integral_float() {
        // 5.5 is not an integer threshold — fail-closed.
        let handler = MaxCallDepthHandler;
        let ctx = make_ctx("user", vec![], 5);
        let value = serde_json::json!(5.5);
        assert!(!handler.evaluate(&value, &ctx).await);
    }

    #[tokio::test]
    async fn max_call_depth_accepts_integral_float_in_lte_form() {
        let handler = MaxCallDepthHandler;
        let ctx = make_ctx("user", vec![], 5);
        let value = serde_json::json!({ "lte": 5.0 });
        assert!(handler.evaluate(&value, &ctx).await);
    }

    // -------------------------------------------------------------------------
    // OrHandler
    // -------------------------------------------------------------------------

    /// Simple async handler for compound-operator tests: checks `{"pass": true}`.
    struct PassHandler;

    #[async_trait]
    impl ACLConditionHandler for PassHandler {
        async fn evaluate(&self, value: &Value, _ctx: &Context<Value>) -> bool {
            value.as_bool().unwrap_or(false)
        }
    }

    /// A built-in must not replace a handler the deployment already claimed.
    ///
    /// Tested on `seed_condition` directly, with a key no other test uses: the
    /// registries are process-global, so driving this through `ACL::new` and a
    /// real built-in key would leave a window in which any concurrently
    /// running ACL test reads the wrong handler.
    #[test]
    fn seed_condition_does_not_replace_a_claimed_key() {
        const KEY: &str = "_test_seed_if_absent_rs";
        register_condition(KEY, Arc::new(PassHandler));

        let claimed = Arc::as_ptr(
            CONDITION_HANDLERS
                .read()
                .get(KEY)
                .expect("the explicit registration landed"),
        );

        seed_condition(KEY, || Arc::new(IdentityTypesHandler));

        let after = Arc::as_ptr(
            CONDITION_HANDLERS
                .read()
                .get(KEY)
                .expect("the key is still registered"),
        );
        assert!(
            std::ptr::addr_eq(claimed, after),
            "seeding replaced a handler that was already registered — a \
             deployment's stricter handler would be silently swapped for the \
             permissive built-in (A-D-010)"
        );
    }

    /// The same call installs the handler when the key is unclaimed.
    #[test]
    fn seed_condition_installs_when_the_key_is_free() {
        const KEY: &str = "_test_seed_when_free_rs";
        assert!(!CONDITION_HANDLERS.read().contains_key(KEY));
        seed_condition(KEY, || Arc::new(IdentityTypesHandler));
        assert!(CONDITION_HANDLERS.read().contains_key(KEY));
    }

    /// Guard the fix at its source: every built-in must go through the
    /// if-absent seed, never through the overwriting `register_condition`.
    ///
    /// The unit tests above pin `seed_condition`'s semantics; this pins that
    /// `register_builtin_handlers` still uses it. Rewriting one line back to
    /// `register_condition("roles", …)` would otherwise reintroduce A-D-010
    /// with both unit tests still green.
    #[test]
    fn built_ins_are_seeded_never_overwritten() {
        let src = include_str!("acl_handlers.rs");
        let start = src
            .find("pub fn register_builtin_handlers() {")
            .expect("register_builtin_handlers still exists");
        let body_end = src[start..]
            .find("\n}")
            .expect("its body is brace-terminated")
            + start;
        let body = &src[start..body_end];

        assert!(
            !body.contains("register_condition(") && !body.contains("register_async_condition("),
            "register_builtin_handlers overwrites instead of seeding — a \
             handler registered before the first ACL::new would be replaced \
             by the built-in (A-D-010):\n{body}"
        );
        assert_eq!(
            body.matches("seed_condition(").count() + body.matches("seed_async_condition(").count(),
            12,
            "expected all twelve built-in registrations to be seeded — eight \
             sync handlers plus the two mode-matched compound pairs:\n{body}"
        );
    }

    /// Register "pass" handler and ensure built-ins are present before compound tests.
    fn setup_compound_test_handlers() {
        register_condition("pass", Arc::new(PassHandler));
        // Ensure the OrHandler / NotHandler themselves are registered so nested
        // compound operators work in the integration tests below.
        register_builtin_handlers();
    }

    #[tokio::test]
    async fn or_handler_true_if_any_sub_passes() {
        setup_compound_test_handlers();
        let handler = OrHandler::new(CompoundMode::Async);
        let ctx = anon_ctx();
        let value = serde_json::json!([
            {"pass": false},
            {"pass": true},
        ]);
        assert!(handler.evaluate(&value, &ctx).await);
    }

    #[tokio::test]
    async fn or_handler_false_if_none_pass() {
        setup_compound_test_handlers();
        let handler = OrHandler::new(CompoundMode::Async);
        let ctx = anon_ctx();
        let value = serde_json::json!([
            {"pass": false},
            {"pass": false},
        ]);
        assert!(!handler.evaluate(&value, &ctx).await);
    }

    #[tokio::test]
    async fn or_handler_rejects_non_array_value() {
        let handler = OrHandler::new(CompoundMode::Async);
        let ctx = anon_ctx();
        let value = serde_json::json!({"pass": true}); // not an array
        assert!(!handler.evaluate(&value, &ctx).await);
    }

    // -------------------------------------------------------------------------
    // NotHandler
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn not_handler_inverts_passing_condition() {
        setup_compound_test_handlers();
        let handler = NotHandler::new(CompoundMode::Async);
        let ctx = anon_ctx();
        let value = serde_json::json!({"pass": true});
        assert!(!handler.evaluate(&value, &ctx).await);
    }

    #[tokio::test]
    async fn not_handler_inverts_failing_condition() {
        setup_compound_test_handlers();
        let handler = NotHandler::new(CompoundMode::Async);
        let ctx = anon_ctx();
        let value = serde_json::json!({"pass": false});
        assert!(handler.evaluate(&value, &ctx).await);
    }

    #[tokio::test]
    async fn not_handler_rejects_non_object_value() {
        let handler = NotHandler::new(CompoundMode::Async);
        let ctx = anon_ctx();
        let value = serde_json::json!([{"pass": true}]); // not an object
        assert!(!handler.evaluate(&value, &ctx).await);
    }

    // -------------------------------------------------------------------------
    // register_condition
    // -------------------------------------------------------------------------

    #[test]
    fn register_condition_stores_and_overwrites() {
        register_condition("_test_handler", Arc::new(MaxCallDepthHandler));
        // Overwrite — should not panic
        register_condition("_test_handler", Arc::new(MaxCallDepthHandler));
        let map = CONDITION_HANDLERS.read();
        assert!(map.contains_key("_test_handler"));
    }

    // -------------------------------------------------------------------------
    // register_async_condition — separate registry from sync (A-D-ACL-002)
    // -------------------------------------------------------------------------

    /// Async-only handler that always returns true.
    struct AsyncOnlyTrue;

    #[async_trait]
    impl ACLConditionHandler for AsyncOnlyTrue {
        async fn evaluate(&self, _value: &Value, _ctx: &Context<Value>) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn register_async_condition_uses_separate_registry() {
        // Register the same key in both registries with opposite outcomes —
        // the async path MUST consult the async registry first.
        struct SyncDeny;
        #[async_trait]
        impl ACLConditionHandler for SyncDeny {
            async fn evaluate(&self, _value: &Value, _ctx: &Context<Value>) -> bool {
                false
            }
        }

        let key = "_test_async_vs_sync";
        register_condition(key, Arc::new(SyncDeny));
        register_async_condition(key, Arc::new(AsyncOnlyTrue));

        // Async path resolves the async-only handler → true.
        let mut conditions: HashMap<String, Value> = HashMap::new();
        conditions.insert(key.to_string(), Value::Null);
        let ctx = anon_ctx();
        assert!(evaluate_conditions_async(&conditions, &ctx).await);

        assert_eq!(
            crate::acl::ACL::evaluate_conditions(&conditions, &ctx),
            ConditionOutcome::Unsatisfied
        );
    }

    #[test]
    fn test_async_evaluation_completes_while_sync_registry_is_locked() {
        use futures_util::FutureExt;
        use std::sync::mpsc;
        use std::time::Duration;

        let key = "_test_async_without_sync_lookup";
        register_async_condition(key, Arc::new(AsyncOnlyTrue));

        let sync_guard = CONDITION_HANDLERS.write();
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let conditions = HashMap::from([(key.to_string(), Value::Null)]);
            let ctx = anon_ctx();
            let outcome = evaluate_conditions_async_outcome(&conditions, &ctx).now_or_never();
            sender.send(outcome).unwrap();
        });
        // Use a separate thread so a blocked registry read cannot also block
        // the timeout. Release the guard before joining even on failure.
        let outcome = receiver.recv_timeout(Duration::from_secs(2));
        drop(sync_guard);
        worker.join().unwrap();
        assert_eq!(outcome, Ok(Some(ConditionOutcome::Satisfied)));
    }

    #[tokio::test]
    async fn async_check_falls_back_to_sync_registry_when_no_async_handler() {
        // Only register on the sync side — async evaluation MUST still find it.
        let key = "_test_async_fallback";
        register_condition(key, Arc::new(AsyncOnlyTrue));

        let mut conditions: HashMap<String, Value> = HashMap::new();
        conditions.insert(key.to_string(), Value::Null);
        let ctx = anon_ctx();
        assert!(evaluate_conditions_async(&conditions, &ctx).await);
    }

    // -------------------------------------------------------------------------
    // handler_error capture (A-D-ACL-001)
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn handler_error_capture_returns_reported_message() {
        let (decision, captured) = with_handler_error_capture(async {
            report_handler_error("simulated handler failure");
            false
        })
        .await;
        assert!(!decision);
        assert_eq!(captured.as_deref(), Some("simulated handler failure"));
    }

    #[tokio::test]
    async fn handler_error_capture_isolates_per_scope() {
        // Two independent scopes must not see each other's errors.
        let ((), first) = with_handler_error_capture(async {
            report_handler_error("first call");
        })
        .await;
        let ((), second) = with_handler_error_capture(async {
            // No report inside this scope.
        })
        .await;
        assert_eq!(first.as_deref(), Some("first call"));
        assert!(second.is_none());
    }

    #[test]
    fn report_handler_error_outside_scope_is_noop() {
        // Calling outside an active capture scope must not panic — it falls
        // through silently because the task-local has no slot.
        report_handler_error("dropped on the floor");
    }
}
