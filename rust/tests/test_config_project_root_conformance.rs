//! Drive `config_project_root.json` — the project root and its narrow
//! deprecation warning (PROTOCOL_SPEC §9.2.2, aiperceivable/apcore#113).
//!
//! v1.35.0 is the DEPRECATION phase: the accessor is required, the warning is a
//! SHOULD, and no resolution behaviour changes. Nothing here asserts that a
//! relative path-typed value resolves against the project root — the
//! `v1x_current_bases_unchanged` case pins the opposite, and this SDK is
//! expected to keep `acl.root` anchored at the config file's directory and
//! `schema.root` at CWD for the whole 1.x line.
//!
//! Every tier case reaches the SDK through DISCOVERY (`Config::discover()`,
//! the no-path load), because the §9.14 tier is the input under test.
//!
//! Kept out of `tests/it.rs` and declared as its own `[[test]]` binary: each
//! case sets `$HOME`, `APCORE_*` and the process working directory, all of
//! which `it.rs`'s threads would share.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use apcore::acl::ACL;
use apcore::config::Config;
use apcore::schema::loader::SchemaLoader;
use serde_json::Value;

#[path = "conformance_env.rs"]
mod conformance_env;

use crate::conformance_env::find_fixtures_root;

const FIXTURE: &str = "config_project_root.json";

/// The `APCORE_*` variables these cases run through.
///
/// Removed — never set to `""` — before every case that does not name them.
/// §9.2 makes an empty `APCORE_CONFIG_FILE` or `APCORE_ACL_ROOT` a declaration
/// with an empty value, so blanking is an override, not isolation.
const OWNED_ENV: &[&str] = &[
    "APCORE_CONFIG_FILE",
    "APCORE_ACL_ROOT",
    "APCORE_SCHEMA_ROOT",
    "APCORE_EXTENSIONS_ROOT",
    "APCORE_BINDINGS_DIR",
];

/// Cases this SDK does not satisfy as the fixture states them.
///
/// EMPTY as of spec v1.36.0. It held `no_warning_when_all_path_values_absolute`,
/// whose config spelled only `schema.root` and `acl.root` absolutely and left
/// `extensions.root` and `bindings.dir` at their relative §9.1.1 defaults —
/// which §9.2.2's target-semantics clause 2 counts, so the case's own
/// precondition (`relative_path_typed_values_present: false`) was not
/// established by the config it wrote and the warning correctly fired. All
/// three SDKs reported it independently; v1.36.0 repaired the case to spell
/// EVERY §9.2.1 key absolutely, and it is now driven green in the main loop.
///
/// Kept as an (empty) declaration rather than deleted so the cross-check below
/// keeps its shape for the next divergence.
const DIVERGENT: &[&str] = &[];

/// Serialises the cases: each sets `$HOME`, `APCORE_*` and the working
/// directory, all process-global.
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

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

/// The directory the driver redirects `$HOME` to, named by the fixture's
/// `layout.home`.
fn home_dir(root: &Path, fx: &Value) -> PathBuf {
    root.join(
        fx["layout"]["home"]
            .as_str()
            .expect("layout names the home directory to redirect to"),
    )
}

/// This platform's §9.14 tier-6 directory under the redirected home.
///
/// §9.14 states tier 6 is platform-varying: `~/.config/apcore` on Linux,
/// `~/Library/Application Support/apcore` on macOS. The fixture therefore names
/// the TIER through a token and leaves the spelling to the driver — as first
/// published it hardcoded the POSIX path, which made every driver fail on macOS
/// while asserting nothing extra on Linux.
fn tier6_dir(root: &Path, fx: &Value) -> PathBuf {
    let home = home_dir(root, fx);
    if cfg!(target_os = "macos") {
        home.join("Library")
            .join("Application Support")
            .join("apcore")
    } else {
        home.join(".config").join("apcore")
    }
}

/// Tier 7's `~/.apcore`. Not platform-varying, but HOME-relative, so it takes
/// the same token treatment rather than a literal path under the workspace.
fn tier7_dir(root: &Path, fx: &Value) -> PathBuf {
    home_dir(root, fx).join(".apcore")
}

/// Resolve one fixture path: a `<...>` TOKEN against this platform's location,
/// anything else as a path relative to the case workspace.
///
/// An unrecognised token is a hard failure rather than a fall-through to
/// `root.join("<something>")`, which would silently create a literal directory
/// named after the token and pass.
fn layout_path(root: &Path, fx: &Value, spec: &str) -> PathBuf {
    match spec {
        "<tier6_config>" => tier6_dir(root, fx).join("config.yaml"),
        "<tier6_dir>" => tier6_dir(root, fx),
        "<tier7_config>" => tier7_dir(root, fx).join("config.yaml"),
        "<tier7_dir>" => tier7_dir(root, fx),
        other => {
            assert!(
                !(other.starts_with('<') && other.ends_with('>')),
                "FAIL: {FIXTURE} grew token `{other}` that this driver cannot resolve — \
                 teach the driver, do not join it as a literal directory name"
            );
            root.join(other)
        }
    }
}

fn canon(path: &Path) -> PathBuf {
    // macOS temp directories are reached through a /private symlink, so both
    // sides of every path comparison are normalised.
    path.canonicalize()
        .unwrap_or_else(|e| panic!("canonicalize {}: {e}", path.display()))
}

/// Write one `fs` entry verbatim, except for the two §9.1 required fields.
///
/// Every configuration document in this fixture is spelled `project: {name:
/// fixture}` with no `version`, which `Config::load` rejects. The line is
/// prepended rather than the document rewritten, so the fixture's own YAML —
/// including every path-typed value the case is about — reaches disk unchanged.
fn write_fs_entry(root: &Path, fx: &Value, relative: &str, content: &str) {
    let target = layout_path(root, fx, relative);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).expect("create fs parent");
    }
    let body = if content.contains("project:") && !content.contains("version:") {
        format!("version: '1.0.0'\n{content}\n")
    } else {
        content.to_string()
    };
    std::fs::write(&target, body).expect("write fs entry");
}

/// Create the layout's own directories.
///
/// The tier-6 and tier-7 subdirectories are deliberately NOT listed by the
/// fixture — tier 6's location is platform-varying — so they are created by
/// [`write_fs_entry`] when a case materialises `<tier6_config>` /
/// `<tier7_config>`. The redirected home itself IS listed, because every case
/// needs it to exist whether or not it holds a config file.
fn build_layout(root: &Path, fx: &Value) {
    for dir in fx["layout"]["dirs"]
        .as_array()
        .expect("layout declares dirs")
    {
        let dir = dir.as_str().expect("dir is a string");
        std::fs::create_dir_all(layout_path(root, fx, dir)).expect("create layout dir");
    }
}

// ---------------------------------------------------------------------------
// Warning capture
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
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

/// Observe the SDK's real warning channel — `tracing` — around one load, per
/// the fixture's `warning_observation` clause. Presence only: the text is not
/// normative.
fn capture_logs(f: impl FnOnce()) -> String {
    let buffer = CaptureWriter::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buffer.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    tracing::subscriber::with_default(subscriber, f);
    let bytes = buffer
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    String::from_utf8_lossy(&bytes).into_owned()
}

// ---------------------------------------------------------------------------
// Observations
// ---------------------------------------------------------------------------

/// The §9.2.2 requirement-2 predicate, computed through public API only:
/// is any path-typed value in the MERGED configuration relative?
///
/// This SDK's own helper (`Config::relative_path_typed_keys`) is private, so the
/// driver restates the predicate over the public `Config::path_typed_keys()` and
/// `Config::get`. Reading through `get` is what makes a defaulted `./schemas`
/// count, which §9.2.2's target-semantics clause 2 requires.
fn relative_path_typed_keys(config: &Config) -> Vec<String> {
    fn is_relative(value: Option<&str>) -> bool {
        value.is_some_and(|s| !s.is_empty() && Path::new(s).is_relative())
    }

    let mut hits = Vec::new();
    for key in Config::path_typed_keys() {
        let hit = match key.strip_suffix("[]") {
            Some(list_key) => matches!(
                config.get(list_key),
                Some(Value::Array(ref items)) if items.iter().any(|item| is_relative(match item {
                    Value::String(s) => Some(s.as_str()),
                    Value::Object(o) => o.get("root").and_then(Value::as_str),
                    _ => None,
                }))
            ),
            None => is_relative(config.get(key).as_ref().and_then(Value::as_str)),
        };
        if hit {
            hits.push((*key).to_string());
        }
    }
    hits
}

/// Which of the two candidate directories `ACL::discover` actually anchored at.
///
/// Read by removing the policy file under the directory the fixture names and
/// re-running discovery: the other candidate's policy file is still in place, so
/// an SDK anchored at CWD would keep finding one. The two policy documents the
/// fixture writes are byte-identical, which is why presence, not content, is the
/// observable.
fn assert_acl_root(config: &Config, root: &Path, fx: &Value, want: &str, id: &str) {
    let policy = layout_path(root, fx, want).join("global_acl.yaml");
    assert!(
        policy.is_file(),
        "[{id}] precondition: the fixture layout must put a policy at {}",
        policy.display()
    );

    let before = ACL::discover(config).expect("acl discovery must not error");
    assert!(
        before.is_some(),
        "[{id}] resolved_acl_root: no ACL was discovered at all"
    );

    std::fs::remove_file(&policy).expect("remove the policy under the expected root");
    let after = ACL::discover(config).expect("acl discovery must not error");
    std::fs::write(&policy, "default_effect: deny\nrules: []\n").expect("restore policy");

    assert!(
        after.is_none(),
        "[{id}] resolved_acl_root: discovery survived removing {}, so it anchored somewhere else",
        policy.display()
    );
}

/// Which of the two candidate directories `SchemaLoader` resolves `schema.root`
/// against.
///
/// The fixture creates both `schemas/` directories (via `.keep`) precisely so
/// exactly one semantics passes; the loader needs a file to find, so the driver
/// drops a marker schema into each and reads back which one loaded.
fn assert_schema_root(
    config: &Config,
    root: &Path,
    fx: &Value,
    want: &str,
    candidates: &[&str],
    id: &str,
) {
    for candidate in candidates {
        let dir = layout_path(root, fx, candidate);
        assert!(
            dir.is_dir(),
            "[{id}] precondition: the fixture layout must create {}",
            dir.display()
        );
        std::fs::write(
            dir.join("probe.schema.yaml"),
            format!("description: {candidate}\ntype: object\n"),
        )
        .expect("write probe schema");
    }

    let mut loader = SchemaLoader::with_config(config, None);
    let definition = loader
        .load("probe")
        .unwrap_or_else(|e| panic!("[{id}] the probe schema must load: {e:?}"));
    assert_eq!(
        definition.description, want,
        "[{id}] resolved_schema_root: schema.root resolved against the wrong base"
    );
}

// ---------------------------------------------------------------------------
// Case runner
// ---------------------------------------------------------------------------

struct CaseEnv {
    home: Option<String>,
    xdg: Option<String>,
    cwd: PathBuf,
}

impl CaseEnv {
    fn capture() -> Self {
        Self {
            home: std::env::var("HOME").ok(),
            xdg: std::env::var("XDG_CONFIG_HOME").ok(),
            cwd: std::env::current_dir().expect("cwd"),
        }
    }

    fn restore(&self) {
        match &self.home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match &self.xdg {
            Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        for name in OWNED_ENV {
            std::env::remove_var(name);
        }
        std::env::set_current_dir(&self.cwd).expect("restore cwd");
    }
}

/// Materialise one case: layout, redirected home, environment, working
/// directory. Returns the loaded `Config` and whatever the load logged.
fn run_case(fx: &Value, tc: &Value, root: &Path, id: &str) -> (Config, String) {
    build_layout(root, fx);

    // Home isolation: tiers 6-7 must never read the real user's home, and no
    // other case may pick up a config file that lives there.
    std::env::set_var("HOME", home_dir(root, fx));
    std::env::remove_var("XDG_CONFIG_HOME");
    for name in OWNED_ENV {
        std::env::remove_var(name);
    }

    let empty = serde_json::Map::new();
    for (name, value) in tc["env"].as_object().unwrap_or(&empty) {
        assert!(
            OWNED_ENV.contains(&name.as_str()),
            "[{id}] case sets {name}, which this driver does not isolate"
        );
        let value = value.as_str().expect("env value is a string");
        if name == "APCORE_CONFIG_FILE" {
            // The fixture's paths are relative to the layout root; CWD is
            // `project/`, so the selector has to be absolutised or tier 1 would
            // resolve against the wrong directory.
            std::env::set_var(name, layout_path(root, fx, value));
        } else {
            std::env::set_var(name, value);
        }
    }

    for (relative, content) in tc["fs"].as_object().unwrap_or(&empty) {
        write_fs_entry(
            root,
            fx,
            relative,
            content.as_str().expect("fs content is text"),
        );
    }

    // `cwd_must_differ`: the process CWD is `project/`, never the directory the
    // configuration file lives in for the cases that discriminate.
    let cwd = tc["cwd"]
        .as_str()
        .or_else(|| fx["layout"]["cwd"].as_str())
        .expect("a case or the layout names a cwd");
    std::env::set_current_dir(layout_path(root, fx, cwd)).expect("chdir into the layout cwd");

    if let Some(mapping) = tc.get("config_from_mapping") {
        // No discovery ran and there is no source path.
        let config: Config = serde_json::from_value(mapping.clone())
            .unwrap_or_else(|e| panic!("[{id}] the case's mapping deserializes: {e}"));
        return (config, String::new());
    }

    let mut loaded = None;
    let logs = capture_logs(|| {
        loaded = Some(
            Config::discover().unwrap_or_else(|e| panic!("[{id}] discovery must not error: {e:?}")),
        );
    });
    (loaded.expect("discovery produced a config"), logs)
}

#[allow(clippy::too_many_lines)] // one arm per expectation key, each asserted where the fixture states it
fn assert_case(fx: &Value, tc: &Value, root: &Path, id: &str) {
    let (config, logs) = run_case(fx, tc, root, id);
    let cwd = std::env::current_dir().expect("cwd");
    let expected = tc["expected"]
        .as_object()
        .unwrap_or_else(|| panic!("[{id}] case has no expected object"));

    for (field, want) in expected {
        match field.as_str() {
            "config_source_dir" => {
                let actual = config.source_path().map(|source| {
                    canon(source)
                        .parent()
                        .expect("a file has a parent")
                        .to_path_buf()
                });
                match want.as_str() {
                    Some(relative) => {
                        let expect = canon(&layout_path(root, fx, relative));
                        assert_eq!(
                            actual,
                            Some(expect),
                            "[{id}] config_source_dir (source_path={:?})",
                            config.source_path()
                        );
                    }
                    None => assert!(
                        want.is_null(),
                        "[{id}] config_source_dir must be a string or null"
                    ),
                }
                if want.is_null() {
                    assert_eq!(actual, None, "[{id}] config_source_dir must be absent");
                }
            }

            "project_root" => {
                let relative = want.as_str().expect("project_root is a string");
                assert_eq!(
                    canon(&config.project_root()),
                    canon(&layout_path(root, fx, relative)),
                    "[{id}] project_root"
                );
            }

            "project_root_equals_cwd" => {
                let same = canon(&config.project_root()) == canon(&cwd);
                assert_eq!(
                    same,
                    want.as_bool().expect("project_root_equals_cwd is a bool"),
                    "[{id}] project_root_equals_cwd (project_root={}, cwd={})",
                    config.project_root().display(),
                    cwd.display()
                );
            }

            "relative_path_typed_values_present" => {
                let hits = relative_path_typed_keys(&config);
                assert_eq!(
                    !hits.is_empty(),
                    want.as_bool()
                        .expect("relative_path_typed_values_present is a bool"),
                    "[{id}] relative_path_typed_values_present (relative keys: {hits:?})"
                );
            }

            "deprecation_warning" => {
                let warned = logs.contains("DEPRECATION");
                assert_eq!(
                    warned,
                    want.as_bool().expect("deprecation_warning is a bool"),
                    "[{id}] deprecation_warning; captured logs:\n{logs}"
                );
                if warned {
                    // The warning must be about this migration, not some other
                    // deprecation that happened to fire during the load.
                    assert!(
                        logs.contains("aiperceivable/apcore#113"),
                        "[{id}] the warning must name the issue that explains it:\n{logs}"
                    );
                }
            }

            "resolved_acl_root" => {
                assert_acl_root(
                    &config,
                    root,
                    fx,
                    want.as_str().expect("resolved_acl_root is a string"),
                    id,
                );
            }

            "resolved_schema_root" => {
                // Both candidates come from the case's own `fs` block: the
                // fixture creates `schemas/` under BOTH `project/` and
                // `elsewhere/` so exactly one semantics passes.
                assert_schema_root(
                    &config,
                    root,
                    fx,
                    want.as_str().expect("resolved_schema_root is a string"),
                    &["project/schemas", "elsewhere/schemas"],
                    id,
                );
            }

            other => panic!(
                "FAIL [{id}]: {FIXTURE} grew expectation `{other}` that this driver does \
                 not assert — teach the driver, do not skip it"
            ),
        }
    }
}

#[test]
fn conformance_config_project_root() {
    let _guard = env_guard();
    let saved = CaseEnv::capture();
    let fx = fixture();
    let cases = fx["test_cases"].as_array().expect("test_cases is an array");
    assert_eq!(cases.len(), 14, "driver is written against all 14 cases");

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

    for tc in cases {
        let id = tc["id"].as_str().expect("every case needs an id");
        if DIVERGENT.contains(&id) {
            continue;
        }
        let workspace = tempfile::tempdir().expect("tempdir");
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_case(&fx, tc, workspace.path(), id);
        }));
        saved.restore();
        drop(workspace);
        if let Err(payload) = outcome {
            std::panic::resume_unwind(payload);
        }
    }

    saved.restore();
}

/// The fixture's `tier_6_config` / `tier_7_config` tokens must resolve to the
/// location this platform's §9.14 discovery actually searches.
///
/// Without this, a driver bug that resolved the tokens to a directory nobody
/// looks in would make both tier cases pass for the wrong reason: no config
/// file is found, `Config::discover` falls back to `from_defaults()`, and
/// `project_root` is CWD — which is exactly what those cases assert. The
/// `config_source_dir` half is what stops that, so this pins the token
/// resolution itself against the SDK rather than against the driver's own
/// arithmetic.
#[test]
fn the_tier_tokens_resolve_where_discovery_looks() {
    let _guard = env_guard();
    let saved = CaseEnv::capture();
    let fx = fixture();
    let workspace = tempfile::tempdir().expect("tempdir");
    let root = workspace.path().to_path_buf();

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        build_layout(&root, &fx);
        std::env::set_var("HOME", home_dir(&root, &fx));
        std::env::remove_var("XDG_CONFIG_HOME");
        for name in OWNED_ENV {
            std::env::remove_var(name);
        }
        std::env::set_current_dir(root.join("project")).expect("chdir");

        for (config_token, dir_token) in [
            ("<tier6_config>", "<tier6_dir>"),
            ("<tier7_config>", "<tier7_dir>"),
        ] {
            let path = layout_path(&root, &fx, config_token);
            assert_eq!(
                path.parent().expect("a config file has a parent"),
                layout_path(&root, &fx, dir_token),
                "{config_token} must sit inside {dir_token}"
            );
            std::fs::create_dir_all(path.parent().expect("parent")).expect("create tier dir");
            std::fs::write(&path, "version: '1.0.0'\nproject: {name: fixture}\n")
                .expect("write tier config");

            let config = Config::discover().expect("discovery must not error");
            assert_eq!(
                config.source_path().map(canon),
                Some(canon(&path)),
                "{config_token} resolved to {}, which §9.14 discovery does not search on \
                 this platform",
                path.display()
            );
            std::fs::remove_file(&path).expect("remove tier config");
        }
    }));

    saved.restore();
    drop(workspace);
    if let Err(payload) = outcome {
        std::panic::resume_unwind(payload);
    }
}
