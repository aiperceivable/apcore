use std::io::Write;
use std::sync::{Mutex, MutexGuard, PoisonError};
use tempfile::TempDir;

/// Serialises the tests in this file against each other.
///
/// Every test here mutates process-global state — `APCORE_CONFIG_FILE`,
/// `APCORE_BINDINGS_DIR`, the current working directory — and Cargo runs the
/// tests within one binary on parallel threads that share it. This file is
/// already a separate `[[test]]` binary (see `Cargo.toml`) so it cannot
/// cross-pollute `tests/it.rs`, but that isolation stops at the process
/// boundary and said nothing about its own tests.
///
/// The observable failure was `test_declared_env_override_still_reaches_the_
/// declared_document` setting `APCORE_BINDINGS_DIR` while the two
/// `…_is_not_a_config_override_*` tests were loading a config, which then saw a
/// `bindings.dir` key their file never declared and failed on the exact-set
/// assertion. Every test takes this guard for its whole body, so the variables
/// are set and removed with no other test in flight.
///
/// Poisoning is ignored: a panicking test leaves the lock poisoned, and the
/// remaining tests should report their own results rather than a cascade of
/// unrelated `PoisonError`s.
static ENV_GUARD: Mutex<()> = Mutex::new(());

fn env_guard() -> MutexGuard<'static, ()> {
    ENV_GUARD.lock().unwrap_or_else(PoisonError::into_inner)
}

fn write_valid_yaml(dir: &TempDir, filename: &str) -> std::path::PathBuf {
    let path = dir.path().join(filename);
    let mut f = std::fs::File::create(&path).unwrap();
    // PROTOCOL_SPEC §9.1 canonical nested namespace form (v0.18.0+).
    // Includes the spec-mandated required fields (A-D-03) so legacy-mode
    // validate() passes: version, project.name, extensions.root, schema.root,
    // acl.root, acl.default_effect.
    writeln!(f, "version: '0.15.0'").unwrap();
    writeln!(f, "project:").unwrap();
    writeln!(f, "  name: demo").unwrap();
    writeln!(f, "extensions:").unwrap();
    writeln!(f, "  root: ./extensions").unwrap();
    writeln!(f, "schema:").unwrap();
    writeln!(f, "  root: ./schemas").unwrap();
    writeln!(f, "acl:").unwrap();
    writeln!(f, "  root: ./acl").unwrap();
    writeln!(f, "  default_effect: deny").unwrap();
    writeln!(f, "executor:").unwrap();
    writeln!(f, "  max_call_depth: 32").unwrap();
    writeln!(f, "  max_module_repeat: 3").unwrap();
    writeln!(f, "  default_timeout: 30000").unwrap();
    writeln!(f, "  global_timeout: 60000").unwrap();
    path
}

#[test]
fn test_discover_uses_apcore_config_file_env_var() {
    let _guard = env_guard();
    let dir = TempDir::new().unwrap();
    let config_path = write_valid_yaml(&dir, "custom.yaml");

    // Set env var, then call discover
    std::env::set_var("APCORE_CONFIG_FILE", config_path.to_str().unwrap());
    let result = apcore::Config::discover();
    std::env::remove_var("APCORE_CONFIG_FILE");

    assert!(result.is_ok(), "discover() failed: {:?}", result.err());
}

#[test]
fn test_discover_falls_back_to_defaults_when_no_file_found() {
    let _guard = env_guard();
    // Make sure env var is not set
    std::env::remove_var("APCORE_CONFIG_FILE");

    // Run from a temp directory with no config files
    let dir = TempDir::new().unwrap();
    let original = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();

    let result = apcore::Config::discover();

    std::env::set_current_dir(original).unwrap();

    assert!(
        result.is_ok(),
        "discover() should fall back to defaults, got: {:?}",
        result.err()
    );
    let config = result.unwrap();
    // Defaults: executor.max_call_depth = 32
    assert_eq!(config.executor.max_call_depth, 32);
}

// ---------------------------------------------------------------------------
// apcore#88: `$APCORE_CONFIG_FILE` selects the document, it is not in it.
// ---------------------------------------------------------------------------

/// Flatten the loaded settings tree into dot-paths.
fn flatten(value: &serde_json::Value, prefix: &str, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) if !map.is_empty() => {
            for (key, child) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten(child, &path, out);
            }
        }
        _ => out.push(prefix.to_string()),
    }
}

/// The declared document as dot-paths: the raw tree the file (plus env
/// overrides) put there. The typed `executor` / `observability` struct fields
/// are excluded by construction — they are declared by the schemas and always
/// resolve, so they say nothing about what the *document* declared.
fn declared_paths(config: &apcore::Config) -> Vec<String> {
    let mut out = Vec::new();
    for (name, value) in &config.user_namespaces {
        flatten(value, name, &mut out);
    }
    out.sort();
    out
}

fn write_minimal_yaml(dir: &TempDir, filename: &str, body: &str) -> std::path::PathBuf {
    let path = dir.path().join(filename);
    std::fs::write(&path, body).unwrap();
    path
}

/// §9.2 turns every `APCORE_*` variable into a configuration override, so the
/// file selector used to lower to the dot-path `config.file` and land in the
/// declared document — the view §9.1's required-field check runs against via
/// `get_declared`. `config.file` is declared by no schema
/// (`conformance/fixtures/config_key_governance.json`).
///
/// Asserts the **exact** declared key set, not merely that `config.file` is
/// gone: absence alone would also hold for an implementation that dropped a key
/// the file really does declare.
#[test]
fn test_config_file_env_var_is_not_a_config_override_legacy_mode() {
    let _guard = env_guard();
    let dir = TempDir::new().unwrap();
    let path = write_minimal_yaml(
        &dir,
        "custom.yaml",
        "version: '1.0.0'\nproject:\n  name: demo\n",
    );

    std::env::set_var("APCORE_CONFIG_FILE", path.to_str().unwrap());
    let config = apcore::Config::load(&path).unwrap();
    std::env::remove_var("APCORE_CONFIG_FILE");

    assert_eq!(declared_paths(&config), vec!["project.name", "version"]);
}

#[test]
fn test_config_file_env_var_is_not_a_config_override_namespace_mode() {
    let _guard = env_guard();
    let dir = TempDir::new().unwrap();
    let path = write_minimal_yaml(
        &dir,
        "ns.yaml",
        "apcore:\n  version: '1.0.0'\n  project:\n    name: demo\n",
    );

    std::env::set_var("APCORE_CONFIG_FILE", path.to_str().unwrap());
    let config = apcore::Config::load(&path).unwrap();
    std::env::remove_var("APCORE_CONFIG_FILE");

    // `version` / `project.name` appear twice: namespace-mode load mirrors the
    // LEGACY_ROOT_FIELDS to the top level for backward compatibility. That is
    // pre-existing and unrelated — what matters is that the set is closed.
    assert_eq!(
        declared_paths(&config),
        vec![
            "apcore.project.name",
            "apcore.version",
            "project.name",
            "version"
        ]
    );
}

/// The exemption is one variable wide: `bindings.dir` IS a declared key, so
/// `APCORE_BINDINGS_DIR` is §9.2 working as designed.
#[test]
fn test_declared_env_override_still_reaches_the_declared_document() {
    let _guard = env_guard();
    let dir = TempDir::new().unwrap();
    let path = write_minimal_yaml(
        &dir,
        "custom.yaml",
        "version: '1.0.0'\nproject:\n  name: demo\n",
    );

    std::env::set_var("APCORE_CONFIG_FILE", path.to_str().unwrap());
    std::env::set_var("APCORE_BINDINGS_DIR", "./generated");
    let config = apcore::Config::load(&path).unwrap();
    std::env::remove_var("APCORE_CONFIG_FILE");
    std::env::remove_var("APCORE_BINDINGS_DIR");

    assert_eq!(
        declared_paths(&config),
        vec!["bindings.dir", "project.name", "version"]
    );
}

// ---------------------------------------------------------------------------
// Config::project_root — one case per §9.14 discovery tier
// (aiperceivable/apcore#113, Option B-prime; spec §9.2.2)
// ---------------------------------------------------------------------------
//
// The tier is what selects the base, so a single "it returns a directory"
// assertion proves nothing. Each case below puts the config file and the
// process working directory in *different* places, so exactly one of the two
// candidate answers can pass. These live in this binary rather than in-file
// because they mutate CWD and `$HOME`, which only this file serialises.

/// Restores the working directory and `$HOME` on unwind as well as on success,
/// so one failing assertion cannot leave the rest of the binary running in the
/// wrong directory.
struct ProcessStateGuard {
    cwd: std::path::PathBuf,
    home: Option<String>,
}

impl ProcessStateGuard {
    fn capture() -> Self {
        Self {
            cwd: std::env::current_dir().unwrap(),
            home: std::env::var("HOME").ok(),
        }
    }
}

impl Drop for ProcessStateGuard {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.cwd).unwrap();
        match &self.home {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }
    }
}

fn canonical(path: &std::path::Path) -> std::path::PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// The tier 6 (XDG-style user-level) location under `home`, per platform.
fn tier_6_config_path(home: &std::path::Path) -> std::path::PathBuf {
    #[cfg(target_os = "macos")]
    let dir = home
        .join("Library")
        .join("Application Support")
        .join("apcore");
    #[cfg(not(target_os = "macos"))]
    let dir = home.join(".config").join("apcore");
    dir.join("config.yaml")
}

fn write_valid_yaml_at(path: &std::path::Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut f = std::fs::File::create(path).unwrap();
    writeln!(f, "version: '0.15.0'").unwrap();
    writeln!(f, "project:").unwrap();
    writeln!(f, "  name: demo").unwrap();
    writeln!(f, "extensions:").unwrap();
    writeln!(f, "  root: ./extensions").unwrap();
    writeln!(f, "schema:").unwrap();
    writeln!(f, "  root: ./schemas").unwrap();
    writeln!(f, "acl:").unwrap();
    writeln!(f, "  root: ./acl").unwrap();
    // `acl.default_effect` deliberately NOT written. It joined §9.2.4's
    // deprecation table in spec v1.47.0 (§9.1.3's first application), so
    // declaring it here would give every config in this file a §9.2.4 notice —
    // and the cases here assert on the ABSENCE of a DEPRECATION line for
    // §9.2.2's project-root notice. A test about one notice must not be made to
    // pass or fail by another.
}

#[derive(Clone, Default)]
struct CaptureWriter(std::sync::Arc<Mutex<Vec<u8>>>);

impl std::io::Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl tracing_subscriber::fmt::MakeWriter<'_> for CaptureWriter {
    type Writer = Self;
    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}

fn capture_logs<T>(f: impl FnOnce() -> T) -> (T, String) {
    let buf = CaptureWriter::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buf.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let out = tracing::subscriber::with_default(subscriber, f);
    let bytes = buf.0.lock().unwrap().clone();
    (out, String::from_utf8_lossy(&bytes).into_owned())
}

/// Tier 1 — `$APCORE_CONFIG_FILE` pointing outside CWD. The one tier where
/// file-relative and CWD-relative genuinely disagree, and the only tier B-prime
/// changes for anybody.
#[test]
fn project_root_tier_1_explicit_config_file_anchors_at_the_config_directory() {
    let _guard = env_guard();
    let _state = ProcessStateGuard::capture();

    let config_dir = TempDir::new().unwrap();
    let run_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("custom.yaml");
    write_valid_yaml_at(&config_path);

    std::env::set_current_dir(run_dir.path()).unwrap();
    std::env::set_var("APCORE_CONFIG_FILE", config_path.to_str().unwrap());
    let (config, logs) = capture_logs(|| apcore::Config::discover().unwrap());
    std::env::remove_var("APCORE_CONFIG_FILE");

    assert_eq!(
        canonical(&config.project_root()),
        canonical(config_dir.path()),
        "an explicitly pointed-at config anchors at its own directory"
    );
    assert_ne!(
        canonical(&config.project_root()),
        canonical(run_dir.path()),
        "and not at the process working directory"
    );
    assert!(
        logs.contains("DEPRECATION"),
        "this is the tier whose relative paths re-root, so it must warn: {logs}"
    );
}

/// Tiers 2-5 — a project-local config. Its directory already *is* CWD, so both
/// candidate bases agree and nothing about this tier changes.
#[test]
fn project_root_tiers_2_to_5_project_local_config_anchors_at_cwd() {
    let _guard = env_guard();
    let _state = ProcessStateGuard::capture();
    std::env::remove_var("APCORE_CONFIG_FILE");

    for name in ["project.yaml", "project.yml", "apcore.yaml", "apcore.yml"] {
        let project = TempDir::new().unwrap();
        write_valid_yaml_at(&project.path().join(name));
        std::env::set_current_dir(project.path()).unwrap();

        let config = apcore::Config::discover().unwrap();
        assert_eq!(
            canonical(&config.project_root()),
            canonical(project.path()),
            "tier for {name} must anchor at CWD"
        );
        assert_eq!(
            config.project_root(),
            std::env::current_dir().unwrap(),
            "tier for {name} must anchor at CWD"
        );
    }
}

/// Tiers 2-5 are the majority case and nothing changes for them, so the
/// deprecation notice must stay quiet. A warning fired here would be the
/// blanket warning #113 rules out.
#[test]
fn project_root_tier_2_config_emits_no_deprecation_warning() {
    let _guard = env_guard();
    let _state = ProcessStateGuard::capture();
    std::env::remove_var("APCORE_CONFIG_FILE");

    let project = TempDir::new().unwrap();
    write_valid_yaml_at(&project.path().join("apcore.yaml"));
    std::env::set_current_dir(project.path()).unwrap();

    let (config, logs) = capture_logs(|| apcore::Config::discover().unwrap());

    assert_eq!(config.project_root(), std::env::current_dir().unwrap());
    assert!(
        !logs.contains("DEPRECATION"),
        "the ordinary project-local case must not be warned about: {logs}"
    );
}

/// Tier 6 — the user-level config, and the case #113 shows is wrong today.
/// A per-user config's relative paths are per-*project*, so the base is CWD;
/// anchoring at the config file would put `./acl` inside the user's home and
/// hand every project the same ACL policy.
#[test]
fn project_root_tier_6_user_level_config_anchors_at_cwd_not_at_home() {
    let _guard = env_guard();
    let _state = ProcessStateGuard::capture();
    std::env::remove_var("APCORE_CONFIG_FILE");

    let fake_home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    let user_config = tier_6_config_path(fake_home.path());
    write_valid_yaml_at(&user_config);

    std::env::set_var("HOME", fake_home.path().to_str().unwrap());
    std::env::set_current_dir(project.path()).unwrap();

    let config = apcore::Config::discover().unwrap();

    assert_eq!(
        canonical(config.source_path().unwrap()),
        canonical(&user_config),
        "precondition: discovery must have reached tier 6"
    );
    assert_eq!(
        canonical(&config.project_root()),
        canonical(project.path()),
        "a user-level config anchors at the project being run, not at itself"
    );
    assert_ne!(
        canonical(&config.project_root()),
        canonical(user_config.parent().unwrap()),
        "anchoring at the user-level config's own directory is the #113 defect"
    );
}

/// Tier 7 — the legacy `~/.apcore/` user-level location, same rule as tier 6.
#[test]
fn project_root_tier_7_legacy_user_level_config_anchors_at_cwd() {
    let _guard = env_guard();
    let _state = ProcessStateGuard::capture();
    std::env::remove_var("APCORE_CONFIG_FILE");

    let fake_home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    let user_config = fake_home.path().join(".apcore").join("config.yaml");
    write_valid_yaml_at(&user_config);

    std::env::set_var("HOME", fake_home.path().to_str().unwrap());
    std::env::set_current_dir(project.path()).unwrap();

    let config = apcore::Config::discover().unwrap();

    assert_eq!(
        canonical(config.source_path().unwrap()),
        canonical(&user_config),
        "precondition: discovery must have reached tier 7"
    );
    assert_eq!(canonical(&config.project_root()), canonical(project.path()),);
}

/// No config file at all — nothing to anchor to but CWD.
#[test]
fn project_root_with_no_config_file_found_anchors_at_cwd() {
    let _guard = env_guard();
    let _state = ProcessStateGuard::capture();
    std::env::remove_var("APCORE_CONFIG_FILE");

    let fake_home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    std::env::set_var("HOME", fake_home.path().to_str().unwrap());
    std::env::set_current_dir(project.path()).unwrap();

    let config = apcore::Config::discover().unwrap();

    assert_eq!(
        config.source_path(),
        None,
        "precondition: no file discovered"
    );
    assert_eq!(config.project_root(), std::env::current_dir().unwrap());
}

/// Behaviour, as opposed to the accessor, MUST NOT move in this phase (§13.2).
/// `SchemaLoader::with_config` still resolves `schema.root` against CWD and
/// `ACL::discover` still resolves `acl.root` against the config file's
/// directory, including for the user-level tier where #113 calls that wrong.
#[test]
fn tier_1_resolution_behaviour_is_unchanged_by_the_project_root_accessor() {
    let _guard = env_guard();
    let _state = ProcessStateGuard::capture();

    let config_dir = TempDir::new().unwrap();
    let run_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("custom.yaml");
    write_valid_yaml_at(&config_path);

    // A same-named target directory under BOTH candidate bases, so exactly one
    // semantics can pass.
    std::fs::create_dir_all(config_dir.path().join("acl")).unwrap();
    std::fs::create_dir_all(run_dir.path().join("acl")).unwrap();
    std::fs::write(
        config_dir.path().join("acl").join("global_acl.yaml"),
        "version: '1.0'\ndefault_effect: deny\nrules: []\n",
    )
    .unwrap();
    std::fs::write(
        run_dir.path().join("acl").join("global_acl.yaml"),
        "version: '1.0'\ndefault_effect: allow\nrules: []\n",
    )
    .unwrap();

    // Likewise for `schema.root`, whose default `./schemas` exists under both.
    for (dir, marker) in [
        (config_dir.path(), "config_dir_marker"),
        (run_dir.path(), "run_dir_marker"),
    ] {
        std::fs::create_dir_all(dir.join("schemas")).unwrap();
        std::fs::write(
            dir.join("schemas").join("probe.schema.yaml"),
            format!(
                "module_id: probe\ndescription: probe schema\ninput_schema:\n  type: object\n  properties:\n    {marker}:\n      type: string\noutput_schema:\n  type: object\n"
            ),
        )
        .unwrap();
    }

    std::env::set_current_dir(run_dir.path()).unwrap();
    std::env::set_var("APCORE_CONFIG_FILE", config_path.to_str().unwrap());
    let config = apcore::Config::discover().unwrap();
    std::env::remove_var("APCORE_CONFIG_FILE");

    let mut loader = apcore::SchemaLoader::with_config(&config, None);
    loader.load("probe").unwrap();
    let raw = loader.get("probe").unwrap();
    assert!(
        raw["input_schema"]["properties"]
            .get("run_dir_marker")
            .is_some(),
        "schema.root must still resolve against the process working directory: {raw}"
    );

    let acl = apcore::ACL::discover(&config).unwrap();
    assert!(
        acl.is_some(),
        "acl.root must still resolve against the config file's directory"
    );
}

// ---------------------------------------------------------------------------
// §9.2.1 requirement 5 — an empty string is not a path
// (aiperceivable/apcore#114, spec v1.36.0)
// ---------------------------------------------------------------------------
//
// §9.2 counts a *set but empty* `APCORE_*` variable as an override, so
// `export APCORE_ACL_ROOT=` used to blank out a directory the configuration
// file correctly declared, and the `""` that replaced it resolves to the
// working directory on every filesystem API. The failure is silent, which is
// what separates it from apcore#88's phantom `config.file` key: nothing errors,
// the process just looks in the wrong place.
//
// The guard sits at the point the environment override is APPLIED
// (`Config::apply_one_env_override`), not at each consumer, so these tests read
// the merged value through `Config::get` rather than through `ACL::discover` or
// `SchemaLoader` — the property under test is that no consumer ever sees the
// empty value, present or future.

/// `APCORE_<KEY>` for a path-typed key, per §9.2's naming convention.
fn env_name_for(key: &str) -> String {
    format!(
        "APCORE_{}",
        key.trim_end_matches("[]").replace('.', "_").to_uppercase()
    )
}

/// A config file declaring exactly the given body plus the two §9.1 required
/// fields.
fn write_config_body(dir: &TempDir, body: &str) -> std::path::PathBuf {
    write_minimal_yaml(
        dir,
        "apcore.yaml",
        &format!("version: '1.0.0'\nproject:\n  name: demo\n{body}"),
    )
}

#[test]
fn an_empty_path_typed_env_var_falls_through_to_the_configuration_file() {
    let _guard = env_guard();
    let dir = TempDir::new().unwrap();
    let path = write_config_body(&dir, "acl:\n  root: ./declared_acl\n");

    std::env::set_var("APCORE_ACL_ROOT", "");
    let config = apcore::Config::load(&path).unwrap();
    std::env::remove_var("APCORE_ACL_ROOT");

    assert_eq!(
        config
            .get("acl.root")
            .and_then(|v| v.as_str().map(str::to_string)),
        Some("./declared_acl".to_string()),
        "an empty APCORE_ACL_ROOT must be discarded, leaving the directory the \
         configuration file declared — blanking it is the silent failure §9.2.1 \
         requirement 5 exists to stop"
    );
}

#[test]
fn an_empty_path_typed_env_var_falls_through_to_the_canonical_default() {
    let _guard = env_guard();
    let dir = TempDir::new().unwrap();
    // Declares no `schema` section at all, so the next tier below the discarded
    // environment value is the §9.1.1 default rather than a file value.
    let path = write_config_body(&dir, "");

    std::env::set_var("APCORE_SCHEMA_ROOT", "");
    let config = apcore::Config::load(&path).unwrap();
    std::env::remove_var("APCORE_SCHEMA_ROOT");

    assert_eq!(
        config
            .get("schema.root")
            .and_then(|v| v.as_str().map(str::to_string)),
        Some("./schemas".to_string()),
        "resolution must fall through to the NEXT tier, exactly as if the \
         variable had been unset — not stop at the empty override"
    );
}

#[test]
fn an_empty_path_typed_env_var_never_reaches_the_merged_configuration() {
    let _guard = env_guard();
    let dir = TempDir::new().unwrap();
    let path = write_config_body(&dir, "");

    for key in apcore::Config::path_typed_keys() {
        let name = env_name_for(key);
        std::env::set_var(&name, "");
        let config = apcore::Config::load(&path).unwrap();
        std::env::remove_var(&name);

        let merged = config.get(key.trim_end_matches("[]"));
        assert_ne!(
            merged,
            Some(serde_json::Value::String(String::new())),
            "`{name}` set but empty left `{key}` holding \"\" in the merged \
             configuration. \"\" is a legal relative path to every filesystem \
             API and resolves to the working directory, so this is the silent \
             re-rooting §9.2.1 requirement 5 forbids"
        );
        assert_eq!(
            merged,
            apcore::Config::default_for(key.trim_end_matches("[]")),
            "with the empty override discarded, `{key}` must read exactly as it \
             would have with `{name}` unset"
        );
    }
}

#[test]
fn an_empty_env_var_for_a_key_that_is_not_path_typed_is_still_an_override() {
    let _guard = env_guard();
    let dir = TempDir::new().unwrap();
    let path = write_config_body(&dir, "logging:\n  level: debug\n");

    // The discriminating case. §9.2.1 requirement 5 is scoped to the CLOSED
    // path-typed set, and deliberately so: "" is meaningless only where the
    // value is a path. A blanket "drop every empty override" would pass every
    // assertion above while silently changing §9.2 for keys the requirement
    // never mentioned.
    std::env::set_var("APCORE_LOGGING_LEVEL", "");
    let config = apcore::Config::load(&path).unwrap();
    std::env::remove_var("APCORE_LOGGING_LEVEL");

    assert_eq!(
        config.get("logging.level"),
        Some(serde_json::Value::String(String::new())),
        "`logging.level` is not path-typed, so §9.2's ordinary override rule \
         still applies to it unchanged"
    );
}

#[test]
fn bindings_pattern_is_not_path_typed_so_an_empty_value_still_overrides() {
    let _guard = env_guard();
    let dir = TempDir::new().unwrap();
    let path = write_config_body(&dir, "");

    // The sharpest discriminator available: `bindings.pattern` sits in the same
    // config section as `bindings.dir` and reads like a path, and §9.2.1
    // requirement 4 says outright that it is NOT one — it is a glob matched
    // against filenames within `bindings.dir`. An implementation that guarded
    // by section, or by "looks like a path", fails here.
    std::env::set_var("APCORE_BINDINGS_PATTERN", "");
    let config = apcore::Config::load(&path).unwrap();
    std::env::remove_var("APCORE_BINDINGS_PATTERN");

    assert_eq!(
        config.get("bindings.pattern"),
        Some(serde_json::Value::String(String::new())),
        "`bindings.pattern` is not in §9.2.1's path-typed set, so the empty \
         override must survive"
    );
}

#[test]
fn discarding_an_empty_path_typed_env_var_names_the_key() {
    let _guard = env_guard();
    let dir = TempDir::new().unwrap();
    let path = write_config_body(&dir, "acl:\n  root: ./declared_acl\n");

    // §9.2.1 requirement 5's MAY. An operator who exported the variable meant
    // something by it, so silently ignoring it trades one invisible behaviour
    // for another; the warning has to name the key, or it cannot be acted on.
    std::env::set_var("APCORE_ACL_ROOT", "");
    let (config, logs) = capture_logs(|| apcore::Config::load(&path).unwrap());
    std::env::remove_var("APCORE_ACL_ROOT");

    assert!(
        logs.contains("acl.root"),
        "the warning must name the discarded key: {logs}"
    );
    assert!(
        logs.contains("APCORE_ACL_ROOT"),
        "and the variable that carried it: {logs}"
    );
    assert_eq!(
        config
            .get("acl.root")
            .and_then(|v| v.as_str().map(str::to_string)),
        Some("./declared_acl".to_string())
    );
}
