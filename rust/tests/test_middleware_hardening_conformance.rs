//! Cross-language conformance tests for Middleware Architecture Hardening
//! (Issue #42).
//!
//! Fixture source: apcore/conformance/fixtures/middleware_hardening.json
//! Spec reference: apcore/docs/features/middleware-system.md
//! (## Middleware Architecture Hardening)
//!
//! Each fixture case verifies one normative rule:
//! - context namespace partitioning (`_apcore.*` vs `ext.*`),
//! - the `CircuitBreakerMiddleware` state machine and event emission,
//! - the `TracingMiddleware` no-op fallback when no telemetry is wired,
//! - the language-specific async-handler detection rule (Rust: static).
//!
//! The fixture's `tracing_span_created` case was REMOVED upstream along with
//! `middleware-system.md` §1.3: it pinned a `module_id`-named span and a
//! single `_apcore.mw.tracing.span_id` slot, both of which contradict
//! `protocol-spec.md` §12 (span `apcore.module.execute`, module id as an
//! attribute) and `observability.md` (a `_apcore.mw.tracing.spans` STACK with
//! explicit `parent_span_id` links). The driver for it is gone with it.

#![allow(clippy::missing_panics_doc)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::Value;

use apcore::context::{Context, Identity};
use apcore::errors::{ErrorCode, ModuleError};
use apcore::events::emitter::{ApCoreEvent, EventEmitter};
use apcore::events::subscribers::EventSubscriber;
use apcore::middleware::circuit_breaker::{CircuitBreakerMiddleware, CircuitBreakerState};
use apcore::middleware::context_namespace::{validate_context_key, ContextWriter};
use apcore::middleware::Middleware;
use apcore::observability::exporters::InMemoryExporter;
use apcore::observability::tracing_middleware::{SamplingStrategy, TracingMiddleware};

// ---------------------------------------------------------------------------
// Fixture loading (mirrors other conformance tests)
// ---------------------------------------------------------------------------

use crate::conformance_env::find_fixtures_root;

fn load_fixture() -> Value {
    let path = find_fixtures_root().join("middleware_hardening.json");
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

fn make_ctx(caller: &str) -> Context<Value> {
    let identity = Identity::new(
        "test".to_string(),
        "user".to_string(),
        vec![],
        std::collections::HashMap::new(),
    );
    let mut ctx: Context<Value> = Context::new(identity);
    ctx.caller_id = Some(caller.to_string());
    ctx
}

// Recording subscriber that captures every event by name. Used to assert
// `apcore.circuit.opened` / `apcore.circuit.closed` emission.
#[derive(Debug, Default)]
struct RecordingSubscriber {
    events: Mutex<Vec<ApCoreEvent>>,
}

impl RecordingSubscriber {
    fn captured(&self) -> Vec<ApCoreEvent> {
        self.events.lock().clone()
    }

    /// Poll until an event of `event_type` is captured or a short timeout
    /// elapses. Needed because `EventEmitter::emit` now dispatches deliveries
    /// on spawned tasks (A-D-024), so events arrive asynchronously.
    async fn wait_for_event(&self, event_type: &str) {
        for _ in 0..200 {
            if self
                .events
                .lock()
                .iter()
                .any(|e| e.event_type == event_type)
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }
}

#[derive(Debug)]
struct RecordingHandle {
    inner: Arc<RecordingSubscriber>,
    id: String,
}

#[async_trait]
impl EventSubscriber for RecordingHandle {
    fn subscriber_id(&self) -> &str {
        &self.id
    }
    #[allow(clippy::unnecessary_literal_bound)]
    fn event_pattern(&self) -> &str {
        "*"
    }
    async fn on_event(&self, event: &ApCoreEvent) -> Result<(), ModuleError> {
        self.inner.events.lock().push(event.clone());
        Ok(())
    }
}

fn build_emitter_with_recorder() -> (Arc<EventEmitter>, Arc<RecordingSubscriber>) {
    let recorder = Arc::new(RecordingSubscriber::default());
    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(RecordingHandle {
        inner: Arc::clone(&recorder),
        id: "recorder".to_string(),
    }));
    (Arc::new(emitter), recorder)
}

// ---------------------------------------------------------------------------
// Case 1 — context_namespace_apcore_prefix
// ---------------------------------------------------------------------------

#[test]
fn conformance_context_namespace_apcore_prefix() {
    let fixture = load_fixture();
    let case = fixture_case(&fixture, "context_namespace_apcore_prefix");
    let writer = case["input"]["writer"].as_str().unwrap();
    let key = case["input"]["key"].as_str().unwrap();
    assert_eq!(writer, "framework");

    let check = validate_context_key(ContextWriter::Framework, key);
    assert!(check.valid);
    assert!(!check.warning);

    assert_eq!(case["expected"]["valid"].as_bool(), Some(true));
    assert_eq!(case["expected"]["warning"].as_bool(), Some(false));
}

// ---------------------------------------------------------------------------
// Case 2 — context_namespace_ext_prefix
// ---------------------------------------------------------------------------

#[test]
fn conformance_context_namespace_ext_prefix() {
    let fixture = load_fixture();
    let case = fixture_case(&fixture, "context_namespace_ext_prefix");
    let writer = case["input"]["writer"].as_str().unwrap();
    let key = case["input"]["key"].as_str().unwrap();
    assert_eq!(writer, "user");

    let check = validate_context_key(ContextWriter::User, key);
    assert!(check.valid);
    assert!(!check.warning);

    assert_eq!(case["expected"]["valid"].as_bool(), Some(true));
    assert_eq!(case["expected"]["warning"].as_bool(), Some(false));
}

// ---------------------------------------------------------------------------
// Case 3 — context_namespace_violation
// ---------------------------------------------------------------------------

#[test]
fn conformance_context_namespace_violation() {
    let fixture = load_fixture();
    let case = fixture_case(&fixture, "context_namespace_violation");
    let writer = case["input"]["writer"].as_str().unwrap();
    let key = case["input"]["key"].as_str().unwrap();
    assert_eq!(writer, "user");

    let check = validate_context_key(ContextWriter::User, key);
    assert!(!check.valid);
    assert!(check.warning);

    assert_eq!(case["expected"]["valid"].as_bool(), Some(false));
    assert_eq!(case["expected"]["warning"].as_bool(), Some(true));
}

// ---------------------------------------------------------------------------
// Case 4 — circuit_breaker_opens_at_threshold
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conformance_circuit_breaker_opens_at_threshold() {
    let fixture = load_fixture();
    let case = fixture_case(&fixture, "circuit_breaker_opens_at_threshold");
    let module_id = case["input"]["module_id"].as_str().unwrap();
    let caller_id = case["input"]["caller_id"].as_str().unwrap();
    let open_threshold = case["input"]["open_threshold"].as_f64().unwrap();
    let window_size = usize::try_from(case["input"]["window_size"].as_u64().unwrap()).unwrap();
    let errors_in_window = case["input"]["errors_in_window"].as_u64().unwrap();
    let successes_in_window = case["input"]["successes_in_window"].as_u64().unwrap();

    let (emitter, recorder) = build_emitter_with_recorder();
    let mw = CircuitBreakerMiddleware::builder()
        .open_threshold(open_threshold)
        .window_size(window_size)
        .min_samples(1)
        .emitter(Arc::clone(&emitter))
        .build();

    let mut ctx = make_ctx(caller_id);
    // Some calls will be rejected once the circuit opens; force caller bucket.
    ctx.caller_id = Some(caller_id.to_string());

    let err = ModuleError::new(ErrorCode::ModuleExecuteError, "fixture-error");

    // Interleave successes and errors so the rolling window contains exactly
    // `errors_in_window` errors and `successes_in_window` successes.
    for _ in 0..successes_in_window {
        mw.after(module_id, Value::Null, Value::Null, &ctx)
            .await
            .unwrap();
    }
    for _ in 0..errors_in_window {
        mw.on_error(module_id, Value::Null, &err, &ctx)
            .await
            .unwrap();
    }

    assert_eq!(
        mw.state(module_id, caller_id),
        CircuitBreakerState::Open,
        "expected circuit OPEN after {errors_in_window} errors / {successes_in_window} successes"
    );
    assert_eq!(case["expected"]["circuit_state"].as_str(), Some("OPEN"),);

    recorder.wait_for_event("apcore.circuit.opened").await;
    let captured = recorder.captured();
    assert!(
        captured
            .iter()
            .any(|e| e.event_type == "apcore.circuit.opened"),
        "expected apcore.circuit.opened to be emitted; got {:?}",
        captured.iter().map(|e| &e.event_type).collect::<Vec<_>>(),
    );
    assert_eq!(
        case["expected"]["event_emitted"].as_str(),
        Some("apcore.circuit.opened"),
    );
}

// ---------------------------------------------------------------------------
// Case 5 — circuit_breaker_short_circuits_open
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conformance_circuit_breaker_short_circuits_open() {
    let fixture = load_fixture();
    let case = fixture_case(&fixture, "circuit_breaker_short_circuits_open");
    let module_id = case["input"]["module_id"].as_str().unwrap();
    let caller_id = case["input"]["caller_id"].as_str().unwrap();
    let recovery_window_ms = case["input"]["recovery_window_ms"].as_u64().unwrap();
    let ms_since_opened = case["input"]["ms_since_opened"].as_u64().unwrap();

    let mw = CircuitBreakerMiddleware::builder()
        .recovery_window_ms(recovery_window_ms)
        .build();

    // Force OPEN with opened_at set so the recovery window has NOT elapsed.
    let opened_at = Instant::now()
        .checked_sub(Duration::from_millis(ms_since_opened))
        .expect("test clock far enough from epoch");
    mw.force_state(
        module_id,
        caller_id,
        CircuitBreakerState::Open,
        Some(opened_at),
    );

    let mut ctx = make_ctx(caller_id);
    ctx.caller_id = Some(caller_id.to_string());

    let result = mw.before(module_id, Value::Null, &ctx).await;
    let result_was_err = result.is_err();
    assert!(
        result_was_err,
        "before() must short-circuit while OPEN within the recovery window"
    );
    let err = result.unwrap_err();

    // CORRECTED: this previously read `expected["error"]` and compared it to the
    // literal "CircuitBreakerOpenError" — fixture text against a string in the
    // test, with the SDK never consulted. It stayed green no matter what
    // `before()` returned. The contract is the WIRE CODE, which is the only
    // thing all three SDKs can carry (apcore-rust has no error CLASS hierarchy,
    // only `ErrorCode` variants), so assert the code the SDK actually produced
    // against the code the fixture declares.
    let actual_code = serde_json::to_value(err.code).expect("ErrorCode serializes");
    assert_eq!(
        actual_code, case["expected"]["error_code"],
        "wire code produced by CircuitBreakerMiddleware::before must match the fixture"
    );

    // `module_reached: false` — the short-circuit means `before()` returned Err,
    // so the pipeline never reached the module. Drive the check off the fixture
    // value rather than asserting the fixture equals a literal.
    let module_reached = !result_was_err;
    assert_eq!(
        module_reached,
        case["expected"]["module_reached"].as_bool().unwrap(),
        "module_reached mismatch"
    );
}

// ---------------------------------------------------------------------------
// Case 6 — circuit_breaker_half_open_probe
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conformance_circuit_breaker_half_open_probe() {
    let fixture = load_fixture();
    let case = fixture_case(&fixture, "circuit_breaker_half_open_probe");
    let module_id = case["input"]["module_id"].as_str().unwrap();
    let caller_id = case["input"]["caller_id"].as_str().unwrap();
    let recovery_window_ms = case["input"]["recovery_window_ms"].as_u64().unwrap();
    let ms_since_opened = case["input"]["ms_since_opened"].as_u64().unwrap();
    assert!(
        ms_since_opened > recovery_window_ms,
        "fixture invariant: ms_since_opened > recovery_window_ms"
    );

    let mw = CircuitBreakerMiddleware::builder()
        .recovery_window_ms(recovery_window_ms)
        .build();

    let opened_at = Instant::now()
        .checked_sub(Duration::from_millis(ms_since_opened))
        .expect("test clock far enough from epoch");
    mw.force_state(
        module_id,
        caller_id,
        CircuitBreakerState::Open,
        Some(opened_at),
    );

    let mut ctx = make_ctx(caller_id);
    ctx.caller_id = Some(caller_id.to_string());

    // First call after recovery window → transitions to HALF_OPEN, allowed.
    let probe = mw.before(module_id, Value::Null, &ctx).await;
    assert!(probe.is_ok(), "first probe call must be allowed");
    assert_eq!(
        mw.state(module_id, caller_id),
        CircuitBreakerState::HalfOpen,
    );
    assert_eq!(
        case["expected"]["circuit_state"].as_str(),
        Some("HALF_OPEN"),
    );
    assert_eq!(case["expected"]["probe_call_allowed"].as_bool(), Some(true));

    // Second concurrent probe must be rejected (max_concurrent_probes = 1).
    let second = mw.before(module_id, Value::Null, &ctx).await;
    assert!(second.is_err());
    assert_eq!(second.unwrap_err().code, ErrorCode::CircuitBreakerOpen,);
    assert_eq!(case["expected"]["max_concurrent_probes"].as_u64(), Some(1),);
}

// ---------------------------------------------------------------------------
// Case 7 — circuit_breaker_closes_on_success
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conformance_circuit_breaker_closes_on_success() {
    let fixture = load_fixture();
    let case = fixture_case(&fixture, "circuit_breaker_closes_on_success");
    let module_id = case["input"]["module_id"].as_str().unwrap();
    let caller_id = case["input"]["caller_id"].as_str().unwrap();

    let (emitter, recorder) = build_emitter_with_recorder();
    let mw = CircuitBreakerMiddleware::builder()
        .emitter(Arc::clone(&emitter))
        .build();

    mw.force_state(
        module_id,
        caller_id,
        CircuitBreakerState::HalfOpen,
        Some(Instant::now()),
    );

    let mut ctx = make_ctx(caller_id);
    ctx.caller_id = Some(caller_id.to_string());

    // Take the probe slot, then complete successfully.
    mw.before(module_id, Value::Null, &ctx).await.unwrap();
    mw.after(module_id, Value::Null, Value::Null, &ctx)
        .await
        .unwrap();

    assert_eq!(mw.state(module_id, caller_id), CircuitBreakerState::Closed,);
    assert_eq!(case["expected"]["circuit_state"].as_str(), Some("CLOSED"),);

    recorder.wait_for_event("apcore.circuit.closed").await;
    let captured = recorder.captured();
    assert!(
        captured
            .iter()
            .any(|e| e.event_type == "apcore.circuit.closed"),
        "expected apcore.circuit.closed to be emitted; got {:?}",
        captured.iter().map(|e| &e.event_type).collect::<Vec<_>>(),
    );
    assert_eq!(
        case["expected"]["event_emitted"].as_str(),
        Some("apcore.circuit.closed"),
    );
}

// ---------------------------------------------------------------------------
// Case 8 — tracing_noop_without_otel
//
// The fixture's `otel_available: false` is a language-neutral way of saying
// "no telemetry pipeline is wired up". apcore-rust's TracingMiddleware links
// against no OpenTelemetry SDK at all, so the Rust reading of that condition
// is "nothing is being sampled or exported": every hook must still return
// Ok(()) and the call must proceed, with no span reaching an exporter.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conformance_tracing_noop_without_otel() {
    let fixture = load_fixture();
    let case = fixture_case(&fixture, "tracing_noop_without_otel");
    let module_id = case["input"]["module_id"].as_str().unwrap();
    let caller_id = case["input"]["caller_id"].as_str().unwrap();
    assert_eq!(case["input"]["otel_available"].as_bool(), Some(false));

    let exporter = InMemoryExporter::new();
    let mw =
        TracingMiddleware::with_sampling(Box::new(exporter.clone()), SamplingStrategy::Never, 0.0);

    let mut ctx = make_ctx(caller_id);
    ctx.caller_id = Some(caller_id.to_string());

    let before = mw.before(module_id, Value::Null, &ctx).await;
    let after = mw.after(module_id, Value::Null, Value::Null, &ctx).await;
    let on_err = mw
        .on_error(
            module_id,
            Value::Null,
            &ModuleError::new(ErrorCode::ModuleExecuteError, "x"),
            &ctx,
        )
        .await;

    assert!(before.is_ok());
    assert!(after.is_ok());
    assert!(on_err.is_ok());
    assert_eq!(
        case["expected"]["error_raised"].as_bool(),
        Some(false),
        "no hook may raise when no telemetry backend is present"
    );

    let span_created = !exporter.get_spans().is_empty();
    assert_eq!(
        span_created,
        case["expected"]["span_created"].as_bool().unwrap(),
        "no span may be exported when nothing is sampled"
    );

    // `execution_continues` — the pipeline is not short-circuited: `before()`
    // returned Ok(None), i.e. inputs pass through unmodified.
    let execution_continues = before.unwrap().is_none();
    assert_eq!(
        execution_continues,
        case["expected"]["execution_continues"].as_bool().unwrap(),
    );
}

// ---------------------------------------------------------------------------
// Case 9 — async_detection_coroutine_function
//
// Rust enforces this rule statically — async fns differ from sync fns at the
// type level (they return `impl Future`). There is no runtime detection
// path to test, so we assert the fixture's premises are language-correct
// and let the static type system carry the conformance guarantee.
// ---------------------------------------------------------------------------

#[test]
fn conformance_async_detection_coroutine_function() {
    let fixture = load_fixture();
    let case = fixture_case(&fixture, "async_detection_coroutine_function");
    // The case carries Python detection guidance; Rust satisfies this rule
    // structurally via `async_trait` and the `Future` trait. The fixture's
    // `is_async` field describes the expected truth value when the language
    // does perform detection — for Rust, the equivalent compile-time fact
    // is that any handler reaching the middleware pipeline is already typed
    // as async (see `Middleware::before/after/on_error` signatures).
    assert_eq!(case["expected"]["is_async"].as_bool(), Some(true));
    assert_eq!(
        case["expected"]["incorrect_method_result"]["result_on_uncalled_function"].as_bool(),
        Some(false),
    );

    // Compile-time witness: the trait methods are async. If this file
    // compiles, the rule is satisfied.
    assert_async_signatures::<TracingMiddleware>();
    assert_async_signatures::<CircuitBreakerMiddleware>();
}

fn assert_async_signatures<M: Middleware>() {}

// ---------------------------------------------------------------------------
// D-114 — `remove` clears the duplicate-identity entry
// ---------------------------------------------------------------------------
//
// The registry records the FIRST registration so a later duplicate can be traced
// back to it. A stale entry corrupts that record in both directions: it names a
// registration that no longer exists, and `use` / `remove` / `use` — a
// legitimate swap — warns about a duplicate that is not one, which is how an
// operator learns to ignore the warning that will next fire for a real one.
//
// Observed through `identity_registered`, which is the public accessor for the
// duplicate-detection state. Counting log lines would pin the tracing setup
// rather than the decision, and the fixture states the count because the peers
// have no such accessor.

#[derive(Debug)]
struct IdentityProbeMiddleware;

#[async_trait]
impl apcore::middleware::Middleware for IdentityProbeMiddleware {
    fn name(&self) -> &str {
        "audit"
    }

    // The hooks are never invoked: this probe exists to occupy a slot in the
    // duplicate-identity registry, which is what the decision is about.
    async fn before(
        &self,
        _module_id: &str,
        _inputs: Value,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        Ok(None)
    }

    async fn after(
        &self,
        _module_id: &str,
        _inputs: Value,
        _output: Value,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        Ok(None)
    }

    async fn on_error(
        &self,
        _module_id: &str,
        _inputs: Value,
        _error: &ModuleError,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        Ok(None)
    }
}

fn identity_probe_manager() -> apcore::middleware::MiddlewareManager {
    apcore::middleware::MiddlewareManager::new()
}

/// A registration with an EXPLICIT identity key, so the two probe registrations
/// share an identity the way two instances of one middleware type would.
fn identity_probe_registration(
) -> apcore::middleware::MiddlewareRegistration<IdentityProbeMiddleware> {
    let mut reg = apcore::middleware::MiddlewareRegistration::new(IdentityProbeMiddleware);
    reg.identity_key = Some("audit".to_string());
    reg
}

/// Replay the fixture's `use` / `remove` sequence and count the duplicate-identity
/// warnings it would emit. `add_with_opts` warns exactly when the identity is
/// already recorded (and `allow_duplicate` is off, as it is here), so the count
/// is read off `identity_registered` immediately before each `use`.
fn replay_duplicate_warnings(manager: &apcore::middleware::MiddlewareManager, case: &Value) -> u64 {
    let name = case["input"]["middleware_name"].as_str().unwrap();
    let mut warnings = 0;
    for op in case["input"]["sequence"].as_array().expect("sequence") {
        match op.as_str().expect("sequence entry") {
            "use" => {
                if manager.identity_registered(name) {
                    warnings += 1;
                }
                manager
                    .add_with_opts(identity_probe_registration())
                    .expect("register");
            }
            "remove" => {
                manager.remove(name);
            }
            other => panic!("unknown sequence entry {other}"),
        }
    }
    warnings
}

#[test]
fn case_remove_clears_the_duplicate_identity_entry() {
    let fixture = load_fixture();
    let case = fixture_case(&fixture, "remove_clears_the_duplicate_identity_entry");
    let name = case["input"]["middleware_name"].as_str().unwrap();

    let manager = identity_probe_manager();
    manager
        .add_with_opts(identity_probe_registration())
        .expect("register");
    assert!(
        manager.identity_registered("audit"),
        "precondition: recorded"
    );

    manager.remove(name);

    assert!(
        !manager.identity_registered("audit"),
        "[{}] a removed registration must not remain in the duplicate-identity registry: \
         the next `use` of the same identity would be reported as colliding with a \
         registration that no longer exists",
        case["id"].as_str().unwrap()
    );

    let replayed = replay_duplicate_warnings(&identity_probe_manager(), case);
    assert_eq!(
        replayed,
        case["expected"]["duplicate_warnings"].as_u64().unwrap(),
        "[{}] duplicate-identity warnings emitted by the sequence",
        case["id"].as_str().unwrap()
    );
}

#[test]
fn case_use_twice_still_warns_about_the_duplicate() {
    // The control. Without it an SDK that simply stopped recording identities
    // passes, and the decision is about CLEARING on removal, not about dropping
    // duplicate detection.
    let fixture = load_fixture();
    let case = fixture_case(&fixture, "use_twice_still_warns_about_the_duplicate");

    let replayed = replay_duplicate_warnings(&identity_probe_manager(), case);
    assert_eq!(
        replayed,
        case["expected"]["duplicate_warnings"].as_u64().unwrap(),
        "[{}] duplicate-identity warnings emitted by the sequence",
        case["id"].as_str().unwrap()
    );

    let manager = identity_probe_manager();
    manager
        .add_with_opts(identity_probe_registration())
        .expect("register");
    manager
        .add_with_opts(identity_probe_registration())
        .expect("register duplicate");

    assert!(
        manager.identity_registered("audit"),
        "[{}] a genuine duplicate must still be detectable",
        case["id"].as_str().unwrap()
    );
    // And removing ONE of two registrations sharing an identity must NOT clear
    // it — duplicate registration warns but succeeds, so this state is reachable
    // and the survivor has to stay visible to duplicate detection.
    manager.remove("audit");
    assert!(
        manager.identity_registered("audit"),
        "[{}] removing one of two registrations sharing an identity must leave the \
         survivor detectable",
        case["id"].as_str().unwrap()
    );
}
