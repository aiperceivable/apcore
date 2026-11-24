//! PROTOCOL_SPEC §10.6.1 requirement 2 — `regex_patterns` applies to STRING
//! values only, and a non-string value is never converted in order to test it.
//!
//! This SDK already conformed when the requirement was written; these cases
//! exist so it keeps conforming, and so the cross-language contract is legible
//! from inside this repository rather than only from the fixture.
//!
//! Each case asserts the half an implementation could otherwise skip. A
//! stringifying implementation redacts MORE, so it passes every test that only
//! checks a secret was caught — the discriminating assertion is that an
//! ordinary number was left alone. And the mirror of that: declining to convert
//! a container must not turn into skipping what is INSIDE it, which is the half
//! apcore-typescript was missing for array elements.
//!
//! Why the rule is a prohibition rather than a definition: for the one value
//! `{"a": 1}` the three host languages render `{'a': 1}`, `[object Object]` and
//! `{"a":1}`, and for `true` they render `True`, `true` and `true`. A matching
//! rule defined over a per-language rendering is three rules.

use apcore::observability::redaction::RedactionConfig;
use serde_json::{json, Value};

/// Patterns chosen to match the RENDERING of each non-string value in at least
/// one of the three languages: `[0-9]+` matches `42`, `true` matches Rust's and
/// TypeScript's rendering of a boolean, `a` matches Python's rendering of
/// `{"a": 1}`. Every one of them must nonetheless leave its value untouched.
fn config() -> RedactionConfig {
    RedactionConfig::builder()
        .sensitive_keys(Vec::<String>::new())
        .value_patterns(["[0-9]+", "true", "a"])
        .try_build()
        .expect("these patterns compile")
}

fn redacted(cfg: &RedactionConfig, mut value: Value) -> Value {
    cfg.redact(&mut value);
    value
}

#[test]
fn a_number_is_not_tested_against_the_patterns() {
    let payload = json!({ "amount": 42, "ratio": 1.5 });
    assert_eq!(
        redacted(&config(), payload.clone()),
        payload,
        "`[0-9]+` matches \"42\", so an implementation that stringifies redacts these. The \
         damage from that is not a leak but its opposite: ordinary numeric telemetry \
         disappearing from logs."
    );
}

#[test]
fn a_boolean_is_not_tested_against_the_patterns() {
    let payload = json!({ "flag": true });
    assert_eq!(
        redacted(&config(), payload.clone()),
        payload,
        "the pattern is spelled `true`, which is how this runtime and JavaScript render a \
         boolean and is NOT how Python renders one — so a stringifying implementation gives a \
         different answer per language for one value"
    );
}

#[test]
fn null_is_not_tested_against_the_patterns() {
    let payload = json!({ "missing": null });
    assert_eq!(redacted(&config(), payload.clone()), payload);
}

#[test]
fn a_container_is_not_matched_against_its_own_rendering() {
    // `a` occurs in Python's `str({'a': 1})` and not in JavaScript's
    // `String({a: 1})`. Neither rendering is the contract.
    let payload = json!({ "mapping": { "b": 1 }, "listing": [1, 2] });
    assert_eq!(redacted(&config(), payload.clone()), payload);
}

#[test]
fn a_string_is_still_tested() {
    // The other half: requirement 2 narrows the rule, it does not remove it.
    assert_eq!(
        redacted(&config(), json!({ "text": "order 42" })),
        json!({ "text": "***REDACTED***" })
    );
}

#[test]
fn a_string_inside_an_object_is_still_reached() {
    let cfg = RedactionConfig::builder()
        .sensitive_keys(Vec::<String>::new())
        .value_patterns(["sk-"])
        .try_build()
        .expect("compiles");
    assert_eq!(
        redacted(
            &cfg,
            json!({ "nested": { "key": "sk-secret", "count": 7 } })
        ),
        json!({ "nested": { "key": "***REDACTED***", "count": 7 } })
    );
}

#[test]
fn a_string_inside_an_array_is_still_reached() {
    // The case apcore-typescript failed: it handed an array element back to its
    // recursive entry point, which consults the value rule only from the
    // named-field path, so a secret in a list came back in plaintext there
    // while this SDK's `redact_inner(item, None)` replaced it. Pinned here so
    // the agreement is asserted from both sides rather than only from the one
    // that was broken.
    let cfg = RedactionConfig::builder()
        .sensitive_keys(Vec::<String>::new())
        .value_patterns(["sk-"])
        .try_build()
        .expect("compiles");
    assert_eq!(
        redacted(&cfg, json!({ "items": ["sk-secret", 7] })),
        json!({ "items": ["***REDACTED***", 7] })
    );
}
