//! Issue #32 — which Config key `RedactionConfig::from_config` reads, and what
//! an operator-supplied `sensitive_keys` list does to the shipped defaults.
//!
//! Two rules are pinned here, neither of which any existing test could catch:
//!
//! 1. **Key resolution (D-53).** The canonical namespace is `obs.redaction.*`
//!    (`features/observability.md`, "Canonical Config keys (cross-SDK)"). This
//!    SDK read ONLY the deprecated `observability.redaction.*` spellings, so an
//!    operator following the documentation had their entire redaction policy
//!    silently discarded. Canonical now wins; legacy still works and warns.
//!
//! 2. **Replace, not merge (D-54).** "Operators override by setting
//!    `obs.redaction.sensitive_keys` in `apcore.yaml` (override **replaces**
//!    the default; it does not merge)." The three-way distinction matters:
//!    absent or `null` -> the shipped defaults; an explicit `[]` -> nothing is
//!    redacted by name. An empty override is an override, not "unset".
//!
//! The payload keys below are chosen to DISCRIMINATE, which
//! `conformance/fixtures/redaction_config.json` does not: its
//! `sensitive_keys: []` case uses keys that either match a default anyway
//! (`auth_header` contains `auth`) or match nothing at all, so it passes
//! whether the empty list is honoured or ignored. `username` matches no
//! default entry; `password` and `_secret_token` match two different kinds of
//! default entry (bare substring and the `_secret_*` glob).
//!
//! The one-shot deprecation WARNING is asserted in the `redaction.rs` unit
//! tests, which can reset the process-global one-shot flag; this binary can
//! only observe the first legacy read in the whole process.

use apcore::config::Config;
use apcore::observability::redaction::{RedactionConfig, DEFAULT_REPLACEMENT};
use serde_json::json;

const CANONICAL_SENSITIVE_KEYS: &str = "obs.redaction.sensitive_keys";
const CANONICAL_REGEX_PATTERNS: &str = "obs.redaction.regex_patterns";
const CANONICAL_REPLACEMENT: &str = "obs.redaction.replacement";
const LEGACY_SENSITIVE_KEYS: &str = "observability.redaction.sensitive_keys";
const LEGACY_REGEX_PATTERNS: &str = "observability.redaction.regex_patterns";
const LEGACY_REPLACEMENT: &str = "observability.redaction.replacement";

/// `password` and `_secret_token` are covered by the shipped defaults;
/// `username` is covered by none of the 16 entries.
fn discriminating_payload() -> serde_json::Value {
    json!({
        "password": "hunter2",
        "_secret_token": "abc",
        "username": "alice",
    })
}

fn redacted(cfg: &RedactionConfig) -> serde_json::Value {
    let mut payload = discriminating_payload();
    cfg.redact(&mut payload);
    payload
}

// ---------------------------------------------------------------------------
// D-54 — absent vs. null vs. explicitly empty
// ---------------------------------------------------------------------------

#[test]
fn absent_sensitive_keys_falls_back_to_shipped_defaults() {
    let cfg = RedactionConfig::from_config(&Config::from_defaults());
    assert_eq!(
        redacted(&cfg),
        json!({
            "password": DEFAULT_REPLACEMENT,
            "_secret_token": DEFAULT_REPLACEMENT,
            "username": "alice",
        }),
        "an unconfigured sensitive_keys must apply the canonical 16-entry default"
    );
}

#[test]
fn explicit_null_sensitive_keys_falls_back_to_shipped_defaults() {
    // apcore-python coerces a `None` back to the default list rather than
    // silently disabling key-based redaction; apcore-typescript treats
    // `null` like `undefined`. A YAML `sensitive_keys:` with no value is the
    // realistic way to hit this.
    let mut config = Config::from_defaults();
    config.set(CANONICAL_SENSITIVE_KEYS, json!(null));

    let cfg = RedactionConfig::from_config(&config);
    assert_eq!(
        redacted(&cfg),
        json!({
            "password": DEFAULT_REPLACEMENT,
            "_secret_token": DEFAULT_REPLACEMENT,
            "username": "alice",
        }),
        "an explicit null means `not configured`, not `redact nothing`"
    );
}

/// An explicit `null` means "not configured", so resolution continues to the
/// deprecated key rather than stopping at the canonical one. Without this the
/// `null`-means-unset rule is only observable through the default list, which a
/// naive `config.get(k).as_array().unwrap_or_default()` also happens to produce.
#[test]
fn explicit_null_canonical_key_falls_through_to_the_legacy_key() {
    let mut config = Config::from_defaults();
    config.set(CANONICAL_SENSITIVE_KEYS, json!(null));
    config.set(LEGACY_SENSITIVE_KEYS, json!(["legacy_key"]));

    let cfg = RedactionConfig::from_config(&config);
    assert!(
        cfg.field_matches("legacy_key"),
        "a null canonical key is unset, so the deprecated key still decides"
    );
    assert!(
        !cfg.field_matches("password"),
        "the legacy list is an override too — it replaces the defaults"
    );
}

#[test]
fn explicitly_empty_sensitive_keys_disables_key_based_redaction() {
    let mut config = Config::from_defaults();
    config.set(CANONICAL_SENSITIVE_KEYS, json!([]));

    let cfg = RedactionConfig::from_config(&config);
    assert!(
        !cfg.field_matches("password"),
        "an explicitly empty override MUST replace the defaults, not be \
         re-read as `unset`"
    );
    assert!(
        !cfg.field_matches("_secret_token"),
        "`_secret_*` is entry [0] of the default list, not a separate \
         hardcoded rule — an empty override drops it too"
    );
    assert_eq!(
        redacted(&cfg),
        discriminating_payload(),
        "nothing may be redacted by NAME once the operator configured an \
         empty sensitive_keys list"
    );
}

#[test]
fn non_empty_override_replaces_rather_than_merges() {
    let mut config = Config::from_defaults();
    config.set(CANONICAL_SENSITIVE_KEYS, json!(["username"]));

    let cfg = RedactionConfig::from_config(&config);
    assert_eq!(
        redacted(&cfg),
        json!({
            // `password` is in the DEFAULT list; a replacing override drops it,
            // so an operator can NARROW the policy and not only widen it.
            "password": "hunter2",
            "_secret_token": "abc",
            "username": DEFAULT_REPLACEMENT,
        }),
        "the override list must be the whole policy (D-54)"
    );
}

#[test]
fn value_regex_survives_an_empty_key_list() {
    let mut config = Config::from_defaults();
    config.set(CANONICAL_SENSITIVE_KEYS, json!([]));
    config.set(CANONICAL_REGEX_PATTERNS, json!(["^sk-[A-Za-z0-9]+$"]));

    let cfg = RedactionConfig::from_config(&config);
    let mut payload = json!({ "anything": "sk-abc123", "plain": "hello" });
    cfg.redact(&mut payload);
    assert_eq!(
        payload,
        json!({ "anything": DEFAULT_REPLACEMENT, "plain": "hello" }),
        "the value rule is independent of the key list"
    );
}

// ---------------------------------------------------------------------------
// D-53 — which key is read
// ---------------------------------------------------------------------------

#[test]
fn canonical_obs_redaction_keys_are_read() {
    // The regression this pins: every one of these three settings used to be
    // discarded, because only the `observability.redaction.*` spelling was
    // consulted. An operator narrowing their policy got the full default set.
    let mut config = Config::from_defaults();
    config.set(CANONICAL_SENSITIVE_KEYS, json!(["canonical_key"]));
    config.set(CANONICAL_REGEX_PATTERNS, json!(["^sk-[A-Za-z0-9]+$"]));
    config.set(CANONICAL_REPLACEMENT, json!("<HIDDEN>"));

    let cfg = RedactionConfig::from_config(&config);
    assert_eq!(cfg.replacement(), "<HIDDEN>", "obs.redaction.replacement");
    assert!(
        cfg.field_matches("canonical_key"),
        "obs.redaction.sensitive_keys"
    );
    assert!(
        cfg.value_matches("sk-abc123"),
        "obs.redaction.regex_patterns"
    );
    assert!(
        !cfg.field_matches("password"),
        "the canonical list replaced the defaults"
    );
}

#[test]
fn legacy_observability_redaction_keys_are_still_honoured() {
    // Backwards compatibility for deployments written against this SDK before
    // the canonical namespace was read. The accompanying one-shot deprecation
    // warning is asserted in the `redaction.rs` unit tests.
    let mut config = Config::from_defaults();
    config.set(LEGACY_SENSITIVE_KEYS, json!(["legacy_key"]));
    config.set(LEGACY_REGEX_PATTERNS, json!(["^sk-[A-Za-z0-9]+$"]));
    config.set(LEGACY_REPLACEMENT, json!("<LEGACY>"));

    let cfg = RedactionConfig::from_config(&config);
    assert_eq!(cfg.replacement(), "<LEGACY>");
    assert!(cfg.field_matches("legacy_key"));
    assert!(cfg.value_matches("sk-abc123"));
}

#[test]
fn canonical_keys_win_when_both_are_present() {
    let mut config = Config::from_defaults();
    config.set(LEGACY_SENSITIVE_KEYS, json!(["legacy_only"]));
    config.set(CANONICAL_SENSITIVE_KEYS, json!(["canonical_only"]));
    config.set(LEGACY_REPLACEMENT, json!("<LEGACY>"));
    config.set(CANONICAL_REPLACEMENT, json!("<CANONICAL>"));

    let cfg = RedactionConfig::from_config(&config);
    assert!(cfg.field_matches("canonical_only"));
    assert!(
        !cfg.field_matches("legacy_only"),
        "the canonical key must take precedence, not merge with the legacy one"
    );
    assert_eq!(cfg.replacement(), "<CANONICAL>");
}
