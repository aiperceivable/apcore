//! Follow-up regressions for the sync findings that were escalated as
//! NEEDS-SPEC-DECISION and have now been decided in the spec repo (`apcore`).
//!
//! Authority:
//!   - T1: `docs/features/core-executor.md` §Pipeline Hardening + the peer
//!     implementations (`apcore-python/src/apcore/pipeline.py`,
//!     `apcore-typescript/src/pipeline.ts`) — `remove`/`replace`/`insert_*`
//!     raise the dedicated step error codes, not a generic invalid-input.
//!   - T2: no cross-field `global_timeout >= default_timeout` constraint
//!     exists in PROTOCOL_SPEC §9.3 or either peer; the per-module timeout is
//!     clamped by the remaining global deadline at runtime instead.
//!   - T3/T4: `Registry.describe` / `Registry.export_schema` envelopes must be
//!     identical across SDKs so a polyglot consumer reads the same document.
//!   - T5: PROTOCOL_SPEC §9.1 / §9.3 step 1 — only `version` and
//!     `project.name` are required, evaluated against the DECLARED document.
//!   - T6: protocol-spec §5.12.5 (D-139) — every automatic mode rejects
//!     targets lacking an inferable schema rather than registering a fallback.

#![allow(clippy::pedantic, clippy::all)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use apcore::context::Context;
use apcore::errors::{ErrorCode, ModuleError};
use apcore::module::{Module, ModuleAnnotations};
use apcore::pipeline::{ExecutionStrategy, PipelineContext, Step, StepResult};
use apcore::registry::registry::{ModuleDescriptor, Registry};
use async_trait::async_trait;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// A step whose `removable` / `replaceable` flags are set per-instance.
struct FlagStep {
    name: String,
    removable: bool,
    replaceable: bool,
}

impl FlagStep {
    fn new(name: &str, removable: bool, replaceable: bool) -> Box<dyn Step> {
        Box::new(Self {
            name: name.to_string(),
            removable,
            replaceable,
        })
    }
}

#[async_trait]
impl Step for FlagStep {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        "test step"
    }
    fn removable(&self) -> bool {
        self.removable
    }
    fn replaceable(&self) -> bool {
        self.replaceable
    }
    async fn execute(&self, _ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        Ok(StepResult::continue_step())
    }
}

fn strategy() -> ExecutionStrategy {
    ExecutionStrategy::new(
        "test",
        vec![
            FlagStep::new("free", true, true),
            FlagStep::new("fixed", false, false),
        ],
    )
    .expect("strategy builds")
}

// ---------------------------------------------------------------------------
// T1 — `remove` / `replace` / `insert_*` must emit the dedicated step codes
// ---------------------------------------------------------------------------

#[test]
fn t1_insert_after_missing_anchor_raises_step_not_found() {
    let mut s = strategy();
    let err = s
        .insert_after("nope", FlagStep::new("new", true, true))
        .expect_err("missing anchor must fail");
    assert_eq!(err.code, ErrorCode::StepNotFound);
}

#[test]
fn t1_insert_before_missing_anchor_raises_step_not_found() {
    let mut s = strategy();
    let err = s
        .insert_before("nope", FlagStep::new("new", true, true))
        .expect_err("missing anchor must fail");
    assert_eq!(err.code, ErrorCode::StepNotFound);
}

#[test]
fn t1_remove_missing_step_raises_step_not_found() {
    let mut s = strategy();
    let err = s.remove("nope").expect_err("missing step must fail");
    assert_eq!(err.code, ErrorCode::StepNotFound);
}

#[test]
fn t1_remove_non_removable_raises_step_not_removable() {
    let mut s = strategy();
    let err = s.remove("fixed").expect_err("fixed step is not removable");
    assert_eq!(err.code, ErrorCode::StepNotRemovable);
}

#[test]
fn t1_replace_missing_step_raises_step_not_found() {
    let mut s = strategy();
    let err = s
        .replace("nope", FlagStep::new("new", true, true))
        .expect_err("missing step must fail");
    assert_eq!(err.code, ErrorCode::StepNotFound);
}

#[test]
fn t1_replace_non_replaceable_raises_step_not_replaceable() {
    let mut s = strategy();
    let err = s
        .replace("fixed", FlagStep::new("fixed", true, true))
        .expect_err("fixed step is not replaceable");
    assert_eq!(err.code, ErrorCode::StepNotReplaceable);
}

/// The `configure_step` vs `replace` split on the MISSING-step code is shared
/// by all three SDKs and deliberate: `configure_step` reports
/// `PIPELINE_STEP_NOT_FOUND`, `replace` reports `STEP_NOT_FOUND`.
#[test]
fn t1_configure_step_missing_keeps_pipeline_step_not_found() {
    let mut s = strategy();
    let err = s
        .configure_step("nope", FlagStep::new("new", true, true))
        .expect_err("missing step must fail");
    assert_eq!(err.code, ErrorCode::PipelineStepNotFound);
}

#[test]
fn t1_configure_step_non_replaceable_raises_step_not_replaceable() {
    let mut s = strategy();
    let err = s
        .configure_step("fixed", FlagStep::new("fixed", true, true))
        .expect_err("fixed step is not replaceable");
    assert_eq!(err.code, ErrorCode::StepNotReplaceable);
}

// ---------------------------------------------------------------------------
// T7 — every duplicate-step-name rejection must emit `STEP_NAME_DUPLICATE`
//
// Same defect class as T1: `ErrorCode::StepNameDuplicate` and
// `ModuleError::step_name_duplicate()` were declared but never constructed,
// while apcore-python (`pipeline.py:218,259,271`) and apcore-typescript
// (`pipeline.ts:235,286,300,355`) raise `StepNameDuplicateError` at every one
// of these sites.
// ---------------------------------------------------------------------------

#[test]
fn t7_constructor_duplicate_step_name_raises_step_name_duplicate() {
    let err = ExecutionStrategy::new(
        "dup",
        vec![
            FlagStep::new("same", true, true),
            FlagStep::new("same", true, true),
        ],
    )
    .expect_err("two steps sharing a name must be rejected at construction");
    assert_eq!(err.code, ErrorCode::StepNameDuplicate);
}

#[test]
fn t7_insert_after_duplicate_step_name_raises_step_name_duplicate() {
    let mut s = strategy();
    let err = s
        .insert_after("free", FlagStep::new("fixed", true, true))
        .expect_err("inserting a step whose name already exists must be rejected");
    assert_eq!(err.code, ErrorCode::StepNameDuplicate);
}

#[test]
fn t7_insert_before_duplicate_step_name_raises_step_name_duplicate() {
    let mut s = strategy();
    let err = s
        .insert_before("free", FlagStep::new("fixed", true, true))
        .expect_err("inserting a step whose name already exists must be rejected");
    assert_eq!(err.code, ErrorCode::StepNameDuplicate);
}

/// The W12 collision guard: renaming a step onto a name held by a *different*
/// step is the same duplicate-name condition, and apcore-typescript raises
/// `StepNameDuplicateError` from `configureStep` for exactly this case.
#[test]
fn t7_replace_with_colliding_name_raises_step_name_duplicate() {
    let mut s = strategy();
    let err = s
        .replace("free", FlagStep::new("fixed", true, true))
        .expect_err("renaming onto an existing step name must be rejected");
    assert_eq!(err.code, ErrorCode::StepNameDuplicate);
    // The strategy must be untouched by a rejected replace.
    assert_eq!(
        s.step_names(),
        vec!["free".to_string(), "fixed".to_string()]
    );
}

#[test]
fn t7_configure_step_with_colliding_name_raises_step_name_duplicate() {
    let mut s = strategy();
    let err = s
        .configure_step("free", FlagStep::new("fixed", true, true))
        .expect_err("renaming onto an existing step name must be rejected");
    assert_eq!(err.code, ErrorCode::StepNameDuplicate);
}

/// The guard must not over-reject: replacing in place (same name) and renaming
/// to a genuinely fresh name both stay legal.
#[test]
fn t7_replace_in_place_and_rename_to_fresh_name_are_not_duplicates() {
    let mut s = strategy();
    s.replace("free", FlagStep::new("free", true, true))
        .expect("replacing in place is legal");
    s.replace("free", FlagStep::new("renamed", true, true))
        .expect("renaming to a fresh name is legal");
    assert_eq!(
        s.step_names(),
        vec!["renamed".to_string(), "fixed".to_string()]
    );
}

// ---------------------------------------------------------------------------
// T2 — no `global_timeout >= default_timeout` cross-field check
// ---------------------------------------------------------------------------

/// `global_timeout: 10000` with `default_timeout: 30000` means "no single
/// module over 30s, whole chain under 10s". `builtin_steps.rs` clamps the
/// per-module timeout to the remaining global deadline, so the configuration
/// behaves sensibly and neither peer rejects it.
#[test]
fn t2_global_timeout_below_default_timeout_is_valid() {
    let cfg: apcore::config::Config = serde_json::from_value(json!({
        "version": "0.26.0",
        "project": { "name": "demo" },
        "executor": { "default_timeout": 30000, "global_timeout": 10000 },
    }))
    .expect("config deserializes");
    assert!(
        cfg.validate().is_ok(),
        "global_timeout < default_timeout must be accepted: {:?}",
        cfg.validate()
    );
}

// ---------------------------------------------------------------------------
// T3 — `Registry::describe` returns the peer Markdown envelope
// ---------------------------------------------------------------------------

struct DescribedModule;

#[async_trait]
impl Module for DescribedModule {
    fn description(&self) -> &'static str {
        "Add two numbers"
    }
    fn input_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn output_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
}

/// A module that overrides `Module::describe()` with a plain human-readable
/// string. Rust has no `hasattr`, so a JSON *string* return is the detectable
/// analogue of the peers' optional `describe()` override.
struct OverrideDescribeModule;

#[async_trait]
impl Module for OverrideDescribeModule {
    fn description(&self) -> &'static str {
        "ignored"
    }
    fn input_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn output_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn describe(&self) -> Value {
        Value::String("CUSTOM DESCRIBE".to_string())
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
}

fn rich_descriptor(id: &str) -> ModuleDescriptor {
    ModuleDescriptor {
        module_id: id.to_string(),
        name: None,
        description: "Add two numbers".to_string(),
        documentation: Some("Long form docs.".to_string()),
        input_schema: json!({
            "type": "object",
            "properties": {
                "a": { "type": "number", "description": "First addend" },
                "b": { "type": "number", "description": "Second addend" }
            },
            "required": ["a"]
        }),
        output_schema: json!({ "type": "object" }),
        version: "1.0.0".to_string(),
        tags: vec!["math".to_string(), "arith".to_string()],
        annotations: Some(ModuleAnnotations::default()),
        examples: vec![],
        metadata: HashMap::new(),
        display: None,
        sunset_date: None,
        dependencies: vec![],
        enabled: true,
    }
}

#[test]
fn t3_describe_missing_module_raises_module_not_found() {
    let registry = Registry::new();
    let err = registry
        .describe("not.registered")
        .expect_err("missing module must error, never a sentinel string");
    assert_eq!(err.code, ErrorCode::ModuleNotFound);
}

#[test]
fn t3_describe_builds_peer_markdown_envelope() {
    let registry = Registry::new();
    registry
        .register_internal(
            "math.add",
            Box::new(DescribedModule),
            rich_descriptor("math.add"),
        )
        .expect("registration succeeds");

    let doc = registry.describe("math.add").expect("module is registered");
    assert!(doc.starts_with("# math.add"), "heading missing: {doc}");
    assert!(
        doc.contains("Add two numbers"),
        "description missing: {doc}"
    );
    assert!(
        doc.contains("**Tags:** math, arith"),
        "tags line missing: {doc}"
    );
    assert!(doc.contains("**Parameters:**"), "parameters missing: {doc}");
    assert!(
        doc.contains("- `a` (number) (required): First addend"),
        "required parameter row missing: {doc}"
    );
    assert!(
        doc.contains("- `b` (number): Second addend"),
        "optional parameter row missing: {doc}"
    );
    assert!(
        doc.contains("**Documentation:**\nLong form docs."),
        "documentation missing: {doc}"
    );
}

#[test]
fn t3_describe_honours_module_supplied_override() {
    let registry = Registry::new();
    registry
        .register_internal(
            "math.custom",
            Box::new(OverrideDescribeModule),
            rich_descriptor("math.custom"),
        )
        .expect("registration succeeds");

    let doc = registry
        .describe("math.custom")
        .expect("module is registered");
    assert_eq!(doc, "CUSTOM DESCRIBE");
}

// ---------------------------------------------------------------------------
// T4 — `Registry::export_schema(name, strict)` returns the aligned envelope
// ---------------------------------------------------------------------------

#[test]
fn t4_export_schema_returns_aligned_envelope() {
    let registry = Registry::new();
    registry
        .register_internal(
            "math.add",
            Box::new(DescribedModule),
            rich_descriptor("math.add"),
        )
        .expect("registration succeeds");

    let exported = registry
        .export_schema("math.add", false)
        .expect("registered module exports a schema");
    assert_eq!(exported["module_id"], json!("math.add"));
    assert_eq!(exported["description"], json!("Add two numbers"));
    assert!(
        exported.get("input_schema").is_some_and(|v| !v.is_null()),
        "polyglot consumers read result['input_schema']: {exported}"
    );
    assert!(
        exported.get("output_schema").is_some_and(|v| !v.is_null()),
        "polyglot consumers read result['output_schema']: {exported}"
    );
}

#[test]
fn t4_export_schema_strict_flag_applies_strict_transform() {
    let registry = Registry::new();
    registry
        .register_internal(
            "math.add",
            Box::new(DescribedModule),
            rich_descriptor("math.add"),
        )
        .expect("registration succeeds");

    let strict = registry
        .export_schema("math.add", true)
        .expect("registered module exports a schema");
    assert_eq!(
        strict["input_schema"]["additionalProperties"],
        json!(false),
        "strict mode must disallow additionalProperties: {strict}"
    );
}

#[test]
fn t4_export_schema_returns_none_for_unregistered_module() {
    let registry = Registry::new();
    assert!(registry.export_schema("not.registered", false).is_none());
}

// ---------------------------------------------------------------------------
// T5 — only `version` and `project.name` are required (PROTOCOL_SPEC §9.1)
// ---------------------------------------------------------------------------

/// A key that carries a canonical default in `defaults.schema.json` is never
/// required. Rust used to reject a document omitting `extensions.root`,
/// `schema.root` and `acl.default_effect`; Python and TypeScript accept it.
#[test]
fn t5_config_with_only_version_and_project_name_is_valid() {
    let cfg: apcore::config::Config = serde_json::from_value(json!({
        "version": "0.26.0",
        "project": { "name": "demo" },
    }))
    .expect("config deserializes");
    assert!(
        cfg.validate().is_ok(),
        "keys with canonical defaults must not be required: {:?}",
        cfg.validate()
    );
}

#[test]
fn t5_config_missing_version_is_rejected() {
    let cfg: apcore::config::Config = serde_json::from_value(json!({
        "project": { "name": "demo" },
    }))
    .expect("config deserializes");
    let err = cfg
        .validate()
        .expect_err("version has no canonical default");
    assert_eq!(err.code, ErrorCode::ConfigInvalid);
    assert!(
        err.message.contains("version"),
        "error must name the missing field: {}",
        err.message
    );
}

#[test]
fn t5_config_missing_project_name_is_rejected() {
    let cfg: apcore::config::Config = serde_json::from_value(json!({
        "version": "0.26.0",
    }))
    .expect("config deserializes");
    let err = cfg
        .validate()
        .expect_err("project.name has no canonical default");
    assert_eq!(err.code, ErrorCode::ConfigInvalid);
    assert!(
        err.message.contains("project.name"),
        "error must name the missing field: {}",
        err.message
    );
}

// ---------------------------------------------------------------------------
// T6 — uninferable automatic schemas fail instead of registering a fallback
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CaptureWriter {
    type Writer = CaptureWriter;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn capture_logs(f: impl FnOnce()) -> String {
    let buf = CaptureWriter::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buf.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    tracing::subscriber::with_default(subscriber, f);
    let bytes = buf.0.lock().unwrap().clone();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn load_binding(body: &str) -> String {
    capture_logs(|| {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("demo.binding.yaml");
        std::fs::write(&path, body).unwrap();
        let mut loader = apcore::bindings::BindingLoader::new();
        loader.load_from_yaml(&path).expect("binding loads");
    })
}

#[test]
fn t6_implicit_auto_schema_without_typed_handler_fails_loudly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("demo.binding.yaml");
    std::fs::write(
        &path,
        r#"
spec_version: "1.0"
bindings:
  - module_id: executor.demo.implicit
    target: "demo:fn"
    description: "implicit auto_schema"
"#,
    )
    .unwrap();
    let mut loader = apcore::bindings::BindingLoader::new();
    loader.load_from_yaml(&path).unwrap();
    let handler: apcore::bindings::BindingHandler =
        Arc::new(|_, _| Box::pin(async { Ok(json!({})) }));
    let registry = Registry::new();
    let error = loader
        .register_into_with_handlers(&registry, HashMap::from([("demo:fn".to_string(), handler)]))
        .expect_err("implicit inference requires typed schemas");
    assert_eq!(error.code, ErrorCode::BindingSchemaInferenceFailed);
    assert_eq!(error.details["module_id"], "executor.demo.implicit");
    assert_eq!(error.details["file_path"], path.to_str().unwrap());
    assert!(!registry.has("executor.demo.implicit"));
}

#[test]
fn t6_explicit_schema_binding_does_not_warn() {
    let logs = load_binding(
        r#"
spec_version: "1.0"
bindings:
  - module_id: executor.demo.explicit
    target: "demo:fn"
    description: "explicit schemas"
    input_schema: { type: object }
    output_schema: { type: object }
"#,
    );
    assert!(
        !logs.contains("executor.demo.explicit"),
        "an explicit schema binding must not warn: {logs}"
    );
}

// ---------------------------------------------------------------------------
// T8 — reload() re-validates the POST-MOUNT tree (PROTOCOL_SPEC §9.11 step 5)
// ---------------------------------------------------------------------------

/// `Self::load` inside `reload()` validates the freshly-read file, but that runs
/// before the mount replay — so before this fix the post-mount tree was never
/// checked and a mount carrying an out-of-range value survived a reload.
///
/// Rust's loaders validate unconditionally (no `validate=false` opt-out to carry
/// forward, unlike apcore-python / apcore-typescript), so §9.11 step 5's second
/// clause applies: always re-validate.
#[test]
fn t8_reload_revalidates_after_mount_replay() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("apcore.yaml");
    std::fs::write(
        &path,
        "version: \"0.26.0\"\nproject:\n  name: demo\nacl:\n  default_effect: deny\n",
    )
    .expect("write config");

    let mut cfg = apcore::config::Config::load(&path).expect("initial load is valid");

    // A mount that violates the acl.default_effect enum constraint.
    cfg.mount(
        "acl",
        apcore::config::MountSource::Dict(json!({ "default_effect": "sometimes" })),
    )
    .expect("mount records the payload");

    let err = cfg
        .reload()
        .expect_err("reload must re-validate the tree the mount replay produced");
    assert_eq!(err.code, ErrorCode::ConfigInvalid);
}

/// A reload whose replayed mounts keep the tree valid still succeeds — the new
/// validation must not reject well-formed reloads.
#[test]
fn t8_reload_with_a_valid_mount_still_succeeds() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("apcore.yaml");
    std::fs::write(&path, "version: \"0.26.0\"\nproject:\n  name: demo\n").expect("write config");

    let mut cfg = apcore::config::Config::load(&path).expect("initial load is valid");
    cfg.mount(
        "my_plugin",
        apcore::config::MountSource::Dict(json!({ "timeout": 10_000 })),
    )
    .expect("mount records the payload");

    cfg.reload().expect("a valid mount must survive reload");
    assert_eq!(
        cfg.get("my_plugin.timeout"),
        Some(json!(10_000)),
        "mounted data must be replayed after reload"
    );
}

// ---------------------------------------------------------------------------
// T9 — `declared` includes env overrides (PROTOCOL_SPEC §9.1)
// ---------------------------------------------------------------------------

/// Serializes the two T9 tests below.
///
/// Both drive `APCORE_PROJECT_NAME`, which is process-global, and `cargo test`
/// runs the tests of one binary on parallel threads. Without this they race:
/// whichever removes the var can do so between the other's `set_var` and its
/// `Config::load`, and the pair failed intermittently (~2 runs in 3). Setting
/// distinct values would not help — one test's assertion is precisely that the
/// var is ABSENT, so the two cannot both hold their own view of it at once.
static T9_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Take `T9_ENV_LOCK`, ignoring poisoning: a panic in one T9 test must surface
/// as that test's own failure, not as a second, misleading failure in the other.
fn t9_env_guard() -> std::sync::MutexGuard<'static, ()> {
    T9_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// `get_declared`'s doc comment claims it covers "file, env override, `set()`,
/// or a typed struct field". §9.1 now pins exactly that, and apcore-python /
/// apcore-typescript were aligned to it — so verify the Rust implementation
/// actually matches its own doc comment rather than assuming it does.
///
/// An operator who supplies `project.name` only through the environment has
/// declared it; a container configured entirely through env is a first-class
/// deployment shape, not a degraded one.
#[test]
fn t9_env_override_counts_as_declared_and_satisfies_requiredness() {
    let _env = t9_env_guard();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("apcore-declared-env-{ts}"));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("apcore.yaml");
    // File declares `version` but NOT `project.name`.
    std::fs::write(&path, "version: \"0.26.0\"\n").unwrap();

    std::env::set_var("APCORE_PROJECT_NAME", "from-env");
    let loaded = apcore::config::Config::load(&path);
    std::env::remove_var("APCORE_PROJECT_NAME");

    let cfg = loaded.expect("an env-supplied required field must satisfy validation");
    assert_eq!(
        cfg.get_declared("project.name"),
        Some(json!("from-env")),
        "an env override must appear in the declared view, not just the resolved one"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// The counterpart: with neither file nor env supplying it, the load fails —
/// proving the test above passes because of the env var and not because the
/// check stopped firing.
#[test]
fn t9_missing_from_both_file_and_env_is_still_rejected() {
    let _env = t9_env_guard();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("apcore-declared-noenv-{ts}"));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("apcore.yaml");
    std::fs::write(&path, "version: \"0.26.0\"\n").unwrap();

    std::env::remove_var("APCORE_PROJECT_NAME");
    let err = apcore::config::Config::load(&path)
        .expect_err("project.name has no canonical default and was supplied by nobody");
    assert_eq!(err.code, ErrorCode::ConfigInvalid);

    let _ = std::fs::remove_dir_all(&dir);
}
