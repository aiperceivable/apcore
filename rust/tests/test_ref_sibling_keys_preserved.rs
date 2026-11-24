//! Regression test: keys sitting alongside a `$ref` MUST survive resolution.
//!
//! JSON Schema 2019-09+ makes `$ref` siblings independent assertions, and
//! PROTOCOL_SPEC §4.11 step 1b already requires them preserved on the
//! self-reference path. `RefResolver::resolve_inner` read only `map["$ref"]`
//! and returned the resolved target, so every other key of the node — its
//! `description`, its `default`, its `x-sensitive` marker — was dropped.
//! apcore-python (`ref_resolver.py`, `result.update(sibling_keys)`) and
//! apcore-typescript (`ref-resolver.ts`, `Object.assign`) both overlay the
//! siblings onto the resolved target.
//!
//! This is a data leak, not a cosmetic gap: PROTOCOL_SPEC §10.6 redaction is
//! driven by `x-sensitive` in the RESOLVED input schema, so a field marked
//! sensitive beside a `$ref` was redacted by both peer SDKs and logged in
//! plaintext by this one.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};

use apcore::config::Config;
use apcore::context::Context;
use apcore::errors::ModuleError;
use apcore::executor::{Executor, REDACTED_VALUE};
use apcore::module::Module;
use apcore::registry::registry::Registry;
use apcore::schema::ref_resolver::RefResolver;

/// The shape the defect was found on: a `$ref` carrying an `x-sensitive`
/// marker and a `description` of its own.
fn schema_with_ref_siblings() -> Value {
    json!({
        "type": "object",
        "properties": {
            "owner": {
                "$ref": "#/$defs/User",
                "x-sensitive": true,
                "description": "The account owner (sensitive)",
            },
            "note": {"type": "string"},
        },
        "$defs": {
            "User": {
                "type": "object",
                "description": "A user record",
                "properties": {
                    "id": {"type": "string"},
                    "email": {"type": "string"},
                },
            }
        }
    })
}

#[test]
fn ref_sibling_keys_survive_resolution() {
    let resolved = RefResolver::new()
        .resolve(&schema_with_ref_siblings())
        .expect("resolve");

    let owner = resolved
        .pointer("/properties/owner")
        .expect("resolved schema must still describe `owner`");

    // The target was inlined.
    assert_eq!(
        owner.pointer("/properties/email"),
        Some(&json!({"type": "string"})),
        "the `$ref` target must be inlined"
    );

    // The siblings survived — this is the whole point.
    assert_eq!(
        owner.get("x-sensitive"),
        Some(&Value::Bool(true)),
        "`x-sensitive` sitting beside a `$ref` MUST survive resolution — \
         §10.6 redaction reads it off the RESOLVED schema"
    );

    // A sibling OVERRIDES the target's value for the same key, matching
    // apcore-python's `result.update(sibling_keys)` and apcore-typescript's
    // `Object.assign`: the target declares `description: "A user record"`, the
    // referring node overrides it.
    assert_eq!(
        owner.get("description"),
        Some(&json!("The account owner (sensitive)")),
        "a sibling key MUST override the resolved target's value for that key"
    );
}

/// A sibling may itself contain a `$ref`; it must be resolved like any other
/// part of the schema rather than carried through verbatim.
#[test]
fn ref_sibling_values_are_themselves_resolved() {
    let schema = json!({
        "type": "object",
        "properties": {
            "owner": {
                "$ref": "#/$defs/User",
                "additionalProperties": {"$ref": "#/$defs/Extra"},
            },
        },
        "$defs": {
            "User": {"type": "object", "properties": {"id": {"type": "string"}}},
            "Extra": {"type": "string", "maxLength": 8},
        }
    });

    let resolved = RefResolver::new().resolve(&schema).expect("resolve");
    assert_eq!(
        resolved.pointer("/properties/owner/additionalProperties"),
        Some(&json!({"type": "string", "maxLength": 8})),
        "a `$ref` inside a sibling key MUST be resolved too"
    );
}

/// A module whose declared input schema is already resolved — the shape a
/// `schema_ref` binding produces — and which records the redacted inputs the
/// pipeline computed for it.
#[derive(Debug)]
struct CapturingModule {
    schema: Value,
    seen_redacted: Arc<Mutex<Option<Value>>>,
}

#[async_trait]
impl Module for CapturingModule {
    fn input_schema(&self) -> Value {
        self.schema.clone()
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "Records the redacted inputs the executor computed"
    }
    async fn execute(&self, inputs: Value, ctx: &Context<Value>) -> Result<Value, ModuleError> {
        *self.seen_redacted.lock().expect("lock poisoned") = ctx
            .redacted_inputs
            .as_ref()
            .map(|m| Value::Object(m.clone().into_iter().collect()));
        Ok(inputs)
    }
}

/// End-to-end: the executor's capture point (`BuiltinInputValidation` →
/// `redact_sensitive_with`) must mask a field whose `x-sensitive` marker sat
/// beside a `$ref` in the authored schema.
#[tokio::test]
async fn x_sensitive_beside_a_ref_is_redacted_at_the_capture_point() {
    let resolved_schema = RefResolver::new()
        .resolve(&schema_with_ref_siblings())
        .expect("resolve");

    let seen_redacted = Arc::new(Mutex::new(None));
    let registry = Arc::new(Registry::new());
    registry
        .register_module(
            "executor.accounts.read_account",
            Box::new(CapturingModule {
                schema: resolved_schema,
                seen_redacted: Arc::clone(&seen_redacted),
            }),
        )
        .expect("register module");

    let executor = Executor::new(Arc::clone(&registry), Arc::new(Config::default()));
    let inputs = json!({
        "owner": {"id": "u-1", "email": "alice@example.com"},
        "note": "hello",
    });
    executor
        .call("executor.accounts.read_account", inputs, None, None)
        .await
        .expect("call succeeds");

    let redacted = seen_redacted
        .lock()
        .expect("lock poisoned")
        .clone()
        .expect("the pipeline must have computed redacted_inputs");

    assert_eq!(
        redacted.get("owner"),
        Some(&json!(REDACTED_VALUE)),
        "a field marked `x-sensitive` beside a `$ref` MUST be redacted at the \
         capture point; dropping the sibling put the raw value in logs and in \
         the captured inputs. Actual: {redacted}"
    );
    assert_eq!(
        redacted.get("note"),
        Some(&json!("hello")),
        "non-sensitive fields must be untouched"
    );
}
