// TDD tests for expanded DEFAULT_SENSITIVE_KEYS canonical superset.
// Issue #43 §5 — match Python's _DEFAULT_SENSITIVE_KEYS list.

use apcore::observability::redaction::DEFAULT_SENSITIVE_KEYS;

#[test]
fn default_sensitive_keys_has_canonical_superset_length() {
    // Python ships 15 canonical entries plus the legacy `_secret_*` glob = 16.
    assert_eq!(
        DEFAULT_SENSITIVE_KEYS.len(),
        16,
        "DEFAULT_SENSITIVE_KEYS must contain the full canonical superset (16 entries)"
    );
}

#[test]
fn default_sensitive_keys_contains_all_canonical_entries() {
    let expected: &[&str] = &[
        "_secret_*",
        "password",
        "passwd",
        "secret",
        "token",
        "api_key",
        "apikey",
        "apiKey",
        "access_key",
        "private_key",
        "authorization",
        "auth",
        "credential",
        "cookie",
        "session",
        "bearer",
    ];
    for key in expected {
        assert!(
            DEFAULT_SENSITIVE_KEYS.contains(key),
            "DEFAULT_SENSITIVE_KEYS missing canonical entry: {key}"
        );
    }
}

#[test]
fn default_sensitive_keys_includes_camel_case_api_key() {
    assert!(DEFAULT_SENSITIVE_KEYS.contains(&"apiKey"));
    assert!(DEFAULT_SENSITIVE_KEYS.contains(&"api_key"));
    assert!(DEFAULT_SENSITIVE_KEYS.contains(&"apikey"));
}
