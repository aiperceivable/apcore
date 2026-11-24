// Spec-traced contract tests for the apcore-rust registry-system feature.
//
// Source spec: apcore/docs/features/registry-system.md
// Canonical clause list mirrored from:
//   apcore-python/tests/test_registry_system_spec.py
//
// Each test maps to exactly one clause in the feature spec's '## Contract:'
// blocks. The verbatim cross-language clause id appears in a leading
// `// clause: <clause_id>` comment on the line above each test fn so a
// cross-language diff tool can line up the Python / TypeScript / Rust rows by
// that exact string. The fn name is the clause id flattened to snake_case.
//
// Contract blocks covered:
//   - Registry.register
//   - Scanner.scan_extensions
//   - Registry.get
//   - Registry.list
//   - Registry.get_definition
//
// Cross-language API notes (Rust surface vs. Python canonical):
//   - Python `register()` returns None; Rust `register_module()` returns
//     `Result<(), ModuleError>` (success == `Ok(())`).
//   - Python rejects invalid IDs with code INVALID_MODULE_ID; Rust now emits
//     INVALID_MODULE_ID too (validate_module_id, registry.rs). Cross-language
//     parity per error-system.md §560 (normative MUST).
//   - Python `get()`/`get_definition()` raise on empty id; Rust returns
//     `Err(ModuleError { code: ModuleNotFound })`.
//   - Python `list(tags, prefix)`; Rust `list(tags, prefix, visibility)`.
//   - Python register-event callback receives `(module_id, payload)`; the Rust
//     callback signature is `Fn(&str, &dyn Module)` (module instance, no
//     metadata payload) — recorded as a cross-language divergence.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};

use apcore::context::Context;
use apcore::errors::{ErrorCode, ModuleError};
use apcore::module::Module;
use apcore::registry::registry::{module_id_pattern, ModuleDescriptor, Registry};
use apcore::registry::scanner::scan_extensions;
use apcore::registry::types::DiscoveredFile;

// ---------------------------------------------------------------------------
// Helper module + descriptor builders
// ---------------------------------------------------------------------------

/// A valid duck-typed module satisfying the Module trait. Carries optional
/// tags surfaced via `tags()` (mirrors the Python `SpecModule.tags`).
struct SpecModule {
    tags: Vec<String>,
}

impl SpecModule {
    fn new() -> Self {
        SpecModule {
            tags: vec!["alpha".to_string(), "beta".to_string()],
        }
    }
    fn with_tags(tags: &[&str]) -> Self {
        SpecModule {
            tags: tags.iter().map(|t| (*t).to_string()).collect(),
        }
    }
}

#[async_trait]
impl Module for SpecModule {
    fn input_schema(&self) -> Value {
        json!({ "type": "object", "properties": { "value": { "type": "string" } } })
    }
    fn output_schema(&self) -> Value {
        json!({ "type": "object", "properties": { "result": { "type": "string" } } })
    }
    fn description(&self) -> &'static str {
        "Spec fixture module"
    }
    fn tags(&self) -> Vec<String> {
        self.tags.clone()
    }
    async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        let value = inputs
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        Ok(json!({ "result": value }))
    }
}

/// Module recording an `on_load` invocation, with an optional observer that
/// fires synchronously inside `on_load` for ordering assertions.
struct OnLoadModule {
    loaded: Arc<AtomicBool>,
    observer: Option<Box<dyn Fn() + Send + Sync>>,
}

impl OnLoadModule {
    fn new() -> Self {
        OnLoadModule {
            loaded: Arc::new(AtomicBool::new(false)),
            observer: None,
        }
    }
    fn with_observer(observer: Box<dyn Fn() + Send + Sync>) -> Self {
        OnLoadModule {
            loaded: Arc::new(AtomicBool::new(false)),
            observer: Some(observer),
        }
    }
}

#[async_trait]
impl Module for OnLoadModule {
    fn input_schema(&self) -> Value {
        Value::Null
    }
    fn output_schema(&self) -> Value {
        Value::Null
    }
    fn description(&self) -> &'static str {
        "on_load fixture module"
    }
    fn on_load(&self) -> Result<(), ModuleError> {
        self.loaded.store(true, Ordering::SeqCst);
        if let Some(obs) = &self.observer {
            obs();
        }
        Ok(())
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(Value::Null)
    }
}

/// Module whose `on_load` fails, to exercise the failure path.
struct FailingOnLoadModule;

#[async_trait]
impl Module for FailingOnLoadModule {
    fn input_schema(&self) -> Value {
        Value::Null
    }
    fn output_schema(&self) -> Value {
        Value::Null
    }
    fn description(&self) -> &'static str {
        "failing on_load fixture module"
    }
    fn on_load(&self) -> Result<(), ModuleError> {
        Err(ModuleError::new(
            ErrorCode::ModuleLoadError,
            "on_load boom".to_string(),
        ))
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(Value::Null)
    }
}

/// Extract the `Err` from a `get()` result whose `Ok` arm (`Arc<dyn Module>`)
/// is not `Debug` and therefore cannot be unwrapped with `expect_err`.
fn expect_get_err(result: Result<Option<Arc<dyn Module>>, ModuleError>) -> ModuleError {
    match result {
        Ok(_) => panic!("expected Err, got Ok"),
        Err(e) => e,
    }
}

/// The serialized (SCREAMING_SNAKE_CASE) wire code carried by a `ModuleError`.
fn code_str(err: &ModuleError) -> String {
    err.to_dict()
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// Build a descriptor for `SpecModule` carrying its tags so that
/// `Registry::list(tags=...)` filters can observe them.
fn spec_descriptor(module_id: &str, m: &SpecModule) -> ModuleDescriptor {
    ModuleDescriptor {
        module_id: module_id.to_string(),
        name: None,
        description: m.description().to_string(),
        documentation: None,
        input_schema: m.input_schema(),
        output_schema: m.output_schema(),
        version: "1.0.0".to_string(),
        tags: m.tags(),
        annotations: None,
        examples: vec![],
        metadata: std::collections::HashMap::new(),
        display: None,
        sunset_date: None,
        dependencies: vec![],
        enabled: true,
    }
}

/// Register a `SpecModule` with a descriptor carrying its tags.
fn register_spec(reg: &Registry, module_id: &str, m: SpecModule) -> Result<(), ModuleError> {
    let descriptor = spec_descriptor(module_id, &m);
    reg.register(module_id, Box::new(m), descriptor)
}

// ===========================================================================
// Contract: Registry.register
// ===========================================================================

// clause: registry_system.register.input.module_id.empty
#[test]
fn register_input_module_id_empty_rejected() {
    let reg = Registry::new();
    let err =
        register_spec(&reg, "", SpecModule::new()).expect_err("empty module_id must be rejected");
    // Rust emits INVALID_MODULE_ID (cross-language parity with Python/TS).
    assert_eq!(err.code, ErrorCode::InvalidModuleId);
    assert_eq!(code_str(&err), "INVALID_MODULE_ID");
}

// clause: registry_system.register.input.module_id.malformed
#[test]
fn register_input_module_id_malformed_rejected() {
    let reg = Registry::new();
    // Hyphens are disallowed by MODULE_ID_PATTERN.
    assert!(!module_id_pattern().is_match("Bad-ID"));
    let err = register_spec(&reg, "Bad-ID", SpecModule::new())
        .expect_err("malformed module_id must be rejected");
    assert_eq!(err.code, ErrorCode::InvalidModuleId);
    assert_eq!(code_str(&err), "INVALID_MODULE_ID");
}

// clause: registry_system.register.input.module_id.reserved
#[test]
fn register_input_module_id_reserved_rejected() {
    let reg = Registry::new();
    // `system.*` is reserved; only register_internal() may use it.
    let err = register_spec(&reg, "system.thing", SpecModule::new())
        .expect_err("reserved-word module_id must be rejected");
    assert_eq!(err.code, ErrorCode::InvalidModuleId);
    assert_eq!(code_str(&err), "INVALID_MODULE_ID");
}

// clause: registry_system.register.error.INVALID_MODULE_ID
#[test]
fn register_error_invalid_module_id() {
    let reg = Registry::new();
    // Must start with a lowercase letter.
    let err = register_spec(&reg, "9bad", SpecModule::new())
        .expect_err("id not starting with a letter must be rejected");
    // Cross-language: Python/TS and Rust all emit INVALID_MODULE_ID.
    assert_eq!(code_str(&err), "INVALID_MODULE_ID");
}

// clause: registry_system.register.error.DUPLICATE_MODULE_ID
#[test]
fn register_error_duplicate_module_id() {
    let reg = Registry::new();
    register_spec(&reg, "math.add", SpecModule::new()).expect("first register");
    let err = register_spec(&reg, "math.add", SpecModule::new())
        .expect_err("duplicate id must be rejected");
    assert_eq!(err.code, ErrorCode::DuplicateModuleId);
    assert_eq!(code_str(&err), "DUPLICATE_MODULE_ID");
}

// clause: registry_system.register.return.none
#[test]
fn register_return_none_on_success() {
    let reg = Registry::new();
    let result: () = register_spec(&reg, "math.add", SpecModule::new())
        .expect("successful register returns Ok(())");
    assert_eq!(result, ());
    assert!(reg.get("math.add").expect("get ok").is_some());
}

// clause: registry_system.register.property.async
#[test]
fn register_property_async_false() {
    // Contract declares async: false -> register is a plain (non-async) call
    // returning a concrete Result without an executor.
    let reg = Registry::new();
    let result = register_spec(&reg, "math.add", SpecModule::new());
    assert!(result.is_ok());
}

// clause: registry_system.register.property.idempotent
#[test]
fn register_property_idempotent_false() {
    // Contract declares idempotent: false -> duplicate registration is an
    // error, not a no-op.
    let reg = Registry::new();
    register_spec(&reg, "math.add", SpecModule::new()).expect("first register");
    let err =
        register_spec(&reg, "math.add", SpecModule::new()).expect_err("second register must error");
    assert_eq!(err.code, ErrorCode::DuplicateModuleId);
    assert_eq!(code_str(&err), "DUPLICATE_MODULE_ID");
}

// clause: registry_system.register.property.pure
#[test]
fn register_property_pure_false() {
    // Contract declares pure: false -> mutates the internal store. Observe the
    // mutation via the public list() API.
    let reg = Registry::new();
    let before = reg.list(None, None, None);
    register_spec(&reg, "math.add", SpecModule::new()).expect("register");
    let after = reg.list(None, None, None);
    assert!(!before.contains(&"math.add".to_string()));
    assert!(after.contains(&"math.add".to_string()));
}

// clause: registry_system.register.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn register_property_thread_safe() {
    // Contract declares thread_safe: true. Drive >=8 concurrent registrations
    // of distinct IDs; all must land without corruption or deadlock.
    let reg = Arc::new(Registry::new());
    let n = 12;
    let mut handles = Vec::new();
    for i in 0..n {
        let reg = Arc::clone(&reg);
        handles.push(tokio::spawn(async move {
            register_spec(&reg, &format!("mod.m{i}"), SpecModule::new())
        }));
    }
    for h in handles {
        h.await.expect("task join").expect("register succeeds");
    }
    let listed = reg.list(None, None, None);
    for i in 0..n {
        assert!(listed.contains(&format!("mod.m{i}")));
    }
    assert_eq!(reg.count(), usize::try_from(n).unwrap());
}

// clause: registry_system.register.property.thread_safe.duplicate
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn register_property_thread_safe_duplicate_single_winner() {
    // Concurrent registrations of the SAME id: exactly one wins, the rest fail
    // with DUPLICATE_MODULE_ID (in-flight set + lock guard).
    let reg = Arc::new(Registry::new());
    let n = 10;
    let mut handles = Vec::new();
    for _ in 0..n {
        let reg = Arc::clone(&reg);
        handles.push(tokio::spawn(async move {
            register_spec(&reg, "dup.mod", SpecModule::new())
        }));
    }
    let mut errors: Vec<ModuleError> = Vec::new();
    let mut successes = 0;
    for h in handles {
        match h.await.expect("task join") {
            Ok(()) => successes += 1,
            Err(e) => errors.push(e),
        }
    }
    assert!(reg.get("dup.mod").expect("get ok").is_some());
    assert_eq!(successes, 1, "exactly one registration must win");
    assert_eq!(errors.len(), n - 1);
    assert!(errors
        .iter()
        .all(|e| e.code == ErrorCode::DuplicateModuleId));
}

// clause: registry_system.register.side_effect.1.validate_before_mutation
#[test]
fn register_side_effect_1_validate_before_mutation() {
    // Side-effect ordering: module_id is validated before any store mutation.
    // An invalid id must leave the store untouched.
    let reg = Registry::new();
    let err = register_spec(&reg, "Bad-ID", SpecModule::new()).expect_err("invalid id must error");
    assert_eq!(err.code, ErrorCode::InvalidModuleId);
    assert_eq!(reg.list(None, None, None), Vec::<String>::new());
    assert_eq!(reg.count(), 0);
}

// clause: registry_system.register.side_effect.6.on_load_invoked
#[test]
fn register_side_effect_6_on_load_invoked() {
    // Side-effect step 6: module.on_load() is invoked during registration.
    let reg = Registry::new();
    let module = OnLoadModule::new();
    let loaded = Arc::clone(&module.loaded);
    reg.register_module("life.cycle", Box::new(module))
        .expect("register");
    assert!(loaded.load(Ordering::SeqCst));
}

// clause: registry_system.register.side_effect.6.on_load_before_visible
#[test]
fn register_side_effect_6_on_load_before_visible() {
    // Side-effect ordering (steps 6 then 7): on_load() MUST complete before the
    // module becomes visible via get(). At the moment on_load runs, get() must
    // NOT yet return the module.
    let reg = Arc::new(Registry::new());
    let observed_visible: Arc<Mutex<Vec<bool>>> = Arc::new(Mutex::new(Vec::new()));

    let reg_for_obs = Arc::clone(&reg);
    let obs_log = Arc::clone(&observed_visible);
    let module = OnLoadModule::with_observer(Box::new(move || {
        let visible = reg_for_obs.get("life.cycle").ok().flatten().is_some();
        obs_log.lock().unwrap().push(visible);
    }));

    reg.register_module("life.cycle", Box::new(module))
        .expect("register");

    assert_eq!(*observed_visible.lock().unwrap(), vec![false]);
    // After registration completes the module IS visible.
    assert!(reg.get("life.cycle").expect("get ok").is_some());
}

// clause: registry_system.register.side_effect.6.on_load_failure_not_visible
#[test]
fn register_side_effect_6_on_load_failure_not_visible() {
    // Side-effect step 6 failure branch: if on_load fails, the original error
    // propagates AND the module never becomes visible.
    let reg = Registry::new();
    let err = reg
        .register_module("life.boom", Box::new(FailingOnLoadModule))
        .expect_err("failing on_load must propagate");
    assert_eq!(err.code, ErrorCode::ModuleLoadError);
    assert!(err.message.contains("on_load boom"));
    assert!(reg.get("life.boom").expect("get ok").is_none());
    assert!(!reg
        .list(None, None, None)
        .contains(&"life.boom".to_string()));
}

// clause: registry_system.register.side_effect.8.register_event_emitted
#[test]
fn register_side_effect_8_register_event_emitted() {
    // Side-effect step 8: a `register` event is emitted to subscribers after
    // successful publication. Rust callback signature is Fn(&str, &dyn Module)
    // (module instance, no metadata payload — cross-language divergence).
    let reg = Registry::new();
    let received: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let received_cb = Arc::clone(&received);
    reg.on(
        "register",
        Box::new(move |module_id: &str, _m: &dyn Module| {
            received_cb.lock().unwrap().push(module_id.to_string());
        }),
    )
    .expect("valid registry event");

    register_spec(&reg, "math.add", SpecModule::new()).expect("register");

    let got = received.lock().unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0], "math.add");
}

// clause: registry_system.register.side_effect.ordering.load_then_event
#[test]
fn register_side_effect_ordering_load_then_event() {
    // Combined ordering: on_load (step 6) fires before the register event
    // (step 8). Observe order via a shared sequence list.
    let reg = Registry::new();
    let sequence: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let seq_load = Arc::clone(&sequence);
    let module = OnLoadModule::with_observer(Box::new(move || {
        seq_load.lock().unwrap().push("on_load".to_string());
    }));

    let seq_event = Arc::clone(&sequence);
    reg.on(
        "register",
        Box::new(move |_module_id: &str, _m: &dyn Module| {
            seq_event.lock().unwrap().push("event".to_string());
        }),
    )
    .expect("valid registry event");

    reg.register_module("order.mod", Box::new(module))
        .expect("register");

    assert_eq!(
        *sequence.lock().unwrap(),
        vec!["on_load".to_string(), "event".to_string()]
    );
}

// ===========================================================================
// Contract: Scanner.scan_extensions
// ===========================================================================

// clause: registry_system.scan_extensions.input.root.missing
#[test]
fn scan_extensions_input_root_missing_rejected() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let missing = tmp.path().join("does_not_exist");
    let err = scan_extensions(&missing, 8, false, None, &[]).expect_err("missing root must error");
    assert_eq!(err.code, ErrorCode::ConfigNotFound);
    assert_eq!(code_str(&err), "CONFIG_NOT_FOUND");
}

// clause: registry_system.scan_extensions.error.CONFIG_NOT_FOUND
#[test]
fn scan_extensions_error_config_not_found() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let err = scan_extensions(&tmp.path().join("nope"), 8, false, None, &[])
        .expect_err("missing root must error");
    assert_eq!(err.code, ErrorCode::ConfigNotFound);
    assert_eq!(code_str(&err), "CONFIG_NOT_FOUND");
}

// clause: registry_system.scan_extensions.return.discovered_modules
#[test]
fn scan_extensions_return_ordered_records() {
    // On success returns a sequence of DiscoveredFile records (path + derived
    // canonical id). Rust returns DiscoveredFile (path-only payload; module-ID
    // derivation downstream) — D10-014.
    let tmp = tempfile::tempdir().expect("tempdir");
    let ext = tmp.path().join("ext");
    std::fs::create_dir(&ext).expect("mkdir ext");
    std::fs::write(ext.join("hello.rs"), b"struct HelloModule;").expect("write hello");
    std::fs::write(ext.join("greet.rs"), b"struct GreetModule;").expect("write greet");

    let results: Vec<DiscoveredFile> =
        scan_extensions(&ext, 8, false, None, &[]).expect("scan succeeds");
    let ids: std::collections::HashSet<String> =
        results.iter().map(|dm| dm.canonical_id.clone()).collect();
    assert_eq!(
        ids,
        ["hello".to_string(), "greet".to_string()]
            .into_iter()
            .collect()
    );
    // Records expose a concrete file path with the .rs suffix.
    assert!(results
        .iter()
        .all(|dm| dm.file_path.extension().and_then(|e| e.to_str()) == Some("rs")));
}

// clause: registry_system.scan_extensions.property.async
#[test]
fn scan_extensions_property_async_false() {
    // Contract declares async: false -> scan_extensions is a plain sync call.
    let tmp = tempfile::tempdir().expect("tempdir");
    let results = scan_extensions(tmp.path(), 8, false, None, &[]).expect("scan empty");
    assert!(results.is_empty());
}

// clause: registry_system.scan_extensions.property.pure
#[test]
fn scan_extensions_property_pure_false_reads_filesystem() {
    // Contract declares pure: false (reads the filesystem). Adding a file
    // changes the output for the same root argument.
    let tmp = tempfile::tempdir().expect("tempdir");
    let ext = tmp.path().join("ext");
    std::fs::create_dir(&ext).expect("mkdir ext");
    std::fs::write(ext.join("a.rs"), b"struct A;").expect("write a");
    let first: std::collections::HashSet<String> = scan_extensions(&ext, 8, false, None, &[])
        .expect("scan 1")
        .iter()
        .map(|dm| dm.canonical_id.clone())
        .collect();
    std::fs::write(ext.join("b.rs"), b"struct B;").expect("write b");
    let second: std::collections::HashSet<String> = scan_extensions(&ext, 8, false, None, &[])
        .expect("scan 2")
        .iter()
        .map(|dm| dm.canonical_id.clone())
        .collect();
    assert_eq!(first, ["a".to_string()].into_iter().collect());
    assert_eq!(
        second,
        ["a".to_string(), "b".to_string()].into_iter().collect()
    );
}

// clause: registry_system.scan_extensions.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scan_extensions_property_thread_safe() {
    // Contract declares thread_safe: true (no shared mutable state). >=8
    // concurrent scans of the same root return consistent results.
    let tmp = tempfile::tempdir().expect("tempdir");
    let ext = tmp.path().join("ext");
    std::fs::create_dir(&ext).expect("mkdir ext");
    for i in 0..5 {
        std::fs::write(ext.join(format!("m{i}.rs")), b"struct M;").expect("write");
    }
    let ext = Arc::new(ext);

    let mut handles = Vec::new();
    for _ in 0..8 {
        let ext = Arc::clone(&ext);
        handles.push(tokio::spawn(async move {
            let out: std::collections::HashSet<String> = scan_extensions(&ext, 8, false, None, &[])
                .expect("scan")
                .iter()
                .map(|dm| dm.canonical_id.clone())
                .collect();
            out
        }));
    }
    let expected: std::collections::HashSet<String> = (0..5).map(|i| format!("m{i}")).collect();
    for h in handles {
        let r = h.await.expect("task join");
        assert_eq!(r, expected);
    }
}

// ===========================================================================
// Contract: Registry.get
// ===========================================================================

// clause: registry_system.get.input.module_id.empty
#[test]
fn get_input_module_id_empty_rejected() {
    let reg = Registry::new();
    let err = expect_get_err(reg.get(""));
    assert_eq!(err.code, ErrorCode::ModuleNotFound);
}

// clause: registry_system.get.error.MODULE_NOT_FOUND
#[test]
fn get_error_module_not_found_on_empty() {
    // Empty module_id yields Err(ModuleNotFound); the serialized code is
    // MODULE_NOT_FOUND.
    let reg = Registry::new();
    let err = expect_get_err(reg.get(""));
    assert_eq!(err.code, ErrorCode::ModuleNotFound);
    assert_eq!(code_str(&err), "MODULE_NOT_FOUND");
}

// clause: registry_system.get.return.none_when_absent
#[test]
fn get_return_none_for_unregistered() {
    // A well-formed but unregistered id returns Ok(None) (no error).
    let reg = Registry::new();
    assert!(reg.get("not.registered").expect("get ok").is_none());
}

// clause: registry_system.get.return.instance_when_found
#[test]
fn get_return_instance_when_found() {
    let reg = Registry::new();
    register_spec(&reg, "math.add", SpecModule::new()).expect("register");
    let got = reg.get("math.add").expect("get ok");
    assert!(got.is_some());
    assert_eq!(got.unwrap().description(), "Spec fixture module");
}

// clause: registry_system.get.property.async
#[test]
fn get_property_async_false() {
    // Contract declares async: false -> get is a plain sync call.
    let reg = Registry::new();
    let result = reg.get("not.registered");
    assert!(result.is_ok());
}

// clause: registry_system.get.property.idempotent
#[test]
fn get_property_idempotent_true() {
    // Read-only; repeated calls return the same module while the registry is
    // unchanged (Arc points to the same instance).
    let reg = Registry::new();
    register_spec(&reg, "math.add", SpecModule::new()).expect("register");
    let a = reg.get("math.add").expect("get ok").expect("present");
    let b = reg.get("math.add").expect("get ok").expect("present");
    assert!(Arc::ptr_eq(&a, &b));
}

// clause: registry_system.get.property.pure
#[test]
fn get_property_pure_false_reads_shared_state() {
    // Contract declares pure: false (reads shared mutable state). The result
    // changes when the underlying store changes.
    let reg = Registry::new();
    assert!(reg.get("math.add").expect("get ok").is_none());
    register_spec(&reg, "math.add", SpecModule::new()).expect("register");
    assert!(reg.get("math.add").expect("get ok").is_some());
}

// clause: registry_system.get.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn get_property_thread_safe() {
    // Contract declares thread_safe: true. >=8 concurrent reads must not
    // corrupt or deadlock.
    let reg = Arc::new(Registry::new());
    register_spec(&reg, "math.add", SpecModule::new()).expect("register");
    let mut handles = Vec::new();
    for _ in 0..10 {
        let reg = Arc::clone(&reg);
        handles.push(tokio::spawn(async move {
            reg.get("math.add").expect("get ok").is_some()
        }));
    }
    for h in handles {
        assert!(h.await.expect("task join"));
    }
}

// ===========================================================================
// Contract: Registry.list
// ===========================================================================

// clause: registry_system.list.input.tags.superset_match
#[test]
fn list_input_tags_filter_all_or_nothing() {
    // When tags supplied, only modules whose tag set is a superset of ALL
    // supplied tags are included.
    let reg = Registry::new();
    register_spec(&reg, "m.alpha", SpecModule::with_tags(&["alpha", "beta"])).expect("reg alpha");
    register_spec(&reg, "m.gamma", SpecModule::with_tags(&["gamma"])).expect("reg gamma");

    assert_eq!(reg.list(Some(&["alpha"]), None, None), vec!["m.alpha"]);
    assert_eq!(
        reg.list(Some(&["alpha", "beta"]), None, None),
        vec!["m.alpha"]
    );
    // Requiring a tag not all-present excludes the module.
    assert_eq!(
        reg.list(Some(&["alpha", "missing"]), None, None),
        Vec::<String>::new()
    );
}

// clause: registry_system.list.input.tags.empty_no_filter
#[test]
fn list_input_tags_empty_means_no_filter() {
    // An empty tags list MUST be treated the same as None (no tag filter).
    let reg = Registry::new();
    register_spec(&reg, "m.alpha", SpecModule::with_tags(&["alpha"])).expect("reg alpha");
    register_spec(&reg, "m.gamma", SpecModule::with_tags(&["gamma"])).expect("reg gamma");

    let empty: &[&str] = &[];
    let mut from_empty = reg.list(Some(empty), None, None);
    let mut from_none = reg.list(None, None, None);
    from_empty.sort();
    from_none.sort();
    assert_eq!(from_empty, from_none);
    assert_eq!(from_empty, vec!["m.alpha", "m.gamma"]);
}

// clause: registry_system.list.input.prefix.startswith
#[test]
fn list_input_prefix_exact_startswith() {
    // Prefix matching is exact string prefix (starts_with), not glob/regex.
    let reg = Registry::new();
    register_spec(&reg, "math.add", SpecModule::new()).expect("reg add");
    register_spec(&reg, "math.sub", SpecModule::new()).expect("reg sub");
    register_spec(&reg, "string.upper", SpecModule::new()).expect("reg upper");

    assert_eq!(
        reg.list(None, Some("math."), None),
        vec!["math.add", "math.sub"]
    );
    // '*' is literal, not a wildcard -> matches nothing.
    assert_eq!(reg.list(None, Some("math.*"), None), Vec::<String>::new());
}

// clause: registry_system.list.input.combined.tags_and_prefix
#[test]
fn list_input_tags_and_prefix_combined() {
    let reg = Registry::new();
    register_spec(&reg, "math.add", SpecModule::with_tags(&["arith"])).expect("reg add");
    register_spec(&reg, "math.sub", SpecModule::with_tags(&["other"])).expect("reg sub");
    register_spec(&reg, "string.cat", SpecModule::with_tags(&["arith"])).expect("reg cat");

    assert_eq!(
        reg.list(Some(&["arith"]), Some("math."), None),
        vec!["math.add"]
    );
}

// clause: registry_system.list.error.none
#[test]
fn list_error_unknown_tag_returns_empty() {
    // No error for unknown tags/prefixes that match nothing -> empty list.
    let reg = Registry::new();
    register_spec(&reg, "math.add", SpecModule::with_tags(&["arith"])).expect("reg add");
    assert_eq!(
        reg.list(Some(&["nonexistent"]), None, None),
        Vec::<String>::new()
    );
    assert_eq!(reg.list(None, Some("zzz"), None), Vec::<String>::new());
}

// clause: registry_system.list.return.sorted_unique
#[test]
fn list_return_sorted_unique() {
    // Returns a lexicographically sorted list of unique module ID strings.
    let reg = Registry::new();
    for mid in ["zeta.one", "alpha.two", "mid.three"] {
        register_spec(&reg, mid, SpecModule::new()).expect("register");
    }
    let result = reg.list(None, None, None);
    assert_eq!(result, vec!["alpha.two", "mid.three", "zeta.one"]);
    let unique: std::collections::HashSet<&String> = result.iter().collect();
    assert_eq!(unique.len(), result.len());
}

// clause: registry_system.list.property.async
#[test]
fn list_property_async_false() {
    // Contract declares async: false -> list is a plain sync call.
    let reg = Registry::new();
    let result = reg.list(None, None, None);
    assert_eq!(result, Vec::<String>::new());
}

// clause: registry_system.list.property.idempotent
#[test]
fn list_property_idempotent_true() {
    // Same registry state -> identical result across calls.
    let reg = Registry::new();
    register_spec(&reg, "a.one", SpecModule::new()).expect("reg a");
    register_spec(&reg, "b.two", SpecModule::new()).expect("reg b");
    assert_eq!(reg.list(None, None, None), reg.list(None, None, None));
}

// clause: registry_system.list.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn list_property_thread_safe() {
    // Contract declares thread_safe: true (snapshot under lock before filtering).
    // >=8 concurrent list() calls return sorted lists without corruption.
    let reg = Arc::new(Registry::new());
    for i in 0..6 {
        register_spec(&reg, &format!("mod.m{i}"), SpecModule::new()).expect("register");
    }
    let mut handles = Vec::new();
    for _ in 0..10 {
        let reg = Arc::clone(&reg);
        handles.push(tokio::spawn(async move { reg.list(None, None, None) }));
    }
    let expected: std::collections::HashSet<String> = (0..6).map(|i| format!("mod.m{i}")).collect();
    for h in handles {
        let r = h.await.expect("task join");
        let mut sorted = r.clone();
        sorted.sort();
        assert_eq!(r, sorted, "list output is always sorted");
        assert!(r.iter().all(|id| expected.contains(id)));
    }
}

// ===========================================================================
// Contract: Registry.get_definition
// ===========================================================================

// clause: registry_system.get_definition.input.module_id.empty
#[test]
fn get_definition_input_module_id_empty_propagates() {
    // Any error get(module_id) raises is propagated (ModuleNotFound on empty).
    let reg = Registry::new();
    let err = reg.get_definition("").expect_err("empty id must error");
    assert_eq!(err.code, ErrorCode::ModuleNotFound);
}

// clause: registry_system.get_definition.error.MODULE_NOT_FOUND
#[test]
fn get_definition_error_propagates_from_get() {
    let reg = Registry::new();
    let err = reg.get_definition("").expect_err("empty id must error");
    assert_eq!(err.code, ErrorCode::ModuleNotFound);
    assert_eq!(code_str(&err), "MODULE_NOT_FOUND");
}

// clause: registry_system.get_definition.return.none_when_absent
#[test]
fn get_definition_return_none_for_unregistered() {
    // A well-formed but unregistered id returns Ok(None) (no error).
    let reg = Registry::new();
    assert!(reg
        .get_definition("not.registered")
        .expect("get_definition ok")
        .is_none());
}

// clause: registry_system.get_definition.return.descriptor_fields
#[test]
fn get_definition_return_descriptor_fields() {
    // On success returns a ModuleDescriptor with the contracted fields.
    let reg = Registry::new();
    register_spec(&reg, "math.add", SpecModule::with_tags(&["alpha", "beta"])).expect("register");

    let desc = reg
        .get_definition("math.add")
        .expect("get_definition ok")
        .expect("descriptor present");
    assert_eq!(desc.module_id, "math.add");
    assert_eq!(desc.description, "Spec fixture module");
    assert!(desc.input_schema.is_object());
    assert!(desc.output_schema.is_object());
    assert_eq!(desc.version, "1.0.0");
    assert!(desc.tags.contains(&"alpha".to_string()));
    assert!(desc.tags.contains(&"beta".to_string()));
}

// clause: registry_system.get_definition.property.async
#[test]
fn get_definition_property_async_false() {
    // Contract declares async: false -> get_definition is a plain sync call.
    let reg = Registry::new();
    let result = reg.get_definition("not.registered");
    assert!(result.is_ok());
}

// clause: registry_system.get_definition.property.idempotent
#[test]
fn get_definition_property_idempotent_true() {
    // For the same registry state, returns an equivalent descriptor each call.
    let reg = Registry::new();
    register_spec(&reg, "math.add", SpecModule::new()).expect("register");
    let d1 = reg
        .get_definition("math.add")
        .expect("ok")
        .expect("present");
    let d2 = reg
        .get_definition("math.add")
        .expect("ok")
        .expect("present");
    assert_eq!(d1.module_id, d2.module_id);
    assert_eq!(d1.input_schema, d2.input_schema);
    assert_eq!(d1.output_schema, d2.output_schema);
}

// clause: registry_system.get_definition.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn get_definition_property_thread_safe() {
    // Contract declares thread_safe: true. >=8 concurrent get_definition calls
    // return consistent descriptors.
    let reg = Arc::new(Registry::new());
    register_spec(&reg, "math.add", SpecModule::new()).expect("register");
    let mut handles = Vec::new();
    for _ in 0..10 {
        let reg = Arc::clone(&reg);
        handles.push(tokio::spawn(async move {
            reg.get_definition("math.add")
                .expect("ok")
                .map(|d| d.module_id)
        }));
    }
    for h in handles {
        let id = h.await.expect("task join");
        assert_eq!(id, Some("math.add".to_string()));
    }
}

// ===========================================================================
// D-86 — `Registry.register` validation order
// ===========================================================================
//
// `registry-system.md` "Side Effects (ordered)" pins it as
// `module_id` -> structure/streaming -> custom validator -> duplicate:
// intrinsic-then-extrinsic, because what is wrong with the MODULE must be
// fixed either way, whereas a duplicate id may only mean the author picked the
// wrong name. This SDK's order is the one the decision adopted, and it had no
// test for it; apcore-python pinned it, apcore-typescript did not.
//
// Each test makes TWO checks fail at once and asserts which is reported. That
// is the entire discriminator: a test tripping one check at a time passes under
// every ordering, including the three the SDKs actually shipped.

/// Declares `streaming: true` and implements no streaming interface.
#[derive(Debug)]
struct StreamingLiar;

#[async_trait]
impl Module for StreamingLiar {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "Claims to stream"
    }
    fn annotations(&self) -> apcore::module::ModuleAnnotations {
        let mut ann = apcore::module::ModuleAnnotations::default();
        ann.streaming = true;
        ann
    }
    async fn execute(&self, _i: Value, _c: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
}

#[derive(Debug)]
struct RejectAllValidator;

impl apcore::registry::registry::ModuleValidator for RejectAllValidator {
    fn validate(
        &self,
        _module: &dyn Module,
        _descriptor: Option<&ModuleDescriptor>,
    ) -> apcore::module::ValidationResult {
        let mut result = apcore::module::ValidationResult::default();
        result.valid = false;
        result.errors = vec![apcore::module::ValidationErrorDetail::message_only(
            "rejected by the custom validator",
        )];
        result
    }
}

// clause: registry_system.register.order.structure_before_validator
#[test]
fn register_reports_structure_before_the_custom_validator() {
    let reg = Registry::new();
    reg.set_validator(Box::new(RejectAllValidator));

    let err = reg
        .register_module("order.streaming_and_validator", Box::new(StreamingLiar))
        .expect_err("both checks fail");
    assert_eq!(err.code, ErrorCode::StreamingInterfaceMismatch, "{err:?}");
}

// clause: registry_system.register.order.structure_before_duplicate
#[test]
fn register_reports_structure_before_the_duplicate_check() {
    // Malformed, validator-rejected AND duplicate at once: structure wins.
    let reg = Registry::new();
    reg.register_module("order.all_three", Box::new(SpecModule::new()))
        .expect("incumbent registers");
    reg.set_validator(Box::new(RejectAllValidator));

    let err = reg
        .register_module("order.all_three", Box::new(StreamingLiar))
        .expect_err("all three checks fail");
    assert_eq!(err.code, ErrorCode::StreamingInterfaceMismatch, "{err:?}");

    // The incumbent registration is untouched — a rejected register must not
    // have removed or replaced what was already there.
    assert!(reg.get("order.all_three").expect("lookup").is_some());
}

// clause: registry_system.register.order.validator_before_duplicate
#[test]
fn register_reports_the_custom_validator_before_the_duplicate_check() {
    let reg = Registry::new();
    reg.register_module("order.validator_and_dup", Box::new(SpecModule::new()))
        .expect("incumbent registers");
    reg.set_validator(Box::new(RejectAllValidator));

    let err = reg
        .register_module("order.validator_and_dup", Box::new(SpecModule::new()))
        .expect_err("both checks fail");
    assert_eq!(err.code, ErrorCode::GeneralInvalidInput, "{err:?}");
}

// clause: registry_system.register.order.module_id_first
#[test]
fn register_reports_the_module_id_before_everything_else() {
    let reg = Registry::new();
    reg.set_validator(Box::new(RejectAllValidator));

    let err = reg
        .register_module("Not A Valid Id", Box::new(StreamingLiar))
        .expect_err("id, structure and validator all fail");
    assert_eq!(err.code, ErrorCode::InvalidModuleId, "{err:?}");
}

// clause: registry_system.register.order.each_check_fires_alone
#[test]
fn control_each_register_check_still_fires_on_its_own() {
    // Without this, "structure wins" is also satisfied by an implementation
    // reporting StreamingInterfaceMismatch for everything, and "validator
    // wins" by one that never reaches the duplicate check at all.
    let structure_only = Registry::new();
    assert_eq!(
        structure_only
            .register_module("order.stream_only", Box::new(StreamingLiar))
            .expect_err("structure alone")
            .code,
        ErrorCode::StreamingInterfaceMismatch
    );

    let validator_only = Registry::new();
    validator_only.set_validator(Box::new(RejectAllValidator));
    assert_eq!(
        validator_only
            .register_module("order.validator_only", Box::new(SpecModule::new()))
            .expect_err("validator alone")
            .code,
        ErrorCode::GeneralInvalidInput
    );

    let duplicate_only = Registry::new();
    duplicate_only
        .register_module("order.dup_only", Box::new(SpecModule::new()))
        .expect("incumbent");
    assert_eq!(
        duplicate_only
            .register_module("order.dup_only", Box::new(SpecModule::new()))
            .expect_err("duplicate alone")
            .code,
        ErrorCode::DuplicateModuleId
    );
}
