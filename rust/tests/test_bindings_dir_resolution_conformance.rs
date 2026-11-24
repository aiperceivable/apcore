//! Drive `bindings_dir_resolution.json` — the binding-directory resolution
//! contract (PROTOCOL_SPEC §5.12.6, aiperceivable/apcore#114).
//!
//! The shape that matters is a `bindings.dir` declared in a config FILE with no
//! explicit directory argument at the loader. Every pre-existing
//! `load_binding_dir` test in this SDK passes a directory explicitly, and that
//! is the one path which behaved identically before and after #114, so none of
//! them can tell the corrected behaviour from the status quo.
//!
//! Entry point: `BindingLoader::load_binding_dir_with_config`, the public
//! loader whose directory argument is an `Option`. It is the Rust spelling of
//! apcore-python's `load_binding_dir()` — `load_binding_dir(dir, pattern)`
//! delegates to it with `Some(dir)` and cannot express "no explicit directory",
//! which is the fixture's `explicit_dir: null`. No private resolution helper is
//! touched, per the fixture's `entry_point` clause.
//!
//! Kept out of `tests/it.rs` and declared as its own `[[test]]` binary: every
//! case mutates `APCORE_BINDINGS_*` and the process working directory, which
//! `it.rs`'s threads would share.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use apcore::bindings::BindingLoader;
use apcore::config::Config;
use apcore::APCore;
use serde_json::Value;

#[path = "conformance_env.rs"]
mod conformance_env;

use crate::conformance_env::find_fixtures_root;

const FIXTURE: &str = "bindings_dir_resolution.json";

/// The `APCORE_*` variables this fixture's precedence chain runs through.
///
/// Removed — never set to `""` — before every case that does not name them:
/// §9.2 makes an empty `APCORE_BINDINGS_DIR` a declaration of `bindings.dir`
/// with an empty value, which is an override, not a neutral state. (Since spec
/// v1.36.0 §9.2.1 requirement 5 makes this SDK discard such a value, but the
/// isolation must not depend on the behaviour under test.)
const OWNED_ENV: &[&str] = &[
    "APCORE_BINDINGS_DIR",
    "APCORE_BINDINGS_PATTERN",
    "APCORE_CONFIG_FILE",
];

/// Cases this SDK does not satisfy as the fixture states them.
///
/// EMPTY as of spec v1.36.0. It held `missing_configured_dir_is_not_an_error`,
/// where the fixture expected a configured `bindings.dir` that does not exist
/// to yield an empty result with no error while this SDK raised
/// `BINDING_FILE_INVALID`. §5.12.6 stated no outcome either way, so the
/// divergence was reported upstream rather than repaired here — and v1.36.0's
/// clause 5 settled it in this SDK's favour: a missing resolved directory
/// **MUST** raise, naming the directory, and **MUST NOT** return an empty
/// result. The case is now `missing_configured_dir_raises` and is driven green
/// in the main loop below.
///
/// Kept as an (empty) declaration rather than deleted so the cross-check below
/// keeps its shape for the next divergence.
const DIVERGENT: &[&str] = &[];

/// Serialises the cases against each other: each one sets `APCORE_BINDINGS_*`
/// and `chdir`s into its own layout, both of which are process-global.
static ENV_GUARD: Mutex<()> = Mutex::new(());

fn env_guard() -> MutexGuard<'static, ()> {
    ENV_GUARD.lock().unwrap_or_else(PoisonError::into_inner)
}

fn fixture() -> Value {
    let path = find_fixtures_root().join(FIXTURE);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("{FIXTURE} parses: {e}"))
}

fn clear_owned_env() {
    for name in OWNED_ENV {
        std::env::remove_var(name);
    }
}

/// Look up the named binding descriptor in the fixture's `binding_files` map.
///
/// Every `fs` value NAMES one of these (the fixture's
/// `fs_values_name_a_descriptor` clause). The descriptors carry DISTINCT
/// `module_id`s on purpose: a case that plants files in two candidate
/// directories can then tell which one was scanned, where a single shared id
/// made such a case pass whichever directory won.
fn descriptor<'a>(fx: &'a Value, name: &str, id: &str) -> &'a Value {
    fx["binding_files"]
        .get(name)
        .unwrap_or_else(|| panic!("[{id}] {FIXTURE} declares no binding_files entry `{name}`"))
}

/// The `module_id` a descriptor declares. Read from the descriptor, never
/// derived from the file name: the descriptor is what the loader parses.
fn descriptor_module_ids(template: &Value, name: &str) -> Vec<String> {
    template["bindings"]
        .as_array()
        .unwrap_or_else(|| panic!("binding_files.{name} declares a bindings array"))
        .iter()
        .map(|entry| {
            entry["module_id"]
                .as_str()
                .unwrap_or_else(|| panic!("binding_files.{name} entry declares a module_id"))
                .to_string()
        })
        .collect()
}

/// Write a fixture binding descriptor to `root/relative`, VERBATIM.
///
/// Nothing is translated any more. Through spec v1.35.0 the fixture spelled the
/// target field `target_id` while the canonical schema, both binding fixtures
/// and all three SDK loaders used `target`, so this driver rewrote the key.
/// v1.36.0 corrected §5.12.2 and the fixture (apcore#115): a file written from
/// the section that defines the binding-file format now loads, and a driver
/// that still rewrote the field would hide a regression in the SDK's own
/// parser.
fn write_binding_file(root: &Path, relative: &str, template: &Value) {
    let target = root.join(relative);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).expect("create binding dir");
    }
    let yaml = serde_yaml_ng::to_string(template).expect("binding descriptor serializes");
    std::fs::write(&target, yaml).expect("write binding file");
}

/// Write the case's `config_file` block as a real YAML document on disk.
///
/// The fixture's content is used verbatim except for the two §9.1 required
/// fields (`version`, `project.name`), which every case omits and which this
/// SDK enforces inside `Config::load`. They carry no path-typed value and
/// change nothing this fixture asserts; without them the document is invalid
/// and no case could reach the loader at all.
fn write_config_file(root: &Path, block: &Value) -> PathBuf {
    let relative = block["path"].as_str().expect("config_file names a path");
    let mut content = block["content"].clone();
    let object = content
        .as_object_mut()
        .expect("config_file content is an object");
    object
        .entry("version")
        .or_insert_with(|| Value::String("1.0.0".to_string()));
    object
        .entry("project")
        .or_insert_with(|| serde_json::json!({ "name": "bindings-dir-resolution-conformance" }));

    let path = root.join(relative);
    let yaml = serde_yaml_ng::to_string(&content).expect("config serializes");
    std::fs::write(&path, yaml).expect("write config file");
    path
}

/// Materialise a case's `fs` block and return `module_id -> directory`, the map
/// that turns an observed load back into the directory it was scanned from.
fn materialise_fs(root: &Path, fx: &Value, fs: &Value, id: &str) -> BTreeMap<String, String> {
    let mut origins = BTreeMap::new();
    let empty = serde_json::Map::new();
    for (relative, name) in fs.as_object().unwrap_or(&empty) {
        let name = name.as_str().unwrap_or_else(|| {
            panic!("[{id}] fs value must NAME a binding_files descriptor, got {name}")
        });
        let template = descriptor(fx, name, id);
        write_binding_file(root, relative, template);
        let dir = Path::new(relative)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        for module_id in descriptor_module_ids(template, name) {
            origins.insert(module_id, dir.clone());
        }
    }
    origins
}

/// Run one case's loader invocation and report `(module_ids, scanned_dirs)`.
///
/// `scanned_dir` is read back from the filesystem layout — which directory held
/// the files that actually loaded — never from the config value the driver
/// supplied, per the fixture's `scan_observation` clause.
fn observe_load(
    config: &Config,
    explicit_dir: Option<&str>,
    origins: &BTreeMap<String, String>,
) -> Result<(Vec<String>, Vec<String>), apcore::errors::ModuleError> {
    let mut loader = BindingLoader::new();
    let explicit = explicit_dir.map(PathBuf::from);
    loader.load_binding_dir_with_config(explicit.as_deref(), None, Some(config))?;

    let mut module_ids: Vec<String> = loader
        .list_bindings()
        .into_iter()
        .map(std::string::ToString::to_string)
        .collect();
    module_ids.sort();

    let mut scanned: Vec<String> = module_ids
        .iter()
        .map(|id| {
            origins
                .get(id)
                .unwrap_or_else(|| panic!("loaded `{id}` came from no directory this driver wrote"))
                .clone()
        })
        .collect();
    scanned.sort();
    scanned.dedup();
    Ok((module_ids, scanned))
}

fn expected_ids(expected: &serde_json::Map<String, Value>, id: &str) -> Vec<String> {
    expected["loaded_module_ids"]
        .as_array()
        .unwrap_or_else(|| panic!("[{id}] loaded_module_ids is an array"))
        .iter()
        .map(|v| v.as_str().expect("module id is a string").to_string())
        .collect()
}

/// Set up one case's layout, environment and working directory.
///
/// Returns the temp dir (kept alive by the caller), the config it loaded and
/// the `module_id -> directory` map.
///
/// `env_set_after_config_load` is applied here, AFTER `Config::load` returns
/// and before the caller invokes the loader. That ordering is the whole point
/// of the §5.12.6 clause 2 case: a loader reading the merged `Config` sees the
/// FILE value, while one reading the raw environment itself sees the variable —
/// the apcore-typescript#36 defect the clause exists to forbid.
fn prepare(
    tc: &Value,
    fx: &Value,
    id: &str,
) -> (tempfile::TempDir, Config, BTreeMap<String, String>) {
    let workspace = tempfile::tempdir().expect("tempdir");
    let root = workspace.path().to_path_buf();

    clear_owned_env();
    let empty = serde_json::Map::new();
    for (name, value) in tc["env"].as_object().unwrap_or(&empty) {
        assert!(
            OWNED_ENV.contains(&name.as_str()),
            "[{id}] case sets {name}, which this driver does not isolate"
        );
        std::env::set_var(name, value.as_str().expect("env value is a string"));
    }

    let origins = materialise_fs(&root, fx, &tc["fs"], id);
    let config_path = write_config_file(&root, &tc["config_file"]);

    // The case's paths (`./custom_bindings`, `from_argument`) are relative to
    // the layout root, which is therefore where the process has to stand.
    std::env::set_current_dir(&root).expect("chdir into the case layout");

    let config = Config::load(&config_path)
        .unwrap_or_else(|e| panic!("[{id}] the case's config file must load: {e:?}"));

    for (name, value) in tc["env_set_after_config_load"]
        .as_object()
        .unwrap_or(&empty)
    {
        assert!(
            OWNED_ENV.contains(&name.as_str()),
            "[{id}] case sets {name} after load, which this driver does not isolate"
        );
        std::env::set_var(name, value.as_str().expect("env value is a string"));
    }

    (workspace, config, origins)
}

/// Assert the `no_auto_scan_at_init` case: constructing a client MUST NOT scan.
fn assert_no_auto_scan(
    expected: &serde_json::Map<String, Value>,
    config: Config,
    origins: &BTreeMap<String, String>,
    id: &str,
) {
    // §5.12.6 clause 3. The configured directory holds a well-formed binding
    // file that would load cleanly if anything scanned it, so the observable
    // claim is that its module ID is absent from the registry.
    assert_eq!(
        expected["scanned"].as_bool(),
        Some(false),
        "[{id}] a no-loader case must expect no scan"
    );
    assert!(
        !origins.is_empty(),
        "[{id}] the case must plant a binding file, or 'nothing was scanned' is vacuous"
    );
    let client = APCore::with_options(None, None, Some(config), None);
    let registered = client.list_modules(None, None);
    assert_eq!(
        registered,
        Vec::<String>::new(),
        "[{id}] registered_module_ids (expected {})",
        expected["registered_module_ids"]
    );
    for module_id in origins.keys() {
        assert!(
            !registered.contains(module_id),
            "[{id}] client construction scanned the binding directory"
        );
    }
}

/// Assert a case whose `expected` declares an `error_code`: §5.12.6 clause 5.
fn assert_raises(
    expected: &serde_json::Map<String, Value>,
    outcome: Result<(Vec<String>, Vec<String>), apcore::errors::ModuleError>,
    id: &str,
) {
    assert!(
        !expected.contains_key("loaded_module_ids"),
        "[{id}] a raising case must not also declare loaded_module_ids"
    );
    let error = match outcome {
        Ok((ids, _)) => panic!(
            "[{id}] §5.12.6 clause 5: a resolved directory that does not exist MUST raise, \
             not return an empty result — got {ids:?}"
        ),
        Err(e) => e,
    };
    assert_eq!(
        format!("{:?}", error.code),
        "BindingFileInvalid",
        "[{id}] error_code is {}, got {:?}: {}",
        expected["error_code"],
        error.code,
        error.message
    );
    assert_eq!(
        expected["error_code"].as_str(),
        Some("BINDING_FILE_INVALID"),
        "[{id}] this driver maps only BINDING_FILE_INVALID"
    );

    if expected["error_message_names_resolved_dir"].as_bool() == Some(true) {
        let resolved = expected["scanned_dir"]
            .as_str()
            .unwrap_or_else(|| panic!("[{id}] a raising case states no scanned_dir"));
        assert!(
            error.message.contains(resolved),
            "[{id}] §5.12.6 clause 5 requires the message to NAME the resolved directory \
             `{resolved}` — an operator who mis-set `bindings.dir` otherwise cannot tell \
             which directory was attempted: {}",
            error.message
        );
    }
}

/// Assert a case that loads successfully.
fn assert_scan(
    expected: &serde_json::Map<String, Value>,
    outcome: Result<(Vec<String>, Vec<String>), apcore::errors::ModuleError>,
    id: &str,
) {
    let (module_ids, scanned) =
        outcome.unwrap_or_else(|e| panic!("[{id}] the loader must succeed: {e:?}"));

    assert_eq!(
        module_ids,
        expected_ids(expected, id),
        "[{id}] loaded_module_ids"
    );

    let want_dir = expected["scanned_dir"]
        .as_str()
        .unwrap_or_else(|| panic!("[{id}] case states no scanned_dir"));
    assert_eq!(
        scanned,
        vec![want_dir.to_string()],
        "[{id}] scanned_dir: the loader enumerated the wrong directory"
    );
}

/// Refuse to pass a case whose `expected` grew a key this driver ignores.
fn assert_expectation_keys_are_known(expected: &serde_json::Map<String, Value>, id: &str) {
    const KNOWN: &[&str] = &[
        "scanned",
        "scanned_dir",
        "loaded_module_ids",
        "registered_module_ids",
        "error_code",
        "error_message_names_resolved_dir",
    ];
    for key in expected.keys() {
        assert!(
            KNOWN.contains(&key.as_str()),
            "FAIL [{id}]: {FIXTURE} grew expectation `{key}` that this driver does not \
             assert — teach the driver, do not skip it"
        );
    }
}

#[test]
fn conformance_bindings_dir_resolution() {
    let _guard = env_guard();
    let original_cwd = std::env::current_dir().expect("cwd");
    let fx = fixture();
    let cases = fx["test_cases"].as_array().expect("test_cases is an array");
    assert_eq!(cases.len(), 13, "driver is written against all 13 cases");

    let ids: Vec<&str> = cases
        .iter()
        .map(|tc| tc["id"].as_str().expect("every case needs an id"))
        .collect();
    for divergent in DIVERGENT {
        assert!(
            ids.contains(divergent),
            "the divergence note names `{divergent}`, which {FIXTURE} no longer declares — \
             re-check whether it still applies"
        );
    }
    assert!(
        ids.contains(&"env_var_must_not_be_read_directly_at_the_loader"),
        "§5.12.6 clause 2 has exactly one case, and without it an implementation reading \
         the raw APCORE_BINDINGS_DIR passes every other case in {FIXTURE}"
    );

    for tc in cases {
        let id = tc["id"].as_str().expect("every case needs an id");
        if DIVERGENT.contains(&id) {
            continue;
        }
        let expected = tc["expected"]
            .as_object()
            .unwrap_or_else(|| panic!("[{id}] case has no expected object"));
        assert_expectation_keys_are_known(expected, id);

        let (workspace, config, origins) = prepare(tc, &fx, id);

        if tc["invoke_loader"].as_bool() == Some(false) {
            assert_no_auto_scan(expected, config, &origins, id);
        } else {
            assert_eq!(
                expected["scanned"].as_bool(),
                Some(true),
                "[{id}] this driver expects a scan for every loader case"
            );
            // `explicit_dir: null` MUST reach the loader as a genuinely absent
            // argument: a directory the driver computed itself works under BOTH
            // the pre-#114 and the corrected behaviour.
            let outcome = observe_load(&config, tc["explicit_dir"].as_str(), &origins);
            if expected.contains_key("error_code") {
                assert_raises(expected, outcome, id);
            } else {
                assert_scan(expected, outcome, id);
            }
        }

        std::env::set_current_dir(&original_cwd).expect("restore cwd");
        drop(workspace);
    }

    clear_owned_env();
    std::env::set_current_dir(&original_cwd).expect("restore cwd");
}

/// §5.12.6 clause 5 holds for all THREE provenances, not only the configured
/// one the fixture exercises.
///
/// The fixture's `missing_configured_dir_raises` resolves the directory from
/// `bindings.dir`. Clause 5 says the requirement "holds whether the directory
/// came from an explicit argument, from `bindings.dir`, or from the
/// `\"./bindings\"` default" — three provenances, one of which the fixture can
/// state. The other two are asserted here, against the same two claims: the
/// loader raises `BINDING_FILE_INVALID`, and the message NAMES the directory it
/// resolved, so an operator can tell which of the three tiers supplied it.
#[test]
fn clause_5_raises_and_names_the_directory_for_every_provenance() {
    let _guard = env_guard();
    let original_cwd = std::env::current_dir().expect("cwd");
    clear_owned_env();

    let workspace = tempfile::tempdir().expect("tempdir");
    let root = workspace.path().to_path_buf();
    let config_path = root.join("apcore.yaml");
    std::fs::write(
        &config_path,
        "version: '1.0.0'\nproject:\n  name: clause-5\nbindings:\n  dir: ./configured_missing\n",
    )
    .expect("write config");
    let bare_path = root.join("bare.yaml");
    std::fs::write(&bare_path, "version: '1.0.0'\nproject:\n  name: clause-5\n")
        .expect("write bare config");

    std::env::set_current_dir(&root).expect("chdir");
    let configured = Config::load(&config_path).expect("config loads");
    let bare = Config::load(&bare_path).expect("bare config loads");

    // (provenance, config, explicit argument, the directory the message must name)
    let cases: Vec<(&str, Option<&Config>, Option<&Path>, &str)> = vec![
        (
            "explicit argument",
            Some(&configured),
            Some(Path::new("explicit_missing")),
            "explicit_missing",
        ),
        (
            "bindings.dir",
            Some(&configured),
            None,
            "./configured_missing",
        ),
        ("the ./bindings default", Some(&bare), None, "./bindings"),
        // The same default reached with no Config at all, which is the one path
        // where `Config::get` has nothing to answer from.
        ("the ./bindings default", None, None, "./bindings"),
    ];

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        for (provenance, config, explicit, named) in cases {
            let mut loader = BindingLoader::new();
            let error = loader
                .load_binding_dir_with_config(explicit, None, config)
                .expect_err(&format!(
                    "§5.12.6 clause 5: a missing directory from {provenance} MUST raise"
                ));
            assert_eq!(
                error.code,
                apcore::errors::ErrorCode::BindingFileInvalid,
                "{provenance}: clause 5 raises the binding-file error"
            );
            assert!(
                error.message.contains(named),
                "{provenance}: the message MUST name the RESOLVED directory `{named}`, \
                 so the three tiers are distinguishable from the error alone: {}",
                error.message
            );
        }
    }));

    std::env::set_current_dir(&original_cwd).expect("restore cwd");
    clear_owned_env();
    drop(workspace);
    if let Err(payload) = outcome {
        std::panic::resume_unwind(payload);
    }
}
