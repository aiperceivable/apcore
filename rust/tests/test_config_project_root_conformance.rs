//! Drive `config_project_root.json` through public Config and SDK consumers.
//! The isolated test binary owns its working directory and environment.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use apcore::{Config, Registry, SchemaLoader, ACL};
use serde_json::Value;

#[path = "conformance_env.rs"]
mod conformance_env;

const FIXTURE: &str = "config_project_root.json";

struct CaseEnv {
    cwd: PathBuf,
    variables: Vec<(OsString, Option<OsString>)>,
}

impl CaseEnv {
    fn capture() -> Self {
        let mut names = vec![OsString::from("HOME"), OsString::from("XDG_CONFIG_HOME")];
        names.extend(
            std::env::vars_os()
                .map(|(name, _)| name)
                .filter(|name| name.to_string_lossy().starts_with("APCORE_")),
        );
        Self {
            cwd: std::env::current_dir().expect("capture cwd"),
            variables: names
                .into_iter()
                .map(|name| {
                    let previous = std::env::var_os(&name);
                    (name, previous)
                })
                .collect(),
        }
    }
}

impl Drop for CaseEnv {
    fn drop(&mut self) {
        clear_apcore_environment();
        for (name, value) in &self.variables {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
        std::env::set_current_dir(&self.cwd).expect("restore cwd");
    }
}

fn clear_apcore_environment() {
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("APCORE_") {
            std::env::remove_var(name);
        }
    }
}

fn home_dir(root: &Path, fx: &Value) -> PathBuf {
    root.join(fx["layout"]["home"].as_str().expect("layout.home"))
}

fn layout_path(root: &Path, fx: &Value, spec: &str) -> PathBuf {
    let (first, tail) = spec.split_once('/').unwrap_or((spec, ""));
    let home = home_dir(root, fx);
    let tier6 = if cfg!(target_os = "macos") {
        home.join("Library/Application Support/apcore")
    } else {
        home.join(".config/apcore")
    };
    let tier7 = home.join(".apcore");
    let path = match first {
        "<tier6_dir>" => tier6,
        "<tier6_config>" => tier6.join("config.yaml"),
        "<tier7_dir>" => tier7,
        "<tier7_config>" => tier7.join("config.yaml"),
        token if token.starts_with('<') => panic!("unknown path token {token}"),
        _ => return root.join(spec),
    };
    if tail.is_empty() {
        path
    } else {
        path.join(tail)
    }
}

fn canonical_dir(path: &Path) -> PathBuf {
    path.canonicalize()
        .unwrap_or_else(|e| panic!("canonicalize {}: {e}", path.display()))
}

fn write_fs_entry(root: &Path, fx: &Value, relative: &str, content: &str) {
    let target = layout_path(root, fx, relative);
    std::fs::create_dir_all(target.parent().expect("file parent")).expect("create fs parent");
    let body = if content.contains("project:") && !content.contains("version:") {
        format!("version: '1.0.0'\n{content}\n")
    } else {
        content.to_owned()
    };
    std::fs::write(target, body).expect("write fs entry");
}

#[cfg(unix)]
fn create_symlink(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).expect("create fixture symlink");
}

#[cfg(windows)]
fn create_symlink(target: &Path, link: &Path) {
    if target.is_dir() {
        std::os::windows::fs::symlink_dir(target, link).expect("create fixture directory symlink");
    } else {
        std::os::windows::fs::symlink_file(target, link).expect("create fixture file symlink");
    }
}

fn load_case(root: &Path, fx: &Value, tc: &Value) -> Config {
    for dir in fx["layout"]["dirs"].as_array().expect("layout dirs") {
        std::fs::create_dir_all(layout_path(root, fx, dir.as_str().expect("layout dir")))
            .expect("create layout directory");
    }
    clear_apcore_environment();
    std::env::set_var("HOME", home_dir(root, fx));
    std::env::remove_var("XDG_CONFIG_HOME");
    let empty = serde_json::Map::new();
    for (name, value) in tc["env"].as_object().unwrap_or(&empty) {
        let value = value.as_str().expect("env string");
        if name == "APCORE_CONFIG_FILE" {
            std::env::set_var(name, layout_path(root, fx, value));
        } else {
            std::env::set_var(name, value);
        }
    }
    for (name, value) in tc["absolute_env"].as_object().unwrap_or(&empty) {
        std::env::set_var(
            name,
            layout_path(root, fx, value.as_str().expect("absolute env path")),
        );
    }
    for (relative, content) in tc["fs"].as_object().unwrap_or(&empty) {
        write_fs_entry(root, fx, relative, content.as_str().expect("fs content"));
    }
    for (link, target) in tc["symlinks"].as_object().unwrap_or(&empty) {
        create_symlink(
            &layout_path(root, fx, target.as_str().expect("symlink target")),
            &layout_path(root, fx, link),
        );
    }
    let cwd = tc["cwd"]
        .as_str()
        .or_else(|| fx["layout"]["cwd"].as_str())
        .expect("load cwd");
    std::env::set_current_dir(layout_path(root, fx, cwd)).expect("change load cwd");
    if let Some(data) = tc.get("config_from_mapping") {
        return serde_json::from_value(data.clone()).expect("deserialize in-memory config");
    }
    if let Some(path) = tc["config_path"].as_str() {
        return Config::load(&layout_path(root, fx, path)).expect("load explicit config");
    }
    Config::discover().expect("discover config")
}

fn assert_resolved_paths(config: &Config, root: &Path, fx: &Value, want: &Value, id: &str) {
    for (key, raw_expected) in want.as_object().expect("resolved_config_paths mapping") {
        if let Some(list) = key.strip_suffix("[]") {
            let raw = config.get(list).expect("configured path list");
            let roots: Vec<PathBuf> = raw
                .as_array()
                .expect("path list")
                .iter()
                .map(|item| {
                    let path = item
                        .as_str()
                        .or_else(|| item["root"].as_str())
                        .expect("root value");
                    config.resolve_path(path)
                })
                .collect();
            let expected: Vec<PathBuf> = raw_expected
                .as_array()
                .expect("expected paths")
                .iter()
                .map(|entry| layout_path(root, fx, entry.as_str().expect("expected path")))
                .collect();
            assert_eq!(roots, expected, "[{id}] {key}");
            if list == "extensions.roots" {
                let registry = Registry::new();
                registry.set_extension_roots_from_config(config);
                let consumed: Vec<PathBuf> = registry
                    .extension_roots()
                    .into_iter()
                    .map(PathBuf::from)
                    .collect();
                assert_eq!(consumed, expected, "[{id}] registry path list");
            }
        } else {
            let expected = layout_path(root, fx, raw_expected.as_str().expect("expected path"));
            let raw = config.get(key).expect("configured path");
            let raw = Path::new(raw.as_str().expect("path string"));
            let actual = config.resolve_path(raw);
            if raw.is_absolute() {
                assert_eq!(
                    actual.as_os_str(),
                    expected.as_os_str(),
                    "[{id}] {key} spelling"
                );
                assert_eq!(
                    raw.as_os_str(),
                    expected.as_os_str(),
                    "[{id}] {key} raw spelling"
                );
            } else {
                assert_eq!(actual, expected, "[{id}] {key}");
            }
            if key == "extensions.root" {
                let registry = Registry::new();
                registry.set_extension_roots_from_config(config);
                let consumed: Vec<PathBuf> = registry
                    .extension_roots()
                    .into_iter()
                    .map(PathBuf::from)
                    .collect();
                assert_eq!(consumed, vec![expected], "[{id}] registry root");
            }
        }
    }
}

#[test]
fn conformance_config_project_root() {
    let path = conformance_env::find_fixtures_root().join(FIXTURE);
    let fixture: Value =
        serde_json::from_slice(&std::fs::read(path).expect("read fixture")).expect("parse fixture");
    for tc in fixture["test_cases"].as_array().expect("test cases") {
        let id = tc["id"].as_str().expect("case id");
        let workspace = tempfile::tempdir().expect("tempdir");
        let root = canonical_dir(workspace.path());
        let _saved = CaseEnv::capture();
        let config = load_case(&root, &fixture, tc);
        let load_cwd = std::env::current_dir().expect("load cwd");
        let source_dir = config
            .source_path()
            .map(|source| canonical_dir(load_cwd.join(source).parent().expect("source directory")));
        if let Some(dir) = tc["consume_from"].as_str() {
            std::env::set_current_dir(layout_path(&root, &fixture, dir))
                .expect("change consumption cwd");
        }
        for (field, want) in tc["expected"].as_object().expect("expected") {
            match field.as_str() {
                "config_source_dir" => {
                    let expected = want
                        .as_str()
                        .map(|path| canonical_dir(&layout_path(&root, &fixture, path)));
                    assert_eq!(source_dir, expected, "[{id}] source dir");
                }
                "project_root" => assert_eq!(
                    canonical_dir(&config.project_root()),
                    canonical_dir(&layout_path(&root, &fixture, want.as_str().expect("root"))),
                    "[{id}] project root"
                ),
                "project_root_equals_cwd" => assert_eq!(
                    canonical_dir(&config.project_root()) == canonical_dir(&load_cwd),
                    want.as_bool().expect("root equality"),
                    "[{id}] project root compared to load cwd"
                ),
                "resolved_config_paths" => {
                    assert_resolved_paths(&config, &root, &fixture, want, id);
                }
                "resolved_acl_root" => {
                    let acl = ACL::discover(&config)
                        .expect("ACL discovery")
                        .expect("expected ACL");
                    assert_eq!(
                        acl.rules()[0].description.as_deref(),
                        want.as_str(),
                        "[{id}] ACL root"
                    );
                }
                "resolved_schema_root" => {
                    let mut loader = SchemaLoader::with_config(&config, None);
                    let definition = loader.load("probe").expect("load selected probe");
                    assert_eq!(
                        Some(definition.description.as_str()),
                        want.as_str(),
                        "[{id}] schema root"
                    );
                }
                "config_path_file_readable" => {
                    for (key, readable) in want.as_object().expect("file readability mapping") {
                        let value = config.get(key).expect("configured file path");
                        let path = config.resolve_path(value.as_str().expect("path string"));
                        assert_eq!(
                            std::fs::read(path).is_ok(),
                            readable.as_bool().expect("readability flag"),
                            "[{id}] {key} file readability"
                        );
                    }
                }
                other => panic!("[{id}] unhandled expected field {other}"),
            }
        }
    }
}
