//! Tests for RefResolver — JSON $ref resolution, self-reference preservation
//! and circular reference detection (PROTOCOL_SPEC §4.15).

use apcore::schema::RefResolver;
use serde_json::json;

// ---------------------------------------------------------------------------
// Local $ref resolution
// ---------------------------------------------------------------------------

#[test]
fn test_schema_resolver_resolve_local_ref() {
    let resolver = RefResolver::new();
    let schema = json!({
        "$defs": {
            "name": { "type": "string" }
        },
        "properties": {
            "first_name": { "$ref": "#/$defs/name" }
        }
    });
    let result = resolver.resolve(&schema).unwrap();
    assert_eq!(result["properties"]["first_name"]["type"], "string");
}

#[test]
fn test_schema_resolver_resolve_definitions_path() {
    let resolver = RefResolver::new();
    let schema = json!({
        "definitions": {
            "count": { "type": "integer" }
        },
        "properties": {
            "total": { "$ref": "#/definitions/count" }
        }
    });
    let result = resolver.resolve(&schema).unwrap();
    assert_eq!(result["properties"]["total"]["type"], "integer");
}

#[test]
fn test_schema_resolver_resolve_root_ref_is_preserved_lazily() {
    // `$ref: "#"` names the document being resolved: a self-reference, not a
    // cycle. PROTOCOL_SPEC §4.15.2 requires it to survive resolution as a lazy
    // reference so the validator can bind it recursively — inlining it would
    // never terminate, and rejecting it would make every recursive data
    // structure unusable.
    let resolver = RefResolver::new();
    let schema = json!({
        "type": "object",
        "properties": {
            "self_ref": { "$ref": "#" }
        }
    });
    let result = resolver
        .resolve(&schema)
        .expect("self-reference must resolve");
    assert_eq!(result["properties"]["self_ref"], json!({ "$ref": "#" }));
}

// ---------------------------------------------------------------------------
// Registered URI references
// ---------------------------------------------------------------------------

#[test]
fn test_schema_resolver_resolve_registered_uri() {
    let mut resolver = RefResolver::new();
    resolver.register(
        "https://example.com/schemas/address",
        json!({
            "type": "object",
            "properties": {
                "street": { "type": "string" }
            }
        }),
    );

    let schema = json!({
        "properties": {
            "home_address": { "$ref": "https://example.com/schemas/address" }
        }
    });
    let result = resolver.resolve(&schema).unwrap();
    assert_eq!(result["properties"]["home_address"]["type"], "object");
    assert_eq!(
        result["properties"]["home_address"]["properties"]["street"]["type"],
        "string"
    );
}

#[test]
fn test_schema_resolver_resolve_unregistered_uri_error() {
    let resolver = RefResolver::new();
    let schema = json!({
        "properties": {
            "x": { "$ref": "https://missing.example.com/schema" }
        }
    });
    let result = resolver.resolve(&schema);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code, apcore::errors::ErrorCode::SchemaNotFound);
    assert!(err.message.contains("Referenced schema not found"));
}

// ---------------------------------------------------------------------------
// Local $ref not found
// ---------------------------------------------------------------------------

#[test]
fn test_schema_resolver_resolve_local_ref_not_found() {
    let resolver = RefResolver::new();
    let schema = json!({
        "properties": {
            "x": { "$ref": "#/$defs/nonexistent" }
        }
    });
    let result = resolver.resolve(&schema);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code, apcore::errors::ErrorCode::SchemaNotFound);
    assert!(err.message.contains("Local $ref not found"));
}

// ---------------------------------------------------------------------------
// Circular reference detection
// ---------------------------------------------------------------------------

#[test]
fn test_schema_resolver_has_circular_refs_false() {
    let resolver = RefResolver::new();
    let schema = json!({
        "$defs": {
            "name": { "type": "string" }
        },
        "properties": {
            "x": { "$ref": "#/$defs/name" }
        }
    });
    assert!(!resolver.has_circular_refs(&schema));
}

// PROTOCOL_SPEC §4.15: a `$ref` re-entered by *structural descent* (through
// `properties` / `items` / a combinator) is a self-reference, not a cycle —
// `resolve` preserves it lazily and returns Ok (see
// `test_schema_resolver_resolve_self_referencing_def_is_preserved_lazily`
// directly below, which asserts exactly that on this same schema). This test
// previously asserted `true`, contradicting its neighbour: `has_circular_refs`
// ran its own traversal with no `from_ref_chain` discriminator, so it answered
// `true` for every recursive schema `resolve` accepted. The predicate now
// delegates to `resolve`, so both agree.
#[test]
fn test_schema_resolver_has_circular_refs_false_for_structural_self_ref() {
    let resolver = RefResolver::new();
    let schema = json!({
        "$defs": {
            "node": {
                "type": "object",
                "properties": {
                    "child": { "$ref": "#/$defs/node" }
                }
            }
        },
        "properties": {
            "root": { "$ref": "#/$defs/node" }
        }
    });
    assert!(resolver.resolve(&schema).is_ok());
    assert!(!resolver.has_circular_refs(&schema));
}

#[test]
fn test_schema_resolver_resolve_self_referencing_def_is_preserved_lazily() {
    // `#/$defs/node` re-entered through `properties` is a recursive data
    // structure, not a cycle: the first occurrence is inlined and the one inside
    // it stays a `$ref` for the validator to bind (PROTOCOL_SPEC §4.15.2).
    let resolver = RefResolver::new();
    let schema = json!({
        "$defs": {
            "node": {
                "type": "object",
                "properties": {
                    "child": { "$ref": "#/$defs/node" }
                }
            }
        },
        "properties": {
            "root": { "$ref": "#/$defs/node" }
        }
    });
    let result = resolver
        .resolve(&schema)
        .expect("self-reference must resolve");
    assert_eq!(result["properties"]["root"]["type"], "object");
    assert_eq!(
        result["properties"]["root"]["properties"]["child"],
        json!({ "$ref": "#/$defs/node" })
    );
}

#[test]
fn test_schema_resolver_resolve_ref_only_cycle_returns_error() {
    // A `$ref` → `$ref` chain reaches no schema body, so there is nothing to
    // defer to and resolution cannot terminate. That is the *circular* case
    // PROTOCOL_SPEC §4.15.2 still requires SCHEMA_CIRCULAR_REF for.
    let resolver = RefResolver::new();
    let schema = json!({
        "$ref": "#/$defs/a",
        "$defs": {
            "a": { "$ref": "#/$defs/b" },
            "b": { "$ref": "#/$defs/a" }
        }
    });
    let result = resolver.resolve(&schema);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code, apcore::errors::ErrorCode::SchemaCircularRef);
    assert!(err.message.contains("Circular"));
}

// ---------------------------------------------------------------------------
// Array resolution
// ---------------------------------------------------------------------------

#[test]
fn test_schema_resolver_resolve_refs_in_array() {
    let resolver = RefResolver::new();
    let schema = json!({
        "$defs": {
            "tag": { "type": "string" }
        },
        "items": [
            { "$ref": "#/$defs/tag" },
            { "type": "integer" }
        ]
    });
    let result = resolver.resolve(&schema).unwrap();
    let items = result["items"].as_array().unwrap();
    assert_eq!(items[0]["type"], "string");
    assert_eq!(items[1]["type"], "integer");
}

// ---------------------------------------------------------------------------
// Nested $ref chains
// ---------------------------------------------------------------------------

#[test]
fn test_schema_resolver_resolve_chained_refs() {
    let resolver = RefResolver::new();
    let schema = json!({
        "$defs": {
            "base": { "type": "string" },
            "alias": { "$ref": "#/$defs/base" }
        },
        "properties": {
            "x": { "$ref": "#/$defs/alias" }
        }
    });
    let result = resolver.resolve(&schema).unwrap();
    assert_eq!(result["properties"]["x"]["type"], "string");
}

// ---------------------------------------------------------------------------
// No $refs — passthrough
// ---------------------------------------------------------------------------

#[test]
fn test_schema_resolver_resolve_no_refs_returns_same() {
    let resolver = RefResolver::new();
    let schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" }
        }
    });
    let result = resolver.resolve(&schema).unwrap();
    assert_eq!(result, schema);
}

// ---------------------------------------------------------------------------
// Scalar passthrough
// ---------------------------------------------------------------------------

#[test]
fn test_schema_resolver_resolve_scalar_values() {
    let resolver = RefResolver::new();
    assert_eq!(resolver.resolve(&json!("hello")).unwrap(), json!("hello"));
    assert_eq!(resolver.resolve(&json!(42)).unwrap(), json!(42));
    assert_eq!(resolver.resolve(&json!(true)).unwrap(), json!(true));
    assert_eq!(resolver.resolve(&json!(null)).unwrap(), json!(null));
}

// ---------------------------------------------------------------------------
// Default impl
// ---------------------------------------------------------------------------

#[test]
fn test_schema_resolver_default() {
    let resolver = RefResolver::default();
    let schema = json!({ "type": "string" });
    assert_eq!(resolver.resolve(&schema).unwrap(), schema);
}

// ---------------------------------------------------------------------------
// has_circular_refs with registered URI
// ---------------------------------------------------------------------------

#[test]
fn test_schema_resolver_has_circular_refs_with_unresolvable_uri() {
    let resolver = RefResolver::new();
    let schema = json!({
        "properties": {
            "x": { "$ref": "https://missing.com/not-here" }
        }
    });
    // unresolvable URI => lookup fails, so no circular detected
    assert!(!resolver.has_circular_refs(&schema));
}

#[test]
fn test_schema_resolver_has_circular_refs_false_for_scalars() {
    let resolver = RefResolver::new();
    assert!(!resolver.has_circular_refs(&json!(42)));
    assert!(!resolver.has_circular_refs(&json!("hello")));
    assert!(!resolver.has_circular_refs(&json!(null)));
}

// Same §4.15 rule reached through an array: descending into `items` is a
// structural descent, so the re-entered `$ref` is a self-reference, not a
// cycle. A genuine `$ref` → `$ref` chain inside an array still answers `true`
// (second half of this test).
#[test]
fn test_schema_resolver_has_circular_refs_in_array() {
    let resolver = RefResolver::new();
    let self_ref = json!({
        "$defs": {
            "node": {
                "type": "object",
                "properties": {
                    "child": { "$ref": "#/$defs/node" }
                }
            }
        },
        "items": [
            { "$ref": "#/$defs/node" }
        ]
    });
    assert!(resolver.resolve(&self_ref).is_ok());
    assert!(!resolver.has_circular_refs(&self_ref));

    let ref_only_cycle = json!({
        "$ref": "#/$defs/a",
        "$defs": {
            "a": { "$ref": "#/$defs/b" },
            "b": { "$ref": "#/$defs/a" }
        }
    });
    assert!(resolver.has_circular_refs(&ref_only_cycle));
}

// ---------------------------------------------------------------------------
// max_depth — sync SCHEMA-001
// ---------------------------------------------------------------------------

#[test]
fn test_schema_resolver_default_max_depth_is_32() {
    // Cross-language parity: apcore-python and apcore-typescript both default
    // to schema.max_ref_depth = 32.
    let resolver = RefResolver::new();
    assert_eq!(resolver.max_depth(), 32);
}

#[test]
fn test_schema_resolver_rejects_chain_exceeding_max_depth() {
    // Build a non-circular chain of 40 cascading $refs:
    //   #/$defs/level0 -> #/$defs/level1 -> ... -> #/$defs/level39
    // With max_depth=32 this MUST fail with SchemaMaxDepthExceeded (A-D-038):
    // depth-cap exhaustion is distinct from an actual cycle.
    let resolver = RefResolver::with_max_depth(32);
    let mut defs = serde_json::Map::new();
    for i in 0..40usize {
        let body = if i + 1 < 40 {
            json!({ "type": "object", "properties": { "next": { "$ref": format!("#/$defs/level{}", i + 1) } } })
        } else {
            json!({ "type": "string" })
        };
        defs.insert(format!("level{i}"), body);
    }
    let schema = json!({
        "$ref": "#/$defs/level0",
        "$defs": serde_json::Value::Object(defs),
    });
    let err = resolver
        .resolve(&schema)
        .expect_err("40-level $ref chain must exceed max_depth=32");
    assert_eq!(err.code, apcore::errors::ErrorCode::SchemaMaxDepthExceeded);
    assert!(
        err.message.to_lowercase().contains("max_depth")
            || err.message.to_lowercase().contains("max-depth")
            || err.message.to_lowercase().contains("recursion"),
        "error should mention the depth cap; got: {}",
        err.message
    );
}

#[test]
fn test_schema_resolver_with_max_depth_constructor_round_trip() {
    let resolver = RefResolver::with_max_depth(8);
    assert_eq!(resolver.max_depth(), 8);
}

#[test]
fn test_schema_resolver_output_still_validates_recursively() {
    // The lazy `$ref` the resolver leaves behind has to remain *bindable*: the
    // resolved document must validate a deep tree and still reject a type error
    // at depth. A resolver that dropped or widened the reference would make the
    // recursive positions of the contract assert nothing.
    let resolver = RefResolver::new();
    let schema = json!({
        "$id": "TreeNode",
        "type": "object",
        "required": ["value"],
        "properties": {
            "value": { "type": "string" },
            "children": { "type": "array", "items": { "$ref": "#" } }
        }
    });
    let resolved = resolver
        .resolve(&schema)
        .expect("self-reference must resolve");

    let deep = json!({
        "value": "root",
        "children": [{ "value": "child", "children": [{ "value": "grandchild" }] }]
    });
    assert!(apcore::executor::validate_against_schema(&deep, &resolved, "Input").is_ok());

    let bad = json!({ "value": "root", "children": [{ "value": 42 }] });
    assert!(apcore::executor::validate_against_schema(&bad, &resolved, "Input").is_err());
}
