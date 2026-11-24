// Spec-traced contract tests for the apcore-rust Module Interface feature.
//
// Source spec: apcore/docs/features/module-interface.md
//   ("## Contract: Module conformance" block).
// Canonical clause list mirrored from:
//   apcore-python/tests/test_module_interface_spec.py
//
// Each test maps to exactly one clause in the feature spec. The verbatim
// cross-language clause id appears in a leading `// clause: <clause_id>`
// comment on the line above each test fn so that a cross-language diff tool can
// line up the Python / TypeScript / Rust rows by that exact string. The fn name
// is the clause id flattened to snake_case.
//
// LANGUAGE NOTE: In Rust, `Module` is a *trait* — structural conformance to the
// required surface (`input_schema` / `output_schema` / `description` /
// `execute`) is enforced by the compiler, not by a runtime `validate_module()`
// of a class. The idiomatic Rust runtime equivalent of Python's
// `validate_module()` is `apcore::registry::validation::validate_descriptor`,
// which inspects a JSON module *descriptor* and returns a list of error
// strings. The required-surface clauses are mirrored against that function.
//
// GAP NOTE (mirrors the Python suite): the contract's ### Errors section names
// six dedicated error types (MissingRequiredAttribute, InvalidSchemaType,
// DescriptionTooLong, DocumentationTooLong, InvalidAnnotations, InvalidExample).
// Rust does NOT expose these as distinct `ErrorCode` variants or error types —
// required-surface conformance is compiler-enforced / duck-typed via
// `validate_descriptor` string output. Those clauses are emitted as
// `#[ignore]` tests tagged "missing symbol" to document the cross-language gap
// while keeping the crate compilable. `MODULE_TIMEOUT` is a real, testable code.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};

use apcore::context::{Context, Identity};
use apcore::errors::{ErrorCode, ModuleError};
use apcore::module::{Module, ModuleAnnotations, ModuleExample};
use apcore::registry::registry::Registry;
use apcore::registry::validation::validate_descriptor;
use apcore::schema::validator::SchemaValidator;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_ctx() -> Context<Value> {
    Context::new(Identity::new(
        "test".to_string(),
        "Test".to_string(),
        vec![],
        HashMap::new(),
    ))
}

/// Wire string for an ErrorCode (SCREAMING_SNAKE_CASE per serde), so we can
/// assert the exact cross-language code string the Python suite checks.
fn code_str(code: ErrorCode) -> String {
    match serde_json::to_value(code).expect("ErrorCode serializes to a JSON string") {
        Value::String(s) => s,
        other => panic!("ErrorCode did not serialize to a string: {other:?}"),
    }
}

/// JSON Schema for the `_EchoInput` surface: a required string field `value`.
fn echo_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "value": { "type": "string", "description": "The value to echo back" }
        },
        "required": ["value"],
        "additionalProperties": false
    })
}

/// JSON Schema for the `_EchoOutput` surface: a required string field `echoed`.
fn echo_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "echoed": { "type": "string", "description": "The echoed value" }
        },
        "required": ["echoed"],
        "additionalProperties": false
    })
}

/// Build a conforming module descriptor (all required surface present).
fn conforming_descriptor() -> Value {
    json!({
        "input_schema": echo_input_schema(),
        "output_schema": echo_output_schema(),
        "description": "Echoes the supplied value back unchanged."
    })
}

// ---------------------------------------------------------------------------
// Module fixtures exercising the required/optional surface
// ---------------------------------------------------------------------------

/// Minimal module satisfying the full required surface (sync-bodied execute).
struct ConformingModule;

#[async_trait]
impl Module for ConformingModule {
    fn input_schema(&self) -> Value {
        echo_input_schema()
    }
    fn output_schema(&self) -> Value {
        echo_output_schema()
    }
    fn description(&self) -> &'static str {
        "Echoes the supplied value back unchanged."
    }
    async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        let value = inputs.get("value").cloned().unwrap_or(Value::Null);
        Ok(json!({ "echoed": value }))
    }
}

/// Async-bodied echo module (mirror of Python `_AsyncModule`). In Rust every
/// `execute` is async by trait definition; this fixture additionally awaits.
struct AsyncModule;

#[async_trait]
impl Module for AsyncModule {
    fn input_schema(&self) -> Value {
        echo_input_schema()
    }
    fn output_schema(&self) -> Value {
        echo_output_schema()
    }
    fn description(&self) -> &'static str {
        "Async echo module."
    }
    async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        tokio::task::yield_now().await;
        let value = inputs.get("value").cloned().unwrap_or(Value::Null);
        Ok(json!({ "echoed": value }))
    }
}

/// Thread-safe module: holds no mutable shared state; `execute` only reads
/// per-call inputs. Mirror of Python `_ConcurrentModule`.
struct ConcurrentModule;

#[async_trait]
impl Module for ConcurrentModule {
    fn input_schema(&self) -> Value {
        echo_input_schema()
    }
    fn output_schema(&self) -> Value {
        echo_output_schema()
    }
    fn description(&self) -> &'static str {
        "Concurrent-safe echo module."
    }
    async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        tokio::task::yield_now().await;
        let value = inputs.get("value").cloned().unwrap_or(Value::Null);
        Ok(json!({ "echoed": value }))
    }
}

/// Module with an observable side effect (call counter). Mirror of Python
/// `_SideEffectfulModule` — `pure` is NOT required by the contract.
struct SideEffectfulModule {
    call_count: Arc<Mutex<u32>>,
}

#[async_trait]
impl Module for SideEffectfulModule {
    fn input_schema(&self) -> Value {
        echo_input_schema()
    }
    fn output_schema(&self) -> Value {
        echo_output_schema()
    }
    fn description(&self) -> &'static str {
        "Counts how many times it ran."
    }
    async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        *self.call_count.lock().unwrap() += 1;
        let value = inputs.get("value").cloned().unwrap_or(Value::Null);
        Ok(json!({ "echoed": value }))
    }
}

/// Module that records lifecycle-hook invocation order into a shared log.
/// Mirror of Python `_LifecycleModule`. `on_load` / `on_unload` are driven by
/// the Registry register / unregister API.
struct LifecycleModule {
    log: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl Module for LifecycleModule {
    fn input_schema(&self) -> Value {
        echo_input_schema()
    }
    fn output_schema(&self) -> Value {
        echo_output_schema()
    }
    fn description(&self) -> &'static str {
        "Lifecycle-instrumented module."
    }
    async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        let value = inputs.get("value").cloned().unwrap_or(Value::Null);
        Ok(json!({ "echoed": value }))
    }
    fn on_load(&self) -> Result<(), ModuleError> {
        self.log.lock().unwrap().push("on_load".to_string());
        Ok(())
    }
    fn on_unload(&self) {
        self.log.lock().unwrap().push("on_unload".to_string());
    }
}

/// Module implementing on_suspend / on_resume for hot-reload round-trip.
/// Mirror of Python `_SuspendResumeModule`.
struct SuspendResumeModule {
    counter: Arc<Mutex<u64>>,
    resumed: Arc<Mutex<Option<Value>>>,
}

impl SuspendResumeModule {
    fn new() -> Self {
        Self {
            counter: Arc::new(Mutex::new(0)),
            resumed: Arc::new(Mutex::new(None)),
        }
    }
    fn count(&self) -> u64 {
        *self.counter.lock().unwrap()
    }
}

#[async_trait]
impl Module for SuspendResumeModule {
    fn input_schema(&self) -> Value {
        echo_input_schema()
    }
    fn output_schema(&self) -> Value {
        echo_output_schema()
    }
    fn description(&self) -> &'static str {
        "Stateful suspend/resume module."
    }
    async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        *self.counter.lock().unwrap() += 1;
        let value = inputs.get("value").cloned().unwrap_or(Value::Null);
        Ok(json!({ "echoed": value }))
    }
    fn on_suspend(&self) -> Option<Value> {
        Some(json!({ "counter": self.count() }))
    }
    fn on_resume(&self, state: Value) {
        let restored = state.get("counter").and_then(Value::as_u64).unwrap_or(0);
        *self.counter.lock().unwrap() = restored;
        *self.resumed.lock().unwrap() = Some(state);
    }
}

// ===========================================================================
// Inputs / required-surface validation
// ===========================================================================

// clause: module_interface.execute.input.input_schema.missing
#[test]
fn module_interface_execute_input_input_schema_missing() {
    // A descriptor lacking `input_schema` MUST fail structural conformance.
    let mut desc = conforming_descriptor();
    desc.as_object_mut().unwrap().remove("input_schema");
    let errors = validate_descriptor(&desc);
    assert!(
        !errors.is_empty(),
        "missing input_schema must produce a conformance error"
    );
    assert!(errors.iter().any(|e| e.contains("input_schema")));
    // A fully-conforming descriptor produces no errors (control assertion).
    assert!(validate_descriptor(&conforming_descriptor()).is_empty());
}

// clause: module_interface.execute.input.output_schema.missing
#[test]
fn module_interface_execute_input_output_schema_missing() {
    let mut desc = conforming_descriptor();
    desc.as_object_mut().unwrap().remove("output_schema");
    let errors = validate_descriptor(&desc);
    assert!(
        !errors.is_empty(),
        "missing output_schema must produce a conformance error"
    );
    assert!(errors.iter().any(|e| e.contains("output_schema")));
}

// clause: module_interface.execute.input.description.missing
#[test]
fn module_interface_execute_input_description_missing() {
    let mut desc = conforming_descriptor();
    desc.as_object_mut().unwrap().remove("description");
    let errors = validate_descriptor(&desc);
    assert!(
        !errors.is_empty(),
        "missing description must produce a conformance error"
    );
    assert!(errors
        .iter()
        .any(|e| e.to_lowercase().contains("description")));
}

// clause: module_interface.execute.input.execute.missing
#[test]
#[ignore = "module_interface.execute.input.execute.missing: missing symbol \
            (contract gap) — `execute` is a required trait method enforced by \
            the Rust compiler, not a runtime-validated descriptor field; \
            `validate_descriptor` has no `execute` check to mirror."]
fn module_interface_execute_input_execute_missing() {
    // A class that does not implement `execute` simply does not satisfy the
    // `Module` trait and fails to compile — there is no runtime check to assert.
    let mut desc = conforming_descriptor();
    desc.as_object_mut().unwrap().remove("description");
    let errors = validate_descriptor(&desc);
    assert!(errors.iter().any(|e| e.to_lowercase().contains("execute")));
}

// clause: module_interface.execute.input.inputs.invalid_against_schema
#[test]
fn module_interface_execute_input_inputs_invalid_against_schema() {
    // `inputs` MUST validate against `input_schema` at execution time; a
    // validation failure surfaces as SCHEMA_VALIDATION_ERROR before the body.
    let validator = SchemaValidator::new();
    // `value` is required and must be a string; an int violates the schema.
    let err = validator
        .validate_input(&json!({ "value": 123 }), &echo_input_schema())
        .expect_err("schema-violating input must error");
    assert_eq!(err.code, ErrorCode::SchemaValidationError);
    assert_eq!(code_str(err.code), "SCHEMA_VALIDATION_ERROR");
}

// ===========================================================================
// Errors — declared error types
// ===========================================================================

// clause: module_interface.errors.MissingRequiredAttribute
#[test]
#[ignore = "module_interface.errors.MissingRequiredAttribute: missing symbol \
            (contract gap) — no such ErrorCode variant or error type in \
            apcore::errors; required-surface conformance is compiler-enforced \
            / `validate_descriptor` string output, not a typed error."]
fn module_interface_errors_missing_required_attribute() {
    unreachable!("missing symbol");
}

// clause: module_interface.errors.InvalidSchemaType
#[test]
#[ignore = "module_interface.errors.InvalidSchemaType: missing symbol \
            (contract gap) — no such ErrorCode variant in apcore::errors."]
fn module_interface_errors_invalid_schema_type() {
    unreachable!("missing symbol");
}

// clause: module_interface.errors.DescriptionTooLong
#[test]
#[ignore = "module_interface.errors.DescriptionTooLong: missing symbol \
            (contract gap) — no such ErrorCode variant; the 200-char limit is \
            not enforced via a typed error in apcore::errors."]
fn module_interface_errors_description_too_long() {
    unreachable!("missing symbol");
}

// clause: module_interface.errors.DocumentationTooLong
#[test]
#[ignore = "module_interface.errors.DocumentationTooLong: missing symbol \
            (contract gap) — no such ErrorCode variant in apcore::errors."]
fn module_interface_errors_documentation_too_long() {
    unreachable!("missing symbol");
}

// clause: module_interface.errors.InvalidAnnotations
#[test]
#[ignore = "module_interface.errors.InvalidAnnotations: missing symbol \
            (contract gap) — no such ErrorCode variant; `ModuleAnnotations` is \
            a typed struct so a wrong type is a compile error, not a runtime one."]
fn module_interface_errors_invalid_annotations() {
    unreachable!("missing symbol");
}

// clause: module_interface.errors.InvalidExample
#[test]
#[ignore = "module_interface.errors.InvalidExample: missing symbol \
            (contract gap) — no such ErrorCode variant; `ModuleExample` is a \
            typed struct without typed-error validation of title/inputs."]
fn module_interface_errors_invalid_example() {
    unreachable!("missing symbol");
}

// clause: module_interface.errors.MODULE_TIMEOUT
#[test]
fn module_interface_errors_module_timeout() {
    // After timeout the framework MUST raise MODULE_TIMEOUT. A ModuleError
    // built with ErrorCode::ModuleTimeout carries the exact wire code.
    let err = ModuleError::new(ErrorCode::ModuleTimeout, "module 'slow.module' timed out");
    assert_eq!(err.code, ErrorCode::ModuleTimeout);
    assert_eq!(code_str(err.code), "MODULE_TIMEOUT");
    // The descriptive message is preserved for AI-facing guidance.
    assert!(err.message.contains("slow.module"));
}

// ===========================================================================
// Properties
// ===========================================================================

// clause: module_interface.execute.property.async
#[tokio::test]
async fn module_interface_execute_property_async() {
    // execute() resolves to a dict validating against output_schema.
    let module = AsyncModule;
    let result = module
        .execute(json!({ "value": "hi" }), &make_ctx())
        .await
        .expect("async execute resolves");
    assert_eq!(result, json!({ "echoed": "hi" }));
    // Result validates against output_schema.
    let validator = SchemaValidator::new();
    validator
        .validate_output(&result, &echo_output_schema())
        .expect("result validates against output_schema");
}

// clause: module_interface.execute.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn module_interface_execute_property_thread_safe() {
    // Instances MUST tolerate concurrent execute() invocations. Launch >=8
    // concurrent calls with distinct inputs across spawned tasks; assert every
    // result is correct and no task panicked.
    let module = Arc::new(ConcurrentModule);
    let n: usize = 16;
    let mut handles = Vec::with_capacity(n);
    for i in 0..n {
        let m = Arc::clone(&module);
        handles.push(tokio::spawn(async move {
            let ctx = make_ctx();
            let out = m
                .execute(json!({ "value": format!("v{i}") }), &ctx)
                .await
                .expect("concurrent execute resolves");
            (i, out)
        }));
    }
    let mut results: Vec<(usize, Value)> = Vec::with_capacity(n);
    for h in handles {
        // No task panicked (join is Ok) and execute did not error.
        results.push(h.await.expect("spawned task must not panic"));
    }
    results.sort_by_key(|(i, _)| *i);
    // Every call returned its own distinct value — consistent state, no
    // cross-call interference.
    for (i, out) in &results {
        assert_eq!(out, &json!({ "echoed": format!("v{i}") }));
    }
    assert_eq!(results.len(), n);
}

// clause: module_interface.execute.property.pure_false
#[tokio::test]
async fn module_interface_execute_property_pure_false() {
    // pure is NOT required: a module MAY perform side effects. Assert the side
    // effect is observable AND conformance still holds.
    let counter = Arc::new(Mutex::new(0));
    let module = SideEffectfulModule {
        call_count: Arc::clone(&counter),
    };
    assert_eq!(*counter.lock().unwrap(), 0);
    module
        .execute(json!({ "value": "a" }), &make_ctx())
        .await
        .expect("execute a");
    module
        .execute(json!({ "value": "b" }), &make_ctx())
        .await
        .expect("execute b");
    // Side effect is observable — the contract permits this.
    assert_eq!(*counter.lock().unwrap(), 2);
    // Despite the side effect, the module still satisfies the required surface.
    assert!(validate_descriptor(&conforming_descriptor()).is_empty());
}

// ===========================================================================
// Return-value constraints
// ===========================================================================

// [mod-describe-annotations] The default describe() must include an
// `annotations` key (spec-default shape: description, input_schema,
// output_schema, annotations). Mirrors apcore-python / apcore-typescript.
#[test]
fn module_interface_default_describe_includes_annotations() {
    let module = ConformingModule;
    let described = module.describe();
    let obj = described.as_object().expect("describe() returns an object");
    assert!(
        obj.contains_key("annotations"),
        "default describe() must include an 'annotations' key; got: {described}"
    );
    // The other spec-default keys must still be present.
    for key in ["description", "input_schema", "output_schema"] {
        assert!(obj.contains_key(key), "describe() must include '{key}'");
    }
}

// clause: module_interface.execute.return.must_be_dict
#[tokio::test]
async fn module_interface_execute_return_must_be_dict() {
    // execute() MUST return a dict (JSON object) that validates against
    // output_schema.
    let module = ConformingModule;
    let result = module
        .execute(json!({ "value": "payload" }), &make_ctx())
        .await
        .expect("execute resolves");
    assert!(result.is_object(), "return value must be a JSON object");
    assert_eq!(result, json!({ "echoed": "payload" }));
    let validator = SchemaValidator::new();
    validator
        .validate_output(&result, &echo_output_schema())
        .expect("return value validates against output_schema");
}

// clause: module_interface.execute.return.must_be_dict
#[test]
fn module_interface_execute_return_validates_against_output_schema() {
    // A return value that does NOT match output_schema is rejected by the
    // validator with SCHEMA_VALIDATION_ERROR — confirming the output MUST
    // validate against output_schema.
    let validator = SchemaValidator::new();
    // 'echoed' missing entirely violates the required output schema.
    let err = validator
        .validate_output(&json!({}), &echo_output_schema())
        .expect_err("output missing required field must error");
    assert_eq!(err.code, ErrorCode::SchemaValidationError);
    assert_eq!(code_str(err.code), "SCHEMA_VALIDATION_ERROR");
}

// ===========================================================================
// Side effects — lifecycle hook ordering
// ===========================================================================

// clause: module_interface.lifecycle.side_effect.1.on_load
#[test]
fn module_interface_lifecycle_side_effect_1_on_load() {
    // on_load() MUST be invoked when the module is registered. It is the first
    // lifecycle hook observed (before any on_unload).
    let log = Arc::new(Mutex::new(Vec::<String>::new()));
    let registry = Registry::new();
    registry
        .register_module(
            "test.lifecycle_load",
            Box::new(LifecycleModule {
                log: Arc::clone(&log),
            }),
        )
        .expect("register lifecycle module");
    assert_eq!(*log.lock().unwrap(), vec!["on_load".to_string()]);
}

// clause: module_interface.lifecycle.side_effect.2.on_unload
#[test]
fn module_interface_lifecycle_side_effect_2_on_unload() {
    // on_unload() MUST be invoked when the module is unregistered, and MUST be
    // ordered strictly after on_load (register -> unregister).
    let log = Arc::new(Mutex::new(Vec::<String>::new()));
    let registry = Registry::new();
    registry
        .register_module(
            "test.lifecycle_unload",
            Box::new(LifecycleModule {
                log: Arc::clone(&log),
            }),
        )
        .expect("register lifecycle module");
    registry
        .unregister("test.lifecycle_unload")
        .expect("unregister lifecycle module");
    // Order MUST be on_load then on_unload.
    assert_eq!(
        *log.lock().unwrap(),
        vec!["on_load".to_string(), "on_unload".to_string()]
    );
}

// clause: module_interface.lifecycle.side_effect.3.suspend_resume
#[tokio::test]
async fn module_interface_lifecycle_side_effect_3_suspend_resume() {
    // on_suspend() exports JSON-serializable state which on_resume() restores.
    // The round-trip MUST preserve the state value (ordered suspend -> resume).
    let old = SuspendResumeModule::new();
    // Build up some state via execute().
    old.execute(json!({ "value": "x" }), &make_ctx())
        .await
        .expect("execute x");
    old.execute(json!({ "value": "y" }), &make_ctx())
        .await
        .expect("execute y");
    assert_eq!(old.count(), 2);

    // Export state (on_suspend) — MUST be JSON-serializable.
    let state = old.on_suspend().expect("on_suspend returns state");
    assert_eq!(state, json!({ "counter": 2 }));
    // Round-trips losslessly through JSON.
    let roundtripped: Value =
        serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
    assert_eq!(roundtripped, json!({ "counter": 2 }));

    // Restore into a new instance (on_resume) — state is preserved.
    let fresh = SuspendResumeModule::new();
    assert_eq!(fresh.count(), 0);
    fresh.on_resume(state.clone());
    assert_eq!(fresh.count(), 2);
    assert_eq!(*fresh.resumed.lock().unwrap(), Some(state));
}

// ===========================================================================
// Structural conformance (the Module trait surface)
// ===========================================================================

// clause: module_interface.execute.property.structural
#[test]
fn module_interface_execute_property_structural() {
    // Conformance is structural via the `Module` trait — any type implementing
    // the required surface IS usable as a `dyn Module`. Assert the conforming
    // type coerces to a trait object and exposes the required surface; and that
    // the real exported optional types (ModuleAnnotations / ModuleExample) are
    // constructible.
    let module: Box<dyn Module> = Box::new(ConformingModule);
    assert_eq!(
        module.description(),
        "Echoes the supplied value back unchanged."
    );
    assert!(module.input_schema().is_object());
    assert!(module.output_schema().is_object());

    // Sanity: ModuleAnnotations / ModuleExample are the real exported types.
    let ann = ModuleAnnotations::default();
    assert!(!ann.readonly);
    let mut example = ModuleExample::default();
    example.title = "t".to_string();
    example.inputs = json!({ "value": "v" });
    assert_eq!(example.title, "t");
}
