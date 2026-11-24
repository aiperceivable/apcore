//! Load-path coverage for the framework sections that had none
//! (apcore-rust#34).
//!
//! ## Which sections, and how they were found
//!
//! `src/config.rs` names a config section in six different places, and the
//! coverage guard (`tests/test_config_load_coverage_guard.rs`) was reading
//! five of them: `CONFIG_DEFAULTS`, the built-in namespace registrations, the
//! constrained-key table, the required-field list and the reserved names. It
//! was **not** reading [`FRAMEWORK_SECTION_KEYS`] — the longest of the six, and
//! the only one projected from `schemas/apcore-config.schema.json`.
//!
//! Seven sections appear only there, and an audit of every `Config::load`-family
//! call in `tests/` confirmed that not one of them was ever written into a real
//! config file by a test:
//!
//! | section | declared by a load test | asserted after load |
//! |---|---|---|
//! | `bindings` | no | no |
//! | `id_map` | no | no |
//! | `logging` | no | no |
//! | `middleware` | no | no |
//! | `obs` | only indirectly[^obs] | via `RedactionConfig`, not `Config` |
//! | `pipeline` | no | no |
//! | `validation` | no | no |
//!
//! [^obs]: `tests/test_redaction_config_conformance.rs` builds an `apcore.yaml`
//! at run time from a conformance fixture and loads it, so `obs.redaction.*`
//! does reach `Config::load` — but it asserts through `RedactionConfig`, never
//! through `Config::get` / `Config::namespace`, and the fixture is assembled
//! dynamically so no static audit of the test sources can see it. It is covered
//! here as well, directly through the `Config` readers.
//!
//! That is the same state `observability` was in the day before #33 and
//! `executor` the day before its own gap: an operator can declare the section,
//! `schemas/apcore-config.schema.json` blesses it, and nothing verified that a
//! value written into a file survives to a reader.
//!
//! ## Every `Config` here comes from a file
//!
//! No test in this file may be rewritten to use `Config::from_defaults()` +
//! `.set(…)`. `set` writes into `user_namespaces` — a shadow entry no YAML file
//! can produce — and skips `Config::deserialize`, which is the single step #33
//! broke. Rewriting these onto `set` would delete the only thing being tested.
//!
//! ## The two assertions per section
//!
//! 1. **`get` returns the file's value.** Paired with
//!    [`absent_sections_are_not_invented`], which proves the value could only
//!    have come from the file: none of these seven sections has an entry in
//!    `CONFIG_DEFAULTS`, so an undeclared one resolves to `None`. Without that
//!    control, a reader that ignored the file could still pass by answering
//!    from the default table — which is exactly how a broken loader stays
//!    green.
//!
//! 2. **`namespace()` reflects the file.** For `observability` and
//!    `sys_modules` this means "rather than the registered §9.15 default",
//!    because those two are the only sections with a registered default layer
//!    underneath (`init_builtin_namespaces`), and that layer answering in place
//!    of the operator's value *was* #33. None of the seven sections here has
//!    such a layer, so the reachable form of the same assertion is the
//!    invariant both #33 and #34 actually violated —
//!    [`namespace_agrees_with_get_for_every_declared_key`]: two readers over
//!    one namespace must not disagree. That is checked per key, in both modes,
//!    alongside an exact whole-subtree comparison
//!    ([`namespace_returns_exactly_the_files_subtree`]) so a reader that
//!    invented a key or dropped one fails even where `get` agrees with it.
//!
//! Registering a namespace with conflicting defaults would exercise the
//! shadowing case literally, but `Config::register_namespace` writes to a
//! process-global registry; this file lives in the consolidated `tests/it.rs`
//! binary and must not mutate global state that its file-mates share.
//!
//! [`FRAMEWORK_SECTION_KEYS`]: apcore::config::FRAMEWORK_SECTION_KEYS

use apcore::config::{Config, ConfigMode};
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

/// The seven sections under audit, each declaring keys that
/// `schemas/apcore-config.schema.json` declares for it.
///
/// Only schema-declared keys are used, so the document is valid under
/// `_config.strict: true` as well — [`strict_mode_accepts_every_declared_key`]
/// loads this same body with strict on, which would fail on any key the
/// canonical schema does not bless.
const SECTIONS_YAML: &str = r#"
$schema: "https://apcore.dev/schemas/apcore-config.schema.json"
logging:
  level: debug
  format: text
middleware:
  disabled:
    - audit
    - tracing
pipeline:
  remove:
    - check_acl
  steps:
    - name: custom_step
      type: noop
      after: validate_input
validation:
  binding: strict
  pipeline: lenient
id_map:
  auto_detect: false
  overrides:
    legacy.name: executor.new.name
bindings:
  dir: ./my-bindings
  pattern: "*.bind.yaml"
obs:
  redaction:
    sensitive_keys:
      - vendor_token
    replacement: "[GONE]"
"#;

/// Namespace-mode document: the §9.6 `apcore:` block plus every section.
fn namespace_mode_yaml() -> String {
    format!("apcore:\n  version: \"9.9.9-fixture\"\n{SECTIONS_YAML}")
}

/// Legacy-mode document: no `apcore:` block, so the §9.1 required fields have
/// to be declared at the root for `validate()` to pass.
fn legacy_mode_yaml() -> String {
    format!("version: \"9.9.9-fixture\"\nproject:\n  name: framework-sections\n{SECTIONS_YAML}")
}

/// Write `body` to `<tmp>/<name>` and load it the way a deployment does.
///
/// The `TempDir` is returned alongside the `Config` only to keep it alive:
/// dropping it deletes the file `reload()` refers back to.
fn load_file(name: &str, body: &str) -> (tempfile::TempDir, Config) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(name);
    std::fs::write(&path, body).expect("write config file");
    let config = Config::load(&path).expect("a real config file must load");
    (dir, config)
}

fn loaded_ns() -> (tempfile::TempDir, Config) {
    let (dir, config) = load_file("apcore.yaml", &namespace_mode_yaml());
    assert_eq!(
        config.mode,
        ConfigMode::Namespace,
        "the `apcore:` block must put the loaded config in namespace mode"
    );
    (dir, config)
}

fn loaded_legacy() -> (tempfile::TempDir, Config) {
    let (dir, config) = load_file("apcore.yaml", &legacy_mode_yaml());
    assert_eq!(
        config.mode,
        ConfigMode::Legacy,
        "a document with no `apcore:` block must stay in legacy mode"
    );
    (dir, config)
}

/// The seven sections this file exists for.
const SECTIONS: &[&str] = &[
    "logging",
    "middleware",
    "pipeline",
    "validation",
    "id_map",
    "bindings",
    "obs",
];

/// Every `(dot-path key, value the file declares)` pair in [`SECTIONS_YAML`].
fn declared_pairs() -> Vec<(&'static str, Value)> {
    vec![
        // Not a section — a scalar top-level key, and the last framework key
        // with no load-path coverage at all. It went unnoticed because the
        // coverage guard read the section table, which never listed it
        // (sync finding A-D-020).
        (
            "$schema",
            json!("https://apcore.dev/schemas/apcore-config.schema.json"),
        ),
        ("logging.level", json!("debug")),
        ("logging.format", json!("text")),
        ("middleware.disabled", json!(["audit", "tracing"])),
        ("pipeline.remove", json!(["check_acl"])),
        (
            "pipeline.steps",
            json!([{"name": "custom_step", "type": "noop", "after": "validate_input"}]),
        ),
        ("validation.binding", json!("strict")),
        ("validation.pipeline", json!("lenient")),
        ("id_map.auto_detect", json!(false)),
        (
            "id_map.overrides",
            json!({"legacy.name": "executor.new.name"}),
        ),
        // Both spelled DIFFERENTLY from their canonical defaults (`./bindings`,
        // `*.binding.yaml`, added to `defaults.schema.json` in spec v1.36.0).
        // `absent_sections_are_not_invented` enforces that difference: since
        // the two keys now have a default layer underneath, only a value the
        // default cannot supply proves the file was read.
        ("bindings.dir", json!("./my-bindings")),
        ("bindings.pattern", json!("*.bind.yaml")),
        ("obs.redaction.sensitive_keys", json!(["vendor_token"])),
        ("obs.redaction.replacement", json!("[GONE]")),
    ]
}

/// The whole subtree each section declares, as `namespace()` must report it.
fn expected_subtrees() -> Vec<(&'static str, Value)> {
    vec![
        ("logging", json!({"level": "debug", "format": "text"})),
        ("middleware", json!({"disabled": ["audit", "tracing"]})),
        (
            "pipeline",
            json!({
                "remove": ["check_acl"],
                "steps": [{"name": "custom_step", "type": "noop", "after": "validate_input"}]
            }),
        ),
        (
            "validation",
            json!({"binding": "strict", "pipeline": "lenient"}),
        ),
        (
            "id_map",
            json!({"auto_detect": false, "overrides": {"legacy.name": "executor.new.name"}}),
        ),
        (
            "bindings",
            json!({"dir": "./my-bindings", "pattern": "*.bind.yaml"}),
        ),
        (
            "obs",
            json!({"redaction": {"regex_patterns": [], "sensitive_keys": ["vendor_token"], "replacement": "[GONE]"}}),
        ),
    ]
}

/// Assert `key` resolves to `expected`, distinguishing "discarded at load"
/// (`None`) from "wrong value" (a precedence bug).
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

/// Walk a dot-path through a JSON value.
fn walk<'a>(root: &'a Value, key: &str) -> Option<&'a Value> {
    let mut current = root;
    for part in key.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

// ---------------------------------------------------------------------------
// Guard: the fixture cannot pass by coincidence
// ---------------------------------------------------------------------------

/// A config that declares none of these sections must answer `None` / empty for
/// every key that has no canonical default, and a value the file did NOT write
/// for the two that do.
///
/// This is what makes every `get` assertion below non-vacuous: it proves the
/// values they see could only have come from the file. `test_config_load_
/// sections.rs` gets the same protection from `every_fixture_value_differs_
/// from_its_canonical_default`; most sections here have no default to differ
/// from, so absence is the discriminator instead.
///
/// `bindings.dir` and `bindings.pattern` are the exception since spec v1.36.0,
/// which added the `bindings` section to `defaults.schema.json`: they now
/// resolve to `./bindings` and `*.binding.yaml` for a config that never
/// declared them. For those two the discriminator is the one
/// `test_config_load_sections.rs` uses — the fixture's value must differ from
/// the canonical default, so a reader answering from the default table cannot
/// pass the assertions below.
#[test]
fn absent_sections_are_not_invented() {
    let (_dir, config) = load_file("apcore.yaml", "apcore:\n  version: \"1.0\"\n");

    for (key, declared) in declared_pairs() {
        let registered_default = key.strip_prefix("obs.").and_then(|rest| {
            let namespace = Value::Object(config.namespace("obs").into_iter().collect());
            walk(&namespace, rest).cloned()
        });
        match registered_default.or_else(|| Config::default_for(key)) {
            None => assert_eq!(
                config.get(key),
                None,
                "`{key}` was never declared and has no canonical default, so \
                 `get` must report absence rather than inventing a value — if \
                 this starts returning Some, every assertion in this file is \
                 vacuous"
            ),
            Some(canonical) => {
                assert_eq!(
                    config.get(key),
                    Some(canonical.clone()),
                    "`{key}` carries a canonical default, so an undeclared \
                     config must resolve it to exactly that value"
                );
                assert_ne!(
                    canonical, declared,
                    "`{key}` has a canonical default, so the fixture MUST \
                     declare a different value — otherwise every assertion \
                     about it passes on a reader that ignored the file"
                );
            }
        }
    }
    for section in SECTIONS {
        if *section == "obs" {
            assert!(
                config.get(section).is_some(),
                "the built-in `{section}` namespace must resolve its defaults"
            );
            assert!(
                !config.namespace(section).is_empty(),
                "the built-in `{section}` namespace must expose its defaults"
            );
            continue;
        }
        assert_eq!(
            config.get(section),
            None,
            "the `{section}` container was never declared and must not resolve"
        );
        assert!(
            config.namespace(section).is_empty(),
            "`namespace({section})` must be empty for a config that never \
             declared it — a non-empty map here means a default layer exists \
             and the fixture assertions cannot tell it from the file"
        );
        assert_eq!(
            Config::default_for(section),
            None,
            "`{section}` gained a CONFIG_DEFAULTS entry; this file's \
             non-vacuity argument rests on it having none, so give it a \
             non-default fixture value the way test_config_load_sections.rs does"
        );
    }
}

// ---------------------------------------------------------------------------
// Assertion 1 — the file's value is what comes back
// ---------------------------------------------------------------------------

/// Namespace mode: every declared key resolves to the file's value.
#[test]
fn get_reflects_every_declared_section() {
    let (_dir, config) = loaded_ns();
    for (key, expected) in declared_pairs() {
        assert_get(&config, key, &expected);
    }
}

/// Legacy mode: the same sections without an `apcore:` block.
///
/// The two modes take different branches through `Config::deserialize` (the
/// namespace-mode branch merges the `apcore:` members into `core_data` first)
/// and different branches through `Config::get` (the §9.9.1 implicit-`apcore`
/// fallback fires only in namespace mode), so a fix covering one is a half fix.
#[test]
fn get_reflects_every_declared_section_in_legacy_mode() {
    let (_dir, config) = loaded_legacy();
    for (key, expected) in declared_pairs() {
        assert_get(&config, key, &expected);
    }
}

/// A container fetch must return the section object, not `None`.
///
/// This is the exact shape of the `executor` half of #34: `get("executor")`
/// answered `None` for every file-loaded config while
/// `get("executor.max_call_depth")` answered the file's value.
#[test]
fn container_fetches_agree_with_their_leaves() {
    let (_dir, config) = loaded_ns();
    for (section, subtree) in expected_subtrees() {
        let declared_subtree = if section == "obs" {
            json!({"redaction": {"sensitive_keys": ["vendor_token"], "replacement": "[GONE]"}})
        } else {
            subtree.clone()
        };
        let container = config.get(section).unwrap_or_else(|| {
            panic!(
                "`get({section})` returned None though the file declares the \
                 section — a container fetch must not contradict its own leaves"
            )
        });
        assert_eq!(
            container, declared_subtree,
            "`get({section})` disagrees with the file's subtree"
        );
        for (key, expected) in declared_pairs() {
            let Some(rest) = key.strip_prefix(&format!("{section}.")) else {
                continue;
            };
            assert_eq!(
                walk(&container, rest),
                Some(&expected),
                "`get({section})` and `get({key})` disagree — two readers over \
                 one namespace, which is what #33 and #34 both were"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Assertion 2 — namespace() reflects the file
// ---------------------------------------------------------------------------

/// `namespace()` returns exactly the file's subtree: nothing dropped, nothing
/// invented.
///
/// An equality check rather than a per-key one, so a reader that silently added
/// a key (a default leaking through) or dropped one fails here even when `get`
/// happens to agree with it.
#[test]
fn namespace_returns_exactly_the_files_subtree() {
    for (tag, (_dir, config)) in [
        ("namespace mode", loaded_ns()),
        ("legacy mode", loaded_legacy()),
    ] {
        for (section, expected) in expected_subtrees() {
            assert_eq!(
                namespace_value(&config, section),
                expected,
                "[{tag}] `namespace({section})` does not match the file's subtree"
            );
        }
    }
}

/// `$schema` survives a load in both modes.
///
/// It is the one framework key that is a root scalar rather than a section, so
/// the per-section loops above cannot carry it, and it had no load-path
/// coverage of any kind: nothing verified that a document declaring the
/// customary JSON-Schema pointer still loads, or that the value comes back
/// unchanged rather than being swallowed as an unknown root key.
#[test]
fn the_schema_pointer_survives_a_load() {
    let expected = json!("https://apcore.dev/schemas/apcore-config.schema.json");
    for (tag, (_dir, config)) in [
        ("namespace mode", loaded_ns()),
        ("legacy mode", loaded_legacy()),
    ] {
        assert_eq!(
            config.get("$schema"),
            Some(expected.clone()),
            "[{tag}] the `$schema` pointer the file declares must survive the load"
        );
    }
}

/// The invariant #33 and #34 both broke: `namespace()` and `get()` must not
/// disagree about the same namespace.
///
/// #33's sharpest edge was `namespace("observability")` reporting a registered
/// default for a key the operator had explicitly set, while `get` on the same
/// key reported something else. None of the seven sections here has a
/// registered default layer to shadow them with, so this — the two readers
/// agreeing, per key, in both modes — is the reachable form of that assertion.
#[test]
fn namespace_agrees_with_get_for_every_declared_key() {
    for (tag, (_dir, config)) in [
        ("namespace mode", loaded_ns()),
        ("legacy mode", loaded_legacy()),
    ] {
        for (key, expected) in declared_pairs() {
            // `$schema` is a root scalar, not a section, so there is no
            // namespace for the two readers to disagree about. `get` on it is
            // asserted by the tests above.
            let Some((section, rest)) = key.split_once('.') else {
                continue;
            };
            let ns = namespace_value(&config, section);
            assert_eq!(
                walk(&ns, rest),
                Some(&expected),
                "[{tag}] `namespace({section})` does not carry `{key}`, but the \
                 file declares it"
            );
            assert_eq!(
                walk(&ns, rest).cloned(),
                config.get(key),
                "[{tag}] `namespace({section})` and `get({key})` disagree"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The other load entry points
// ---------------------------------------------------------------------------

/// The same document through `Config::from_json_file` (reached by `load`'s
/// extension dispatch) must resolve identically. The JSON branch is a separate
/// reader with its own `serde_json::from_reader` call.
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
    for (section, expected) in expected_subtrees() {
        assert_eq!(namespace_value(&config, section), expected);
    }
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
    for (section, expected) in expected_subtrees() {
        assert_eq!(
            namespace_value(&config, section),
            expected,
            "`namespace({section})` lost the file's subtree across a reload"
        );
    }
}

/// The §9.1 wire form must carry every section and survive
/// `data()` → parse → `data()`, which is what a cross-process config handoff
/// does. A section dropped on the way through shows up here.
#[test]
fn data_round_trip_is_stable_for_every_section() {
    let (_dir, config) = loaded_ns();
    let once = config.data();

    for (key, expected) in declared_pairs() {
        assert_eq!(
            walk(&once, key),
            Some(&expected),
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

// ---------------------------------------------------------------------------
// §9.14 strict mode
// ---------------------------------------------------------------------------

/// Every key the fixture declares is one `schemas/apcore-config.schema.json`
/// declares, so the whole document loads under `_config.strict: true`.
///
/// This is what keeps the fixture honest: each section in that schema is
/// closed (`additionalProperties: false`), so a key invented for convenience
/// here would fail this test rather than quietly testing a shape no operator
/// can deploy under strict mode.
#[test]
fn strict_mode_accepts_every_declared_key() {
    let body =
        format!("apcore:\n  version: \"9.9.9-fixture\"\n_config:\n  strict: true\n{SECTIONS_YAML}");
    let (_dir, config) = load_file("apcore.yaml", &body);
    for (key, expected) in declared_pairs() {
        assert_get(&config, key, &expected);
    }
}

/// §9.14: under strict mode an undeclared key inside a framework section MUST
/// raise `CONFIG_INVALID` — for each of these sections too, not just the ones
/// that already had coverage.
#[test]
fn strict_mode_rejects_an_undeclared_key_in_each_section() {
    for section in SECTIONS {
        let body = format!(
            "apcore:\n  version: \"9.9.9-fixture\"\n_config:\n  strict: true\n\
             {section}:\n  definitely_not_a_declared_key: 1\n"
        );
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("apcore.yaml");
        std::fs::write(&path, &body).expect("write config file");

        let err = Config::load(&path)
            .expect_err("strict mode must reject a key the canonical schema does not declare");
        let rendered = err.to_string();
        assert!(
            rendered.contains(&format!("{section}.definitely_not_a_declared_key")),
            "the CONFIG_INVALID error must name the offending key; got: {rendered}"
        );
    }
}
