//! Cross-language parity regressions — CRITICAL findings from `/apcore-skills:sync`.
//!
//! Each test pins a behavior where apcore-rust diverged from apcore-python and
//! apcore-typescript. The authority is the spec repo (`PROTOCOL_SPEC`,
//! `docs/features/execution-pipeline.md`).

#![allow(clippy::pedantic, clippy::all)]

use std::collections::HashMap;
use std::path::PathBuf;

use apcore::bindings::{typed_handler, BindingHandler, BindingLoader, TypedBindingHandler};
use apcore::config::{Config, MountSource};
use apcore::errors::ErrorCode;
use apcore::registry::registry::Registry;
use apcore::schema::RefResolver;
use serde_json::json;

fn write(dir: &tempfile::TempDir, name: &str, body: &str) -> PathBuf {
    let p = dir.path().join(name);
    std::fs::write(&p, body).unwrap();
    p
}

// ---------------------------------------------------------------------------
// C1 — `auto_schema: strict` must be able to fail
// ---------------------------------------------------------------------------

const STRICT_BINDING: &str = r#"
spec_version: "1.0"
bindings:
  - module_id: executor.demo.strict
    target: "demo:strict_fn"
    description: "strict binding"
    auto_schema: strict
"#;

fn noop_handler() -> BindingHandler {
    std::sync::Arc::new(|_input: serde_json::Value, _ctx: &_| {
        Box::pin(async move { Ok(json!({})) })
            as std::pin::Pin<Box<dyn std::future::Future<Output = _> + Send>>
    })
}

/// `register_into_with_handlers` supplies untyped handlers, so nothing can be
/// inferred and the strict promise can never be kept. apcore-python and
/// apcore-typescript both raise `BINDING_SCHEMA_INFERENCE_FAILED` here; Rust
/// previously registered the module against a permissive `{"type":"object"}`
/// pair without ever running the strict check.
#[test]
fn c1_auto_schema_strict_with_untyped_handlers_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(&dir, "strict.binding.yaml", STRICT_BINDING);
    let mut loader = BindingLoader::new();
    loader.load_from_yaml(&p).unwrap();

    let registry = Registry::new();
    let mut handlers: HashMap<String, BindingHandler> = HashMap::new();
    handlers.insert("demo:strict_fn".to_string(), noop_handler());

    let err = loader
        .register_into_with_handlers(&registry, handlers)
        .expect_err("auto_schema: strict must not pass vacuously on the untyped path");
    assert_eq!(err.code, ErrorCode::BindingSchemaInferenceFailed);
    assert!(
        err.message.contains("executor.demo.strict"),
        "message must name the binding: {}",
        err.message
    );
}

/// Same on the typed path when the handler carries no schemas: the permissive
/// fallback would make `assert_openai_strict_compatible` succeed vacuously.
#[test]
fn c1_auto_schema_strict_with_schemaless_typed_handler_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(&dir, "strict.binding.yaml", STRICT_BINDING);
    let mut loader = BindingLoader::new();
    loader.load_from_yaml(&p).unwrap();

    let registry = Registry::new();
    let mut handlers: HashMap<String, TypedBindingHandler> = HashMap::new();
    handlers.insert(
        "demo:strict_fn".to_string(),
        TypedBindingHandler {
            handler: noop_handler(),
            input_schema: None,
            output_schema: None,
        },
    );

    let err = loader
        .register_into_with_typed_handlers(&registry, handlers)
        .expect_err("auto_schema: strict must not pass vacuously without a typed schema");
    assert_eq!(err.code, ErrorCode::BindingSchemaInferenceFailed);
}

/// protocol-spec §5.12.8 requires the `{file_path}: ` message prefix
/// and a `file_path` details key. Rust hard-coded `file_path = None` at the
/// only call site, so both were dead code.
#[test]
fn c1_strict_incompatible_error_carries_binding_file_path() {
    #[derive(serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
    struct Ok0 {
        name: String,
    }

    let dir = tempfile::tempdir().unwrap();
    let p = write(&dir, "strict.binding.yaml", STRICT_BINDING);
    let expected_path = p.display().to_string();
    let mut loader = BindingLoader::new();
    loader.load_from_yaml(&p).unwrap();

    let registry = Registry::new();
    let mut typed = typed_handler::<Ok0, Ok0>(|i: Ok0| Ok(i));
    // Inject an OpenAI-strict-incompatible keyword so the assertion fires.
    typed.input_schema = Some(json!({
        "type": "object",
        "properties": { "tags": { "type": "array", "uniqueItems": true } }
    }));
    let mut handlers: HashMap<String, TypedBindingHandler> = HashMap::new();
    handlers.insert("demo:strict_fn".to_string(), typed);

    let err = loader
        .register_into_with_typed_handlers(&registry, handlers)
        .expect_err("uniqueItems is not OpenAI-strict compatible");
    assert_eq!(err.code, ErrorCode::BindingStrictSchemaIncompatible);
    assert!(
        err.message.starts_with(&format!("{expected_path}: ")),
        "message must carry the `{{file_path}}: ` prefix (§7.2): {}",
        err.message
    );
    assert_eq!(
        err.details.get("file_path"),
        Some(&json!(expected_path)),
        "details must carry file_path (§7.2)"
    );
}

/// A strict binding whose typed handler yields a compatible schema still
/// registers — the fix must not reject the happy path.
#[test]
fn c1_auto_schema_strict_with_compatible_typed_handler_registers() {
    #[derive(serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
    struct Payload {
        name: String,
    }

    let dir = tempfile::tempdir().unwrap();
    let p = write(&dir, "strict.binding.yaml", STRICT_BINDING);
    let mut loader = BindingLoader::new();
    loader.load_from_yaml(&p).unwrap();

    let registry = Registry::new();
    let mut typed = typed_handler::<Payload, Payload>(|i: Payload| Ok(i));
    typed.input_schema = Some(json!({
        "type": "object",
        "properties": { "name": { "type": "string" } }
    }));
    typed.output_schema = Some(json!({
        "type": "object",
        "properties": { "name": { "type": "string" } }
    }));
    let mut handlers: HashMap<String, TypedBindingHandler> = HashMap::new();
    handlers.insert("demo:strict_fn".to_string(), typed);

    let count = loader
        .register_into_with_typed_handlers(&registry, handlers)
        .expect("compatible strict binding must register");
    assert_eq!(count, 1);
}

// ---------------------------------------------------------------------------
// C2 — `has_circular_refs` must agree with `resolve`
// ---------------------------------------------------------------------------

/// PROTOCOL_SPEC §4.15's own legal example. `resolve` returns `Ok` with the
/// `$ref` preserved lazily, so the predicate must answer `false`. It previously
/// answered `true`, which would make a caller gating registration on it reject
/// every recursive contract the spec mandates support for.
#[test]
fn c2_has_circular_refs_false_for_spec_legal_recursive_treenode() {
    let resolver = RefResolver::new();
    let schema = json!({
        "$id": "TreeNode",
        "type": "object",
        "properties": {
            "children": {
                "type": "array",
                "items": { "$ref": "#" }
            }
        }
    });
    assert!(
        resolver.resolve(&schema).is_ok(),
        "spec-legal recursive schema must resolve"
    );
    assert!(
        !resolver.has_circular_refs(&schema),
        "a self-reference reached by structural descent is not circular (§4.15)"
    );
}

/// The `$defs` flavour of the same shape.
#[test]
fn c2_has_circular_refs_agrees_with_resolve_for_defs_self_reference() {
    let resolver = RefResolver::new();
    let schema = json!({
        "$defs": {
            "node": {
                "type": "object",
                "properties": { "child": { "$ref": "#/$defs/node" } }
            }
        },
        "properties": { "root": { "$ref": "#/$defs/node" } }
    });
    assert!(resolver.resolve(&schema).is_ok());
    assert!(!resolver.has_circular_refs(&schema));
}

/// A genuine `$ref` → `$ref` chain still answers `true`, and agrees with the
/// error `resolve` raises.
#[test]
fn c2_has_circular_refs_true_for_ref_only_cycle() {
    let resolver = RefResolver::new();
    let schema = json!({
        "$ref": "#/$defs/a",
        "$defs": {
            "a": { "$ref": "#/$defs/b" },
            "b": { "$ref": "#/$defs/a" }
        }
    });
    let err = resolver.resolve(&schema).unwrap_err();
    assert_eq!(err.code, ErrorCode::SchemaCircularRef);
    assert!(resolver.has_circular_refs(&schema));
}

// ---------------------------------------------------------------------------
// C3 — the child Context must be derived in a non-removable step
// ---------------------------------------------------------------------------

mod c3 {
    use apcore::context::{Context, Identity};
    use apcore::errors::ModuleError;
    use apcore::module::{Module, ModuleAnnotations};
    use apcore::registry::registry::Registry;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    /// Records the `call_chain` observed inside the module body.
    #[derive(Debug)]
    struct ChainProbe {
        seen: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl Module for ChainProbe {
        fn input_schema(&self) -> serde_json::Value {
            json!({"type": "object"})
        }
        fn output_schema(&self) -> serde_json::Value {
            json!({"type": "object"})
        }
        fn description(&self) -> &str {
            "records call_chain"
        }
        fn annotations(&self) -> ModuleAnnotations {
            ModuleAnnotations::default()
        }
        async fn execute(
            &self,
            _inputs: serde_json::Value,
            ctx: &Context<serde_json::Value>,
        ) -> Result<serde_json::Value, ModuleError> {
            *self.seen.lock().unwrap() = ctx.call_chain.clone();
            Ok(json!({}))
        }
    }

    async fn chain_under_preset(preset: &str) -> Vec<String> {
        let registry = Registry::new();
        let seen = Arc::new(Mutex::new(Vec::new()));
        registry
            .register_module(
                "executor.probe.chain",
                Box::new(ChainProbe {
                    seen: Arc::clone(&seen),
                }),
            )
            .unwrap();

        let executor = apcore::executor::Executor::with_strategy_name(
            Arc::new(registry),
            Arc::new(apcore::config::Config::from_defaults()),
            preset,
        )
        .unwrap();

        let identity = Identity::new("u1".into(), "user".into(), vec![], HashMap::new());
        let ctx: Context<serde_json::Value> = Context::new(identity);
        executor
            .call("executor.probe.chain", json!({}), Some(&ctx), None)
            .await
            .unwrap();
        let out = seen.lock().unwrap().clone();
        out
    }

    /// `call_chain_guard` is removable and IS removed by the `testing` and
    /// `minimal` presets. Deriving the child context there meant `call_chain`
    /// never grew under those presets, resetting depth limits, circular-call
    /// detection and frequency throttling. apcore-python and apcore-typescript
    /// derive it in the non-removable `context_creation` step.
    #[tokio::test]
    async fn c3_call_chain_grows_under_every_preset() {
        for preset in ["standard", "internal", "testing", "performance", "minimal"] {
            let chain = chain_under_preset(preset).await;
            assert_eq!(
                chain,
                vec!["executor.probe.chain".to_string()],
                "preset '{preset}' must derive the child context (call_chain must contain the target)"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// C4 — canonical config defaults
// ---------------------------------------------------------------------------

/// A config that omits a key must still resolve to the canonical default
/// declared in `apcore/schemas/defaults.schema.json`, as apcore-python
/// (`_DEFAULTS`) and apcore-typescript (`DEFAULTS`) do. Rust previously
/// returned `None` for all of these.
///
/// Namespace mode is used because Rust (unlike its peers) still enforces the
/// legacy-mode required-field check against *declared* values — see
/// `Config::get_declared`.
#[test]
fn c4_omitted_keys_resolve_to_canonical_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(&dir, "minimal.yaml", "apcore:\n  modules_path: ./modules\n");
    let config = Config::load(&p).expect("minimal namespace-mode config must load");

    // `version` and `project.name` are deliberately absent: they have NO canonical
    // default (defaults.schema.json declares neither), which is exactly what makes
    // them the only two required fields (PROTOCOL_SPEC §9.1). An earlier revision
    // of this test pinned the invented `version: "0.16.0"` / `project.name: "apcore"`
    // pair that Python, TypeScript and Rust each carried — the same pair that made
    // every SDK's required-field check unreachable.
    let expected: &[(&str, serde_json::Value)] = &[
        ("extensions.root", json!("./extensions")),
        ("extensions.auto_discover", json!(true)),
        ("extensions.max_depth", json!(8)),
        ("extensions.follow_symlinks", json!(false)),
        ("schema.root", json!("./schemas")),
        ("schema.strategy", json!("yaml_first")),
        ("schema.max_ref_depth", json!(32)),
        ("acl.root", json!("./acl")),
        ("acl.default_effect", json!("deny")),
        ("sys_modules.enabled", json!(false)),
        ("stream.max_merge_depth", json!(32)),
    ];
    for (key, want) in expected {
        assert_eq!(
            config.get(key).as_ref(),
            Some(want),
            "config.get({key:?}) must resolve to the canonical default"
        );
    }
}

/// An explicit value in the file always beats the default table.
#[test]
fn c4_file_value_wins_over_default() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(
        &dir,
        "override.yaml",
        "apcore:\n  modules_path: ./modules\nschema:\n  root: ./custom-schemas\n  max_ref_depth: 8\n",
    );
    let config = Config::load(&p).unwrap();
    assert_eq!(config.get("schema.root"), Some(json!("./custom-schemas")));
    assert_eq!(config.get("schema.max_ref_depth"), Some(json!(8)));
}

/// The default table must NOT satisfy the legacy-mode required-field check:
/// a required field has to be *declared*. This pins decision A-D-03 against
/// regression from the new `CONFIG_DEFAULTS` fallback.
#[test]
fn c4_defaults_do_not_satisfy_required_field_validation() {
    let config = Config::default();
    // `schema.root` HAS a canonical default, so it exercises the two accessors'
    // split; `version` has none, which is why it is required at all.
    assert_eq!(
        config.get("schema.root"),
        Some(json!("./schemas")),
        "get() must surface the canonical default"
    );
    assert_eq!(
        config.get_declared("schema.root"),
        None,
        "get_declared() must not surface a defaulted value"
    );
    assert_eq!(
        config.get("version"),
        None,
        "`version` has no canonical default — defaults.schema.json declares \
         none, and neither peer SDK invents one"
    );
    let err = config
        .validate()
        .expect_err("a bare default config declares no required fields (A-D-03)");
    assert_eq!(err.code, ErrorCode::ConfigInvalid);
}

// ---------------------------------------------------------------------------
// C5 — `observability.tracing.sampling_rate` must be enforceable
// ---------------------------------------------------------------------------

/// An out-of-range sampling rate must be rejected with `CONFIG_INVALID`, as
/// apcore-python and apcore-typescript do. Rust accepted it because
/// `TracingConfig` had no such field and `observability` was a typed struct
/// excluded from `user_namespaces`, so the value was dropped before the
/// constraint table could see it.
///
/// CORRECTED for apcore-rust#33: modelling `sampling_rate` on `TracingConfig`
/// fixed this ONE key; every unmodelled sibling was still dropped the same way
/// until `Config::deserialize` started keeping the raw `observability` object
/// in `user_namespaces` too. The typed leaf still wins, so this case is
/// unaffected — see `tests/test_config_load_observability_subkeys.rs` for the
/// siblings.
#[test]
fn c5_out_of_range_sampling_rate_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(
        &dir,
        "bad_sampling.yaml",
        "apcore:\n  modules_path: ./modules\nobservability:\n  tracing:\n    enabled: true\n    sampling_rate: 5.0\n",
    );
    let err = Config::load(&p).expect_err("sampling_rate 5.0 is outside [0.0, 1.0]");
    assert_eq!(err.code, ErrorCode::ConfigInvalid);
    assert!(
        err.message.contains("sampling_rate"),
        "error must name the offending key: {}",
        err.message
    );
}

/// A legitimate sampling rate must survive deserialization, `get()` and the
/// `data()` round-trip — it was previously discarded silently.
#[test]
fn c5_valid_sampling_rate_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(
        &dir,
        "sampling.yaml",
        "apcore:\n  modules_path: ./modules\nobservability:\n  tracing:\n    enabled: true\n    sampling_rate: 0.1\n",
    );
    let config = Config::load(&p).unwrap();
    assert_eq!(
        config.get("observability.tracing.sampling_rate"),
        Some(json!(0.1))
    );
    assert_eq!(
        config.data()["observability"]["tracing"]["sampling_rate"],
        json!(0.1)
    );
}

// ---------------------------------------------------------------------------
// C6 — built-in `observability` namespace defaults (PROTOCOL_SPEC §9.15.2)
// ---------------------------------------------------------------------------

#[test]
fn c6_observability_namespace_defaults_match_spec_9_15_2() {
    let config = Config::from_defaults();
    let ns = config.namespace("observability");

    let tracing = ns.get("tracing").expect("tracing defaults present");
    assert_eq!(tracing["enabled"], json!(false));
    assert_eq!(tracing["strategy"], json!("full"));
    assert_eq!(tracing["sampling_rate"], json!(1.0));
    assert_eq!(tracing["exporter"], json!("stdout"));
    assert_eq!(
        tracing["otlp_endpoint"],
        json!(null),
        "spec and both peers default otlp_endpoint to null; a live localhost \
         endpoint would make a Rust service export where its peers do not"
    );

    let metrics = ns.get("metrics").expect("metrics defaults present");
    assert_eq!(metrics["enabled"], json!(false));
    assert_eq!(metrics["exporter"], json!("stdout"));

    for absent in ["logging", "platform_notify", "error_history", "redaction"] {
        assert!(
            ns.get(absent).is_none(),
            "observability.{absent} belongs to another namespace and MUST NOT be registered here"
        );
    }
}

// ---------------------------------------------------------------------------
// C7 — mounts must survive `reload()` (PROTOCOL_SPEC §9.11)
// ---------------------------------------------------------------------------

#[test]
fn c7_mounted_namespace_survives_reload() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(
        &dir,
        "mountable.yaml",
        "apcore:\n  modules_path: ./modules\nexecutor:\n  default_timeout: 30000\n  global_timeout: 60000\n",
    );
    let mut config = Config::load(&p).unwrap();
    config
        .mount(
            "my-plugin",
            MountSource::Dict(json!({ "timeout": 1234, "nested": { "a": 1 } })),
        )
        .unwrap();
    assert_eq!(config.get("my-plugin.timeout"), Some(json!(1234)));

    config.reload().expect("reload must succeed");

    assert_eq!(
        config.get("my-plugin.timeout"),
        Some(json!(1234)),
        "§9.11: mounted data must survive reload"
    );
    assert_eq!(config.get("my-plugin.nested.a"), Some(json!(1)));
}

/// File content still wins for keys the reloaded file declares.
#[test]
fn c7_reload_reapplies_mount_over_fresh_file_content() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(
        &dir,
        "m2.yaml",
        "apcore:\n  modules_path: ./modules\nexecutor:\n  default_timeout: 30000\n",
    );
    let mut config = Config::load(&p).unwrap();
    config
        .mount("plug", MountSource::Dict(json!({ "only_mounted": true })))
        .unwrap();

    std::fs::write(
        &p,
        "apcore:\n  modules_path: ./modules\nexecutor:\n  default_timeout: 5000\nplug:\n  from_file: true\n",
    )
    .unwrap();
    config.reload().unwrap();

    assert_eq!(config.get("executor.default_timeout"), Some(json!(5000)));
    assert_eq!(config.get("plug.from_file"), Some(json!(true)));
    assert_eq!(config.get("plug.only_mounted"), Some(json!(true)));
}
