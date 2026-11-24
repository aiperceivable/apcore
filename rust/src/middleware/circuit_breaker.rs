// APCore Protocol — CircuitBreakerMiddleware (Issue #42)
// Spec reference: middleware-system.md §CircuitBreakerMiddleware
//
// Tracks per-(module_id, caller_id) error rates over a rolling window. When
// the error rate exceeds `open_threshold`, the circuit transitions to OPEN
// and short-circuits subsequent calls with `ErrorCode::CircuitBreakerOpen`.
// After `recovery_window_ms` elapses the circuit moves to HALF_OPEN and
// allows exactly one probe call; success closes the circuit, failure
// re-opens it.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use super::base::Middleware;
use super::context_namespace::{enforce_context_key, namespace_keys::CIRCUIT_STATE, ContextWriter};
use crate::context::Context;
use crate::errors::ModuleError;
use crate::events::emitter::{ApCoreEvent, EventEmitter};

/// Default error-rate at which the circuit opens.
pub const DEFAULT_OPEN_THRESHOLD: f64 = 0.5;
/// Default rolling-window size (sample count).
pub const DEFAULT_WINDOW_SIZE: usize = 20;
/// Default duration the circuit stays OPEN before probing.
pub const DEFAULT_RECOVERY_WINDOW_MS: u64 = 30_000;
/// Default minimum samples required before the circuit can open. This avoids
/// false-positives on the very first failure.
pub const DEFAULT_MIN_SAMPLES: usize = 5;

/// Lifecycle state of a single per-(module, caller) circuit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CircuitBreakerState {
    Closed,
    Open,
    HalfOpen,
}

impl CircuitBreakerState {
    /// Spec-canonical uppercase string ("CLOSED" | "OPEN" | "HALF_OPEN").
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Closed => "CLOSED",
            Self::Open => "OPEN",
            Self::HalfOpen => "HALF_OPEN",
        }
    }
}

/// Per-(module, caller) circuit record.
#[derive(Debug)]
struct Circuit {
    state: CircuitBreakerState,
    /// Rolling window of recent outcomes — `true` = success, `false` = error.
    /// Capped at `window_size`; oldest entry is evicted on push.
    window: VecDeque<bool>,
    /// When the circuit last transitioned to OPEN. Used to detect recovery.
    opened_at: Option<Instant>,
    /// `true` if a probe call is currently in flight in HALF_OPEN state.
    /// Limits HALF_OPEN concurrency to a single probe.
    probe_in_flight: bool,
    /// When the active HALF_OPEN probe was admitted. If the probe never
    /// completes (e.g. async cancellation between `before` and
    /// `after`/`on_error`), the slot would otherwise leak — the next
    /// `before()` reclaims it once it has been held longer than the
    /// recovery window.
    probe_started_at: Option<Instant>,
}

impl Circuit {
    fn new() -> Self {
        Self {
            state: CircuitBreakerState::Closed,
            window: VecDeque::new(),
            opened_at: None,
            probe_in_flight: false,
            probe_started_at: None,
        }
    }

    fn push(&mut self, success: bool, window_size: usize) {
        let cap = window_size.max(1);
        while self.window.len() >= cap {
            self.window.pop_front();
        }
        self.window.push_back(success);
    }

    fn error_rate(&self) -> f64 {
        if self.window.is_empty() {
            return 0.0;
        }
        let errors = self.window.iter().filter(|s| !**s).count();
        #[allow(clippy::cast_precision_loss)] // window_size is bounded; precision loss negligible
        let rate = errors as f64 / self.window.len() as f64;
        rate
    }
}

/// Configuration for [`CircuitBreakerMiddleware`].
#[derive(Debug, Clone, Copy)]
pub struct CircuitBreakerConfig {
    /// Error-rate threshold (0.0–1.0). When the error rate in the rolling
    /// window meets or exceeds this value, the circuit opens.
    pub open_threshold: f64,
    /// Number of recent calls retained in the rolling window per circuit.
    pub window_size: usize,
    /// Time the circuit stays OPEN before it transitions to HALF_OPEN to
    /// admit a probe.
    pub recovery_window_ms: u64,
    /// Minimum samples required before the threshold check can open the
    /// circuit. Prevents a single failure from tripping the breaker.
    pub min_samples: usize,
    /// Middleware ordering priority (higher runs first; max 1000).
    /// Defaults to 900 so the breaker runs *before* tracing (800) and
    /// logging (700) — short-circuiting an OPEN circuit before downstream
    /// middleware accumulates per-call instrumentation state.
    pub priority: u16,
}

/// Default priority — runs first in the chain so OPEN circuits short-circuit
/// before tracing/logging instrumentation fires.
pub const DEFAULT_PRIORITY: u16 = 900;

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            open_threshold: DEFAULT_OPEN_THRESHOLD,
            window_size: DEFAULT_WINDOW_SIZE,
            recovery_window_ms: DEFAULT_RECOVERY_WINDOW_MS,
            min_samples: DEFAULT_MIN_SAMPLES,
            priority: DEFAULT_PRIORITY,
        }
    }
}

/// Builder-friendly entry point for [`CircuitBreakerMiddleware`].
#[derive(Default)]
pub struct CircuitBreakerBuilder {
    config: CircuitBreakerConfig,
    emitter: Option<Arc<EventEmitter>>,
    clock: Option<Box<dyn Fn() -> Instant + Send + Sync>>,
}

impl std::fmt::Debug for CircuitBreakerBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CircuitBreakerBuilder")
            .field("config", &self.config)
            .field("has_emitter", &self.emitter.is_some())
            .field("has_clock", &self.clock.is_some())
            .finish()
    }
}

impl CircuitBreakerBuilder {
    #[must_use]
    pub fn open_threshold(mut self, value: f64) -> Self {
        self.config.open_threshold = value;
        self
    }

    #[must_use]
    pub fn window_size(mut self, value: usize) -> Self {
        self.config.window_size = value.max(1);
        self
    }

    #[must_use]
    pub fn recovery_window_ms(mut self, value: u64) -> Self {
        self.config.recovery_window_ms = value;
        self
    }

    #[must_use]
    pub fn min_samples(mut self, value: usize) -> Self {
        self.config.min_samples = value;
        self
    }

    #[must_use]
    pub fn priority(mut self, value: u16) -> Self {
        self.config.priority = value;
        self
    }

    #[must_use]
    pub fn emitter(mut self, emitter: Arc<EventEmitter>) -> Self {
        self.emitter = Some(emitter);
        self
    }

    /// Override the monotonic clock used for recovery-window checks.
    /// Intended for tests; production code should rely on the default.
    #[must_use]
    pub fn clock<F>(mut self, clock: F) -> Self
    where
        F: Fn() -> Instant + Send + Sync + 'static,
    {
        self.clock = Some(Box::new(clock));
        self
    }

    #[must_use]
    pub fn build(self) -> CircuitBreakerMiddleware {
        let mut mw = CircuitBreakerMiddleware::with_parts(self.config, self.emitter);
        if let Some(clock) = self.clock {
            mw.clock = clock;
        }
        mw
    }
}

/// Per-module circuit-breaker middleware.
///
/// Internally keyed by `(module_id, caller_id)`. The `caller_id` is sourced
/// from `context.caller_id` (an empty string when absent so top-level calls
/// share a single bucket).
pub struct CircuitBreakerMiddleware {
    config: CircuitBreakerConfig,
    emitter: Option<Arc<EventEmitter>>,
    circuits: Mutex<HashMap<(String, String), Circuit>>,
    /// Pluggable monotonic clock; defaults to [`Instant::now`]. Override via
    /// [`Self::with_clock`] for time-sensitive tests.
    clock: Box<dyn Fn() -> Instant + Send + Sync>,
}

impl std::fmt::Debug for CircuitBreakerMiddleware {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CircuitBreakerMiddleware")
            .field("config", &self.config)
            .field("circuit_count", &self.circuits.lock().len())
            .finish_non_exhaustive()
    }
}

impl CircuitBreakerMiddleware {
    /// Builder for the most common construction path.
    #[must_use]
    pub fn builder() -> CircuitBreakerBuilder {
        CircuitBreakerBuilder::default()
    }

    /// Create a middleware with explicit config and optional event emitter.
    #[must_use]
    pub fn with_parts(
        mut config: CircuitBreakerConfig,
        emitter: Option<Arc<EventEmitter>>,
    ) -> Self {
        // Clamp pathological window_size: 0 would otherwise let the rolling
        // window grow unbounded.
        if config.window_size == 0 {
            tracing::warn!(
                "CircuitBreakerConfig.window_size was 0; clamping to 1. \
                 The breaker will only retain the most recent sample."
            );
            config.window_size = 1;
        }
        if config.min_samples > config.window_size {
            tracing::warn!(
                min_samples = config.min_samples,
                window_size = config.window_size,
                "CircuitBreakerConfig.min_samples exceeds window_size; \
                 the breaker can never open. Reduce min_samples or grow window_size."
            );
        }
        Self {
            config,
            emitter,
            circuits: Mutex::new(HashMap::new()),
            clock: Box::new(Instant::now),
        }
    }

    /// Override the clock used for recovery-window checks.
    #[must_use]
    pub fn with_clock<F>(mut self, clock: F) -> Self
    where
        F: Fn() -> Instant + Send + Sync + 'static,
    {
        self.clock = Box::new(clock);
        self
    }

    /// Read the current state for a `(module_id, caller_id)` pair without
    /// triggering recovery. Returns `CLOSED` for unknown pairs.
    #[must_use]
    pub fn state(&self, module_id: &str, caller_id: &str) -> CircuitBreakerState {
        let key = (module_id.to_string(), caller_id.to_string());
        self.circuits
            .lock()
            .get(&key)
            .map_or(CircuitBreakerState::Closed, |c| c.state)
    }

    /// Force the state of a circuit. Intended for tests and operational
    /// overrides — not part of normal flow.
    pub fn force_state(
        &self,
        module_id: &str,
        caller_id: &str,
        state: CircuitBreakerState,
        opened_at: Option<Instant>,
    ) {
        let key = (module_id.to_string(), caller_id.to_string());
        let mut circuits = self.circuits.lock();
        let entry = circuits.entry(key).or_insert_with(Circuit::new);
        entry.state = state;
        entry.opened_at = opened_at;
        entry.probe_in_flight = false;
        entry.probe_started_at = None;
    }

    /// Reset a circuit to CLOSED and clear its rolling window.
    ///
    /// Mirrors apcore-python / apcore-typescript `reset()`: returns the circuit
    /// to a pristine CLOSED state, discarding accumulated outcomes and any
    /// in-flight probe bookkeeping. `caller_id` defaults to the empty bucket
    /// (top-level calls) when `None`. A reset on an unknown `(module, caller)`
    /// pair is a no-op (the circuit is already CLOSED with an empty window).
    pub fn reset(&self, module_id: &str, caller_id: Option<&str>) {
        let key = (
            module_id.to_string(),
            caller_id.unwrap_or_default().to_string(),
        );
        let mut circuits = self.circuits.lock();
        if let Some(circuit) = circuits.get_mut(&key) {
            circuit.state = CircuitBreakerState::Closed;
            circuit.window.clear();
            circuit.opened_at = None;
            circuit.probe_in_flight = false;
            circuit.probe_started_at = None;
        }
    }

    fn key_of(module_id: &str, ctx: &Context<serde_json::Value>) -> (String, String) {
        (
            module_id.to_string(),
            ctx.caller_id.clone().unwrap_or_default(),
        )
    }

    fn write_state_to_context(ctx: &Context<serde_json::Value>, state: CircuitBreakerState) {
        // CIRCUIT_STATE is in the framework namespace; this is a Framework write.
        let _ = enforce_context_key(ContextWriter::Framework, CIRCUIT_STATE);
        let mut data = ctx.data.write();
        data.insert(
            CIRCUIT_STATE.to_string(),
            serde_json::Value::String(state.as_str().to_string()),
        );
    }

    fn build_event(
        event_type: &str,
        module_id: &str,
        caller_id: &str,
        error_rate: f64,
        severity: &str,
    ) -> ApCoreEvent {
        ApCoreEvent::with_module(
            event_type,
            serde_json::json!({
                "module_id": module_id,
                "caller_id": caller_id,
                "error_rate": error_rate,
            }),
            module_id,
            severity,
        )
    }

    async fn emit(&self, event: ApCoreEvent) {
        if let Some(emitter) = &self.emitter {
            // EventEmitter::emit returns unit (D10-008) — error isolation
            // happens internally; nothing to handle here.
            emitter.emit(&event).await;
        }
    }
}

#[async_trait]
impl Middleware for CircuitBreakerMiddleware {
    fn name(&self) -> &'static str {
        "circuit_breaker"
    }

    fn priority(&self) -> u16 {
        self.config.priority
    }

    async fn before(
        &self,
        module_id: &str,
        _inputs: serde_json::Value,
        ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        let key = Self::key_of(module_id, ctx);
        let now = (self.clock)();

        // Decide the action under lock; defer event emission until lock release.
        let recovery = Duration::from_millis(self.config.recovery_window_ms);
        let outcome = {
            let mut circuits = self.circuits.lock();
            let circuit = circuits.entry(key.clone()).or_insert_with(Circuit::new);

            // Possibly transition OPEN → HALF_OPEN if recovery window elapsed.
            if circuit.state == CircuitBreakerState::Open {
                if let Some(opened_at) = circuit.opened_at {
                    let elapsed = now.saturating_duration_since(opened_at);
                    if elapsed >= recovery {
                        circuit.state = CircuitBreakerState::HalfOpen;
                        circuit.probe_in_flight = false;
                        circuit.probe_started_at = None;
                    }
                }
            }

            // Reclaim a stale HALF_OPEN probe slot if the prior probe never
            // completed (cancellation, panic) and has been held longer than
            // the recovery window.
            if circuit.state == CircuitBreakerState::HalfOpen && circuit.probe_in_flight {
                if let Some(started) = circuit.probe_started_at {
                    if now.saturating_duration_since(started) >= recovery {
                        circuit.probe_in_flight = false;
                        circuit.probe_started_at = None;
                    }
                }
            }

            match circuit.state {
                CircuitBreakerState::Open => Outcome::Reject,
                CircuitBreakerState::HalfOpen => {
                    if circuit.probe_in_flight {
                        Outcome::Reject
                    } else {
                        circuit.probe_in_flight = true;
                        circuit.probe_started_at = Some(now);
                        Outcome::Allow(CircuitBreakerState::HalfOpen)
                    }
                }
                CircuitBreakerState::Closed => Outcome::Allow(CircuitBreakerState::Closed),
            }
        };

        match outcome {
            Outcome::Reject => {
                // Record the rejected state on the context for observability.
                Self::write_state_to_context(ctx, CircuitBreakerState::Open);
                Err(ModuleError::circuit_breaker_open(&key.0, &key.1))
            }
            Outcome::Allow(state) => {
                Self::write_state_to_context(ctx, state);
                Ok(None)
            }
        }
    }

    async fn after(
        &self,
        module_id: &str,
        _inputs: serde_json::Value,
        _output: serde_json::Value,
        ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        let key = Self::key_of(module_id, ctx);
        let event = {
            let mut circuits = self.circuits.lock();
            let circuit = circuits.entry(key.clone()).or_insert_with(Circuit::new);
            circuit.push(true, self.config.window_size);

            if circuit.state == CircuitBreakerState::HalfOpen {
                circuit.state = CircuitBreakerState::Closed;
                circuit.opened_at = None;
                circuit.probe_in_flight = false;
                circuit.probe_started_at = None;
                // Reset the rolling window — the prior failures that opened
                // the breaker are stale; future trips must accumulate again
                // before re-opening.
                circuit.window.clear();
                Some(Self::build_event(
                    "apcore.circuit.closed",
                    &key.0,
                    &key.1,
                    0.0,
                    "info",
                ))
            } else {
                None
            }
        };

        if let Some(event) = event {
            self.emit(event).await;
        }
        Ok(None)
    }

    async fn on_error(
        &self,
        module_id: &str,
        _inputs: serde_json::Value,
        error: &ModuleError,
        ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        // Don't double-count our own short-circuits.
        if error.code == crate::errors::ErrorCode::CircuitBreakerOpen {
            return Ok(None);
        }

        let key = Self::key_of(module_id, ctx);
        let now = (self.clock)();
        let event = {
            let mut circuits = self.circuits.lock();
            let circuit = circuits.entry(key.clone()).or_insert_with(Circuit::new);
            circuit.push(false, self.config.window_size);

            let rate = circuit.error_rate();
            let opens = match circuit.state {
                CircuitBreakerState::HalfOpen => true,
                CircuitBreakerState::Closed => {
                    circuit.window.len() >= self.config.min_samples
                        && rate >= self.config.open_threshold
                }
                CircuitBreakerState::Open => false,
            };

            if opens {
                circuit.state = CircuitBreakerState::Open;
                circuit.opened_at = Some(now);
                circuit.probe_in_flight = false;
                circuit.probe_started_at = None;
                Some(Self::build_event(
                    "apcore.circuit.opened",
                    &key.0,
                    &key.1,
                    rate,
                    "warn",
                ))
            } else {
                None
            }
        };

        if let Some(event) = event {
            self.emit(event).await;
        }
        Ok(None)
    }
}

/// Decision computed under the per-circuit lock in `before()`.
/// File-private — callers operate via `Middleware::before`.
enum Outcome {
    Allow(CircuitBreakerState),
    Reject,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{Context, Identity};
    use crate::errors::ErrorCode;

    fn ctx_with_caller(caller: &str) -> Context<serde_json::Value> {
        let identity = Identity::new(
            "test".to_string(),
            "user".to_string(),
            vec![],
            std::collections::HashMap::new(),
        );
        let mut ctx: Context<serde_json::Value> = Context::new(identity);
        ctx.caller_id = Some(caller.to_string());
        ctx
    }

    #[tokio::test]
    async fn closed_circuit_allows_calls() {
        let mw = CircuitBreakerMiddleware::builder().build();
        let ctx = ctx_with_caller("orchestrator.x");
        let result = mw.before("executor.foo", serde_json::json!({}), &ctx).await;
        assert!(result.is_ok());
        assert_eq!(
            mw.state("executor.foo", "orchestrator.x"),
            CircuitBreakerState::Closed
        );
    }

    #[tokio::test]
    async fn opens_when_error_rate_exceeds_threshold() {
        let mw = CircuitBreakerMiddleware::builder()
            .open_threshold(0.5)
            .window_size(10)
            .min_samples(5)
            .build();
        let ctx = ctx_with_caller("orchestrator.billing");
        let module = "executor.payment.charge";
        let err = ModuleError::new(ErrorCode::ModuleExecuteError, "boom");

        // 6 errors followed by 4 successes in a window of 10:
        // error_rate = 6/10 = 0.6 ≥ threshold 0.5 → OPEN.
        for _ in 0..6 {
            mw.on_error(module, serde_json::json!({}), &err, &ctx)
                .await
                .unwrap();
        }
        assert_eq!(
            mw.state(module, "orchestrator.billing"),
            CircuitBreakerState::Open
        );
    }

    #[tokio::test]
    async fn open_state_short_circuits_before() {
        let mw = CircuitBreakerMiddleware::builder().build();
        let ctx = ctx_with_caller("orchestrator.billing");
        let module = "executor.payment.charge";
        // Force open with a recent opened_at so recovery hasn't elapsed.
        mw.force_state(
            module,
            "orchestrator.billing",
            CircuitBreakerState::Open,
            Some(Instant::now()),
        );
        let result = mw.before(module, serde_json::json!({}), &ctx).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, ErrorCode::CircuitBreakerOpen);
    }

    #[tokio::test]
    async fn open_to_half_open_after_recovery_window() {
        let mw = CircuitBreakerMiddleware::builder()
            .recovery_window_ms(30_000)
            .build();
        let ctx = ctx_with_caller("orchestrator.billing");
        let module = "executor.payment.charge";
        let opened_at = Instant::now()
            .checked_sub(Duration::from_secs(35))
            .expect("test clock far enough from epoch");
        mw.force_state(
            module,
            "orchestrator.billing",
            CircuitBreakerState::Open,
            Some(opened_at),
        );
        // before() must transition to HALF_OPEN and let the probe through.
        let r = mw.before(module, serde_json::json!({}), &ctx).await;
        assert!(r.is_ok());
        assert_eq!(
            mw.state(module, "orchestrator.billing"),
            CircuitBreakerState::HalfOpen
        );
    }

    #[tokio::test]
    async fn half_open_concurrent_probes_capped_at_one() {
        let mw = CircuitBreakerMiddleware::builder().build();
        let ctx = ctx_with_caller("orchestrator.billing");
        let module = "executor.payment.charge";
        mw.force_state(
            module,
            "orchestrator.billing",
            CircuitBreakerState::HalfOpen,
            Some(Instant::now()),
        );
        // First probe is allowed.
        let first = mw.before(module, serde_json::json!({}), &ctx).await;
        assert!(first.is_ok());
        // Second concurrent probe is rejected with CIRCUIT_OPEN.
        let second = mw.before(module, serde_json::json!({}), &ctx).await;
        assert!(second.is_err());
        assert_eq!(second.unwrap_err().code, ErrorCode::CircuitBreakerOpen);
    }

    #[tokio::test]
    async fn half_open_success_closes_circuit() {
        let mw = CircuitBreakerMiddleware::builder().build();
        let ctx = ctx_with_caller("orchestrator.billing");
        let module = "executor.payment.charge";
        mw.force_state(
            module,
            "orchestrator.billing",
            CircuitBreakerState::HalfOpen,
            Some(Instant::now()),
        );
        // Take the probe slot via before().
        mw.before(module, serde_json::json!({}), &ctx)
            .await
            .unwrap();
        // Successful response closes the circuit.
        mw.after(module, serde_json::json!({}), serde_json::json!({}), &ctx)
            .await
            .unwrap();
        assert_eq!(
            mw.state(module, "orchestrator.billing"),
            CircuitBreakerState::Closed
        );
    }

    #[tokio::test]
    async fn writes_state_to_context_data() {
        let mw = CircuitBreakerMiddleware::builder().build();
        let ctx = ctx_with_caller("orch");
        mw.before("mod.a", serde_json::json!({}), &ctx)
            .await
            .unwrap();
        let data = ctx.data.read();
        assert_eq!(
            data.get("_apcore.mw.circuit.state")
                .and_then(|v| v.as_str()),
            Some("CLOSED")
        );
    }

    #[tokio::test]
    async fn reset_returns_open_circuit_to_closed_with_cleared_window() {
        // [mw-reset] reset() must return an OPEN circuit to CLOSED and clear its
        // rolling window (parity with Python/TS reset()).
        let mw = CircuitBreakerMiddleware::builder()
            .open_threshold(0.5)
            .window_size(10)
            .min_samples(5)
            .build();
        let ctx = ctx_with_caller("orch");
        let module = "mod.a";
        let err = ModuleError::new(ErrorCode::ModuleExecuteError, "boom");

        // Accumulate enough errors to OPEN the circuit (fills the window).
        for _ in 0..6 {
            mw.on_error(module, serde_json::json!({}), &err, &ctx)
                .await
                .unwrap();
        }
        assert_eq!(mw.state(module, "orch"), CircuitBreakerState::Open);

        // Reset: back to CLOSED.
        mw.reset(module, Some("orch"));
        assert_eq!(mw.state(module, "orch"), CircuitBreakerState::Closed);

        // The window must be cleared: a single new failure cannot re-open the
        // circuit because min_samples (5) is no longer met by the empty window.
        mw.on_error(module, serde_json::json!({}), &err, &ctx)
            .await
            .unwrap();
        assert_eq!(
            mw.state(module, "orch"),
            CircuitBreakerState::Closed,
            "window should be cleared, so one failure must not re-open"
        );
    }

    #[tokio::test]
    async fn ignores_circuit_breaker_open_in_on_error() {
        let mw = CircuitBreakerMiddleware::builder()
            .open_threshold(0.5)
            .min_samples(2)
            .build();
        let ctx = ctx_with_caller("orch");
        let err = ModuleError::circuit_breaker_open("mod.a", "orch");
        mw.on_error("mod.a", serde_json::json!({}), &err, &ctx)
            .await
            .unwrap();
        // The breaker should NOT count its own short-circuit.
        assert_eq!(mw.state("mod.a", "orch"), CircuitBreakerState::Closed);
    }
}
