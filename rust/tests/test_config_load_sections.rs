//! Every config section a FILE can declare must survive `Config::load`
//! (apcore-rust#34).
//!
//! ## What this file is for
//!
//! `tests/test_config_load_executor_namespace.rs` and
//! `tests/test_config_load_observability_subkeys.rs` exist because `executor`
//! and `observability` broke. This file covers **the sections that had no
//! `Config::load` coverage at all** — the ones that, on the day before #33 was
//! found, had exactly as much verification as `observability` did.
//!
//! The measured gap (apcore-rust#34): of the Config-touching tests in this
//! repo, the overwhelming majority build their `Config` with
//! `Config::from_defaults()` + `.set(…)`. **`Config::set` writes into
//! `user_namespaces` — a shadow entry no YAML file can produce**, and it skips
//! `Config::deserialize` entirely, which is the one step #33 broke. A test on
//! that path asserts against a state no deployment can reach.
//!
//! So **every test in this file goes through `Config::load` (or
//! `Config::from_json_file`, via `load`'s extension dispatch) from a real file
//! on disk**. Nothing here may be rewritten to use `Config::set`: that would
//! delete the only thing being tested.
//!
//! ## The two assertions that would have caught #33
//!
//! Each section is checked for both:
//!
//! 1. the file's value is what comes back from [`Config::get`], and
//! 2. [`Config::namespace`] reflects the **file** rather than a registered
//!    default.
//!
//! (2) is the sharper one. `namespace()` deep-merges a registered namespace's
//! §9.15 defaults as its base layer; when the loaded subtree goes missing the
//! base layer *is* the answer, so the caller reads a confident default back in
//! place of the value they wrote. `sys_modules` is the only section besides
//! `observability` with a registered default layer underneath it, which makes
//! [`namespace_sys_modules_reflects_the_file_not_the_registered_default`] the
//! direct analogue of the #33 probe.
//!
//! ## Fixture values are all deliberately non-default
//!
//! Every value the fixtures declare differs from the canonical default for its
//! key (`CONFIG_DEFAULTS` in `src/config.rs`, or the §9.15.3 `sys_modules`
//! registration). A reader that silently ignored the file and answered from the
//! default table would therefore fail every assertion here rather than passing
//! by coincidence.

use apcore::config::{Config, ConfigMode};
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// The section bodies shared by the namespace-mode, legacy-mode and JSON
/// fixtures, so one set of expectations covers all three load shapes.
///
/// `acl.default_effect: allow` is a **fixture value, not a recommendation.**
/// The canonical default is `deny` and it MUST stay `deny` in every example and
/// deployment (`PROTOCOL_SPEC` §9.3, `docs/features/acl-system.md`). It is
/// spelled `allow` here for exactly one reason: it is the only other legal
/// value, so it is the only way to prove that `get("acl.default_effect")`
/// reports the FILE rather than the default table. A test that wrote `deny`
/// would pass identically against a loader that discarded the whole `acl:`
/// block.
const SECTIONS_YAML: &str = r#"
modules_path: ./my-modules
extensions:
  root: ./ext
  auto_discover: false
  max_depth: 4
  follow_symlinks: true
schema:
  root: ./sch
  strategy: json_first
  max_ref_depth: 12
acl:
  root: ./my-acl
  default_effect: allow
sys_modules:
  enabled: true
  health:
    enabled: false
  usage:
    retention_hours: 24
    bucketing_strategy: daily
  events:
    enabled: true
    thresholds:
      error_rate: 0.5
stream:
  max_merge_depth: 7
my_vendor:
  knob: from-file
  nested:
    deep: 3
"#;

/// Namespace-mode document: the §9.6 `apcore:` block plus every section.
fn namespace_mode_yaml() -> String {
    format!("apcore:\n  version: \"9.9.9-fixture\"\n{SECTIONS_YAML}")
}

/// Legacy-mode document: no `apcore:` block, so the §9.3 required fields
/// (`version`, `project.name`) have to be declared at the root for `validate()`
/// to pass.
fn legacy_mode_yaml() -> String {
    format!("version: \"9.9.9-fixture\"\nproject:\n  name: sections-fixture\n{SECTIONS_YAML}")
}

/// Write `body` to `<tmp>/<name>` and load it the way a deployment does.
///
/// The `TempDir` is returned alongside the `Config` only to keep it alive:
/// dropping it deletes the file that `reload()` and `source_path()` refer back
/// to.
fn load_file(name: &str, body: &str) -> (tempfile::TempDir, Config) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(name);
    std::fs::write(&path, body).expect("write config file");
    let config = Config::load(&path).expect("a real config file must load");
    (dir, config)
}

/// Load the all-sections document in namespace mode.
fn loaded_ns() -> (tempfile::TempDir, Config) {
    let (dir, config) = load_file("apcore.yaml", &namespace_mode_yaml());
    assert_eq!(
        config.mode,
        ConfigMode::Namespace,
        "the `apcore:` block must put the loaded config in namespace mode"
    );
    (dir, config)
}

/// Load the all-sections document in legacy mode.
fn loaded_legacy() -> (tempfile::TempDir, Config) {
    let (dir, config) = load_file("apcore.yaml", &legacy_mode_yaml());
    assert_eq!(
        config.mode,
        ConfigMode::Legacy,
        "a document with no `apcore:` block must stay in legacy mode"
    );
    (dir, config)
}

/// Every `(key, file value)` pair the fixture declares outside `sys_modules`.
///
/// `sys_modules` is excluded because it is the one section with a registered
/// default layer under it, and it gets its own dedicated cases below.
fn declared_pairs() -> Vec<(&'static str, Value)> {
    vec![
        ("modules_path", json!("./my-modules")),
        ("extensions.root", json!("./ext")),
        ("extensions.auto_discover", json!(false)),
        ("extensions.max_depth", json!(4)),
        ("extensions.follow_symlinks", json!(true)),
        ("schema.root", json!("./sch")),
        ("schema.strategy", json!("json_first")),
        ("schema.max_ref_depth", json!(12)),
        ("acl.root", json!("./my-acl")),
        ("acl.default_effect", json!("allow")),
        ("stream.max_merge_depth", json!(7)),
        ("my_vendor.knob", json!("from-file")),
        ("my_vendor.nested.deep", json!(3)),
    ]
}

/// The canonical default for each key above that has one, so a test can prove
/// the fixture value is not simply the default arriving by another route.
fn canonical_defaults() -> Vec<(&'static str, Option<Value>)> {
    vec![
        ("modules_path", None),
        ("extensions.root", Some(json!("./extensions"))),
        ("extensions.auto_discover", Some(json!(true))),
        ("extensions.max_depth", Some(json!(8))),
        ("extensions.follow_symlinks", Some(json!(false))),
        ("schema.root", Some(json!("./schemas"))),
        ("schema.strategy", Some(json!("yaml_first"))),
        ("schema.max_ref_depth", Some(json!(32))),
        ("acl.root", Some(json!("./acl"))),
        ("acl.default_effect", Some(json!("deny"))),
        ("stream.max_merge_depth", Some(json!(32))),
        ("my_vendor.knob", None),
        ("my_vendor.nested.deep", None),
    ]
}

/// Assert `key` resolves to `expected` through `Config::get`, distinguishing
/// "discarded at load" (`None`) from "wrong value" (a precedence bug).
fn assert_get(config: &Config, key: &str, expected: &Value) {
    match config.get(key) {
        None => panic!(
            "`{key}` was written into a real config file and came back None — \
             the section is being discarded on the load path (apcore-rust#34)"
        ),
        Some(actual) => assert_eq!(
            &actual, expected,
            "`{key}` came back with the wrong value — the file says {expected}"
        ),
    }
}

/// Compare a `namespace()` map against a JSON object without depending on
/// `HashMap` iteration order.
fn namespace_value(config: &Config, name: &str) -> Value {
    Value::Object(config.namespace(name).into_iter().collect())
}

// ---------------------------------------------------------------------------
// Guard: the fixture cannot pass by coincidence
// ---------------------------------------------------------------------------

/// Every fixture value differs from the canonical default for its key.
///
/// Without this, a loader that discarded a whole section could still satisfy
/// [`get_reflects_every_declared_section`] through the `CONFIG_DEFAULTS`
/// fallback in `Config::get`, and every assertion in this file would be
/// vacuous. This runs the comparison rather than asserting it in a comment, so
/// editing a fixture value to match its default fails loudly here instead of
/// silently hollowing out the suite.
#[test]
fn every_fixture_value_differs_from_its_canonical_default() {
    let defaults = canonical_defaults();
    assert_eq!(
        declared_pairs().len(),
        defaults.len(),
        "the fixture table and the default table must cover the same keys"
    );
    for ((key, declared), (default_key, default)) in declared_pairs().iter().zip(defaults.iter()) {
        assert_eq!(
            key, default_key,
            "the two tables must stay in the same order"
        );
        assert_eq!(
            Config::default_for(key),
            *default,
            "`{key}`'s canonical default moved; update this table and pick a \
             fixture value that still differs from it"
        );
        if let Some(default) = default {
            assert_ne!(
                declared, default,
                "the fixture declares `{key}` = {declared}, which IS the canonical \
                 default — this key can no longer distinguish 'the file was read' \
                 from 'the default table answered'"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// get() — the file's value is what comes back
// ---------------------------------------------------------------------------

/// Namespace mode: every declared key resolves to the file's value.
#[test]
fn get_reflects_every_declared_section() {
    let (_dir, config) = loaded_ns();
    for (key, expected) in declared_pairs() {
        assert_get(&config, key, &expected);
    }
}

/// Legacy mode: the same document without an `apcore:` block resolves
/// identically. The two modes take different branches through
/// `Config::deserialize` and `Config::get`, so a fix that covers only one of
/// them is a half fix.
#[test]
fn get_reflects_every_declared_section_in_legacy_mode() {
    let (_dir, config) = loaded_legacy();
    for (key, expected) in declared_pairs() {
        assert_get(&config, key, &expected);
    }
    assert_get(&config, "version", &json!("9.9.9-fixture"));
    assert_get(&config, "project.name", &json!("sections-fixture"));
}

/// `modules_path` is the third typed field on `Config` (beside `executor` and
/// `observability`), and the only one with no `Config::load` coverage before
/// this file: it sits outside the `#[serde(flatten)]` bag exactly like the two
/// that broke.
///
/// Being a scalar it has no subkeys to lose, so it cannot fail the #33 way —
/// but it can fail the #34 way, by one reader disagreeing with another. All
/// four readers are checked against the same file.
#[test]
fn modules_path_typed_field_reflects_the_file_in_every_reader() {
    let (_dir, config) = loaded_ns();

    assert_eq!(
        config.modules_path.as_deref(),
        Some(std::path::Path::new("./my-modules")),
        "the typed field must carry the file's value"
    );
    assert_eq!(config.get("modules_path"), Some(json!("./my-modules")));
    assert_eq!(
        config.get_declared("modules_path"),
        Some(json!("./my-modules")),
        "a value the file declares must count as declared, not defaulted"
    );
    assert_eq!(config.data()["modules_path"], json!("./my-modules"));
}

/// The `version` / `project.name` pair — the only two §9.1 required fields,
/// because they are the only two with no canonical default. `get_declared` is
/// what the legacy-mode required-field check consults, so it is asserted
/// alongside `get`.
#[test]
fn required_fields_are_read_from_the_file() {
    let (_dir, config) = loaded_legacy();

    assert_eq!(config.get("version"), Some(json!("9.9.9-fixture")));
    assert_eq!(config.get_declared("version"), Some(json!("9.9.9-fixture")));
    assert_eq!(config.get("project.name"), Some(json!("sections-fixture")));
    assert_eq!(
        config.get_declared("project.name"),
        Some(json!("sections-fixture"))
    );
    assert_eq!(
        Config::default_for("version"),
        None,
        "`version` must have no canonical default — it is required precisely \
         because defaults.schema.json declares none"
    );
}

// ---------------------------------------------------------------------------
// namespace() — the file, not a registered default
// ---------------------------------------------------------------------------

/// `namespace("sys_modules")` must report the FILE, not the §9.15.3 registered
/// default.
///
/// This is the #33 probe pointed at the only other section with a registered
/// default layer underneath it. `namespace()` deep-merges the registration's
/// defaults as the base and overlays the loaded subtree; if the subtree were
/// discarded — or never merged — the base layer would be the answer, and the
/// operator would read `health.enabled: true` back from a file that says
/// `false`. A missing value is diagnosable; a confidently wrong one is not.
///
/// Every key asserted from the file differs from its registered default, so a
/// regression cannot pass by coincidence. The un-overridden defaults are
/// asserted too: the overlay must ADD to the registration, not replace it
/// wholesale.
#[test]
fn namespace_sys_modules_reflects_the_file_not_the_registered_default() {
    let (_dir, config) = loaded_ns();
    let ns = config.namespace("sys_modules");

    assert!(
        !ns.is_empty(),
        "`namespace(\"sys_modules\")` returned an EMPTY map for a config whose \
         file declares a `sys_modules:` block"
    );

    // --- values the file overrides (registered default in the message) ---
    assert_eq!(
        ns["health"]["enabled"],
        json!(false),
        "namespace() returned the registered default `true` for \
         `sys_modules.health.enabled` while the file says `false` — the exact \
         confidently-wrong value apcore-rust#33 was about, in the one other \
         namespace that has a default layer"
    );
    assert_eq!(
        ns["usage"]["retention_hours"],
        json!(24),
        "registered default is 168"
    );
    assert_eq!(
        ns["usage"]["bucketing_strategy"],
        json!("daily"),
        "registered default is \"hourly\""
    );
    assert_eq!(
        ns["events"]["enabled"],
        json!(true),
        "registered default is false"
    );
    assert_eq!(
        ns["events"]["thresholds"]["error_rate"],
        json!(0.5),
        "registered default is 0.1"
    );

    // --- registered defaults the file leaves alone must survive the overlay ---
    assert_eq!(
        ns["usage"]["enabled"],
        json!(true),
        "the file overrides two `usage` leaves; the third must still come from \
         the registration, not be wiped by the overlay"
    );
    assert_eq!(
        ns["events"]["thresholds"]["latency_p99_ms"],
        json!(5000.0),
        "a sibling threshold the file does not mention must survive"
    );
    assert_eq!(
        ns["manifest"]["enabled"],
        json!(true),
        "a family the file does not mention at all must still be present"
    );
    assert_eq!(ns["control"]["enabled"], json!(true));
}

/// `namespace()` and `get()` must agree about every key the file declares.
///
/// Two readers resolving the same namespace differently is the class of bug
/// both #33 and #34 are; this pins it for `sys_modules`, where the two readers
/// reach the value by different routes (`get` walks `user_namespaces`,
/// `namespace` deep-merges over the registered defaults).
#[test]
fn namespace_sys_modules_agrees_with_get_for_every_declared_key() {
    let (_dir, config) = loaded_ns();
    let declared: &[(&str, Value)] = &[
        ("sys_modules.enabled", json!(true)),
        ("sys_modules.health.enabled", json!(false)),
        ("sys_modules.usage.retention_hours", json!(24)),
        ("sys_modules.usage.bucketing_strategy", json!("daily")),
        ("sys_modules.events.enabled", json!(true)),
        ("sys_modules.events.thresholds.error_rate", json!(0.5)),
    ];
    let ns = namespace_value(&config, "sys_modules");

    for (key, expected) in declared {
        assert_get(&config, key, expected);
        let mut walked = ns.clone();
        for part in key.trim_start_matches("sys_modules.").split('.') {
            walked = walked
                .get(part)
                .unwrap_or_else(|| panic!("namespace(\"sys_modules\") has no `{part}` for {key}"))
                .clone();
        }
        assert_eq!(
            &walked, expected,
            "namespace() and get() disagree about `{key}`"
        );
    }
}

/// Sections with no registered default layer must still surface through
/// `namespace()` as the file's subtree — not an empty map.
///
/// `extensions`, `schema`, `acl`, `stream` and a vendor namespace are all
/// unregistered, so `namespace()` has nothing to merge underneath and returns
/// exactly what the file declared. An empty map here would be the `executor`
/// failure of #34: `get("extensions.root")` answering the file's value while
/// `namespace("extensions")` answers nothing.
#[test]
fn namespace_reflects_the_file_for_unregistered_sections() {
    let (_dir, config) = loaded_ns();

    assert_eq!(
        namespace_value(&config, "extensions"),
        json!({
            "root": "./ext",
            "auto_discover": false,
            "max_depth": 4,
            "follow_symlinks": true,
        })
    );
    assert_eq!(
        namespace_value(&config, "schema"),
        json!({ "root": "./sch", "strategy": "json_first", "max_ref_depth": 12 })
    );
    assert_eq!(
        namespace_value(&config, "acl"),
        json!({ "root": "./my-acl", "default_effect": "allow" })
    );
    assert_eq!(
        namespace_value(&config, "stream"),
        json!({ "max_merge_depth": 7 })
    );
    assert_eq!(
        namespace_value(&config, "my_vendor"),
        json!({ "knob": "from-file", "nested": { "deep": 3 } })
    );
}

/// A container fetch must agree with every leaf under it — the invariant #34
/// pinned for `executor`, applied to the sections that never had it.
#[test]
fn container_fetches_agree_with_their_leaves() {
    let (_dir, config) = loaded_ns();

    for section in ["extensions", "schema", "acl", "stream", "my_vendor"] {
        let container = config.get(section).unwrap_or_else(|| {
            panic!("`get(\"{section}\")` came back None for a declared section")
        });
        assert_eq!(
            container,
            namespace_value(&config, section),
            "`get(\"{section}\")` and `namespace(\"{section}\")` disagree"
        );
    }

    for (key, expected) in declared_pairs() {
        let Some((section, rest)) = key.split_once('.') else {
            continue;
        };
        let container = config.get(section).expect("container fetch");
        let mut walked = container;
        for part in rest.split('.') {
            walked = walked
                .get(part)
                .unwrap_or_else(|| panic!("`get(\"{section}\")` has no `{part}`"))
                .clone();
        }
        assert_eq!(walked, expected, "container/leaf disagreement on `{key}`");
    }
}

/// `bind` into a caller's own type must see the file's values — it is built on
/// `namespace()`, so a section missing there is invisible here too.
#[test]
fn bind_sees_the_files_values_for_a_user_namespace() {
    #[derive(serde::Deserialize)]
    struct Nested {
        deep: u32,
    }
    #[derive(serde::Deserialize)]
    struct Vendor {
        knob: String,
        nested: Nested,
    }

    let (_dir, config) = loaded_ns();
    let vendor: Vendor = config.bind("my_vendor").expect("bind my_vendor");
    assert_eq!(vendor.knob, "from-file");
    assert_eq!(vendor.nested.deep, 3);
}

// ---------------------------------------------------------------------------
// The `_config` control block and namespace-mode nesting
// ---------------------------------------------------------------------------

/// `_config.strict` must be read from the file and must gate validation.
///
/// The pre-existing coverage for strict mode (`strict_mode_rejects_unknown_namespace`
/// in `src/config.rs`) builds its `Config` with `serde_json::from_value` and
/// then calls `validate()` by hand — which never runs `init_builtin_namespaces`,
/// so the namespace registry it validates against is whatever earlier tests in
/// the same binary happened to leave behind. On the load path the registry is
/// initialized first, which is the only configuration a deployment can produce.
#[test]
fn config_strict_block_is_read_from_the_file() {
    let dir = tempfile::tempdir().expect("tempdir");

    let rejected = dir.path().join("strict-unknown.yaml");
    std::fs::write(
        &rejected,
        "apcore:\n  version: \"1.0\"\n_config:\n  strict: true\n\
         unregistered_ns_for_the_strict_case:\n  a: 1\n",
    )
    .expect("write config");
    let err =
        Config::load(&rejected).expect_err("strict mode must reject a namespace nobody registered");
    assert_eq!(err.code, apcore::errors::ErrorCode::ConfigInvalid);
    assert!(
        err.message.contains("unregistered_ns_for_the_strict_case"),
        "the error must name the offending namespace: {}",
        err.message
    );

    let accepted = dir.path().join("strict-ok.yaml");
    std::fs::write(
        &accepted,
        "apcore:\n  version: \"1.0\"\n_config:\n  strict: true\n\
         sys_modules:\n  enabled: true\n",
    )
    .expect("write config");
    let config = Config::load(&accepted).expect("a registered namespace passes strict mode");
    assert_eq!(
        config.get("_config.strict"),
        Some(json!(true)),
        "`_config.strict` must be readable from the loaded document, not only \
         consumed internally by validate()"
    );
    assert_eq!(config.get("sys_modules.enabled"), Some(json!(true)));
}

/// §9.6 lets a namespace-mode document nest sections under `apcore:`. Those
/// sections must reach the same readers as their top-level spelling.
///
/// `Config::deserialize` merges the `apcore:` block into the document before
/// handing it to the helper struct; this pins that the merge feeds
/// `user_namespaces` too, so `get`/`namespace` see the nested sections. #33's
/// fix depended on reading the raw `observability` object from the POST-merge
/// document for exactly this reason.
#[test]
fn sections_nested_under_the_apcore_block_reach_the_same_readers() {
    let (_dir, config) = load_file(
        "apcore.yaml",
        "apcore:\n  version: \"1.0\"\n  extensions:\n    max_depth: 4\n\
         \x20 sys_modules:\n    usage:\n      retention_hours: 24\n",
    );

    assert_eq!(config.mode, ConfigMode::Namespace);
    assert_eq!(config.get("extensions.max_depth"), Some(json!(4)));
    assert_eq!(
        config.get("sys_modules.usage.retention_hours"),
        Some(json!(24))
    );
    assert_eq!(
        config.namespace("sys_modules")["usage"]["retention_hours"],
        json!(24),
        "a nested section must reach namespace() as well, or the two readers \
         disagree about a value the file plainly declares"
    );
    assert_eq!(
        config.namespace("extensions"),
        [("max_depth".to_string(), json!(4))].into_iter().collect(),
    );
    assert_eq!(
        config.get("apcore.version"),
        Some(json!("1.0")),
        "the `apcore:` block itself stays addressable"
    );
}

// ---------------------------------------------------------------------------
// Absent sections
// ---------------------------------------------------------------------------

/// A document that declares none of these sections must not have values
/// invented for it beyond the canonical default table.
///
/// This is the counterweight to every assertion above: without it, a `get`
/// that answered from some default layer for *everything* would satisfy the
/// positive cases and still be wrong.
#[test]
fn absent_sections_resolve_to_the_default_table_and_nothing_more() {
    let (_dir, config) = load_file("apcore.yaml", "apcore:\n  version: \"1.0\"\n");

    // Keys WITH a canonical default resolve to it.
    for (key, default) in canonical_defaults() {
        let Some(default) = default else { continue };
        assert_eq!(
            config.get(key),
            Some(default.clone()),
            "`{key}` is absent from the file and must resolve to its canonical default"
        );
        assert_eq!(
            config.get_declared(key),
            None,
            "`{key}` was never declared, so `get_declared` must report absence — \
             the required-field check in `validate()` depends on this distinction"
        );
    }

    // Keys with no canonical default resolve to nothing.
    for key in [
        "modules_path",
        "my_vendor.knob",
        "extensions",
        "schema",
        "acl",
        "stream",
        "my_vendor",
    ] {
        assert_eq!(
            config.get(key),
            None,
            "`{key}` has no canonical default and was never declared, so `get` \
             must report absence rather than inventing a value"
        );
    }
    assert!(config.namespace("extensions").is_empty());
    assert!(config.namespace("my_vendor").is_empty());
}

/// `Config::get` and `Config::namespace` agree on a registered namespace's
/// defaults (§9.15) — the cross-language divergence pinned here is closed.
///
/// Previously `namespace()` deep-merged the registration defaults as its base
/// layer while `get()` consulted only `user_namespaces` and the flat
/// `CONFIG_DEFAULTS` table, so the two readers disagreed on any subkey the file
/// left undeclared: `namespace("sys_modules")["usage"]["enabled"]` was `true`
/// while `get("sys_modules.usage.enabled")` was `None`. apcore-python and
/// apcore-typescript never had it — both seed the registration defaults into
/// their data tree at load in namespace mode.
///
/// The earlier version of this test recorded the fix as blocked on a spec
/// question: Rust could not order the registration defaults against
/// `CONFIG_DEFAULTS` while the two disagreed outright on `sys_modules.enabled`
/// (registration `true`, `defaults.schema.json` `false`). PROTOCOL_SPEC §9.15.3
/// is corrected in spec v1.17.0 — both now say `false` — so the ordering could
/// finally be stated: loaded config, then registration defaults, then
/// CONFIG_DEFAULTS. The lookup lives in `get` and NOT in `get_direct`, so
/// `get_declared` still distinguishes "declared" from "defaulted" and
/// legacy-mode required-field validation is unaffected.
#[test]
fn get_and_namespace_agree_on_registered_namespace_defaults() {
    let (_dir, config) = loaded_ns();

    // The file declares `usage.retention_hours` but not `usage.enabled`.
    // Both readers now resolve it from the §9.15.3 registration.
    assert_eq!(
        config.namespace("sys_modules")["usage"]["enabled"],
        json!(true),
        "namespace() merges the §9.15.3 registration"
    );
    assert_eq!(
        config.get("sys_modules.usage.enabled"),
        Some(json!(true)),
        "get() now consults it too — previously None, which is what made the \
         two readers disagree"
    );

    // A value the file DID declare still wins over the registration default.
    assert_eq!(
        config.get("sys_modules.usage.retention_hours"),
        config.namespace("sys_modules")["usage"]["retention_hours"]
            .clone()
            .into(),
        "the loaded config outranks the registration defaults"
    );

    // The activation flag agrees across both readers and both default tables.
    let (_dir2, bare) = load_file("apcore.yaml", "apcore:\n  version: \"1.0\"\n");
    assert_eq!(
        bare.get("sys_modules.enabled"),
        Some(json!(false)),
        "CONFIG_DEFAULTS and the §9.15.3 registration both say false as of spec v1.17.0"
    );
    assert_eq!(
        bare.namespace("sys_modules")["enabled"],
        json!(false),
        "namespace() agrees — this pair used to read false vs true"
    );
}

/// `get_declared` must NOT see registration defaults: legacy-mode required-field
/// validation (§9.3 step 1) distinguishes declared from defaulted, and a default
/// leaking there would let the check pass on a key nobody wrote.
#[test]
fn registered_namespace_defaults_do_not_leak_into_get_declared() {
    let (_dir, config) = loaded_ns();
    assert_eq!(
        config.get("sys_modules.usage.enabled"),
        Some(json!(true)),
        "get() resolves it"
    );
    assert_eq!(
        config.get_declared("sys_modules.usage.enabled"),
        None,
        "get_declared() must not — the file never declared it"
    );
}

// ---------------------------------------------------------------------------
// Other load entry points: JSON, reload, round-trip
// ---------------------------------------------------------------------------

/// The same document through `Config::from_json_file` (reached by `load`'s
/// extension dispatch) must resolve identically.
///
/// The JSON branch is a separate reader with its own `serde_json::from_reader`
/// call; before this the only `.json` config any test loaded was
/// `tests/test_acl_root_discovery.rs`'s, which asserts on `acl.root` alone.
#[test]
fn json_load_path_reflects_every_declared_section() {
    let body: Value =
        serde_yaml_ng::from_str(&legacy_mode_yaml()).expect("the YAML fixture reparses as a value");
    let (_dir, config) = load_file(
        "apcore.config.json",
        &serde_json::to_string_pretty(&body).expect("serialize json"),
    );

    assert_eq!(config.mode, ConfigMode::Legacy);
    for (key, expected) in declared_pairs() {
        assert_get(&config, key, &expected);
    }
    assert_eq!(
        config.namespace("sys_modules")["usage"]["retention_hours"],
        json!(24)
    );
}

/// `reload()` re-reads the file through the same deserializer, so a fix
/// confined to the first load would leave a reloaded config half-empty.
#[test]
fn reload_preserves_every_section() {
    let (_dir, mut config) = loaded_ns();
    config.reload().expect("reload from the stored path");

    for (key, expected) in declared_pairs() {
        assert_get(&config, key, &expected);
    }
    assert_eq!(
        config.namespace("sys_modules")["health"]["enabled"],
        json!(false)
    );
    assert_eq!(
        namespace_value(&config, "stream"),
        json!({"max_merge_depth": 7})
    );
}

/// The §9.1 wire form must carry every section and survive
/// `data()` → parse → `data()`, which is what a cross-process config handoff
/// does. A section dropped on the way through shows up here.
#[test]
fn data_round_trip_is_stable_for_every_section() {
    let (_dir, config) = loaded_ns();
    let once = config.data();

    for (key, expected) in declared_pairs() {
        let mut walked = once.clone();
        for part in key.split('.') {
            walked = walked
                .get(part)
                .unwrap_or_else(|| panic!("data() has no `{part}` for `{key}`"))
                .clone();
        }
        assert_eq!(
            walked, expected,
            "data() disagrees with the file on `{key}`"
        );
    }

    let reparsed: Config = serde_json::from_value(once.clone()).expect("data() must reparse");
    assert_eq!(
        reparsed.data(),
        once,
        "a config serialized, reparsed and re-serialized must be identical — if \
         a section is dropped on the way through, this is where it shows"
    );
}
