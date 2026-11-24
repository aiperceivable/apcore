//! The `executor` namespace must be reachable as a NAMESPACE — not only through
//! the typed `Config::executor` field — after `Config::load` from a real YAML
//! file (apcore-rust#34, the follow-on gap flagged while fixing #33).
//!
//! ## How this differs from the `observability` case, and why it is narrower
//!
//! #33 was data loss: the §9.15.2 namespace registration declares fifteen
//! `observability.*` keys that `ObservabilityConfig` does not model, and
//! `Config::deserialize` discarded every one of them at parse time.
//!
//! `executor` is not that. `ExecutorConfig` models `default_timeout`,
//! `global_timeout`, `max_call_depth` and `max_module_repeat` — which is
//! *exactly* the property set `$defs/ExecutorConfig` declares in
//! `apcore/schemas/apcore-config.schema.json`, and that schema is
//! `additionalProperties: false`. There is no §9.15 `executor` registration
//! declaring more. **No spec-declared `executor` subkey is lost at load.**
//!
//! An **un**declared one was, until `PROTOCOL_SPEC` §9.14 made retention
//! normative for every framework section; see
//! [`unmodelled_executor_key_in_the_file_is_retained`], which used to assert
//! the opposite and carries the correction. The strict tier that rejects such
//! a key under `_config.strict: true` lives in
//! `test_config_unknown_framework_keys.rs`.
//!
//! What was broken is the namespace SURFACE over that struct:
//!
//! | reader | before | after |
//! |---|---|---|
//! | `get("executor")` | `None` | the executor object |
//! | `namespace("executor")` | `{}` | the executor object |
//! | `bind("executor")` | typed struct only | reconciled view |
//! | `data()["executor"]` after `set("executor.<unmodelled>", …)` | typed leaves **erased** | both |
//!
//! The first two contradicted `get("executor.max_call_depth")` on the same
//! config, which answered the file's value all along. apcore-python
//! (`Config.namespace` → `self._data["executor"]`) and apcore-typescript
//! (`namespace(name)` → `this._data[name]`) both return the object; Rust was
//! the only SDK returning nothing, because it is the only one that models the
//! namespace as a typed struct outside its data tree.
//!
//! ## Why the existing tests could not catch it
//!
//! `src/config.rs`'s 39 unit tests reach `Config` through `Config::default()`
//! or `serde_json::from_value`, and assert on `cfg.executor.<field>` — the
//! typed struct, which was never broken. Not one of them calls
//! `namespace("executor")` or `get("executor")`. **So every test here goes
//! through `Config::load` from a file on disk**, the path a deployment takes.

use apcore::config::{Config, ConfigMode, ExecutorConfig, MountSource};
use serde_json::{json, Value};

/// A namespace-mode `apcore.yaml` overriding two of the four executor leaves.
///
/// Two rather than four on purpose: `global_timeout` and `max_module_repeat`
/// are left to their defaults so every assertion below distinguishes "the file
/// won" from "the defaults happened to match".
const EXECUTOR_YAML: &str = r#"
apcore:
  version: "1.0"
executor:
  max_call_depth: 7
  default_timeout: 1234
"#;

/// Write `yaml` to a real `apcore.yaml` and load it the way a deployment does.
///
/// The `TempDir` is returned alongside the `Config` only to keep it alive;
/// dropping it would delete the file `reload()` refers back to.
fn load(yaml: &str) -> (tempfile::TempDir, Config) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("apcore.yaml");
    std::fs::write(&path, yaml).expect("write apcore.yaml");
    let config = Config::load(&path).expect("a real apcore.yaml must load");
    (dir, config)
}

fn loaded() -> (tempfile::TempDir, Config) {
    let (dir, config) = load(EXECUTOR_YAML);
    assert_eq!(
        config.mode,
        ConfigMode::Namespace,
        "these cases must exercise namespace mode, the shape a deployment uses"
    );
    (dir, config)
}

/// The full four-leaf object the reconciled `executor` namespace must present:
/// the file's two values, the defaults for the two it left alone.
fn expected_executor() -> Value {
    json!({
        "max_call_depth": 7,
        "default_timeout": 1234,
        "global_timeout": 60_000,
        "max_module_repeat": 3,
    })
}

/// Compare a `namespace()` map against a JSON object without depending on
/// `HashMap` iteration order.
fn assert_namespace_eq(actual: &std::collections::HashMap<String, Value>, expected: &Value) {
    let actual = Value::Object(actual.clone().into_iter().collect());
    assert_eq!(&actual, expected);
}

// ---------------------------------------------------------------------------
// The container fetch — `get("executor")`
// ---------------------------------------------------------------------------

/// `get("executor")` must return the namespace object.
///
/// This is the assertion that fails hardest before the fix: `None`, for a
/// config whose file plainly declares an `executor:` block, on the same
/// `Config` where `get("executor.max_call_depth")` returns `7`. `executor` is a
/// typed field, so `user_namespaces` held no entry for the dot-split fallback
/// in `get_direct` to traverse and the lookup simply ran off the end.
#[test]
fn get_executor_container_returns_the_namespace_object() {
    let (_dir, config) = loaded();
    match config.get("executor") {
        None => panic!(
            "`get(\"executor\")` came back None while `get(\"executor.max_call_depth\")` \
             returns the file's value — the container fetch contradicts its own leaf \
             (apcore-rust#34)"
        ),
        Some(actual) => assert_eq!(actual, expected_executor()),
    }
}

/// A CONTAINER fetch must agree with every LEAF fetch under it — the same
/// invariant #33 pinned for `observability.tracing`, one level up.
#[test]
fn get_executor_container_agrees_with_every_leaf() {
    let (_dir, config) = loaded();
    let container = config.get("executor").expect("container fetch");

    for leaf in [
        "max_call_depth",
        "default_timeout",
        "global_timeout",
        "max_module_repeat",
    ] {
        assert_eq!(
            container.get(leaf),
            config.get(&format!("executor.{leaf}")).as_ref(),
            "`get(\"executor\")[\"{leaf}\"]` disagrees with `get(\"executor.{leaf}\")`"
        );
    }
}

// ---------------------------------------------------------------------------
// namespace()
// ---------------------------------------------------------------------------

/// `namespace("executor")` must report the file, not an empty map.
///
/// Unlike `observability` there is no registered §9.15 default layer under
/// this namespace, so the failure mode was a missing value rather than a
/// confidently wrong one — an empty map where apcore-python and
/// apcore-typescript both hand back the object. Still fatal for the intended
/// use: `namespace()` is how a caller reads a namespace as a unit, and `bind`
/// is built on it.
#[test]
fn namespace_executor_reflects_the_file_not_an_empty_map() {
    let (_dir, config) = loaded();
    let ns = config.namespace("executor");

    assert!(
        !ns.is_empty(),
        "`namespace(\"executor\")` returned an EMPTY map for a config whose file \
         declares an `executor:` block — apcore-rust#34"
    );
    assert_namespace_eq(&ns, &expected_executor());
}

/// `namespace()` must agree with `get()` key for key. The two readers resolving
/// the same namespace differently is the class of bug #33 and #34 both are.
#[test]
fn namespace_executor_agrees_with_get() {
    let (_dir, config) = loaded();
    let ns = config.namespace("executor");
    // Without this the loop below is vacuous: an empty `namespace()` — the
    // pre-fix return value — iterates zero times and the test passes green
    // while asserting nothing.
    assert_eq!(
        ns.len(),
        4,
        "namespace(\"executor\") must carry all four modelled leaves, got {ns:?}"
    );
    for (key, value) in ns {
        assert_eq!(
            config.get(&format!("executor.{key}")),
            Some(value.clone()),
            "namespace() and get() disagree about `executor.{key}`"
        );
    }
}

// ---------------------------------------------------------------------------
// Typed-leaf invariance
// ---------------------------------------------------------------------------

/// Routing the namespace readers through `executor_view` must NOT change how
/// the four typed leaves resolve: `get_typed_field` still answers first, and
/// the view overlays the typed struct last.
#[test]
fn typed_executor_leaves_keep_resolving_from_the_typed_struct() {
    let (_dir, config) = loaded();

    for (key, expected) in [
        ("executor.max_call_depth", json!(7)),
        ("executor.default_timeout", json!(1234)),
        ("executor.global_timeout", json!(60_000)),
        ("executor.max_module_repeat", json!(3)),
    ] {
        assert_eq!(config.get(key), Some(expected.clone()), "get({key})");
    }

    // Same values, same config, through the typed struct itself.
    assert_eq!(config.executor.max_call_depth, 7);
    assert_eq!(config.executor.default_timeout, 1234);
    assert_eq!(config.executor.global_timeout, 60_000);
    assert_eq!(config.executor.max_module_repeat, 3);
}

/// A key the file declares under `executor:` that `$defs/ExecutorConfig` does
/// not declare is RETAINED and resolvable.
///
/// **CORRECTED.** This test previously asserted the opposite —
/// `get("executor.vendor_knob") == None` — on the reasoning that
/// `$defs/ExecutorConfig` is `additionalProperties: false`, so an undeclared
/// key is "a document the canonical schema rejects" rather than configuration
/// the SDK is dropping. It closed by calling the divergence from apcore-python
/// and apcore-typescript (both of which preserved the key) "a spec question for
/// the apcore repo".
///
/// `PROTOCOL_SPEC` §9.14 answered that question the other way. Closedness is
/// enforced under `_config.strict: true`, where the key raises `CONFIG_INVALID`
/// naming every offender; under the default it **MUST** be retained and
/// readable through `get()`, because "an SDK that models a section as a typed
/// record still has to keep what the record does not model" — "the operator
/// wrote it and it vanished" is indistinguishable from "the operator never
/// wrote it". So the peers were right and Rust was the outlier, and
/// `Config::deserialize` now stashes the raw `executor:` block exactly as it
/// does for `observability`.
///
/// The strict half lives in `test_config_unknown_framework_keys.rs`.
#[test]
fn unmodelled_executor_key_in_the_file_is_retained() {
    let (_dir, config) =
        load("apcore:\n  version: \"1.0\"\nexecutor:\n  max_call_depth: 7\n  vendor_knob: hello\n");

    assert_eq!(
        config.get("executor.vendor_knob"),
        Some(json!("hello")),
        "§9.14 default tier: an undeclared key inside a framework section MUST \
         be retained and readable through get()"
    );
    assert_eq!(
        config.get("executor.max_call_depth"),
        Some(json!(7)),
        "the schema-declared sibling must still load"
    );
    assert_eq!(
        config.get("executor"),
        Some(json!({
            "max_call_depth": 7,
            "default_timeout": 30_000,
            "global_timeout": 60_000,
            "max_module_repeat": 3,
            "vendor_knob": "hello",
        })),
        "the container fetch reports the modelled leaves AND the retained key"
    );
}

// ---------------------------------------------------------------------------
// set() precedence, in both directions
// ---------------------------------------------------------------------------

/// A runtime `set()` on a typed leaf must win over the file in EVERY reader.
///
/// `set` routes into the typed struct (`set_typed_field` matches first), so any
/// reader consulting a different store would report the file's stale value.
#[test]
fn set_on_a_typed_leaf_wins_over_the_file_in_every_reader() {
    let (_dir, mut config) = loaded();
    config.set("executor.max_call_depth", json!(99));

    assert_eq!(config.get("executor.max_call_depth"), Some(json!(99)));
    assert_eq!(
        config.get("executor").expect("container fetch")["max_call_depth"],
        json!(99),
        "a CONTAINER fetch must agree with the leaf fetch"
    );
    assert_eq!(
        config.namespace("executor")["max_call_depth"],
        json!(99),
        "namespace() must not resurrect the file's 7"
    );
    assert_eq!(
        config.data()["executor"]["max_call_depth"],
        json!(99),
        "data() must not resurrect the file's 7"
    );
}

/// A runtime `set()` on an UNMODELLED key must be visible AND must not erase
/// the four typed leaves.
///
/// This is the destructive half of #34, and the only half a file cannot reach.
/// `set("executor.vendor_knob", …)` falls past `set_typed_field` into
/// `user_namespaces`, creating a second store for the namespace; `Serialize`
/// then wrote the typed struct and let the flattened bag overwrite it, so
/// `data()["executor"]` became `{"vendor_knob": "x"}` — `max_call_depth: 7`,
/// straight from the operator's file, gone from the §9.1 wire form. Identical
/// mechanism to the `observability` clobber in #33; only the trigger differs.
#[test]
fn set_on_an_unmodelled_key_does_not_erase_the_typed_leaves() {
    let (_dir, mut config) = loaded();
    config.set("executor.vendor_knob", json!("x"));

    assert_eq!(
        config.get("executor.vendor_knob"),
        Some(json!("x")),
        "the value the caller set must be readable"
    );

    let data_executor = &config.data()["executor"];
    assert_eq!(
        data_executor["max_call_depth"],
        json!(7),
        "the file's `max_call_depth` was erased from the wire form by an \
         unrelated set() on a sibling key — apcore-rust#34"
    );
    assert_eq!(data_executor["default_timeout"], json!(1234));
    assert_eq!(data_executor["global_timeout"], json!(60_000));
    assert_eq!(data_executor["max_module_repeat"], json!(3));
    assert_eq!(data_executor["vendor_knob"], json!("x"));

    // …and the same object through the other two readers.
    assert_eq!(
        config.get("executor").expect("container fetch"),
        json!({
            "max_call_depth": 7,
            "default_timeout": 1234,
            "global_timeout": 60_000,
            "max_module_repeat": 3,
            "vendor_knob": "x",
        })
    );
    assert_eq!(config.namespace("executor")["vendor_knob"], json!("x"));
    assert_eq!(config.namespace("executor")["max_call_depth"], json!(7));
}

/// `mount("executor", …)` reaches the same second store as `set`, and must not
/// erase the typed leaves either.
#[test]
fn mount_on_executor_does_not_erase_the_typed_leaves() {
    let (_dir, mut config) = loaded();
    config
        .mount("executor", MountSource::Dict(json!({"vendor_knob": "y"})))
        .expect("mount executor");

    let data_executor = &config.data()["executor"];
    assert_eq!(
        data_executor["max_call_depth"],
        json!(7),
        "a mount must not wipe the file's executor values from the wire form"
    );
    assert_eq!(data_executor["vendor_knob"], json!("y"));
    assert_eq!(config.namespace("executor")["vendor_knob"], json!("y"));
}

// ---------------------------------------------------------------------------
// Absent / empty executor blocks
// ---------------------------------------------------------------------------

/// A file with NO `executor:` block must resolve the canonical defaults through
/// every reader — not `None` / `{}`.
///
/// `ExecutorConfig` has no optional leaf, so `get("executor.max_call_depth")`
/// has always answered `Some(32)` here. The container and the namespace must
/// answer consistently with it. (apcore-python and apcore-typescript answer
/// `None`/`{}` for an absent block in namespace mode and the defaults object in
/// legacy mode; Rust's typed struct makes the leaf unconditional in both, so
/// the container follows the leaf.)
#[test]
fn absent_executor_block_resolves_the_canonical_defaults() {
    let (_dir, config) = load("apcore:\n  version: \"1.0\"\n");

    let defaults = json!({
        "max_call_depth": 32,
        "default_timeout": 30_000,
        "global_timeout": 60_000,
        "max_module_repeat": 3,
    });

    assert_eq!(config.get("executor.max_call_depth"), Some(json!(32)));
    assert_eq!(config.get("executor"), Some(defaults.clone()));
    assert_namespace_eq(&config.namespace("executor"), &defaults);
    assert_eq!(config.data()["executor"], defaults);
    assert_eq!(
        config.get("executor.vendor_knob"),
        None,
        "an out-of-schema key must not be invented"
    );
}

/// An EMPTY `executor: {}` block behaves identically to the absent case, and
/// must not panic on the empty map.
#[test]
fn empty_executor_block_is_inert() {
    let (_dir, config) = load("apcore:\n  version: \"1.0\"\nexecutor: {}\n");

    let defaults = json!({
        "max_call_depth": 32,
        "default_timeout": 30_000,
        "global_timeout": 60_000,
        "max_module_repeat": 3,
    });
    assert_eq!(config.get("executor"), Some(defaults.clone()));
    assert_namespace_eq(&config.namespace("executor"), &defaults);
}

// ---------------------------------------------------------------------------
// data() — the §9.1 wire form
// ---------------------------------------------------------------------------

/// `data()` must carry the file's executor values, and must agree with `get()`.
#[test]
fn data_carries_the_files_executor_values() {
    let (_dir, config) = loaded();
    assert_eq!(config.data()["executor"], expected_executor());
    assert_eq!(
        config.data()["executor"],
        config.get("executor").expect("container fetch"),
        "data() and get() must resolve the same object"
    );
}

/// The wire form must survive `data()` → parse → `data()`, which is what a
/// cross-process config handoff does.
///
/// Only the schema-declared leaves are asserted to survive: `data()` also emits
/// anything a caller `set`/`mount`ed into the namespace, and `Config::deserialize`
/// hands the whole `executor` object to `ExecutorConfig`, which — per
/// `$defs/ExecutorConfig`'s `additionalProperties: false` — models no such key
/// and drops it. Round-tripping an out-of-schema key would require the raw-copy
/// treatment this issue deliberately does not apply; see
/// [`unmodelled_executor_key_in_the_file_is_not_resolvable`].
#[test]
fn data_round_trip_through_deserialize_is_stable() {
    let (_dir, config) = loaded();
    let once = config.data();
    let reparsed: Config = serde_json::from_value(once.clone()).expect("data() must reparse");
    assert_eq!(
        reparsed.data(),
        once,
        "a config serialized, reparsed and re-serialized must be identical — if \
         an executor leaf is dropped on the way through, this is where it shows"
    );
    assert_eq!(reparsed.data()["executor"], expected_executor());
}

// ---------------------------------------------------------------------------
// reload()
// ---------------------------------------------------------------------------

/// `reload()` re-reads the file through the same deserializer, so a fix
/// confined to the first load would leave a reloaded config answering `None`
/// again.
#[test]
fn reload_preserves_the_executor_namespace() {
    let (_dir, mut config) = loaded();
    config.reload().expect("reload from the stored path");

    assert_eq!(config.get("executor"), Some(expected_executor()));
    assert_namespace_eq(&config.namespace("executor"), &expected_executor());
    assert_eq!(config.get("executor.max_call_depth"), Some(json!(7)));
}

/// A mount replayed by `reload()` (§9.11) must still leave the typed leaves in
/// the wire form.
#[test]
fn reload_replays_an_executor_mount_without_erasing_the_typed_leaves() {
    let (_dir, mut config) = loaded();
    config
        .mount("executor", MountSource::Dict(json!({"vendor_knob": "y"})))
        .expect("mount executor");
    config.reload().expect("reload from the stored path");

    assert_eq!(config.get("executor.vendor_knob"), Some(json!("y")));
    assert_eq!(config.data()["executor"]["max_call_depth"], json!(7));
}

// ---------------------------------------------------------------------------
// bind()
// ---------------------------------------------------------------------------

/// `bind::<ExecutorConfig>("executor")` must keep returning the file's values.
/// The special case now binds the reconciled view rather than the struct, and
/// this pins that the typed path is unaffected.
#[test]
fn bind_executor_into_the_canonical_struct_is_unchanged() {
    let (_dir, config) = loaded();
    let bound: ExecutorConfig = config.bind("executor").expect("bind executor");

    assert_eq!(bound.max_call_depth, 7);
    assert_eq!(bound.default_timeout, 1234);
    assert_eq!(bound.global_timeout, 60_000);
    assert_eq!(bound.max_module_repeat, 3);
}

/// `bind` into a caller's OWN type must see the whole reconciled namespace,
/// including anything mounted into it — the point of `bind`, and the half that
/// the typed-struct special case previously stripped.
#[test]
fn bind_executor_into_a_custom_type_sees_the_mounted_extension() {
    #[derive(serde::Deserialize)]
    struct VendorExecutor {
        max_call_depth: u32,
        default_timeout: u64,
        vendor_knob: String,
    }

    let (_dir, mut config) = loaded();
    config
        .mount("executor", MountSource::Dict(json!({"vendor_knob": "y"})))
        .expect("mount executor");

    let bound: VendorExecutor = config.bind("executor").expect("bind executor");
    assert_eq!(bound.max_call_depth, 7);
    assert_eq!(bound.default_timeout, 1234);
    assert_eq!(
        bound.vendor_knob, "y",
        "bind returned the typed struct alone, so the mounted key was invisible"
    );
}
