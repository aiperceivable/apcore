//! D-77 (spec v1.49.0) — `Registry::describe` returns a String; a structured
//! override falls through to the generated envelope.
//!
//! apcore-rust is the decision's AUTHORITY — "Rust, and the type system" — and
//! had no test for it. The other two SDKs were changed to match this behaviour
//! and both pinned it; the one they were aligned to did not.
//!
//! The three SDKs diverged three ways on the same module: apcore-python
//! returned `str(dict)` (a Python repr, not a description), apcore-typescript
//! returned the raw object through a method it types as `string`, and this SDK
//! returned the override only when it was itself a string. §12.2 was corrected
//! to `-> String`, and the rule is: no stringifying a mapping, and no returning
//! a mapping through a string-typed interface.
//!
//! What makes these RED: treating any `describe()` return as the override — the
//! shape apcore-typescript shipped — or serialising a non-string one, the shape
//! apcore-python shipped.

use apcore::context::Context;
use apcore::errors::ModuleError;
use apcore::module::Module;
use apcore::registry::registry::Registry;
use async_trait::async_trait;
use serde_json::{json, Value};

/// Overrides `describe()` with a STRING — the author's own text.
#[derive(Debug)]
struct StringDescribe;

#[async_trait]
impl Module for StringDescribe {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "the descriptor description"
    }
    fn describe(&self) -> Value {
        Value::String("AUTHORED PROSE".to_string())
    }
    async fn execute(&self, _i: Value, _c: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
}

/// Overrides `describe()` with a MAPPING — the declared shape of the optional
/// introspection hook, and NOT a description.
#[derive(Debug)]
struct StructuredDescribe;

#[async_trait]
impl Module for StructuredDescribe {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "the descriptor description"
    }
    fn describe(&self) -> Value {
        json!({"module_id": "probe.structured", "secret_marker": "MAPPING_LEAKED"})
    }
    async fn execute(&self, _i: Value, _c: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
}

/// No override at all: the default trait method.
#[derive(Debug)]
struct DefaultDescribe;

#[async_trait]
impl Module for DefaultDescribe {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "the descriptor description"
    }
    async fn execute(&self, _i: Value, _c: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
}

#[test]
fn a_string_override_is_returned_verbatim() {
    let reg = Registry::new();
    reg.register_module("probe.string", Box::new(StringDescribe))
        .expect("register");

    assert_eq!(
        reg.describe("probe.string").expect("describe"),
        "AUTHORED PROSE"
    );
}

#[test]
fn a_structured_override_falls_through_to_the_generated_envelope() {
    // The two failures D-77 forbids, asserted separately: the mapping must not
    // be returned (it is not a description), and it must not be SERIALISED
    // into the description either — `str(dict)` is a language-specific repr.
    let reg = Registry::new();
    reg.register_module("probe.structured", Box::new(StructuredDescribe))
        .expect("register");

    let text = reg.describe("probe.structured").expect("describe");

    assert!(
        !text.contains("MAPPING_LEAKED"),
        "a structured describe() must not reach the caller through a \
         string-typed interface, serialised or otherwise: {text}"
    );
    assert!(
        text.contains("probe.structured") && text.contains("the descriptor description"),
        "the generated envelope is what a non-string override falls through to: {text}"
    );
}

#[test]
fn control_no_override_yields_the_same_generated_envelope() {
    // Without this, "the structured override fell through" would also hold for
    // an implementation that returned an error, an empty string, or a constant
    // — none of which is the envelope the decision requires.
    let reg = Registry::new();
    reg.register_module("probe.default", Box::new(DefaultDescribe))
        .expect("register");
    reg.register_module("probe.structured2", Box::new(StructuredDescribe))
        .expect("register");

    let from_default = reg.describe("probe.default").expect("describe");
    let from_structured = reg.describe("probe.structured2").expect("describe");

    assert!(from_default.contains("the descriptor description"));
    // Same envelope shape, differing only in the module id each carries.
    assert_eq!(
        from_default.replace("probe.default", "<id>"),
        from_structured.replace("probe.structured2", "<id>"),
        "a structured override must land on exactly the no-override envelope"
    );
}

#[test]
fn describe_returns_the_module_not_found_code_for_an_unknown_id() {
    // §12.2's other half: never a `"Module not found"` sentinel STRING, which
    // a caller would print as a description.
    let reg = Registry::new();
    let err = reg.describe("probe.absent").expect_err("unknown id");
    assert_eq!(err.code, apcore::errors::ErrorCode::ModuleNotFound);
}
