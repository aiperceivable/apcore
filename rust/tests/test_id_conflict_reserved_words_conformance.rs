//! Cross-language driver for `id_conflict_reserved_words.json`.
//!
//! PROTOCOL_SPEC §2.6 step 2, narrowed to the **first segment** in spec
//! v1.26.0 (#99). A reserved word claims a namespace, not a token, so
//! `foo.system.bar` and `executor.schema.validate` are legal and
//! `system.custom_module` is not.
//!
//! The driver exercises the **public** `register()` path deliberately.
//! `register_internal()` bypasses the reserved-word check by design, so running
//! the cases through it would report agreement while testing nothing.
//!
//! The fixture lands in the spec repo one push after this driver, so that
//! `check_driver_coverage.py --strict` has a driver to find for it. Until then
//! the test skips and names the unexercised fixture — "not verified", never
//! "passed".

use apcore::registry::registry::RESERVED_WORDS;
use apcore::registry::ModuleDescriptor;
use apcore::{Context, ModuleError, Registry};
use apcore::{Module, ModuleAnnotations};
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::conformance_env::find_fixtures_root;

const FIXTURE: &str = "id_conflict_reserved_words.json";

#[derive(Debug)]
struct FixtureModule;

#[async_trait]
impl Module for FixtureModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "conformance fixture module"
    }
    async fn execute(&self, _i: Value, _c: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
}

fn descriptor(id: &str) -> ModuleDescriptor {
    ModuleDescriptor {
        module_id: id.to_string(),
        name: None,
        description: "conformance fixture module".to_string(),
        documentation: None,
        input_schema: json!({"type": "object"}),
        output_schema: json!({"type": "object"}),
        version: "1.0.0".to_string(),
        tags: vec![],
        annotations: Some(ModuleAnnotations::default()),
        examples: vec![],
        metadata: std::collections::HashMap::new(),
        display: None,
        sunset_date: None,
        dependencies: vec![],
        enabled: true,
    }
}

#[test]
fn id_conflict_reserved_words_conformance() {
    let path = find_fixtures_root().join(FIXTURE);
    if !path.is_file() {
        eprintln!("SKIP: {FIXTURE} not in the spec repo yet (spec v1.26.0, #99) — not verified");
        return;
    }
    let fixture: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read fixture"))
            .expect("parse fixture");

    // The canonical set lives in the fixture, not in this SDK. Reading it from
    // `apcore` would let a divergent local list agree with itself: every case
    // would be computed from the same wrong set and pass.
    let declared: std::collections::BTreeSet<String> = fixture["reserved_words"]
        .as_array()
        .expect("reserved_words")
        .iter()
        .map(|v| v.as_str().expect("string").to_string())
        .collect();
    let ours: std::collections::BTreeSet<String> =
        RESERVED_WORDS.iter().map(|s| (*s).to_string()).collect();
    assert_eq!(
        declared, ours,
        "reserved-word set diverges from the fixture"
    );

    let cases = fixture["test_cases"].as_array().expect("test_cases");
    for tc in cases {
        let id = tc["id"].as_str().expect("case id");
        let new_id = tc["new_id"].as_str().expect("new_id");
        let note = tc["note"].as_str().unwrap_or("");
        let registry = Registry::new();

        if let Some(existing) = tc.get("existing_ids").and_then(|v| v.as_array()) {
            for e in existing {
                let e = e.as_str().expect("existing id");
                registry
                    .register(e, Box::new(FixtureModule), descriptor(e))
                    .unwrap_or_else(|err| panic!("[{id}] setup register {e} failed: {err:?}"));
            }
        }

        let result = registry.register(new_id, Box::new(FixtureModule), descriptor(new_id));
        match tc["expected"].as_str() {
            // Must register cleanly. An error here is the pre-v1.26.0
            // per-segment reading resurfacing.
            None => assert!(
                result.is_ok(),
                "[{id}] expected `{new_id}` to register; got {:?}\n  {note}",
                result.err()
            ),
            // The fixture names the conflict `type`; SDKs surface it through
            // their own error types, so assert the registration was refused and
            // that the message identifies the offending id, rather than pinning
            // a type name the three languages do not share.
            Some(conflict) => {
                let err = result.expect_err(&format!("[{id}] expected refusal\n  {note}"));
                let msg = format!("{err:?}");
                let first = new_id.split('.').next().unwrap_or(new_id);
                assert!(
                    msg.contains(new_id) || msg.contains(first),
                    "[{id}] refusal did not name `{new_id}`: {msg}\n  {note}"
                );

                // ...and the WIRE CODE says WHICH conflict it was. Without this
                // the driver only asserts "refused", so `check_case_pinning.py`
                // could mutate `expected` from `reserved_word` to
                // `duplicate_id` — a value this very fixture uses — and every
                // SDK stayed green: the distinction the fixture exists to draw
                // was asserted by nobody. The mapping lives in the fixture, not
                // here, because it is one rule and §8's codes are the
                // cross-language contract.
                let want = fixture["error_code_by_conflict"][conflict]
                    .as_str()
                    .unwrap_or_else(|| panic!("[{id}] fixture declares no code for {conflict:?}"));
                let got = serde_json::to_value(err.code)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned));
                assert_eq!(
                    got.as_deref(),
                    Some(want),
                    "[{id}] the {conflict:?} conflict must surface as {want}\n  {note}"
                );
            }
        }
    }
    println!(
        "id_conflict_reserved_words: {} case(s) executed",
        cases.len()
    );
}
