//! Issue #43 §5 — Config-driven redaction rules.
//!
//! `RedactionConfig::from_config()` reads the canonical keys (D-53)
//!   - `obs.redaction.sensitive_keys: Vec<String>` (name patterns)
//!   - `obs.redaction.regex_patterns: Vec<String>` (value regexes)
//!   - `obs.redaction.replacement: String`
//!
//! and applies the canonical 16-entry default key list when `sensitive_keys`
//! is not configured. Key resolution (canonical vs. the deprecated
//! `observability.redaction.*` spellings) and the absent / null / empty
//! three-way distinction are pinned in `test_redaction_config_keys.rs`.
//!
//! Issue #32: this file previously drove the LEGACY keys exclusively and
//! asserted that operator entries were merged into the defaults. Both were
//! wrong — see the correction note on `user_sensitive_keys_replace_defaults`.

use apcore::config::Config;
use apcore::observability::redaction::RedactionConfig;
use serde_json::json;

#[test]
fn defaults_redact_common_sensitive_keys() {
    let cfg = Config::from_defaults();
    let redaction = RedactionConfig::from_config(&cfg);

    let mut payload = json!({
        "api_key": "sk-test-1234",
        "token": "bearer-xyz",
        "authorization": "Bearer abc",
        "password": "hunter2",
        "_secret_db_url": "postgres://...",
        "username": "alice"
    });
    redaction.redact(&mut payload);

    assert_eq!(payload["api_key"], json!(redaction.replacement()));
    assert_eq!(payload["token"], json!(redaction.replacement()));
    assert_eq!(payload["authorization"], json!(redaction.replacement()));
    assert_eq!(payload["password"], json!(redaction.replacement()));
    assert_eq!(payload["_secret_db_url"], json!(redaction.replacement()));
    assert_eq!(payload["username"], json!("alice"));
}

/// CORRECTED (issue #32). This test used to be `user_sensitive_keys_extend_defaults`
/// and asserted that `api_key` was still redacted after the operator configured
/// `sensitive_keys: ["custom_token"]` — i.e. it encoded the union behaviour that
/// D-54 forbids: "Operators override by setting `obs.redaction.sensitive_keys`
/// in `apcore.yaml` (override **replaces** the default; it does not merge)".
/// The assertion is inverted rather than deleted, so the rule stays executable.
#[test]
fn user_sensitive_keys_replace_defaults() {
    let mut cfg = Config::from_defaults();
    cfg.set("obs.redaction.sensitive_keys", json!(["custom_token"]));

    let redaction = RedactionConfig::from_config(&cfg);
    let mut payload = json!({
        "custom_token": "abc",
        "api_key": "no longer covered - the override replaced the default list",
        "username": "alice"
    });
    redaction.redact(&mut payload);
    assert_eq!(payload["custom_token"], json!(redaction.replacement()));
    assert_eq!(
        payload["api_key"],
        json!("no longer covered - the override replaced the default list"),
        "an operator override REPLACES the default sensitive_keys (D-54); \
         a default entry the operator left out must no longer redact"
    );
    assert_eq!(payload["username"], json!("alice"));
}

#[test]
fn user_regex_patterns_redact_values() {
    let mut cfg = Config::from_defaults();
    cfg.set("obs.redaction.regex_patterns", json!([r"^Bearer\s+\S+"]));

    let redaction = RedactionConfig::from_config(&cfg);
    let mut payload = json!({
        "url": "https://api.example.com/data",
        "auth_header": "Bearer abc123",
    });
    redaction.redact(&mut payload);
    assert_eq!(payload["auth_header"], json!(redaction.replacement()));
    assert_eq!(payload["url"], json!("https://api.example.com/data"));
}

#[test]
fn case_insensitive_regex() {
    let mut cfg = Config::from_defaults();
    cfg.set("obs.redaction.regex_patterns", json!([r"secret-\w+"]));

    let redaction = RedactionConfig::from_config(&cfg);
    let mut payload = json!({
        "raw": "SECRET-XYZ",
    });
    redaction.redact(&mut payload);
    assert_eq!(
        payload["raw"],
        json!(redaction.replacement()),
        "regex_patterns should compile case-insensitive"
    );
}
