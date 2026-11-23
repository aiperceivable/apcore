//! D-104's node fallback is scoped to the document it was declared for.
//!
//! §4.11 step 4a resolves a local `#/…` pointer against the schema FILE root
//! first and, failing that, against the schema NODE being resolved — so a
//! `$defs` block nested inside `input_schema` works as well as a `definitions`
//! block beside it. Both layouts are normative; `deep_chain_v150_spec.rs`
//! pins them.
//!
//! What that fix did not say is how far the fallback travels. It was stored on
//! the `RefResolver` and consulted for **every** local pointer, including ones
//! resolved after following a reference into a different document. So an
//! external schema's `#/$defs/Missing` — a pointer that document does not
//! define — fell back to the *calling* module's `input_schema` `$defs` and
//! resolved to whatever happened to share the name.
//!
//! Three things go wrong with that, in increasing order of cost:
//!
//! 1. An invalid reference reports success. The caller owes it a
//!    `SCHEMA_NOT_FOUND`.
//! 2. The resolved schema validates against the wrong contract, so inputs are
//!    accepted or rejected on a definition the external author never wrote.
//! 3. §10.6 reads `x-sensitive` off the RESOLVED schema. A field the external
//!    document marks sensitive can be silently replaced by a local definition
//!    that does not, and the value is then logged in plaintext — the same
//!    class of leak as dropping `$ref` sibling keys (SCH-001).
//!
//! The fallback is now carried per hop, alongside the root and the file, and is
//! cleared the moment resolution crosses into another document.

use serde_json::{json, Value};

use apcore::config::Config;
use apcore::errors::{ErrorCode, ModuleError};
use apcore::schema::loader::SchemaLoader;

/// The caller: a `$defs/Shared` of its own, and a property that reaches into an
/// external file. `Shared` is the decoy — the external document must never
/// resolve against it.
const CALLER: &str = r##"
module_id: caller
description: "Caller with a local $defs the external document must not reach"
input_schema:
  type: object
  $defs:
    Shared:
      type: object
      properties:
        local_marker: { type: string }
  properties:
    thing:
      $ref: "./ext.schema.yaml#/$defs/Thing"
output_schema:
  type: object
"##;

/// The external document. `Thing` resolves; the `#/$defs/Shared` inside it does
/// NOT exist here — and this document has never heard of the caller's `$defs`.
const EXT_DANGLING: &str = r##"
$defs:
  Thing:
    type: object
    properties:
      inner:
        $ref: "#/$defs/Shared"
"##;

/// The control: the same shape with `Shared` actually defined locally, carrying
/// a marker that tells the two documents apart in the resolved output.
const EXT_RESOLVABLE: &str = r##"
$defs:
  Shared:
    type: object
    properties:
      external_marker: { type: string }
  Thing:
    type: object
    properties:
      inner:
        $ref: "#/$defs/Shared"
"##;

fn load(dir: &std::path::Path, module_id: &str) -> Result<Value, ModuleError> {
    let mut config = Config::default();
    config.set("schema.root", json!(dir.to_string_lossy()));
    let mut loader = SchemaLoader::with_config(&config, Some(dir));
    loader
        .load(module_id)
        .map(|def| serde_json::to_value(def).expect("serialize SchemaDefinition"))
}

/// A dangling pointer inside an external document is an error, not a lookup in
/// the caller's `$defs`.
#[test]
fn an_external_documents_dangling_local_ref_does_not_fall_back_to_the_caller() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("caller.schema.yaml"), CALLER).expect("write caller");
    std::fs::write(dir.path().join("ext.schema.yaml"), EXT_DANGLING).expect("write ext");

    let result = load(dir.path(), "caller");

    match result {
        Err(err) => assert_eq!(
            err.code,
            ErrorCode::SchemaNotFound,
            "a pointer the external document does not define owes the caller \
             SCHEMA_NOT_FOUND, got {err:?}"
        ),
        Ok(resolved) => {
            // The precise failure this pins: not merely "it succeeded", but
            // that it succeeded by binding the external reference to the
            // CALLER's definition.
            let inner = &resolved["input_schema"]["properties"]["thing"]["properties"]["inner"];
            assert!(
                inner["properties"]["local_marker"].is_null(),
                "the external document's #/$defs/Shared resolved against the \
                 CALLER's $defs — the resolved schema now validates, and \
                 redacts, against a contract the external author never \
                 wrote: {resolved}"
            );
            panic!("expected SCHEMA_NOT_FOUND, got a resolved schema: {resolved}");
        }
    }
}

/// The control, which is what keeps the fix from being "disable the fallback".
///
/// A local pointer the external document DOES define must still resolve — in
/// that document — and the marker proves which of the two `Shared` definitions
/// it bound to.
#[test]
fn an_external_documents_own_local_ref_still_resolves_in_that_document() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("caller.schema.yaml"), CALLER).expect("write caller");
    std::fs::write(dir.path().join("ext.schema.yaml"), EXT_RESOLVABLE).expect("write ext");

    let resolved =
        load(dir.path(), "caller").expect("the external pointer resolves in its own document");

    let inner = &resolved["input_schema"]["properties"]["thing"]["properties"]["inner"];
    assert!(
        !inner["properties"]["external_marker"].is_null(),
        "the external document's own $defs/Shared must be the one inlined: {resolved}"
    );
    assert!(
        inner["properties"]["local_marker"].is_null(),
        "and the caller's same-named definition must not be: {resolved}"
    );
}
