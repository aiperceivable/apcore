//! Cross-language parity regressions — WARNING findings from `/apcore-skills:sync`.
//!
//! Each test pins a behavior where apcore-rust diverged from apcore-python and
//! apcore-typescript. The authority is the spec repo (`PROTOCOL_SPEC`,
//! JSON Schema 2020-12).

#![allow(clippy::pedantic, clippy::all)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use apcore::errors::ErrorCode;
use apcore::schema::RefResolver;
use serde_json::json;

fn write(dir: &Path, rel: &str, body: &str) -> PathBuf {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&p, body).unwrap();
    p
}

// ---------------------------------------------------------------------------
// W1 — cross-file `$ref` formats required by PROTOCOL_SPEC §4.11
// ---------------------------------------------------------------------------

/// `apcore://<dotted.module.id>/<Target>` must resolve under the schemas root.
/// The canonical form was entirely absent — `lookup_ref` only handled
/// in-document `#` pointers and exact-string registered URIs.
#[test]
fn w1_canonical_apcore_ref_resolves_under_schemas_root() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "common/types/error.schema.yaml",
        "definitions:\n  ErrorDetail:\n    type: object\n    properties:\n      code: {type: string}\n",
    );

    let resolver = RefResolver::new().with_schemas_dir(root);
    let schema = json!({
        "type": "object",
        "properties": {
            "error": { "$ref": "apcore://common.types.error/ErrorDetail" }
        }
    });
    let out = resolver
        .resolve(&schema)
        .expect("canonical $ref must resolve");
    assert_eq!(
        out["properties"]["error"]["properties"]["code"]["type"],
        json!("string")
    );
}

/// The relative cross-file form with a `#` fragment. The fragment was never
/// split off, so even registering the bare file path could not match.
#[test]
fn w1_relative_cross_file_ref_with_fragment_resolves() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "common/error.schema.yaml",
        "definitions:\n  ErrorDetail:\n    type: object\n    properties:\n      msg: {type: string}\n",
    );
    let entry = write(
        root,
        "executor/validator.schema.yaml",
        "type: object\nproperties:\n  err:\n    $ref: \"../common/error.schema.yaml#/definitions/ErrorDetail\"\n",
    );

    let doc: serde_json::Value =
        serde_yaml_ng::from_str(&std::fs::read_to_string(&entry).unwrap()).unwrap();
    let resolver = RefResolver::new()
        .with_schemas_dir(root)
        .with_current_file(&entry);
    let out = resolver.resolve(&doc).expect("relative $ref must resolve");
    assert_eq!(
        out["properties"]["err"]["properties"]["msg"]["type"],
        json!("string")
    );
}

/// A reference that escapes the schemas root must be rejected, not followed.
#[test]
fn w1_cross_file_ref_outside_schemas_root_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("schemas");
    std::fs::create_dir_all(&root).unwrap();
    write(dir.path(), "outside.schema.yaml", "type: object\n");
    let entry = write(
        &root,
        "entry.schema.yaml",
        "type: object\nproperties:\n  x:\n    $ref: \"../outside.schema.yaml\"\n",
    );

    let doc: serde_json::Value =
        serde_yaml_ng::from_str(&std::fs::read_to_string(&entry).unwrap()).unwrap();
    let err = RefResolver::new()
        .with_schemas_dir(&root)
        .with_current_file(&entry)
        .resolve(&doc)
        .expect_err("a $ref escaping the schemas root must be rejected");
    assert_eq!(err.code, ErrorCode::SchemaNotFound);
    assert!(
        err.message.contains("outside the schemas root"),
        "error must name the containment violation: {}",
        err.message
    );
}

/// Without a schemas root, a cross-file reference must fail loudly rather than
/// resolve against an arbitrary directory.
#[test]
fn w1_cross_file_ref_without_schemas_root_is_rejected() {
    let resolver = RefResolver::new();
    let schema = json!({ "$ref": "apcore://common.types.error/ErrorDetail" });
    let err = resolver.resolve(&schema).unwrap_err();
    assert_eq!(err.code, ErrorCode::SchemaNotFound);
}

// ---------------------------------------------------------------------------
// W3 — a `#/…` pointer inside an external schema must rebase on that document
// ---------------------------------------------------------------------------

/// The `root` used to be threaded unchanged into the resolved external
/// document, so `#/definitions/Inner` written *inside* `other.schema.yaml` was
/// looked up in the CALLING document's tree. JSON Schema 2020-12 §8.2 requires
/// the rebase; apcore-python and apcore-typescript compute a per-hop
/// `effective_file` precisely to avoid this.
#[test]
fn w3_pointer_inside_external_schema_resolves_in_that_document() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "other.schema.yaml",
        "definitions:\n  Outer:\n    type: object\n    properties:\n      inner: {$ref: \"#/definitions/Inner\"}\n  Inner:\n    type: string\n    const: from-external\n",
    );
    // The CALLING document also defines `Inner`, with a different value. If the
    // root were not rebased, the external `#/definitions/Inner` would pick this
    // one up.
    let entry = write(
        root,
        "entry.schema.yaml",
        "type: object\ndefinitions:\n  Inner:\n    type: string\n    const: from-caller\nproperties:\n  outer:\n    $ref: \"./other.schema.yaml#/definitions/Outer\"\n",
    );

    let doc: serde_json::Value =
        serde_yaml_ng::from_str(&std::fs::read_to_string(&entry).unwrap()).unwrap();
    let out = RefResolver::new()
        .with_schemas_dir(root)
        .with_current_file(&entry)
        .resolve(&doc)
        .expect("cross-file resolution must succeed");
    assert_eq!(
        out["properties"]["outer"]["properties"]["inner"]["const"],
        json!("from-external"),
        "a `#/…` pointer inside an external schema must resolve in that schema's tree"
    );
}

// ---------------------------------------------------------------------------
// W2 — `SchemaLoader` must actually invoke `RefResolver`
// ---------------------------------------------------------------------------

mod w2 {
    use super::{write, ErrorCode};
    use apcore::config::Config;
    use apcore::schema::loader::SchemaLoader;

    fn config_with(schema_root: &std::path::Path, max_ref_depth: u64) -> Config {
        let mut config = Config::default();
        config.set(
            "schema.root",
            serde_json::json!(schema_root.to_string_lossy()),
        );
        config.set("schema.max_ref_depth", serde_json::json!(max_ref_depth));
        config
    }

    /// Nothing on the Rust load path resolved `$ref`, so a `$ref` → `$ref`
    /// cycle in a schema file loaded fine and only blew up later (or never).
    /// apcore-python (`loader.py`) and apcore-typescript (`loader.ts`) both
    /// wire the resolver into the loader.
    #[test]
    fn w2_loader_raises_circular_ref_at_load_time() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "bad/cycle.schema.yaml",
            "$ref: \"#/$defs/a\"\n$defs:\n  a: {$ref: \"#/$defs/b\"}\n  b: {$ref: \"#/$defs/a\"}\n",
        );
        let config = config_with(dir.path(), 32);
        let mut loader = SchemaLoader::with_config(&config, None);
        let err = loader
            .load("bad.cycle")
            .expect_err("a $ref -> $ref cycle must be rejected at load time");
        assert_eq!(err.code, ErrorCode::SchemaCircularRef);
    }

    /// `schema.max_ref_depth` was silently inert: `with_max_depth` /
    /// `max_depth()` had no caller anywhere on the load path.
    #[test]
    fn w2_loader_honours_schema_max_ref_depth() {
        let dir = tempfile::tempdir().unwrap();
        // A well-formed chain of 4 hops: root -> a -> b -> c -> leaf.
        write(
            dir.path(),
            "deep/chain.schema.yaml",
            "$ref: \"#/$defs/a\"\n$defs:\n  a: {$ref: \"#/$defs/b\"}\n  b: {$ref: \"#/$defs/c\"}\n  c: {type: string}\n",
        );

        let generous = config_with(dir.path(), 32);
        let mut loader = SchemaLoader::with_config(&generous, None);
        assert_eq!(loader.max_ref_depth(), 32);
        loader
            .load("deep.chain")
            .expect("a 3-hop chain fits inside max_ref_depth=32");

        let tight = config_with(dir.path(), 2);
        let mut loader = SchemaLoader::with_config(&tight, None);
        assert_eq!(loader.max_ref_depth(), 2);
        let err = loader
            .load("deep.chain")
            .expect_err("the same chain must exceed max_ref_depth=2");
        assert_eq!(err.code, ErrorCode::SchemaMaxDepthExceeded);
    }

    /// A plain schema with a resolvable local `$ref` still loads and is
    /// dereferenced — the fix must not reject the happy path.
    #[test]
    fn w2_loader_inlines_resolvable_local_refs() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "ok/simple.schema.yaml",
            "module_id: ok.simple\ndescription: simple\ninput_schema:\n  type: object\n  properties:\n    name: {$ref: \"#/$defs/name\"}\noutput_schema: {type: object}\n$defs:\n  name: {type: string}\n",
        );
        let config = config_with(dir.path(), 32);
        let mut loader = SchemaLoader::with_config(&config, None);
        let def = loader.load("ok.simple").expect("must load");
        assert_eq!(def.input_schema["properties"]["name"]["type"], "string");
    }
}

// ---------------------------------------------------------------------------
// W4 — compound ACL operators must evaluate sub-conditions in the enclosing mode
// ---------------------------------------------------------------------------

mod w4 {
    use apcore::acl::{ACLRule, ACL};
    use apcore::acl_handlers::{register_async_condition, register_condition, ACLConditionHandler};
    use apcore::context::{Context, Identity};
    use async_trait::async_trait;
    use serde_json::{json, Value};
    use std::collections::HashMap;

    struct Fixed(bool);

    #[async_trait]
    impl ACLConditionHandler for Fixed {
        async fn evaluate(&self, _value: &Value, _ctx: &Context<Value>) -> bool {
            self.0
        }
    }

    fn ctx() -> Context<Value> {
        Context::new(Identity::new(
            "u".into(),
            "user".into(),
            vec![],
            HashMap::new(),
        ))
    }

    fn acl_with_or() -> ACL {
        ACL::init_builtin_handlers();
        let mut conditions = serde_json::Map::new();
        conditions.insert("$or".to_string(), json!([{ "_w4_mode_probe": true }]));
        ACL::new(
            vec![{
                let mut rule = ACLRule::new(vec!["*".to_string()], vec!["*".to_string()], "allow");
                rule.conditions = Some(Value::Object(conditions));
                rule
            }],
            "deny",
            None,
        )
    }

    /// PROTOCOL_SPEC §6.1: sub-conditions MUST be evaluated in the same mode as
    /// the enclosing call. `$or`/`$not` were registered only in the SYNC
    /// registry yet delegated to `evaluate_conditions_async`, which consults
    /// the ASYNC registry first — so a sync `ACL::check` resolved the
    /// sub-condition key from the async registry. Python and TypeScript resolve
    /// it from the sync registry only.
    #[tokio::test]
    async fn w4_or_sub_condition_resolves_from_the_enclosing_mode_registry() {
        // Sync registry says DENY, async registry says ALLOW for the same key.
        register_condition("_w4_mode_probe", std::sync::Arc::new(Fixed(false)));
        register_async_condition("_w4_mode_probe", std::sync::Arc::new(Fixed(true)));

        let acl = acl_with_or();
        let c = ctx();

        assert!(
            !acl.check(Some("caller"), "target", Some(&c)),
            "sync check must resolve the $or sub-condition from the SYNC registry (false -> deny)"
        );
        assert!(
            acl.async_check(Some("caller"), "target", Some(&c)).await,
            "async check must resolve it from the ASYNC registry (true -> allow)"
        );
    }
}

// ---------------------------------------------------------------------------
// W5 — a non-string `default_effect` must not be silently coerced
// ---------------------------------------------------------------------------

#[test]
fn w5_non_string_default_effect_is_rejected_at_load() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(
        dir.path(),
        "acl.yaml",
        "default_effect: true\nrules:\n  - callers: ['*']\n    targets: ['*']\n    effect: allow\n",
    );
    let err = apcore::acl::ACL::load(p.to_str().unwrap())
        .expect_err("a non-string default_effect must be reported, not coerced to 'deny'");
    assert_eq!(err.code, ErrorCode::ACLRuleError);
    assert!(
        err.message.contains("default_effect"),
        "error must name the offending key: {}",
        err.message
    );
}

#[test]
fn w5_absent_default_effect_still_defaults_to_deny() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(
        dir.path(),
        "acl.yaml",
        "rules:\n  - callers: ['a']\n    targets: ['b']\n    effect: allow\n",
    );
    let acl = apcore::acl::ACL::load(p.to_str().unwrap()).expect("absent key is valid");
    assert!(!acl.check(Some("x"), "y", None), "default must be deny");
}

// ---------------------------------------------------------------------------
// W6 — an unknown condition key must leave a forensic record
// ---------------------------------------------------------------------------

mod w6 {
    use apcore::acl::{ACLRule, AuditEntry, ACL};
    use apcore::context::{Context, Identity};
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    fn acl_with_unknown_condition(captured: Arc<Mutex<Vec<AuditEntry>>>) -> ACL {
        ACL::init_builtin_handlers();
        let mut conditions = serde_json::Map::new();
        conditions.insert("_w6_never_registered".to_string(), json!(true));
        let mut acl = ACL::new(
            vec![{
                let mut rule = ACLRule::new(vec!["*".to_string()], vec!["*".to_string()], "allow");
                rule.conditions = Some(Value::Object(conditions));
                rule
            }],
            "deny",
            None,
        );
        acl.set_audit_logger(move |entry: &AuditEntry| {
            captured.lock().unwrap().push(entry.clone());
        });
        acl
    }

    fn ctx() -> Context<Value> {
        Context::new(Identity::new(
            "u".into(),
            "user".into(),
            vec![],
            HashMap::new(),
        ))
    }

    /// A typo'd condition key produces an identical DENY everywhere, so the
    /// audit record is the only way to tell it apart from "no rule matched".
    /// TypeScript sets `handlerError` on this branch; Rust left it null.
    #[test]
    fn w6_sync_unknown_condition_key_populates_handler_error() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let acl = acl_with_unknown_condition(Arc::clone(&captured));
        let _ = acl.check(Some("caller"), "target", Some(&ctx()));

        let entries = captured.lock().unwrap();
        let entry = entries.last().expect("an audit entry must be emitted");
        let err = entry
            .handler_error
            .as_deref()
            .expect("handler_error must be populated for an unknown condition key");
        assert!(
            err.contains("_w6_never_registered"),
            "handler_error must name the unknown key: {err}"
        );
    }

    #[tokio::test]
    async fn w6_async_unknown_condition_key_populates_handler_error() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let acl = acl_with_unknown_condition(Arc::clone(&captured));
        let _ = acl
            .async_check(Some("caller"), "target", Some(&ctx()))
            .await;

        let entries = captured.lock().unwrap();
        let entry = entries.last().expect("an audit entry must be emitted");
        assert!(
            entry
                .handler_error
                .as_deref()
                .is_some_and(|e| e.contains("_w6_never_registered")),
            "handler_error must name the unknown key, got {:?}",
            entry.handler_error
        );
    }
}

// ---------------------------------------------------------------------------
// W7 — a Discoverer must not populate the reserved `ephemeral.*` namespace
// ---------------------------------------------------------------------------

mod w7 {
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::module::{Module, ModuleAnnotations};
    use apcore::registry::registry::{DiscoveredModule, Discoverer, ModuleDescriptor, Registry};
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::Arc;

    #[derive(Debug)]
    struct Noop;

    #[async_trait]
    impl Module for Noop {
        fn input_schema(&self) -> serde_json::Value {
            json!({"type": "object"})
        }
        fn output_schema(&self) -> serde_json::Value {
            json!({"type": "object"})
        }
        fn description(&self) -> &str {
            "noop"
        }
        fn annotations(&self) -> ModuleAnnotations {
            ModuleAnnotations::default()
        }
        async fn execute(
            &self,
            _inputs: serde_json::Value,
            _ctx: &Context<serde_json::Value>,
        ) -> Result<serde_json::Value, ModuleError> {
            Ok(json!({}))
        }
    }

    struct EphemeralDiscoverer;

    #[async_trait]
    impl Discoverer for EphemeralDiscoverer {
        async fn discover(&self, _roots: &[String]) -> Result<Vec<DiscoveredModule>, ModuleError> {
            let descriptor = ModuleDescriptor {
                module_id: "ephemeral.agent.tool".to_string(),
                name: None,
                description: String::new(),
                documentation: None,
                input_schema: json!({ "type": "object" }),
                output_schema: json!({ "type": "object" }),
                version: "1.0.0".to_string(),
                tags: vec![],
                annotations: Some(ModuleAnnotations::default()),
                examples: vec![],
                metadata: std::collections::HashMap::new(),
                display: None,
                sunset_date: None,
                dependencies: vec![],
                enabled: true,
            };
            Ok(vec![DiscoveredModule {
                name: "ephemeral.agent.tool".to_string(),
                source: "test".to_string(),
                descriptor,
                module: Arc::new(Noop),
            }])
        }
    }

    /// PROTOCOL_SPEC:424 — `ephemeral.*` may only be populated via
    /// `Registry::register()`. `register_discovered` (the shared sink for
    /// `discover()` and `discover_internal()`) never checked, so a Discoverer
    /// could skip `warn_if_missing_approval` and the namespace's
    /// audit-provenance contract. Python (`registry.py`) and TypeScript
    /// (`registry.ts`) both reject.
    #[tokio::test]
    async fn w7_discover_rejects_ephemeral_namespace() {
        let registry = Registry::new();
        let count = registry
            .discover(&EphemeralDiscoverer)
            .await
            .expect("discovery itself must not error — bad entries are skipped");
        assert_eq!(count, 0, "an ephemeral.* discovery must be rejected");
        assert!(
            registry.get("ephemeral.agent.tool").unwrap().is_none(),
            "the reserved namespace must stay empty"
        );
    }
}

// ---------------------------------------------------------------------------
// W10 — deep-merge at the depth cap must still merge peer keys
// ---------------------------------------------------------------------------

/// At `DEEP_MERGE_MAX_DEPTH` (32) Rust replaced the whole node, dropping every
/// base-only key: chunk A `{a:{a:…{x:1}}}` + chunk B `{a:{a:…{y:2}}}` produced
/// `{y:2}` where apcore-python (`executor.py`) and apcore-typescript
/// (`executor.ts`) shallow-assign and produce `{x:1,y:2}`.
#[test]
fn w10_deep_merge_at_depth_cap_preserves_base_only_keys() {
    fn nest(depth: usize, leaf: serde_json::Value) -> serde_json::Value {
        let mut v = leaf;
        for _ in 0..depth {
            v = json!({ "a": v });
        }
        v
    }

    // DEEP_MERGE_MAX_DEPTH is 32, so 32 wrappers put the leaf objects exactly
    // at the node where the cap fires.
    let chunk_a = nest(32, json!({ "x": 1 }));
    let chunk_b = nest(32, json!({ "y": 2 }));
    let merged = apcore::executor::deep_merge_chunks_checked(&[chunk_a, chunk_b])
        .expect("both chunks are objects");

    let mut node = &merged;
    for _ in 0..32 {
        node = &node["a"];
    }
    assert_eq!(node["x"], json!(1), "base-only key must survive the cap");
    assert_eq!(node["y"], json!(2), "overlay key must be applied");
}

// ---------------------------------------------------------------------------
// W11 — per-instance ToggleState must bind under every built-in preset
// ---------------------------------------------------------------------------

mod w11 {
    use apcore::config::Config;
    use apcore::context::{Context, Identity};
    use apcore::errors::ModuleError;
    use apcore::module::{Module, ModuleAnnotations};
    use apcore::registry::registry::Registry;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;

    #[derive(Debug)]
    struct Noop;

    #[async_trait::async_trait]
    impl Module for Noop {
        fn input_schema(&self) -> serde_json::Value {
            json!({"type": "object"})
        }
        fn output_schema(&self) -> serde_json::Value {
            json!({"type": "object"})
        }
        fn description(&self) -> &str {
            "noop"
        }
        fn annotations(&self) -> ModuleAnnotations {
            ModuleAnnotations::default()
        }
        async fn execute(
            &self,
            _inputs: serde_json::Value,
            _ctx: &Context<serde_json::Value>,
        ) -> Result<serde_json::Value, ModuleError> {
            Ok(json!({}))
        }
    }

    /// Every preset is derived from `build_standard_strategy()`, which binds
    /// the process-global toggle store, so all five need `module_lookup`
    /// rebound — not just `standard`. Previously `apcore.disable(module)` on
    /// one instance was silently ignored by any non-standard-preset executor
    /// (issue #71). apcore-python honours it in all five presets.
    #[tokio::test]
    async fn w11_per_instance_toggle_state_applies_to_every_preset() {
        for preset in ["standard", "internal", "testing", "performance", "minimal"] {
            let registry = Registry::new();
            registry
                .register_module("executor.probe.toggle", Box::new(Noop))
                .unwrap();

            let mut executor = apcore::executor::Executor::with_strategy_name(
                Arc::new(registry),
                Arc::new(Config::from_defaults()),
                preset,
            )
            .unwrap();

            let toggles = Arc::new(apcore::sys_modules::ToggleState::new());
            executor.set_toggle_state(Arc::clone(&toggles));
            toggles.disable("executor.probe.toggle");

            let ctx: Context<serde_json::Value> = Context::new(Identity::new(
                "u".into(),
                "user".into(),
                vec![],
                HashMap::new(),
            ));
            let err = executor
                .call("executor.probe.toggle", json!({}), Some(&ctx), None)
                .await
                .expect_err(&format!(
                    "preset '{preset}' must honour the per-instance toggle store"
                ));
            assert_eq!(
                err.code,
                apcore::errors::ErrorCode::ModuleDisabled,
                "preset '{preset}'"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// W12 — replace / configure_step must not create duplicate step names
// ---------------------------------------------------------------------------

mod w12 {
    use apcore::errors::{ErrorCode, ModuleError};
    use apcore::pipeline::{PipelineContext, Step, StepResult};
    use async_trait::async_trait;

    struct Named(&'static str);

    #[async_trait]
    impl Step for Named {
        fn name(&self) -> &str {
            self.0
        }
        fn description(&self) -> &str {
            "test step"
        }
        fn removable(&self) -> bool {
            true
        }
        fn replaceable(&self) -> bool {
            true
        }
        async fn execute(&self, _ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
            Ok(StepResult::continue_step())
        }
    }

    /// Without the guard the strategy holds two identically-named steps while
    /// `rebuild_index` keeps only the last: a `skip_to` resolves past the first
    /// and `remove` targets the wrong position. apcore-typescript guards the
    /// same case in `ExecutionStrategy.configureStep` (`pipeline.ts`).
    #[test]
    fn w12_replace_rejects_a_name_that_collides_with_another_step() {
        let mut strategy = apcore::builtin_steps::build_standard_strategy();
        let err = strategy
            .replace("acl_check", Box::new(Named("approval_gate")))
            .expect_err("renaming a step onto an existing name must be rejected");
        assert_eq!(err.code, ErrorCode::StepNameDuplicate);
        assert!(
            err.message.contains("already exists"),
            "error must name the collision: {}",
            err.message
        );
        // The strategy must be untouched.
        assert_eq!(
            strategy
                .step_names()
                .iter()
                .filter(|n| *n == "approval_gate")
                .count(),
            1
        );
    }

    #[test]
    fn w12_configure_step_rejects_a_colliding_name() {
        let mut strategy = apcore::builtin_steps::build_standard_strategy();
        let err = strategy
            .configure_step("acl_check", Box::new(Named("approval_gate")))
            .expect_err("renaming a step onto an existing name must be rejected");
        assert_eq!(err.code, ErrorCode::StepNameDuplicate);
        assert!(err.message.contains("already exists"));
    }

    /// Replacing in place (same name) and renaming to a fresh name both stay
    /// legal — the guard must not over-reject.
    #[test]
    fn w12_replace_in_place_and_rename_to_fresh_name_still_work() {
        let mut strategy = apcore::builtin_steps::build_standard_strategy();
        strategy
            .replace("acl_check", Box::new(Named("acl_check")))
            .expect("in-place replacement is legal");
        strategy
            .configure_step("acl_check", Box::new(Named("acl_check_v2")))
            .expect("renaming to an unused name is legal");
        assert!(strategy.step_names().contains(&"acl_check_v2".to_string()));
    }
}

// Silence unused-import warnings for helpers only some cfgs use.
const _: fn() = || {
    let _ = std::mem::size_of::<HashMap<String, String>>();
    let _: Option<Arc<Mutex<u8>>> = None;
};
