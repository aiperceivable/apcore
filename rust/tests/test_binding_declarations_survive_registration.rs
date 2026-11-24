//! Regression test: everything a `*.binding.yaml` declares MUST survive
//! registration.
//!
//! `BindingLoader::register_into_with_handlers` builds a [`FunctionModule`]
//! carrying the binding's `annotations`, `tags`, `documentation`, `metadata`
//! (including the `display` block, folded in under `apcore.display`) and then
//! hands it to `Registry::register_module`. `register_module` derives the
//! descriptor from the `Module` trait — so any field the trait does not expose
//! is silently dropped on the way in.
//!
//! `FunctionModule` implemented only `input_schema` / `output_schema` /
//! `description` / `execute`, so `annotations()` and `tags()` fell through to
//! the trait defaults and `register_module` hardcoded `documentation: None` /
//! `metadata: {}`. Every declaration above was therefore discarded.
//!
//! Cross-SDK parity: apcore-python (`decorator.py` + `registry/metadata.py`)
//! and apcore-typescript (`bindings.ts` + `registry/metadata-pure.ts`) both
//! carry the declared values through to the registered descriptor.
//!
//! The security half is the composed case at the bottom: a binding declaring
//! `requires_approval: true` must actually reach the `ApprovalHandler`.
//! `tests/conformance_test.rs` (`binding_yaml_canonical`) asserts only module
//! IDs and never registers a module, so nothing pinned this.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use apcore::approval::{ApprovalHandler, ApprovalRequest, ApprovalResult};
use apcore::bindings::{BindingHandler, BindingLoader};
use apcore::config::Config;
use apcore::context::Context;
use apcore::errors::ModuleError;
use apcore::executor::Executor;
use apcore::registry::registry::Registry;

const BINDING_YAML: &str = r#"
spec_version: "1.0"
bindings:
  - module_id: executor.orders.delete_order
    target: "orders:delete_order"
    input_schema: {type: object}
    output_schema: {type: object}
    description: "Delete an order"
    documentation: "Long-form **Markdown** documentation for delete_order."
    tags: ["orders", "dangerous"]
    annotations:
      requires_approval: true
      destructive: true
    display:
      cli:
        hidden: false
      mcp:
        title: "Delete Order"
"#;

fn echo_handler() -> BindingHandler {
    Arc::new(|inputs: Value, _ctx: &Context<Value>| Box::pin(async move { Ok(inputs) }))
}

/// Write the binding YAML into a temp dir and register it into `registry`.
fn register_binding(registry: &Registry) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("orders.binding.yaml");
    std::fs::write(&path, BINDING_YAML).expect("write binding yaml");

    let mut loader = BindingLoader::new();
    loader.load_from_yaml(&path).expect("load binding yaml");

    let mut handlers: HashMap<String, BindingHandler> = HashMap::new();
    handlers.insert("orders:delete_order".to_string(), echo_handler());

    let count = loader
        .register_into_with_handlers(registry, handlers)
        .expect("register bindings");
    assert_eq!(count, 1, "exactly one binding is declared");
    dir
}

#[test]
fn binding_declarations_reach_the_registry_descriptor() {
    let registry = Registry::new();
    let _dir = register_binding(&registry);

    let descriptor = registry
        .get_definition("executor.orders.delete_order")
        .expect("get_definition")
        .expect("module must be registered");

    // annotations — the governance half.
    let annotations = descriptor
        .annotations
        .as_ref()
        .expect("descriptor MUST carry the binding's annotations");
    assert!(
        annotations.requires_approval,
        "a binding declaring `requires_approval: true` MUST reach the descriptor"
    );
    assert!(
        annotations.destructive,
        "a binding declaring `destructive: true` MUST reach the descriptor"
    );

    // tags
    assert_eq!(
        descriptor.tags,
        vec!["orders".to_string(), "dangerous".to_string()],
        "a binding's `tags` MUST reach the descriptor"
    );

    // documentation
    assert_eq!(
        descriptor.documentation.as_deref(),
        Some("Long-form **Markdown** documentation for delete_order."),
        "a binding's `documentation` MUST reach the descriptor"
    );

    // display, folded into metadata under the canonical `apcore.display` key
    // by the binding loader — `register_module` used to overwrite the whole
    // metadata map with an empty one.
    assert_eq!(
        descriptor.metadata.get("apcore.display"),
        Some(&json!({
            "cli": {"hidden": false},
            "mcp": {"title": "Delete Order"},
        })),
        "a binding's `display` block MUST survive as metadata[\"apcore.display\"]"
    );

    // description was already carried through; assert it so a regression in
    // the path this test exercises is not mistaken for an unrelated failure.
    assert_eq!(descriptor.description, "Delete an order");
}

/// Handler that counts invocations and approves.
#[derive(Debug)]
struct CountingHandler {
    invocations: Arc<AtomicUsize>,
}

#[async_trait]
impl ApprovalHandler for CountingHandler {
    async fn request_approval(
        &self,
        _request: &ApprovalRequest,
    ) -> Result<ApprovalResult, ModuleError> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        let mut result = ApprovalResult::default();
        result.status = "approved".to_string();
        result.approved_by = Some("counter".to_string());
        Ok(result)
    }

    async fn check_approval(&self, _approval_id: &str) -> Result<ApprovalResult, ModuleError> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        let mut result = ApprovalResult::default();
        result.status = "approved".to_string();
        Ok(result)
    }
}

/// The composed case. The gate reads the LIVE module's `annotations()` (see
/// `test_approval_gate_decides_from_live_module.rs`), and the live module here
/// is the `FunctionModule` the binding loader built — so before the two fixes
/// this call ran ungated twice over: the loader's declaration never reached
/// the module's `annotations()`, and the gate was not reading that accessor
/// anyway. This asserts the end-to-end path a `*.binding.yaml` author relies on.
#[tokio::test]
async fn binding_requires_approval_reaches_the_approval_handler() {
    let registry = Arc::new(Registry::new());
    let _dir = register_binding(&registry);

    let invocations = Arc::new(AtomicUsize::new(0));
    let mut executor = Executor::new(Arc::clone(&registry), Arc::new(Config::default()));
    executor.set_approval_handler(Box::new(CountingHandler {
        invocations: Arc::clone(&invocations),
    }));

    let outcome = executor
        .call("executor.orders.delete_order", json!({"id": 7}), None, None)
        .await;
    assert!(
        outcome.is_ok(),
        "the handler approves, so the call proceeds: {outcome:?}"
    );

    assert_eq!(
        invocations.load(Ordering::SeqCst),
        1,
        "a binding declaring `requires_approval: true` MUST gate the call"
    );
}

// ===========================================================================
// The other half of the same gap: `version` and `examples`
// ===========================================================================
//
// D-97 gave `FunctionModule` trait accessors for `annotations` / `tags` /
// `documentation` / `metadata`, which is what the tests above pin. It left
// `version` and `examples` as fields the `Module` trait exposes no accessor
// for — so `Registry::register_module` had nothing to read and hardcoded
// `DEFAULT_MODULE_VERSION` and `vec![]`, exactly as it had hardcoded
// `documentation: None` and `metadata: {}` before.
//
// The version half is the one with teeth, because the value is **validated on
// the way in and discarded on the way out**: `BindingValidator` checks
// `entry.version` against `validation.binding.version_require_semver`, the
// loader copies it onto the `FunctionModule`, and the registry then registered
// the module as `1.0.0`. A binding declaring `version: "2.3.0"` therefore
// passed semver validation and became un-addressable by that version.
//
// apcore-python (`merge_module_metadata`: `getattr(module, "version",
// "1.0.0")` / `merge_examples`) and apcore-typescript (`mergeModuleMetadata`)
// both carry the module's declarations through. Measured, not assumed: both
// report `descriptor.version == "2.3.0"` and one example for the same input.

const VERSIONED_BINDING_YAML: &str = r#"
spec_version: "1.0"
bindings:
  - module_id: executor.orders.archive_order
    target: "orders:archive_order"
    input_schema: {type: object}
    output_schema: {type: object}
    description: "Archive an order"
    version: "2.3.0"
"#;

#[test]
fn a_binding_declared_version_survives_registration() {
    let registry = Registry::new();

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("orders_versioned.binding.yaml");
    std::fs::write(&path, VERSIONED_BINDING_YAML).expect("write binding yaml");

    let mut loader = BindingLoader::new();
    loader.load_from_yaml(&path).expect("load binding yaml");
    let mut handlers: HashMap<String, BindingHandler> = HashMap::new();
    handlers.insert("orders:archive_order".to_string(), echo_handler());
    loader
        .register_into_with_handlers(&registry, handlers)
        .expect("register bindings");

    let descriptor = registry
        .get_definition("executor.orders.archive_order")
        .expect("lookup")
        .expect("registered");

    assert_eq!(
        descriptor.version, "2.3.0",
        "the binding's declared version is validated against \
         validation.binding.version_require_semver on the way in; registering the \
         module as 1.0.0 discards the value that was just checked and leaves it \
         un-addressable by the version it declares"
    );
}

#[test]
fn a_modules_own_version_and_examples_reach_the_descriptor() {
    use apcore::decorator::FunctionModule;
    use apcore::module::{ModuleAnnotations, ModuleExample};

    let registry = Registry::new();
    let mut example = ModuleExample::default();
    example.title = "archive one order".to_string();

    let module = FunctionModule::with_description(
        ModuleAnnotations::default(),
        json!({"type": "object"}),
        json!({"type": "object"}),
        "probe".to_string(),
        None,
        vec![],
        "2.3.0",
        HashMap::new(),
        vec![example],
        move |inputs: Value, _ctx: &Context<Value>| Box::pin(async move { Ok(inputs) }),
    );
    registry
        .register_module("executor.probe.versioned", Box::new(module))
        .expect("register module");

    let descriptor = registry
        .get_definition("executor.probe.versioned")
        .expect("lookup")
        .expect("registered");

    assert_eq!(descriptor.version, "2.3.0");
    assert_eq!(
        descriptor.examples.len(),
        1,
        "both peers merge the module's examples into the descriptor"
    );
    assert_eq!(descriptor.examples[0].title, "archive one order");
}

#[test]
fn register_versioned_prefers_its_argument_then_the_module() {
    use apcore::decorator::FunctionModule;
    use apcore::module::ModuleAnnotations;

    fn probe(version: &str) -> FunctionModule {
        FunctionModule::with_description(
            ModuleAnnotations::default(),
            json!({"type": "object"}),
            json!({"type": "object"}),
            "probe".to_string(),
            Some("module docs".to_string()),
            vec![],
            version,
            HashMap::from([(
                "dependencies".to_string(),
                json!([{"module_id": "common.util", "version": ">=1.0.0"}]),
            )]),
            Vec::new(),
            move |inputs: Value, _ctx: &Context<Value>| Box::pin(async move { Ok(inputs) }),
        )
    }

    let registry = Registry::new();

    // No explicit version: fall through to the module's own.
    registry
        .register_versioned(
            "executor.probe.fallback",
            Box::new(probe("2.3.0")),
            None,
            None,
        )
        .expect("register");
    let d = registry
        .get_definition("executor.probe.fallback")
        .expect("lookup")
        .expect("registered");
    assert_eq!(d.version, "2.3.0", "module version is the fallback");
    assert_eq!(
        d.dependencies.len(),
        1,
        "and so are the module's own dependencies — parsing only the `metadata` \
         ARGUMENT left a module that declares them on ITSELF with an empty graph, \
         which `ReloadModule::topo_sort_modules` then sorts into alphabetical order"
    );
    assert_eq!(
        d.documentation.as_deref(),
        Some("module docs"),
        "the canonical four-argument form dropped `documentation` too — and it is \
         the path registry-system.md names as the cross-language-symmetric one"
    );

    // An explicit argument wins, matching apcore-python's
    // `meta.get("version") or code_version or "1.0.0"`.
    registry
        .register_versioned(
            "executor.probe.explicit",
            Box::new(probe("2.3.0")),
            Some("4.0.0"),
            None,
        )
        .expect("register");
    assert_eq!(
        registry
            .get_definition("executor.probe.explicit")
            .expect("lookup")
            .expect("registered")
            .version,
        "4.0.0",
        "the caller's explicit version outranks the module's declaration"
    );
}
