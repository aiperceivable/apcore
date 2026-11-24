//! Reload ordering through the two registration paths that dropped declared
//! dependencies (aiperceivable/apcore-rust#35).
//!
//! Fixture source: apcore/conformance/fixtures/system_modules_hardening.json,
//! case `reload_order_is_topological_not_alphabetical`.
//!
//! That case is driven in `test_system_modules_hardening_conformance.rs` through
//! the three-argument `Registry::register`, which takes a hand-built
//! `ModuleDescriptor` — the one path that always carried `dependencies`. It
//! therefore passed against both defects this file pins:
//!
//! 1. **Filesystem discovery.** `DefaultDiscoverer::build_descriptor` hard-coded
//!    `dependencies: vec![]` while the pipeline parsed the YAML `dependencies`
//!    into a separate value used only for stage-6 load ordering. Discovery-time
//!    sorting worked, `resolve_dependencies` looked healthy, and
//!    `get_definition().dependencies` came back empty for every discovered
//!    module.
//! 2. **The canonical four-argument `register`.**
//!    `Registry::register_versioned(name, module, version, metadata)` — this
//!    SDK's `register(module_id, module, version?, metadata?)` — discarded
//!    `metadata["dependencies"]`, the exact shape apcore-python and
//!    apcore-typescript accept there.
//!
//! Both end at the same accessor: `ReloadModule::topo_sort_modules` reads
//! `Registry::get_definition(...).dependencies`, so an empty list there sorts an
//! empty graph and Kahn's sort emits its seed order — alphabetical, which looks
//! plausible and is wrong whenever the graph disagrees with the alphabet.
//!
//! The fixture's `driver_contract` governs both tests here:
//!
//! * `ordering_needs_a_disagreeing_graph` — the observation is the sequence of
//!   `unregister` events the reload actually produced, not `reloaded_modules`
//!   from the response (which is appended to inside the same loop, so asserting
//!   it would only check the report against itself).
//! * `dependencies_must_survive_registration` — the edge is declared through the
//!   path under test and read back through `get_definition`, never handed
//!   straight to the sort.

#![allow(clippy::missing_panics_doc)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use apcore::context::{Context, Identity};
use apcore::errors::ModuleError;
use apcore::events::emitter::EventEmitter;
use apcore::module::Module;
use apcore::registry::registry::{Registry, RegistryEvents};
use apcore::sys_modules::control::ReloadModule;

use crate::conformance_env::find_fixtures_root;

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

const CASE_ID: &str = "reload_order_is_topological_not_alphabetical";

/// The parts of the canonical case both tests need, read once so neither test
/// hand-transcribes an id, an edge or an expected order.
struct Case {
    /// `setup.registered_modules`.
    registered: Vec<String>,
    /// `setup.declared_dependencies` — module id -> ids it depends on.
    declared: HashMap<String, Vec<String>>,
    /// `action.input` for `system.control.reload_module`.
    input: Value,
    /// `expected.reload_order_observed`.
    observed: Vec<String>,
    /// `expected.alphabetical_order_would_be`.
    alphabetical: Vec<String>,
}

fn load_case() -> Case {
    let path = find_fixtures_root().join("system_modules_hardening.json");
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Failed to read fixture: {}", path.display()));
    let fixture: Value =
        serde_json::from_str(&content).unwrap_or_else(|e| panic!("Invalid JSON: {e}"));
    let case = fixture["test_cases"]
        .as_array()
        .expect("test_cases must be an array")
        .iter()
        .find(|c| c["id"].as_str() == Some(CASE_ID))
        .unwrap_or_else(|| panic!("fixture case '{CASE_ID}' not present"));

    let registered: Vec<String> = case["setup"]["registered_modules"]
        .as_array()
        .expect("setup.registered_modules is an array")
        .iter()
        .map(|v| v.as_str().expect("module id is a string").to_string())
        .collect();
    let declared: HashMap<String, Vec<String>> =
        serde_json::from_value(case["setup"]["declared_dependencies"].clone())
            .expect("setup.declared_dependencies is a map of module id -> dependency ids");
    let observed: Vec<String> = case["expected"]["reload_order_observed"]
        .as_array()
        .expect("expected.reload_order_observed is an array")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let alphabetical: Vec<String> = case["expected"]["alphabetical_order_would_be"]
        .as_array()
        .expect("expected.alphabetical_order_would_be is an array")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();

    // The whole point of the case: the two candidate orders must disagree, or
    // neither test below can tell a topological sort from a plain one.
    assert_eq!(
        case["expected"]["orders_differ"].as_bool(),
        Some(true),
        "{CASE_ID} must declare orders_differ: true"
    );
    assert_ne!(
        observed, alphabetical,
        "{CASE_ID} declares orders_differ but states the same order twice"
    );

    Case {
        registered,
        declared,
        input: case["action"]["input"].clone(),
        observed,
        alphabetical,
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

struct StubModule;

#[async_trait::async_trait]
impl Module for StubModule {
    fn description(&self) -> &'static str {
        "stub module for reload-order tests"
    }
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
}

fn make_ctx() -> Context<Value> {
    Context {
        trace_id: "trace-reload-order".to_string(),
        identity: Some(Identity::new(
            "ops".to_string(),
            "user".to_string(),
            vec![],
            HashMap::new(),
        )),
        services: Value::Null,
        caller_id: None,
        data: Arc::new(parking_lot::RwLock::new(HashMap::new())),
        call_chain: vec![],
        redacted_inputs: None,
        redacted_output: None,
        cancel_token: None,
        global_deadline: None,
        executor: None,
    }
}

/// `dependencies_must_survive_registration` — read the graph back through the
/// post-registration accessor `ReloadModule::topo_sort_modules` itself uses.
fn assert_dependencies_survived(
    registry: &Registry,
    declared: &HashMap<String, Vec<String>>,
    path: &str,
) {
    for (module_id, deps) in declared {
        let descriptor = registry
            .get_definition(module_id)
            .expect("get_definition must not error")
            .unwrap_or_else(|| panic!("{module_id} is registered"));
        let stored: Vec<String> = descriptor
            .dependencies
            .iter()
            .map(|d| d.module_id.clone())
            .collect();
        assert_eq!(
            &stored, deps,
            "{module_id} declared dependencies {deps:?} via {path}, but \
             get_definition() reports {stored:?}"
        );
    }
}

/// Run the fixture's bulk reload and return the order the work happened in,
/// taken from the `unregister` registry event (`ordering_needs_a_disagreeing_graph`).
async fn observe_reload_order(registry: &Arc<Registry>, input: Value) -> Vec<String> {
    let recorded = Arc::new(Mutex::new(Vec::<String>::new()));
    let recorder = Arc::clone(&recorded);
    registry
        .on(
            RegistryEvents::UNREGISTER,
            Box::new(move |name: &str, _module: &dyn Module| {
                recorder.lock().unwrap().push(name.to_string());
            }),
        )
        .expect("valid registry event");

    let module = ReloadModule::new(Arc::clone(registry), Arc::new(EventEmitter::new()));
    let out = module
        .execute(input, &make_ctx())
        .await
        .expect("bulk reload should succeed");
    assert_eq!(
        out["success"].as_bool(),
        Some(true),
        "bulk reload must report success"
    );

    let order = recorded.lock().unwrap().clone();
    order
}

fn assert_topological_not_alphabetical(actual: &[String], case: &Case, path: &str) {
    assert_eq!(
        actual,
        &case.observed[..],
        "modules registered via {path} were reloaded in {actual:?}; the declared \
         dependency graph requires {:?}",
        case.observed
    );
    assert_ne!(
        actual,
        &case.alphabetical[..],
        "reload order collapsed to the alphabetical order — the dependency graph \
         declared via {path} was ignored"
    );
}

// ---------------------------------------------------------------------------
// Path 1: filesystem discovery
// ---------------------------------------------------------------------------

/// Lay the fixture's module set out on disk: `executor.alpha` becomes
/// `<root>/executor/alpha.rs`, and any declared dependencies become the
/// companion `<root>/executor/alpha_meta.yaml` the scanner looks for.
fn write_discovery_tree(root: &std::path::Path, case: &Case) {
    for module_id in &case.registered {
        let segments: Vec<&str> = module_id.split('.').collect();
        let (stem, dirs) = segments.split_last().expect("module id has a segment");
        let dir: PathBuf = dirs.iter().fold(root.to_path_buf(), |acc, s| acc.join(s));
        std::fs::create_dir_all(&dir).expect("create module dir");
        std::fs::write(dir.join(format!("{stem}.rs")), "// discovered stub\n")
            .expect("write module file");

        if let Some(deps) = case.declared.get(module_id) {
            // The YAML shape `parse_dependencies` reads, and the one
            // `features/registry-system.md` documents for `_meta.yaml`.
            let mut yaml = String::from("description: discovered stub\ndependencies:\n");
            for dep in deps {
                yaml.push_str(&format!("  - module_id: {dep}\n"));
            }
            std::fs::write(dir.join(format!("{stem}_meta.yaml")), yaml).expect("write meta file");
        }
    }
}

#[tokio::test]
async fn discovered_dependencies_survive_registration_and_order_the_reload() {
    let case = load_case();
    let tmp = tempfile::tempdir().expect("tempdir");
    write_discovery_tree(tmp.path(), &case);

    let factory: apcore::ModuleFactory =
        Arc::new(|_file, _entry_point| Ok(Some(Arc::new(StubModule) as Arc<dyn Module>)));

    let registry = Arc::new(Registry::new());
    registry.set_extension_roots(vec![tmp.path().to_string_lossy().into_owned()]);
    registry.set_discoverer(Box::new(
        apcore::DefaultDiscoverer::new().with_factory(factory),
    ));

    let count = registry
        .discover_internal()
        .await
        .expect("discovery should succeed");
    assert_eq!(
        count,
        case.registered.len(),
        "every fixture module should have been discovered from {}",
        tmp.path().display()
    );

    assert_dependencies_survived(&registry, &case.declared, "filesystem discovery");

    let actual = observe_reload_order(&registry, case.input.clone()).await;
    assert_topological_not_alphabetical(&actual, &case, "filesystem discovery");
}

// ---------------------------------------------------------------------------
// Path 2: the canonical four-argument register
// ---------------------------------------------------------------------------

#[tokio::test]
async fn four_arg_register_metadata_dependencies_survive_registration_and_order_the_reload() {
    let case = load_case();
    let registry = Arc::new(Registry::new());

    for module_id in &case.registered {
        // The metadata shape apcore-python and apcore-typescript accept in the
        // same argument position of `register(module_id, module, version?,
        // metadata?)`: a list of `{module_id, version?, optional?}` objects.
        let metadata: Option<HashMap<String, Value>> = case.declared.get(module_id).map(|deps| {
            let entries: Vec<Value> = deps.iter().map(|d| json!({"module_id": d})).collect();
            HashMap::from([("dependencies".to_string(), Value::Array(entries))])
        });
        registry
            .register_versioned(module_id, Box::new(StubModule), None, metadata)
            .expect("four-argument registration");
    }

    assert_dependencies_survived(&registry, &case.declared, "register_versioned metadata");

    // Path 1 above installs a DefaultDiscoverer; this path registers
    // programmatically, so supply the equivalent restoring discoverer or the
    // reload fails with RELOAD_FAILED before order can be observed.
    crate::reload_support::RestoringDiscoverer::attach_for(&registry, &case.registered);

    let actual = observe_reload_order(&registry, case.input.clone()).await;
    assert_topological_not_alphabetical(&actual, &case, "register_versioned metadata");
}
