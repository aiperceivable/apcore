// Spec-traced contract tests for the Config Bus feature (Rust SDK).
//
// Source spec: apcore/docs/features/config-bus.md (## Contract: blocks).
// Canonical suite mirrored: apcore-python/tests/test_config_bus_spec.py.
//
// Each test carries a verbatim clause id of the form
// `config_bus.<method>.<kind>.<detail>` in a leading `// clause:` comment so
// the cross-language test matrix (python / typescript / rust) lines up
// row-for-row. Tests exercise the PUBLIC contract only and never mutate
// production source.
//
// Rust API divergences from the Python canonical surface (asserted against
// ACTUAL Rust behavior, see DIVERGENCES in the task report):
//   * `Config::get(&str) -> Option<Value>` — no `default` argument; a missing
//     key yields `None` (Python returns the caller-supplied default).
//   * `Config::namespace(&str) -> Option<Value>` — `None` for an absent
//     namespace (Python returns an empty dict).
//   * `Config::bind::<T>(&str) -> Result<T, ModuleError>` — generic target
//     type (Python passes a dataclass type positionally).
//   * `Config::load(&Path)` + `Config::reload(&mut self)` are the file
//     entry points; `mount` takes a `MountSource` enum.
//   * Error code is an `ErrorCode` enum; the exact wire string is obtained via
//     serde SCREAMING_SNAKE_CASE serialization and asserted to match.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

use apcore::config::{Config, EnvStyle, MountSource, NamespaceRegistration, DEFAULT_MAX_DEPTH};
use apcore::errors::{ErrorCode, ModuleError};
use serde::Deserialize;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Serialize an `ErrorCode` to its canonical wire string (SCREAMING_SNAKE_CASE),
/// so we can assert the EXACT code string the way the Python suite does
/// (`exc.value.code == "CONFIG_NOT_FOUND"`).
fn code_str(code: ErrorCode) -> String {
    serde_json::to_value(code)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Unique suffix to keep the process-global namespace registry collision-free
/// across tests (the registry is never reset between tests in Rust).
fn uniq(tag: &str) -> String {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{tag}_{n}")
}

fn reg(name: &str) -> NamespaceRegistration {
    NamespaceRegistration {
        name: name.to_string(),
        env_prefix: None,
        defaults: None,
        schema: None,
        env_style: EnvStyle::Auto,
        max_depth: DEFAULT_MAX_DEPTH,
        env_map: None,
    }
}

fn reg_with_defaults(name: &str, defaults: serde_json::Value) -> NamespaceRegistration {
    NamespaceRegistration {
        defaults: Some(defaults),
        ..reg(name)
    }
}

/// Write a YAML file into a fresh temp dir; return its path. The temp dir is
/// leaked into the returned `TempDir` guard which the caller must keep alive.
fn write_yaml(dir: &tempfile::TempDir, name: &str, body: &str) -> std::path::PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, body).unwrap();
    path
}

/// Minimal namespace-mode YAML preamble (activates namespace mode via the
/// top-level `apcore:` key) with a valid executor block so `validate()` passes.
const NS_HEADER: &str = "apcore:\n  version: '0.15.0'\n";

/// Legacy-mode preamble carrying the spec-mandated required fields (A-D-03:
/// version, project.name, extensions.root, schema.root, acl.root,
/// acl.default_effect) so that `validate()` passes for legacy fixtures. Prepend
/// this to a legacy YAML body that needs to load successfully.
const LEGACY_REQUIRED: &str = "version: '0.15.0'\n\
project:\n  name: demo\n\
extensions:\n  root: ./extensions\n\
schema:\n  root: ./schemas\n\
acl:\n  root: ./acl\n  default_effect: deny\n";

/// Plugin config mirroring Python's `_PluginCfg` dataclass. `deny_unknown_fields`
/// makes an unexpected field a hard deserialization error, mirroring the
/// dataclass behavior the Python suite relies on for CONFIG_BIND_ERROR.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct PluginCfg {
    #[serde(default = "default_timeout")]
    timeout: i64,
    #[serde(default = "default_retries")]
    retries: i64,
}

fn default_timeout() -> i64 {
    5000
}
fn default_retries() -> i64 {
    3
}

// ===========================================================================
// Contract: Config::register_namespace
// ===========================================================================

// clause: config_bus.register_namespace.input.name.reserved_apcore
#[test]
fn config_bus_register_namespace_input_name_reserved_apcore() {
    let err = Config::register_namespace(reg("apcore")).unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigNamespaceReserved);
    assert_eq!(code_str(err.code), "CONFIG_NAMESPACE_RESERVED");
}

// clause: config_bus.register_namespace.input.name.reserved_config
#[test]
fn config_bus_register_namespace_input_name_reserved_config() {
    let err = Config::register_namespace(reg("_config")).unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigNamespaceReserved);
    assert_eq!(code_str(err.code), "CONFIG_NAMESPACE_RESERVED");
}

// clause: config_bus.register_namespace.error.CONFIG_NAMESPACE_DUPLICATE
#[test]
fn config_bus_register_namespace_error_config_namespace_duplicate() {
    let name = uniq("dup_spec_ns");
    Config::register_namespace(reg(&name)).unwrap();
    let err = Config::register_namespace(reg(&name)).unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigNamespaceDuplicate);
    assert_eq!(code_str(err.code), "CONFIG_NAMESPACE_DUPLICATE");
}

// clause: config_bus.register_namespace.error.CONFIG_ENV_PREFIX_CONFLICT
#[test]
fn config_bus_register_namespace_error_config_env_prefix_conflict() {
    let prefix = uniq("SHARED_PREFIX");
    let mut a = reg(&uniq("alpha_spec"));
    a.env_prefix = Some(prefix.clone());
    Config::register_namespace(a).unwrap();

    let mut b = reg(&uniq("beta_spec"));
    b.env_prefix = Some(prefix);
    let err = Config::register_namespace(b).unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigEnvPrefixConflict);
    assert_eq!(code_str(err.code), "CONFIG_ENV_PREFIX_CONFLICT");
}

// clause: config_bus.register_namespace.error.CONFIG_ENV_MAP_CONFLICT
#[test]
fn config_bus_register_namespace_error_config_env_map_conflict() {
    let env_var = uniq("SPEC_PORT");
    Config::env_map(HashMap::from([(env_var.clone(), "port".to_string())])).unwrap();

    let mut ns = reg(&uniq("gamma_spec"));
    ns.env_map = Some(HashMap::from([(env_var, "server_port".to_string())]));
    let err = Config::register_namespace(ns).unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigEnvMapConflict);
    assert_eq!(code_str(err.code), "CONFIG_ENV_MAP_CONFLICT");
}

// clause: config_bus.register_namespace.property.async
// Contract declares async: false. In Rust the call is a plain `fn` returning
// `Result<(), ModuleError>` (no `Future`); we observe it returns `Ok(())`
// synchronously, with no `.await`.
#[test]
fn config_bus_register_namespace_property_async() {
    let result: Result<(), ModuleError> = Config::register_namespace(reg(&uniq("sync_spec_ns")));
    assert!(result.is_ok());
}

// clause: config_bus.register_namespace.property.idempotent
// Contract declares idempotent: false — a second identical registration must
// error rather than silently succeed; exactly one registration remains.
#[test]
fn config_bus_register_namespace_property_idempotent() {
    let name = uniq("once_spec_ns");
    Config::register_namespace(reg(&name)).unwrap();
    let err = Config::register_namespace(reg(&name)).unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigNamespaceDuplicate);

    let count = Config::registered_namespaces()
        .iter()
        .filter(|n| n.name == name)
        .count();
    assert_eq!(count, 1);
}

// clause: config_bus.register_namespace.property.pure
// Contract declares pure: false — it mutates the process-global registry,
// observable via the public introspection API.
#[test]
fn config_bus_register_namespace_property_pure() {
    let name = uniq("pure_spec_ns");
    let before = Config::registered_namespaces()
        .iter()
        .any(|n| n.name == name);
    assert!(!before);
    Config::register_namespace(reg(&name)).unwrap();
    let after = Config::registered_namespaces()
        .iter()
        .any(|n| n.name == name);
    assert!(after);
}

// clause: config_bus.register_namespace.property.thread_safe
// Contract declares thread_safe: false ("call before any concurrent
// Config.load()"); concurrent registration is explicitly unsupported, so there
// is no safe behavior to assert (mirrors the skipped Python clause).
#[test]
#[ignore = "config_bus.register_namespace.property.thread_safe: contract declares thread_safe: false; concurrent registration unsupported (no safe behavior to assert)"]
fn config_bus_register_namespace_property_thread_safe() {}

// ===========================================================================
// Contract: Config::load
// ===========================================================================

// clause: config_bus.load.error.CONFIG_NOT_FOUND
#[test]
fn config_bus_load_error_config_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("does_not_exist.yaml");
    let err = Config::load(&missing).unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigNotFound);
    assert_eq!(code_str(err.code), "CONFIG_NOT_FOUND");
}

// clause: config_bus.load.error.CONFIG_INVALID
// Spec names ConfigInvalidError(code=CONFIG_INVALID); in this SDK that error is
// `ModuleError` carrying `ErrorCode::ConfigInvalid`, whose wire code is exactly
// CONFIG_INVALID.
#[test]
fn config_bus_load_error_config_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_yaml(&dir, "bad.yaml", "key: [unterminated\n");
    let err = Config::load(&path).unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigInvalid);
    assert_eq!(code_str(err.code), "CONFIG_INVALID");
}

// clause: config_bus.load.error.CONFIG_INVALID.non_mapping
// A syntactically valid YAML whose root is not a mapping is rejected.
#[test]
fn config_bus_load_error_config_invalid_non_mapping() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_yaml(&dir, "list.yaml", "- a\n- b\n");
    let err = Config::load(&path).unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigInvalid);
    assert_eq!(code_str(err.code), "CONFIG_INVALID");
}

// clause: config_bus.load.property.async
// Contract declares async: false — load returns a `Config`, not a `Future`.
#[test]
fn config_bus_load_property_async() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_yaml(&dir, "ok.yaml", NS_HEADER);
    let result: Result<Config, ModuleError> = Config::load(&path);
    assert!(result.is_ok());
}

// clause: config_bus.load.property.idempotent
// Loading the same file twice produces equivalent Config snapshots.
#[test]
fn config_bus_load_property_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_yaml(
        &dir,
        "idem.yaml",
        &format!("{LEGACY_REQUIRED}executor:\n  default_timeout: 30000\n  global_timeout: 60000\n"),
    );
    let first = Config::load(&path).unwrap();
    let second = Config::load(&path).unwrap();
    assert_eq!(
        first.get("executor.default_timeout"),
        second.get("executor.default_timeout")
    );
    assert_eq!(first.data(), second.data());
}

// clause: config_bus.load.property.pure
// Contract declares pure: false — the result reflects on-disk content, proving
// it reads the filesystem rather than returning a constant.
#[test]
fn config_bus_load_property_pure() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_yaml(
        &dir,
        "fs.yaml",
        &format!("{LEGACY_REQUIRED}executor:\n  default_timeout: 12345\n  global_timeout: 60000\n"),
    );
    let config = Config::load(&path).unwrap();
    assert_eq!(
        config.get("executor.default_timeout"),
        Some(serde_json::json!(12345))
    );
}

// ===========================================================================
// Contract: Config::get
// ===========================================================================

// clause: config_bus.get.input.key.empty
// D-74: an empty key is NOT an error. It resolves no value and answers like any
// other absent key — `None` here, where Python and TypeScript return the
// supplied default. Previously `#[ignore]`d in all three SDKs as a "spec/impl
// divergence", a reason written before D-74 deleted the rejection row and
// recorded that it "described behaviour no SDK has ever had". A symmetric skip
// is invisible to the skip-asymmetry guard, which needs a LIVE peer as oracle.
#[test]
fn config_bus_get_input_key_empty() {
    let config = Config::from_defaults();
    assert_eq!(config.get(""), None);
    // ...and it is the same answer as any other absent key, not a special case.
    assert_eq!(config.get("definitely.absent.key"), None);
}

// clause: config_bus.get.input.default.missing_key
// Contract: missing key returns the provided default (no error). Rust `get`
// takes no default argument and returns `Option`; a missing key is `None`, and
// idiomatic callers supply the default via `.unwrap_or(...)`.
#[test]
fn config_bus_get_input_default_missing_key() {
    let config = Config::from_defaults();
    assert_eq!(config.get("definitely.absent.key"), None);
    let sentinel = serde_json::json!("SENTINEL");
    assert_eq!(
        config
            .get("definitely.absent.key")
            .unwrap_or_else(|| sentinel.clone()),
        sentinel
    );
}

// clause: config_bus.get.property.async
// async: false — `get` is a plain `fn` returning `Option<Value>` (no Future).
#[test]
fn config_bus_get_property_async() {
    let config = Config::from_defaults();
    let result: Option<serde_json::Value> = config.get("executor.default_timeout");
    assert!(result.is_some());
}

// clause: config_bus.get.property.idempotent
// Two identical calls on the same state return identical outcomes.
#[test]
fn config_bus_get_property_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_yaml(
        &dir,
        "g.yaml",
        &format!("{LEGACY_REQUIRED}executor:\n  default_timeout: 1234\n  global_timeout: 60000\n"),
    );
    let config = Config::load(&path).unwrap();
    let first = config.get("executor.default_timeout");
    let second = config.get("executor.default_timeout");
    assert_eq!(first, second);
    assert_eq!(first, Some(serde_json::json!(1234)));
}

// clause: config_bus.get.property.pure
// get must not mutate config: the public snapshot is unchanged after calling.
#[test]
fn config_bus_get_property_pure() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_yaml(
        &dir,
        "p.yaml",
        &format!("{LEGACY_REQUIRED}executor:\n  default_timeout: 77\n  global_timeout: 60000\n"),
    );
    let config = Config::load(&path).unwrap();
    let snapshot = config.data();
    let _ = config.get("executor.default_timeout");
    let _ = config.get("missing.key");
    assert_eq!(config.data(), snapshot);
}

// clause: config_bus.get.property.thread_safe
// Contract declares thread_safe: true. Launch >=8 concurrent reads with
// distinct keys; assert no panic and every result is correct.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn config_bus_get_property_thread_safe() {
    let ns = uniq("concur_ns");
    Config::register_namespace(reg(&ns)).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut body = String::from(NS_HEADER);
    writeln!(body, "{ns}:").unwrap();
    for i in 0..8 {
        writeln!(body, "  k{i}: {i}").unwrap();
    }
    let path = write_yaml(&dir, "c.yaml", &body);
    let config = Arc::new(Config::load(&path).unwrap());

    let mut handles = Vec::new();
    for i in 0..8 {
        let cfg = Arc::clone(&config);
        let ns = ns.clone();
        handles.push(tokio::spawn(async move { cfg.get(&format!("{ns}.k{i}")) }));
    }
    for (i, h) in handles.into_iter().enumerate() {
        let value = h.await.expect("task must not panic");
        assert_eq!(value, Some(serde_json::json!(i64::try_from(i).unwrap())));
    }
}

// ===========================================================================
// Contract: Config::namespace
// ===========================================================================

// clause: config_bus.namespace.input.name.unregistered
// Unregistered/empty namespace never raises and returns an EMPTY map (never
// None) per config-bus.md §914 — cross-language parity with Python's empty dict.
#[test]
fn config_bus_namespace_input_name_unregistered() {
    let config = Config::from_defaults();
    assert!(config.namespace("never_registered_ns_xyz").is_empty());
}

// clause: config_bus.namespace.returns.merged
// Returns the namespace map merged from defaults + YAML + env overrides
// (config-bus.md §917/920): registered defaults form the base, overlaid by the
// loaded YAML values. A default-only key (`retries`) and a YAML-only key
// (`timeout`) both appear, and YAML overrides defaults on shared keys.
#[test]
fn config_bus_namespace_returns_merged() {
    let ns = uniq("nsret");
    Config::register_namespace(reg_with_defaults(
        &ns,
        serde_json::json!({"retries": 3, "timeout": 1}),
    ))
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let body = format!("{NS_HEADER}{ns}:\n  timeout: 10000\n");
    let path = write_yaml(&dir, "n.yaml", &body);
    let config = Config::load(&path).unwrap();
    let result = config.namespace(&ns);
    // YAML override wins over the default for `timeout`.
    assert_eq!(result["timeout"], serde_json::json!(10000));
    // Default-only key is merged in from the registered defaults.
    assert_eq!(result["retries"], serde_json::json!(3));
}

// clause: config_bus.namespace.property.async
// async: false — `namespace` returns a `HashMap` directly (no Future). An
// unregistered namespace yields an empty map.
#[test]
fn config_bus_namespace_property_async() {
    let config = Config::from_defaults();
    let result = config.namespace("anything");
    assert!(result.is_empty());
}

// clause: config_bus.namespace.property.pure
// namespace() returns an owned map: mutating the result must not affect config.
#[test]
fn config_bus_namespace_property_pure() {
    let ns = uniq("nspure");
    let dir = tempfile::tempdir().unwrap();
    let body = format!("{NS_HEADER}{ns}:\n  a: 1\n");
    let path = write_yaml(&dir, "np.yaml", &body);
    let config = Config::load(&path).unwrap();

    let mut result = config.namespace(&ns);
    result.insert("a".to_string(), serde_json::json!(999));
    result.insert("injected".to_string(), serde_json::json!(true));

    let fresh = config.namespace(&ns);
    assert_eq!(fresh["a"], serde_json::json!(1));
    assert!(!fresh.contains_key("injected"));
}

// clause: config_bus.namespace.property.thread_safe
// thread_safe: true — concurrent namespace() calls stay consistent.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn config_bus_namespace_property_thread_safe() {
    let ns = uniq("nsconcur");
    let dir = tempfile::tempdir().unwrap();
    let body = format!("{NS_HEADER}{ns}:\n  v: 42\n");
    let path = write_yaml(&dir, "nc.yaml", &body);
    let config = Arc::new(Config::load(&path).unwrap());

    let mut handles = Vec::new();
    for _ in 0..8 {
        let cfg = Arc::clone(&config);
        let ns = ns.clone();
        handles.push(tokio::spawn(async move { cfg.namespace(&ns) }));
    }
    for h in handles {
        let r = h.await.expect("task must not panic");
        assert_eq!(r["v"], serde_json::json!(42));
    }
}

// ===========================================================================
// Contract: Config::bind
// ===========================================================================

// clause: config_bus.bind.error.CONFIG_BIND_ERROR
// Deserialization failure (unexpected field for a deny_unknown_fields target)
// raises ConfigBindError(code=CONFIG_BIND_ERROR).
#[test]
fn config_bus_bind_error_config_bind_error() {
    let ns = uniq("bindbad");
    let dir = tempfile::tempdir().unwrap();
    let body = format!("{NS_HEADER}{ns}:\n  not_a_field: 1\n");
    let path = write_yaml(&dir, "bb.yaml", &body);
    let config = Config::load(&path).unwrap();
    let err = config.bind::<PluginCfg>(&ns).unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigBindError);
    assert_eq!(code_str(err.code), "CONFIG_BIND_ERROR");
}

// clause: config_bus.bind.returns.instance
// Successful bind returns a populated instance of the target type.
#[test]
fn config_bus_bind_returns_instance() {
    let ns = uniq("bindok");
    let dir = tempfile::tempdir().unwrap();
    let body = format!("{NS_HEADER}{ns}:\n  timeout: 8000\n  retries: 3\n");
    let path = write_yaml(&dir, "bo.yaml", &body);
    let config = Config::load(&path).unwrap();
    let result: PluginCfg = config.bind(&ns).unwrap();
    assert_eq!(result.timeout, 8000);
    assert_eq!(result.retries, 3);
}

// clause: config_bus.bind.property.async
// async: false — `bind` returns `Result<T, _>` directly (no Future).
#[test]
fn config_bus_bind_property_async() {
    let ns = uniq("bindasync");
    let dir = tempfile::tempdir().unwrap();
    // No data for the namespace -> binds into empty object, serde defaults fill.
    let path = write_yaml(&dir, "ba.yaml", NS_HEADER);
    let config = Config::load(&path).unwrap();
    let result: Result<PluginCfg, ModuleError> = config.bind(&ns);
    assert!(result.is_ok());
    let cfg = result.unwrap();
    assert_eq!(cfg.timeout, 5000);
    assert_eq!(cfg.retries, 3);
}

// clause: config_bus.bind.property.pure
// bind reads a snapshot and does not mutate config state.
#[test]
fn config_bus_bind_property_pure() {
    let ns = uniq("bindpure");
    let dir = tempfile::tempdir().unwrap();
    let path = write_yaml(&dir, "bp.yaml", NS_HEADER);
    let config = Config::load(&path).unwrap();
    let snapshot = config.data();
    let _: PluginCfg = config.bind(&ns).unwrap();
    assert_eq!(config.data(), snapshot);
}

// clause: config_bus.bind.property.thread_safe
// thread_safe: true — concurrent binds succeed and agree.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn config_bus_bind_property_thread_safe() {
    let ns = uniq("bindconcur");
    let dir = tempfile::tempdir().unwrap();
    let body = format!("{NS_HEADER}{ns}:\n  timeout: 5000\n  retries: 3\n");
    let path = write_yaml(&dir, "bc.yaml", &body);
    let config = Arc::new(Config::load(&path).unwrap());

    let mut handles = Vec::new();
    for _ in 0..8 {
        let cfg = Arc::clone(&config);
        let ns = ns.clone();
        handles.push(tokio::spawn(async move { cfg.bind::<PluginCfg>(&ns) }));
    }
    for h in handles {
        let r = h.await.expect("task must not panic").expect("bind ok");
        assert_eq!(r.timeout, 5000);
        assert_eq!(r.retries, 3);
    }
}

// ===========================================================================
// Contract: Config::mount
// ===========================================================================

// clause: config_bus.mount.input.namespace.reserved_config
// Mounting into the reserved `_config` namespace raises
// ConfigMountError(code=CONFIG_MOUNT_ERROR).
#[test]
fn config_bus_mount_input_namespace_reserved_config() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_yaml(&dir, "m.yaml", NS_HEADER);
    let mut config = Config::load(&path).unwrap();
    let err = config
        .mount(
            "_config",
            MountSource::Dict(serde_json::json!({"strict": true})),
        )
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigMountError);
    assert_eq!(code_str(err.code), "CONFIG_MOUNT_ERROR");
}

// clause: config_bus.mount.error.CONFIG_MOUNT_ERROR.missing_file
#[test]
fn config_bus_mount_error_config_mount_error_missing_file() {
    let ns = uniq("mountmiss");
    let dir = tempfile::tempdir().unwrap();
    let path = write_yaml(&dir, "mm.yaml", NS_HEADER);
    let mut config = Config::load(&path).unwrap();
    let missing = dir.path().join("nope.yaml");
    let err = config.mount(&ns, MountSource::File(missing)).unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigMountError);
    assert_eq!(code_str(err.code), "CONFIG_MOUNT_ERROR");
}

// clause: config_bus.mount.error.CONFIG_MOUNT_ERROR.not_a_mapping
#[test]
fn config_bus_mount_error_config_mount_error_not_a_mapping() {
    let ns = uniq("mountlist");
    let dir = tempfile::tempdir().unwrap();
    let bad = write_yaml(&dir, "list.yaml", "- a\n- b\n");
    let path = write_yaml(&dir, "ml.yaml", NS_HEADER);
    let mut config = Config::load(&path).unwrap();
    let err = config.mount(&ns, MountSource::File(bad)).unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigMountError);
    assert_eq!(code_str(err.code), "CONFIG_MOUNT_ERROR");
}

// clause: config_bus.mount.side_effect.1.merge_over_defaults
// Mounted dict data is merged into the namespace, observable via get().
#[test]
fn config_bus_mount_side_effect_1_merge_over_defaults() {
    let ns = uniq("mountmerge");
    let dir = tempfile::tempdir().unwrap();
    let path = write_yaml(&dir, "mg.yaml", NS_HEADER);
    let mut config = Config::load(&path).unwrap();
    config
        .mount(
            &ns,
            MountSource::Dict(serde_json::json!({"timeout": 10000})),
        )
        .unwrap();
    assert_eq!(
        config.get(&format!("{ns}.timeout")),
        Some(serde_json::json!(10000))
    );
}

// clause: config_bus.mount.property.async
// async: false — `mount` returns `Result<(), _>` directly (no Future).
#[test]
fn config_bus_mount_property_async() {
    let ns = uniq("mountasync");
    let dir = tempfile::tempdir().unwrap();
    let path = write_yaml(&dir, "ma.yaml", NS_HEADER);
    let mut config = Config::load(&path).unwrap();
    let result: Result<(), ModuleError> =
        config.mount(&ns, MountSource::Dict(serde_json::json!({"x": 1})));
    assert!(result.is_ok());
}

// clause: config_bus.mount.property.idempotent
// Contract declares idempotent: false — mounting different data twice changes
// observable state (the second mount is not a no-op).
#[test]
fn config_bus_mount_property_idempotent() {
    let ns = uniq("mountstack");
    let dir = tempfile::tempdir().unwrap();
    let path = write_yaml(&dir, "ms.yaml", NS_HEADER);
    let mut config = Config::load(&path).unwrap();

    config
        .mount(&ns, MountSource::Dict(serde_json::json!({"counter": 1})))
        .unwrap();
    let first = config.get(&format!("{ns}.counter"));
    config
        .mount(&ns, MountSource::Dict(serde_json::json!({"counter": 2})))
        .unwrap();
    let second = config.get(&format!("{ns}.counter"));

    assert_eq!(first, Some(serde_json::json!(1)));
    assert_eq!(second, Some(serde_json::json!(2)));
    assert_ne!(first, second);
}

// clause: config_bus.mount.property.pure
// Contract declares pure: false — mount mutates config state.
#[test]
fn config_bus_mount_property_pure() {
    let ns = uniq("mountmut");
    let dir = tempfile::tempdir().unwrap();
    let path = write_yaml(&dir, "mu.yaml", NS_HEADER);
    let mut config = Config::load(&path).unwrap();

    let before = config.get(&format!("{ns}.v"));
    config
        .mount(&ns, MountSource::Dict(serde_json::json!({"v": 5})))
        .unwrap();
    let after = config.get(&format!("{ns}.v"));

    assert_eq!(before, None);
    assert_eq!(after, Some(serde_json::json!(5)));
}

// clause: config_bus.mount.property.thread_safe
// Contract declares thread_safe: false ("do not call concurrently with reads");
// concurrent mutation is explicitly unsupported (no safe behavior to assert).
#[test]
#[ignore = "config_bus.mount.property.thread_safe: contract declares thread_safe: false; concurrent mutation unsupported (no safe behavior to assert)"]
fn config_bus_mount_property_thread_safe() {}

// ===========================================================================
// Contract: Config::reload
// ===========================================================================

// clause: config_bus.reload.error.CONFIG_NOT_FOUND
// Source file removed before reload -> CONFIG_NOT_FOUND.
#[test]
fn config_bus_reload_error_config_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_yaml(
        &dir,
        "r.yaml",
        &format!("{LEGACY_REQUIRED}executor:\n  default_timeout: 30000\n  global_timeout: 60000\n"),
    );
    let mut config = Config::load(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    let err = config.reload().unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigNotFound);
    assert_eq!(code_str(err.code), "CONFIG_NOT_FOUND");
}

// clause: config_bus.reload.error.CONFIG_INVALID
// Source file becomes invalid YAML before reload -> CONFIG_INVALID.
#[test]
fn config_bus_reload_error_config_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_yaml(
        &dir,
        "ri.yaml",
        &format!("{LEGACY_REQUIRED}executor:\n  default_timeout: 30000\n  global_timeout: 60000\n"),
    );
    let mut config = Config::load(&path).unwrap();
    std::fs::write(&path, "bad: [unterminated\n").unwrap();
    let err = config.reload().unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigInvalid);
    assert_eq!(code_str(err.code), "CONFIG_INVALID");
}

// clause: config_bus.reload.property.async
// async: false — `reload` returns `Result<(), _>` directly (no Future).
#[test]
fn config_bus_reload_property_async() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_yaml(
        &dir,
        "ra.yaml",
        &format!("{LEGACY_REQUIRED}executor:\n  default_timeout: 30000\n  global_timeout: 60000\n"),
    );
    let mut config = Config::load(&path).unwrap();
    let result: Result<(), ModuleError> = config.reload();
    assert!(result.is_ok());
}

// clause: config_bus.reload.property.idempotent
// Two reloads with unchanged files produce identical state.
#[test]
fn config_bus_reload_property_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_yaml(
        &dir,
        "rid.yaml",
        &format!("{LEGACY_REQUIRED}executor:\n  default_timeout: 2222\n  global_timeout: 60000\n"),
    );
    let mut config = Config::load(&path).unwrap();
    config.reload().unwrap();
    let first = config.get("executor.default_timeout");
    config.reload().unwrap();
    let second = config.get("executor.default_timeout");
    assert_eq!(first, second);
    assert_eq!(first, Some(serde_json::json!(2222)));
}

// clause: config_bus.reload.side_effect.1.reread_filesystem
// reload re-reads the YAML: a post-load edit becomes visible only after
// reload(), proving the filesystem re-read side effect and its ordering.
#[test]
fn config_bus_reload_side_effect_1_reread_filesystem() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_yaml(
        &dir,
        "rc.yaml",
        &format!("{LEGACY_REQUIRED}executor:\n  default_timeout: 1\n  global_timeout: 60000\n"),
    );
    let mut config = Config::load(&path).unwrap();
    assert_eq!(
        config.get("executor.default_timeout"),
        Some(serde_json::json!(1))
    );

    // Edit on disk; pre-reload state must be unchanged.
    std::fs::write(
        &path,
        format!("{LEGACY_REQUIRED}executor:\n  default_timeout: 999\n  global_timeout: 60000\n"),
    )
    .unwrap();
    assert_eq!(
        config.get("executor.default_timeout"),
        Some(serde_json::json!(1))
    );

    // After reload the new value is visible.
    config.reload().unwrap();
    assert_eq!(
        config.get("executor.default_timeout"),
        Some(serde_json::json!(999))
    );
}

// clause: config_bus.reload.property.pure
// Contract declares pure: false — reload mutates state by re-reading the file.
// (Cross-language note: Rust's reload() rebuilds Config from disk and does NOT
// re-apply prior in-memory mounts, unlike Python; we assert the file-driven
// mutation, which is the observable pure: false behavior the clause guarantees.)
#[test]
fn config_bus_reload_property_pure() {
    let ns = uniq("reloadmount");
    let dir = tempfile::tempdir().unwrap();
    let body = format!("{NS_HEADER}{ns}:\n  v: 1\n");
    let path = write_yaml(&dir, "rm.yaml", &body);
    let mut config = Config::load(&path).unwrap();

    // Mutate the file on disk, then reload -> state changes (pure: false).
    let body2 = format!("{NS_HEADER}{ns}:\n  v: 2\n");
    std::fs::write(&path, &body2).unwrap();
    config.reload().unwrap();
    assert_eq!(config.get(&format!("{ns}.v")), Some(serde_json::json!(2)));
}

// clause: config_bus.reload.property.thread_safe
// Contract declares thread_safe: false ("no in-flight read protection");
// concurrent reload is explicitly unsupported (no safe behavior to assert).
#[test]
#[ignore = "config_bus.reload.property.thread_safe: contract declares thread_safe: false; concurrent reload unsupported (no safe behavior to assert)"]
fn config_bus_reload_property_thread_safe() {}
