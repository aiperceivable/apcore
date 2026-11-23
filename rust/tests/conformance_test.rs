// Cross-language conformance tests driven by canonical JSON fixtures.
//
// Fixture source: apcore/conformance/fixtures/*.json (single source of truth).
//
// Fixture discovery order:
//   1. CONFORMANCE_SPEC_REPO env var
//   2. Sibling ../apcore/ directory (standard workspace layout & CI)

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use apcore::acl::{ACLRule, ACL};
use apcore::config::{Config, EnvStyle, NamespaceRegistration};
use apcore::context::{Context, Identity};
use apcore::errors::{ErrorCode, ErrorCodeRegistry};
use apcore::schema::SchemaValidator;
use apcore::utils::{
    calculate_specificity, guard_call_chain_with_repeat, match_glob, match_pattern,
    normalize_to_canonical_id,
};
use apcore::version::negotiate_version;

#[path = "conformance_env.rs"]
mod conformance_env;

use crate::conformance_env::{find_fixtures_root, find_schemas_root};

fn load_fixture(name: &str) -> Value {
    let path = find_fixtures_root().join(format!("{name}.json"));
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Failed to read fixture: {}", path.display()));
    serde_json::from_str(&content).unwrap_or_else(|e| panic!("Invalid JSON in {name}: {e}"))
}

/// Resolve a WIRE error code declared by a fixture (`ERROR_CODE_COLLISION`) to
/// the [`ErrorCode`] variant, through serde — the same spelling the SDK puts on
/// the wire.
///
/// Two failures this exists to prevent, both measured in apcore#92:
///
/// * asserting a Rust type name instead of the declared code, which stays green
///   however the fixture's code is changed — the contract then lives in the
///   driver's literal and the fixture is decoration; and
/// * skipping the assertion when the declared code is not recognised, which
///   turns a wrong fixture value into a passing test.
///
/// A code that does not resolve is a hard failure here, never a skipped branch.
fn wire_error_code(id: &str, fixture: &str, wire: &str) -> ErrorCode {
    serde_json::from_value(Value::String(wire.to_string())).unwrap_or_else(|e| {
        panic!(
            "FAIL [{id}]: {fixture} declares error code `{wire}`, which is not an apcore \
             ErrorCode ({e}) — teach the driver, do not skip it"
        )
    })
}

// ---------------------------------------------------------------------------
// 1. Pattern Matching (A09)
// ---------------------------------------------------------------------------

#[test]
fn conformance_pattern_matching() {
    let fixture = load_fixture("pattern_matching");
    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();
        let pattern = tc["pattern"].as_str().unwrap();
        let value = tc["value"].as_str().unwrap();
        let expected = tc["expected"].as_bool().unwrap();

        assert_eq!(
            match_pattern(pattern, value),
            expected,
            "FAIL [{id}]: match_pattern({pattern:?}, {value:?}) expected {expected}"
        );
    }
}

// ---------------------------------------------------------------------------
// 1b. Glob Matching (A25) — PROTOCOL_SPEC 9.2.3 (#116, #117)
//
// The matcher for every glob-dialect pattern-valued value EXCEPT module-ID
// matching: `bindings.pattern`, `obs.redaction.sensitive_keys` glob entries,
// event patterns, and `path_filter`. Kept in its own fixture rather than folded
// into `pattern_matching` on purpose — 9.2.3 requirement 5 states why the two
// algorithms are separate, and one shared fixture would hide it.
// ---------------------------------------------------------------------------

#[test]
fn conformance_glob_matching() {
    let fixture = load_fixture("glob_matching");
    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();
        let pattern = tc["pattern"].as_str().unwrap();
        let value = tc["value"].as_str().unwrap();
        let expected = tc["expected"].as_bool().unwrap();
        let why = tc["_why"].as_str().unwrap_or("");

        assert_eq!(
            match_glob(pattern, value),
            expected,
            "FAIL [{id}]: match_glob({pattern:?}, {value:?}) expected {expected} — {why}"
        );
    }
}

/// 9.2.3 requirement 2: every string is a valid pattern, so there is no parse
/// phase.
///
/// Pinned separately because the failure it guards is a PANIC or an `Err`, not
/// a wrong boolean. This is the requirement `glob::Pattern` violated — it
/// rejects `a[b` ("invalid range pattern") and `a**b` ("recursive wildcards
/// must form a single path component") — which is how the same control-plane
/// request came to be refused by this SDK and served by the other two.
#[test]
fn conformance_glob_matching_never_rejects_a_pattern() {
    for pattern in ["a[b", "a**b", "[!", "{a,b}", "\\", "***", "[]", "?", ""] {
        for value in ["", "a", "a[b", "executor.email.send"] {
            let _ = match_glob(pattern, value);
        }
    }
}

// ---------------------------------------------------------------------------
// 2. Specificity Scoring (A10)
// ---------------------------------------------------------------------------

#[test]
fn conformance_specificity() {
    let fixture = load_fixture("specificity");
    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();
        let pattern = tc["pattern"].as_str().unwrap();
        #[allow(clippy::cast_possible_truncation)] // specificity scores are small integers
        let expected = tc["expected_score"].as_u64().unwrap() as u32;

        assert_eq!(
            calculate_specificity(pattern),
            expected,
            "FAIL [{id}]: calculate_specificity({pattern:?}) expected {expected}"
        );
    }
}

// ---------------------------------------------------------------------------
// 3. ID Normalization (A02)
// ---------------------------------------------------------------------------

#[test]
fn conformance_normalize_id() {
    let fixture = load_fixture("normalize_id");
    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();
        let local_id = tc["local_id"].as_str().unwrap();
        let language = tc["language"].as_str().unwrap();

        if tc["expected_error"].as_bool().unwrap_or(false) {
            let expected_normalized = tc["expected_normalized"].as_str().unwrap();
            let error = normalize_to_canonical_id(local_id, language)
                .expect_err("invalid local IDs must be rejected");
            assert!(
                error
                    .message
                    .contains(&format!("Normalized ID '{expected_normalized}'")),
                "FAIL [{id}]: expected error to contain normalized form {expected_normalized:?}, got {:?}",
                error.message
            );
            continue;
        }

        let expected = tc["expected"].as_str().unwrap();

        let result = normalize_to_canonical_id(local_id, language)
            .unwrap_or_else(|e| panic!("FAIL [{id}]: normalize errored: {}", e.message));
        assert_eq!(
            result, expected,
            "FAIL [{id}]: normalize({local_id:?}, {language:?}) = {result:?}, expected {expected:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// 4. Version Negotiation (A14)
//
// CORRECTED (apcore#92): this driver branched on whether the `expected_error`
// KEY existed and then asserted only `is_err()`. The declared value never
// reached an assertion, so changing `VERSION_INCOMPATIBLE` in the fixture to
// anything at all left the suite green — measured, 3 of 10 cases could not be
// made to fail. The fixture's `driver_contract` block states the rule: the
// expectation is a wire code, and an expectation the driver does not recognise
// MUST be a hard failure rather than a skipped branch.
// ---------------------------------------------------------------------------

#[test]
fn conformance_version_negotiation() {
    let fixture = load_fixture("version_negotiation");
    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();
        let declared = tc["declared"].as_str().unwrap();
        let sdk = tc["sdk"].as_str().unwrap();
        let result = negotiate_version(declared, sdk);

        if let Some(expected_error) = tc.get("expected_error") {
            let wire = expected_error.as_str().unwrap_or_else(|| {
                panic!("FAIL [{id}]: expected_error must be a string, got {expected_error}")
            });
            let err = match result {
                Err(e) => e,
                Ok(v) => panic!("FAIL [{id}]: expected {wire} but negotiation returned {v:?}"),
            };
            match wire {
                // The fixture states PARSE_ERROR is language-specific: it is a
                // sentinel for "this semver string does not parse", not an
                // apcore wire code, and the Rust SDK has no ParseError variant.
                // `negotiate_version` reports the malformed input as
                // VERSION_INCOMPATIBLE with a message naming the offending
                // string, so both are asserted. Asserting only `is_err()` — the
                // apcore-python shape for this case — accepts an error raised
                // for any reason at all, including a panic in the parser.
                "PARSE_ERROR" => {
                    assert_eq!(
                        err.code,
                        ErrorCode::VersionIncompatible,
                        "FAIL [{id}]: an unparseable version is reported as \
                         VERSION_INCOMPATIBLE by this SDK; got {} ({})",
                        err.code.wire_str(),
                        err.message
                    );
                    assert!(
                        err.message.contains(declared),
                        "FAIL [{id}]: the parse failure must name the offending version \
                         {declared:?}; message was {:?}",
                        err.message
                    );
                }
                other => {
                    let want = wire_error_code(id, "version_negotiation.json", other);
                    assert_eq!(
                        err.code,
                        want,
                        "FAIL [{id}]: expected wire code {other}, got {} ({})",
                        err.code.wire_str(),
                        err.message
                    );
                }
            }
        } else {
            let expected = tc["expected"].as_str().unwrap();
            let negotiated = result.unwrap_or_else(|e| {
                panic!("FAIL [{id}]: expected Ok({expected}) but got {}", e.message)
            });
            assert_eq!(
                negotiated, expected,
                "FAIL [{id}]: negotiate({declared:?}, {sdk:?}) expected {expected:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 5. Call Chain Safety (A20)
//
// CORRECTED (apcore#93): the six positive cases — `valid_chain`, `empty_chain`,
// `single_element`, `frequency_within_limit`, `self_call_not_circular`,
// `default_depth_32_ok` — asserted only `result.is_ok()`. A guard that inspects
// nothing and returns `Ok(())` unconditionally satisfies that, so mutating
// their declared `"ok"` left apcore-rust green. The fixture's own
// `driver_contract.positive_cases_need_a_post_condition` says so in as many
// words. apcore-python pinned these during apcore#92 and the default sweep went
// green as soon as one SDK did, which is why the fix never reached here.
//
// Two things now make the declared expectation load-bearing:
//   * the `"ok"` sentinel is asserted rather than inferred from the ABSENCE of
//     `expected_error`, so an unrecognised expectation is a hard failure; and
//   * a BOUNDARY PROBE — re-running the same chain with the one limit it sits
//     under tightened by one, and requiring the matching rejection. That is the
//     observable post-condition: the guard accepted this chain because it is
//     within the limits, not because it looks at nothing.
// ---------------------------------------------------------------------------

#[test]
#[allow(clippy::too_many_lines)] // one case-by-case ladder over the fixture; splitting it would separate each expectation from the branch that states it
fn conformance_call_chain() {
    let fixture = load_fixture("call_chain");
    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();
        let module_id = tc["module_id"].as_str().unwrap();
        let call_chain: Vec<String> = tc["call_chain"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        #[allow(clippy::cast_possible_truncation)]
        // max_call_depth from fixtures is a small integer
        let max_depth = tc
            .get("max_call_depth")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(32) as usize;
        #[allow(clippy::cast_possible_truncation)]
        // max_module_repeat from fixtures is a small integer
        let max_repeat = tc
            .get("max_module_repeat")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(3) as usize;

        let identity = Identity::new(
            "test".to_string(),
            "user".to_string(),
            vec![],
            HashMap::new(),
        );
        let mut ctx: Context<Value> =
            Context::create(Some(identity), None, None, None, Value::Null, None);
        ctx.call_chain.clone_from(&call_chain);

        let result = guard_call_chain_with_repeat(&ctx, module_id, max_depth, max_repeat);

        if let Some(expected_error) = tc.get("expected_error").and_then(|v| v.as_str()) {
            assert!(
                tc.get("expected").is_none(),
                "FAIL [{id}]: states BOTH `expected` and `expected_error`; the case has no \
                 single declared outcome"
            );
            let Err(err) = result else {
                panic!("FAIL [{id}]: expected error {expected_error} but got Ok")
            };
            match expected_error {
                // The codes the fixture names are real wire codes, so resolve
                // them THROUGH serde and compare the variant. A message
                // substring ("depth", "circular") stays green when the fixture's
                // declared code changes, which is the shape apcore#92 caught.
                "CALL_DEPTH_EXCEEDED" | "CIRCULAR_CALL" | "CALL_FREQUENCY_EXCEEDED" => {
                    assert_eq!(
                        err.code,
                        wire_error_code(id, "call_chain", expected_error),
                        "FAIL [{id}]: wire code; message was {}",
                        err.message
                    );
                }
                // Non-positive limit floor. Since D-84 (spec v1.49.0) the
                // fixture pins the WIRE CODE `GENERAL_INVALID_INPUT` for all
                // three SDKs; it used to carry the placeholder `INVALID_LIMIT`
                // because Python raised `ValueError` and TypeScript a bare
                // `Error`, neither of which a cross-language caller can catch.
                // The placeholder is NOT accepted here: the fixture in this
                // repo's pinned spec version no longer emits it, and the
                // apcore-python and apcore-typescript drivers dropped their
                // aliases too. Accepting a spelling no fixture produces would
                // let the three drivers drift apart again, silently.
                "GENERAL_INVALID_INPUT" => {
                    assert_eq!(
                        err.code,
                        ErrorCode::GeneralInvalidInput,
                        "FAIL [{id}]: the invalid-limit floor MUST carry the typed \
                         GENERAL_INVALID_INPUT (D-84); message was {}",
                        err.message
                    );
                    assert!(
                        err.message.contains(">= 1"),
                        "FAIL [{id}]: expected the floor to be named in the message, got: {}",
                        err.message
                    );
                    assert!(
                        err.ai_guidance.is_some(),
                        "FAIL [{id}]: D-84's typed error carries ai_guidance like its peers"
                    );
                }
                other => panic!(
                    "FAIL [{id}]: call_chain declares expected_error {other:?}, which this \
                     driver does not know — teach the driver, do not skip it"
                ),
            }
        } else {
            // Positive case. The sentinel VALUE is asserted, not inferred from
            // the absence of `expected_error`: a fixture that states an
            // expectation this driver does not recognise must go red, never
            // slide into the positive branch.
            let expected = tc.get("expected").unwrap_or_else(|| {
                panic!("FAIL [{id}]: states neither `expected` nor `expected_error`")
            });
            assert_eq!(
                expected, "ok",
                "FAIL [{id}]: states expectation {expected}, which this driver does not \
                 recognise. Teach the driver, do not skip it."
            );
            result.unwrap_or_else(|e| panic!("FAIL [{id}]: expected Ok but got Err({e})"));

            // OBSERVABLE POST-CONDITION (fixture `driver_contract`). A guard
            // that inspects nothing also returns Ok, so re-run the same chain
            // with the one limit it sits under tightened by one and require the
            // matching rejection. Skipped where the chain cannot be pushed over
            // a LEGAL limit — `empty_chain` and `single_element`, since
            // max_* < 1 is itself invalid input.
            //
            // (There is no Rust analogue of apcore-python's "must not mutate
            // the caller's chain" post-condition: the guard takes `&Context`,
            // so the borrow checker already forbids it.)
            let depth = call_chain.len();
            if depth >= 2 {
                let err = guard_call_chain_with_repeat(&ctx, module_id, depth - 1, max_repeat)
                    .expect_err(&format!(
                        "FAIL [{id}]: a chain of {depth} accepted under max_call_depth \
                         {} must be rejected under {}",
                        depth,
                        depth - 1
                    ));
                assert_eq!(
                    err.code,
                    ErrorCode::CallDepthExceeded,
                    "FAIL [{id}]: boundary probe; message was {}",
                    err.message
                );
            }
            let repeats = call_chain.iter().filter(|m| *m == module_id).count();
            if repeats >= 2 {
                let err = guard_call_chain_with_repeat(&ctx, module_id, max_depth, repeats - 1)
                    .expect_err(&format!(
                        "FAIL [{id}]: {module_id} appears {repeats} times and was accepted \
                         under max_module_repeat {max_repeat}; it must be rejected under {}",
                        repeats - 1
                    ));
                assert_eq!(
                    err.code,
                    ErrorCode::CallFrequencyExceeded,
                    "FAIL [{id}]: boundary probe; message was {}",
                    err.message
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 6. Error Code Collision (A17)
//
// CORRECTED (apcore#92): this driver branched on whether the `expected_error`
// KEY existed and then asserted only `is_err()` / `is_ok()`. The fixture's
// declared WIRE code never reached an assertion, and the nine `expected: "ok"`
// cases asserted nothing beyond "did not return Err" — an `ErrorCodeRegistry`
// that stored nothing at all passed every one of them. Measured, 12 of 18 cases
// could not be made to fail by mutating their expectation.
//
// The fixture's `driver_contract` block states the three rules now followed:
// assert the wire code and never a class name; the VALUE must reach an
// assertion, not the key's presence; and a positive case needs an OBSERVABLE
// POST-CONDITION — here, that the registration is queryable afterwards.
// ---------------------------------------------------------------------------

/// The outcome a case declares: `Some(code)` from `expected_error`, or `None`
/// from the `"ok"` sentinel.
///
/// A case that states neither, both, or an `expected` value this driver does not
/// recognise is a hard failure. Skipping it is exactly how a wrong fixture value
/// becomes a passing test.
fn error_code_case_expectation(id: &str, tc: &Value) -> Option<ErrorCode> {
    match (tc.get("expected_error"), tc.get("expected")) {
        (Some(declared), None) => {
            let wire = declared.as_str().unwrap_or_else(|| {
                panic!("FAIL [{id}]: expected_error must be a wire code string, got {declared}")
            });
            Some(wire_error_code(id, "error_codes.json", wire))
        }
        (None, Some(declared)) => {
            let sentinel = declared.as_str().unwrap_or_else(|| {
                panic!("FAIL [{id}]: expected must be a string sentinel, got {declared}")
            });
            assert!(
                sentinel == "ok",
                "FAIL [{id}]: error_codes.json states expected `{sentinel}` this driver does \
                 not recognise — teach the driver, do not skip it"
            );
            None
        }
        _ => panic!(
            "FAIL [{id}]: a case states exactly one of `expected_error` or `expected`; this \
             one states {:?} — teach the driver, do not skip it",
            (tc.get("expected_error"), tc.get("expected"))
        ),
    }
}

/// The observable post-condition owed by a case whose expectation is the `"ok"`
/// sentinel: the code just registered is queryable, both under its owning module
/// and in the registry-wide set.
///
/// Without this the positive cases prove nothing — `register` could drop its
/// argument and still satisfy them.
fn assert_code_registered(id: &str, registry: &ErrorCodeRegistry, module_id: &str, code: &str) {
    let owned = registry.codes_for_module(module_id).unwrap_or_else(|| {
        panic!("FAIL [{id}]: {module_id} owns no codes at all after a successful register")
    });
    assert!(
        owned.contains(code),
        "FAIL [{id}]: {module_id} owns {owned:?} after registering {code:?}"
    );
    assert!(
        registry.all_codes().contains(code),
        "FAIL [{id}]: {code:?} is not in all_codes() after a successful register"
    );
}

/// The mirror post-condition for a rejected registration: nothing was stored.
/// A registry that recorded the code and *then* reported the collision would
/// hand the next caller a corrupt ownership map.
fn assert_code_not_registered(id: &str, registry: &ErrorCodeRegistry, module_id: &str, code: &str) {
    assert!(
        registry
            .codes_for_module(module_id)
            .is_none_or(|owned| !owned.contains(code)),
        "FAIL [{id}]: {code:?} was stored under {module_id} even though registration failed"
    );
}

#[test]
#[allow(clippy::too_many_lines)] // one arm per fixture action, each with its own post-condition
fn conformance_error_codes() {
    let fixture = load_fixture("error_codes");
    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();
        let action = tc["action"].as_str().unwrap();
        let expectation = error_code_case_expectation(id, tc);
        let mut registry = ErrorCodeRegistry::new();

        match action {
            "register" => {
                let module_id = tc["module_id"].as_str().unwrap();
                let code = tc["error_code"].as_str().unwrap();
                let codes: HashSet<String> = [code.to_string()].into_iter().collect();
                let result = registry.register(module_id, &codes);

                if let Some(want) = expectation {
                    let err = result.err().unwrap_or_else(|| {
                        panic!(
                            "FAIL [{id}]: expected {} registering {code:?}, but it was accepted",
                            want.wire_str()
                        )
                    });
                    assert_eq!(
                        err.code,
                        want,
                        "FAIL [{id}]: expected wire code {}, got {} ({})",
                        want.wire_str(),
                        err.code.wire_str(),
                        err.message
                    );
                    assert_code_not_registered(id, &registry, module_id, code);
                } else {
                    result.unwrap_or_else(|e| {
                        panic!(
                            "FAIL [{id}]: expected ok, got {} ({})",
                            e.code.wire_str(),
                            e.message
                        )
                    });
                    assert_code_registered(id, &registry, module_id, code);
                }
            }
            "register_sequence" => {
                let steps = tc["steps"].as_array().unwrap();
                for (idx, step) in steps.iter().enumerate() {
                    let mid = step["module_id"].as_str().unwrap();
                    let code = step["error_code"].as_str().unwrap();
                    let codes: HashSet<String> = [code.to_string()].into_iter().collect();
                    let result = registry.register(mid, &codes);
                    let is_last = idx == steps.len() - 1;

                    // Only the final step may fail, and it must fail with the
                    // code the fixture declares; every earlier step must be
                    // accepted AND observable.
                    if let (true, Some(want)) = (is_last, expectation) {
                        let err = result.err().unwrap_or_else(|| {
                            panic!(
                                "FAIL [{id}] step {idx}: expected {} but the registration \
                                 was accepted",
                                want.wire_str()
                            )
                        });
                        assert_eq!(
                            err.code,
                            want,
                            "FAIL [{id}] step {idx}: expected wire code {}, got {} ({})",
                            want.wire_str(),
                            err.code.wire_str(),
                            err.message
                        );
                        assert_code_not_registered(id, &registry, mid, code);
                    } else {
                        result.unwrap_or_else(|e| {
                            panic!(
                                "FAIL [{id}] step {idx}: expected ok, got {} ({})",
                                e.code.wire_str(),
                                e.message
                            )
                        });
                        assert_code_registered(id, &registry, mid, code);
                    }
                }
            }
            "register_unregister_register" => {
                assert!(
                    expectation.is_none(),
                    "FAIL [{id}]: this action drives an all-succeeding sequence; a failing \
                     expectation on it needs a driver branch — teach the driver, do not skip it"
                );
                for step in tc["steps"].as_array().unwrap() {
                    let step_action = step["action"].as_str().unwrap();
                    let mid = step["module_id"].as_str().unwrap();
                    match step_action {
                        "register" => {
                            let code = step["error_code"].as_str().unwrap();
                            let codes: HashSet<String> = [code.to_string()].into_iter().collect();
                            registry.register(mid, &codes).unwrap_or_else(|e| {
                                panic!("FAIL [{id}]: {} ({})", e.code.wire_str(), e.message)
                            });
                            assert_code_registered(id, &registry, mid, code);
                        }
                        "unregister" => {
                            registry.unregister(mid);
                            // Post-condition: the release is observable, which
                            // is the whole point of `unregister_allows_reuse` —
                            // the later re-registration under another module
                            // could otherwise succeed for the wrong reason.
                            assert!(
                                registry.codes_for_module(mid).is_none(),
                                "FAIL [{id}]: {mid} still owns {:?} after unregister",
                                registry.codes_for_module(mid)
                            );
                        }
                        other => panic!(
                            "FAIL [{id}]: error_codes.json uses step action `{other}` this \
                             driver does not run — teach the driver, do not skip it"
                        ),
                    }
                }
            }
            other => panic!(
                "FAIL [{id}]: error_codes.json uses action `{other}` this driver does not \
                 run — teach the driver, do not skip it"
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// 7. ACL Evaluation
// ---------------------------------------------------------------------------

/// Build one `acl_evaluation` case's rules as this SDK's in-memory type.
fn acl_rules_of(tc: &Value) -> Vec<ACLRule> {
    let patterns = |r: &Value, key: &str| -> Vec<String> {
        r[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect()
    };
    tc["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let mut rule = ACLRule::new(
                patterns(r, "callers"),
                patterns(r, "targets"),
                r["effect"].as_str().unwrap().to_string(),
            );
            rule.description = r
                .get("description")
                .and_then(|v| v.as_str())
                .map(String::from);
            rule.conditions = r.get("conditions").cloned();
            rule
        })
        .collect()
}

#[test]
fn conformance_acl_evaluation() {
    ACL::init_builtin_handlers();
    let fixture = load_fixture("acl_evaluation");
    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();
        let caller_id_val = &tc["caller_id"];
        let target_id = tc["target_id"].as_str().unwrap();
        let expected = tc["expected"].as_bool().unwrap();
        let default_effect = tc["default_effect"].as_str().unwrap();

        let acl = ACL::new(acl_rules_of(tc), default_effect, None);

        let needs_context = tc.get("caller_identity").is_some()
            || tc
                .get("call_depth")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
                > 0
            || tc["rules"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r.get("conditions").is_some());

        let ctx: Option<Context<Value>> = if needs_context {
            let identity = if let Some(id_data) = tc.get("caller_identity") {
                Identity::new(
                    caller_id_val.as_str().unwrap_or("unknown").to_string(),
                    id_data
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("user")
                        .to_string(),
                    id_data
                        .get("roles")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .map(|v| v.as_str().unwrap().to_string())
                                .collect()
                        })
                        .unwrap_or_default(),
                    HashMap::new(),
                )
            } else {
                Identity::new(
                    "anonymous".to_string(),
                    "user".to_string(),
                    vec![],
                    HashMap::new(),
                )
            };

            let mut ctx: Context<Value> =
                Context::create(Some(identity), None, None, None, Value::Null, None);

            let call_depth = tc
                .get("call_depth")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            for i in 0..call_depth {
                ctx.call_chain.push(format!("_depth_{i}"));
            }

            Some(ctx)
        } else {
            None
        };

        let caller_id = if caller_id_val.is_null() {
            None
        } else {
            Some(caller_id_val.as_str().unwrap())
        };

        let result = acl.check(caller_id, target_id, ctx.as_ref());

        assert_eq!(
            result, expected,
            "FAIL [{id}]: ACL check(caller={caller_id:?}, target={target_id:?}) returned {result}, expected {expected}"
        );
    }
}

// ---------------------------------------------------------------------------
// 8. Context Serialization
// ---------------------------------------------------------------------------

fn build_context_from_input(input: &Value) -> Context<Value> {
    let identity: Option<Identity> = input.get("identity").and_then(|v| {
        if v.is_null() {
            None
        } else {
            Some(Identity::new(
                v["id"].as_str().unwrap().to_string(),
                v.get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("user")
                    .to_string(),
                v.get("roles")
                    .and_then(|r| r.as_array())
                    .map(|a| a.iter().map(|v| v.as_str().unwrap().to_string()).collect())
                    .unwrap_or_default(),
                v.get("attrs")
                    .and_then(|a| serde_json::from_value(a.clone()).ok())
                    .unwrap_or_default(),
            ))
        }
    });

    // Per Issue #66, `caller_id` is no longer a `Context::create` input;
    // top-level Contexts always carry `caller_id = None` and the fixture's
    // `caller_id` is assigned post-construction below alongside `trace_id`
    // and `call_chain`, all of which the fixture treats as Executor-managed
    // state replayed in tests.
    let mut ctx: Context<Value> = Context::create(identity, None, None, None, Value::Null, None);

    ctx.trace_id = input["trace_id"].as_str().unwrap().to_string();
    ctx.caller_id = input["caller_id"].as_str().map(String::from);
    ctx.call_chain = input["call_chain"]
        .as_array()
        .map(|a| a.iter().map(|v| v.as_str().unwrap().to_string()).collect())
        .unwrap_or_default();

    if let Some(ri) = input.get("redacted_inputs") {
        if !ri.is_null() {
            ctx.redacted_inputs = serde_json::from_value(ri.clone()).ok();
        }
    }

    if let Some(data_obj) = input.get("data").and_then(|d| d.as_object()) {
        let mut data = ctx.data.write();
        for (k, v) in data_obj {
            data.insert(k.clone(), v.clone());
        }
    }

    ctx
}

#[test]
fn conformance_context_serialization() {
    let fixture = load_fixture("context_serialization");
    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();
        if tc.get("sub_cases").is_some() {
            continue;
        }

        let input = &tc["input"];
        let expected = &tc["expected"];

        if id == "deserialization_round_trip" {
            let ctx: Context<Value> = Context::deserialize(input.clone()).unwrap();
            assert_eq!(
                ctx.trace_id,
                expected["trace_id"].as_str().unwrap(),
                "FAIL [{id}]"
            );
            assert_eq!(
                ctx.caller_id.as_deref(),
                expected["caller_id"].as_str(),
                "FAIL [{id}]"
            );
            if let Some(expected_id) = expected.get("identity_id").and_then(|v| v.as_str()) {
                let identity = ctx.identity.as_ref().unwrap();
                assert_eq!(identity.id(), expected_id, "FAIL [{id}]");
                assert_eq!(
                    identity.identity_type(),
                    expected["identity_type"].as_str().unwrap(),
                    "FAIL [{id}]"
                );
            }
            continue;
        }

        if id == "unknown_context_version_warns_but_proceeds" {
            let ctx: Context<Value> = Context::deserialize(input.clone()).unwrap();
            assert_eq!(
                ctx.trace_id,
                expected["trace_id"].as_str().unwrap(),
                "FAIL [{id}]"
            );
            continue;
        }

        // Standard: build context, serialize, compare
        let ctx = build_context_from_input(input);
        let serialized = ctx.serialize();

        if id == "redacted_inputs_serialized" {
            assert_eq!(
                serialized["trace_id"].as_str().unwrap(),
                expected["trace_id"].as_str().unwrap(),
                "FAIL [{id}]"
            );
            assert_eq!(
                serialized["redacted_inputs"], expected["redacted_inputs"],
                "FAIL [{id}]"
            );
            continue;
        }

        assert_eq!(
            serialized["_context_version"], expected["_context_version"],
            "FAIL [{id}] _context_version"
        );
        assert_eq!(
            serialized["trace_id"], expected["trace_id"],
            "FAIL [{id}] trace_id"
        );
        assert_eq!(
            serialized["caller_id"], expected["caller_id"],
            "FAIL [{id}] caller_id"
        );
        assert_eq!(
            serialized["call_chain"], expected["call_chain"],
            "FAIL [{id}] call_chain"
        );
        assert_eq!(
            serialized["identity"], expected["identity"],
            "FAIL [{id}] identity"
        );
        assert_eq!(serialized["data"], expected["data"], "FAIL [{id}] data");
    }
}

#[test]
fn conformance_context_identity_types() {
    let fixture = load_fixture("context_serialization");
    for tc in fixture["test_cases"].as_array().unwrap() {
        if let Some(sub_cases) = tc.get("sub_cases").and_then(|v| v.as_array()) {
            for sub in sub_cases {
                let id_data = &sub["input_identity"];
                let expected_type = sub["expected_type"].as_str().unwrap();

                let identity = Identity::new(
                    id_data["id"].as_str().unwrap().to_string(),
                    id_data["type"].as_str().unwrap().to_string(),
                    id_data["roles"]
                        .as_array()
                        .map(|a| a.iter().map(|v| v.as_str().unwrap().to_string()).collect())
                        .unwrap_or_default(),
                    HashMap::new(),
                );

                let ctx: Context<Value> =
                    Context::create(Some(identity), None, None, None, Value::Null, None);
                let serialized = ctx.serialize();

                assert_eq!(
                    serialized["identity"]["type"].as_str().unwrap(),
                    expected_type,
                    "FAIL identity type {expected_type}"
                );

                let restored: Context<Value> = Context::deserialize(serialized).unwrap();
                assert_eq!(
                    restored.identity.as_ref().unwrap().identity_type(),
                    expected_type,
                    "FAIL roundtrip identity type {expected_type}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 9. Schema Validation (S4.15)
// ---------------------------------------------------------------------------

#[test]
fn conformance_schema_validation() {
    let fixture = load_fixture("schema_validation");
    // `SchemaValidator::new()` no longer coerces (TYPE_MAPPING §17.3 — the
    // module-invocation boundary never does, and the two paths must agree), so
    // this driver names the mode it wants instead of leaning on a default. A
    // case carrying `expected_valid_strict` / `expected_valid_coerce` documents
    // BOTH modes and is asserted against both.
    let validator = SchemaValidator::with_coerce_types(false);
    let coercing_validator = SchemaValidator::with_coerce_types(true);

    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();
        let schema = &tc["schema"];
        let input = &tc["input"];

        // Determine expected validity for the no-coercion validator.
        //
        // CORRECTED (apcore#93): the trailing `else { true }` was a catch-all
        // that turned "the case states no validity expectation this driver
        // reads" into a silent assertion that it is valid. A case is now
        // required to state one.
        let expected_valid = if let Some(v) = tc.get("expected_valid") {
            v.as_bool().unwrap()
        } else if let Some(v) = tc.get("expected_valid_strict") {
            v.as_bool().unwrap()
        } else {
            panic!(
                "FAIL [{id}]: states neither `expected_valid` nor `expected_valid_strict` — \
                 teach the driver, do not default it to valid"
            )
        };

        // CORRECTED (apcore#93): `empty_schema_accepts_string` used to `continue`
        // here as a "known gap: empty schema + string input". It is not a gap —
        // `SchemaValidator` delegates to the `jsonschema` crate, for which `{}`
        // is the Draft 2020-12 always-true schema and accepts any instance,
        // scalar roots included. The skip made the one case in this fixture that
        // pins non-object root handling unpinnable in apcore-rust.
        let result = validator.validate(input, schema);
        assert_eq!(
            result.valid, expected_valid,
            "FAIL [{}]: valid={}, expected={}, errors={:?}",
            id, result.valid, expected_valid, result.errors
        );

        // The MODULE-INVOCATION boundary must reach the same verdict as the
        // library validator's default. TYPE_MAPPING §17.3 names apcore-rust for
        // having once shipped a validator whose default coerced while the
        // executor path did not, so the two answered differently for the same
        // schema and input. Asserted here on every case rather than trusted.
        let boundary = apcore::executor::validate_against_schema(input, schema, "Input");
        assert_eq!(
            boundary.is_ok(),
            expected_valid,
            "FAIL [{id}]: the executor's module-invocation boundary disagrees with \
             SchemaValidator::with_coerce_types(false) — boundary said {:?}, the fixture \
             declares valid={expected_valid}",
            boundary.err().map(|e| e.message)
        );

        // The opt-in coercing mode is a separate library-level contract, whose
        // BEHAVIOUR (not existence) TYPE_MAPPING §11 makes normative as of spec
        // v1.12.0: from a string only, `"42"`/`"-7"` → integer, `"1.5"` →
        // number, and exactly `"true"`/`"false"` case-sensitive → boolean.
        //
        // The fixture's `driver_contract.both_halves_or_neither` requires BOTH
        // halves of a coercion case to be asserted — the failure it exists for
        // is the strict path and the knob answering the same way, so a case
        // stating `expected_valid_strict` without `expected_valid_coerce` is a
        // half-assertion and is refused rather than silently skipped.
        match tc.get("expected_valid_coerce").and_then(Value::as_bool) {
            Some(expected_coerce) => {
                let coerced_result = coercing_validator.validate(input, schema);
                assert_eq!(
                    coerced_result.valid, expected_coerce,
                    "FAIL [{}] (coerce_types=true): valid={}, expected={}, errors={:?}",
                    id, coerced_result.valid, expected_coerce, coerced_result.errors
                );
            }
            None => assert!(
                tc.get("expected_valid_strict").is_none(),
                "FAIL [{id}]: states `expected_valid_strict` without \
                 `expected_valid_coerce` — the fixture's `both_halves_or_neither` \
                 contract requires both, and asserting only the strict half hides \
                 the knob disagreeing with it"
            ),
        }

        // When the fixture pins a coerced value, validate_input must return it.
        if let Some(expected_coerced) = tc.get("expected_coerced_value") {
            let coerced = coercing_validator
                .validate_input(input, schema)
                .unwrap_or_else(|e| panic!("FAIL [{id}]: coercion errored: {e:?}"));
            if let Some(obj) = coerced.as_object() {
                let found = obj.values().any(|v| v == expected_coerced);
                assert!(
                    found,
                    "FAIL [{id}]: expected coerced value {expected_coerced:?} in {coerced:?}"
                );
            }
        }

        // Verify error path when expected
        if !expected_valid {
            if let Some(expected_path) = tc.get("expected_error_path").and_then(|v| v.as_str()) {
                let has_matching = result.errors.iter().any(|e| e.path == expected_path);
                assert!(
                    has_matching,
                    "FAIL [{}]: expected error at {:?}, got {:?}",
                    id, expected_path, result.errors
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 10. Config Env Mapping (A12-NS, §9.8)
//
// CORRECTED (apcore#93): the four `env_style` cases — `env_style_flat`,
// `env_style_nested`, `env_style_nested_literal_underscore`,
// `env_style_auto_detect_flat` — were skipped wholesale with `continue`,
// leaving §9.8's entire style-resolution contract unasserted in apcore-rust.
// The stated reason was true but not a dead end: the namespace registry IS
// process-global and a name or prefix can be claimed only once, so the `mcp`
// namespace the fixture's `namespaces` block registers cannot be re-registered
// per style. It can, however, be SHADOWED — see `shadow_namespace_for_style`.
// ---------------------------------------------------------------------------

/// The `env_style` a case declares, as the SDK enum.
fn env_style_from_fixture(id: &str, style: &str) -> EnvStyle {
    match style {
        "flat" => EnvStyle::Flat,
        "nested" => EnvStyle::Nested,
        "auto" => EnvStyle::Auto,
        other => panic!(
            "FAIL [{id}]: config_env declares env_style {other:?}, which this driver does not \
             know — teach the driver, do not skip it"
        ),
    }
}

/// The fixture namespace whose `env_prefix` owns `env_var` (longest match),
/// as `(name, env_prefix, max_depth)`.
fn owning_namespace(id: &str, fixture: &Value, env_var: &str) -> (String, String, usize) {
    let owner = fixture["namespaces"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|ns| ns["name"].as_str() != Some("global"))
        .filter(|ns| {
            ns.get("env_prefix")
                .and_then(|v| v.as_str())
                .is_some_and(|p| env_var.starts_with(p))
        })
        .max_by_key(|ns| ns["env_prefix"].as_str().unwrap_or_default().len())
        .unwrap_or_else(|| {
            panic!("FAIL [{id}]: no fixture namespace declares a prefix owning {env_var:?}")
        });
    #[allow(clippy::cast_possible_truncation)] // max_depth from fixtures is a small integer
    let max_depth = owner
        .get("max_depth")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(5) as usize;
    (
        owner["name"].as_str().unwrap().to_string(),
        owner["env_prefix"].as_str().unwrap().to_string(),
        max_depth,
    )
}

/// Register a per-style shadow of the namespace that owns the case's env var,
/// and return `(shadow_name, shadow_prefix, owner_name, owner_prefix)`.
///
/// The shadow carries the SAME `max_depth` and is driven with the SAME env-var
/// SUFFIX (`SERVER_PORT`, `SERVER__URL`) as the case declares — only the
/// namespace segment differs, so §9.8's suffix algorithm is what decides the
/// outcome. Longest-prefix dispatch (asserted by `longest_prefix_rule` in this
/// same fixture) routes `APCORE_MCPNESTED_*` to the shadow rather than to
/// `mcp`, so the two coexist in the process-global registry.
fn shadow_namespace_for_style(
    id: &str,
    fixture: &Value,
    env_var: &str,
    style: &str,
) -> (String, String, String, String) {
    let (owner_name, owner_prefix, max_depth) = owning_namespace(id, fixture, env_var);
    let shadow_name = format!("{owner_name}{style}");
    let shadow_prefix = format!("{owner_prefix}{}", style.to_uppercase());
    // Duplicate registration is expected when several cases share a style;
    // the registry rejects it and the first registration stands.
    let _ = Config::register_namespace(NamespaceRegistration {
        name: shadow_name.clone(),
        env_prefix: Some(shadow_prefix.clone()),
        defaults: None,
        schema: None,
        env_style: env_style_from_fixture(id, style),
        max_depth,
        env_map: None,
    });
    (shadow_name, shadow_prefix, owner_name, owner_prefix)
}

#[test]
fn conformance_config_env() {
    let fixture = load_fixture("config_env");

    // Register namespaces from fixture metadata.
    // Config::register_namespace uses a global registry, so we must register
    // all namespaces before testing env resolution.
    for ns in fixture["namespaces"].as_array().unwrap() {
        let name = ns["name"].as_str().unwrap();
        // Skip "apcore" — implicitly registered by framework.
        if name == "apcore" {
            continue;
        }

        let env_prefix = ns
            .get("env_prefix")
            .and_then(|v| v.as_str())
            .map(String::from);
        #[allow(clippy::cast_possible_truncation)] // max_depth from fixtures is a small integer
        let max_depth = ns
            .get("max_depth")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(5) as usize;

        let env_map_obj = ns.get("env_map").and_then(|v| v.as_object()).map(|obj| {
            obj.iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
                .collect::<HashMap<String, String>>()
        });

        // The "global" namespace is special: its env_map entries are top-level
        // (un-namespaced) mappings, and its APCORE prefix should NOT capture
        // vars into a "global." sub-namespace. Register only its env_map as
        // global, skip namespace registration.
        if name == "global" {
            if let Some(ref mapping) = env_map_obj {
                let _ = Config::env_map(mapping.clone());
            }
            continue;
        }

        // Attempt registration; ignore duplicates from prior test runs in the
        // same process (global registry is process-wide).
        let _ = Config::register_namespace(NamespaceRegistration {
            name: name.to_string(),
            env_prefix,
            defaults: None,
            schema: None,
            env_style: EnvStyle::Auto,
            max_depth,
            env_map: env_map_obj,
        });
    }

    // Test each case by setting env var, creating a fresh Config, and
    // checking the resolved path.
    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();
        let mut env_var = tc["env_var"].as_str().unwrap().to_string();
        let env_value = tc["env_value"].as_str().unwrap();

        // A case declaring an explicit `env_style` is re-anchored onto a
        // per-style shadow namespace (see `shadow_namespace_for_style`). Only
        // the namespace segment is substituted: the env-var SUFFIX and every
        // path segment after the first are the fixture's own text, so the
        // declared expectation still decides the outcome.
        let mut expected_path = tc
            .get("expected_path")
            .and_then(|v| v.as_str())
            .map(String::from);
        if let Some(style) = tc.get("env_style").and_then(|v| v.as_str()) {
            let (shadow_name, shadow_prefix, owner_name, owner_prefix) =
                shadow_namespace_for_style(id, &fixture, &env_var, style);
            env_var = format!("{shadow_prefix}{}", &env_var[owner_prefix.len()..]);
            expected_path = expected_path.map(|p| {
                p.strip_prefix(&format!("{owner_name}.")).map_or_else(
                    || panic!("FAIL [{id}]: expected_path {p:?} is not under {owner_name:?}"),
                    |rest| format!("{shadow_name}.{rest}"),
                )
            });
        }
        let env_var = env_var.as_str();
        let expected_path = expected_path.as_deref();
        let expected_value = tc.get("expected_value").and_then(|v| v.as_str());

        // Set the env var, build a namespace-mode config, then clean up.
        // Config must have an "apcore" top-level key to activate namespace mode.
        // We write a minimal temp YAML to trigger detection.
        std::env::set_var(env_var, env_value);
        let config = {
            let dir = std::env::temp_dir().join("apcore_conformance_config_env");
            std::fs::create_dir_all(&dir).unwrap();
            let yaml_path = dir.join("apcore.yaml");
            std::fs::write(
                &yaml_path,
                "executor:\n  max_call_depth: 32\n  max_module_repeat: 3\napcore:\n  version: \"0.16.0\"\n",
            )
            .unwrap();
            Config::load(yaml_path.as_path()).unwrap()
        };
        std::env::remove_var(env_var);

        if let (Some(path), Some(value)) = (expected_path, expected_value) {
            let actual = config.get(path);
            assert!(
                actual.is_some(),
                "FAIL [{id}]: expected path {path:?} to have a value, got None. env_var={env_var}, env_value={env_value}"
            );
            let actual_str = match actual.unwrap() {
                Value::String(s) => s,
                Value::Bool(b) => b.to_string(),
                Value::Number(n) => n.to_string(),
                other => other.to_string(),
            };
            assert_eq!(
                actual_str, value,
                "FAIL [{id}]: path {path:?} expected {value:?}, got {actual_str:?}"
            );
        } else {
            // expected_path is null — env var should be ignored.
            // We can't easily verify absence without knowing the key, so just
            // assert the config loaded without panic.
        }
    }
}

// ---------------------------------------------------------------------------
// Context.create trace_parent handling (PROTOCOL_SPEC §10.5)
// ---------------------------------------------------------------------------

// MakeWriter that buffers tracing output into a shared Vec<u8> so tests can
// assert on emitted log lines. Mirrors the Python conformance test's use of
// pytest's caplog fixture.
#[derive(Clone, Default)]
struct CapturedLogs(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl CapturedLogs {
    fn as_string(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl std::io::Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CapturedLogs {
    type Writer = CapturedLogs;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[test]
fn conformance_context_trace_parent() {
    use apcore::trace_context::TraceParent;

    let fixture = load_fixture("context_trace_parent");
    let hex_re = regex::Regex::new(r"^[0-9a-f]{32}$").unwrap();

    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();
        let incoming = tc["input"]["trace_parent_trace_id"].as_str();
        let expected = &tc["expected"];
        let expected_regen = expected["regenerated"].as_bool().unwrap();
        let expected_warn = expected["warn_logged"].as_bool().unwrap();

        // Construct a TraceParent directly from the raw trace_id string,
        // bypassing TraceParent::parse so we can exercise the builder's
        // defensive validation with every fixture input — including those
        // that a well-behaved parser would never emit.
        let trace_parent = incoming.map(|trace_id| TraceParent {
            version: 0,
            trace_id: trace_id.to_string(),
            parent_id: "0000000000000001".to_string(),
            trace_flags: 1,
            tracestate: vec![],
        });

        let captured = CapturedLogs::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(captured.clone())
            .with_max_level(tracing::Level::WARN)
            .with_ansi(false)
            .with_target(false)
            .finish();

        let ctx: Context<serde_json::Value> = tracing::subscriber::with_default(subscriber, || {
            Context::builder().trace_parent(trace_parent).build()
        });

        assert!(
            hex_re.is_match(&ctx.trace_id),
            "FAIL [{id}]: trace_id {:?} is not 32-char lowercase hex",
            ctx.trace_id
        );
        assert_ne!(
            ctx.trace_id,
            "0".repeat(32),
            "FAIL [{id}]: trace_id is W3C-invalid all-zero"
        );
        assert_ne!(
            ctx.trace_id,
            "f".repeat(32),
            "FAIL [{id}]: trace_id is W3C-invalid all-f"
        );

        if expected_regen {
            if let Some(src) = incoming {
                assert_ne!(
                    ctx.trace_id, src,
                    "FAIL [{id}]: expected regeneration but inherited {src:?}"
                );
            }
        } else {
            let want = expected["trace_id"].as_str().unwrap();
            assert_eq!(
                ctx.trace_id, want,
                "FAIL [{id}]: expected inheritance of {want:?}, got {:?}",
                ctx.trace_id
            );
        }

        let log_output = captured.as_string();
        let warn_seen = log_output.contains("Invalid trace_id format");
        assert_eq!(
            warn_seen, expected_warn,
            "FAIL [{id}]: expected warn_logged={expected_warn}, got warn_seen={warn_seen} output={log_output:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Helper: load_schema — resolves from the canonical schemas/ directory.
// ---------------------------------------------------------------------------

fn load_schema(name: &str) -> Value {
    // §8.2.1 rule 4: resolve `schemas/` on its own rather than climbing out of
    // the fixtures root — under CONFORMANCE_FIXTURES there is no repo above it.
    let path = find_schemas_root().join(format!("{name}.schema.json"));
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Failed to read schema: {}", path.display()));
    serde_json::from_str(&content).unwrap_or_else(|e| panic!("Invalid JSON in schema {name}: {e}"))
}

// ---------------------------------------------------------------------------
// 11. Config Defaults (A12-DEF)
//
// CORRECTED (apcore#93): this driver carried a six-entry `supported_keys`
// allowlist and `continue`d on everything else, so 12 of the fixture's 18 cases
// — the whole `extensions.*` / `schema.*` / `acl.*` / `sys_modules.*` /
// `stream.*` half of the canonical default table, plus
// `observability.tracing.sampling_rate` — never reached an assertion. Mutating
// any of their declared values left apcore-rust green, which is precisely the
// cross-language divergence this fixture exists to catch.
//
// The allowlist's stated reason ("not part of the Rust SDK's typed Config
// struct ... and have no default") went stale when `CONFIG_DEFAULTS` landed:
// `Config::get` now falls back to the canonical default table for exactly
// these keys. Every case is asserted; a key that resolves to `None` is a hard
// failure, not a skip.
// ---------------------------------------------------------------------------

#[test]
fn conformance_config_defaults() {
    let fixture = load_fixture("config_defaults");
    // `Config::default()`, NOT `Config::from_defaults()`: the latter applies
    // env overrides, and `conformance_config_env` sets and clears
    // `APCORE_OBSERVABILITY_TRACING_ENABLED` (among others) in the same process
    // while this test runs. Reading process env here would make the canonical
    // default table's answer depend on another test's timing.
    let config = Config::default();

    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();
        let key = tc["key"].as_str().unwrap();
        let expected = &tc["expected"];

        let actual = config.get(key).unwrap_or_else(|| {
            panic!(
                "FAIL [{id}]: Config::default().get({key:?}) returned None, but \
                 config_defaults.json declares the canonical default {expected}. A key that \
                 resolves to null here resolves to the documented value in apcore-python and \
                 apcore-typescript."
            )
        });

        // Compare numerically where the expected value is a JSON number.
        match (expected, &actual) {
            (Value::Number(exp_n), Value::Number(act_n)) => {
                assert_eq!(
                    exp_n.as_f64(),
                    act_n.as_f64(),
                    "FAIL [{id}]: key={key:?} expected={expected} got={actual}"
                );
            }
            _ => {
                assert_eq!(
                    &actual, expected,
                    "FAIL [{id}]: key={key:?} expected={expected} got={actual}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 12. Stream Aggregation — deep merge (A11-STREAM)
// ---------------------------------------------------------------------------

/// Recursive deep-merge of two JSON objects (matches executor internal logic).
fn deep_merge_objects(
    base: &mut serde_json::Map<String, Value>,
    overlay: &serde_json::Map<String, Value>,
) {
    for (k, v) in overlay {
        let entry = base.entry(k.clone()).or_insert(Value::Null);
        match (entry, v) {
            (Value::Object(base_map), Value::Object(overlay_map)) => {
                deep_merge_objects(base_map, overlay_map);
            }
            (base_entry, overlay_val) => {
                *base_entry = overlay_val.clone();
            }
        }
    }
}

#[test]
fn conformance_stream_aggregation() {
    let fixture = load_fixture("stream_aggregation");
    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();
        let chunks = tc["chunks"].as_array().unwrap();

        if chunks.is_empty() {
            assert!(
                tc["expected"].is_null(),
                "FAIL [{id}]: expected null for empty chunks"
            );
            continue;
        }

        let mut acc = serde_json::Map::new();
        for chunk in chunks {
            match chunk {
                Value::Object(obj) => {
                    deep_merge_objects(&mut acc, obj);
                }
                other => {
                    // Non-object chunk replaces entire accumulator (last-value-wins).
                    // This path is not exercised by current fixtures (all chunks are objects).
                    let _ = acc;
                    acc = serde_json::Map::new();
                    if let Some(obj) = other.as_object() {
                        acc = obj.clone();
                    }
                }
            }
        }

        assert_eq!(Value::Object(acc), tc["expected"], "FAIL [{id}]");
    }
}

// ---------------------------------------------------------------------------
// 13. Identity System (AC-014, AC-015)
//
// CORRECTED (apcore#92): `identity_propagates_to_child_context` used to state
// its expectation as the prose string "child.identity === parent.identity" — a
// sentence in a value slot, which no driver can assert — so every driver
// hardcoded the comparison behind `if id == "..."` and the fixture value was
// decoration. The expectation is now four named fields, and each one is read
// from the fixture and asserted below.
//
// Expectations are matched BY NAME and every `expected*` key a case states must
// reach an assertion: an unread key is a hard failure, not a silent skip.
// ---------------------------------------------------------------------------

#[test]
#[allow(clippy::too_many_lines)] // one arm per expectation key, each asserted
fn conformance_identity_system() {
    let fixture = load_fixture("identity_system");
    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();
        let input_id = tc["input_id"].as_str().unwrap().to_string();
        let input_type = tc
            .get("input_type")
            .and_then(|v| v.as_str())
            .unwrap_or("user")
            .to_string();
        let input_roles: Vec<String> = tc["input_roles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        let input_attrs: HashMap<String, Value> = tc
            .get("input_attrs")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let identity = Identity::new(
            input_id.clone(),
            input_type,
            input_roles.clone(),
            input_attrs,
        );

        let expectation_keys: Vec<&String> = tc
            .as_object()
            .unwrap()
            .keys()
            .filter(|k| k.starts_with("expected"))
            .collect();
        assert!(
            !expectation_keys.is_empty(),
            "FAIL [{id}]: the case states no expectation, so it asserts nothing"
        );

        for key in expectation_keys {
            match key.as_str() {
                "expected_type" => {
                    let expected_type = tc[key].as_str().unwrap();
                    assert_eq!(identity.identity_type(), expected_type, "FAIL [{id}] type");
                }
                "expected_roles" => {
                    let exp: Vec<String> = tc[key]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_str().unwrap().to_string())
                        .collect();
                    assert_eq!(identity.roles(), &exp, "FAIL [{id}] roles");
                }
                "expected_attrs" => {
                    let expected_attrs = tc[key].as_object().unwrap();
                    for (k, exp_v) in expected_attrs {
                        let actual_v = identity
                            .attrs()
                            .get(k)
                            .unwrap_or_else(|| panic!("FAIL [{id}] attrs: missing key {k:?}"));
                        assert_eq!(actual_v, exp_v, "FAIL [{id}] attrs[{k}]");
                    }
                }
                // Child-context propagation, field for field.
                "expected" => {
                    let child_module_id = tc
                        .get("child_module_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or_else(|| {
                            panic!("FAIL [{id}]: an `expected` block needs `child_module_id`")
                        });
                    let parent: Context<Value> = Context::create(
                        Some(identity.clone()),
                        None,
                        None,
                        None,
                        Value::Null,
                        None,
                    );
                    let child = parent.child(child_module_id);
                    let child_identity = child.identity.as_ref().unwrap_or_else(|| {
                        panic!("FAIL [{id}]: the child context carries no identity at all")
                    });

                    for (field, expected) in tc[key].as_object().unwrap() {
                        match field.as_str() {
                            "child_identity_id" => assert_eq!(
                                child_identity.id(),
                                expected.as_str().unwrap(),
                                "FAIL [{id}] child_identity_id"
                            ),
                            "child_identity_type" => assert_eq!(
                                child_identity.identity_type(),
                                expected.as_str().unwrap(),
                                "FAIL [{id}] child_identity_type"
                            ),
                            "child_identity_roles" => assert_eq!(
                                serde_json::to_value(child_identity.roles()).unwrap(),
                                *expected,
                                "FAIL [{id}] child_identity_roles"
                            ),
                            // Whole-value equality, which is stronger than the
                            // three field checks above and is what the retired
                            // prose expectation meant.
                            "child_identity_equals_parent" => assert_eq!(
                                child.identity == parent.identity,
                                expected.as_bool().unwrap(),
                                "FAIL [{id}] child_identity_equals_parent: child {:?} vs \
                                 parent {:?}",
                                child.identity,
                                parent.identity
                            ),
                            other => panic!(
                                "FAIL [{id}]: identity_system.json states expected.{other} \
                                 which this driver does not read — teach the driver, do not \
                                 skip it"
                            ),
                        }
                    }
                }
                other => panic!(
                    "FAIL [{id}]: identity_system.json states `{other}` which this driver \
                     does not read — teach the driver, do not skip it"
                ),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 14. ModuleAnnotations Extra Round-Trip (spec §4.4)
// ---------------------------------------------------------------------------

/// D-115: a malformed annotation value is dropped, the rest survives.
///
/// Returns `true` when the case declared `expected_survivors` and was handled
/// here. Extracted from the round-trip driver so that driver stays under the
/// line limit — and because the two assertions answer different questions: the
/// round-trip cases ask whether a WELL-FORMED value survives the wire, this asks
/// whether a MALFORMED one is survivable at all.
///
/// `expected_deserialized_extra` alone cannot express it: an SDK that rejects
/// the whole value never reaches the assertion, and one that drops everything
/// satisfies it.
fn assert_malformed_value_is_tolerated(id: &str, tc: &Value) -> bool {
    let Some(survivors) = tc.get("expected_survivors").and_then(Value::as_object) else {
        return false;
    };
    let raw = tc
        .get("input_serialized")
        .expect("expected_survivors needs input_serialized");
    let ann: apcore::module::ModuleAnnotations = serde_json::from_value(raw.clone())
        .unwrap_or_else(|e| {
            panic!(
                "FAIL [{id}]: a malformed annotation value must be dropped, not reject \
             the whole value: {e}"
            )
        });
    let got = serde_json::to_value(&ann).expect("reserialize");
    for (field, want) in survivors {
        assert_eq!(
            got.get(field.as_str()),
            Some(want),
            "FAIL [{id}]: {field} must survive a malformed sibling"
        );
    }
    if let Some(expected_extra) = tc
        .get("expected_deserialized_extra")
        .and_then(Value::as_object)
    {
        assert_eq!(
            ann.extra.len(),
            expected_extra.len(),
            "FAIL [{id}]: extra should be {expected_extra:?}, got {:?}",
            ann.extra
        );
    }
    true
}

#[test]
fn conformance_annotations_extra_round_trip() {
    use apcore::module::ModuleAnnotations;

    let fixture = load_fixture("annotations_extra_round_trip");
    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();

        // Cases that use "input" (canonical nested form or producer test).
        if assert_malformed_value_is_tolerated(id, tc) {
            continue;
        }

        if let Some(input) = tc.get("input") {
            let annotations: ModuleAnnotations = serde_json::from_value(input.clone())
                .unwrap_or_else(|e| panic!("FAIL [{id}] deserialize: {e}"));

            // Verify deserialized extra keys.
            if let Some(expected_extra) = tc
                .get("expected_deserialized_extra")
                .and_then(|v| v.as_object())
            {
                for (k, exp_v) in expected_extra {
                    let actual_v = annotations.extra.get(k).unwrap_or_else(|| {
                        panic!(
                            "FAIL [{id}] extra: missing key {k:?}; got {:?}",
                            annotations.extra
                        )
                    });
                    assert_eq!(actual_v, exp_v, "FAIL [{id}] extra[{k}]");
                }
                assert_eq!(
                    annotations.extra.len(),
                    expected_extra.len(),
                    "FAIL [{id}] extra length mismatch"
                );
            }

            // Re-serialize and compare with expected_serialized.
            //
            // Pilot-tolerant comparator for the v0.21.0 `discoverable` rollout:
            // per the ephemeral-namespace rollout (protocol-spec §2.5.1), the
            // canonical
            // `annotations_extra_round_trip.json` fixture MUST NOT be updated
            // to require `discoverable` until ALL three SDKs have shipped
            // support. SDKs that have shipped the field strip it from actual
            // serialized output before comparison so the suite stays green.
            // Mirrors apcore-typescript's `stripDiscoverableForPilot`.
            if let Some(expected_ser) = tc.get("expected_serialized") {
                let mut serialized = serde_json::to_value(&annotations)
                    .unwrap_or_else(|e| panic!("FAIL [{id}] serialize: {e}"));
                if let (Some(actual_obj), Some(expected_obj)) =
                    (serialized.as_object_mut(), expected_ser.as_object())
                {
                    if actual_obj.contains_key("discoverable")
                        && !expected_obj.contains_key("discoverable")
                    {
                        actual_obj.remove("discoverable");
                    }
                }
                assert_eq!(&serialized, expected_ser, "FAIL [{id}] serialized mismatch");
            }

            // Producer MUST NOT emit forbidden root keys.
            if let Some(forbidden) = tc.get("forbidden_root_keys").and_then(|v| v.as_array()) {
                let serialized = serde_json::to_value(&annotations)
                    .unwrap_or_else(|e| panic!("FAIL [{id}] serialize: {e}"));
                let obj = serialized.as_object().unwrap();
                for fk in forbidden {
                    let fk_str = fk.as_str().unwrap();
                    assert!(
                        !obj.contains_key(fk_str),
                        "FAIL [{id}]: serialized output contains forbidden root key {fk_str:?}"
                    );
                }
            }
        }

        // Cases that use "input_serialized" (legacy flattened form or precedence test).
        if let Some(input_ser) = tc.get("input_serialized") {
            let annotations: ModuleAnnotations = serde_json::from_value(input_ser.clone())
                .unwrap_or_else(|e| panic!("FAIL [{id}] deserialize legacy: {e}"));

            if let Some(expected_extra) = tc
                .get("expected_deserialized_extra")
                .and_then(|v| v.as_object())
            {
                for (k, exp_v) in expected_extra {
                    let actual_v = annotations.extra.get(k).unwrap_or_else(|| {
                        panic!(
                            "FAIL [{id}] extra: missing key {k:?}; got {:?}",
                            annotations.extra
                        )
                    });
                    assert_eq!(actual_v, exp_v, "FAIL [{id}] extra[{k}]");
                }
                assert_eq!(
                    annotations.extra.len(),
                    expected_extra.len(),
                    "FAIL [{id}] extra length mismatch"
                );
            }

            // Re-serialize legacy-deserialized form must emit canonical nested form.
            // Same pilot-tolerant comparator as above for `discoverable`.
            if let Some(expected_reser) = tc.get("expected_reserialized") {
                let mut serialized = serde_json::to_value(&annotations)
                    .unwrap_or_else(|e| panic!("FAIL [{id}] reserialize: {e}"));
                if let (Some(actual_obj), Some(expected_obj)) =
                    (serialized.as_object_mut(), expected_reser.as_object())
                {
                    if actual_obj.contains_key("discoverable")
                        && !expected_obj.contains_key("discoverable")
                    {
                        actual_obj.remove("discoverable");
                    }
                }
                assert_eq!(
                    &serialized, expected_reser,
                    "FAIL [{id}] reserialized mismatch"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 15. Approval Gate (A05)
//
// CORRECTED: this driver used to compute the gate decision itself
// (`gate_would_fire = handler_configured && module_requires_approval`) and then
// assert things like `expected["error_code"] == "APPROVAL_DENIED"` — fixture
// text against a literal in the test, with the Executor never invoked. Both the
// gate rule and the status→error mapping were untested. It now runs the real
// Step 5 through `Executor::call`.
// ---------------------------------------------------------------------------

#[tokio::test]
#[allow(clippy::too_many_lines)] // one arm per fixture case; splitting hides the mapping
async fn conformance_approval_gate() {
    use apcore::approval::{ApprovalHandler, ApprovalRequest, ApprovalResult};
    use apcore::executor::Executor;
    use apcore::module::{Module, ModuleAnnotations};
    use apcore::registry::{ModuleDescriptor, Registry};
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Returns the fixture's declared `approval_result` and counts invocations,
    /// which is what makes `gate_invoked` an observation rather than a guess.
    #[derive(Debug)]
    struct ScriptedHandler {
        result: ApprovalResult,
        invocations: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl ApprovalHandler for ScriptedHandler {
        async fn request_approval(
            &self,
            _request: &ApprovalRequest,
        ) -> Result<ApprovalResult, apcore::errors::ModuleError> {
            self.invocations.fetch_add(1, Ordering::SeqCst);
            Ok(self.result.clone())
        }
        async fn check_approval(
            &self,
            _approval_id: &str,
        ) -> Result<ApprovalResult, apcore::errors::ModuleError> {
            self.invocations.fetch_add(1, Ordering::SeqCst);
            Ok(self.result.clone())
        }
    }

    /// The fixture's `module_requires_approval` is declared on the LIVE module
    /// instance, because that is the binding PROTOCOL_SPEC §7.4 Step 5 gates on
    /// (`annotations = module.annotations`). The descriptor below carries the
    /// same value, so this driver deliberately does NOT discriminate between
    /// the two sources — `tests/test_approval_gate_decides_from_live_module.rs`
    /// is the case that makes them disagree.
    struct SensitiveModule {
        requires_approval: bool,
    }
    #[async_trait]
    impl Module for SensitiveModule {
        fn input_schema(&self) -> Value {
            json!({"type": "object"})
        }
        fn output_schema(&self) -> Value {
            json!({"type": "object"})
        }
        fn description(&self) -> &'static str {
            "Module gated by the approval step"
        }
        fn annotations(&self) -> ModuleAnnotations {
            ModuleAnnotations {
                requires_approval: self.requires_approval,
                ..Default::default()
            }
        }
        async fn execute(
            &self,
            inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Value, apcore::errors::ModuleError> {
            Ok(inputs)
        }
    }

    /// Resolve a case's governance declarations for the two sources SEPARATELY.
    ///
    /// Returns `(module_declares, descriptor_declares)`.
    ///
    /// The original cases carry a single `module_requires_approval` boolean,
    /// which each SDK was free to satisfy from whichever source it preferred --
    /// and that is exactly how an approval bypass survived this fixture. This
    /// driver set BOTH from that one boolean, so a case could never tell the
    /// two apart, while the gate read only the descriptor (D-96). The newer
    /// `governance_sources` cases state them independently, and this SDK is the
    /// one that can actually set them independently: its `ModuleDescriptor` is
    /// caller-supplied, not derived from the module.
    fn governance_of(tc: &Value) -> (bool, bool) {
        tc.get("governance_sources").map_or_else(
            || {
                let both = tc["module_requires_approval"].as_bool().unwrap();
                (both, both)
            },
            |src| {
                (
                    src["module"].as_bool().unwrap_or(false),
                    src["descriptor"].as_bool().unwrap_or(false),
                )
            },
        )
    }

    let fixture = load_fixture("approval_gate");
    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();
        let expected = &tc["expected"];
        let module_id = "executor.test.sensitive";
        let (module_declares, descriptor_declares) = governance_of(tc);

        let registry = Arc::new(Registry::new());
        let descriptor = ModuleDescriptor {
            module_id: module_id.to_string(),
            name: None,
            description: "Module gated by the approval step".to_string(),
            documentation: None,
            input_schema: json!({"type": "object"}),
            output_schema: json!({"type": "object"}),
            version: "1.0.0".to_string(),
            tags: vec![],
            annotations: Some(ModuleAnnotations {
                requires_approval: descriptor_declares,
                ..Default::default()
            }),
            examples: vec![],
            metadata: HashMap::new(),
            display: None,
            sunset_date: None,
            dependencies: vec![],
            enabled: true,
        };
        registry
            .register(
                module_id,
                Box::new(SensitiveModule {
                    requires_approval: module_declares,
                }),
                descriptor,
            )
            .unwrap();

        let invocations = Arc::new(AtomicUsize::new(0));
        let mut executor = Executor::new(Arc::clone(&registry), Arc::new(Config::default()));
        if tc["approval_handler_configured"].as_bool().unwrap() {
            let raw = &tc["approval_result"];
            let mut result = ApprovalResult::default();
            if !raw.is_null() {
                result.status = raw["status"].as_str().unwrap().to_string();
                result.approved_by = raw["approved_by"].as_str().map(String::from);
                result.reason = raw["reason"].as_str().map(String::from);
                result.approval_id = raw["approval_id"].as_str().map(String::from);
            }
            executor.set_approval_handler(Box::new(ScriptedHandler {
                result,
                invocations: Arc::clone(&invocations),
            }));
        }

        let outcome: Result<Value, apcore::errors::ModuleError> =
            executor.call(module_id, json!({"v": 1}), None, None).await;

        // `gate_invoked` — counted from the handler itself.
        assert_eq!(
            invocations.load(Ordering::SeqCst) > 0,
            expected["gate_invoked"].as_bool().unwrap(),
            "FAIL [{id}]: gate_invoked — handler ran {} time(s)",
            invocations.load(Ordering::SeqCst)
        );

        // `outcome`
        let actual_outcome = if outcome.is_ok() { "proceed" } else { "error" };
        assert_eq!(
            actual_outcome,
            expected["outcome"].as_str().unwrap(),
            "FAIL [{id}]: outcome — executor returned {outcome:?}"
        );

        match outcome {
            Ok(value) => {
                assert_eq!(
                    value,
                    json!({"v": 1}),
                    "FAIL [{id}]: a proceeding call must reach the module"
                );
            }
            Err(err) => {
                // `error_code` — the wire code the Executor actually raised.
                assert_eq!(
                    serde_json::to_value(err.code).unwrap(),
                    expected["error_code"],
                    "FAIL [{id}]: error_code; message was {}",
                    err.message
                );

                // `approval_id` — round-tripped from the handler's result into
                // the raised error so a caller can resume with `_approval_token`.
                if let Some(want) = expected.get("approval_id") {
                    assert_eq!(
                        err.details.get("approval_id"),
                        Some(want),
                        "FAIL [{id}]: approval_id must be carried on the error; \
                         details were {:?}",
                        err.details
                    );
                }

                // NOTE: `http_status` (403 / 202) is NOT asserted. No apcore SDK
                // exposes an error-code→HTTP-status mapping — apcore-rust has no
                // such accessor on `ErrorCode` or `ModuleError`, and neither
                // apcore-python nor apcore-typescript defines one either. The
                // mapping in protocol-spec.md §7.5 is a contract for the HTTP
                // transport adapters (fastapi-apcore, express-apcore,
                // axum-apcore), not for the core SDKs. Reported as a fixture /
                // spec-layering issue rather than faked here.
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 16. Binding Errors (protocol-spec §5.12.8)
//
// CORRECTED (apcore#93). Four of this fixture's six cases reached no assertion
// that any SDK behaviour could fail:
//
//   * `binding_invalid_target_missing_colon` asserted the FIXTURE'S OWN INPUT
//     back at itself — `assert!(!target.contains(':'))` — a tautology no
//     implementation can break;
//   * `binding_module_not_found` and `binding_schema_inference_failed_python`
//     shared an arm whose whole body was `let _ = input;`;
//   * `binding_file_invalid_missing_bindings_key` read `expected_message` and
//     then discarded it with `let _ = (expected_msg, &err);`, on the stated
//     grounds that the Rust text "may differ". It did differ, and that was the
//     finding, not the excuse: see the two SDK fixes below.
//
// The fixture asserts MESSAGE PARITY (§7.2), which is why apcore-python and
// apcore-typescript construct the canonical error and compare its text.
// apcore-rust has no error classes, so each case here drives the real code path
// that produces the condition and compares the message the SDK emits.
// ---------------------------------------------------------------------------

/// Assert the wire `error_code` the case declares against the code the SDK
/// produced, resolving the fixture's string through serde (never a Rust type
/// name — see [`wire_error_code`]).
fn assert_binding_code(id: &str, tc: &Value, code: ErrorCode) {
    let want = tc["error_code"]
        .as_str()
        .unwrap_or_else(|| panic!("FAIL [{id}]: case declares no error_code"));
    assert_eq!(
        code,
        wire_error_code(id, "binding_errors", want),
        "FAIL [{id}]: wire code"
    );
}

/// Assert the case's declared message expectation against `message`.
///
/// `expected_message` is byte-exact. The fixture writes it with its own
/// illustrative `input.file_path` (`bindings.yaml`), so the ONE substitution
/// applied is that path → the temp path the driver actually loaded; every other
/// character is the fixture's text and decides the outcome.
///
/// A case stating neither expectation is a hard failure — "nothing to assert"
/// is how a case ends up proving nothing.
fn assert_binding_message(id: &str, tc: &Value, message: &str, real_path: Option<&str>) {
    let mut asserted = false;
    if let Some(want) = tc.get("expected_message").and_then(|v| v.as_str()) {
        let declared_path = tc["input"].get("file_path").and_then(|v| v.as_str());
        let want = match (real_path, declared_path) {
            (Some(real), Some(declared)) => want.replace(declared, real),
            _ => want.to_string(),
        };
        assert_eq!(message, want, "FAIL [{id}]: message");
        asserted = true;
    }
    if let Some(fragments) = tc
        .get("expected_message_contains")
        .and_then(|v| v.as_array())
    {
        assert!(
            !fragments.is_empty(),
            "FAIL [{id}]: expected_message_contains is empty, so it asserts nothing"
        );
        for fragment in fragments {
            let frag = fragment
                .as_str()
                .unwrap_or_else(|| panic!("FAIL [{id}]: fragment is not a string"));
            assert!(
                message.contains(frag),
                "FAIL [{id}]: message must contain {frag:?}; got: {message}"
            );
        }
        asserted = true;
    }
    assert!(
        asserted,
        "FAIL [{id}]: states no message expectation this driver can assert"
    );
}

#[test]
#[allow(clippy::too_many_lines)] // one arm per fixture case; splitting it would separate each case from the code path that produces its condition
fn conformance_binding_errors() {
    use apcore::bindings::BindingLoader;
    use apcore::registry::Registry;

    let dir = tempfile::tempdir().expect("tempdir");
    let fixture = load_fixture("binding_errors");
    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();
        let input = &tc["input"];

        match id {
            "binding_file_invalid_missing_bindings_key" => {
                // A bindings document with no top-level `bindings` key.
                let path = dir.path().join("bindings_missing.json");
                std::fs::write(&path, r#"{"spec_version": "1.0"}"#).unwrap();
                let err = BindingLoader::new()
                    .load_from_file(&path)
                    .expect_err(&format!(
                        "FAIL [{id}]: a bindings file with no 'bindings' key must be rejected"
                    ));
                assert_binding_code(id, tc, err.code);
                assert_binding_message(id, tc, &err.message, Some(&path.display().to_string()));
            }

            "binding_schema_mode_conflict" => {
                // Two schema modes on one entry. `modes_listed` is read from the
                // case so the document produces exactly the modes whose names
                // the expected message spells out.
                let module_id = input["module_id"].as_str().unwrap();
                let modes: Vec<&str> = input["modes_listed"]
                    .as_array()
                    .expect("modes_listed is an array")
                    .iter()
                    .map(|m| m.as_str().unwrap())
                    .collect();
                assert_eq!(
                    modes,
                    ["auto_schema", "input_schema/output_schema"],
                    "FAIL [{id}]: this driver builds a document for exactly these two modes"
                );
                let path = dir.path().join("bindings_conflict.yaml");
                std::fs::write(
                    &path,
                    format!(
                        "spec_version: \"1.0\"\nbindings:\n  - module_id: {module_id}\n    \
                         target: \"m:fn\"\n    auto_schema: true\n    input_schema:\n      \
                         type: object\n"
                    ),
                )
                .unwrap();
                let err = BindingLoader::new()
                    .load_from_yaml(&path)
                    .expect_err(&format!(
                        "FAIL [{id}]: two schema modes on one entry must be rejected"
                    ));
                assert_binding_code(id, tc, err.code);
                assert_binding_message(id, tc, &err.message, Some(&path.display().to_string()));
            }

            "pipeline_handler_not_supported_rust" => {
                // Parse a pipeline YAML with a Python-style `handler:` path.
                use apcore::pipeline_config::build_strategy_from_config;
                let yaml_str = format!(
                    "steps:\n  - name: {}\n    handler: {}\n",
                    input["step_name"].as_str().unwrap(),
                    input["handler_path"].as_str().unwrap()
                );
                let yaml_val: Value = serde_yaml_ng::from_str(&yaml_str)
                    .unwrap_or_else(|e| panic!("FAIL [{id}] yaml parse: {e}"));
                let err = build_strategy_from_config(&yaml_val).expect_err(&format!(
                    "FAIL [{id}]: a `handler:` step must be rejected in apcore-rust"
                ));
                assert_binding_code(id, tc, err.code);
                // Fragments read FROM THE FIXTURE. They used to be hardcoded
                // here — `"not supported in apcore-rust" || "register_step_type"`,
                // an OR of two literals — so the fixture's list was decoration
                // and mutating it left this test green (apcore#92).
                assert_binding_message(id, tc, &err.message, None);
            }

            "binding_invalid_target_missing_colon" => {
                // §2.2: a target with no `<module_path>:<symbol>` split is
                // rejected AT PARSE TIME. Before apcore#93 apcore-rust validated
                // nothing here and this arm asserted `!target.contains(':')` —
                // the fixture's own input, restated.
                let target = input["target"].as_str().unwrap();
                let path = dir.path().join("bindings_bad_target.yaml");
                std::fs::write(
                    &path,
                    format!(
                        "spec_version: \"1.0\"\nbindings:\n  - module_id: conformance.bad_target\
                         \n    target: \"{target}\"\n"
                    ),
                )
                .unwrap();
                let err = BindingLoader::new()
                    .load_from_yaml(&path)
                    .expect_err(&format!(
                        "FAIL [{id}]: target {target:?} has no ':' separator and must be \
                         rejected at parse time (protocol-spec §5.12.3)"
                    ));
                assert_binding_code(id, tc, err.code);
                assert_binding_message(id, tc, &err.message, None);
            }

            "binding_module_not_found" => {
                // §2.2.1: "`<module_path>` is resolved by the host language's own
                // module resolution — ... and Rust's handler-map lookup", and a
                // target that does not resolve "fails ... with
                // BindingModuleNotFoundError". apcore-python imports and
                // apcore-typescript `await import()`s; apcore-rust looks the
                // target up in the caller-supplied handler map, so registering
                // with an EMPTY map is the same condition.
                let module_path = input["module_path"].as_str().unwrap();
                let path = dir.path().join("bindings_unresolvable.yaml");
                std::fs::write(
                    &path,
                    format!(
                        "spec_version: \"1.0\"\nbindings:\n  - module_id: conformance.unresolvable\
                         \n    target: \"{module_path}:some_callable\"\n"
                    ),
                )
                .unwrap();
                let mut loader = BindingLoader::new();
                loader
                    .load_from_yaml(&path)
                    .unwrap_or_else(|e| panic!("FAIL [{id}]: document must parse: {}", e.message));
                let err = loader
                    .register_into_with_handlers(&Registry::new(), HashMap::new())
                    .expect_err(&format!(
                        "FAIL [{id}]: target {module_path:?} resolves to no handler and must be \
                         rejected"
                    ));
                assert_binding_code(id, tc, err.code);
                assert_binding_message(id, tc, &err.message, None);
            }

            // Required skip, recorded in the spec repo's
            // `conformance/case_pinning_allowlist.json`:
            // BINDING_SCHEMA_INFERENCE_FAILED is raised when a target's schema
            // cannot be inferred from its signature. apcore-rust resolves a
            // binding through an opaque handler-map key with no compile-time
            // linking (protocol-spec §5.12.3), performs no
            // inference from a target, and so has no such failure to produce
            // for THIS case's shape. Named explicitly rather than folded into a
            // catch-all, so a case this driver has merely not been taught still
            // fails below.
            "binding_schema_inference_failed_python" => {}

            other => panic!(
                "FAIL [binding_errors :: {other}]: this driver does not know the case — teach \
                 the driver, do not skip it. A genuinely per-SDK case belongs in the spec \
                 repo's conformance/case_pinning_allowlist.json with the fixture text that \
                 says so."
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// 17. Binding YAML Canonical Parse (protocol-spec §5.12)
// ---------------------------------------------------------------------------

#[test]
fn conformance_binding_yaml_canonical() {
    use apcore::bindings::BindingLoader;

    let fixtures_root = find_fixtures_root();
    let yaml_path = fixtures_root.join("binding_yaml_canonical.yaml");

    let mut loader = BindingLoader::new();
    loader
        .load_from_yaml(&yaml_path)
        .unwrap_or_else(|e| panic!("FAIL [binding_yaml_canonical]: parse failed: {e}"));

    let mut binding_ids = loader.list_bindings();
    binding_ids.sort_unstable();

    // The canonical YAML defines exactly 3 bindings.
    assert_eq!(
        binding_ids.len(),
        3,
        "FAIL [binding_yaml_canonical]: expected 3 bindings, got {binding_ids:?}"
    );

    // Verify the expected module_ids are present.
    let expected_ids = [
        "conformance.auto_permissive",
        "conformance.explicit_schema",
        "conformance.auto_strict",
    ];
    for expected in &expected_ids {
        assert!(
            binding_ids.contains(expected),
            "FAIL [binding_yaml_canonical]: missing module_id {expected:?}; got {binding_ids:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// 18. Dependency Version Constraints (spec §5.3, §5.15.2)
//
// CORRECTED: this driver used to re-implement constraint checking and
// topological ordering inside the test (a private `check_version_constraint`
// plus a hand-rolled edge walk) and then assert that its OWN result matched the
// fixture. The SDK was never called, so every case was green regardless of what
// `apcore::registry::resolve_dependencies` did. It now drives the real resolver.
// ---------------------------------------------------------------------------

#[test]
fn conformance_dependency_version_constraints() {
    use apcore::registry::{resolve_dependencies, DepInfo};

    let fixture = load_fixture("dependency_version_constraints");
    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();
        let expected = &tc["expected"];
        let expected_outcome = expected["outcome"].as_str().unwrap();

        let mut modules: Vec<(String, Vec<DepInfo>)> = Vec::new();
        let mut versions: HashMap<String, String> = HashMap::new();
        for m in tc["modules"].as_array().unwrap() {
            let module_id = m["module_id"].as_str().unwrap().to_string();
            versions.insert(
                module_id.clone(),
                m["version"].as_str().unwrap().to_string(),
            );
            let deps: Vec<DepInfo> = m["dependencies"]
                .as_array()
                .unwrap()
                .iter()
                .map(|d| DepInfo {
                    module_id: d["module_id"].as_str().unwrap().to_string(),
                    version: d.get("version").and_then(|v| v.as_str()).map(String::from),
                    optional: d
                        .get("optional")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                })
                .collect();
            modules.push((module_id, deps));
        }

        // D-79 needs a dependency that is a KNOWN id and absent from the batch
        // — the only way to stall Kahn's algorithm without a cycle. Cases that
        // do not declare `known_ids` keep the default.
        let known: Option<std::collections::HashSet<String>> = tc.get("known_ids").map(|v| {
            v.as_array()
                .expect("known_ids is an array")
                .iter()
                .map(|x| x.as_str().expect("known id is a string").to_string())
                .collect()
        });
        let result = resolve_dependencies(&modules, known.as_ref(), Some(&versions));

        match expected_outcome {
            "ok" => {
                let order = match result {
                    Ok(order) => order,
                    Err(e) => panic!("FAIL [{id}]: expected ok, resolver returned {e:?}"),
                };

                if let Some(want) = expected.get("load_order").and_then(|v| v.as_array()) {
                    let want: Vec<&str> = want.iter().map(|v| v.as_str().unwrap()).collect();
                    assert_eq!(order, want, "FAIL [{id}]: load_order");
                }

                // `skipped_edges` — an edge that WAS applied forces the
                // dependency ahead of its dependent in the load order. Observing
                // the dependent first is therefore direct evidence the edge was
                // dropped, which is what skipping an unsatisfiable OPTIONAL
                // constraint means. (Turning the same constraint into a hard
                // dependency, or into an error, both turn this red.)
                if let Some(skipped) = expected.get("skipped_edges").and_then(|v| v.as_array()) {
                    for edge in skipped {
                        let pair = edge.as_array().expect("skipped edge is a [from, to] pair");
                        let from = pair[0].as_str().unwrap();
                        let to = pair[1].as_str().unwrap();
                        let pos = |m: &str| {
                            order.iter().position(|x| x == m).unwrap_or_else(|| {
                                panic!("FAIL [{id}]: {m} missing from {order:?}")
                            })
                        };
                        assert!(
                            pos(from) < pos(to),
                            "FAIL [{id}]: skipped_edges declares {from} -> {to} dropped, but the \
                             load order {order:?} still places {to} before {from} — the edge was \
                             enforced"
                        );
                    }
                }
            }
            "error" => {
                let err = match result {
                    Err(e) => e,
                    Ok(order) => panic!(
                        "FAIL [{id}]: expected {} but the resolver returned {order:?}",
                        expected["error_code"]
                    ),
                };
                assert_eq!(
                    serde_json::to_value(err.code).unwrap(),
                    expected["error_code"],
                    "FAIL [{id}]: error_code"
                );
                for field in ["module_id", "dependency_id", "required", "actual"] {
                    assert_eq!(
                        err.details.get(field),
                        expected.get(field),
                        "FAIL [{id}]: error details.{field}; full details {:?}",
                        err.details
                    );
                }
            }
            other => panic!("FAIL [{id}]: unknown outcome {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// 19. Middleware On-Error Recovery (A11)
// ---------------------------------------------------------------------------

#[test]
fn conformance_middleware_on_error_recovery() {
    use apcore::errors::{ErrorCode, ModuleError};

    let fixture = load_fixture("middleware_on_error_recovery");
    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();
        let module_raises_error = tc["module_raises_error"].as_bool().unwrap();
        let module_output = tc.get("module_output").cloned().unwrap_or(Value::Null);
        let after_middleware = tc["after_middleware"].as_array().unwrap();
        let expected = &tc["expected"];
        let expected_outcome = expected["outcome"].as_str().unwrap();
        let expected_invoked: Vec<&str> = expected["after_middleware_invoked"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();

        // Simulate the after-middleware execution loop (Algorithm A11).
        // Rule: after-middleware always runs (even on error); first dict
        // returned by any after-middleware replaces the error result.
        let mut invoked: Vec<String> = Vec::new();
        let mut recovered_result: Option<Value> = None;

        let initial_result: Result<Value, ModuleError> = if module_raises_error {
            Err(ModuleError::new(
                ErrorCode::GeneralInternalError,
                "module error",
            ))
        } else {
            Ok(module_output.clone())
        };

        for mw in after_middleware {
            let mw_id = mw["id"].as_str().unwrap();
            invoked.push(mw_id.to_string());
            let mw_returns = &mw["returns"];

            // First dict recovery (only when module raised an error).
            if initial_result.is_err() && recovered_result.is_none() && mw_returns.is_object() {
                recovered_result = Some(mw_returns.clone());
            }
        }

        // Verify all middleware was invoked.
        let expected_invoked_owned: Vec<String> =
            expected_invoked.iter().map(ToString::to_string).collect();
        assert_eq!(invoked, expected_invoked_owned, "FAIL [{id}] invoked order");

        // Verify final outcome.
        match expected_outcome {
            "success" => {
                let expected_result = &expected["result"];
                let actual_result = if let Some(rec) = &recovered_result {
                    rec
                } else {
                    initial_result.as_ref().ok().unwrap()
                };
                assert_eq!(actual_result, expected_result, "FAIL [{id}] result");
            }
            "error" => {
                assert!(
                    initial_result.is_err() && recovered_result.is_none(),
                    "FAIL [{id}]: expected error outcome but got recovery"
                );
            }
            other => panic!("FAIL [{id}]: unknown outcome {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// 20. Core Schema Structure (no SDK code — pure JSON Schema checks)
// ---------------------------------------------------------------------------

#[test]
fn conformance_core_schema_structure() {
    // acl-config
    let s = load_schema("acl-config");
    let required: Vec<&str> = s["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        required.contains(&"rules"),
        "acl-config: missing 'rules' in required"
    );
    assert!(
        s["properties"].get("default_effect").is_some(),
        "acl-config: missing 'default_effect' property"
    );
    assert!(
        s["properties"].get("audit").is_some(),
        "acl-config: missing 'audit' property"
    );

    // apcore-config
    let s = load_schema("apcore-config");
    let required: Vec<&str> = s["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    // PROTOCOL_SPEC §9.1: a key is required only when it has no canonical
    // default. Exactly two qualify — `version` and `project`. `extensions`,
    // `schema` and `acl` all carry defaults in `defaults.schema.json`, so
    // requiring them would reject a resolvable configuration.
    assert_eq!(
        required,
        vec!["version", "project"],
        "apcore-config: required must be exactly [version, project]; got {required:?}"
    );

    // binding
    let s = load_schema("binding");
    let required: Vec<&str> = s["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        required.contains(&"bindings"),
        "binding: missing 'bindings' in required"
    );
    let entry_required: Vec<&str> = s["$defs"]["BindingEntry"]["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        entry_required.contains(&"module_id"),
        "binding BindingEntry: missing 'module_id'"
    );
    assert!(
        entry_required.contains(&"target"),
        "binding BindingEntry: missing 'target'"
    );

    // module-meta
    let s = load_schema("module-meta");
    for key in &["description", "dependencies", "annotations", "version"] {
        assert!(
            s["properties"].get(*key).is_some(),
            "module-meta: missing property {key:?}"
        );
    }

    // module-schema
    let s = load_schema("module-schema");
    let required: Vec<&str> = s["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    for key in &["module_id", "description", "input_schema", "output_schema"] {
        assert!(
            required.contains(key),
            "module-schema: missing {key:?} in required; got {required:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// 21. Defaults Schema Completeness
// ---------------------------------------------------------------------------

#[test]
fn conformance_defaults_schema_completeness() {
    // Verify the defaults schema itself is valid JSON and contains expected
    // top-level namespace keys.
    let schema = load_schema("defaults");

    let expected_namespaces = ["extensions", "schema", "acl", "executor", "observability"];
    for ns in &expected_namespaces {
        assert!(
            schema["properties"].get(*ns).is_some(),
            "defaults schema: missing namespace {ns:?}"
        );
    }

    // Spot-check a few leaf defaults match what Config::default() returns.
    let config = Config::default();

    // executor.max_call_depth default in schema
    let schema_max_depth = schema["properties"]["executor"]["properties"]["max_call_depth"]
        .get("default")
        .and_then(serde_json::Value::as_u64);
    if let Some(schema_val) = schema_max_depth {
        let config_val = config
            .get("executor.max_call_depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        assert_eq!(
            schema_val, config_val,
            "defaults schema executor.max_call_depth mismatch"
        );
    }

    // executor.default_timeout
    let schema_timeout = schema["properties"]["executor"]["properties"]["default_timeout"]
        .get("default")
        .and_then(serde_json::Value::as_u64);
    if let Some(schema_val) = schema_timeout {
        let config_val = config
            .get("executor.default_timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        assert_eq!(
            schema_val, config_val,
            "defaults schema executor.default_timeout mismatch"
        );
    }
}

// ---------------------------------------------------------------------------
// 22. Sys Module Output Schema Required Fields
// ---------------------------------------------------------------------------

#[test]
fn conformance_sys_module_output_schemas() {
    let cases = vec![
        (
            "sys-control-update-config",
            vec!["success", "key", "old_value", "new_value"],
        ),
        ("sys-control-reload-module", vec!["success", "module_id"]),
        (
            "sys-control-toggle-feature",
            vec!["success", "module_id", "enabled"],
        ),
        ("sys-health-summary", vec!["project", "summary", "modules"]),
        (
            "sys-health-module",
            vec![
                "module_id",
                "status",
                "total_calls",
                "error_count",
                "error_rate",
            ],
        ),
        ("sys-manifest-module", vec!["module_id", "description"]),
        (
            "sys-manifest-full",
            vec!["project_name", "module_count", "modules"],
        ),
    ];

    for (schema_name, expected_required) in cases {
        let s = load_schema(schema_name);
        let actual_required: Vec<&str> = s["required"]
            .as_array()
            .unwrap_or_else(|| {
                panic!("schema {schema_name}: 'required' array missing or not an array")
            })
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        for key in &expected_required {
            assert!(
                actual_required.contains(key),
                "schema {schema_name}: missing required key {key:?}; got {actual_required:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Context.create unified signature (apcore Issue #66)
//
// Drives every case in `context_create.json` against the Rust SDK. The
// fixture validates the v0.22.0 normative input list (identity, trace_parent,
// cancel_token, data, services, global_deadline), the removal of `executor`
// and `caller_id` as inputs, the Executor binding rules, and child()
// propagation.
//
// Each case is dispatched by its `id` because the assertions are
// case-specific (some construct only a Context, others spin up an Executor
// and exercise the binding pipeline).
// ---------------------------------------------------------------------------

#[tokio::test]
#[allow(clippy::too_many_lines)] // case-per-id dispatch is intentionally long
async fn conformance_context_create() {
    use apcore::cancel::CancelToken;
    use apcore::executor::Executor;
    use apcore::module::Module;
    use apcore::registry::{ModuleDescriptor, Registry};
    use apcore::trace_context::TraceParent;
    use apcore::APCore;
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::Arc;

    // Trivial echo module used by binding cases — returns the inputs as-is.
    struct EchoModule;
    #[async_trait]
    impl Module for EchoModule {
        fn input_schema(&self) -> Value {
            json!({"type": "object"})
        }
        fn output_schema(&self) -> Value {
            json!({"type": "object"})
        }
        fn description(&self) -> &'static str {
            "Echo module for conformance tests"
        }
        async fn execute(
            &self,
            inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Value, apcore::errors::ModuleError> {
            Ok(inputs)
        }
    }

    fn register_echo(registry: &Registry, module_id: &str) {
        let descriptor = ModuleDescriptor {
            module_id: module_id.to_string(),
            name: None,
            description: "Echo module for conformance tests".to_string(),
            documentation: None,
            input_schema: json!({"type": "object"}),
            output_schema: json!({"type": "object"}),
            version: "1.0.0".to_string(),
            tags: vec![],
            annotations: None,
            examples: vec![],
            metadata: HashMap::new(),
            display: None,
            sunset_date: None,
            dependencies: vec![],
            enabled: true,
        };
        registry
            .register(module_id, Box::new(EchoModule), descriptor)
            .unwrap();
    }

    // Compile-time pin of the `Context::create` signature.
    //
    // This coercion type-checks only while `create` takes exactly these six
    // parameters. None of them is an executor, a caller_id, or a standalone
    // tracestate — so adding any of the three breaks the BUILD. That is the
    // strongest form the fixture's three `*_is_not_a_parameter` expectations
    // can take in a statically typed SDK: they are about the shape of the API,
    // and here the compiler is the thing that checks it.
    type ContextCreateFn = fn(
        Option<Identity>,
        Option<TraceParent>,
        Option<CancelToken>,
        Option<HashMap<String, Value>>,
        Value,
        Option<f64>,
    ) -> Context<Value>;
    let context_create_signature: ContextCreateFn = Context::<Value>::create;
    // Use the pinned pointer so it is not merely a declaration: the very first
    // Context the loop needs is built through it.
    let pinned_default_ctx = context_create_signature(None, None, None, None, Value::Null, None);
    assert!(
        pinned_default_ctx.executor.is_none() && pinned_default_ctx.caller_id.is_none(),
        "a Context built through the pinned six-parameter signature is unbound and top-level"
    );

    let fixture = load_fixture("context_create");

    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();
        let expected = &tc["expected"];
        match id {
            "create_minimal_all_defaults" => {
                let ctx: Context<Value> =
                    Context::create(None, None, None, None, Value::Null, None);

                // `trace_id_pattern` — the fixture's own regex, applied to the
                // trace_id the SDK generated.
                let pattern = expected["trace_id_pattern"].as_str().unwrap();
                assert!(
                    regex::Regex::new(pattern).unwrap().is_match(&ctx.trace_id),
                    "FAIL [{id}]: trace_id {:?} does not match {pattern}",
                    ctx.trace_id
                );

                assert_eq!(
                    json!(ctx.identity.as_ref().map(Identity::id)),
                    expected["identity"],
                    "FAIL [{id}]: identity"
                );
                assert_eq!(
                    ctx.executor.is_none(),
                    expected["executor"].is_null(),
                    "FAIL [{id}]: executor"
                );
                assert_eq!(
                    ctx.cancel_token.is_none(),
                    expected["cancel_token"].is_null(),
                    "FAIL [{id}]: cancel_token"
                );
                assert_eq!(ctx.services, expected["services"], "FAIL [{id}]: services");
                assert_eq!(
                    json!(ctx.global_deadline),
                    expected["global_deadline"],
                    "FAIL [{id}]: global_deadline"
                );
                assert_eq!(
                    json!(ctx.caller_id),
                    expected["caller_id"],
                    "FAIL [{id}]: caller_id"
                );
                assert_eq!(
                    json!(ctx.call_chain),
                    expected["call_chain"],
                    "FAIL [{id}]: call_chain"
                );

                // `data_empty`
                assert_eq!(
                    ctx.data.read().is_empty(),
                    expected["data_empty"].as_bool().unwrap(),
                    "FAIL [{id}]: data_empty"
                );
            }
            "create_with_identity_only" => {
                let input_identity = &tc["input"]["identity"];
                let identity = Identity::new(
                    input_identity["id"].as_str().unwrap().to_string(),
                    input_identity["type"].as_str().unwrap().to_string(),
                    input_identity["roles"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_str().unwrap().to_string())
                        .collect(),
                    HashMap::new(),
                );
                let ctx: Context<Value> =
                    Context::create(Some(identity), None, None, None, Value::Null, None);

                let pattern = expected["trace_id_pattern"].as_str().unwrap();
                assert!(
                    regex::Regex::new(pattern).unwrap().is_match(&ctx.trace_id),
                    "FAIL [{id}]: trace_id {:?} does not match {pattern}",
                    ctx.trace_id
                );
                assert_eq!(
                    json!(ctx.identity.as_ref().map(Identity::id)),
                    expected["identity_id"],
                    "FAIL [{id}]: identity_id"
                );
                assert_eq!(
                    ctx.executor.is_none(),
                    expected["executor"].is_null(),
                    "FAIL [{id}]: executor"
                );
                assert_eq!(
                    ctx.cancel_token.is_none(),
                    expected["cancel_token"].is_null(),
                    "FAIL [{id}]: cancel_token"
                );
            }
            "create_with_cancel_token" => {
                let token = CancelToken::new();
                let ctx: Context<Value> =
                    Context::create(None, None, Some(token.clone()), None, Value::Null, None);

                // `cancel_token_bound`
                assert_eq!(
                    ctx.cancel_token.is_some(),
                    expected["cancel_token_bound"].as_bool().unwrap(),
                    "FAIL [{id}]: cancel_token_bound"
                );

                // `cancel_token_matches_input` — the SAME handle, observed by
                // cancelling the caller's side and reading the context's.
                token.cancel();
                let matches_input = ctx.cancel_token.as_ref().unwrap().is_cancelled();
                assert_eq!(
                    matches_input,
                    expected["cancel_token_matches_input"].as_bool().unwrap(),
                    "FAIL [{id}]: cancel_token_matches_input — the bound token \
                     does not observe the caller-supplied handle's cancellation"
                );

                // `executor_at_create_time`
                assert_eq!(
                    ctx.executor.is_none(),
                    expected["executor_at_create_time"].is_null(),
                    "FAIL [{id}]: executor_at_create_time"
                );
            }
            "create_with_global_deadline" => {
                let deadline = tc["input"]["global_deadline"].as_f64().unwrap();
                let ctx: Context<Value> =
                    Context::create(None, None, None, None, Value::Null, Some(deadline));
                assert_eq!(
                    json!(ctx.global_deadline),
                    expected["global_deadline"],
                    "FAIL [{id}]: global_deadline must be preserved"
                );
                assert_eq!(
                    ctx.executor.is_none(),
                    expected["executor"].is_null(),
                    "FAIL [{id}]: executor"
                );
            }
            "create_rejects_executor_input" => {
                // `executor_is_not_a_parameter` — enforced at compile time by
                // `_CONTEXT_CREATE_SIGNATURE` above: an `executor` parameter
                // cannot be added without breaking that coercion. The runtime
                // half confirms the consequence a caller can see, which is that
                // no caller-constructed Context arrives pre-bound.
                assert!(
                    expected["executor_is_not_a_parameter"].as_bool().unwrap(),
                    "FAIL [{id}]: this driver only knows how to verify the \
                     'no such parameter' branch of the contract"
                );
                let ctx: Context<Value> =
                    Context::create(None, None, None, None, Value::Null, None);
                assert!(
                    ctx.executor.is_none(),
                    "FAIL [{id}]: caller cannot supply executor"
                );
            }
            "create_rejects_caller_id_input" => {
                // `caller_id_is_not_a_parameter` — same compile-time pin.
                assert!(
                    expected["caller_id_is_not_a_parameter"].as_bool().unwrap(),
                    "FAIL [{id}]: this driver only knows how to verify the \
                     'no such parameter' branch of the contract"
                );
                let ctx: Context<Value> =
                    Context::create(None, None, None, None, Value::Null, None);

                // `caller_id_after_create`
                assert_eq!(
                    json!(ctx.caller_id),
                    expected["caller_id_after_create"],
                    "FAIL [{id}]: caller_id_after_create"
                );
            }
            "executor_binds_on_first_call_local" => {
                let client = APCore::new();
                let call_module = tc["input"]["call_module"].as_str().unwrap();
                register_echo(client.registry(), call_module);
                let ctx: Context<Value> =
                    Context::create(None, None, None, None, Value::Null, None);

                // `executor_at_create_time`
                assert_eq!(
                    ctx.executor.is_none(),
                    expected["executor_at_create_time"].is_null(),
                    "FAIL [{id}]: executor_at_create_time"
                );

                // `raised_binding_error` — `Executor::call` clones the Context
                // before binding, so the caller's handle never observes the
                // mutation (documented in the fixture). Bind explicitly to get
                // at the binding result itself.
                let exec = client.executor();
                let handle: Arc<dyn std::any::Any + Send + Sync> = exec.instance_handle();
                let mut ctx_for_bind = ctx.clone();
                let bind_result = ctx_for_bind.bind_executor(handle);
                assert_eq!(
                    bind_result.is_err(),
                    expected["raised_binding_error"].as_bool().unwrap(),
                    "FAIL [{id}]: raised_binding_error — bind returned {bind_result:?}"
                );
                assert!(
                    ctx_for_bind.executor.is_some(),
                    "FAIL [{id}]: executor must be bound after first call"
                );

                // `call_succeeded`
                let result = exec.call(call_module, json!({"v": 1}), None, None).await;
                assert_eq!(
                    result.is_ok(),
                    expected["call_succeeded"].as_bool().unwrap(),
                    "FAIL [{id}]: call_succeeded — call returned {result:?}"
                );
            }
            "executor_binds_idempotent_same_instance" => {
                let client = APCore::new();
                let calls: Vec<&str> = tc["input"]["calls"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap())
                    .collect();
                register_echo(client.registry(), calls[0]);
                let exec = client.executor();
                let mut ctx: Context<Value> =
                    Context::create(None, None, None, None, Value::Null, None);
                ctx.bind_executor(exec.instance_handle()).unwrap();
                let first_handle = ctx.executor.clone().expect("bound on first call");

                // `rebind_noop` — re-binding the SAME Executor instance must
                // neither raise nor replace the stored handle.
                let rebinds: Vec<Result<(), _>> = vec![
                    ctx.bind_executor(exec.instance_handle()),
                    ctx.bind_executor(exec.instance_handle()),
                ];
                let rebind_noop = rebinds.iter().all(Result::is_ok)
                    && Arc::ptr_eq(&first_handle, ctx.executor.as_ref().unwrap());
                assert_eq!(
                    rebind_noop,
                    expected["rebind_noop"].as_bool().unwrap(),
                    "FAIL [{id}]: rebind_noop — rebinds {rebinds:?}"
                );

                // `raised_error` / `executor_identity_stable` across repeated
                // top-level calls sharing one Context.
                let mut results = Vec::new();
                for (i, module_id) in calls.iter().enumerate() {
                    results.push(
                        exec.call(module_id, json!({"i": i}), Some(&ctx), None)
                            .await,
                    );
                }
                assert_eq!(
                    results.iter().any(Result::is_err),
                    expected["raised_error"].as_bool().unwrap(),
                    "FAIL [{id}]: raised_error — {results:?}"
                );
                assert_eq!(
                    Arc::ptr_eq(&first_handle, ctx.executor.as_ref().unwrap()),
                    expected["executor_identity_stable"].as_bool().unwrap(),
                    "FAIL [{id}]: executor_identity_stable"
                );
            }
            "executor_rejects_cross_executor_rebind" => {
                // CORRECTED (apcore#92): this branch hardcoded the raise and
                // named the fixture's `expected_one_of: [raise, silent_accept]`
                // only in a comment, so mutating the whole expectation block
                // left the case green — nothing here read the fixture. The
                // alternation is gone as of spec v1.11.0: all three SDKs raise,
                // so the SHOULD became a MUST and the expectation is now
                // `{raises: true, error_code: "CONTEXT_BINDING_ERROR"}`, read
                // below. The wire code is what is asserted — `ContextBindingError`
                // as a *type* name is a class two SDKs share and this one does
                // not have.
                let registry_a = Arc::new(Registry::new());
                let registry_b = Arc::new(Registry::new());
                let cfg = Arc::new(Config::default());
                let exec_a = Executor::new(registry_a, Arc::clone(&cfg));
                let exec_b = Executor::new(registry_b, cfg);
                let mut ctx: Context<Value> =
                    Context::create(None, None, None, None, Value::Null, None);
                ctx.bind_executor(exec_a.instance_handle()).unwrap();
                let rebind = ctx.bind_executor(exec_b.instance_handle());

                let raises = expected["raises"].as_bool().unwrap_or_else(|| {
                    panic!(
                        "FAIL [{id}]: `raises` must be a bool, got {}",
                        expected["raises"]
                    )
                });
                assert!(
                    raises,
                    "FAIL [{id}]: context_create.json says a cross-Executor rebind need not \
                     raise, but apcore v1.11.0 made raising a MUST — teach the driver against \
                     the spec, do not weaken the SDK"
                );
                let err = rebind.err().unwrap_or_else(|| {
                    panic!(
                        "FAIL [{id}]: rebinding a Context to a second Executor was accepted; \
                         the executor handle is now {:?}",
                        ctx.executor.is_some()
                    )
                });
                let wire = expected["error_code"].as_str().unwrap_or_else(|| {
                    panic!(
                        "FAIL [{id}]: `error_code` must be a wire code string, got {}",
                        expected["error_code"]
                    )
                });
                assert_eq!(
                    err.code,
                    wire_error_code(id, "context_create.json", wire),
                    "FAIL [{id}]: expected wire code {wire}, got {} ({})",
                    err.code.wire_str(),
                    err.message
                );
            }
            "child_propagates_executor" => {
                let client = APCore::new();
                let exec = client.executor();
                let target = tc["input"]["create_child_module_id"].as_str().unwrap();
                // The parent is itself a child so its call_chain is non-empty —
                // otherwise `child_caller_id_from_parent_chain_tip` would be
                // satisfied vacuously by two Nones.
                let mut root: Context<Value> =
                    Context::create(None, None, None, None, Value::Null, None);
                root.bind_executor(exec.instance_handle()).unwrap();
                let parent = root.child("orchestrator.main");
                let child = parent.child(target);

                // `child_executor_matches_parent`
                let executor_matches = match (parent.executor.as_ref(), child.executor.as_ref()) {
                    (Some(p), Some(c)) => Arc::ptr_eq(p, c),
                    _ => false,
                };
                assert_eq!(
                    executor_matches,
                    expected["child_executor_matches_parent"].as_bool().unwrap(),
                    "FAIL [{id}]: child_executor_matches_parent"
                );

                // `child_caller_id_from_parent_chain_tip`
                let caller_id_from_tip =
                    child.caller_id.as_deref() == parent.call_chain.last().map(String::as_str);
                assert_eq!(
                    caller_id_from_tip,
                    expected["child_caller_id_from_parent_chain_tip"]
                        .as_bool()
                        .unwrap(),
                    "FAIL [{id}]: child_caller_id_from_parent_chain_tip — child.caller_id={:?}, \
                     parent.call_chain={:?}",
                    child.caller_id,
                    parent.call_chain
                );

                // `child_call_chain_appends_target`
                let mut want_chain = parent.call_chain.clone();
                want_chain.push(target.to_string());
                assert_eq!(
                    child.call_chain == want_chain,
                    expected["child_call_chain_appends_target"]
                        .as_bool()
                        .unwrap(),
                    "FAIL [{id}]: child_call_chain_appends_target — got {:?}, want {want_chain:?}",
                    child.call_chain
                );
            }
            "child_propagates_cancel_token" => {
                let token = CancelToken::new();
                let parent: Context<Value> =
                    Context::create(None, None, Some(token.clone()), None, Value::Null, None);
                let child = parent.child(tc["input"]["create_child_module_id"].as_str().unwrap());

                // `child_cancel_token_bound`
                assert_eq!(
                    child.cancel_token.is_some(),
                    expected["child_cancel_token_bound"].as_bool().unwrap(),
                    "FAIL [{id}]: child_cancel_token_bound"
                );

                // `child_cancel_token_matches_parent` — cancelling through the
                // parent's handle must be observable from the child's.
                token.cancel();
                let matches_parent = child.cancel_token.as_ref().unwrap().is_cancelled();
                assert_eq!(
                    matches_parent,
                    expected["child_cancel_token_matches_parent"]
                        .as_bool()
                        .unwrap(),
                    "FAIL [{id}]: child_cancel_token_matches_parent — a module deep \
                     in the call chain would not observe cancellation"
                );
            }
            "deserialize_then_call_binds_local_executor" => {
                let serialized = tc["input"]["serialized_context"].clone();
                let ctx_des: Context<Value> = Context::deserialize(serialized).unwrap();

                // Fields stripped by §5.7 must arrive absent.
                assert_eq!(
                    ctx_des.executor.is_none(),
                    expected["executor_after_deserialize"].is_null(),
                    "FAIL [{id}]: executor_after_deserialize"
                );
                assert_eq!(
                    ctx_des.cancel_token.is_none(),
                    expected["cancel_token_after_deserialize"].is_null(),
                    "FAIL [{id}]: cancel_token_after_deserialize"
                );
                assert_eq!(
                    ctx_des.services, expected["services_after_deserialize"],
                    "FAIL [{id}]: services_after_deserialize"
                );
                assert_eq!(
                    json!(ctx_des.global_deadline),
                    expected["global_deadline_after_deserialize"],
                    "FAIL [{id}]: global_deadline_after_deserialize"
                );
                assert_eq!(
                    json!(ctx_des.caller_id),
                    expected["caller_id_preserved"],
                    "FAIL [{id}]: caller_id_preserved"
                );

                // `executor_bound_on_first_call` — the receiving node binds its
                // OWN executor to the arriving Context.
                let client = APCore::new();
                let exec = client.executor();
                let mut ctx_bound = ctx_des.clone();
                ctx_bound.bind_executor(exec.instance_handle()).unwrap();
                assert_eq!(
                    ctx_bound.executor.is_some(),
                    expected["executor_bound_on_first_call"].as_bool().unwrap(),
                    "FAIL [{id}]: executor_bound_on_first_call"
                );
            }
            "distributed_cancel_token_post_deserialize_null" => {
                // Negative invariant (PROTOCOL_SPEC §5.7): cancel_token MUST NOT
                // serialize across process boundaries.
                use apcore::cancel::CancelToken;
                let token = CancelToken::new();
                let ctx_with_token: Context<Value> =
                    Context::create(None, None, Some(token), None, Value::Null, None);
                assert!(
                    ctx_with_token.cancel_token.is_some(),
                    "FAIL [{id}]: pre-condition — cancel_token must be set before serialize"
                );
                let serialized = ctx_with_token.serialize();
                let field_on_the_wire =
                    serialized.as_object().unwrap().contains_key("cancel_token");
                let ctx_des: Context<Value> = Context::deserialize(serialized).unwrap();

                // `cancel_token_after_deserialize`
                assert_eq!(
                    ctx_des.cancel_token.is_none(),
                    expected["cancel_token_after_deserialize"].is_null(),
                    "FAIL [{id}]: cancel_token_after_deserialize"
                );

                // `no_in_context_token_rides_across_processes` — neither the
                // wire form nor the rebuilt Context may carry the token.
                let no_token_rides = !field_on_the_wire && ctx_des.cancel_token.is_none();
                assert_eq!(
                    no_token_rides,
                    expected["no_in_context_token_rides_across_processes"]
                        .as_bool()
                        .unwrap(),
                    "FAIL [{id}]: no_in_context_token_rides_across_processes — \
                     cancel_token present on the wire: {field_on_the_wire}"
                );
            }
            "distributed_global_deadline_post_deserialize_null" => {
                // Negative invariant (PROTOCOL_SPEC §5.7): global_deadline MUST
                // NOT serialize across process boundaries.
                let ctx_with_deadline: Context<Value> =
                    Context::create(None, None, None, None, Value::Null, Some(9_999_999.0));
                assert!(
                    ctx_with_deadline.global_deadline.is_some(),
                    "FAIL [{id}]: pre-condition — global_deadline must be set before serialize"
                );
                let serialized = ctx_with_deadline.serialize();
                let field_on_the_wire = serialized
                    .as_object()
                    .unwrap()
                    .contains_key("global_deadline");
                let ctx_des: Context<Value> = Context::deserialize(serialized).unwrap();

                // `global_deadline_after_deserialize`
                assert_eq!(
                    json!(ctx_des.global_deadline),
                    expected["global_deadline_after_deserialize"],
                    "FAIL [{id}]: global_deadline_after_deserialize"
                );

                // `no_remote_deadline_rides_via_global_deadline_field`
                let no_deadline_rides = !field_on_the_wire && ctx_des.global_deadline.is_none();
                assert_eq!(
                    no_deadline_rides,
                    expected["no_remote_deadline_rides_via_global_deadline_field"]
                        .as_bool()
                        .unwrap(),
                    "FAIL [{id}]: no_remote_deadline_rides_via_global_deadline_field — \
                     global_deadline present on the wire: {field_on_the_wire}"
                );
            }
            "tracestate_carried_inside_traceparent" => {
                let tp_input = &tc["input"]["trace_parent"];
                let tracestate: Vec<(String, String)> = tp_input["tracestate"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|pair| {
                        let arr = pair.as_array().unwrap();
                        (
                            arr[0].as_str().unwrap().to_string(),
                            arr[1].as_str().unwrap().to_string(),
                        )
                    })
                    .collect();
                let trace_parent = TraceParent {
                    version: 0,
                    trace_id: tp_input["trace_id"].as_str().unwrap().to_string(),
                    parent_id: tp_input["parent_id"].as_str().unwrap().to_string(),
                    trace_flags: u8::from_str_radix(tp_input["trace_flags"].as_str().unwrap(), 16)
                        .unwrap(),
                    tracestate: tracestate.clone(),
                };
                let ctx: Context<Value> =
                    Context::create(None, Some(trace_parent), None, None, Value::Null, None);
                assert_eq!(
                    json!(ctx.trace_id),
                    expected["trace_id"],
                    "FAIL [{id}]: trace_id must be inherited from trace_parent"
                );

                // `tracestate_preserved` — every inbound vendor/value pair must
                // survive into the outbound header.
                let headers = apcore::trace_context::TraceContext::inject(&ctx);
                let outbound = headers.get("tracestate").cloned().unwrap_or_default();
                let preserved = tracestate
                    .iter()
                    .all(|(k, v)| outbound.contains(&format!("{k}={v}")));
                assert_eq!(
                    preserved,
                    expected["tracestate_preserved"].as_bool().unwrap(),
                    "FAIL [{id}]: tracestate_preserved — outbound header was {outbound:?}"
                );

                // `no_separate_tracestate_parameter` — TraceParent is the ONLY
                // carrier: the compile-time signature pin above admits no
                // tracestate argument, and a Context built without a
                // TraceParent emits no tracestate at all.
                let bare: Context<Value> =
                    Context::create(None, None, None, None, Value::Null, None);
                let bare_headers = apcore::trace_context::TraceContext::inject(&bare);
                let only_via_trace_parent =
                    !outbound.is_empty() && !bare_headers.contains_key("tracestate");
                assert_eq!(
                    only_via_trace_parent,
                    expected["no_separate_tracestate_parameter"]
                        .as_bool()
                        .unwrap(),
                    "FAIL [{id}]: no_separate_tracestate_parameter — a Context created \
                     without a TraceParent emitted {:?}",
                    bare_headers.get("tracestate")
                );
            }
            _ => panic!("Unhandled context_create fixture case: {id} — add a branch above."),
        }
    }
}

// ---------------------------------------------------------------------------
// Error-recovery metadata: defaults resolved from error code.
//
// For each fixture case, a default error constructed with the given code must
// carry the expected `retryable` and `user_fixable` values.
// ---------------------------------------------------------------------------

#[test]
fn conformance_error_recovery_metadata_defaults() {
    use apcore::errors::{ErrorCode, ModuleError};

    let fixture = load_fixture("error_recovery_metadata");
    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();
        let code_str = tc["code"].as_str().unwrap();
        let expected_retryable = &tc["expected"]["retryable"];
        let expected_user_fixable = &tc["expected"]["user_fixable"];

        // Resolve the wire code string to the typed ErrorCode via serde.
        let code: ErrorCode = serde_json::from_value(Value::String(code_str.to_string()))
            .unwrap_or_else(|e| panic!("FAIL [{id}]: unknown error code '{code_str}': {e}"));

        let err = ModuleError::new(code, "conformance");

        let expected_retryable = if expected_retryable.is_null() {
            None
        } else {
            Some(
                expected_retryable
                    .as_bool()
                    .unwrap_or_else(|| panic!("FAIL [{id}]: retryable must be bool or null")),
            )
        };
        let expected_user_fixable = if expected_user_fixable.is_null() {
            None
        } else {
            Some(
                expected_user_fixable
                    .as_bool()
                    .unwrap_or_else(|| panic!("FAIL [{id}]: user_fixable must be bool or null")),
            )
        };

        assert_eq!(
            err.retryable, expected_retryable,
            "FAIL [{id}]: code '{code_str}' retryable mismatch"
        );
        assert_eq!(
            err.user_fixable, expected_user_fixable,
            "FAIL [{id}]: code '{code_str}' user_fixable mismatch"
        );
    }
}

// ---------------------------------------------------------------------------
// ACL agent tool-governance (issue #72)
//
// One shared default-deny ruleset scopes tool access by caller pattern +
// identity roles + call-chain depth. Each case is a
// (caller_id, caller_identity, call_depth, target_id) -> decision tuple.
// Mirrors `conformance_acl_evaluation` machinery, but `default_effect`/`rules`
// are shared at the top level rather than per-case.
// ---------------------------------------------------------------------------

#[test]
fn conformance_acl_agent_scoping() {
    ACL::init_builtin_handlers();
    let fixture = load_fixture("acl_agent_scoping");

    let default_effect = fixture["default_effect"].as_str().unwrap();
    let rules: Vec<ACLRule> = fixture["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let mut rule = ACLRule::new(
                r["callers"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_string())
                    .collect(),
                r["targets"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_string())
                    .collect(),
                r["effect"].as_str().unwrap().to_string(),
            );
            rule.description = r
                .get("description")
                .and_then(|v| v.as_str())
                .map(String::from);
            rule.conditions = r.get("conditions").cloned();
            rule
        })
        .collect();

    // Build the ACL once from the shared ruleset (spec §6 first-match-wins).
    let acl = ACL::new(rules, default_effect, None);

    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();
        let caller_id_val = &tc["caller_id"];
        let target_id = tc["target_id"].as_str().unwrap();
        let expected = tc["expected"].as_bool().unwrap();

        // `caller_identity` may be present-and-an-object, or present-and-null
        // (the "agent without identity" case). Only a real object yields an
        // identity; null/absent means no identity on the context.
        let identity_obj = tc.get("caller_identity").filter(|v| v.is_object());

        // A context is needed whenever the case carries identity or a call
        // depth — same condition shape as `conformance_acl_evaluation`.
        let call_depth = tc
            .get("call_depth")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let needs_context = identity_obj.is_some() || call_depth > 0;

        let ctx: Option<Context<Value>> = if needs_context {
            let identity = identity_obj.map(|id_data| {
                Identity::new(
                    caller_id_val.as_str().unwrap_or("unknown").to_string(),
                    id_data
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("user")
                        .to_string(),
                    id_data
                        .get("roles")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .map(|v| v.as_str().unwrap().to_string())
                                .collect()
                        })
                        .unwrap_or_default(),
                    HashMap::new(),
                )
            });

            let mut ctx: Context<Value> =
                Context::create(identity, None, None, None, Value::Null, None);
            for i in 0..call_depth {
                ctx.call_chain.push(format!("_depth_{i}"));
            }
            Some(ctx)
        } else {
            None
        };

        let caller_id = if caller_id_val.is_null() {
            None
        } else {
            Some(caller_id_val.as_str().unwrap())
        };

        let result = acl.check(caller_id, target_id, ctx.as_ref());

        assert_eq!(
            result, expected,
            "FAIL [{id}]: ACL check(caller={caller_id:?}, target={target_id:?}) returned {result}, expected {expected}"
        );
    }
}

// ---------------------------------------------------------------------------
// Per-instance ToggleState isolation (issue #71)
//
// Each named instance is a real `APCore` constructed in this single process.
// Operations drive the owning instance's toggle WRITE path
// (`system.control.toggle_feature` via `disable`/`enable`); `reload`
// re-registers that instance's modules while preserving its ToggleState. The
// disabled-set is then asserted via each instance's READ path (its own
// `Arc<ToggleState>`), proving disabling on A does not affect B.
// ---------------------------------------------------------------------------

#[tokio::test]
#[allow(clippy::too_many_lines)] // fixture-driven runner: construct, drive, assert per case
async fn conformance_toggle_state_isolation() {
    use apcore::config::Config;
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::module::Module;
    use apcore::APCore;
    use async_trait::async_trait;
    use serde_json::json;
    use std::collections::{HashMap, HashSet};

    /// Trivial no-op module: the toggle write path requires the referenced
    /// module to exist in the registry, so each `module_id` is registered as
    /// one of these before being toggled.
    struct NoopModule;

    #[async_trait]
    impl Module for NoopModule {
        fn input_schema(&self) -> Value {
            json!({"type": "object"})
        }
        fn output_schema(&self) -> Value {
            json!({"type": "object"})
        }
        fn description(&self) -> &'static str {
            "no-op module for toggle isolation conformance"
        }
        async fn execute(
            &self,
            inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Value, ModuleError> {
            Ok(inputs)
        }
    }

    /// Production-like config: sys_modules + events enabled so Rust
    /// auto-registers `system.control.toggle_feature` (the write path).
    fn sys_config() -> Config {
        let mut config = Config::default();
        config.set("sys_modules.enabled", json!(true));
        config.set("sys_modules.events.enabled", json!(true));
        config
    }

    let fixture = load_fixture("toggle_state_isolation");

    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();

        // Collect every module_id referenced by this case so each instance can
        // pre-register them (reload re-registers this same set).
        let mut referenced: HashSet<String> = HashSet::new();
        for op in tc["operations"].as_array().unwrap() {
            if let Some(m) = op.get("module_id").and_then(|v| v.as_str()) {
                referenced.insert(m.to_string());
            }
        }
        for ids in tc["expected_disabled"].as_object().unwrap().values() {
            for m in ids.as_array().unwrap() {
                referenced.insert(m.as_str().unwrap().to_string());
            }
        }

        // Helper: (re-)register the referenced modules on an instance.
        let register_modules = |client: &APCore| {
            for m in &referenced {
                // Re-registration is idempotent at the toggle level: a reload
                // resets the descriptor but ToggleState is preserved.
                let _ = client.register(m, Box::new(NoopModule));
            }
        };

        // Construct one real APCore per instance name, in the SAME process.
        let mut instances: HashMap<String, APCore> = HashMap::new();
        for name in tc["instances"].as_array().unwrap() {
            let name = name.as_str().unwrap().to_string();
            let client = APCore::with_config(sys_config());
            register_modules(&client);
            instances.insert(name, client);
        }

        // Apply each operation in order through the owning instance's write path.
        for op in tc["operations"].as_array().unwrap() {
            let inst_name = op["instance"].as_str().unwrap();
            let action = op["action"].as_str().unwrap();
            let client = instances
                .get(inst_name)
                .unwrap_or_else(|| panic!("FAIL [{id}]: unknown instance '{inst_name}'"));
            match action {
                "disable" => {
                    let module_id = op["module_id"].as_str().unwrap();
                    client.disable(module_id, None).await.unwrap_or_else(|e| {
                        panic!("FAIL [{id}]: disable {module_id} on {inst_name}: {e:?}")
                    });
                }
                "enable" => {
                    let module_id = op["module_id"].as_str().unwrap();
                    client.enable(module_id, None).await.unwrap_or_else(|e| {
                        panic!("FAIL [{id}]: enable {module_id} on {inst_name}: {e:?}")
                    });
                }
                "reload" => {
                    // Re-register modules on this instance; its ToggleState must
                    // survive (the instance store is independent of registry
                    // re-registration — A-D-12 re-scoped to instance-scope).
                    register_modules(client);
                }
                other => panic!("FAIL [{id}]: unknown action '{other}'"),
            }
        }

        // Assert each instance's disabled-set via its READ path (instance store),
        // NOT the process-global fallback.
        for (name, expected_ids) in tc["expected_disabled"].as_object().unwrap() {
            let client = instances.get(name).unwrap_or_else(|| {
                panic!("FAIL [{id}]: expected instance '{name}' not constructed")
            });
            let toggle = client.toggle_state();

            let expected_set: HashSet<String> = expected_ids
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect();

            // Every referenced module either is or is not disabled — check both
            // directions so a stray disable on the wrong instance is caught.
            for m in &referenced {
                let is_disabled = toggle.is_disabled(m);
                let should_be_disabled = expected_set.contains(m);
                assert_eq!(
                    is_disabled, should_be_disabled,
                    "FAIL [{id}]: instance '{name}' module '{m}' disabled={is_disabled}, expected {should_be_disabled}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// extension_point_lookup (D-108)
//
// An UNKNOWN extension point is an error; an EMPTY one is not. The two halves
// are asserted together because each is the other's control: answering
// `None`/`&[]`/`false` for a misspelled name turns a typo into a wiring bug
// that first surfaces at `apply()`, while erroring for a registered point that
// holds nothing breaks every host that probes before it registers.
// ---------------------------------------------------------------------------

#[test]
#[allow(clippy::too_many_lines)] // fixture-driven runner: set up, drive, assert per case
fn conformance_extension_point_lookup() {
    use apcore::errors::ModuleError;
    use apcore::extensions::{ExtensionKind, ExtensionManager};
    use apcore::middleware::Middleware;
    use async_trait::async_trait;
    use std::sync::Arc;

    /// A distinct allocation per construction — `unregister` compares by the
    /// extension object's own address, so a stranger must not be a clone.
    #[derive(Debug)]
    struct LookupMiddleware;

    #[async_trait]
    impl Middleware for LookupMiddleware {
        fn name(&self) -> &'static str {
            "lookup"
        }
        async fn before(
            &self,
            _module_id: &str,
            _inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            Ok(None)
        }
        async fn after(
            &self,
            _module_id: &str,
            _inputs: Value,
            _output: Value,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            Ok(None)
        }
        async fn on_error(
            &self,
            _module_id: &str,
            _inputs: Value,
            _error: &ModuleError,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            Ok(None)
        }
    }

    /// Two `ExtensionKind` handles onto ONE extension object: the first is
    /// registered, the second is the identity a host would hand back to
    /// `unregister`. `ExtensionKind` is not `Clone` (the address it wraps is
    /// the identity, so a derived clone would be misleading), and building the
    /// pair from a single `Arc` is how a caller reaches the same object twice.
    fn make_extension_pair(id: &str, point_name: &str) -> (ExtensionKind, ExtensionKind) {
        match point_name {
            "middleware" => {
                let mw: Arc<dyn Middleware> = Arc::new(LookupMiddleware);
                (
                    ExtensionKind::Middleware(Arc::clone(&mw)),
                    ExtensionKind::Middleware(mw),
                )
            }
            "acl" => {
                let acl = Arc::new(ACL::new(vec![], "deny", None));
                (
                    ExtensionKind::Acl(Arc::clone(&acl)),
                    ExtensionKind::Acl(acl),
                )
            }
            other => {
                panic!("FAIL [{id}]: extension_point_lookup has no factory for point `{other}`")
            }
        }
    }

    let fixture = load_fixture("extension_point_lookup");

    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();
        let mut mgr = ExtensionManager::new();
        let mut registered: Vec<ExtensionKind> = Vec::new();
        for point in tc["setup"].as_array().unwrap() {
            let point_name = point.as_str().unwrap();
            let (to_register, same_object) = make_extension_pair(id, point_name);
            mgr.register(point_name, to_register).unwrap_or_else(|e| {
                panic!("FAIL [{id}]: setup register({point_name}) failed: {e}")
            });
            registered.push(same_object);
        }

        let point_name = tc["point_name"].as_str().unwrap();
        let operation = tc["operation"].as_str().unwrap();
        let expected = &tc["expected"];

        // Perform the operation, collapsing the three return shapes into one
        // (error code, textual answer) pair so the assertions below read the
        // fixture's keys rather than the SDK's types.
        let (err_code, answer) = match operation {
            "get" => match mgr.get(point_name) {
                Ok(v) => (None, format!("{:?}", v.is_some())),
                Err(e) => (Some(e.code), String::new()),
            },
            "get_all" => match mgr.get_all(point_name) {
                Ok(v) => (None, v.len().to_string()),
                Err(e) => (Some(e.code), String::new()),
            },
            "unregister" => {
                let stranger;
                let target = if tc["unregister_target"].as_str().unwrap() == "setup" {
                    &registered[0]
                } else {
                    // A stranger of the SAME type, never registered, so the call
                    // is a genuine identity miss rather than a type mismatch.
                    let kind = if point_name == "acl" {
                        "acl"
                    } else {
                        "middleware"
                    };
                    stranger = make_extension_pair(id, kind).0;
                    &stranger
                };
                match mgr.unregister(point_name, target) {
                    Ok(v) => (None, v.to_string()),
                    Err(e) => (Some(e.code), String::new()),
                }
            }
            other => panic!("FAIL [{id}]: unknown operation `{other}`"),
        };

        if let Some(wire) = expected["error_code"].as_str() {
            let want = wire_error_code(id, "extension_point_lookup", wire);
            let got = err_code.unwrap_or_else(|| {
                panic!(
                    "FAIL [{id}]: {operation}(`{point_name}`) must reject an unregistered \
                     extension point with {wire}, but answered `{answer}`"
                )
            });
            assert_eq!(
                got, want,
                "FAIL [{id}]: {operation}(`{point_name}`) raised {got:?}, expected {want:?}"
            );
            continue;
        }

        // No error expected: the call must answer, and answer this.
        assert!(
            err_code.is_none(),
            "FAIL [{id}]: {operation}(`{point_name}`) raised {:?} for a REGISTERED point; \
             an empty point is not an error",
            err_code.unwrap()
        );
        if let Some(value) = expected["value"].as_str() {
            assert_eq!(
                answer,
                (value == "present").to_string(),
                "FAIL [{id}]: get(`{point_name}`) answered `{answer}`, expected {value}"
            );
        } else if let Some(count) = expected["count"].as_u64() {
            assert_eq!(
                answer,
                count.to_string(),
                "FAIL [{id}]: get_all(`{point_name}`) returned {answer} extensions, expected {count}"
            );
        } else if let Some(removed) = expected["removed"].as_bool() {
            assert_eq!(
                answer,
                removed.to_string(),
                "FAIL [{id}]: unregister(`{point_name}`) returned `{answer}`, expected {removed}"
            );
        } else {
            panic!("FAIL [{id}]: expected block names no assertion: {expected}");
        }
    }
}
