//! Spec-traced contract tests for the apcore Schema System (Rust SDK).
//!
//! Source spec: apcore/docs/features/schema-system.md
//! Canonical suite mirrored: apcore-python/tests/test_schema_system_spec.py
//!
//! Each test carries the verbatim clause id (format
//! `schema_system.<method>.<kind>.<detail>`) in a leading `// clause:` comment
//! so cross-language diffs line up row-for-row with the Python and TypeScript
//! suites. The Python suite is the CANONICAL clause source.
//!
//! API mapping note:
//!   Python phrases these as module-level functions in `apcore.schema.hardening`
//!   (`validate_schema_dict`, `content_hash`) and `RefResolver`. The idiomatic
//!   Rust surface is:
//!     - `apcore::schema::SchemaValidator::validate(value, schema) -> ValidationResult`
//!       and `validate_detailed(...) -> DetailedValidationResult` (carries
//!       `error_code: Option<ErrorCode>`). Neither raises — failures are reported
//!       via the result object (D10-012), matching the Python `validate` contract.
//!     - `apcore::schema::RefResolver::new()` / `with_max_depth(usize)` plus
//!       `resolve(&Value) -> Result<Value, ModuleError>`. Rust resolves the whole
//!       document in one traversal (no separate `resolve_ref`/`schemas_dir`).
//!     - `apcore::schema::content_hash(&Value) -> String`.
//!
//! These tests are READ-ONLY contract verification — they never modify src/.

use apcore::errors::{ErrorCode, ModuleError};
use apcore::schema::{content_hash, RefResolver, SchemaValidator};
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Shared fixture schemas (canonical conformance shapes from the spec)
// ---------------------------------------------------------------------------

fn constraint_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "count": { "type": "integer", "minimum": 1, "maximum": 100 },
            "label": { "type": "string", "minLength": 1, "maxLength": 50, "pattern": "^[a-z_]+$" }
        },
        "required": ["count", "label"]
    })
}

fn oneof_schema() -> Value {
    json!({
        "oneOf": [
            { "type": "object", "properties": { "kind": { "const": "a" } }, "required": ["kind"] },
            { "type": "object", "properties": { "kind": { "const": "b" } }, "required": ["kind"] }
        ]
    })
}

fn anyof_schema() -> Value {
    json!({
        "anyOf": [
            { "type": "object", "properties": { "kind": { "const": "a" } }, "required": ["kind"] },
            { "type": "object", "properties": { "kind": { "const": "b" } }, "required": ["kind"] }
        ]
    })
}

// A oneOf schema where a single input matches BOTH branches (ambiguous).
fn oneof_ambiguous_schema() -> Value {
    json!({
        "oneOf": [
            { "type": "object" },
            { "type": "object", "properties": { "kind": { "type": "string" } } }
        ]
    })
}

fn tree_node_schema() -> Value {
    json!({
        "$id": "TreeNode",
        "type": "object",
        "properties": {
            "value": { "type": "string" },
            "children": { "type": "array", "items": { "$ref": "TreeNode" } }
        },
        "required": ["value"]
    })
}

/// Extract the SCREAMING_SNAKE wire code string a `ModuleError` serializes to.
fn wire_code(err: &ModuleError) -> String {
    err.to_dict()["code"]
        .as_str()
        .expect("code serializes as a string")
        .to_string()
}

/// Register the recursive `TreeNode` schema so a self-`$ref` resolves, then
/// validate `data` against it. The Rust validator (`jsonschema` crate) natively
/// supports recursive `$ref` via `$id`, so no manual model-rebuild is needed.
fn validate_value(data: &Value, schema: &Value) -> apcore::module::ValidationResult {
    SchemaValidator::new().validate(data, schema)
}

// ===========================================================================
// Contract: Schema.validate  ->  SchemaValidator::validate(value, schema)
// ===========================================================================

// clause: schema_system.validate.input.data_and_schema
#[test]
fn schema_system_validate_input_data_and_schema() {
    // The validator MUST accept both a `value` and a `schema` and return a
    // result object (never raising).
    let validator = SchemaValidator::new();
    let result = validator.validate(
        &json!({ "count": 50, "label": "hello_world" }),
        &constraint_schema(),
    );
    assert!(result.valid, "well-formed input must validate true");
}

// clause: schema_system.validate.error.no_raise
#[test]
fn schema_system_validate_error_no_raise() {
    // Validation failure is reported via the returned result object, NOT via a
    // panic/Err. An input violating `minimum` MUST surface as valid == false.
    let result = validate_value(
        &json!({ "count": 0, "label": "hello" }),
        &constraint_schema(),
    );
    assert!(!result.valid);
    assert!(
        !result.errors.is_empty(),
        "failure must carry >= 1 error detail"
    );
}

// clause: schema_system.validate.return.success_shape
#[test]
fn schema_system_validate_return_success_shape() {
    // On success: valid == true and an empty errors array.
    let result = validate_value(
        &json!({ "count": 50, "label": "hello_world" }),
        &constraint_schema(),
    );
    assert!(result.valid);
    assert!(result.errors.is_empty());
}

// clause: schema_system.validate.return.failure_shape
#[test]
fn schema_system_validate_return_failure_shape() {
    // On failure: valid == false with a non-empty errors array carrying
    // per-failure detail objects exposing a path + message.
    let result = validate_value(
        &json!({ "count": 200, "label": "INVALID LABEL!" }),
        &constraint_schema(),
    );
    assert!(!result.valid);
    assert!(!result.errors.is_empty());
    let detail = &result.errors[0];
    // `path` and `message` fields exist on every detail object.
    let _: &String = &detail.path;
    assert!(!detail.message.is_empty(), "each detail exposes a message");
}

// clause: schema_system.validate.property.async_false
#[test]
fn schema_system_validate_property_async_false() {
    // Property: async == false. `validate` is an ordinary synchronous call —
    // it returns a value directly (no `.await`).
    let result = SchemaValidator::new()
        .validate(&json!({ "count": 1, "label": "ok" }), &constraint_schema());
    assert!(result.valid);
}

// clause: schema_system.validate.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn schema_system_validate_property_thread_safe() {
    // Property: thread_safe == true. >= 8 concurrent validations (mix of valid
    // and invalid) MUST each return the expected verdict with no cross-talk.
    let mut cases: Vec<(Value, bool)> = (0..8)
        .map(|i| (json!({ "count": i + 1, "label": "ok" }), true))
        .collect();
    cases.extend((0..4).map(|_| (json!({ "count": 0, "label": "ok" }), false)));

    let mut handles = Vec::new();
    for (data, expected) in cases {
        handles.push(tokio::spawn(async move {
            let result = SchemaValidator::new().validate(&data, &constraint_schema());
            result.valid == expected
        }));
    }

    let mut outcomes = Vec::new();
    for h in handles {
        outcomes.push(h.await.expect("validation task must not panic"));
    }
    assert!(outcomes.len() >= 8);
    assert!(outcomes.into_iter().all(|ok| ok), "all verdicts consistent");
}

// clause: schema_system.validate.property.pure_idempotent
#[test]
fn schema_system_validate_property_pure_idempotent() {
    // Property: pure + idempotent. Same data + same schema yield equal results
    // across repeated calls, and the inputs are not mutated.
    let data = json!({ "count": 50, "label": "hello_world" });
    let schema = constraint_schema();
    let data_snapshot = data.clone();
    let schema_snapshot = schema.clone();

    let validator = SchemaValidator::new();
    let first = validator.validate(&data, &schema);
    let second = validator.validate(&data, &schema);
    assert_eq!(first.valid, second.valid);
    let first_paths: Vec<&String> = first.errors.iter().map(|e| &e.path).collect();
    let second_paths: Vec<&String> = second.errors.iter().map(|e| &e.path).collect();
    assert_eq!(first_paths, second_paths);
    // Inputs unchanged (no side effects).
    assert_eq!(data, data_snapshot);
    assert_eq!(schema, schema_snapshot);
}

// ===========================================================================
// Contract: RefResolver -- $ref resolution
//   Rust surface: RefResolver::new() / with_max_depth(usize)
//                 .resolve(&Value) -> Result<Value, ModuleError>
// ===========================================================================

// clause: schema_system.resolve_ref.input.construction
#[test]
fn schema_system_resolve_ref_input_construction() {
    // Construction: a default resolver caps recursion at max_depth == 32; an
    // explicit cap is accepted via `with_max_depth`.
    let resolver = RefResolver::new();
    assert_eq!(resolver.max_depth(), 32, "default max_depth is 32");
    let custom = RefResolver::with_max_depth(8);
    assert_eq!(custom.max_depth(), 8);
}

// clause: schema_system.resolve_ref.input.resolve_ref_params
#[test]
fn schema_system_resolve_ref_input_resolve_ref_params() {
    // Rust resolves the whole document in one `resolve(&schema)` traversal
    // (no separate `resolve_ref(ref_string, current_file)` surface). A local
    // `#/definitions/...` $ref MUST inline into the parent document.
    let resolver = RefResolver::new();
    let schema = json!({
        "type": "object",
        "properties": { "addr": { "$ref": "#/definitions/Address" } },
        "definitions": { "Address": { "type": "string" } }
    });
    let out = resolver.resolve(&schema).expect("resolve succeeds");
    assert_eq!(out["properties"]["addr"], json!({ "type": "string" }));
}

// clause: schema_system.resolve.return.inline_resolved
#[test]
fn schema_system_resolve_return_inline_resolved() {
    // On success: the requested local ($ref) is inlined into the parent doc.
    let resolver = RefResolver::new();
    let schema = json!({
        "type": "object",
        "properties": { "addr": { "$ref": "#/definitions/Address" } },
        "definitions": { "Address": { "type": "string", "minLength": 2 } }
    });
    let out = resolver.resolve(&schema).expect("resolve succeeds");
    assert_eq!(out["properties"]["addr"]["type"], json!("string"));
    assert_eq!(out["properties"]["addr"]["minLength"], json!(2));
}

// clause: schema_system.resolve.side_effect.input_not_mutated
#[test]
fn schema_system_resolve_side_effect_input_not_mutated() {
    // The input document is never mutated (a resolved copy is returned).
    let resolver = RefResolver::new();
    let schema = json!({
        "type": "object",
        "properties": { "addr": { "$ref": "#/definitions/Address" } },
        "definitions": { "Address": { "type": "string" } }
    });
    let snapshot = schema.clone();
    let out = resolver.resolve(&schema).expect("resolve succeeds");
    // Original $ref still present and unchanged.
    assert_eq!(
        schema["properties"]["addr"],
        json!({ "$ref": "#/definitions/Address" })
    );
    assert_eq!(schema, snapshot);
    // Returned document differs from the input (the ref was inlined).
    assert_ne!(out, schema);
}

// clause: schema_system.resolve.error.circular_ref
#[test]
fn schema_system_resolve_error_circular_ref() {
    // SchemaCircularRefError(code=SCHEMA_CIRCULAR_REF) on a $ref cycle.
    let resolver = RefResolver::new();
    let schema = json!({
        "$ref": "#/definitions/A",
        "definitions": {
            "A": { "$ref": "#/definitions/B" },
            "B": { "$ref": "#/definitions/A" }
        }
    });
    let err = resolver.resolve(&schema).expect_err("cycle must error");
    assert_eq!(err.code, ErrorCode::SchemaCircularRef);
    assert_eq!(wire_code(&err), "SCHEMA_CIRCULAR_REF");
}

// clause: schema_system.resolve.error.ref_not_found
#[test]
fn schema_system_resolve_error_ref_not_found() {
    // A referenced schema that cannot be resolved MUST error with
    // `ErrorCode::SchemaNotFound` (wire `SCHEMA_NOT_FOUND`) — the symbol named
    // by the schema-system Contract ### Errors block, emitted identically
    // across all three SDKs.
    let resolver = RefResolver::new();
    let schema = json!({ "$ref": "#/definitions/DoesNotExist", "definitions": {} });
    let err = resolver
        .resolve(&schema)
        .expect_err("missing ref must error");
    assert_eq!(err.code, ErrorCode::SchemaNotFound);
    assert_eq!(wire_code(&err), "SCHEMA_NOT_FOUND");
}

// clause: schema_system.resolve_ref.property.async_false
#[test]
fn schema_system_resolve_ref_property_async_false() {
    // Property: async == false. `resolve` returns a value directly (no .await).
    let resolver = RefResolver::new();
    let out = resolver
        .resolve(&json!({ "type": "object" }))
        .expect("resolve succeeds");
    assert_eq!(out, json!({ "type": "object" }));
}

// clause: schema_system.resolve.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn schema_system_resolve_property_thread_safe() {
    // Property: thread_safe == true. >= 8 concurrent resolutions over independent
    // resolver instances MUST each return the correctly inlined document.
    let mut handles = Vec::new();
    for i in 0..8 {
        handles.push(tokio::spawn(async move {
            let resolver = RefResolver::new();
            let schema = json!({
                "type": "object",
                "properties": { "v": { "$ref": "#/definitions/V" } },
                "definitions": { "V": { "type": "integer", "const": i } }
            });
            let out = resolver.resolve(&schema).expect("resolve succeeds");
            out["properties"]["v"]["const"].as_i64().unwrap() == i
        }));
    }
    let mut results = Vec::new();
    for h in handles {
        results.push(h.await.expect("resolve task must not panic"));
    }
    assert!(results.len() >= 8);
    assert!(
        results.into_iter().all(|ok| ok),
        "each inlined const matches its index"
    );
}

// clause: schema_system.resolve.property.idempotent
#[test]
fn schema_system_resolve_property_idempotent() {
    // Property: idempotent == true. Re-resolving the same input yields an equal
    // resolved document.
    let resolver = RefResolver::new();
    let schema = json!({
        "type": "object",
        "properties": { "addr": { "$ref": "#/definitions/Address" } },
        "definitions": { "Address": { "type": "string" } }
    });
    let first = resolver.resolve(&schema).expect("resolve succeeds");
    let second = resolver.resolve(&schema).expect("resolve succeeds");
    assert_eq!(first, second);
}

// clause: schema_system.resolve.error.max_depth_exceeded
#[test]
fn schema_system_resolve_error_max_depth_exceeded() {
    // max_depth caps recursion; exceeding it MUST emit
    // SchemaMaxDepthExceededError(code=SCHEMA_MAX_DEPTH_EXCEEDED).
    let resolver = RefResolver::with_max_depth(3);
    let schema = json!({
        "$ref": "#/definitions/A",
        "definitions": {
            "A": { "$ref": "#/definitions/B" },
            "B": { "$ref": "#/definitions/C" },
            "C": { "$ref": "#/definitions/D" },
            "D": { "$ref": "#/definitions/E" },
            "E": { "type": "string" }
        }
    });
    let err = resolver.resolve(&schema).expect_err("depth cap must error");
    assert_eq!(err.code, ErrorCode::SchemaMaxDepthExceeded);
    assert_eq!(wire_code(&err), "SCHEMA_MAX_DEPTH_EXCEEDED");
}

// ===========================================================================
// Contract: Schema.validate_union  ->  validate over anyOf/oneOf
// ===========================================================================

// clause: schema_system.validate_union.input.anyof
#[test]
fn schema_system_validate_union_input_anyof() {
    // anyOf: an input matching at least one branch MUST be accepted.
    assert!(validate_value(&json!({ "kind": "a" }), &anyof_schema()).valid);
    assert!(validate_value(&json!({ "kind": "b" }), &anyof_schema()).valid);
}

// clause: schema_system.validate_union.input.oneof
#[test]
fn schema_system_validate_union_input_oneof() {
    // oneOf: an input matching exactly one branch MUST be accepted.
    assert!(validate_value(&json!({ "kind": "a" }), &oneof_schema()).valid);
}

// clause: schema_system.validate_union.error.no_match
#[test]
fn schema_system_validate_union_error_no_match() {
    // SchemaValidationError(code=SCHEMA_UNION_NO_MATCH) -- no branch matched.
    // Reported via the result object's error_code (D10-012 amendment).
    let one = SchemaValidator::new().validate_detailed(&json!({ "kind": "c" }), &oneof_schema());
    assert!(!one.valid);
    assert_eq!(one.error_code, Some(ErrorCode::SchemaUnionNoMatch));

    let any = SchemaValidator::new().validate_detailed(&json!({ "kind": "c" }), &anyof_schema());
    assert!(!any.valid);
    assert_eq!(any.error_code, Some(ErrorCode::SchemaUnionNoMatch));
}

// clause: schema_system.validate_union.error.ambiguous
#[test]
fn schema_system_validate_union_error_ambiguous() {
    // SchemaValidationError(code=SCHEMA_UNION_AMBIGUOUS) -- more than one oneOf
    // branch matched. Reported via the result object's error_code.
    let result = SchemaValidator::new()
        .validate_detailed(&json!({ "kind": "x" }), &oneof_ambiguous_schema());
    assert!(!result.valid);
    assert_eq!(result.error_code, Some(ErrorCode::SchemaUnionAmbiguous));
}

// clause: schema_system.validate_union.property.all_branches
#[test]
fn schema_system_validate_union_property_all_branches() {
    // Implementations MUST evaluate ALL branches (no short-circuit). A
    // second-branch-only match MUST be accepted for both anyOf and oneOf.
    assert!(validate_value(&json!({ "kind": "b" }), &oneof_schema()).valid);
    assert!(validate_value(&json!({ "kind": "b" }), &anyof_schema()).valid);
}

// clause: schema_system.validate_union.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn schema_system_validate_union_property_thread_safe() {
    // Property: thread_safe == true. >= 8 concurrent union validations MUST each
    // return the expected verdict.
    let cases: Vec<(Value, bool)> = vec![
        (json!({ "kind": "a" }), true),
        (json!({ "kind": "b" }), true),
        (json!({ "kind": "c" }), false),
        (json!({ "kind": "a" }), true),
        (json!({ "kind": "b" }), true),
        (json!({ "kind": "c" }), false),
        (json!({ "kind": "a" }), true),
        (json!({ "kind": "b" }), true),
    ];

    let mut handles = Vec::new();
    for (data, expected) in cases {
        handles.push(tokio::spawn(async move {
            SchemaValidator::new()
                .validate(&data, &oneof_schema())
                .valid
                == expected
        }));
    }
    let mut outcomes = Vec::new();
    for h in handles {
        outcomes.push(h.await.expect("union task must not panic"));
    }
    assert!(outcomes.len() >= 8);
    assert!(outcomes.into_iter().all(|ok| ok));
}

// clause: schema_system.validate_union.property.pure_idempotent
#[test]
fn schema_system_validate_union_property_pure_idempotent() {
    // Property: pure + idempotent for union validation — same verdict and error
    // code across repeated calls.
    let validator = SchemaValidator::new();
    let first = validator.validate_detailed(&json!({ "kind": "x" }), &oneof_ambiguous_schema());
    let second = validator.validate_detailed(&json!({ "kind": "x" }), &oneof_ambiguous_schema());
    assert_eq!(first.valid, second.valid);
    assert_eq!(first.error_code, second.error_code);
}

// ===========================================================================
// Contract: Schema.validate_recursive  ->  validate over $id self-$ref
// ===========================================================================

// clause: schema_system.validate_recursive.input.nested_data
#[test]
fn schema_system_validate_recursive_input_nested_data() {
    // A valid nested structure (up to depth 5) MUST validate true against the
    // self-referencing TreeNode schema.
    let schema = tree_node_schema();
    assert!(validate_value(&json!({ "value": "root" }), &schema).valid);
    assert!(
        validate_value(
            &json!({ "value": "r", "children": [{ "value": "c" }] }),
            &schema
        )
        .valid
    );
    let deep = json!({
        "value": "a",
        "children": [
            { "value": "b", "children": [
                { "value": "c", "children": [
                    { "value": "d", "children": [{ "value": "e" }] }
                ] }
            ] }
        ]
    });
    assert!(validate_value(&deep, &schema).valid);
}

// clause: schema_system.validate_recursive.error.validation_error
#[test]
fn schema_system_validate_recursive_error_validation_error() {
    // SchemaValidationError(code=SCHEMA_VALIDATION_ERROR) -- data does not
    // conform at the top level (missing required `value`).
    let result =
        SchemaValidator::new().validate_detailed(&json!({ "children": [] }), &tree_node_schema());
    assert!(!result.valid);
    assert_eq!(result.error_code, Some(ErrorCode::SchemaValidationError));
}

// clause: schema_system.validate_recursive.error.nested_validation_error
#[test]
fn schema_system_validate_recursive_error_nested_validation_error() {
    // Validation MUST reach nested levels: a child node missing `value` MUST be
    // rejected.
    let result = SchemaValidator::new().validate_detailed(
        &json!({ "value": "root", "children": [{ "children": [] }] }),
        &tree_node_schema(),
    );
    assert!(!result.valid);
    assert_eq!(result.error_code, Some(ErrorCode::SchemaValidationError));
}

// clause: schema_system.validate_recursive.property.idempotent
#[test]
fn schema_system_validate_recursive_property_idempotent() {
    // Property: pure + idempotent for recursive validation; input not mutated.
    let data = json!({ "value": "root", "children": [{ "value": "c" }] });
    let snapshot = data.clone();
    let validator = SchemaValidator::new();
    let first = validator.validate(&data, &tree_node_schema());
    let second = validator.validate(&data, &tree_node_schema());
    assert_eq!(first.valid, second.valid);
    assert!(first.valid);
    assert_eq!(data, snapshot);
}

// clause: schema_system.validate_recursive.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn schema_system_validate_recursive_property_thread_safe() {
    // Property: thread_safe == true. >= 8 concurrent recursive validations MUST
    // each return the expected verdict.
    let valid_payload = json!({ "value": "root", "children": [{ "value": "c" }] });
    let invalid_payload = json!({ "children": [] });
    let cases: Vec<(Value, bool)> = (0..8)
        .map(|i| {
            if i % 2 == 0 {
                (valid_payload.clone(), true)
            } else {
                (invalid_payload.clone(), false)
            }
        })
        .collect();

    let mut handles = Vec::new();
    for (data, expected) in cases {
        handles.push(tokio::spawn(async move {
            SchemaValidator::new()
                .validate(&data, &tree_node_schema())
                .valid
                == expected
        }));
    }
    let mut outcomes = Vec::new();
    for h in handles {
        outcomes.push(h.await.expect("recursive task must not panic"));
    }
    assert!(outcomes.len() >= 8);
    assert!(outcomes.into_iter().all(|ok| ok));
}

// ===========================================================================
// Contract: Schema.content_hash  ->  content_hash(&Value)
// ===========================================================================

// clause: schema_system.content_hash.input.schema_dict
#[test]
fn schema_system_content_hash_input_schema_dict() {
    // Inputs: a single resolved JSON Schema value; returns a String digest.
    let digest = content_hash(&json!({ "type": "object" }));
    assert!(!digest.is_empty());
    assert_eq!(digest.len(), 64);
}

// clause: schema_system.content_hash.return.hex_digest
#[test]
fn schema_system_content_hash_return_hex_digest() {
    // On success: lowercase hexadecimal SHA-256 digest (64 characters).
    let digest = content_hash(&constraint_schema());
    assert_eq!(digest.len(), 64);
    assert_eq!(digest, digest.to_lowercase());
    assert!(digest
        .chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
}

// clause: schema_system.content_hash.error.no_raise
#[test]
fn schema_system_content_hash_error_no_raise() {
    // This operation MUST NOT panic for a serializable schema value.
    let digest = content_hash(&json!({ "a": 1, "b": [1, 2, { "c": "d" }] }));
    assert_eq!(digest.len(), 64);
}

// clause: schema_system.content_hash.property.canonical_dedup
#[test]
fn schema_system_content_hash_property_canonical_dedup() {
    // Two schemas identical after canonical (sorted-key) JSON serialization MUST
    // hash identically; key ordering MUST NOT affect the digest. Distinct
    // content -> distinct hash.
    let a = json!({ "b": 1, "a": 2, "z": { "y": 1, "x": 2 } });
    let b = json!({ "a": 2, "z": { "x": 2, "y": 1 }, "b": 1 });
    assert_eq!(content_hash(&a), content_hash(&b));
    assert_ne!(
        content_hash(&json!({ "a": 1 })),
        content_hash(&json!({ "a": 2 }))
    );
}

// clause: schema_system.content_hash.property.idempotent
#[test]
fn schema_system_content_hash_property_idempotent() {
    // Property: idempotent == true. Repeated calls produce the same digest, and
    // the input is not mutated.
    let schema = json!({ "type": "object", "properties": { "x": { "type": "string" } } });
    let snapshot = schema.clone();
    let first = content_hash(&schema);
    let second = content_hash(&schema);
    assert_eq!(first, second);
    assert_eq!(schema, snapshot);
}

// clause: schema_system.content_hash.property.async_false
#[test]
fn schema_system_content_hash_property_async_false() {
    // Property: async == false. `content_hash` returns a String directly.
    let digest = content_hash(&json!({ "type": "object" }));
    assert_eq!(digest.len(), 64);
}

// clause: schema_system.content_hash.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn schema_system_content_hash_property_thread_safe() {
    // Property: thread_safe == true. >= 8 concurrent hashes of the same schema
    // MUST all agree.
    let expected = content_hash(&constraint_schema());
    let mut handles = Vec::new();
    for _ in 0..8 {
        handles.push(tokio::spawn(
            async move { content_hash(&constraint_schema()) },
        ));
    }
    let mut digests = Vec::new();
    for h in handles {
        digests.push(h.await.expect("hash task must not panic"));
    }
    assert!(digests.len() >= 8);
    assert!(digests.into_iter().all(|d| d == expected));
}
