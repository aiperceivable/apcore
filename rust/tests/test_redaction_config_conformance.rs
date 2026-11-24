//! Drive `redaction_config.json` — config-driven redaction
//! (docs/features/observability.md#redaction-configuration, D-54).
//!
//! Constructor note: the derived `RedactionConfig::default()` yields ZERO
//! rules and an EMPTY replacement string — it is `#[derive(Default)]`, not the
//! canonical default policy. The fixture's `use_defaults: true` means the
//! canonical 16-entry `sensitive_keys` list, which in this SDK is
//! `RedactionConfig::defaults()` / `::with_default_sensitive_keys()`. This
//! driver calls the latter; using `default()` would make the case vacuous.
//!
//! Two case SHAPES live in this fixture and are dispatched separately:
//!
//! * `redaction_config` cases hand the driver a ready-made rule block. They
//!   pin the MATCHING semantics — substring, case-insensitivity, the canonical
//!   default key list, the never-redact set.
//! * `config` cases hand the driver a map keyed by CONFIG DOT-PATH
//!   (`obs.redaction.sensitive_keys` /
//!   `observability.redaction.sensitive_keys`) and go through
//!   [`RedactionConfig::from_config`]. They pin WHICH key path an
//!   implementation reads — the fixture's
//!   `driver_contract.which_key_is_read_is_part_of_the_contract`. Each
//!   dot-path is handed to `Config::set` VERBATIM; rewriting it to whatever
//!   this SDK happens to read would defeat the whole point of the case, which
//!   exists because apcore-rust once read only the legacy path and silently
//!   discarded a documented redaction policy (apcore-rust#32).
//!
//! How this file divides the work with `tests/test_redaction_config_keys.rs`
//! and the `src/observability/redaction.rs` unit tests. All three are needed;
//! none is a copy of another:
//!
//! | Question | Owned by |
//! |---|---|
//! | Do all SDKs agree on which key path is read? | this file, from the canonical fixture |
//! | absent vs. `null` vs. explicit `[]`; replace-not-merge (D-54) | `tests/test_redaction_config_keys.rs` |
//! | Is the legacy-key deprecation warning emitted, and one-shot? | `redaction.rs` unit tests |
//!
//! That last row is not a matter of taste. The one-shot bookkeeping is a
//! process-global `AtomicBool` that only `redaction.rs` can reset, and several
//! tests in this shared `it` binary read a legacy key — whichever runs first
//! consumes the single warning. An integration test asserting
//! `deprecation_warning_emitted: true` would pass or fail on test ORDER. So
//! this driver asserts the fixture's BEHAVIOURAL expectation for the legacy
//! case (the legacy list must actually take effect), asserts the `false`
//! expectation on the canonical case (silence is order-independent), opens the
//! warning text when the race happened to be won, and delegates the `true`
//! expectations to the unit test that owns the flag — with
//! `conformance_deprecation_warning_expectations_are_delegated` below going red
//! if the fixture ever changes what is being delegated.

use std::io::Write;
use std::sync::{Arc, Mutex};

use apcore::config::Config;
use apcore::executor::redact_sensitive;
use apcore::observability::redaction::RedactionConfig;
use serde_json::{json, Value};

use crate::conformance_env::find_fixtures_root;

fn fixture() -> Value {
    let path = find_fixtures_root().join("redaction_config.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("redaction_config.json parses")
}

/// Expectation keys in a `config` case that describe the deprecation WARNING
/// rather than a field of the redacted payload. See the module docs for why
/// the `true` half is executed by the `redaction.rs` unit tests instead of
/// here; `conformance_deprecation_warning_expectations_are_delegated` keeps the
/// delegation honest.
const WARNING_EMITTED: &str = "deprecation_warning_emitted";
const WARNING_IS_ONE_SHOT: &str = "deprecation_warning_is_one_shot";

/// The canonical spelling this SDK MUST read, and the deprecated one it MUST
/// still honour. Spelled out so a failure message can say what was expected;
/// the fixture, not this list, decides which path each case exercises.
const CANONICAL_SENSITIVE_KEYS: &str = "obs.redaction.sensitive_keys";
/// The pre-D-53 spelling THIS SDK shipped. Per-SDK history, not a contract.
const LEGACY_SENSITIVE_KEYS: &str = "observability.redaction.sensitive_keys";

#[derive(Clone, Default)]
struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

impl Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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

/// Run `f` under a THREAD-LOCAL subscriber and return everything it logged.
/// Thread-local (not global) so a driver running beside the rest of the `it`
/// binary neither steals nor is polluted by other tests' output.
fn capture_logs(f: impl FnOnce()) -> String {
    let buf = CaptureWriter::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buf.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    tracing::subscriber::with_default(subscriber, f);
    let bytes = buf
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn string_list(value: &Value) -> Vec<String> {
    match value {
        Value::Null => Vec::new(),
        Value::Array(a) => a
            .iter()
            .map(|v| v.as_str().expect("pattern is a string").to_string())
            .collect(),
        other => panic!("expected an array or null, got {other}"),
    }
}

/// Build the SDK object the case describes.
fn build(id: &str, spec: &Value) -> RedactionConfig {
    let regexes = string_list(&spec["regex_patterns"]);
    let keys = string_list(&spec["sensitive_keys"]);
    let replacement = spec["replacement"]
        .as_str()
        .unwrap_or_else(|| panic!("[{id}] redaction_config.replacement missing"));

    if spec["use_defaults"].as_bool() == Some(true) {
        // The canonical default policy. `defaults()` fixes both the key list
        // and the replacement string, so the case can only be driven honestly
        // when it asks for nothing else on top.
        assert!(
            keys.is_empty() && regexes.is_empty(),
            "[{id}] use_defaults cases cannot also declare rules: \
             RedactionConfig::defaults() takes no extra input"
        );
        let cfg = RedactionConfig::defaults();
        assert_eq!(
            cfg.replacement(),
            replacement,
            "[{id}] the SDK's canonical default replacement must match the fixture"
        );
        return cfg;
    }

    RedactionConfig::builder()
        .value_patterns(regexes)
        .sensitive_keys(keys)
        .replacement(replacement)
        .try_build()
        .unwrap_or_else(|e| panic!("[{id}] fixture patterns must compile: {e}"))
}

/// Cases held out of the bulk loop, each with a stated reason. Every id listed
/// here MUST also have a test of its own below, so the held-out expectation
/// stays executable rather than disappearing.
///
/// Empty: `correlation_fields_never_redacted` was held out until
/// `RedactionConfig::redact_inner` threaded `NEVER_REDACT_FIELDS` into the
/// value-regex arm as well as the field-name arm. It now runs in the main loop.
const QUARANTINED: &[&str] = &[];

fn case_by_id(fx: &Value, id: &str) -> Value {
    fx["test_cases"]
        .as_array()
        .expect("test_cases is an array")
        .iter()
        .find(|tc| tc["id"].as_str() == Some(id))
        .unwrap_or_else(|| panic!("redaction_config.json no longer carries case `{id}`"))
        .clone()
}

/// This SDK's key in the fixture's `legacy_key_by_sdk` map.
const SDK_LANGUAGE: &str = "rust";

/// The `(dot_path, value)` pairs a `config`-shaped case wants written into a
/// [`Config`], or `None` when the case does not apply to this SDK.
///
/// Three shapes are accepted, and anything else panics rather than being
/// skipped:
///
/// * `config` — a flat dot-path map that applies to every SDK. Used by
///   `canonical_config_key_is_read`, whose key IS the cross-language contract.
/// * `legacy_key_by_sdk` — the LEGACY spelling is per-SDK history, not a
///   contract (fixture `driver_contract.legacy_spelling_is_per_sdk_history`).
///   This driver reads the `rust` entry only. A `null` entry means the case
///   does not apply and MUST be skipped with the fixture's stated reason, not
///   xfailed as though the SDK were deficient — that is apcore-python, which
///   never shipped an `observability.*` namespace and MUST NOT grow one.
/// * `config_canonical` + `legacy_value` — both spellings at once.
fn config_pairs(tc: &Value, id: &str) -> Option<Vec<(String, Value)>> {
    let mut pairs: Vec<(String, Value)> = Vec::new();

    if let Some(flat) = tc["config"].as_object() {
        assert!(
            !flat.is_empty(),
            "[{id}] `config` must name at least one key path — an empty map \
             would assert nothing about which key is read"
        );
        for (dot_path, value) in flat {
            pairs.push((dot_path.clone(), value.clone()));
        }
        return Some(pairs);
    }

    if let Some(canonical) = tc["config_canonical"].as_object() {
        for (dot_path, value) in canonical {
            pairs.push((dot_path.clone(), value.clone()));
        }
    }

    let by_sdk = tc["legacy_key_by_sdk"].as_object().unwrap_or_else(|| {
        panic!(
            "[{id}] case has neither `config` nor `legacy_key_by_sdk` — teach \
             this driver the new shape rather than skipping the case"
        )
    });
    let entry = by_sdk.get(SDK_LANGUAGE).unwrap_or_else(|| {
        panic!(
            "[{id}] `legacy_key_by_sdk` does not mention `{SDK_LANGUAGE}`; a \
             driver must not guess its own legacy spelling"
        )
    });
    if entry.is_null() {
        return None;
    }
    let legacy_key = entry
        .as_str()
        .unwrap_or_else(|| panic!("[{id}] `legacy_key_by_sdk.{SDK_LANGUAGE}` must be a string"));

    // `driver_contract.a_case_must_carry_its_own_inputs`: the case names both
    // the key to write AND the value to write. A driver that invents the value
    // is not executing a cross-language contract, it is guessing — and three
    // drivers guessing alike today is not agreement, it is coincidence. So this
    // is a hard failure, never a default.
    let legacy_value = tc.get("legacy_value").cloned().unwrap_or_else(|| {
        panic!(
            "[{id}] names `legacy_key_by_sdk.{SDK_LANGUAGE}` but carries no \
             `legacy_value`, so the config to build is underdetermined. The \
             fixture must state the value; this driver MUST NOT invent one."
        )
    });
    pairs.push((legacy_key.to_string(), legacy_value));

    assert!(
        !pairs.is_empty(),
        "[{id}] resolved to an empty config — nothing would be asserted"
    );
    Some(pairs)
}

/// Compare a redacted payload against the case's `expected` block.
///
/// `logs` is everything `from_config` emitted, so the deprecation-warning
/// expectations can be judged where that is sound. See the module docs for the
/// split between what is asserted here and what the unit tests own.
fn check_expectations(id: &str, tc: &Value, cfg: &RedactionConfig, logs: &str) -> Vec<String> {
    let mut payload = tc["input"].clone();
    let input_field_count = payload
        .as_object()
        .unwrap_or_else(|| panic!("[{id}] input must be an object to redact"))
        .len();
    cfg.redact(&mut payload);
    let got = payload.as_object().expect("redact keeps the object shape");

    let expected = tc["expected"]
        .as_object()
        .unwrap_or_else(|| panic!("[{id}] case has no expected object"));

    let mut failures = Vec::new();
    let warned = logs.contains("deprecated");

    for (field, want) in expected {
        if field == "_note" {
            continue;
        }
        if field == WARNING_EMITTED {
            match want.as_bool() {
                // Order-independent: a canonical-only config must never warn,
                // whatever the process-global one-shot flag already holds.
                Some(false) if warned => failures.push(format!(
                    "  [{id}] the canonical key path must stay silent, but logged: {logs}"
                )),
                Some(false) => {}
                // Cannot be asserted from a shared test binary — see the module
                // docs. When this test happened to win the race for the single
                // warning, at least check the text is useful to an operator.
                Some(true) if warned => {
                    let legacy_key = tc["legacy_key_by_sdk"][SDK_LANGUAGE]
                        .as_str()
                        .unwrap_or_default();
                    for needle in [legacy_key, CANONICAL_SENSITIVE_KEYS] {
                        if !needle.is_empty() && !logs.contains(needle) {
                            failures.push(format!(
                                "  [{id}] the deprecation warning must name `{needle}`: {logs}"
                            ));
                        }
                    }
                }
                Some(true) => {}
                None => failures.push(format!("  [{id}] `{WARNING_EMITTED}` must be a boolean")),
            }
            continue;
        }
        if field == WARNING_IS_ONE_SHOT {
            // Owned by `redaction.rs::legacy_redaction_keys_emit_a_one_shot_
            // deprecation_warning`, the only place that can reset the flag.
            continue;
        }
        match got.get(field.as_str()) {
            Some(actual) if actual == want => {}
            Some(actual) => failures.push(format!(
                "  [{id}] field `{field}`: expected {want}, got {actual}"
            )),
            None => failures.push(format!("  [{id}] field `{field}` missing after redact()")),
        }
    }

    // Unlike the `redaction_config` cases, a `config` case may name only the
    // fields that discriminate, so the expectation is a SUBSET. Redaction must
    // still neither invent nor drop fields.
    if got.len() != input_field_count {
        failures.push(format!(
            "  [{id}] field count changed: input had {input_field_count}, got {} ({:?})",
            got.len(),
            got.keys().collect::<Vec<_>>()
        ));
    }
    failures
}

/// Replay a `config`-shaped case: build a [`Config`] from the exact dot-paths
/// the case names, resolve a [`RedactionConfig`] out of it, and check the
/// resulting redaction.
///
/// The payload is deliberately DISCRIMINATING (fixture
/// `driver_contract.discriminating_payload`): `username` matches no entry of
/// the canonical default list while `password` and `_secret_token` both do, so
/// "the override was read and replaced the defaults" is distinguishable from
/// "the defaults are still in force". A payload whose keys all happen to match
/// a default stays green either way, which is how this defect survived in
/// every SDK.
///
/// CONSTRUCTION PATH: `Config::set`, which writes the dot-path straight into
/// the namespace tree. That is the only path on which this SDK's LEGACY
/// spelling is reachable at all — see
/// `conformance_canonical_config_key_is_read_from_a_real_apcore_yaml` below and
/// the finding it documents.
fn run_config_key_case(tc: &Value) -> Vec<String> {
    let id = tc["id"].as_str().expect("every case needs an id");

    let Some(pairs) = config_pairs(tc, id) else {
        // Unreachable for `rust` while the fixture keeps a non-null entry;
        // `conformance_legacy_case_applies_to_this_sdk` pins that.
        return Vec::new();
    };

    let mut config = Config::from_defaults();
    for (dot_path, value) in pairs {
        // VERBATIM. The case exists to pin this string.
        config.set(&dot_path, value);
    }

    let mut cfg = None;
    let logs = capture_logs(|| cfg = Some(RedactionConfig::from_config(&config)));
    let cfg = cfg.expect("from_config always returns a config");

    check_expectations(id, tc, &cfg, &logs)
}

/// Replay one case and return the assertion failures it produced.
fn run_case(tc: &Value) -> Vec<String> {
    let id = tc["id"].as_str().expect("every case needs an id");
    if tc["config"].is_object()
        || tc["config_canonical"].is_object()
        || tc["legacy_key_by_sdk"].is_object()
    {
        return run_config_key_case(tc);
    }
    assert!(
        tc["redaction_config"].is_object(),
        "[{id}] case carries neither a `redaction_config` block nor any \
         config-shaped key (`config` / `config_canonical` / \
         `legacy_key_by_sdk`) — teach this driver the new shape rather than \
         skipping the case"
    );
    let cfg = build(id, &tc["redaction_config"]);

    let mut payload = tc["input"].clone();
    assert!(
        payload.is_object(),
        "[{id}] input must be an object to redact"
    );
    if let Some(schema) = tc.get("schema") {
        payload = redact_sensitive(&payload, schema);
    }
    cfg.redact(&mut payload);

    let expected = tc["expected"]
        .as_object()
        .unwrap_or_else(|| panic!("[{id}] case has no expected object"));
    let got = payload.as_object().expect("redact keeps the object shape");

    let mut failures = Vec::new();
    for (field, want) in expected {
        // `_note` is prose the fixture attaches to the expectation block, not a
        // payload field. Every other field is compared.
        if field == "_note" {
            continue;
        }
        match got.get(field) {
            Some(actual) if actual == want => {}
            Some(actual) => failures.push(format!(
                "  [{id}] field `{field}`: expected {want}, got {actual}"
            )),
            None => failures.push(format!("  [{id}] field `{field}` missing after redact()")),
        }
    }

    // Redaction must not invent or drop fields.
    let expected_field_count = expected.keys().filter(|k| *k != "_note").count();
    if got.len() != expected_field_count {
        failures.push(format!(
            "  [{id}] field count changed: expected {expected_field_count}, got {} ({:?})",
            got.len(),
            got.keys().collect::<Vec<_>>()
        ));
    }
    failures
}

#[test]
fn conformance_redaction_config() {
    let fx = fixture();
    let cases = fx["test_cases"].as_array().expect("test_cases is an array");
    assert!(!cases.is_empty(), "fixture must carry at least one case");

    // A quarantined id that vanished from the fixture must fail loudly rather
    // than shrink this suite silently.
    for id in QUARANTINED {
        let _ = case_by_id(&fx, id);
    }

    let mut failures: Vec<String> = Vec::new();
    for tc in cases {
        let id = tc["id"].as_str().expect("every case needs an id");
        if QUARANTINED.contains(&id) {
            // Driven by `conformance_correlation_fields_never_redacted` below.
            continue;
        }
        failures.extend(run_case(tc));
    }

    assert!(
        failures.is_empty(),
        "redaction_config: {} assertion(s) diverge from the spec fixture:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Case `correlation_fields_never_redacted`, kept as its own named test so the
/// MUST it encodes stays visible in the run output.
///
/// `RedactionConfig::redact_inner` checks `NEVER_REDACT_FIELDS` before BOTH the
/// field-name rule and the value-regex rule, so `regex_patterns: [".*"]` leaves
/// `trace_id`, `caller_id`, `target_id`, `module_id`, and `span_id` intact —
/// matching apcore-typescript's `_shouldRedact` and apcore-python's
/// `_apply_redaction_config`, which both test the protected set first.
#[test]
fn conformance_correlation_fields_never_redacted() {
    let fx = fixture();
    let tc = case_by_id(&fx, "correlation_fields_never_redacted");
    let failures = run_case(&tc);
    assert!(
        failures.is_empty(),
        "correlation fields must survive every rule:\n{}",
        failures.join("\n")
    );
}

// ---------------------------------------------------------------------------
// `config` cases — WHICH config key path is read (apcore-rust#32, D-53)
//
// Each runs in the bulk loop above as well; the named copies exist so the MUST
// each one encodes is legible in `cargo test` output instead of hiding behind
// a single aggregate test name.
// ---------------------------------------------------------------------------

/// Canonical `obs.redaction.sensitive_keys` MUST be the key that is read.
/// Nothing else in this fixture pins the key PATH, which is exactly how this
/// SDK shipped reading only `observability.redaction.*`: an operator who
/// followed the documentation had their whole redaction policy discarded with
/// neither warning nor error.
#[test]
fn conformance_canonical_config_key_is_read() {
    let fx = fixture();
    let tc = case_by_id(&fx, "canonical_config_key_is_read");
    let failures = run_case(&tc);
    assert!(
        failures.is_empty(),
        "the canonical config key must be the one consulted:\n{}",
        failures.join("\n")
    );
}

/// The pre-canonical `observability.redaction.*` path MUST still be honoured —
/// refusing it outright would break deployments written against this SDK
/// before the canonical namespace was read. The accompanying one-shot warning
/// is asserted in `redaction.rs`; see the module docs.
#[test]
fn conformance_legacy_config_key_is_honoured() {
    let fx = fixture();
    let tc = case_by_id(
        &fx,
        "legacy_config_key_is_honoured_with_a_deprecation_warning",
    );
    let failures = run_case(&tc);
    assert!(
        failures.is_empty(),
        "the deprecated config key must still take effect:\n{}",
        failures.join("\n")
    );
}

/// With both spellings present the canonical one MUST win — not merge, and not
/// lose to the older key just because it was read second.
#[test]
fn conformance_canonical_config_key_wins_over_legacy() {
    let fx = fixture();
    let tc = case_by_id(&fx, "canonical_config_key_wins_over_legacy");
    let failures = run_case(&tc);
    assert!(
        failures.is_empty(),
        "the canonical key must take precedence over the deprecated one:\n{}",
        failures.join("\n")
    );
}

/// Pins WHAT is delegated, so the delegation cannot quietly become a skip.
///
/// `deprecation_warning_emitted: true` and `deprecation_warning_is_one_shot`
/// are executed by `src/observability/redaction.rs::
/// legacy_redaction_keys_emit_a_one_shot_deprecation_warning`, which is the
/// only code that can reset the process-global one-shot flag. If the fixture
/// ever revises those expectations — say by dropping the one-shot requirement,
/// or by demanding a warning on the canonical path — this test fails and sends
/// the reader to the unit test that has to change with it, rather than letting
/// the driver keep ignoring a key whose meaning moved.
#[test]
fn conformance_deprecation_warning_expectations_are_delegated() {
    let fx = fixture();

    let legacy = case_by_id(
        &fx,
        "legacy_config_key_is_honoured_with_a_deprecation_warning",
    );
    assert_eq!(
        legacy["expected"][WARNING_EMITTED],
        json!(true),
        "reading a deprecated key must warn — asserted in redaction.rs, \
         `legacy_redaction_keys_emit_a_one_shot_deprecation_warning`"
    );
    assert_eq!(
        legacy["expected"][WARNING_IS_ONE_SHOT],
        json!(true),
        "the warning must stay one-shot per process — asserted in redaction.rs, \
         `legacy_redaction_keys_emit_a_one_shot_deprecation_warning`"
    );

    let canonical = case_by_id(&fx, "canonical_config_key_is_read");
    assert_eq!(
        canonical["expected"][WARNING_EMITTED],
        json!(false),
        "the documented key path must stay silent — asserted both by \
         `conformance_canonical_config_key_is_read` above and by redaction.rs, \
         `canonical_redaction_keys_emit_no_deprecation_warning`"
    );
}

/// The legacy spelling is per-SDK history, so a driver that reads
/// `legacy_key_by_sdk` can be silently disarmed by an upstream edit: flip
/// `rust` to `null` and both legacy cases become no-ops that still report
/// green. This pins the entry against the spelling this SDK actually reads.
#[test]
fn conformance_legacy_case_applies_to_this_sdk() {
    let fx = fixture();
    for id in [
        "legacy_config_key_is_honoured_with_a_deprecation_warning",
        "canonical_config_key_wins_over_legacy",
    ] {
        let tc = case_by_id(&fx, id);
        let entry = &tc["legacy_key_by_sdk"][SDK_LANGUAGE];
        assert_eq!(
            entry.as_str(),
            Some(LEGACY_SENSITIVE_KEYS),
            "[{id}] this SDK shipped `{LEGACY_SENSITIVE_KEYS}` before D-53 and still \
             reads it (src/observability/redaction.rs). A `null` here would silently \
             turn the case into a no-op; a different spelling would mean the fixture \
             and the SDK disagree about this SDK's own history."
        );
    }
}

/// Render `(dot_path, value)` pairs as the nested `apcore.yaml` an operator
/// would actually write.
///
/// §9.6: the `apcore:` block selects namespace mode — the shape a real
/// deployment uses, and the one the peer SDKs materialize namespace defaults
/// into. Building the file (rather than calling `Config::set`) is the entire
/// point: a `set` on an unmodelled `observability.*` key wrote a
/// `user_namespaces` entry that no YAML file could produce, so a driver that
/// only uses `set` could not see a parse-time drop.
///
/// CORRECTED for apcore-rust#33: `Config::deserialize` now writes that same
/// `user_namespaces` entry from the file, so the two paths finally converge —
/// which is exactly why they no longer disagree here. The reason to keep
/// building a real file is unchanged and permanent: `set` bypasses
/// deserialization, so only a file can prove deserialization preserved the
/// key. Do not "simplify" these cases back to `Config::set`.
fn yaml_for(pairs: &[(String, Value)]) -> String {
    let mut tree = serde_json::Map::new();
    for (dot_path, value) in pairs {
        let parts: Vec<&str> = dot_path.split('.').collect();
        let mut cursor = &mut tree;
        for part in &parts[..parts.len() - 1] {
            cursor = cursor
                .entry((*part).to_string())
                .or_insert_with(|| Value::Object(serde_json::Map::new()))
                .as_object_mut()
                .expect("intermediate nodes are objects");
        }
        cursor.insert(parts[parts.len() - 1].to_string(), value.clone());
    }
    tree.insert("apcore".to_string(), json!({ "version": "1.0" }));
    serde_yaml_ng::to_string(&Value::Object(tree)).expect("serialize yaml")
}

/// The canonical key, read from a config built the way a DEPLOYMENT builds one:
/// a namespace-mode `apcore.yaml` on disk, loaded through `Config::load`.
///
/// Every other case here goes through `Config::set`, which writes a dot-path
/// straight into the namespace tree. That is a weaker path than production, and
/// a driver that only ever uses it can pass while the real config shape is
/// broken. This test closes that gap for the CANONICAL key — the one the
/// fixture makes a cross-language MUST.
///
/// The legacy key's counterpart is
/// `legacy_config_key_read_from_a_real_apcore_yaml` below, which was
/// `#[ignore]`d against apcore-rust#33 until that path was fixed.
#[test]
fn conformance_canonical_config_key_is_read_from_a_real_apcore_yaml() {
    let fx = fixture();
    let tc = case_by_id(&fx, "canonical_config_key_is_read");
    let id = "canonical_config_key_is_read (via apcore.yaml)";

    let pairs: Vec<(String, Value)> = tc["config"]
        .as_object()
        .expect("case has a `config` map")
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("apcore.yaml");
    std::fs::write(&path, yaml_for(&pairs)).expect("write apcore.yaml");

    let config = Config::load(&path).expect("a real apcore.yaml must load");
    assert_eq!(
        config.mode,
        apcore::config::ConfigMode::Namespace,
        "the probe must exercise namespace mode, not the legacy flat shape"
    );
    assert!(
        config.get(CANONICAL_SENSITIVE_KEYS).is_some(),
        "`{CANONICAL_SENSITIVE_KEYS}` written into a real apcore.yaml must be \
         reachable through Config::get, or every config-key case in this file is \
         only testing Config::set"
    );

    let mut cfg = None;
    let logs = capture_logs(|| cfg = Some(RedactionConfig::from_config(&config)));
    let failures = check_expectations(id, &tc, &cfg.expect("from_config"), &logs);
    assert!(
        failures.is_empty(),
        "the canonical key must be honoured when it comes from a real config file:\n{}",
        failures.join("\n")
    );
}

/// The LEGACY key, read from a real `apcore.yaml` — the mirror of
/// `conformance_canonical_config_key_is_read_from_a_real_apcore_yaml`.
///
/// Was `#[ignore]`d against **apcore-rust#33** and is now the acceptance check
/// for its fix. `Config::observability` is a typed struct modelling only
/// `tracing` and `metrics`, and it sits OUTSIDE the `user_namespaces` bag, so
/// `Config::deserialize` used to hand the whole `observability` object to that
/// struct and discard the `redaction` subtree at parse time. Nothing downstream
/// could recover it — `read_redaction_key` was handed a `Config` that never
/// contained the operator's value. `Config::deserialize` now also keeps the raw
/// object in `user_namespaces`, so the legacy key survives the load.
///
/// This was deliberately NOT written to assert the broken behaviour. A test
/// that pinned "the legacy key is ignored from YAML" would have made the defect
/// look intended and would have had to be deleted, not fixed.
///
/// Blast radius was wider than redaction — `tracing.strategy`,
/// `tracing.otlp_endpoint`, `metrics.exporter`, `logging.*`, `error_history.*`
/// and `platform_notify.*` are all declared configurable by the namespace
/// registration (spec §9.15.2) and were all dropped the same way. Those are
/// covered by `tests/test_config_load_observability_subkeys.rs`.
#[test]
fn legacy_config_key_read_from_a_real_apcore_yaml() {
    let fx = fixture();
    let tc = case_by_id(
        &fx,
        "legacy_config_key_is_honoured_with_a_deprecation_warning",
    );
    let id = "legacy_config_key_is_honoured (via apcore.yaml)";

    let legacy_key = tc["legacy_key_by_sdk"][SDK_LANGUAGE]
        .as_str()
        .expect("this SDK has a legacy spelling");
    let legacy_value = tc
        .get("legacy_value")
        .expect("the fixture states the value to write");

    let yaml = yaml_for(&[(legacy_key.to_string(), legacy_value.clone())]);
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("apcore.yaml");
    std::fs::write(&path, yaml).expect("write apcore.yaml");

    let config = Config::load(&path).expect("a real apcore.yaml must load");
    assert!(
        config.get(legacy_key).is_some(),
        "`{legacy_key}` written into a real apcore.yaml must survive \
         deserialization (apcore-rust#33 regression): if it is discarded again, \
         an operator's redaction policy is silently dropped — the exact failure \
         mode apcore-rust#32 fixed for the canonical key"
    );

    let mut cfg = None;
    let logs = capture_logs(|| cfg = Some(RedactionConfig::from_config(&config)));
    let failures = check_expectations(id, &tc, &cfg.expect("from_config"), &logs);
    assert!(
        failures.is_empty(),
        "the deprecated key must be honoured when it comes from a real config file:\n{}",
        failures.join("\n")
    );
}
