//! `observability.*` subkeys must survive `Config::load` from a real YAML file
//! (apcore-rust#33).
//!
//! ## Why this file exists as a separate suite
//!
//! `Config::observability` is a typed [`ObservabilityConfig`] struct modelling
//! exactly four leaves — `tracing.enabled`, `tracing.sampling_rate`,
//! `tracing.exporter`, `metrics.enabled` — and it sits outside the
//! `user_namespaces` bag. `Config::deserialize` therefore handed the whole
//! `observability:` block to that struct and **discarded every subkey the
//! struct did not model**, at parse time, before any accessor could run. The
//! §9.15.2 namespace registration declares all of these configurable:
//!
//! | family | keys |
//! |---|---|
//! | `redaction` | `sensitive_keys`, `regex_patterns`, `replacement` |
//! | `tracing` (modelled since spec v1.44.0) | `strategy`, `otlp_endpoint` |
//! | `metrics` (unmodelled leaf) | `exporter` |
//! | `logging` | `enabled`, `level`, `format`, `redact_sensitive` |
//! | `error_history` | `max_entries_per_module`, `max_total_entries` |
//! | `platform_notify` | `enabled`, `error_rate_threshold`, `latency_p99_threshold_ms` |
//!
//! ## Why the existing tests could not catch it
//!
//! Every pre-existing redaction-config test builds its `Config` with
//! `Config::from_defaults()` + `.set(…)`. `set` writes straight into
//! `user_namespaces`, skipping deserialization entirely — the one step that was
//! broken. A test on that path asserts against a state a deployment could not
//! reach, and passes while production silently drops the operator's config.
//!
//! **So every test here goes through `Config::load` from a file on disk.**
//! Nothing in this file may be rewritten to use `Config::set`: that would
//! delete the only thing being tested.
//!
//! ## The half that is worse than a missing value
//!
//! `namespace("observability")` deep-merges the registered §9.15.2 defaults
//! under the loaded subtree. With the subtree discarded there was nothing to
//! overlay, so it returned **the default in place of the operator's value** —
//! a file saying `logging.enabled: false` read back `true`. A missing value is
//! diagnosable; a confidently-wrong one is not.
//! [`namespace_observability_reflects_the_file_not_the_registered_default`]
//! pins that directly.

use apcore::config::{Config, ConfigMode};
use serde_json::{json, Value};

/// A namespace-mode `apcore.yaml` carrying every observability family at once.
///
/// One file rather than one per test, deliberately: the defect dropped the
/// whole `observability` object, so a per-family file would let a partial fix
/// (one family rescued, the rest still discarded) pass every case. Loading all
/// families together means one assertion failing names exactly which family
/// regressed.
const OBSERVABILITY_YAML: &str = r#"
apcore:
  version: "1.0"
observability:
  tracing:
    enabled: true
    sampling_rate: 0.25
    exporter: otlp
    strategy: error_first
    otlp_endpoint: "http://collector.internal:4318"
  metrics:
    enabled: true
    exporter: prometheus
  logging:
    enabled: false
    level: debug
    format: text
    redact_sensitive: false
  redaction:
    sensitive_keys: ["username", "session"]
    regex_patterns: ["^sk-[A-Za-z0-9]+$"]
    replacement: "<HIDDEN>"
  error_history:
    max_entries_per_module: 5
    max_total_entries: 7
  platform_notify:
    enabled: true
    error_rate_threshold: 0.42
    latency_p99_threshold_ms: 1234.0
"#;

/// Write `yaml` to a real `apcore.yaml` and load it the way a deployment does.
///
/// The `TempDir` is returned alongside the `Config` only to keep it alive;
/// dropping it would delete the file that `reload()` and `source_path()` refer
/// back to.
fn load(yaml: &str) -> (tempfile::TempDir, Config) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("apcore.yaml");
    std::fs::write(&path, yaml).expect("write apcore.yaml");
    let config = Config::load(&path).expect("a real apcore.yaml must load");
    (dir, config)
}

fn loaded() -> (tempfile::TempDir, Config) {
    let (dir, config) = load(OBSERVABILITY_YAML);
    assert_eq!(
        config.mode,
        ConfigMode::Namespace,
        "these cases must exercise namespace mode, the shape a deployment uses"
    );
    (dir, config)
}

/// Assert `key` resolves to `expected` through `Config::get`.
///
/// Reports `None` distinctly from a wrong value: `None` is the parse-time
/// discard this issue is about, a wrong value would be a precedence bug.
fn assert_get(config: &Config, key: &str, expected: &Value) {
    match config.get(key) {
        None => panic!(
            "`{key}` was written into a real apcore.yaml and came back None — \
             the observability subtree is being discarded at deserialization \
             again (apcore-rust#33)"
        ),
        Some(actual) => assert_eq!(
            &actual, expected,
            "`{key}` came back with the wrong value — the file says {expected}"
        ),
    }
}

// ---------------------------------------------------------------------------
// One test per observability family, all through Config::load
// ---------------------------------------------------------------------------

/// `observability.redaction.*` — the family that motivated the issue.
///
/// apcore-rust#32 taught `RedactionConfig::from_config` to fall back to this
/// legacy spelling, but the fallback was starved: the subtree it reads was
/// discarded before `from_config` ever ran, so #32's production behaviour was
/// half-dead while its `Config::set`-based tests passed.
#[test]
fn load_preserves_observability_redaction_subtree() {
    let (_dir, config) = loaded();
    assert_get(
        &config,
        "observability.redaction.sensitive_keys",
        &json!(["username", "session"]),
    );
    assert_get(
        &config,
        "observability.redaction.regex_patterns",
        &json!(["^sk-[A-Za-z0-9]+$"]),
    );
    assert_get(
        &config,
        "observability.redaction.replacement",
        &json!("<HIDDEN>"),
    );
}

/// `observability.tracing.strategy` / `.otlp_endpoint` — once the unmodelled
/// leaves sitting *beside* modelled ones, now typed fields themselves.
///
/// The sharpest shape of the original defect: `tracing.enabled` survived and
/// `tracing.strategy` did not, from the same YAML mapping. An operator had no
/// way to infer that half their `tracing:` block was being read.
///
/// **Spec v1.44.0 changed why this passes, not whether it does.** Both keys are
/// now `TracingConfig` fields with a closed value set, because §9.15.2 declared
/// them and `schemas/apcore-config.schema.json` did not — so `_config.strict`
/// rejected them as unknown keys while the specification documented their
/// defaults (#118, D-68 C'). The value here changed from `sampled`, which was
/// never one of §10.7's four and survived only because nothing validated an
/// untyped passthrough leaf. That it had to change is the point: this file
/// pinned that the string arrived intact and could not have noticed that no
/// string was ever legal.
///
/// `otlp_endpoint` is the one with teeth. §9.15.2 defaults it to `null`; a
/// service that sets it and has its value dropped exports spans nowhere while
/// reporting tracing as enabled.
#[test]
fn load_preserves_unmodelled_observability_tracing_leaves() {
    let (_dir, config) = loaded();
    assert_get(
        &config,
        "observability.tracing.strategy",
        &json!("error_first"),
    );
    assert_get(
        &config,
        "observability.tracing.otlp_endpoint",
        &json!("http://collector.internal:4318"),
    );
}

/// `observability.metrics.exporter` — unmodelled leaf next to the modelled
/// `metrics.enabled`.
#[test]
fn load_preserves_unmodelled_observability_metrics_leaf() {
    let (_dir, config) = loaded();
    assert_get(
        &config,
        "observability.metrics.exporter",
        &json!("prometheus"),
    );
}

/// `observability.logging.*` — the family the issue's probe used, because
/// `logging.enabled` is the one whose default (`true`) is the *opposite* of
/// what an operator disabling it writes. Discarding it did not degrade to
/// "unconfigured", it inverted the setting.
#[test]
fn load_preserves_observability_logging_family() {
    let (_dir, config) = loaded();
    assert_get(&config, "observability.logging.enabled", &json!(false));
    assert_get(&config, "observability.logging.level", &json!("debug"));
    assert_get(&config, "observability.logging.format", &json!("text"));
    assert_get(
        &config,
        "observability.logging.redact_sensitive",
        &json!(false),
    );
}

/// `observability.error_history.*` — bounded-retention knobs. Dropping these
/// silently restored the 50/1000 defaults, so an operator who lowered them to
/// bound memory kept the original footprint.
#[test]
fn load_preserves_observability_error_history_family() {
    let (_dir, config) = loaded();
    assert_get(
        &config,
        "observability.error_history.max_entries_per_module",
        &json!(5),
    );
    assert_get(
        &config,
        "observability.error_history.max_total_entries",
        &json!(7),
    );
}

/// `observability.platform_notify.*` — alerting thresholds. The default is
/// `enabled: false`, so a discarded `enabled: true` meant alerts an operator
/// had explicitly turned on never fired.
#[test]
fn load_preserves_observability_platform_notify_family() {
    let (_dir, config) = loaded();
    assert_get(
        &config,
        "observability.platform_notify.enabled",
        &json!(true),
    );
    assert_get(
        &config,
        "observability.platform_notify.error_rate_threshold",
        &json!(0.42),
    );
    assert_get(
        &config,
        "observability.platform_notify.latency_p99_threshold_ms",
        &json!(1234.0),
    );
}

// ---------------------------------------------------------------------------
// namespace() — the confidently-wrong-value half
// ---------------------------------------------------------------------------

/// `namespace("observability")` must report the FILE, not the registered
/// §9.15.2 default.
///
/// This is the assertion the issue singled out as the worse half. `namespace()`
/// deep-merges the registration's defaults as the base layer and overlays the
/// loaded subtree; with the subtree discarded, the base layer *was* the answer.
/// The probe in the issue wrote `logging.enabled: false` and read back `true`.
///
/// Every key checked here is one whose file value differs from its registered
/// default, so a regression cannot pass by coincidence. The un-overridden
/// `logging.level` default is checked too — the overlay must ADD to the
/// defaults, not replace the map wholesale.
#[test]
fn namespace_observability_reflects_the_file_not_the_registered_default() {
    let (_dir, config) = loaded();
    let ns = config.namespace("observability");

    let logging = ns.get("logging").expect("logging present");
    assert_eq!(
        logging["enabled"],
        json!(false),
        "namespace() returned the registered default `true` for \
         `logging.enabled` while the file says `false` — the exact \
         confidently-wrong value apcore-rust#33 is about"
    );
    assert_eq!(logging["format"], json!("text"));

    let tracing = ns.get("tracing").expect("tracing present");
    assert_eq!(
        tracing["strategy"],
        json!("error_first"),
        "default is 'full'"
    );
    assert_eq!(
        tracing["otlp_endpoint"],
        json!("http://collector.internal:4318"),
        "default is null"
    );

    assert_eq!(
        ns.get("metrics").expect("metrics present")["exporter"],
        json!("prometheus"),
        "default is 'stdout'"
    );
    assert_eq!(
        ns.get("error_history").expect("error_history present")["max_total_entries"],
        json!(7),
        "default is 1000"
    );
    assert_eq!(
        ns.get("platform_notify").expect("platform_notify present")["enabled"],
        json!(true),
        "default is false"
    );
    assert_eq!(
        ns.get("redaction").expect("redaction present")["replacement"],
        json!("<HIDDEN>"),
        "the registration declares no `redaction` block at all, so this key \
         exists only if the file's subtree survived the load"
    );

    // A registered default the file does NOT override must still be present:
    // the loaded subtree overlays the defaults, it does not replace them.
    assert_eq!(
        ns.get("logging").expect("logging present")["redact_sensitive"],
        json!(false),
        "file overrides this one"
    );
    assert_eq!(
        ns.get("platform_notify").expect("platform_notify present")["latency_p99_threshold_ms"],
        json!(1234.0)
    );
}

// ---------------------------------------------------------------------------
// Precedence: the four typed leaves must behave exactly as before
// ---------------------------------------------------------------------------

/// Keeping the raw object in `user_namespaces` must NOT change how the four
/// typed leaves resolve. `get_direct` consults `get_typed_field` first and
/// `observability_view` overlays the typed struct last, so the typed struct
/// stays authoritative in `get()`, `namespace()`, `data()` and `bind()` alike.
#[test]
fn typed_observability_leaves_keep_resolving_from_the_typed_struct() {
    let (_dir, config) = loaded();

    for (key, expected) in [
        ("observability.tracing.enabled", json!(true)),
        ("observability.tracing.sampling_rate", json!(0.25)),
        ("observability.tracing.exporter", json!("otlp")),
        ("observability.metrics.enabled", json!(true)),
    ] {
        assert_eq!(config.get(key), Some(expected.clone()), "get({key})");
    }

    // Same values, same config, through the typed struct itself.
    assert!(config.observability.tracing.enabled);
    assert!((config.observability.tracing.sampling_rate - 0.25).abs() < f64::EPSILON);
    assert_eq!(config.observability.tracing.exporter, "otlp");
    assert!(config.observability.metrics.enabled);

    // …and through namespace(), which must agree with get().
    let ns = config.namespace("observability");
    assert_eq!(ns["tracing"]["enabled"], json!(true));
    assert_eq!(ns["tracing"]["sampling_rate"], json!(0.25));
    assert_eq!(ns["tracing"]["exporter"], json!("otlp"));
    assert_eq!(ns["metrics"]["enabled"], json!(true));
}

/// A runtime `set()` on a typed leaf must still win over the file's copy of
/// that leaf, in every reader.
///
/// This is the "two sources for one key" question the fix has to answer out
/// loud. After the fix the file's `tracing.enabled: true` lives BOTH in the
/// typed struct and in `user_namespaces`. `set` routes to the typed struct
/// (`set_typed_field` matches first) and leaves the raw copy stale, so any
/// reader that consulted the raw bag directly would resurrect the old value.
#[test]
fn set_on_a_typed_leaf_wins_over_the_files_stale_raw_copy() {
    let (_dir, mut config) = loaded();
    config.set("observability.tracing.enabled", json!(false));

    assert_eq!(
        config.get("observability.tracing.enabled"),
        Some(json!(false))
    );
    assert_eq!(
        config.namespace("observability")["tracing"]["enabled"],
        json!(false),
        "namespace() must not resurrect the file's `true` from the raw bag"
    );
    assert_eq!(
        config.data()["observability"]["tracing"]["enabled"],
        json!(false),
        "data() must not resurrect the file's `true` from the raw bag"
    );
    assert_eq!(
        config
            .get("observability.tracing")
            .expect("container fetch")["enabled"],
        json!(false),
        "a CONTAINER fetch must agree with the leaf fetch"
    );
}

/// A runtime `set()` on an UNMODELLED key must win over the file, and must not
/// wipe its siblings.
///
/// `set` and the loader now write to the same `user_namespaces` tree, which is
/// the point: one store, last write wins, no shadow entry. `set` replaces only
/// the addressed leaf, so the rest of the file's subtree survives.
#[test]
fn set_on_an_unmodelled_key_wins_over_the_file_without_clobbering_siblings() {
    let (_dir, mut config) = loaded();
    config.set("observability.logging.level", json!("trace"));

    assert_eq!(
        config.get("observability.logging.level"),
        Some(json!("trace")),
        "the runtime set must win over the file's `debug`"
    );
    assert_eq!(
        config.get("observability.logging.enabled"),
        Some(json!(false)),
        "the file's sibling key must survive a set() on its neighbour"
    );
    assert_eq!(
        config.get("observability.redaction.replacement"),
        Some(json!("<HIDDEN>")),
        "an unrelated family must survive a set()"
    );
}

// ---------------------------------------------------------------------------
// Absent / empty observability blocks
// ---------------------------------------------------------------------------

/// A file with NO `observability:` block resolves through the registered
/// §9.15.2 defaults in BOTH readers.
///
/// Until spec v1.17.0 `get` consulted only the typed fields and the flat
/// `CONFIG_DEFAULTS` table, so an unmodelled key such as
/// `observability.logging.enabled` answered `None` here while
/// `namespace("observability")["logging"]["enabled"]` answered `true` — and
/// apcore-python and apcore-typescript both answered `true` from either reader
/// (verified 2026-08-27). `get` now consults the registration defaults too.
#[test]
fn absent_observability_block_falls_back_to_defaults() {
    let (_dir, config) = load("apcore:\n  version: \"1.0\"\n");

    assert_eq!(
        config.get("observability.tracing.enabled"),
        Some(json!(false)),
        "typed default"
    );
    assert_eq!(
        config.get("observability.logging.enabled"),
        None,
        "logging is not part of the §9.15.2 observability namespace"
    );
    assert_eq!(
        config.get("observability").is_some(),
        true,
        "the namespace key resolves to its registered defaults, as it does in \
         apcore-python and apcore-typescript"
    );
    assert_eq!(
        config.get_declared("observability"),
        None,
        "`get_declared` still reports absence — required-field validation \
         depends on that distinction, and the registration lookup lives in \
         `get`, not in the `get_direct` that `get_declared` delegates to"
    );

    let ns = config.namespace("observability");
    assert!(
        ns.get("logging").is_none(),
        "with nothing loaded, namespace() exposes only its registered blocks"
    );
    assert_eq!(ns["tracing"]["strategy"], json!("full"));
    assert_eq!(ns["metrics"]["exporter"], json!("stdout"));
}

/// A file with an EMPTY `observability:` block must behave like the absent
/// case for values, without panicking on the empty map.
///
/// YAML renders `observability:` with no children as `null`, which the typed
/// struct rejects, so the empty shape an operator can actually write is
/// `observability: {}`.
#[test]
fn empty_observability_block_is_inert() {
    let (_dir, config) = load("apcore:\n  version: \"1.0\"\nobservability: {}\n");

    assert_eq!(
        config.get("observability.tracing.enabled"),
        Some(json!(false)),
        "typed default, unchanged"
    );
    assert_eq!(
        config.get("observability.logging.enabled"),
        None,
        "an empty block does not invent a logging configuration family"
    );

    let ns = config.namespace("observability");
    assert!(
        ns.get("logging").is_none(),
        "an empty block exposes only the registered defaults"
    );
    assert_eq!(ns["tracing"]["strategy"], json!("full"));
}

// ---------------------------------------------------------------------------
// data() — the §9.1 wire form must match what get() resolves
// ---------------------------------------------------------------------------

/// `data()` must carry the unmodelled subkeys AND the typed leaves.
///
/// `#[derive(Serialize)]` wrote the typed `observability` field and then the
/// flattened `user_namespaces` bag into the same map, so once the bag held an
/// `observability` entry the second write won and the typed leaves vanished
/// from the wire form entirely. That was latent before this fix — reachable
/// only via `set()` on an unmodelled key — and would have become the normal
/// path for every loaded file. `Config`'s hand-written `Serialize` emits the
/// reconciled view once instead.
#[test]
fn data_round_trips_both_typed_leaves_and_unmodelled_subkeys() {
    let (_dir, config) = loaded();
    let observability = &config.data()["observability"];

    assert_eq!(observability["tracing"]["enabled"], json!(true));
    assert_eq!(observability["tracing"]["sampling_rate"], json!(0.25));
    assert_eq!(observability["tracing"]["exporter"], json!("otlp"));
    assert_eq!(observability["metrics"]["enabled"], json!(true));

    assert_eq!(observability["tracing"]["strategy"], json!("error_first"));
    assert_eq!(
        observability["metrics"]["exporter"],
        json!("prometheus"),
        "an unmodelled leaf must not be dropped from the §9.1 wire form"
    );
    assert_eq!(observability["logging"]["enabled"], json!(false));
    assert_eq!(
        observability["redaction"]["sensitive_keys"],
        json!(["username", "session"])
    );
    assert_eq!(
        observability["error_history"]["max_total_entries"],
        json!(7)
    );
    assert_eq!(observability["platform_notify"]["enabled"], json!(true));
}

/// A file declaring ONLY an unmodelled family must still carry the typed
/// leaves' canonical defaults in `data()`.
///
/// This is the case the all-families file cannot see. When the YAML happens to
/// declare every typed leaf, a serializer that lets the raw bag overwrite the
/// typed field still emits the right numbers — by coincidence. Declaring only
/// `logging:` removes the coincidence: `observability.tracing` and
/// `observability.metrics` exist in the wire form only if the typed struct was
/// merged in rather than overwritten.
#[test]
fn data_keeps_typed_defaults_when_the_file_declares_only_an_unmodelled_family() {
    let (_dir, config) =
        load("apcore:\n  version: \"1.0\"\nobservability:\n  logging:\n    enabled: false\n");
    let observability = &config.data()["observability"];

    assert_eq!(observability["logging"]["enabled"], json!(false));
    assert_eq!(
        observability["tracing"]["exporter"],
        json!("stdout"),
        "the typed defaults must survive alongside an unmodelled family — a \
         serializer that writes the raw bag over the typed field drops the \
         whole `tracing` object here"
    );
    assert_eq!(observability["tracing"]["enabled"], json!(false));
    assert_eq!(observability["tracing"]["sampling_rate"], json!(1.0));
    assert_eq!(observability["metrics"]["enabled"], json!(false));
}

/// The wire form must survive a `data()` → parse → `data()` round-trip, which
/// is what a reload or a cross-process config handoff does.
#[test]
fn data_round_trip_through_deserialize_is_stable() {
    let (_dir, config) = loaded();
    let once = config.data();
    let reparsed: Config = serde_json::from_value(once.clone()).expect("data() must reparse");
    assert_eq!(
        reparsed.data(),
        once,
        "a config serialized, reparsed and re-serialized must be identical — \
         if an observability subkey is dropped on the way through, this is \
         where it shows"
    );
}

/// `reload()` must not lose the subkeys either: it re-reads the file through
/// the same deserializer, so a fix confined to `Config::load` would still leave
/// a reloaded config half-empty.
#[test]
fn reload_preserves_observability_subkeys() {
    let (_dir, mut config) = loaded();
    config.reload().expect("reload from the stored path");

    assert_eq!(
        config.get("observability.logging.enabled"),
        Some(json!(false))
    );
    assert_eq!(
        config.get("observability.redaction.replacement"),
        Some(json!("<HIDDEN>"))
    );
    assert_eq!(
        config.namespace("observability")["platform_notify"]["enabled"],
        json!(true)
    );
}

/// `bind` on the `observability` namespace must see the unmodelled subkeys.
///
/// `bind` special-cases this namespace to the typed struct; before the fix that
/// meant a caller's own type received a payload with `redaction`, `logging`,
/// `error_history` and `platform_notify` stripped out, indistinguishable from
/// "unconfigured".
#[test]
fn bind_observability_sees_unmodelled_subkeys() {
    #[derive(serde::Deserialize)]
    struct Logging {
        enabled: bool,
        level: String,
    }
    #[derive(serde::Deserialize)]
    struct Obs {
        logging: Logging,
        redaction: Redaction,
    }
    #[derive(serde::Deserialize)]
    struct Redaction {
        replacement: String,
    }

    let (_dir, config) = loaded();
    let obs: Obs = config.bind("observability").expect("bind observability");
    assert!(!obs.logging.enabled);
    assert_eq!(obs.logging.level, "debug");
    assert_eq!(obs.redaction.replacement, "<HIDDEN>");
}
