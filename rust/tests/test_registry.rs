//! Tests for Registry — creation, read-only operations, and new methods.

use apcore::context::{Context, Identity};
use apcore::errors::ModuleError;
use apcore::module::{Module, ModuleAnnotations};
use apcore::registry::registry::{ModuleDescriptor, Registry};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct StubModule;

#[async_trait]
impl Module for StubModule {
    fn description(&self) -> &'static str {
        "stub"
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({ "type": "object" })
    }
    fn output_schema(&self) -> Value {
        serde_json::json!({ "type": "object" })
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(serde_json::json!({}))
    }
}

fn make_descriptor(name: &str) -> ModuleDescriptor {
    ModuleDescriptor {
        module_id: name.to_string(),
        name: None,
        description: String::new(),
        documentation: None,
        input_schema: serde_json::json!({ "type": "object" }),
        output_schema: serde_json::json!({ "type": "object" }),
        version: "1.0.0".to_string(),
        tags: vec![],
        annotations: Some(ModuleAnnotations::default()),
        examples: vec![],
        metadata: std::collections::HashMap::new(),
        display: None,
        sunset_date: None,
        dependencies: vec![],
        enabled: true,
    }
}

fn dummy_identity() -> Identity {
    Identity::new(
        "@test".to_string(),
        "test".to_string(),
        vec![],
        HashMap::default(),
    )
}

// ---------------------------------------------------------------------------
// Empty-registry read tests
// ---------------------------------------------------------------------------

#[test]
fn test_registry_new_is_empty() {
    let registry = Registry::new();
    assert!(registry.list(None, None, None).is_empty());
}

#[test]
fn test_registry_default_is_empty() {
    let registry = Registry::default();
    assert!(registry.list(None, None, None).is_empty());
}

#[test]
fn test_registry_get_unknown_module_returns_none() {
    let registry = Registry::new();
    assert!(registry.get("nonexistent").unwrap().is_none());
}

#[test]
fn test_registry_contains_unknown_module_returns_false() {
    let registry = Registry::new();
    assert!(!registry.has("nonexistent"));
}

#[test]
fn test_registry_get_definition_unknown_returns_none() {
    let registry = Registry::new();
    assert!(registry.get_definition("nonexistent").unwrap().is_none());
}

#[test]
fn test_registry_get_definition_empty_id_errors_like_get() {
    // A-D-001: get_definition("") MUST propagate get()'s empty-id error
    // (ModuleNotFound), matching apcore-python / apcore-typescript.
    let registry = Registry::new();
    // `get()`'s Ok variant holds `Option<Arc<dyn Module>>` which is not Debug,
    // so map to the error code before comparing.
    let def_code = registry
        .get_definition("")
        .map(|_| ())
        .err()
        .map(|e| e.code);
    let get_code = registry.get("").map(|_| ()).err().map(|e| e.code);
    assert!(def_code.is_some(), "get_definition(\"\") must be Err");
    assert!(get_code.is_some(), "get(\"\") must be Err");
    assert_eq!(
        def_code, get_code,
        "get_definition empty-id error code must match get()"
    );
}

#[test]
fn test_registry_list_returns_vec_of_str() {
    let registry = Registry::new();
    let list: Vec<String> = registry.list(None, None, None);
    assert!(list.is_empty());
}

// ---------------------------------------------------------------------------
// export_schema tests (C-3)
// ---------------------------------------------------------------------------

#[test]
fn test_export_schema_returns_none_for_unregistered_module() {
    let registry = Registry::new();
    assert!(registry.export_schema("not.registered", false).is_none());
}

#[test]
fn test_export_schema_returns_schema_after_registration() {
    let registry = Registry::new();
    let descriptor = make_descriptor("math.add");
    registry
        .register_internal("math.add", Box::new(StubModule), descriptor)
        .expect("registration should succeed");

    let schema = registry.export_schema("math.add", false);
    assert!(
        schema.is_some(),
        "schema should be exported after registration"
    );
    let s = schema.unwrap();
    // Cross-SDK envelope keys (0.27): `input_schema` / `output_schema`, not
    // the old raw-cache `input` / `output`.
    assert_eq!(s.get("module_id"), Some(&serde_json::json!("math.add")));
    assert!(
        s.get("input_schema").is_some(),
        "envelope should have 'input_schema' key"
    );
    assert!(
        s.get("output_schema").is_some(),
        "envelope should have 'output_schema' key"
    );
}

// ---------------------------------------------------------------------------
// disable / enable / is_enabled tests (C-3)
// ---------------------------------------------------------------------------

#[test]
fn test_is_enabled_returns_none_for_unregistered_module() {
    let registry = Registry::new();
    assert!(registry.is_enabled("not.registered").is_none());
}

#[test]
fn test_disable_returns_error_for_unregistered_module() {
    let registry = Registry::new();
    let err = registry
        .disable("not.registered")
        .expect_err("should fail for unregistered module");
    assert!(
        err.message.contains("not found"),
        "error message should mention 'not found'"
    );
}

#[test]
fn test_enable_returns_error_for_unregistered_module() {
    let registry = Registry::new();
    let err = registry
        .enable("not.registered")
        .expect_err("should fail for unregistered module");
    assert!(err.message.contains("not found"));
}

#[test]
fn test_disable_sets_enabled_to_false() {
    let registry = Registry::new();
    registry
        .register_internal(
            "email.send",
            Box::new(StubModule),
            make_descriptor("email.send"),
        )
        .expect("registration should succeed");

    assert_eq!(registry.is_enabled("email.send"), Some(true));

    registry
        .disable("email.send")
        .expect("disable should succeed");
    assert_eq!(registry.is_enabled("email.send"), Some(false));
}

#[test]
fn test_enable_restores_enabled_to_true() {
    let registry = Registry::new();
    registry
        .register_internal("greet", Box::new(StubModule), make_descriptor("greet"))
        .expect("registration should succeed");

    registry.disable("greet").expect("disable should succeed");
    assert_eq!(registry.is_enabled("greet"), Some(false));

    registry.enable("greet").expect("enable should succeed");
    assert_eq!(registry.is_enabled("greet"), Some(true));
}

#[test]
fn test_module_enabled_by_default_after_registration() {
    let registry = Registry::new();
    registry
        .register_internal(
            "util.noop",
            Box::new(StubModule),
            make_descriptor("util.noop"),
        )
        .expect("registration should succeed");

    assert_eq!(
        registry.is_enabled("util.noop"),
        Some(true),
        "newly registered module should be enabled"
    );
}

// ---------------------------------------------------------------------------
// Reserved word validation tests
// ---------------------------------------------------------------------------

#[test]
fn test_register_rejects_reserved_first_segment() {
    let registry = Registry::new();
    let result = registry.register(
        "system.health",
        Box::new(StubModule),
        make_descriptor("system.health"),
    );
    assert!(result.is_err(), "registering 'system.health' should fail");
    let err = result.unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("reserved word"),
        "error should mention reserved word, got: {msg}"
    );
}

#[test]
fn test_register_allows_reserved_word_in_middle_segment() {
    // PROTOCOL_SPEC §2.7: reserved words are only checked against the first
    // segment. Middle/last segments may contain reserved words.
    // Aligned with apcore-python and apcore-typescript.
    let registry = Registry::new();
    let result = registry.register(
        "email.system",
        Box::new(StubModule),
        make_descriptor("email.system"),
    );
    assert!(
        result.is_ok(),
        "registering 'email.system' should succeed — 'system' is not the first segment"
    );
}

#[test]
fn test_register_allows_normal_module_id() {
    let registry = Registry::new();
    let result = registry.register(
        "email.send",
        Box::new(StubModule),
        make_descriptor("email.send"),
    );
    assert!(result.is_ok(), "registering 'email.send' should succeed");
}

#[test]
fn test_register_rejects_all_reserved_words() {
    use apcore::registry::RESERVED_WORDS;
    for word in RESERVED_WORDS {
        let registry = Registry::new();
        let module_id = format!("{word}.something");
        let result = registry.register(
            &module_id,
            Box::new(StubModule),
            make_descriptor(&module_id),
        );
        assert!(
            result.is_err(),
            "registering '{module_id}' should fail — '{word}' is reserved"
        );
    }
}

#[test]
fn test_register_module_rejects_reserved_first_segment() {
    let registry = Registry::new();
    let result = registry.register_module("core.utils", Box::new(StubModule));
    assert!(
        result.is_err(),
        "register_module with 'core.utils' should fail"
    );
}

// ---------------------------------------------------------------------------
// Module ID length boundary tests (PROTOCOL_SPEC §2.7 EBNF constraint #1)
// ---------------------------------------------------------------------------

#[test]
fn test_max_module_id_length_matches_spec() {
    // Per PROTOCOL_SPEC §2.7. Bumped from 128 to 192 in spec 1.6.0-draft.
    // Filesystem-safe: 192 + ".binding.yaml".len()=13 = 205 < 255-byte filename limit.
    use apcore::registry::MAX_MODULE_ID_LENGTH;
    assert_eq!(MAX_MODULE_ID_LENGTH, 192);
}

#[test]
fn test_register_accepts_module_id_at_max_length() {
    use apcore::registry::MAX_MODULE_ID_LENGTH;
    let registry = Registry::new();
    // Pure 'a' run satisfies the EBNF pattern [a-z][a-z0-9_]*.
    let exact_id = "a".repeat(MAX_MODULE_ID_LENGTH);
    let result = registry.register(&exact_id, Box::new(StubModule), make_descriptor(&exact_id));
    assert!(
        result.is_ok(),
        "registering an ID at exactly MAX_MODULE_ID_LENGTH should succeed"
    );
}

#[test]
fn test_register_rejects_module_id_exceeding_max_length() {
    use apcore::registry::MAX_MODULE_ID_LENGTH;
    let registry = Registry::new();
    let overlong_id = "a".repeat(MAX_MODULE_ID_LENGTH + 1);
    let result = registry.register(
        &overlong_id,
        Box::new(StubModule),
        make_descriptor(&overlong_id),
    );
    assert!(
        result.is_err(),
        "registering an ID longer than MAX_MODULE_ID_LENGTH should fail"
    );
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("maximum length"),
        "error should mention maximum length, got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// PROTOCOL_SPEC §2.7 EBNF compliance — empty / pattern checks
// (parity with apcore-python and apcore-typescript)
// ---------------------------------------------------------------------------

#[test]
fn test_register_rejects_empty_module_id() {
    let registry = Registry::new();
    let result = registry.register("", Box::new(StubModule), make_descriptor(""));
    assert!(result.is_err(), "registering empty ID must fail");
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("non-empty"),
        "error should mention non-empty, got: {msg}"
    );
}

#[test]
fn test_register_rejects_invalid_pattern() {
    let registry = Registry::new();
    for bad_id in [
        "INVALID-ID", // hyphens not allowed
        "1abc",       // starts with digit
        "Module",     // uppercase
        "a..b",       // consecutive dots
        ".leading",   // leading dot
        "trailing.",  // trailing dot
        "has space",  // space
        "has!bang",   // special char
    ] {
        let result = registry.register(bad_id, Box::new(StubModule), make_descriptor(bad_id));
        assert!(
            result.is_err(),
            "registering pattern-invalid ID '{bad_id}' must fail"
        );
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("Invalid module ID") || msg.contains("Must match pattern"),
            "error for '{bad_id}' should mention pattern, got: {msg}"
        );
    }
}

// ---------------------------------------------------------------------------
// register_internal — bypasses ONLY reserved word check
// (parity with apcore-python and apcore-typescript)
// ---------------------------------------------------------------------------

#[test]
fn test_register_internal_accepts_reserved_first_segment() {
    let registry = Registry::new();
    let result = registry.register_internal(
        "system.health",
        Box::new(StubModule),
        make_descriptor("system.health"),
    );
    assert!(
        result.is_ok(),
        "register_internal must accept reserved first segment 'system'"
    );
}

#[test]
fn test_register_internal_accepts_reserved_any_segment() {
    let registry = Registry::new();
    let result = registry.register_internal(
        "myapp.system.config",
        Box::new(StubModule),
        make_descriptor("myapp.system.config"),
    );
    assert!(
        result.is_ok(),
        "register_internal must accept reserved word in any segment"
    );
}

#[test]
fn test_register_internal_still_rejects_empty() {
    let registry = Registry::new();
    let result = registry.register_internal("", Box::new(StubModule), make_descriptor(""));
    assert!(
        result.is_err(),
        "register_internal must still reject empty IDs"
    );
}

#[test]
fn test_register_internal_still_rejects_invalid_pattern() {
    let registry = Registry::new();
    let result = registry.register_internal(
        "INVALID-ID",
        Box::new(StubModule),
        make_descriptor("INVALID-ID"),
    );
    assert!(
        result.is_err(),
        "register_internal must still enforce EBNF pattern"
    );
}

#[test]
fn test_register_internal_still_rejects_over_length() {
    use apcore::registry::MAX_MODULE_ID_LENGTH;
    let registry = Registry::new();
    let overlong = "a".repeat(MAX_MODULE_ID_LENGTH + 1);
    let result =
        registry.register_internal(&overlong, Box::new(StubModule), make_descriptor(&overlong));
    assert!(
        result.is_err(),
        "register_internal must still enforce length limit"
    );
}

#[test]
fn test_register_internal_rejects_duplicate() {
    let registry = Registry::new();
    registry
        .register_internal(
            "system.dup",
            Box::new(StubModule),
            make_descriptor("system.dup"),
        )
        .expect("first register_internal should succeed");
    let result = registry.register_internal(
        "system.dup",
        Box::new(StubModule),
        make_descriptor("system.dup"),
    );
    assert!(
        result.is_err(),
        "register_internal must reject duplicate IDs"
    );
}

// Suppress unused-import warning — dummy_identity is available for future async tests.
#[allow(dead_code)]
fn _use_identity() -> Identity {
    dummy_identity()
}

#[test]
fn test_on_returns_unique_handles() {
    let registry = Registry::new();

    let h1 = registry
        .on(
            "register",
            Box::new(|_: &str, _: &dyn apcore::module::Module| {}),
        )
        .expect("register is a valid event");
    let h2 = registry
        .on(
            "register",
            Box::new(|_: &str, _: &dyn apcore::module::Module| {}),
        )
        .expect("register is a valid event");

    assert_ne!(h1, h2, "each on() call must return a distinct handle");
}

#[test]
fn test_on_rejects_event_names_outside_the_closed_set() {
    // D-80: the event set is closed. Accepting a typo and returning a
    // valid-looking handle made the subscription permanently silent.
    let registry = Registry::new();

    for bad in ["change", "add", "remove", "Register", ""] {
        let err = registry
            .on(bad, Box::new(|_: &str, _: &dyn apcore::module::Module| {}))
            .expect_err("unknown event must be rejected");
        assert_eq!(
            err.code,
            apcore::errors::ErrorCode::GeneralInvalidInput,
            "event {bad:?}"
        );
    }

    // apcore-rust's watch() re-runs discovery and emits unregister/register,
    // so per requirement 3 it must NOT emit — nor advertise — file_changed.
    assert!(registry
        .on(
            "file_changed",
            Box::new(|_: &str, _: &dyn apcore::module::Module| {})
        )
        .is_err());

    // The whole advertised set is accepted (requirement 2).
    for good in apcore::registry::registry_events::ALL {
        assert!(registry
            .on(good, Box::new(|_: &str, _: &dyn apcore::module::Module| {}))
            .is_ok());
    }
}

#[test]
fn test_off_removes_callback_by_handle() {
    use std::sync::{Arc, Mutex};
    let registry = Registry::new();
    let counter = Arc::new(Mutex::new(0u32));

    let c = counter.clone();
    let handle = registry
        .on(
            "register",
            Box::new(move |_: &str, _: &dyn apcore::module::Module| {
                *c.lock().unwrap() += 1;
            }),
        )
        .expect("register is a valid event");

    // Register a module to trigger the callback once
    registry
        .register_module("math.add", Box::new(StubModule))
        .unwrap();
    assert_eq!(*counter.lock().unwrap(), 1, "callback should fire once");

    // Remove the callback
    let removed = registry.off(handle);
    assert!(removed, "off() should return true when callback exists");

    // Register another module — callback should NOT fire again
    registry
        .register_module("math.sub", Box::new(StubModule))
        .unwrap();
    assert_eq!(
        *counter.lock().unwrap(),
        1,
        "callback should not fire after off()"
    );
}

#[test]
fn test_off_returns_false_for_unknown_handle() {
    let registry = Registry::new();
    let removed = registry.off(99999);
    assert!(!removed, "off() with unknown handle should return false");
}

// ---------------------------------------------------------------------------
// Discoverer — cross-language parity tests
// ---------------------------------------------------------------------------

mod discoverer_tests {
    use super::*;
    use apcore::module::ValidationResult;
    use apcore::registry::registry::{DiscoveredModule, Discoverer, ModuleValidator};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };

    struct OnLoadCountingModule {
        counter: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Module for OnLoadCountingModule {
        fn description(&self) -> &'static str {
            "on_load counter"
        }
        fn input_schema(&self) -> Value {
            serde_json::json!({ "type": "object" })
        }
        fn output_schema(&self) -> Value {
            serde_json::json!({ "type": "object" })
        }
        fn on_load(&self) -> Result<(), ModuleError> {
            self.counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn execute(
            &self,
            _inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Value, ModuleError> {
            Ok(serde_json::json!({}))
        }
    }

    struct FixedDiscoverer {
        entries: Mutex<Option<Vec<DiscoveredModule>>>,
    }

    impl FixedDiscoverer {
        fn new(entries: Vec<DiscoveredModule>) -> Self {
            Self {
                entries: Mutex::new(Some(entries)),
            }
        }
    }

    #[async_trait]
    impl Discoverer for FixedDiscoverer {
        async fn discover(&self, _roots: &[String]) -> Result<Vec<DiscoveredModule>, ModuleError> {
            Ok(self.entries.lock().unwrap().take().unwrap_or_default())
        }
    }

    struct RejectAllValidator {
        called: Arc<AtomicUsize>,
    }

    impl ModuleValidator for RejectAllValidator {
        fn validate(
            &self,
            _module: &dyn Module,
            _descriptor: Option<&ModuleDescriptor>,
        ) -> ValidationResult {
            self.called.fetch_add(1, Ordering::SeqCst);
            let mut r = ValidationResult::default();
            r.valid = false;
            r.errors = vec![apcore::ValidationErrorDetail::message_only(
                "rejected by test validator",
            )];
            r
        }
    }

    fn dm(name: &str, module: Arc<dyn Module>) -> DiscoveredModule {
        DiscoveredModule {
            name: name.to_string(),
            source: "test".to_string(),
            descriptor: make_descriptor(name),
            module,
        }
    }

    fn stub() -> Arc<dyn Module> {
        Arc::new(StubModule)
    }

    #[tokio::test]
    async fn registers_instance_and_fires_on_load() {
        let counter = Arc::new(AtomicUsize::new(0));
        let module: Arc<dyn Module> = Arc::new(OnLoadCountingModule {
            counter: Arc::clone(&counter),
        });
        let registry = Registry::new();
        let discoverer = FixedDiscoverer::new(vec![dm("math.add", module)]);

        let count = registry.discover(&discoverer).await.unwrap();

        assert_eq!(count, 1);
        assert!(registry.has("math.add"));
        assert!(registry.get_definition("math.add").unwrap().is_some());
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "on_load called exactly once"
        );
    }

    #[tokio::test]
    async fn invalid_module_id_is_skipped_and_does_not_abort_batch() {
        let registry = Registry::new();
        let discoverer = FixedDiscoverer::new(vec![
            dm("Invalid-ID", stub()),    // uppercase + hyphen — EBNF fail
            dm("", stub()),              // empty
            dm("system.hacker", stub()), // reserved first segment
            dm("good.one", stub()),
        ]);

        let count = registry.discover(&discoverer).await.unwrap();

        assert_eq!(count, 1, "only the single valid entry should register");
        assert!(registry.has("good.one"));
        assert!(registry.get_definition("Invalid-ID").unwrap().is_none());
        // A-D-001: empty id propagates get()'s ModuleNotFound error.
        assert!(registry.get_definition("").is_err());
        assert!(registry.get_definition("system.hacker").unwrap().is_none());
    }

    #[tokio::test]
    async fn duplicate_entry_within_batch_is_skipped() {
        let registry = Registry::new();
        let discoverer = FixedDiscoverer::new(vec![
            dm("math.add", stub()),
            dm("math.add", stub()), // duplicate
        ]);

        let count = registry.discover(&discoverer).await.unwrap();

        assert_eq!(count, 1, "second duplicate should be skipped");
        assert!(registry.has("math.add"));
    }

    #[tokio::test]
    async fn custom_validator_rejects_entry() {
        let called = Arc::new(AtomicUsize::new(0));
        let registry = Registry::new();
        registry.set_validator(Box::new(RejectAllValidator {
            called: Arc::clone(&called),
        }));
        let discoverer = FixedDiscoverer::new(vec![dm("math.add", stub())]);

        let count = registry.discover(&discoverer).await.unwrap();

        assert_eq!(count, 0);
        assert!(!registry.has("math.add"));
        assert!(registry.get_definition("math.add").unwrap().is_none());
        assert_eq!(called.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn register_callback_fires_once_per_entry() {
        let callback_count = Arc::new(std::sync::Mutex::new(0usize));
        let cc = Arc::clone(&callback_count);
        let registry = Registry::new();
        registry
            .on(
                "register",
                Box::new(move |_: &str, _: &dyn Module| {
                    *cc.lock().unwrap() += 1;
                }),
            )
            .expect("valid registry event");

        let discoverer =
            FixedDiscoverer::new(vec![dm("math.add", stub()), dm("math.subtract", stub())]);
        let count = registry.discover(&discoverer).await.unwrap();

        assert_eq!(count, 2);
        assert_eq!(
            *callback_count.lock().unwrap(),
            2,
            "register callback fires once per registered entry"
        );
    }

    #[tokio::test]
    async fn discover_internal_without_discoverer_returns_error() {
        let registry = Registry::new();
        let err = registry.discover_internal().await.unwrap_err();
        assert_eq!(
            err.code,
            apcore::errors::ErrorCode::NoDiscovererConfigured,
            "discover_internal must return the dedicated NoDiscovererConfigured \
             error so real load failures surfaced as ModuleLoadError are not \
             masked by APCore::discover's swallow policy"
        );
    }

    #[tokio::test]
    async fn discovered_module_with_invalid_descriptor_schema_is_skipped() {
        // Regression: register_discovered must validate descriptor schema shapes
        // before inserting into schema_cache. A discoverer returning non-object
        // schemas (e.g., a string) must be rejected, not silently cached.
        fn dm_with_bad_schema(name: &str, module: Arc<dyn Module>) -> DiscoveredModule {
            let mut desc = make_descriptor(name);
            desc.input_schema = serde_json::json!("not-an-object"); // invalid schema shape
            DiscoveredModule {
                name: name.to_string(),
                source: "test".to_string(),
                descriptor: desc,
                module,
            }
        }

        let registry = Registry::new();
        let discoverer = FixedDiscoverer::new(vec![
            dm_with_bad_schema("bad.schema", stub()),
            dm("good.one", stub()), // valid entry in the same batch
        ]);

        let count = registry.discover(&discoverer).await.unwrap();

        assert_eq!(count, 1, "only the valid module should register");
        assert!(
            !registry.has("bad.schema"),
            "module with non-object schema must be rejected"
        );
        assert!(registry.has("good.one"), "valid module must still register");
    }

    #[tokio::test]
    async fn discoverer_is_restored_even_when_discover_panics() {
        // Regression: previously, if a custom Discoverer's discover().await
        // panicked, the RAII-less restore block was unreachable and the
        // discoverer was permanently lost.
        struct PanickingDiscoverer;
        #[async_trait]
        impl Discoverer for PanickingDiscoverer {
            async fn discover(
                &self,
                _roots: &[String],
            ) -> Result<Vec<DiscoveredModule>, ModuleError> {
                panic!("simulated discoverer failure");
            }
        }

        let registry = Arc::new(Registry::new());
        registry.set_discoverer(Box::new(PanickingDiscoverer));

        // First call panics; catch_unwind isolates the panic from the test harness.
        let r = Arc::clone(&registry);
        let first = tokio::spawn(async move { r.discover_internal().await }).await;
        assert!(first.is_err(), "panicking discoverer must propagate panic");

        // The Drop guard should have restored the discoverer; a second call
        // should find it still present (and panic again, not return
        // NoDiscovererConfigured).
        let r2 = Arc::clone(&registry);
        let second = tokio::spawn(async move { r2.discover_internal().await }).await;
        assert!(
            second.is_err(),
            "discoverer must still be present after first panic — it should panic again, \
             not disappear into 'NoDiscovererConfigured'"
        );
    }
}

// ---------------------------------------------------------------------------
// Lifecycle + conflict-detection regression tests
// ---------------------------------------------------------------------------

mod lifecycle_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Module that records the sequence of `on_load` / `on_unload` calls and
    /// tracks whether `execute` has ever been called — used to verify that
    /// `on_unload` never runs before the module is removed from the registry's
    /// live map (otherwise a concurrent `call()` could dispatch to an
    /// already-torn-down module).
    struct LifecycleModule {
        load_count: Arc<AtomicUsize>,
        unload_count: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Module for LifecycleModule {
        fn description(&self) -> &'static str {
            "lifecycle"
        }
        fn input_schema(&self) -> Value {
            serde_json::json!({ "type": "object" })
        }
        fn output_schema(&self) -> Value {
            serde_json::json!({ "type": "object" })
        }
        async fn execute(
            &self,
            _inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Value, ModuleError> {
            Ok(serde_json::json!({}))
        }
        fn on_load(&self) -> Result<(), ModuleError> {
            self.load_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn on_unload(&self) {
            self.unload_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn register_rejects_exact_duplicate() {
        let registry = Registry::new();
        registry
            .register("foo.bar", Box::new(StubModule), make_descriptor("foo.bar"))
            .expect("first registration succeeds");

        let err = registry
            .register("foo.bar", Box::new(StubModule), make_descriptor("foo.bar"))
            .unwrap_err();
        // apcore #65: duplicate module IDs now return DuplicateModuleId.
        assert_eq!(err.code, apcore::errors::ErrorCode::DuplicateModuleId);
        assert!(err.message.contains("already registered"));
    }

    #[test]
    fn on_load_is_skipped_when_registration_is_rejected_as_duplicate() {
        // Regression: previously on_load ran BEFORE the duplicate check, so
        // a rejected duplicate would leak resources opened by on_load.
        let registry = Registry::new();
        let load_count = Arc::new(AtomicUsize::new(0));
        let unload_count = Arc::new(AtomicUsize::new(0));

        registry
            .register(
                "foo.bar",
                Box::new(LifecycleModule {
                    load_count: Arc::clone(&load_count),
                    unload_count: Arc::clone(&unload_count),
                }),
                make_descriptor("foo.bar"),
            )
            .unwrap();
        assert_eq!(load_count.load(Ordering::SeqCst), 1);

        // Second register for same ID: rejected. A *new* LifecycleModule
        // instance's on_load must NOT run because the ID is a duplicate.
        let rejected_load_count = Arc::new(AtomicUsize::new(0));
        let rejected_unload_count = Arc::new(AtomicUsize::new(0));
        let err = registry.register(
            "foo.bar",
            Box::new(LifecycleModule {
                load_count: Arc::clone(&rejected_load_count),
                unload_count: Arc::clone(&rejected_unload_count),
            }),
            make_descriptor("foo.bar"),
        );
        assert!(err.is_err());
        assert_eq!(
            rejected_load_count.load(Ordering::SeqCst),
            0,
            "on_load MUST NOT fire for a registration rejected due to duplicate ID"
        );
    }

    #[test]
    fn unregister_removes_module_before_calling_on_unload() {
        // Regression: previously on_unload ran BEFORE the core-map removal,
        // so a concurrent `get()` could still dispatch to a module whose
        // resources had already been freed.
        let registry = Arc::new(Registry::new());
        let load_count = Arc::new(AtomicUsize::new(0));
        let unload_count = Arc::new(AtomicUsize::new(0));

        registry
            .register(
                "foo.bar",
                Box::new(LifecycleModule {
                    load_count: Arc::clone(&load_count),
                    unload_count: Arc::clone(&unload_count),
                }),
                make_descriptor("foo.bar"),
            )
            .unwrap();

        // Install a callback that runs DURING unregister (after remove,
        // before on_unload) and observes the registry state — the module
        // must already be gone from `get()` at this point.
        let present_at_callback = Arc::new(AtomicUsize::new(0));
        let pac_clone = Arc::clone(&present_at_callback);
        let registry_weak = Arc::downgrade(&registry);
        registry
            .on(
                "unregister",
                Box::new(move |name, _module| {
                    if let Some(reg) = registry_weak.upgrade() {
                        if matches!(reg.get(name), Ok(Some(_))) {
                            pac_clone.store(1, Ordering::SeqCst);
                        }
                    }
                }),
            )
            .expect("valid registry event");

        registry.unregister("foo.bar").unwrap();

        assert_eq!(
            present_at_callback.load(Ordering::SeqCst),
            0,
            "by the time the 'unregister' callback fires, the module must \
             already be gone from the registry's live map"
        );
        assert_eq!(
            unload_count.load(Ordering::SeqCst),
            1,
            "on_unload runs exactly once, after removal"
        );
    }

    #[test]
    fn validator_is_invoked_without_registry_lock_held() {
        // Regression: validator was previously called while holding the
        // validator read guard, so a validator that re-registered itself
        // would deadlock (parking_lot guards are non-reentrant). With the
        // Arc-snapshot fix the validator sees no lock held.
        use apcore::module::ValidationResult;
        use apcore::registry::registry::ModuleValidator;

        struct ReentrantValidator {
            registry: Arc<Registry>,
        }
        impl ModuleValidator for ReentrantValidator {
            fn validate(
                &self,
                _module: &dyn Module,
                _descriptor: Option<&ModuleDescriptor>,
            ) -> ValidationResult {
                // Re-entering the registry during validation must NOT deadlock.
                // We replace the validator — this takes the validator write lock.
                self.registry.set_validator(Box::new(PermissiveValidator));
                let mut r = ValidationResult::default();
                r.valid = true;
                r
            }
        }

        struct PermissiveValidator;
        impl ModuleValidator for PermissiveValidator {
            fn validate(
                &self,
                _module: &dyn Module,
                _descriptor: Option<&ModuleDescriptor>,
            ) -> ValidationResult {
                let mut r = ValidationResult::default();
                r.valid = true;
                r
            }
        }

        let registry = Arc::new(Registry::new());
        registry.set_validator(Box::new(ReentrantValidator {
            registry: Arc::clone(&registry),
        }));

        registry
            .register("foo.bar", Box::new(StubModule), make_descriptor("foo.bar"))
            .expect("validator that re-enters set_validator must not deadlock");
    }
}

mod on_load_rollback_tests {
    use super::*;
    use apcore::errors::ErrorCode;

    struct FailingOnLoadModule;

    #[async_trait]
    impl Module for FailingOnLoadModule {
        fn description(&self) -> &'static str {
            "fails on_load"
        }
        fn input_schema(&self) -> Value {
            serde_json::json!({ "type": "object" })
        }
        fn output_schema(&self) -> Value {
            serde_json::json!({ "type": "object" })
        }
        fn on_load(&self) -> Result<(), ModuleError> {
            Err(ModuleError::new(
                ErrorCode::ModuleLoadError,
                "simulated on_load failure".to_string(),
            ))
        }
        async fn execute(
            &self,
            _inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Value, ModuleError> {
            Ok(serde_json::json!({}))
        }
    }

    #[test]
    fn register_rolls_back_when_on_load_returns_err() {
        let registry = Registry::new();
        let err = registry
            .register(
                "foo.bar",
                Box::new(FailingOnLoadModule),
                make_descriptor("foo.bar"),
            )
            .unwrap_err();

        assert_eq!(
            err.code,
            ErrorCode::ModuleLoadError,
            "register must propagate on_load error"
        );
        assert!(
            err.message.contains("on_load"),
            "error message: {}",
            err.message
        );
        assert!(
            registry.get("foo.bar").unwrap().is_none(),
            "module must not remain in registry after on_load failure"
        );
        assert_eq!(
            registry.list(None, None, None).len(),
            0,
            "registry must be empty after failed registration"
        );
    }

    #[test]
    fn register_succeeding_module_after_failed_on_load_works() {
        let registry = Registry::new();

        // First registration fails due to on_load
        let _ = registry.register(
            "foo.bad",
            Box::new(FailingOnLoadModule),
            make_descriptor("foo.bad"),
        );

        // Registry must still accept a valid module with the same ID slot
        registry
            .register("foo.bad", Box::new(StubModule), make_descriptor("foo.bad"))
            .expect(
                "registry must accept registration after a prior failed on_load for the same id",
            );

        assert!(registry.get("foo.bad").unwrap().is_some());
    }
}

// ---------------------------------------------------------------------------
// A-D-005: DefaultDiscoverer wires the 8-stage pipeline through Registry
// ---------------------------------------------------------------------------

#[tokio::test]
async fn default_discoverer_via_registry_raises_config_not_found_on_missing_root() {
    use std::sync::Arc;
    let registry = Arc::new(Registry::new());
    registry.set_extension_roots(vec!["/this/does/not/exist".to_string()]);
    registry.set_discoverer(Box::new(apcore::DefaultDiscoverer::new()));

    let err = registry
        .discover_internal()
        .await
        .expect_err("missing root should error");
    assert_eq!(err.code, apcore::errors::ErrorCode::ConfigNotFound);
}

#[tokio::test]
async fn default_discoverer_via_registry_registers_factory_modules() {
    use std::sync::Arc;
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("greet.rs"), "// stub").unwrap();

    let factory: apcore::ModuleFactory =
        Arc::new(|_file, _entry_point| Ok(Some(Arc::new(StubModule) as Arc<dyn Module>)));

    let registry = Arc::new(Registry::new());
    registry.set_extension_roots(vec![tmp.path().to_string_lossy().into_owned()]);
    registry.set_discoverer(Box::new(
        apcore::DefaultDiscoverer::new().with_factory(factory),
    ));

    let count = registry.discover_internal().await.unwrap();
    assert_eq!(count, 1, "exactly one module discovered");
    assert!(registry.get("greet").unwrap().is_some());
}

// ---------------------------------------------------------------------------
// A-D-009: acquire() bumps ref_counts for safe_unregister drain
// ---------------------------------------------------------------------------

#[tokio::test]
async fn acquire_bumps_ref_count_and_release_decrements() {
    let registry = Registry::new();
    registry
        .register(
            "drain.test",
            Box::new(StubModule),
            make_descriptor("drain.test"),
        )
        .unwrap();

    // First acquire — ref count goes to 1
    let _m1 = registry.acquire("drain.test").unwrap();
    // Second acquire — ref count goes to 2
    let _m2 = registry.acquire("drain.test").unwrap();

    // Releases bring it back to 0
    registry.release("drain.test");
    registry.release("drain.test");

    // Releasing past zero is a no-op (idempotent, like Python/TS)
    registry.release("drain.test");
    registry.release("never.acquired");
}

// ---------------------------------------------------------------------------
// A-D-010: Registry::watch() triggers debounced re-discover on file changes
// ---------------------------------------------------------------------------

struct CountingDiscoverer {
    counter: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait]
impl apcore::registry::registry::Discoverer for CountingDiscoverer {
    async fn discover(
        &self,
        _roots: &[String],
    ) -> Result<Vec<apcore::registry::registry::DiscoveredModule>, ModuleError> {
        self.counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(Vec::new())
    }
}

#[tokio::test]
async fn watch_with_no_extension_roots_is_noop() {
    use std::sync::Arc;
    let registry = Arc::new(Registry::new());
    // No extension roots set — watch() should return Ok without spawning
    registry.watch().await.unwrap();
    registry.unwatch();
}

#[tokio::test]
async fn watch_re_runs_discover_on_file_change() {
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::time::Duration;

    let tmp = tempfile::tempdir().unwrap();
    let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let registry = Arc::new(Registry::new());
    registry.set_extension_roots(vec![tmp.path().to_string_lossy().into_owned()]);
    registry.set_discoverer(Box::new(CountingDiscoverer {
        counter: Arc::clone(&counter),
    }));

    registry.watch().await.unwrap();

    // Touch a file to trigger an event
    tokio::time::sleep(Duration::from_millis(50)).await;
    std::fs::write(tmp.path().join("foo.txt"), "hello").unwrap();

    // Wait past the 300ms debounce + filesystem event propagation
    tokio::time::sleep(Duration::from_millis(800)).await;

    let count = counter.load(Ordering::SeqCst);
    assert!(
        count >= 1,
        "discover_internal should have fired at least once, got {count}"
    );

    registry.unwatch();
}

#[tokio::test]
async fn safe_unregister_waits_for_acquire_drain() {
    use std::sync::Arc;
    use std::time::Duration;

    let registry = Arc::new(Registry::new());
    registry
        .register(
            "drain.wait",
            Box::new(StubModule),
            make_descriptor("drain.wait"),
        )
        .unwrap();

    let m = registry.acquire("drain.wait").unwrap();
    assert_eq!(m.description(), "stub");

    // safe_unregister should not complete until release() is called.
    let registry_clone = Arc::clone(&registry);
    let unregister_task =
        tokio::spawn(async move { registry_clone.safe_unregister("drain.wait", 5000).await });

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !unregister_task.is_finished(),
        "safe_unregister must wait for ref count to drain"
    );

    registry.release("drain.wait");

    let result = tokio::time::timeout(Duration::from_secs(2), unregister_task)
        .await
        .expect("safe_unregister must complete after release()")
        .expect("join")
        .expect("safe_unregister Result");
    assert!(
        result,
        "safe_unregister returned Ok(true) after clean drain"
    );
}

// ---------------------------------------------------------------------------
// Sorted list contract (sync A-D-103)
// ---------------------------------------------------------------------------

#[test]
fn test_list_returns_sorted_unique_ids() {
    // Spec: Registry.list() must return sorted, unique IDs for cross-language
    // parity with apcore-python and apcore-typescript (sync A-D-103).
    let registry = Registry::new();
    let names = ["zeta.module", "alpha.module", "mike.module", "beta.module"];
    for n in names {
        let descriptor = make_descriptor(n);
        registry
            .register_internal(n, Box::new(StubModule), descriptor)
            .expect("registration should succeed");
    }

    let listed = registry.list(None, None, None);
    let mut expected: Vec<String> = names.iter().map(|s| (*s).to_string()).collect();
    expected.sort();
    assert_eq!(
        listed, expected,
        "Registry::list() must return module IDs in sorted order"
    );
}

#[test]
fn test_list_with_prefix_returns_sorted() {
    let registry = Registry::new();
    let names = ["math.zeta", "math.alpha", "other.gamma", "math.beta"];
    for n in names {
        let descriptor = make_descriptor(n);
        registry
            .register_internal(n, Box::new(StubModule), descriptor)
            .expect("registration should succeed");
    }

    let listed = registry.list(None, Some("math."), None);
    let expected: Vec<String> = vec![
        "math.alpha".to_string(),
        "math.beta".to_string(),
        "math.zeta".to_string(),
    ];
    assert_eq!(
        listed, expected,
        "Registry::list(prefix) must return sorted IDs"
    );
}

// ---------------------------------------------------------------------------
// D10-010: Registry::register_versioned — 4-arg canonical form matching
// apcore-python `register(module_id, module, version?, metadata?)` and
// apcore-typescript `register(moduleId, module, version?, metadata?)`.
// ---------------------------------------------------------------------------

#[test]
fn test_register_versioned_with_version_and_metadata() {
    let registry = apcore::registry::registry::Registry::new();
    let mut metadata = std::collections::HashMap::new();
    metadata.insert(
        "x-team".to_string(),
        serde_json::Value::String("platform".to_string()),
    );
    registry
        .register_versioned(
            "versioned.module",
            Box::new(StubModule),
            Some("2.5.0"),
            Some(metadata),
        )
        .expect("register_versioned should succeed");

    let definition = registry
        .get_definition("versioned.module")
        .expect("get_definition must not error for a valid id")
        .expect("registered module has a descriptor");
    assert_eq!(definition.version, "2.5.0", "version flows into descriptor");
    assert_eq!(
        definition.metadata.get("x-team"),
        Some(&serde_json::Value::String("platform".to_string())),
        "metadata flows into descriptor"
    );
}

#[test]
fn test_register_versioned_with_none_version_falls_back_to_default() {
    let registry = apcore::registry::registry::Registry::new();
    registry
        .register_versioned("default.version", Box::new(StubModule), None, None)
        .expect("register_versioned should succeed with None args");

    let definition = registry
        .get_definition("default.version")
        .expect("get_definition must not error for a valid id")
        .expect("registered module has a descriptor");
    // DEFAULT_MODULE_VERSION matches what register_module would set.
    assert!(
        !definition.version.is_empty(),
        "default version is non-empty"
    );
    assert!(
        definition.metadata.is_empty(),
        "None metadata yields empty map"
    );
}

// ---------------------------------------------------------------------------
// D11-003: Registry::list tag filter unions descriptor.tags + module.tags()
// ---------------------------------------------------------------------------
//
// Python (registry.py:1027) and TypeScript (registry.ts:689) build the
// filter set by unioning module-instance `tags` AND merged-meta tags.
// Rust previously only inspected `descriptor.tags`, so a module ported
// from Python/TS that exposes its tags via `fn tags(&self)` but is
// registered with empty descriptor.tags was filtered OUT by Rust while
// still appearing in Python/TS list(tags=...) results.

struct TaggedModule;

#[async_trait::async_trait]
impl apcore::module::Module for TaggedModule {
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    fn output_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "Module that publishes tags via the trait method"
    }
    async fn execute(
        &self,
        _inputs: serde_json::Value,
        _ctx: &apcore::context::Context<serde_json::Value>,
    ) -> Result<serde_json::Value, apcore::errors::ModuleError> {
        Ok(serde_json::json!({}))
    }
    fn tags(&self) -> Vec<String> {
        vec!["alpha".to_string(), "beta".to_string()]
    }
}

#[test]
fn test_list_tag_filter_unions_module_instance_tags() {
    let registry = apcore::registry::registry::Registry::new();
    // register_module builds an empty descriptor.tags — only the trait
    // method `tags()` advertises ["alpha", "beta"].
    registry
        .register_module("tagged.module", Box::new(TaggedModule))
        .expect("register_module should succeed");

    let with_alpha = registry.list(Some(&["alpha"]), None, None);
    assert_eq!(
        with_alpha,
        vec!["tagged.module".to_string()],
        "module-instance tags from `fn tags()` must participate in tag filter"
    );

    let with_alpha_and_beta = registry.list(Some(&["alpha", "beta"]), None, None);
    assert_eq!(
        with_alpha_and_beta,
        vec!["tagged.module".to_string()],
        "all required tags satisfied via module-instance tags"
    );

    let with_unknown = registry.list(Some(&["unknown"]), None, None);
    assert!(
        with_unknown.is_empty(),
        "tag not declared on module instance must not match"
    );
}

// ---------------------------------------------------------------------------
// A-D-017: register_module populates descriptor.tags from module.tags()
// (reuses the TaggedModule defined above, which advertises ["alpha","beta"]).
// ---------------------------------------------------------------------------

#[test]
fn test_register_module_populates_descriptor_tags_from_module() {
    let registry = Registry::new();
    registry
        .register_module("executor.test.tagged", Box::new(TaggedModule))
        .expect("registration must succeed");

    let def = registry
        .get_definition("executor.test.tagged")
        .expect("get_definition must succeed")
        .expect("module must be registered");
    assert_eq!(
        def.tags,
        vec!["alpha".to_string(), "beta".to_string()],
        "register_module must populate descriptor.tags from module.tags() (A-D-017)"
    );
}

// ---------------------------------------------------------------------------
// sync-2026-08-26 A-D-005: a discovery-path on_load failure must fire the same
// two signals as the manual register() path.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn discovery_on_load_failure_fires_callbacks_and_event() {
    use apcore::registry::registry::{DiscoveredModule, Discoverer};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    // `register_core`'s Err arm fires `load_failed_callbacks` AND
    // `emit_module_load_failed`. `register_discovered`'s Err arm had only a
    // tracing line and the in_flight release, so an on_load failure reached
    // through discovery notified nobody — while the identical failure through
    // `register()` notified both. registry-system.md names "discovery-driven
    // paths (discover(), register_discovered, hot-reload, etc.)" explicitly.

    struct FailingOnLoad;
    #[async_trait::async_trait]
    impl Module for FailingOnLoad {
        fn description(&self) -> &'static str {
            "fails on load"
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        fn output_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        async fn execute(
            &self,
            _i: serde_json::Value,
            _c: &Context<serde_json::Value>,
        ) -> Result<serde_json::Value, ModuleError> {
            Ok(serde_json::json!({}))
        }
        fn on_load(&self) -> Result<(), ModuleError> {
            Err(ModuleError::new(
                apcore::errors::ErrorCode::ModuleLoadError,
                "boom",
            ))
        }
    }

    struct OneModuleDiscoverer;
    #[async_trait::async_trait]
    impl Discoverer for OneModuleDiscoverer {
        async fn discover(&self, _roots: &[String]) -> Result<Vec<DiscoveredModule>, ModuleError> {
            Ok(vec![DiscoveredModule {
                name: "discovered.failing".to_string(),
                source: "test".to_string(),
                descriptor: ModuleDescriptor {
                    module_id: "discovered.failing".to_string(),
                    name: None,
                    description: "fails on load".to_string(),
                    documentation: None,
                    input_schema: serde_json::json!({}),
                    output_schema: serde_json::json!({}),
                    version: "1.0.0".to_string(),
                    tags: vec![],
                    annotations: None,
                    examples: vec![],
                    metadata: std::collections::HashMap::new(),
                    display: None,
                    sunset_date: None,
                    dependencies: vec![],
                    enabled: true,
                },
                module: Arc::new(FailingOnLoad),
            }])
        }
    }

    let registry = Arc::new(Registry::new());
    let hits = Arc::new(AtomicUsize::new(0));
    let seen_id = Arc::new(parking_lot::Mutex::new(String::new()));

    let h = Arc::clone(&hits);
    let sid = Arc::clone(&seen_id);
    registry.on_load_failed(Arc::new(move |module_id: &str, _e: &ModuleError| {
        h.fetch_add(1, Ordering::SeqCst);
        *sid.lock() = module_id.to_string();
    }));
    registry.set_discoverer(Box::new(OneModuleDiscoverer));

    let count = registry
        .discover_internal()
        .await
        .expect("discovery itself succeeds; the module's on_load is what fails");

    assert_eq!(
        count, 0,
        "a module whose on_load fails must not be registered"
    );
    assert!(
        !registry.has("discovered.failing"),
        "the failed module must not be visible"
    );
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "on_load_failed callbacks must fire on the discovery path, as they do on register()"
    );
    assert_eq!(*seen_id.lock(), "discovered.failing");
}
