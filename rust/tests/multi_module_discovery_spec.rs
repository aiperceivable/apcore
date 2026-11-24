// Spec-traced contract tests for the multi-module-discovery feature.
//
// MIRRORS the canonical Python suite
// apcore-python/tests/test_multi_module_discovery_spec.py. Each test carries the
// verbatim clause-id (format `multi_module_discovery.<method>.<kind>.<detail>`)
// in a leading `// clause:` comment, and the fn name is the clause-id flattened
// to snake_case.
//
// Feature spec: apcore/docs/features/multi-module-discovery.md
//
// Cross-language note (D11-004, intentional language-idiomatic divergence):
// Rust has no runtime class reflection, so the public discovery surface is the
// pure helper `apcore::registry::multi_class::derive_module_ids(file_path,
// extensions_root, classes, config)` — it takes a pre-resolved
// `&[DiscoveredClass]` and does NOT read or import the file at runtime. It
// returns `Vec<String>` (the derived IDs), not `(module_id, class)` pairs.
// Registration is a separate step via `Registry::register_multi_class`.
// Python-only surfaces (`pre_approval_hook`, the runtime-import `ModuleLoadError`
// path, the `class`-bearing pair return shape) have no Rust equivalent; the
// corresponding clauses are mirrored against the closest Rust behavior and any
// genuinely-absent symbol is marked `#[ignore]` so the crate still compiles.
//
// Framework: cargo test (+ #[tokio::test] for async clauses). Tests only — src/
// is never modified.

#![allow(clippy::result_large_err)]

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};

use apcore::context::Context;
use apcore::errors::{ErrorCode, ModuleError};
use apcore::module::Module;
use apcore::registry::{
    class_name_to_segment, derive_module_ids, DiscoveredClass, DiscoveryConfig, MultiClassEntry,
    MAX_MODULE_ID_LEN,
};
use apcore::Registry;

// ---------------------------------------------------------------------------
// Canonical ID grammar (PROTOCOL_SPEC §2.7), mirrored from the feature spec.
// ---------------------------------------------------------------------------
fn canonical_id_re() -> Regex {
    Regex::new(r"^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$").unwrap()
}

// Resolve an `ErrorCode` to its exact wire string (SCREAMING_SNAKE_CASE), so a
// clause can assert the emitted `code` string exactly the way the Python suite
// asserts `exc.value.code`.
fn code_str(code: ErrorCode) -> String {
    match serde_json::to_value(code).unwrap() {
        Value::String(s) => s,
        other => panic!("ErrorCode did not serialize to a string: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Class-list builders. Rust resolves classes at compile time; the discovery
// helper takes a pre-resolved `&[DiscoveredClass]` (all qualifying here).
// ---------------------------------------------------------------------------
fn qualifying(names: &[&str]) -> Vec<DiscoveredClass> {
    // D-107: the per-class marker is the only multi-class opt-in path, so the
    // participating classes carry it. `DiscoveryConfig` is still passed by
    // these clauses but no longer gates anything.
    names
        .iter()
        .map(|n| DiscoveredClass::new(*n, true).with_multi_class(true))
        .collect()
}

fn ext_path(rel: &str) -> PathBuf {
    // Build a path under an `extensions` root so Algorithm A01 has context.
    PathBuf::from("extensions").join(rel)
}

// A minimal in-process Module used to exercise register_multi_class side
// effects (the registration step that derive_module_ids deliberately omits).
struct StubModule;

#[async_trait]
impl Module for StubModule {
    fn input_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn output_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn description(&self) -> &'static str {
        "test"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
}

fn entries(names: &[&str]) -> Vec<MultiClassEntry> {
    names
        .iter()
        .map(|n| MultiClassEntry::new(*n, Box::new(StubModule)).with_multi_class(true))
        .collect()
}

// ===========================================================================
// RETURN / single-class identity guarantee
// ===========================================================================

// clause: multi_module_discovery.discover_multi_class.return.single_class_identity
#[test]
fn discover_multi_class_return_single_class_identity() {
    let classes = qualifying(&["Addition"]);
    let ids = derive_module_ids(
        &ext_path("math/math_ops.rs"),
        "extensions",
        &classes,
        &DiscoveryConfig::with_multi_class(),
    )
    .unwrap();
    // Single class -> bare base_id, no class segment appended.
    assert_eq!(ids, vec!["math.math_ops".to_string()]);
}

// clause: multi_module_discovery.discover_multi_class.return.two_class_distinct_ids
#[test]
fn discover_multi_class_return_two_class_distinct_ids() {
    let classes = qualifying(&["Addition", "Subtraction"]);
    let mut ids = derive_module_ids(
        &ext_path("math/math_ops.rs"),
        "extensions",
        &classes,
        &DiscoveryConfig::with_multi_class(),
    )
    .unwrap();
    ids.sort();
    assert_eq!(
        ids,
        vec![
            "math.math_ops.addition".to_string(),
            "math.math_ops.subtraction".to_string()
        ]
    );
}

// clause: multi_module_discovery.discover_multi_class.return.pairs_shape
#[test]
fn discover_multi_class_return_pairs_shape() {
    // Python returns (module_id: str, cls) pairs. The Rust idiomatic surface
    // returns Vec<String> (IDs only) — class refs are pre-resolved by the
    // caller's macro expansion. Assert the actual Rust shape: every entry is a
    // non-empty String ID, one per qualifying class.
    let classes = qualifying(&["Addition", "Subtraction"]);
    let ids = derive_module_ids(
        &ext_path("math/math_ops.rs"),
        "extensions",
        &classes,
        &DiscoveryConfig::with_multi_class(),
    )
    .unwrap();
    assert_eq!(ids.len(), 2);
    for module_id in &ids {
        assert!(!module_id.is_empty());
        assert!(
            module_id.contains('.'),
            "expected base_id.segment, got {module_id}"
        );
    }
}

// ===========================================================================
// INPUT contracts
// ===========================================================================

// clause: multi_module_discovery.discover_multi_class.input.file_path.nonexistent
#[test]
#[ignore = "multi_module_discovery.discover_multi_class.input.file_path.nonexistent: missing symbol \
            ModuleLoadError-via-discovery (contract gap): Rust derive_module_ids is pure and never \
            reads/imports the file, so a nonexistent path cannot raise MODULE_LOAD_ERROR — D11-004 \
            language-idiomatic divergence (Python-only runtime import)"]
fn discover_multi_class_input_file_path_nonexistent() {
    // Rust derives IDs from a path string without touching the filesystem; a
    // "nonexistent" path yields a perfectly valid base_id. Documenting the
    // divergence: no MODULE_LOAD_ERROR is possible from the discovery surface.
    let classes = qualifying(&["Addition"]);
    let ids = derive_module_ids(
        &ext_path("math/does_not_exist.rs"),
        "extensions",
        &classes,
        &DiscoveryConfig::with_multi_class(),
    )
    .unwrap();
    assert_eq!(ids, vec!["math.does_not_exist".to_string()]);
}

// clause: multi_module_discovery.discover_multi_class.input.extensions_root.default
#[test]
fn discover_multi_class_input_extensions_root_default() {
    // Python relies on a default extensions_root="extensions". Rust has no
    // default parameter; the idiomatic equivalent is passing "extensions"
    // explicitly. Directory context beneath the root is kept.
    let classes = qualifying(&["Addition", "Subtraction"]);
    let mut ids = derive_module_ids(
        &ext_path("math/math_ops.rs"),
        "extensions",
        &classes,
        &DiscoveryConfig::with_multi_class(),
    )
    .unwrap();
    ids.sort();
    assert_eq!(
        ids,
        vec![
            "math.math_ops.addition".to_string(),
            "math.math_ops.subtraction".to_string()
        ]
    );
}

// clause: multi_module_discovery.discover_multi_class.input.extensions_root.custom
#[test]
fn discover_multi_class_input_extensions_root_custom() {
    // Custom root name drives Algorithm A01; path beneath "plugins" is used.
    let classes = qualifying(&["Addition", "Subtraction"]);
    let mut ids = derive_module_ids(
        &PathBuf::from("plugins/math/math_ops.rs"),
        "plugins",
        &classes,
        &DiscoveryConfig::with_multi_class(),
    )
    .unwrap();
    ids.sort();
    assert_eq!(
        ids,
        vec![
            "math.math_ops.addition".to_string(),
            "math.math_ops.subtraction".to_string()
        ]
    );
}

// clause: multi_module_discovery.discover_multi_class.input.pre_approval_hook.reject
#[test]
#[ignore = "multi_module_discovery.discover_multi_class.input.pre_approval_hook.reject: missing symbol \
            pre_approval_hook (contract gap): Python-only safety hook — Rust parses source via syn/\
            proc-macros and never executes code at scan time, so no hook parameter exists (D-30)"]
fn discover_multi_class_input_pre_approval_hook_reject() {
    // No Rust API to exercise; this clause is Python-only. Keep a real (trivially
    // true on the closest analog) assertion so the body is not empty.
    let classes = qualifying(&["Addition"]);
    let ids = derive_module_ids(
        &ext_path("math/math_ops.rs"),
        "extensions",
        &classes,
        &DiscoveryConfig::with_multi_class(),
    )
    .unwrap();
    assert_eq!(ids.len(), 1);
}

// clause: multi_module_discovery.discover_multi_class.input.pre_approval_hook.allow
#[test]
#[ignore = "multi_module_discovery.discover_multi_class.input.pre_approval_hook.allow: missing symbol \
            pre_approval_hook (contract gap): Python-only safety hook — absent in the Rust SDK (D-30)"]
fn discover_multi_class_input_pre_approval_hook_allow() {
    let classes = qualifying(&["Addition"]);
    let ids = derive_module_ids(
        &ext_path("math/math_ops.rs"),
        "extensions",
        &classes,
        &DiscoveryConfig::with_multi_class(),
    )
    .unwrap();
    assert_eq!(ids.len(), 1);
}

// ===========================================================================
// ERROR contracts (assert .code string exactly)
// ===========================================================================

// clause: multi_module_discovery.discover_multi_class.error.MODULE_ID_CONFLICT
#[test]
fn discover_multi_class_error_module_id_conflict() {
    // MyModule and My_Module both produce segment "my_module".
    let classes = qualifying(&["MyModule", "My_Module"]);
    let err = derive_module_ids(
        &ext_path("pkg/dup.rs"),
        "extensions",
        &classes,
        &DiscoveryConfig::with_multi_class(),
    )
    .unwrap_err();
    assert_eq!(code_str(err.code), "MODULE_ID_CONFLICT");
    // Details carry file_path, class_names, conflicting_segment per spec.
    assert_eq!(
        err.details
            .get("conflicting_segment")
            .and_then(Value::as_str),
        Some("my_module")
    );
    let class_names: Vec<String> = err
        .details
        .get("class_names")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(class_names.contains(&"MyModule".to_string()));
    assert!(class_names.contains(&"My_Module".to_string()));
    assert!(err.details.contains_key("file_path"));
}

// clause: multi_module_discovery.discover_multi_class.error.INVALID_SEGMENT
#[test]
fn discover_multi_class_error_invalid_segment() {
    // A class whose snake_case segment starts with a digit violates the grammar.
    // Need >=2 classes to enter the multi-class validation path.
    let classes = qualifying(&["Addition", "_3D"]);
    let err = derive_module_ids(
        &ext_path("pkg/bad.rs"),
        "extensions",
        &classes,
        &DiscoveryConfig::with_multi_class(),
    )
    .unwrap_err();
    assert_eq!(code_str(err.code), "INVALID_SEGMENT");
}

// clause: multi_module_discovery.discover_multi_class.error.ID_TOO_LONG
#[test]
fn discover_multi_class_error_id_too_long() {
    // Force the full module_id over MAX_MODULE_ID_LEN (192) via a very long
    // class name. Two classes are needed to enter the multi-class path.
    let long_name = format!("A{}", "b".repeat(MAX_MODULE_ID_LEN + 10));
    let classes = qualifying(&[long_name.as_str(), "Subtraction"]);
    let err = derive_module_ids(
        &ext_path("pkg/long.rs"),
        "extensions",
        &classes,
        &DiscoveryConfig::with_multi_class(),
    )
    .unwrap_err();
    assert_eq!(code_str(err.code), "ID_TOO_LONG");
}

// ===========================================================================
// PROPERTY: snake_case derivation correctness (pure helper)
// ===========================================================================

// clause: multi_module_discovery.discover_multi_class.property.snake_case_conversion
#[test]
fn discover_multi_class_property_snake_case_conversion() {
    let cases = [
        ("Addition", "addition"),
        ("MathOps", "math_ops"),
        ("HTTPSender", "http_sender"),
        ("MyModule_V2", "my_module_v2"),
    ];
    for (class_name, expected) in cases {
        assert_eq!(class_name_to_segment(class_name), expected, "{class_name}");
    }
}

// clause: multi_module_discovery.discover_multi_class.property.grammar_conformance
#[test]
fn discover_multi_class_property_grammar_conformance() {
    let classes = qualifying(&["Addition", "Subtraction", "Multiplication"]);
    let ids = derive_module_ids(
        &ext_path("math/math_ops.rs"),
        "extensions",
        &classes,
        &DiscoveryConfig::with_multi_class(),
    )
    .unwrap();
    assert_eq!(ids.len(), 3);
    let re = canonical_id_re();
    for module_id in &ids {
        assert!(re.is_match(module_id), "{module_id}");
    }
}

// clause: multi_module_discovery.discover_multi_class.property.pure
#[test]
fn discover_multi_class_property_pure() {
    // Cross-language note: Python documents pure=false because it imports the
    // file at scan time. The Rust derive_module_ids surface is genuinely PURE —
    // it never touches the filesystem; output depends only on its arguments.
    // We assert that determinism here (same inputs -> same output), which is the
    // Rust-actual behavior for this clause.
    let classes = qualifying(&["Addition", "Subtraction"]);
    let path = ext_path("math/math_ops.rs");
    let cfg = DiscoveryConfig::with_multi_class();
    let a = derive_module_ids(&path, "extensions", &classes, &cfg).unwrap();
    let b = derive_module_ids(&path, "extensions", &classes, &cfg).unwrap();
    assert_eq!(a, b);
    assert_eq!(
        a,
        vec!["math.math_ops.addition", "math.math_ops.subtraction"]
    );
}

// clause: multi_module_discovery.discover_multi_class.property.idempotent
#[test]
fn discover_multi_class_property_idempotent() {
    // idempotent: true — repeated calls with the same inputs yield the same IDs.
    let classes = qualifying(&["Addition", "Subtraction"]);
    let path = ext_path("math/math_ops.rs");
    let cfg = DiscoveryConfig::with_multi_class();
    let mut a = derive_module_ids(&path, "extensions", &classes, &cfg).unwrap();
    let mut b = derive_module_ids(&path, "extensions", &classes, &cfg).unwrap();
    let mut c = derive_module_ids(&path, "extensions", &classes, &cfg).unwrap();
    a.sort();
    b.sort();
    c.sort();
    assert_eq!(a, b);
    assert_eq!(b, c);
    assert_eq!(
        a,
        vec!["math.math_ops.addition", "math.math_ops.subtraction"]
    );
}

// clause: multi_module_discovery.discover_multi_class.property.async
#[tokio::test]
async fn discover_multi_class_property_async() {
    // async: false — the discovery helper is a plain (non-async) callable.
    // Calling it inside an async context must not require .await and must
    // resolve to a value immediately.
    let classes = qualifying(&["Addition"]);
    let ids = derive_module_ids(
        &ext_path("math/math_ops.rs"),
        "extensions",
        &classes,
        &DiscoveryConfig::with_multi_class(),
    )
    .unwrap();
    assert_eq!(ids.len(), 1);
    assert_eq!(ids, vec!["math.math_ops".to_string()]);
}

// clause: multi_module_discovery.discover_multi_class.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn discover_multi_class_property_thread_safe() {
    // thread_safe: true — >=8 concurrent discoveries must all agree and not
    // corrupt one another (each derives an independent file's IDs).
    let mut handles = Vec::new();
    for i in 0..8u32 {
        handles.push(tokio::spawn(async move {
            let classes = qualifying(&["Addition", "Subtraction"]);
            let path = ext_path(&format!("math/ops_{i}.rs"));
            let mut ids = derive_module_ids(
                &path,
                "extensions",
                &classes,
                &DiscoveryConfig::with_multi_class(),
            )
            .unwrap();
            ids.sort();
            (i, ids)
        }));
    }

    for handle in handles {
        let (i, ids) = handle.await.expect("spawned task panicked");
        assert_eq!(
            ids,
            vec![
                format!("math.ops_{i}.addition"),
                format!("math.ops_{i}.subtraction"),
            ]
        );
    }
}

// ===========================================================================
// SIDE EFFECTS — observable via public API
// ===========================================================================

// clause: multi_module_discovery.discover_multi_class.side_effect.1.discovery_does_not_register
#[test]
fn discover_multi_class_side_effect_1_discovery_does_not_register() {
    // derive_module_ids returns candidate IDs but MUST NOT itself register
    // modules into the registry (registration is a separate caller step via
    // Registry::register_multi_class).
    let registry = Registry::new();
    let before: Vec<String> = registry.module_ids();

    let classes = qualifying(&["Addition", "Subtraction"]);
    let _ids = derive_module_ids(
        &ext_path("math/math_ops.rs"),
        "extensions",
        &classes,
        &DiscoveryConfig::with_multi_class(),
    )
    .unwrap();

    let after: Vec<String> = registry.module_ids();
    assert_eq!(after, before);
    assert!(after.is_empty());
}

// clause: multi_module_discovery.discover_multi_class.side_effect.2.conflict_aborts_whole_file
#[test]
fn discover_multi_class_side_effect_2_conflict_aborts_whole_file() {
    // On conflict the whole file is aborted: no IDs/registrations escape (the
    // Err propagates before any results are returned, and register_multi_class
    // leaves the registry untouched — no partial registration).
    let registry = Registry::new();

    // Pure derivation: conflict -> Err, nothing returned.
    let derive_err = derive_module_ids(
        &ext_path("pkg/dup.rs"),
        "extensions",
        &qualifying(&["Addition", "MyModule", "My_Module"]),
        &DiscoveryConfig::with_multi_class(),
    )
    .unwrap_err();
    assert_eq!(code_str(derive_err.code), "MODULE_ID_CONFLICT");

    // Registration path: same conflict must leave the registry empty.
    let reg_err = registry
        .register_multi_class(
            &ext_path("pkg/dup.rs"),
            "extensions",
            entries(&["Addition", "MyModule", "My_Module"]),
            &DiscoveryConfig::with_multi_class(),
        )
        .unwrap_err();
    assert_eq!(code_str(reg_err.code), "MODULE_ID_CONFLICT");
    assert!(
        registry.module_ids().is_empty(),
        "conflict must not partially register any module"
    );
}

// Compile-time / silence-the-linter touch: ensure the imported trait object
// path and Arc are exercised so unused-import lints never fail the binary.
#[test]
fn discover_multi_class_register_multi_class_registers_all() {
    // Bonus coverage of the registration step (the Rust analog of Python's
    // "registry now contains the derived IDs"). Confirms register_multi_class
    // wires derive_module_ids output into the registry under the derived IDs.
    let registry = Registry::new();
    let ids = registry
        .register_multi_class(
            &ext_path("math/math_ops.rs"),
            "extensions",
            entries(&["Addition", "Subtraction"]),
            &DiscoveryConfig::with_multi_class(),
        )
        .unwrap();
    let mut got = ids.clone();
    got.sort();
    assert_eq!(
        got,
        vec![
            "math.math_ops.addition".to_string(),
            "math.math_ops.subtraction".to_string()
        ]
    );
    for module_id in &ids {
        let resolved: Option<Arc<dyn Module>> = registry.get(module_id).unwrap();
        assert!(resolved.is_some(), "expected {module_id} to be registered");
    }
}
