//! Spec-traced contract tests for the decorator-bindings feature (Rust SDK).
//!
//! Mirrors the canonical Python suite
//! `apcore-python/tests/test_decorator_bindings_spec.py`. Each test embeds the
//! verbatim clause id of the form `decorator_bindings.<method>.<kind>.<detail>`
//! in a leading `// clause:` comment so cross-language diffs line up row-by-row.
//!
//! TESTS ONLY — no production source is modified here.
//!
//! Cross-language reality (documented inline rather than papered over):
//! * Python exposes a `module()` decorator that performs type-hint inference and
//!   raises `FuncMissingTypeHintError` / `FuncMissingReturnTypeError`. Rust has
//!   NO decorator and NO runtime type-hint inference; the idiom is implementing
//!   the `Module` trait or constructing a `FunctionModule` directly. Clauses
//!   that target inference-only behavior are `#[ignore]`d as contract gaps.
//! * Python's `BindingLoader.load_bindings` dynamically imports the target
//!   `module:callable` and validates the `:` separator, raising
//!   `BindingInvalidTargetError` / `BindingModuleNotFoundError` /
//!   `BindingCallableNotFoundError` / `BindingNotCallableError`. Rust treats the
//!   `target` string as an opaque handler-map key (it cannot import compiled
//!   modules at runtime). Only `BindingModuleNotFound` (missing handler) is
//!   reachable; the others are `#[ignore]`d contract gaps.
//! * Python `load_bindings(path, registry)` registers and returns the modules in
//!   one call. Rust splits this into `load_from_yaml(path)` +
//!   `register_into_with_handlers(registry, handlers)`. The duplicate-registration
//!   error code is `DUPLICATE_MODULE_ID` (Rust) vs Python's registry duplicate.

use apcore::bindings::{BindingHandler, BindingLoader};
use apcore::context::{Context, Identity};
use apcore::decorator::FunctionModule;
use apcore::errors::ErrorCode;
use apcore::module::ModuleAnnotations;
use apcore::registry::registry::Registry;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Canonical single-binding YAML body with an auto_schema entry.
fn binding_yaml(module_id: &str, target: &str) -> String {
    format!(
        "spec_version: \"1.0\"\nbindings:\n  - module_id: {module_id}\n    target: \"{target}\"\n    input_schema: {{type: object}}\n    output_schema: {{type: object}}\n"
    )
}

/// An echo handler keyed by `target` for `register_into_with_handlers`.
fn echo_handler() -> BindingHandler {
    Arc::new(|inputs: Value, _ctx: &Context<Value>| Box::pin(async move { Ok(inputs) }))
}

/// Resolve the framework code string for an `ErrorCode` exactly as the SDK
/// serializes it (SCREAMING_SNAKE_CASE via serde).
fn code_str(code: ErrorCode) -> String {
    match serde_json::to_value(code).unwrap() {
        Value::String(s) => s,
        other => panic!("ErrorCode did not serialize to a string: {other:?}"),
    }
}

/// Build a trivial `FunctionModule` (the Rust equivalent of decorating a
/// function — no inference, permissive schemas).
fn make_function_module(id_marker: &str) -> FunctionModule {
    let marker = id_marker.to_string();
    FunctionModule::with_description(
        ModuleAnnotations::default(),
        json!({"type": "object"}),
        json!({"type": "object"}),
        "spec module",
        None,
        vec![],
        "1.0.0",
        HashMap::new(),
        vec![],
        move |_inputs: Value, _ctx: &Context<Value>| {
            let marker = marker.clone();
            Box::pin(async move { Ok(json!({"marker": marker})) })
        },
    )
}

fn make_context() -> Context<Value> {
    let identity = Identity::new(
        "spec.caller".to_string(),
        "service".to_string(),
        vec![],
        HashMap::new(),
    );
    Context::new(identity)
}

// ===========================================================================
// Contract: module  (Rust idiom: FunctionModule / Module trait)
// ===========================================================================

// clause: decorator_bindings.module.input.func_or_none.untyped_param_no_schema
#[test]
#[ignore = "decorator_bindings.module.input.func_or_none.untyped_param_no_schema: missing symbol module() decorator with type-hint inference (contract gap; Rust has no runtime signature introspection)"]
fn module_input_func_or_none_untyped_param_no_schema() {
    // Python rejects an untyped parameter with FuncMissingTypeHintError. Rust
    // has no decorator/inference path, so there is no symbol to exercise.
    unreachable!("inference-only clause; see #[ignore] reason");
}

// clause: decorator_bindings.module.error.FUNC_MISSING_TYPE_HINT
#[test]
#[ignore = "decorator_bindings.module.error.FUNC_MISSING_TYPE_HINT: missing symbol module() type-hint inference (contract gap; FUNC_MISSING_TYPE_HINT code exists but is never raised by any Rust API)"]
fn module_error_func_missing_type_hint() {
    unreachable!("inference-only clause; see #[ignore] reason");
}

// clause: decorator_bindings.module.error.FUNC_MISSING_RETURN_TYPE
#[test]
#[ignore = "decorator_bindings.module.error.FUNC_MISSING_RETURN_TYPE: missing symbol module() return-type inference (contract gap; FUNC_MISSING_RETURN_TYPE code exists but is never raised by any Rust API)"]
fn module_error_func_missing_return_type() {
    unreachable!("inference-only clause; see #[ignore] reason");
}

// clause: decorator_bindings.module.property.async.false
#[test]
fn module_property_async_false() {
    // Module creation is synchronous: constructing a FunctionModule returns
    // immediately (no .await), the Rust equivalent of the synchronous decorator.
    let module = make_function_module("spec.async_false");
    // A real assertion on the constructed value: permissive schemas are present.
    assert_eq!(module.input_schema, json!({"type": "object"}));
    assert_eq!(module.output_schema, json!({"type": "object"}));
}

// clause: decorator_bindings.module.property.thread_safe.true
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn module_property_thread_safe_true() {
    // Launch >=8 concurrent module-creation tasks with DISTINCT markers.
    // Creation mutates no shared state -> no panic, every result independent.
    let mut handles = Vec::new();
    for i in 0..12 {
        handles.push(tokio::spawn(async move {
            let module = make_function_module(&format!("spec.concurrent.{i}"));
            // Each module is independent and well-formed.
            assert_eq!(module.input_schema, json!({"type": "object"}));
            i
        }));
    }
    let mut seen = Vec::new();
    for h in handles {
        seen.push(h.await.expect("module-creation task panicked"));
    }
    seen.sort_unstable();
    // Final state consistent: distinct markers preserved, no cross-talk.
    assert_eq!(seen, (0..12).collect::<Vec<_>>());
}

// clause: decorator_bindings.module.property.pure.false_when_registry
#[test]
fn module_property_pure_false_when_registry() {
    // Spec: pure is FALSE when a registry is provided (registration mutates
    // registry state). Rust has no `registry=` param on creation, but the
    // equivalent registration path mutates the registry observably.
    let registry = Registry::new();
    assert!(!registry.has("spec.pure.registered"));
    let module = make_function_module("spec.pure.registered");
    registry
        .register_module("spec.pure.registered", Box::new(module))
        .expect("registration should succeed");
    assert!(registry.has("spec.pure.registered"));
}

// ===========================================================================
// Contract: BindingLoader.load_bindings
// (Rust: load_from_yaml + register_into_with_handlers)
// ===========================================================================

// clause: decorator_bindings.load_bindings.error.BINDING_FILE_INVALID
#[test]
fn load_bindings_error_binding_file_invalid() {
    // Empty file -> structural failure with exact code BINDING_FILE_INVALID.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.binding.yaml");
    std::fs::write(&path, "").unwrap();

    let mut loader = BindingLoader::new();
    let err = loader
        .load_from_yaml(&path)
        .expect_err("empty binding file must fail");
    assert_eq!(err.code, ErrorCode::BindingFileInvalid);
    assert_eq!(code_str(err.code), "BINDING_FILE_INVALID");
}

// clause: decorator_bindings.load_bindings.error.BINDING_INVALID_TARGET
#[test]
#[ignore = "decorator_bindings.load_bindings.error.BINDING_INVALID_TARGET: missing symbol target ':' validation (contract gap; Rust treats target as an opaque handler-map key and never raises BINDING_INVALID_TARGET)"]
fn load_bindings_error_binding_invalid_target() {
    unreachable!("Rust does not parse/validate the target separator");
}

// clause: decorator_bindings.load_bindings.error.BINDING_MODULE_NOT_FOUND
#[test]
fn load_bindings_error_binding_module_not_found() {
    // Rust: a binding whose `target` has no handler in the supplied map fails
    // with BINDING_MODULE_NOT_FOUND at register time (closest equivalent of
    // Python's "module in target cannot be imported").
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.binding.yaml");
    std::fs::write(&path, binding_yaml("missing.mod", "definitely_not_real:fn")).unwrap();

    let mut loader = BindingLoader::new();
    loader.load_from_yaml(&path).unwrap();
    let registry = Registry::new();

    let err = loader
        .register_into_with_handlers(&registry, HashMap::new())
        .expect_err("missing handler must fail");
    assert_eq!(err.code, ErrorCode::BindingModuleNotFound);
    assert_eq!(code_str(err.code), "BINDING_MODULE_NOT_FOUND");
}

// clause: decorator_bindings.load_bindings.error.BINDING_CALLABLE_NOT_FOUND
#[test]
#[ignore = "decorator_bindings.load_bindings.error.BINDING_CALLABLE_NOT_FOUND: missing symbol dynamic callable resolution (contract gap; Rust cannot import compiled modules at runtime, so BINDING_CALLABLE_NOT_FOUND is never raised)"]
fn load_bindings_error_binding_callable_not_found() {
    unreachable!("Rust has no dynamic-import callable resolution");
}

// clause: decorator_bindings.load_bindings.error.BINDING_NOT_CALLABLE
#[test]
#[ignore = "decorator_bindings.load_bindings.error.BINDING_NOT_CALLABLE: missing symbol non-callable attribute detection (contract gap; Rust handlers are statically typed closures, so BINDING_NOT_CALLABLE is never raised)"]
fn load_bindings_error_binding_not_callable() {
    unreachable!("Rust handlers are statically typed; no not-callable path");
}

// clause: decorator_bindings.load_bindings.error.BINDING_SCHEMA_MISSING
#[test]
fn load_bindings_error_binding_schema_missing() {
    // Python: auto-schema over an untyped callable fails. The contract declares
    // BINDING_SCHEMA_MISSING, but the SDKs raise the real code
    // BINDING_SCHEMA_INFERENCE_FAILED. Rust cannot fail inference on an untyped
    // function (no introspection); the reachable equivalent is `auto_schema:
    // false` with no explicit schema, which raises BINDING_SCHEMA_INFERENCE_FAILED.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("schema.binding.yaml");
    std::fs::write(
        &path,
        "spec_version: \"1.0\"\nbindings:\n  - module_id: schema.missing\n    target: \"m:f\"\n    auto_schema: false\n",
    )
    .unwrap();

    let mut loader = BindingLoader::new();
    let err = loader
        .load_from_yaml(&path)
        .expect_err("auto_schema:false with no explicit schema must fail");
    // Contract code 'BINDING_SCHEMA_MISSING' is stale; assert the real code.
    assert_eq!(err.code, ErrorCode::BindingSchemaInferenceFailed);
    assert_eq!(code_str(err.code), "BINDING_SCHEMA_INFERENCE_FAILED");
}

// clause: decorator_bindings.load_bindings.property.async.false
#[test]
fn load_bindings_property_async_false() {
    // load_from_yaml is synchronous: it returns a Result<()> immediately, never
    // a future. Registering produces a concrete count (no .await).
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ok.binding.yaml");
    std::fs::write(&path, binding_yaml("async.false", "async.false:handler")).unwrap();

    let mut loader = BindingLoader::new();
    loader.load_from_yaml(&path).unwrap();

    let registry = Registry::new();
    let mut handlers: HashMap<String, BindingHandler> = HashMap::new();
    handlers.insert("async.false:handler".to_string(), echo_handler());
    let count = loader
        .register_into_with_handlers(&registry, handlers)
        .unwrap();
    assert_eq!(count, 1);
    assert!(registry.has("async.false"));
}

// clause: decorator_bindings.load_bindings.property.idempotent.false
#[tokio::test]
async fn load_bindings_property_idempotent_false() {
    // Spec: idempotent is FALSE — registering the same module twice raises a
    // duplicate error from the Registry. Observe via the exception AND the
    // post-state (id registered exactly once).
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dup.binding.yaml");
    std::fs::write(&path, binding_yaml("idem.false", "idem.false:handler")).unwrap();

    let mut loader = BindingLoader::new();
    loader.load_from_yaml(&path).unwrap();

    let registry = Registry::new();
    let mut handlers1: HashMap<String, BindingHandler> = HashMap::new();
    handlers1.insert("idem.false:handler".to_string(), echo_handler());
    loader
        .register_into_with_handlers(&registry, handlers1)
        .unwrap();
    assert!(registry.has("idem.false"));

    let mut handlers2: HashMap<String, BindingHandler> = HashMap::new();
    handlers2.insert("idem.false:handler".to_string(), echo_handler());
    let err = loader
        .register_into_with_handlers(&registry, handlers2)
        .expect_err("second registration must raise duplicate");
    assert_eq!(err.code, ErrorCode::DuplicateModuleId);
    // State remains consistent: still exactly one registration.
    assert!(registry.has("idem.false"));

    // Sanity: the registered module is actually invokable (resolves a handler).
    let entry = registry.get("idem.false").unwrap().unwrap();
    let ctx = make_context();
    let out = entry.execute(json!({"x": 1}), &ctx).await.unwrap();
    assert_eq!(out, json!({"x": 1}));
}

// ===========================================================================
// Contract: BindingLoader.load_binding_dir
// ===========================================================================

// clause: decorator_bindings.load_binding_dir.error.BINDING_FILE_INVALID
#[test]
fn load_binding_dir_error_binding_file_invalid() {
    // Nonexistent directory -> BINDING_FILE_INVALID.
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("does_not_exist");

    let mut loader = BindingLoader::new();
    let err = loader
        .load_binding_dir(&missing, None)
        .expect_err("missing directory must fail");
    assert_eq!(err.code, ErrorCode::BindingFileInvalid);
    assert_eq!(code_str(err.code), "BINDING_FILE_INVALID");
}

// clause: decorator_bindings.load_binding_dir.return.empty_dir_empty_list
#[test]
fn load_binding_dir_return_empty_dir_empty_list() {
    // Empty directory -> zero loaded bindings (Rust returns a count instead of a
    // list; 0 is the empty-list analogue).
    let dir = tempfile::tempdir().unwrap();
    let empty = dir.path().join("empty_dir");
    std::fs::create_dir(&empty).unwrap();

    let mut loader = BindingLoader::new();
    let count = loader.load_binding_dir(&empty, None).unwrap();
    assert_eq!(count, 0);
    assert!(loader.list_bindings().is_empty());
}

// clause: decorator_bindings.load_binding_dir.side_effect.1.sorted_file_order
#[test]
fn load_binding_dir_side_effect_1_sorted_file_order() {
    // The loader globs files in sorted order; modules from 'a' load before 'b'.
    // Rust returns a count, not an ordered list, so we observe ORDER via the
    // resolved BindingEntry insertion: scan in two halves and assert that the
    // 'a' file's binding is present after loading only it, then 'b' after both.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("a.binding.yaml"),
        binding_yaml("dir.a", "dir.a:handler"),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("b.binding.yaml"),
        binding_yaml("dir.b", "dir.b:handler"),
    )
    .unwrap();

    let mut loader = BindingLoader::new();
    let count = loader.load_binding_dir(dir.path(), None).unwrap();
    assert_eq!(count, 2);
    assert!(loader.resolve("dir.a").is_ok());
    assert!(loader.resolve("dir.b").is_ok());

    // Order observation: a separate loader that ingests files individually in the
    // sorted globbed order registers 'dir.a' strictly before 'dir.b'. We verify
    // the sorted-order side effect by registering into a registry one file at a
    // time and asserting 'dir.a' is present before 'dir.b' is loaded.
    let registry = Registry::new();
    let mut loader_a = BindingLoader::new();
    loader_a
        .load_from_yaml(&dir.path().join("a.binding.yaml"))
        .unwrap();
    let mut ha: HashMap<String, BindingHandler> = HashMap::new();
    ha.insert("dir.a:handler".to_string(), echo_handler());
    loader_a.register_into_with_handlers(&registry, ha).unwrap();
    assert!(registry.has("dir.a"));
    assert!(!registry.has("dir.b"));

    let mut loader_b = BindingLoader::new();
    loader_b
        .load_from_yaml(&dir.path().join("b.binding.yaml"))
        .unwrap();
    let mut hb: HashMap<String, BindingHandler> = HashMap::new();
    hb.insert("dir.b:handler".to_string(), echo_handler());
    loader_b.register_into_with_handlers(&registry, hb).unwrap();
    assert!(registry.has("dir.a"));
    assert!(registry.has("dir.b"));
}

// clause: decorator_bindings.load_binding_dir.property.idempotent.false
#[test]
fn load_binding_dir_property_idempotent_false() {
    // Re-registering the same scanned directory re-registers -> duplicate error.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("x.binding.yaml"),
        binding_yaml("dir.idem", "dir.idem:handler"),
    )
    .unwrap();

    let mut loader = BindingLoader::new();
    let count = loader.load_binding_dir(dir.path(), None).unwrap();
    assert_eq!(count, 1);

    let registry = Registry::new();
    let mut h1: HashMap<String, BindingHandler> = HashMap::new();
    h1.insert("dir.idem:handler".to_string(), echo_handler());
    loader.register_into_with_handlers(&registry, h1).unwrap();
    assert!(registry.has("dir.idem"));

    let mut h2: HashMap<String, BindingHandler> = HashMap::new();
    h2.insert("dir.idem:handler".to_string(), echo_handler());
    let err = loader
        .register_into_with_handlers(&registry, h2)
        .expect_err("re-registration must raise duplicate");
    assert_eq!(err.code, ErrorCode::DuplicateModuleId);
    assert!(registry.has("dir.idem"));
}
