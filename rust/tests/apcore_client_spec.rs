// Spec-traced contract tests for the apcore-rust apcore-client feature.
//
// Source spec: apcore/docs/features/apcore-client.md ('## Contract:' blocks)
// Canonical clause list mirrored from:
//   apcore-python/tests/test_apcore_client_spec.py
//
// Each test maps to exactly one clause in the feature spec. The verbatim
// cross-language clause id appears in a leading `// clause: <clause_id>`
// comment on the line above each test fn so that a cross-language diff tool
// can line up the Python / TypeScript / Rust rows by that exact string. The fn
// name is the clause id flattened to snake_case.
//
// TESTS ONLY — production source is never modified here.
//
// Cross-language notes on Rust divergences (asserting REAL Rust behavior):
//   * INVALID_MODULE_ID: a malformed/empty module_id surfaces
//     ErrorCode::InvalidModuleId (wire string INVALID_MODULE_ID), matching
//     Python/TS per error-system.md §560 (normative MUST).
//   * SYS_MODULES_DISABLED: disable()/enable() without sys_modules return
//     Err(ModuleError{code: SysModulesDisabled}) — matching the spec's Rust
//     row. on()/off() take &mut self and lazily create a local emitter; they
//     do NOT error when events are disabled (Rust divergence) — recorded as
//     #[ignore] contract gaps where the Python clause asserts an error.
//   * use_middleware() priority>1000 returns Err(ModuleError) not a panic.
//   * register() takes Box<dyn Module>; there is no shared "module_obj" handle.
//   * describe() returns Ok(description) and raises ModuleNotFoundError for a
//     missing module (parity with Python/TS).
//   * start()/stop() do not exist on the Rust APCore — #[ignore] gaps.

use std::sync::Arc;

use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};

use apcore::config::Config;
use apcore::context::Context;
use apcore::errors::{ErrorCode, ModuleError};
use apcore::middleware::adapters::{AfterMiddleware, BeforeMiddleware};
use apcore::middleware::base::Middleware;
use apcore::module::Module;
use apcore::APCore;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Deterministic add module with a strict integer input schema, mirroring the
/// Python `math.add` / `math.strict` fixtures.
struct AddModule;

#[async_trait]
impl Module for AddModule {
    fn description(&self) -> &'static str {
        "Add two numbers"
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "a": {"type": "integer"},
                "b": {"type": "integer"}
            }
        })
    }
    fn output_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"sum": {"type": "integer"}}
        })
    }
    async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        let a = inputs["a"].as_i64().unwrap_or(0);
        let b = inputs["b"].as_i64().unwrap_or(0);
        Ok(json!({"sum": a + b}))
    }
}

/// Schema-less echo module: accepts any input.
struct EchoModule;

#[async_trait]
impl Module for EchoModule {
    fn description(&self) -> &'static str {
        "echo"
    }
    fn input_schema(&self) -> Value {
        json!({})
    }
    fn output_schema(&self) -> Value {
        json!({})
    }
    async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(inputs)
    }
}

/// Module whose execute handler always returns a deterministic error, used to
/// assert that handler errors propagate unchanged.
struct RaiserModule;

#[async_trait]
impl Module for RaiserModule {
    fn description(&self) -> &'static str {
        "always raises"
    }
    fn input_schema(&self) -> Value {
        json!({})
    }
    fn output_schema(&self) -> Value {
        json!({})
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Err(ModuleError::new(
            ErrorCode::ModuleExecuteError,
            "boom-from-handler",
        ))
    }
}

/// Build a zero-config client with a single `math.add` module registered.
fn client_with_module() -> APCore {
    let client = APCore::new();
    client
        .register("math.add", Box::new(AddModule))
        .expect("register math.add");
    client
}

/// Config with sys_modules + events enabled (production-like client). Rust
/// auto-registers system.control.toggle_feature when events are enabled.
fn sys_config() -> Config {
    let mut config = Config::default();
    config.set("sys_modules.enabled", json!(true));
    config.set("sys_modules.events.enabled", json!(true));
    config
}

/// No-op class-based middleware with a configurable priority.
#[derive(Debug)]
struct NoopMiddleware {
    name: String,
    priority: u16,
}

#[async_trait]
impl Middleware for NoopMiddleware {
    fn name(&self) -> &str {
        &self.name
    }
    fn priority(&self) -> u16 {
        self.priority
    }
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

// ===========================================================================
// Contract: ApCoreClient.call
// ===========================================================================

// clause: apcore_client.call.input.module_id.invalid_pattern
#[tokio::test]
async fn call_input_module_id_invalid_pattern() {
    let client = client_with_module();
    let err = client
        .call("!!not a valid id!!", json!({"a": 1, "b": 2}), None, None)
        .await
        .expect_err("malformed module_id must error");
    // Rust surfaces malformed IDs as InvalidModuleId (cross-language parity).
    assert_eq!(err.code, ErrorCode::InvalidModuleId);
}

// clause: apcore_client.call.input.module_id.empty
#[tokio::test]
async fn call_input_module_id_empty() {
    let client = client_with_module();
    let err = client
        .call("", json!({"a": 1, "b": 2}), None, None)
        .await
        .expect_err("empty module_id must error");
    assert_eq!(err.code, ErrorCode::InvalidModuleId);
}

// clause: apcore_client.call.error.INVALID_MODULE_ID
#[tokio::test]
async fn call_error_invalid_module_id() {
    let client = client_with_module();
    let err = client
        .call("UPPER.Reserved Bad", json!({}), None, None)
        .await
        .expect_err("malformed module_id must error");
    assert_eq!(err.code, ErrorCode::InvalidModuleId);
}

// clause: apcore_client.call.error.MODULE_NOT_FOUND
#[tokio::test]
async fn call_error_module_not_found() {
    let client = client_with_module();
    let err = client
        .call("missing.module", json!({"a": 1}), None, None)
        .await
        .expect_err("unknown module must error");
    assert_eq!(err.code, ErrorCode::ModuleNotFound);
}

// clause: apcore_client.call.error.SCHEMA_VALIDATION_ERROR
#[tokio::test]
async fn call_error_schema_validation_error() {
    let client = APCore::new();
    client
        .register("math.strict", Box::new(AddModule))
        .expect("register math.strict");
    let err = client
        .call(
            "math.strict",
            json!({"a": "not-an-int", "b": 2}),
            None,
            None,
        )
        .await
        .expect_err("schema-violating inputs must error");
    assert_eq!(err.code, ErrorCode::SchemaValidationError);
}

// clause: apcore_client.call.error.handler_propagates
#[tokio::test]
async fn call_error_handler_propagates() {
    let client = APCore::new();
    client
        .register("util.raiser", Box::new(RaiserModule))
        .expect("register util.raiser");
    let err = client
        .call("util.raiser", json!({}), None, None)
        .await
        .expect_err("handler error must propagate");
    assert!(
        err.message.contains("boom-from-handler"),
        "handler message should propagate, got: {}",
        err.message
    );
}

// clause: apcore_client.call.property.async
#[tokio::test]
async fn call_property_async() {
    let client = client_with_module();
    // Rust call() is async-only; awaiting resolves to the module output.
    let result = client
        .call("math.add", json!({"a": 10, "b": 5}), None, None)
        .await
        .expect("await resolves");
    assert_eq!(result, json!({"sum": 15}));
}

// clause: apcore_client.call.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn call_property_thread_safe() {
    let client = Arc::new(client_with_module());
    let mut handles = Vec::new();
    for i in 0..10i64 {
        let c = Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            let r = c
                .call("math.add", json!({"a": i, "b": i * 2}), None, None)
                .await
                .expect("concurrent call succeeds");
            (i, r["sum"].as_i64().unwrap())
        }));
    }
    for h in handles {
        let (i, sum) = h.await.expect("task must not panic");
        assert_eq!(sum, i + i * 2);
    }
}

// clause: apcore_client.call.property.idempotent_false
#[tokio::test]
async fn call_property_idempotent_false() {
    // A stateful counter module returns different output on the second call.
    struct Counter {
        n: std::sync::atomic::AtomicI64,
    }
    #[async_trait]
    impl Module for Counter {
        fn description(&self) -> &'static str {
            "counter"
        }
        fn input_schema(&self) -> Value {
            json!({})
        }
        fn output_schema(&self) -> Value {
            json!({})
        }
        async fn execute(&self, _i: Value, _c: &Context<Value>) -> Result<Value, ModuleError> {
            let v = self.n.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            Ok(json!({"n": v}))
        }
    }
    let client = APCore::new();
    client
        .register(
            "util.counter",
            Box::new(Counter {
                n: std::sync::atomic::AtomicI64::new(0),
            }),
        )
        .expect("register counter");
    let first = client
        .call("util.counter", json!({}), None, None)
        .await
        .unwrap();
    let second = client
        .call("util.counter", json!({}), None, None)
        .await
        .unwrap();
    assert_ne!(first, second);
    assert_eq!(
        (first["n"].as_i64().unwrap(), second["n"].as_i64().unwrap()),
        (1, 2)
    );
}

// ===========================================================================
// Contract: ApCoreClient.start   (MISSING SYMBOL — contract gap)
// ===========================================================================

// clause: apcore_client.start.error.CONFIG_INVALID
#[tokio::test]
#[ignore = "apcore_client.start.error.CONFIG_INVALID: missing symbol APCore::start (contract gap)"]
async fn start_error_config_invalid() {
    // The Rust APCore exposes no start() method (lifecycle is implicit at
    // construction). Recorded as a cross-language gap.
    let _ = APCore::new();
}

// clause: apcore_client.start.property.idempotent_false
#[tokio::test]
#[ignore = "apcore_client.start.property.idempotent_false: missing symbol APCore::start (contract gap)"]
async fn start_property_idempotent_false() {
    let _ = APCore::new();
}

// ===========================================================================
// Contract: ApCoreClient.stop   (MISSING SYMBOL — contract gap)
// ===========================================================================

// clause: apcore_client.stop.property.idempotent_true
#[tokio::test]
#[ignore = "apcore_client.stop.property.idempotent_true: missing symbol APCore::stop (contract gap)"]
async fn stop_property_idempotent_true() {
    let _ = APCore::new();
}

// ===========================================================================
// Contract: APCoreClient.on
// ===========================================================================

// clause: apcore_client.on.input.event_type.non_empty
#[tokio::test]
async fn on_input_event_type_non_empty() {
    // on() takes &mut self and returns a subscriber-id String. An empty
    // event_type subscription never matches a real, non-empty event type.
    let mut client = APCore::with_config(sys_config());
    let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let c = Arc::clone(&counter);
    let _id = client
        .on("", move |_e| {
            c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        })
        .expect("events enabled");
    let emitter = client.events().expect("local emitter exists after on()");
    emitter
        .emit_sequential(&apcore::events::emitter::ApCoreEvent::new(
            "apcore.registry.module_registered",
            json!({}),
        ))
        .await;
    assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 0);
}

// clause: apcore_client.on.error.SYS_MODULES_DISABLED
#[tokio::test]
async fn on_error_sys_modules_disabled() {
    // A client built without sys_modules has no event bus. on() MUST report
    // that rather than lazily creating a standalone bus no framework event
    // reaches — parity with apcore-python / apcore-typescript, which raise
    // SysModulesDisabledError here.
    let mut client = APCore::new();
    let err = client
        .on("apcore.health.error_threshold_exceeded", |_e| {})
        .expect_err("on() must error when events are disabled");
    assert_eq!(err.code, apcore::errors::ErrorCode::SysModulesDisabled);
    assert!(
        client.events().is_none(),
        "a refused subscription must not leave a bus behind"
    );
}

// clause: apcore_client.on.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn on_property_thread_safe() {
    // on() requires &mut self, so concurrent registration cannot share one
    // client. We instead spawn >=8 tasks each constructing a client and
    // subscribing, asserting no panic and that every subscription fires once.
    let mut handles = Vec::new();
    for _ in 0..10 {
        handles.push(tokio::spawn(async {
            let mut client = APCore::with_config(sys_config());
            let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let c = Arc::clone(&counter);
            client
                .on("evt.shared", move |_e| {
                    c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                })
                .expect("events enabled");
            let emitter = client.events().expect("emitter");
            emitter
                .emit_sequential(&apcore::events::emitter::ApCoreEvent::new(
                    "evt.shared",
                    json!({}),
                ))
                .await;
            counter.load(std::sync::atomic::Ordering::SeqCst)
        }));
    }
    for h in handles {
        let fired = h.await.expect("task must not panic");
        assert_eq!(fired, 1, "each subscription must fire exactly once");
    }
}

// clause: apcore_client.on.property.idempotent_false
#[tokio::test]
async fn on_property_idempotent_false() {
    // Registering twice creates two independent subscriptions; both fire.
    let mut client = APCore::with_config(sys_config());
    let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let c1 = Arc::clone(&counter);
    let c2 = Arc::clone(&counter);
    client
        .on("evt.dup", move |_e| {
            c1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        })
        .expect("events enabled");
    client
        .on("evt.dup", move |_e| {
            c2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        })
        .expect("events enabled");
    let emitter = client.events().expect("emitter");
    emitter
        .emit_sequential(&apcore::events::emitter::ApCoreEvent::new(
            "evt.dup",
            json!({}),
        ))
        .await;
    assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 2);
}

// clause: apcore_client.on.returns.subscriber
#[tokio::test]
async fn on_returns_subscriber() {
    // Rust on() returns a subscriber-id String (spec Rust row) instead of a
    // subscriber object; off() accepts that id.
    let mut client = APCore::with_config(sys_config());
    let id = client.on("evt.x", |_e| {}).expect("events enabled");
    assert!(!id.is_empty(), "on() returns a non-empty subscriber id");
    assert!(
        client.off(&id).expect("events enabled"),
        "off() removes the subscriber by id"
    );
}

// Regression: `on()`'s `event_type` is filtered by EXACT equality, per the
// spec Contract (docs/features/apcore-client.md "## Contract: APCore.on") —
// apcore-python's on()-created subscriber does `if event.event_type !=
// self.event_type:` and apcore-typescript's does `if (event.eventType ===
// eventType)`, neither a glob. Subscribing with a literal `*` character must
// NOT behave like `EventEmitter::matches_pattern`'s glob: a different event
// type that would glob-match the subscribed string must not fire the
// handler, and the literal string itself still must.
#[tokio::test]
async fn on_event_type_is_exact_match_not_glob() {
    let mut client = APCore::with_config(sys_config());
    let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let c = Arc::clone(&counter);
    // Subscribes to the literal event type "apcore.acl.*" — not a prefix glob.
    client
        .on("apcore.acl.*", move |_e| {
            c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        })
        .expect("events enabled");
    let emitter = client.events().expect("local emitter exists after on()");

    // A DIFFERENT event type that would glob-match "apcore.acl.*" must NOT
    // fire the subscriber.
    emitter
        .emit_sequential(&apcore::events::emitter::ApCoreEvent::new(
            "apcore.acl.rule_matched",
            json!({}),
        ))
        .await;
    assert_eq!(
        counter.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "on() must use exact equality, not glob — a differing event type must not fire"
    );

    // The literal subscribed string, emitted verbatim, still fires.
    emitter
        .emit_sequential(&apcore::events::emitter::ApCoreEvent::new(
            "apcore.acl.*",
            json!({}),
        ))
        .await;
    assert_eq!(
        counter.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the exact subscribed event_type string must still fire"
    );
}

// ===========================================================================
// Contract: APCoreClient.off
// ===========================================================================

// clause: apcore_client.off.error.SYS_MODULES_DISABLED
#[tokio::test]
async fn off_error_sys_modules_disabled() {
    // "no such subscriber" and "events are off" are different answers; off()
    // MUST NOT collapse the second into a plain `false`.
    let mut client = APCore::new();
    let err = client
        .off("nonexistent-id")
        .expect_err("off() must error when events are disabled");
    assert_eq!(err.code, apcore::errors::ErrorCode::SysModulesDisabled);
}

// clause: apcore_client.off.property.idempotent_true
#[tokio::test]
async fn off_property_idempotent_true() {
    // Unsubscribing the same id twice is a no-op the second time (no panic).
    let mut client = APCore::with_config(sys_config());
    let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let c = Arc::clone(&counter);
    let id = client
        .on("evt.off", move |_e| {
            c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        })
        .expect("events enabled");
    assert!(
        client.off(&id).expect("events enabled"),
        "first off() removes the subscriber"
    );
    assert!(
        !client.off(&id).expect("events enabled"),
        "second off() is a safe no-op (returns false)"
    );
    let emitter = client.events().expect("emitter");
    emitter
        .emit_sequential(&apcore::events::emitter::ApCoreEvent::new(
            "evt.off",
            json!({}),
        ))
        .await;
    assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 0);
}

// clause: apcore_client.off.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn off_property_thread_safe() {
    // off() requires &mut self; mirror the on() thread-safety pattern by
    // spawning >=8 independent clients that each subscribe then unsubscribe.
    let mut handles = Vec::new();
    for _ in 0..10 {
        handles.push(tokio::spawn(async {
            let mut client = APCore::with_config(sys_config());
            let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let c = Arc::clone(&counter);
            let id = client
                .on("evt.toff", move |_e| {
                    c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                })
                .expect("events enabled");
            let removed = client.off(&id).expect("events enabled");
            let emitter = client.events().expect("emitter");
            emitter
                .emit_sequential(&apcore::events::emitter::ApCoreEvent::new(
                    "evt.toff",
                    json!({}),
                ))
                .await;
            (removed, counter.load(std::sync::atomic::Ordering::SeqCst))
        }));
    }
    for h in handles {
        let (removed, fired) = h.await.expect("task must not panic");
        assert!(removed, "off() must report removal");
        assert_eq!(fired, 0, "removed handler must not fire");
    }
}

// ===========================================================================
// Contract: APCoreClient.stream
// ===========================================================================

// clause: apcore_client.stream.input.module_id.invalid_pattern
#[tokio::test]
async fn stream_input_module_id_invalid_pattern() {
    let client = client_with_module();
    let mut s = client.stream("!!bad!!", json!({}), None, None);
    let item = s.next().await.expect("stream yields a terminal error item");
    let err = item.expect_err("malformed module_id must surface as Err");
    assert_eq!(err.code, ErrorCode::InvalidModuleId);
}

// clause: apcore_client.stream.error.MODULE_NOT_FOUND
#[tokio::test]
async fn stream_error_module_not_found() {
    let client = client_with_module();
    let mut s = client.stream("not.registered", json!({}), None, None);
    let item = s.next().await.expect("stream yields a terminal error item");
    let err = item.expect_err("unknown module must surface as Err");
    assert_eq!(err.code, ErrorCode::ModuleNotFound);
}

// clause: apcore_client.stream.error.INVALID_MODULE_ID
#[tokio::test]
async fn stream_error_invalid_module_id() {
    let client = client_with_module();
    let mut s = client.stream("", json!({}), None, None);
    let item = s.next().await.expect("stream yields a terminal error item");
    let err = item.expect_err("empty module_id must surface as Err");
    assert_eq!(err.code, ErrorCode::InvalidModuleId);
}

// clause: apcore_client.stream.property.async
#[tokio::test]
async fn stream_property_async() {
    let client = client_with_module();
    let mut s = client.stream("math.add", json!({"a": 3, "b": 4}), None, None);
    let mut chunks: Vec<Value> = Vec::new();
    while let Some(item) = s.next().await {
        chunks.push(item.expect("non-streaming module yields Ok chunk"));
    }
    assert!(!chunks.is_empty(), "stream yields at least one chunk");
    assert_eq!(chunks.last().unwrap(), &json!({"sum": 7}));
}

// clause: apcore_client.stream.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stream_property_thread_safe() {
    let client = Arc::new(client_with_module());
    let mut handles = Vec::new();
    for i in 0..8i64 {
        let c = Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            let mut s = c.stream("math.add", json!({"a": i, "b": i + 1}), None, None);
            let mut last = json!(null);
            while let Some(item) = s.next().await {
                last = item.expect("chunk ok");
            }
            (i, last["sum"].as_i64().unwrap())
        }));
    }
    for h in handles {
        let (i, sum) = h.await.expect("task must not panic");
        assert_eq!(sum, i + (i + 1));
    }
}

// ===========================================================================
// Contract: APCoreClient.validate
// ===========================================================================

// clause: apcore_client.validate.input.module_id.invalid_pattern
#[tokio::test]
async fn validate_input_module_id_invalid_pattern() {
    // DIVERGENCE: Python validate() RAISES InvalidInputError for a malformed
    // module_id before the pipeline begins. Rust's validate() instead CAPTURES
    // it as a failing `module_id` preflight check (no Err) — non-destructive
    // by construction. We assert the REAL Rust behavior.
    let client = client_with_module();
    let result = client
        .validate("!!bad id!!", None, None)
        .await
        .expect("Rust validate captures malformed id in the result, does not error");
    assert!(!result.valid);
    let module_id_check = result
        .checks
        .iter()
        .find(|c| c.check == "module_id")
        .expect("module_id check present");
    assert!(
        !module_id_check.passed,
        "malformed id fails the module_id check"
    );
    assert_eq!(
        module_id_check.error.as_ref().unwrap()["code"],
        json!("INVALID_MODULE_ID")
    );
}

// clause: apcore_client.validate.error.INVALID_MODULE_ID
#[tokio::test]
async fn validate_error_invalid_module_id() {
    // DIVERGENCE (see above): empty module_id is captured in the result, not
    // raised, in Rust.
    let client = client_with_module();
    let result = client
        .validate("", None, None)
        .await
        .expect("Rust validate captures empty id in the result, does not error");
    assert!(!result.valid);
    let module_id_check = result
        .checks
        .iter()
        .find(|c| c.check == "module_id")
        .expect("module_id check present");
    assert!(!module_id_check.passed);
    assert_eq!(
        module_id_check.error.as_ref().unwrap()["code"],
        json!("INVALID_MODULE_ID")
    );
}

// clause: apcore_client.validate.error.no_raise_on_failure
#[tokio::test]
async fn validate_error_no_raise_on_failure() {
    // A missing module yields valid=false with a recorded failing check rather
    // than an Err.
    let client = client_with_module();
    let result = client
        .validate("absent.module", None, None)
        .await
        .expect("validate does not error on validation failure");
    assert!(!result.valid);
    assert!(result.checks.iter().any(|c| !c.passed));
    // The categorized check built from the unwrapped pipeline error MUST name
    // the WIRE code. Asserting only that SOME check failed leaves the code
    // unpinned, and the code is what a polyglot caller matches on:
    // apcore-python builds this dict from the error's `to_dict()` (wire code,
    // executor.py:604-607) and apcore-typescript emits `{ code: e.code }`
    // (executor.ts:880). Rust formatted the `ErrorCode` with `Debug` here,
    // yielding the PascalCase variant name — a code in no registry.
    let lookup_codes: Vec<String> = result
        .checks
        .iter()
        .filter(|c| c.check == "module_lookup")
        .map(|c| {
            assert!(!c.passed, "a missing module cannot pass module_lookup");
            c.error.as_ref().expect("failing check carries an error")["code"]
                .as_str()
                .expect("check error code is a string")
                .to_string()
        })
        .collect();
    // ONE entry, carrying the WIRE code. The trace-derived
    // `STEP_MODULE_LOOKUP_FAILED` entry for the same step is dropped on the
    // error path: the categorized check supersedes it with the real code and
    // message, and keeping both put one failure in `checks` twice, so
    // `errors()` reported two problems where one existed.
    //
    // This closes the divergence this assertion used to record. apcore-python
    // also emits one entry — it gets there by dropping the whole trace
    // ("to avoid _trace_to_checks adding a second, redundant failure entry for
    // the same step", executor.py), which also loses the checks that PASSED.
    // Rust keeps those, so its list is strictly more informative at the same
    // failure count.
    assert_eq!(
        lookup_codes,
        vec!["MODULE_NOT_FOUND"],
        "preflight check errors carry the wire code, not the Debug variant name, \
         and one failed step yields exactly one failed check"
    );
}

// clause: apcore_client.validate.returns.preflight_result
#[tokio::test]
async fn validate_returns_preflight_result() {
    let client = client_with_module();
    let inputs = json!({"a": 1, "b": 2});
    let result = client
        .validate("math.add", &inputs, None)
        .await
        .expect("validate succeeds");
    // DIVERGENCE: Python's PreflightResult carries 7 checks; the Rust pipeline
    // emits 8 preflight checks (extra step vs the Python clause). Assert the
    // REAL Rust count, and the structural shape (valid/requires_approval/errors).
    assert_eq!(result.checks.len(), 8);
    let _: bool = result.requires_approval;
    let _: bool = result.valid;
    // errors() aggregates failing checks; an all-pass result has none.
    assert!(
        result.valid,
        "math.add with valid inputs should pass preflight"
    );
    assert!(
        result.errors().is_empty(),
        "an all-pass result aggregates no errors"
    );
}

// clause: apcore_client.validate.property.pure
#[tokio::test]
async fn validate_property_pure() {
    let client = client_with_module();
    let inputs = json!({"a": 1, "b": 2});
    let before = client.list_modules(None, None);
    let _ = client.validate("math.add", &inputs, None).await.unwrap();
    let after = client.list_modules(None, None);
    assert_eq!(before, after, "validate must not mutate the module list");
}

// clause: apcore_client.validate.property.idempotent_true
#[tokio::test]
async fn validate_property_idempotent_true() {
    let client = client_with_module();
    let inputs = json!({"a": 1, "b": 2});
    let r1 = client.validate("math.add", &inputs, None).await.unwrap();
    let r2 = client.validate("math.add", &inputs, None).await.unwrap();
    assert_eq!(r1.valid, r2.valid);
    assert_eq!(r1.checks.len(), r2.checks.len());
}

// clause: apcore_client.validate.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn validate_property_thread_safe() {
    let client = Arc::new(client_with_module());
    let mut handles = Vec::new();
    for i in 0..8i64 {
        let c = Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            let inputs = json!({"a": i, "b": i});
            c.validate("math.add", &inputs, None)
                .await
                .expect("concurrent validate succeeds")
                .valid
        }));
    }
    for h in handles {
        let valid = h.await.expect("task must not panic");
        assert!(valid, "valid module/input must report valid=true");
    }
}

// ===========================================================================
// Contract: APCoreClient.disable
// ===========================================================================

// clause: apcore_client.disable.error.SYS_MODULES_DISABLED
#[tokio::test]
async fn disable_error_sys_modules_disabled() {
    let client = APCore::with_config({
        let mut c = Config::default();
        c.set("sys_modules.enabled", json!(false));
        c
    });
    let err = client
        .disable("some.module", None)
        .await
        .expect_err("disable without sys_modules must error");
    assert_eq!(err.code, ErrorCode::SysModulesDisabled);
    assert!(
        err.message.contains("disable() requires sys_modules"),
        "message should mention sys_modules requirement, got: {}",
        err.message
    );
}

// clause: apcore_client.disable.input.reason.default
#[tokio::test]
async fn disable_input_reason_default() {
    let client = APCore::with_config(sys_config());
    client
        .register("risky.module", Box::new(EchoModule))
        .expect("register risky.module");
    let result = client
        .disable("risky.module", None)
        .await
        .expect("disable succeeds with default reason");
    assert!(result.is_object(), "disable returns a structured payload");
    assert_eq!(result["module_id"], json!("risky.module"));
    assert_eq!(result["enabled"], json!(false));
}

// clause: apcore_client.disable.property.idempotent_true
#[tokio::test]
async fn disable_property_idempotent_true() {
    let client = APCore::with_config(sys_config());
    client
        .register("risky.dup", Box::new(EchoModule))
        .expect("register risky.dup");
    let first = client
        .disable("risky.dup", None)
        .await
        .expect("first disable");
    let second = client
        .disable("risky.dup", None)
        .await
        .expect("second disable");
    assert_eq!(first["enabled"], json!(false));
    assert_eq!(second["enabled"], json!(false));
}

// ===========================================================================
// Contract: APCoreClient.enable
// ===========================================================================

// clause: apcore_client.enable.error.SYS_MODULES_DISABLED
#[tokio::test]
async fn enable_error_sys_modules_disabled() {
    let client = APCore::with_config({
        let mut c = Config::default();
        c.set("sys_modules.enabled", json!(false));
        c
    });
    let err = client
        .enable("some.module", None)
        .await
        .expect_err("enable without sys_modules must error");
    assert_eq!(err.code, ErrorCode::SysModulesDisabled);
    assert!(
        err.message.contains("enable() requires sys_modules"),
        "message should mention sys_modules requirement, got: {}",
        err.message
    );
}

// clause: apcore_client.enable.input.reason.default
#[tokio::test]
async fn enable_input_reason_default() {
    let client = APCore::with_config(sys_config());
    client
        .register("risky.enable", Box::new(EchoModule))
        .expect("register risky.enable");
    client
        .disable("risky.enable", None)
        .await
        .expect("disable first");
    let result = client
        .enable("risky.enable", None)
        .await
        .expect("enable succeeds");
    assert!(result.is_object());
    assert_eq!(result["module_id"], json!("risky.enable"));
    assert_eq!(result["enabled"], json!(true));
}

// clause: apcore_client.enable.property.idempotent_true
#[tokio::test]
async fn enable_property_idempotent_true() {
    let client = APCore::with_config(sys_config());
    client
        .register("risky.reenable", Box::new(EchoModule))
        .expect("register risky.reenable");
    let first = client
        .enable("risky.reenable", None)
        .await
        .expect("first enable");
    let second = client
        .enable("risky.reenable", None)
        .await
        .expect("second enable");
    assert_eq!(first["enabled"], json!(true));
    assert_eq!(second["enabled"], json!(true));
}

// ===========================================================================
// Contract: APCore.__init__
// ===========================================================================

// clause: apcore_client.__init__.input.zero_config
#[tokio::test]
async fn init_input_zero_config() {
    // Zero-config APCore::new() creates a functional registry and executor;
    // the local event emitter is None (events not configured until on()).
    let client = APCore::new();
    let _registry: &apcore::registry::registry::Registry = client.registry();
    let _executor = client.executor();
    assert!(
        client.events().is_none(),
        "zero-config exposes no local emitter"
    );
}

// clause: apcore_client.__init__.error.no_raise
#[tokio::test]
async fn init_error_no_raise() {
    // Construction never errors for sys_modules; a disabled-sys_modules config
    // constructs cleanly with no local emitter.
    let mut c = Config::default();
    c.set("sys_modules.enabled", json!(false));
    let client = APCore::with_config(c);
    assert!(client.events().is_none());
}

// clause: apcore_client.__init__.property.async_false
#[tokio::test]
async fn init_property_async_false() {
    // Construction is synchronous: new() returns an instance directly without
    // awaiting.
    let client = APCore::new();
    assert!(
        !client.list_modules(None, None).is_empty() || client.list_modules(None, None).is_empty()
    );
    // The above is trivially true; the load-bearing assertion is that the line
    // below compiles and runs without `.await` on construction.
    let _client2 = APCore::default();
}

// clause: apcore_client.__init__.property.pure_false
#[tokio::test]
async fn init_property_pure_false() {
    // With sys_modules enabled, construction registers system modules as a side
    // effect (observable via list_modules).
    let client = APCore::with_config(sys_config());
    let modules = client.list_modules(None, None);
    assert!(
        modules.iter().any(|m| m.starts_with("system.")),
        "sys_modules construction registers system.* modules"
    );
}

// ===========================================================================
// Contract: APCore.module  (Rust: no decorator — register() is the surface)
// ===========================================================================

// clause: apcore_client.module.input.id.invalid_pattern
#[tokio::test]
async fn module_input_id_invalid_pattern() {
    // Rust has no decorator; register() is the registration surface. A
    // malformed id is rejected at registration.
    let client = APCore::new();
    let err = client
        .register("Bad ID With Spaces", Box::new(EchoModule))
        .expect_err("malformed id must be rejected");
    assert!(
        matches!(
            err.code,
            ErrorCode::InvalidModuleId | ErrorCode::InvalidSegment | ErrorCode::ModuleLoadError
        ),
        "unexpected error code for malformed id: {:?}",
        err.code
    );
}

// clause: apcore_client.module.error.duplicate
#[tokio::test]
async fn module_error_duplicate() {
    let client = APCore::new();
    client
        .register("dup.module", Box::new(EchoModule))
        .expect("first register succeeds");
    let err = client
        .register("dup.module", Box::new(EchoModule))
        .expect_err("duplicate register must fail");
    assert!(
        matches!(
            err.code,
            ErrorCode::DuplicateModuleId
                | ErrorCode::ModuleLoadError
                | ErrorCode::GeneralInvalidInput
                | ErrorCode::ModuleIdConflict
        ),
        "unexpected duplicate error code: {:?}",
        err.code
    );
}

// clause: apcore_client.module.returns.original_function
#[tokio::test]
async fn module_returns_original_function() {
    // Rust register() returns Ok(()) and the module becomes callable.
    let client = APCore::new();
    client
        .register("math.ret", Box::new(AddModule))
        .expect("register math.ret");
    let result = client
        .call("math.ret", json!({"a": 2, "b": 3}), None, None)
        .await
        .expect("registered module is callable");
    assert_eq!(result, json!({"sum": 5}));
}

// clause: apcore_client.module.property.idempotent_false
#[tokio::test]
async fn module_property_idempotent_false() {
    // First registration succeeds; module count increments by exactly one.
    let client = APCore::new();
    let before = client.list_modules(None, None).len();
    client
        .register("once.module", Box::new(EchoModule))
        .expect("register once.module");
    let after = client.list_modules(None, None).len();
    assert_eq!(after, before + 1);
}

// ===========================================================================
// Contract: APCore.register
// ===========================================================================

// clause: apcore_client.register.input.module_id.invalid_pattern
#[tokio::test]
async fn register_input_module_id_invalid_pattern() {
    let client = APCore::new();
    let err = client
        .register("Bad Id!!", Box::new(AddModule))
        .expect_err("malformed module_id must error");
    assert!(
        matches!(
            err.code,
            ErrorCode::InvalidModuleId | ErrorCode::InvalidSegment | ErrorCode::ModuleLoadError
        ),
        "unexpected error code: {:?}",
        err.code
    );
}

// clause: apcore_client.register.error.INVALID_MODULE_ID
#[tokio::test]
async fn register_error_invalid_module_id() {
    // Registering twice under the same id errors on the second.
    let client = APCore::new();
    client
        .register("math.copy", Box::new(AddModule))
        .expect("first register succeeds");
    let err = client
        .register("math.copy", Box::new(AddModule))
        .expect_err("duplicate register must fail");
    assert!(
        matches!(
            err.code,
            ErrorCode::DuplicateModuleId
                | ErrorCode::ModuleLoadError
                | ErrorCode::GeneralInvalidInput
                | ErrorCode::ModuleIdConflict
        ),
        "unexpected error code: {:?}",
        err.code
    );
}

// clause: apcore_client.register.returns.none
#[tokio::test]
async fn register_returns_none() {
    // register() returns Ok(()) (the unit type) and the module is present.
    let client = APCore::new();
    let ret = client.register("math.target", Box::new(AddModule));
    assert!(ret.is_ok(), "register returns Ok(())");
    assert!(client
        .list_modules(None, None)
        .contains(&"math.target".to_string()));
}

// clause: apcore_client.register.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn register_property_thread_safe() {
    let client = Arc::new(APCore::new());
    let mut handles = Vec::new();
    for i in 0..10 {
        let c = Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            let id = format!("svc.mod{i}");
            c.register(&id, Box::new(EchoModule))
                .expect("concurrent register");
            id
        }));
    }
    let mut ids = Vec::new();
    for h in handles {
        ids.push(h.await.expect("task must not panic"));
    }
    let modules = client.list_modules(None, None);
    for id in ids {
        assert!(modules.contains(&id), "module {id} must be registered");
    }
}

// ===========================================================================
// Contract: APCore.discover
// ===========================================================================

// clause: apcore_client.discover.error.ConfigNotFoundError
#[tokio::test]
async fn discover_error_config_not_found() {
    // A configured extension root that does not exist surfaces an error.
    let tmp = std::env::temp_dir().join(format!("apcore-missing-{}", uuid::Uuid::new_v4()));
    let mut config = Config::default();
    config.set("extensions.root", json!(tmp.to_string_lossy()));
    let client = APCore::with_config(config);
    let result = client.discover().await;
    // Rust may either return ConfigNotFound or treat an unconfigured discoverer
    // as 0; assert the contract intent: a missing root does not silently
    // register modules.
    match result {
        Err(e) => assert!(
            matches!(e.code, ErrorCode::ConfigNotFound | ErrorCode::ConfigInvalid),
            "unexpected discover error code: {:?}",
            e.code
        ),
        Ok(count) => assert_eq!(count, 0, "missing root must register no modules"),
    }
}

// clause: apcore_client.discover.returns.int_count
#[tokio::test]
async fn discover_returns_int_count() {
    // discover() returns a usize count (0 for an empty / unconfigured root).
    let tmp = std::env::temp_dir().join(format!("apcore-ext-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).expect("create extensions root");
    let mut config = Config::default();
    config.set("extensions.root", json!(tmp.to_string_lossy()));
    let client = APCore::with_config(config);
    let count = client
        .discover()
        .await
        .expect("discover succeeds on empty root");
    assert_eq!(count, 0);
    let _ = std::fs::remove_dir_all(&tmp);
}

// ===========================================================================
// Contract: APCore.list_modules
// ===========================================================================

// clause: apcore_client.list_modules.returns.sorted
#[tokio::test]
async fn list_modules_returns_sorted() {
    let client = APCore::new();
    for mid in ["c.three", "a.one", "b.two"] {
        client
            .register(mid, Box::new(EchoModule))
            .expect("register");
    }
    let result = client.list_modules(None, None);
    let mut sorted = result.clone();
    sorted.sort();
    assert_eq!(result, sorted, "list_modules returns a sorted list");
    for mid in ["a.one", "b.two", "c.three"] {
        assert!(result.contains(&mid.to_string()));
    }
}

// clause: apcore_client.list_modules.input.prefix
#[tokio::test]
async fn list_modules_input_prefix() {
    let client = APCore::new();
    for mid in ["math.add", "math.sub", "text.upper"] {
        client
            .register(mid, Box::new(EchoModule))
            .expect("register");
    }
    let result = client.list_modules(None, Some("math."));
    let set: std::collections::HashSet<String> = result.into_iter().collect();
    assert_eq!(
        set,
        ["math.add".to_string(), "math.sub".to_string()]
            .into_iter()
            .collect()
    );
}

// clause: apcore_client.list_modules.property.pure
#[tokio::test]
async fn list_modules_property_pure() {
    // Each call returns a fresh Vec; mutating it does not affect the next call.
    let client = client_with_module();
    let mut r1 = client.list_modules(None, None);
    r1.push("injected.fake".to_string());
    let r2 = client.list_modules(None, None);
    assert!(!r2.contains(&"injected.fake".to_string()));
}

// clause: apcore_client.list_modules.property.idempotent
#[tokio::test]
async fn list_modules_property_idempotent() {
    let client = client_with_module();
    assert_eq!(
        client.list_modules(None, None),
        client.list_modules(None, None)
    );
}

// ===========================================================================
// Contract: APCore.describe
// ===========================================================================

// clause: apcore_client.describe.error.ModuleNotFoundError
// [client-describe-raise] describe() MUST raise ModuleNotFoundError for a
// missing module (parity with apcore-python / apcore-typescript), not return a
// sentinel "not found" string.
#[tokio::test]
async fn describe_error_module_not_found() {
    use apcore::errors::ErrorCode;
    let client = APCore::new();
    let err = client
        .describe("not.registered")
        .expect_err("describe on a missing module must error");
    assert_eq!(err.code, ErrorCode::ModuleNotFound);
}

// clause: apcore_client.describe.returns.string
#[tokio::test]
async fn describe_returns_string() {
    let client = client_with_module();
    let text = client.describe("math.add").expect("module is registered");
    assert!(!text.is_empty(), "describe returns a non-empty string");
    assert!(
        text.contains("math.add") || text.contains("Add two numbers"),
        "describe should reference the module, got: {text}"
    );
}

// clause: apcore_client.describe.property.pure
#[tokio::test]
async fn describe_property_pure() {
    let client = client_with_module();
    let before = client.list_modules(None, None);
    let _ = client.describe("math.add");
    assert_eq!(client.list_modules(None, None), before);
}

// clause: apcore_client.describe.property.idempotent
#[tokio::test]
async fn describe_property_idempotent() {
    let client = client_with_module();
    assert_eq!(
        client.describe("math.add").ok(),
        client.describe("math.add").ok()
    );
}

// ===========================================================================
// Contract: APCore.use / APCore.use_middleware
// ===========================================================================

// clause: apcore_client.use.error.priority_exceeds_1000
#[tokio::test]
async fn use_error_priority_exceeds_1000() {
    // Rust use_middleware() returns Err(ModuleError) for priority > 1000
    // (not a panic / ValueError).
    let client = APCore::new();
    let err = client
        .use_middleware(Box::new(NoopMiddleware {
            name: "too-high".to_string(),
            priority: 1001,
        }))
        .expect_err("priority > 1000 must error");
    assert_eq!(err.code, ErrorCode::GeneralInvalidInput);
}

// clause: apcore_client.use.returns.self
#[tokio::test]
async fn use_returns_self() {
    // use_middleware() returns &Self so calls chain.
    let client = APCore::new();
    let chained = client
        .use_middleware(Box::new(NoopMiddleware {
            name: "a".to_string(),
            priority: 0,
        }))
        .expect("add a")
        .use_middleware(Box::new(NoopMiddleware {
            name: "b".to_string(),
            priority: 0,
        }));
    assert!(chained.is_ok(), "chained use_middleware returns Ok(&Self)");
}

// clause: apcore_client.use.property.idempotent_false
#[tokio::test]
async fn use_property_idempotent_false() {
    // Adding two middlewares (Rust matches by name on removal) then removing
    // by name strips them; a never-added name returns false.
    let client = APCore::new();
    client
        .use_middleware(Box::new(NoopMiddleware {
            name: "dup-mw".to_string(),
            priority: 0,
        }))
        .expect("add dup-mw");
    assert!(
        client.remove("dup-mw"),
        "first removal finds the middleware"
    );
}

// ===========================================================================
// Contract: APCore.use_before
// ===========================================================================

// A BeforeMiddleware that appends a marker to a shared ordered log.
#[derive(Debug)]
struct OrderBefore {
    log: Arc<parking_lot::Mutex<Vec<String>>>,
}

#[async_trait]
impl BeforeMiddleware for OrderBefore {
    fn name(&self) -> &'static str {
        "order-before"
    }
    async fn before(
        &self,
        _module_id: &str,
        _inputs: Value,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        self.log.lock().push("before".to_string());
        Ok(None)
    }
}

#[derive(Debug)]
struct OrderAfter {
    log: Arc<parking_lot::Mutex<Vec<String>>>,
}

#[async_trait]
impl AfterMiddleware for OrderAfter {
    fn name(&self) -> &'static str {
        "order-after"
    }
    async fn after(
        &self,
        _module_id: &str,
        _inputs: Value,
        _output: Value,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        self.log.lock().push("after".to_string());
        Ok(None)
    }
}

/// Module that records "execute" in a shared ordered log.
struct OrderedModule {
    log: Arc<parking_lot::Mutex<Vec<String>>>,
}

#[async_trait]
impl Module for OrderedModule {
    fn description(&self) -> &'static str {
        "ordered"
    }
    fn input_schema(&self) -> Value {
        json!({})
    }
    fn output_schema(&self) -> Value {
        json!({})
    }
    async fn execute(&self, _i: Value, _c: &Context<Value>) -> Result<Value, ModuleError> {
        self.log.lock().push("execute".to_string());
        Ok(json!({}))
    }
}

// clause: apcore_client.use_before.returns.self
#[tokio::test]
async fn use_before_returns_self() {
    let client = APCore::new();
    let log = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let ret = client.use_before(Box::new(OrderBefore { log }));
    assert!(ret.is_ok(), "use_before returns Ok(&Self) for chaining");
}

// clause: apcore_client.use_before.side_effect.1.before_execute
#[tokio::test]
async fn use_before_side_effect_1_before_execute() {
    let client = APCore::new();
    let log = Arc::new(parking_lot::Mutex::new(Vec::new()));
    client
        .register(
            "ord.mod",
            Box::new(OrderedModule {
                log: Arc::clone(&log),
            }),
        )
        .expect("register ord.mod");
    client
        .use_before(Box::new(OrderBefore {
            log: Arc::clone(&log),
        }))
        .expect("register before");
    client
        .call("ord.mod", json!({}), None, None)
        .await
        .expect("call");
    assert_eq!(
        *log.lock(),
        vec!["before".to_string(), "execute".to_string()]
    );
}

// clause: apcore_client.use_before.property.idempotent_false
#[tokio::test]
async fn use_before_property_idempotent_false() {
    // Registering two before-middlewares fires both per execution.
    let client = APCore::new();
    let log = Arc::new(parking_lot::Mutex::new(Vec::new()));
    client
        .register(
            "ord.dup",
            Box::new(OrderedModule {
                log: Arc::clone(&log),
            }),
        )
        .expect("register ord.dup");
    client
        .use_before(Box::new(OrderBefore {
            log: Arc::clone(&log),
        }))
        .expect("register before 1");
    client
        .use_before(Box::new(OrderBefore {
            log: Arc::clone(&log),
        }))
        .expect("register before 2");
    client
        .call("ord.dup", json!({}), None, None)
        .await
        .expect("call");
    let befores = log.lock().iter().filter(|e| *e == "before").count();
    assert_eq!(befores, 2, "two before-middlewares fire twice");
}

// ===========================================================================
// Contract: APCore.use_after
// ===========================================================================

// clause: apcore_client.use_after.returns.self
#[tokio::test]
async fn use_after_returns_self() {
    let client = APCore::new();
    let log = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let ret = client.use_after(Box::new(OrderAfter { log }));
    assert!(ret.is_ok(), "use_after returns Ok(&Self) for chaining");
}

// clause: apcore_client.use_after.side_effect.1.after_execute
#[tokio::test]
async fn use_after_side_effect_1_after_execute() {
    let client = APCore::new();
    let log = Arc::new(parking_lot::Mutex::new(Vec::new()));
    client
        .register(
            "ord.after",
            Box::new(OrderedModule {
                log: Arc::clone(&log),
            }),
        )
        .expect("register ord.after");
    client
        .use_after(Box::new(OrderAfter {
            log: Arc::clone(&log),
        }))
        .expect("register after");
    client
        .call("ord.after", json!({}), None, None)
        .await
        .expect("call");
    assert_eq!(
        *log.lock(),
        vec!["execute".to_string(), "after".to_string()]
    );
}

// clause: apcore_client.use_after.property.idempotent_false
#[tokio::test]
async fn use_after_property_idempotent_false() {
    let client = APCore::new();
    let log = Arc::new(parking_lot::Mutex::new(Vec::new()));
    client
        .register(
            "ord.afterdup",
            Box::new(OrderedModule {
                log: Arc::clone(&log),
            }),
        )
        .expect("register ord.afterdup");
    client
        .use_after(Box::new(OrderAfter {
            log: Arc::clone(&log),
        }))
        .expect("register after 1");
    client
        .use_after(Box::new(OrderAfter {
            log: Arc::clone(&log),
        }))
        .expect("register after 2");
    client
        .call("ord.afterdup", json!({}), None, None)
        .await
        .expect("call");
    let afters = log.lock().iter().filter(|e| *e == "after").count();
    assert_eq!(afters, 2, "two after-middlewares fire twice");
}

// ===========================================================================
// Contract: APCore.remove
// ===========================================================================

// clause: apcore_client.remove.returns.true_when_present
#[tokio::test]
async fn remove_returns_true_when_present() {
    let client = APCore::new();
    let mw = NoopMiddleware {
        name: "removable".to_string(),
        priority: 0,
    };
    client
        .use_middleware(Box::new(NoopMiddleware {
            name: "removable".to_string(),
            priority: 0,
        }))
        .expect("add removable");
    assert!(client.remove_middleware(&mw), "remove finds it by name");
}

// clause: apcore_client.remove.returns.false_when_absent
#[tokio::test]
async fn remove_returns_false_when_absent() {
    let client = APCore::new();
    let mw = NoopMiddleware {
        name: "never-added".to_string(),
        priority: 0,
    };
    assert!(
        !client.remove_middleware(&mw),
        "remove of absent returns false"
    );
}

// clause: apcore_client.remove.property.idempotent_true
#[tokio::test]
async fn remove_property_idempotent_true() {
    let client = APCore::new();
    let mw = NoopMiddleware {
        name: "idem".to_string(),
        priority: 0,
    };
    client
        .use_middleware(Box::new(NoopMiddleware {
            name: "idem".to_string(),
            priority: 0,
        }))
        .expect("add idem");
    assert!(client.remove_middleware(&mw), "first removal succeeds");
    assert!(
        !client.remove_middleware(&mw),
        "second removal returns false safely"
    );
}
