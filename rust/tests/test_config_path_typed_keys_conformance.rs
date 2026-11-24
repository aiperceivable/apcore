//! Drive `config_path_typed_keys.json` — the closed set of path-typed
//! configuration keys (PROTOCOL_SPEC §9.2.1, aiperceivable/apcore#113).
//!
//! `src/config.rs` already unit-tests `Config::path_typed_keys()` against a
//! hand-written literal. That literal is the thing this driver replaces: a
//! key added to `schemas/apcore-config.schema.json` upstream reaches the Rust
//! SDK here on the next test run, where a copied list would keep asserting the
//! set as it stood the day it was copied.
//!
//! Scope, per the fixture: WHICH keys are path-typed and that the SDK exposes
//! the set. Nothing here asserts what a *relative* value resolves against —
//! that base is §9.2.2's subject and is driven by
//! `test_config_project_root_conformance.rs`.
//!
//! Kept out of `tests/it.rs` and declared as its own `[[test]]` binary because
//! `no_scalar_env_encoding_for_roots` mutates `APCORE_EXTENSIONS_ROOTS`, which
//! is process-global state that `it.rs`'s threads would share.

use std::collections::BTreeSet;

use apcore::config::Config;
use apcore::registry::Registry;
use serde_json::Value;

#[path = "conformance_env.rs"]
mod conformance_env;

use crate::conformance_env::{find_fixtures_root, find_schemas_root};

const FIXTURE: &str = "config_path_typed_keys.json";

fn fixture() -> Value {
    let path = find_fixtures_root().join(FIXTURE);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("{FIXTURE} parses: {e}"))
}

/// Read a canonical schema named by the fixture's `canonical_sources`.
fn schema(relative: &str) -> Value {
    // `find_schemas_root()` locates `<spec repo>/schemas/`, so a source path of
    // `schemas/foo.json` is joined by its file name.
    let file = std::path::Path::new(relative)
        .file_name()
        .unwrap_or_else(|| panic!("{relative} names a file"));
    let path = find_schemas_root().join(file);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("{relative} parses: {e}"))
}

/// Project `"x-apcore-path": true` markers out of a JSON Schema into the dotted
/// key spelling §9.2.1 uses.
///
/// `frozen` implements the fixture's `roots_element_form` rule: once the walk
/// descends into an array's `items`, every marker beneath it is reported as the
/// ARRAY's key (`extensions.roots[]`), never as `extensions.roots[].root`. Both
/// element forms of `extensions.roots` therefore collapse onto one entry, which
/// is what the fixture's declared set spells.
fn collect_path_keys(
    node: &Value,
    key: &str,
    defs: &Value,
    frozen: bool,
    out: &mut BTreeSet<String>,
    referenced: &mut BTreeSet<String>,
) {
    let Some(obj) = node.as_object() else { return };

    if let Some(reference) = obj.get("$ref").and_then(Value::as_str) {
        if let Some(name) = reference.strip_prefix("#/$defs/") {
            if let Some(target) = defs.get(name) {
                collect_path_keys(target, key, defs, frozen, out, referenced);
            }
        } else if !reference.starts_with('#') {
            // `driver_contract.cross_file_ref`: a reference to a sibling schema
            // file (`sys_modules` -> `sys-modules.schema.json`) is followed
            // exactly like a local one, under the key it is referenced at, with
            // that file's own `$defs`.
            referenced.insert(reference.to_string());
            let document = schema(reference);
            let file_defs = document
                .get("$defs")
                .cloned()
                .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
            collect_path_keys(&document, key, &file_defs, frozen, out, referenced);
        }
        return;
    }

    if obj.get("x-apcore-path") == Some(&Value::Bool(true)) && !key.is_empty() {
        out.insert(key.to_string());
    }

    if let Some(properties) = obj.get("properties").and_then(Value::as_object) {
        for (name, child) in properties {
            let child_key = if frozen {
                key.to_string()
            } else if key.is_empty() {
                name.clone()
            } else {
                format!("{key}.{name}")
            };
            collect_path_keys(child, &child_key, defs, frozen, out, referenced);
        }
    }

    if let Some(items) = obj.get("items") {
        let child_key = if frozen {
            key.to_string()
        } else {
            format!("{key}[]")
        };
        collect_path_keys(items, &child_key, defs, true, out, referenced);
    }

    for combinator in ["oneOf", "anyOf", "allOf"] {
        if let Some(branches) = obj.get(combinator).and_then(Value::as_array) {
            for branch in branches {
                collect_path_keys(branch, key, defs, frozen, out, referenced);
            }
        }
    }
}

/// The projected keys of one source, and the sibling files it `$ref`s.
fn schema_path_keys(relative: &str) -> (BTreeSet<String>, BTreeSet<String>) {
    let document = schema(relative);
    let defs = document
        .get("$defs")
        .cloned()
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    let mut out = BTreeSet::new();
    let mut referenced = BTreeSet::new();
    collect_path_keys(&document, "", &defs, false, &mut out, &mut referenced);
    (out, referenced)
}

fn sdk_keys() -> BTreeSet<String> {
    Config::path_typed_keys()
        .iter()
        .map(|k| (*k).to_string())
        .collect()
}

fn declared_set(fx: &Value) -> BTreeSet<String> {
    fx["path_typed_keys"]
        .as_array()
        .expect("fixture declares path_typed_keys")
        .iter()
        .map(|v| {
            v.as_str()
                .expect("path_typed_keys entries are strings")
                .to_string()
        })
        .collect()
}

fn string_set(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .expect("expected a JSON array of strings")
        .iter()
        .map(|v| v.as_str().expect("array of strings").to_string())
        .collect()
}

/// Whether a config value read through the public API is a relative path.
fn relative_roots(config: &Config) -> Vec<String> {
    config
        .get("extensions.roots")
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|item| match item {
            Value::String(s) => Some(s.clone()),
            Value::Object(o) => o
                .get("root")
                .and_then(Value::as_str)
                .map(std::string::ToString::to_string),
            _ => None,
        })
        .collect()
}

/// Remove the variables this driver sets, so an ambient value from the
/// developer's shell cannot decide a case. An empty string is NOT a neutral
/// value here: §9.2 makes every `APCORE_*` variable an override, so
/// `APCORE_EXTENSIONS_ROOTS=""` still declares the key.
fn clear_env() {
    for name in ["APCORE_EXTENSIONS_ROOTS", "APCORE_EXTENSIONS_ROOT"] {
        std::env::remove_var(name);
    }
}

#[test]
#[allow(clippy::too_many_lines)] // one arm per fixture case; splitting hides the mapping
fn conformance_config_path_typed_keys() {
    let fx = fixture();
    let declared = declared_set(&fx);
    let cases = fx["test_cases"].as_array().expect("test_cases is an array");
    assert_eq!(cases.len(), 8, "driver is written against all 8 cases");

    clear_env();

    for tc in cases {
        let id = tc["id"].as_str().expect("every case needs an id");
        let expected = tc["expected"]
            .as_object()
            .unwrap_or_else(|| panic!("[{id}] case has no expected object"));

        match id {
            "declared_set_matches_schemas" => {
                // The fixture's own guard: its declared set must equal the
                // projection of `x-apcore-path` out of the canonical schemas.
                // Driven from `canonical_sources` rather than a literal, so a
                // schema file added there is projected without a driver edit.
                let sources: Vec<String> = fx["canonical_sources"]
                    .as_array()
                    .expect("fixture lists canonical_sources")
                    .iter()
                    .map(|v| v.as_str().expect("source is a string").to_string())
                    .collect();
                assert!(
                    !sources.is_empty(),
                    "[{id}] the fixture must name at least one canonical schema"
                );

                // A source another source `$ref`s is projected THROUGH that
                // reference, at the key it is mounted under; projected on its
                // own it would report its keys without their namespace.
                let mut projected: BTreeSet<String> = BTreeSet::new();
                let mut per_source: Vec<(String, BTreeSet<String>)> = Vec::new();
                let mut referenced: BTreeSet<String> = BTreeSet::new();
                for source in &sources {
                    let (keys, refs) = schema_path_keys(source);
                    referenced.extend(refs);
                    per_source.push((source.clone(), keys));
                }
                per_source.retain(|(source, _)| {
                    let file = std::path::Path::new(source)
                        .file_name()
                        .and_then(|f| f.to_str())
                        .unwrap_or_default();
                    !referenced.contains(file)
                });
                for (_, keys) in &per_source {
                    projected.extend(keys.iter().cloned());
                }

                let want = string_set(&expected["path_typed_keys"]);
                assert_eq!(
                    projected, want,
                    "[{id}] the schema projection and the case's expected set disagree"
                );
                assert_eq!(
                    declared, want,
                    "[{id}] the fixture's root path_typed_keys and this case disagree"
                );

                // `defaults.schema.json` MIRRORS the subset of keys that carry a
                // default; a key marked there but absent from the config schema
                // would mean the two canonical sources have drifted apart.
                let (authority, authority_keys) = per_source
                    .iter()
                    .find(|(name, _)| name.contains("apcore-config"))
                    .unwrap_or_else(|| {
                        panic!("[{id}] no canonical config schema among {sources:?}")
                    });
                for (name, keys) in &per_source {
                    assert!(
                        keys.is_subset(authority_keys),
                        "[{id}] {name} marks a key {authority} does not: {:?}",
                        keys.difference(authority_keys).collect::<Vec<_>>()
                    );
                }
            }

            "sdk_accessor_matches_declared_set" => {
                // Both directions, as the fixture's `both_directions_required`
                // clause demands: an SDK that omits one key and invents another
                // passes any length check.
                let sdk = sdk_keys();
                let mut missing: Vec<String> = declared.difference(&sdk).cloned().collect();
                let mut extra: Vec<String> = sdk.difference(&declared).cloned().collect();
                missing.sort();
                extra.sort();

                // Compared against the fixture's OWN lists rather than against
                // hardcoded empties. The previous form asserted `== vec![]` and
                // merely interpolated `expected` into the failure message, so
                // `check_case_pinning.py` could rewrite either list and nothing
                // went red — the case ran and its expectation was decoration.
                let want = |key: &str| -> Vec<String> {
                    expected[key]
                        .as_array()
                        .unwrap_or_else(|| panic!("[{id}] {key} must be a list"))
                        .iter()
                        .map(|v| v.as_str().unwrap_or_default().to_owned())
                        .collect()
                };
                assert_eq!(missing, want("missing_from_sdk"), "[{id}] missing_from_sdk");
                assert_eq!(extra, want("extra_in_sdk"), "[{id}] extra_in_sdk");
            }

            "bindings_pattern_is_not_path_typed" => {
                let key = tc["key"].as_str().expect("case names a key");
                let want = expected["path_typed"].as_bool().expect("path_typed bool");
                assert_eq!(
                    sdk_keys().contains(key),
                    want,
                    "[{id}] {key}: a glob matched WITHIN bindings.dir is not itself a path"
                );
            }

            "non_path_string_keys_are_not_path_typed" | "id_map_overrides_is_path_typed" => {
                let want = expected["path_typed"].as_bool().expect("path_typed bool");
                let sdk = sdk_keys();
                for key in tc["keys"].as_array().expect("case names keys") {
                    let key = key.as_str().expect("key is a string");
                    assert_eq!(sdk.contains(key), want, "[{id}] {key}");
                }
            }

            "extensions_roots_elements_are_path_typed" => {
                let reported = expected["reported_key"]
                    .as_str()
                    .expect("case names reported_key");
                assert!(
                    sdk_keys().contains(reported),
                    "[{id}] the list-valued key must be reported as {reported}"
                );

                // Both element forms must survive into the SDK's model, or the
                // key is only half implemented. `Registry::extension_roots()`
                // is the public reader of the resolved list.
                let config: Config = serde_json::from_value(tc["config"].clone())
                    .unwrap_or_else(|e| panic!("[{id}] case config deserializes: {e}"));
                let declared_roots = relative_roots(&config);
                assert_eq!(
                    declared_roots,
                    vec!["./extensions".to_string(), "./plugins".to_string()],
                    "[{id}] both the bare-string and the {{root, namespace}} form carry a path"
                );

                let registry = Registry::new();
                registry.set_extension_roots_from_config(&config);
                assert_eq!(
                    registry.extension_roots(),
                    declared_roots,
                    "[{id}] the resolved roots must keep both element forms"
                );

                let want = expected["path_typed"].as_bool().expect("path_typed bool");
                assert!(want, "[{id}] the fixture states this key IS path-typed");
            }

            "no_scalar_env_encoding_for_roots" => {
                // §9.2.1 requirement 3. The list-valued key has no scalar
                // environment encoding, so a delimiter-separated variable MUST
                // NOT yield a two-element roots list.
                assert!(
                    expected["roots_from_env"].is_null(),
                    "[{id}] the fixture expects no roots list from the environment"
                );
                let env = tc["env"].as_object().expect("case declares env");
                for (name, value) in env {
                    std::env::set_var(name, value.as_str().expect("env value is a string"));
                }

                let config = Config::from_defaults();
                let from_env = config.get("extensions.roots");
                assert!(
                    !matches!(from_env, Some(Value::Array(ref items)) if items.len() > 1),
                    "[{id}] APCORE_EXTENSIONS_ROOTS must not be split into a list, got {from_env:?}"
                );

                let registry = Registry::new();
                registry.set_extension_roots_from_config(&config);
                assert_ne!(
                    registry.extension_roots(),
                    vec!["./a".to_string(), "./b".to_string()],
                    "[{id}] the delimiter-separated encoding must not be invented"
                );

                clear_env();
            }

            "accessor_is_stable_across_config_instances" => {
                // The set is a property of the specification, not of a loaded
                // document.
                let want = expected["same_set"].as_bool().expect("same_set bool");
                assert!(want, "[{id}] the fixture expects one stable set");

                let from_defaults = Config::from_defaults();
                let dir = tempfile::tempdir().expect("tempdir");
                let path = dir.path().join("apcore.yaml");
                // Declares NONE of the path-typed keys, which is the point:
                // the accessor answers the same either way.
                std::fs::write(
                    &path,
                    "version: '1.0.0'\nproject:\n  name: path-typed-conformance\n",
                )
                .expect("write config");
                let from_file = Config::load(&path).expect("config loads");

                let a = sdk_keys();
                let b = sdk_keys();
                assert_eq!(a, b, "[{id}] the accessor is not stable");
                assert_eq!(a, declared, "[{id}] and it must equal the declared set");
                for key in &a {
                    // Reading through each instance must not change the answer.
                    let _ = from_defaults.get(key);
                    let _ = from_file.get(key);
                }
                assert_eq!(sdk_keys(), a, "[{id}] a load must not mutate the set");
            }

            other => panic!(
                "FAIL: {FIXTURE} grew case `{other}` that this driver does not \
                 handle — teach the driver, do not skip it"
            ),
        }
    }

    clear_env();
}
