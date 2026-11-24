//! Unit + binding-path regression tests for OpenAI strict-mode compatibility
//! detection (protocol-spec §5.12.5).
//!
//! Before this feature existed, `ErrorCode::BindingStrictSchemaIncompatible`
//! was declared in the error enum but no code path ever produced it —
//! `auto_schema: strict` silently accepted schemas OpenAI structured outputs
//! would reject. The `register_into_with_typed_handlers` cases below fail
//! without the detector wired into `src/bindings.rs`.
//!
//! Cross-SDK feature-list parity lives in
//! `tests/test_openai_strict_compat_conformance.rs`; this file covers the
//! Rust-specific wiring and the error payload.

use std::collections::HashMap;

use serde_json::json;

use apcore::bindings::{typed_handler, BindingLoader};
use apcore::errors::ErrorCode;
use apcore::registry::registry::Registry;
use apcore::{assert_openai_strict_compatible, detect_openai_strict_incompatibilities};

// ---------------------------------------------------------------------------
// Detector unit tests
// ---------------------------------------------------------------------------

#[test]
fn compatible_schema_yields_no_findings() {
    let schema = json!({
        "type": "object",
        "properties": {"a": {"type": "string"}, "n": {"type": "integer", "minimum": 0}},
        "required": ["a", "n"],
        "additionalProperties": false
    });
    assert!(detect_openai_strict_incompatibilities(&schema).is_empty());
}

#[test]
fn nested_anyof_is_not_reported() {
    // OpenAI supports anyOf below the root. This is the nullable wrapper all
    // three SDKs emit; reporting it would sink every optional field.
    let schema = json!({
        "type": "object",
        "properties": {"note": {"anyOf": [{"type": "string"}, {"type": "null"}]}}
    });
    assert!(detect_openai_strict_incompatibilities(&schema).is_empty());
}

#[test]
fn root_anyof_is_reported() {
    let schema = json!({"anyOf": [{"type": "object"}]});
    assert_eq!(
        detect_openai_strict_incompatibilities(&schema),
        vec!["$.anyOf"]
    );
}

#[test]
fn author_written_oneof_is_reported_and_not_rewritten() {
    let schema = json!({
        "type": "object",
        "properties": {"mode": {"oneOf": [{"type": "string"}, {"type": "integer"}]}}
    });
    let snapshot = schema.clone();

    assert_eq!(
        detect_openai_strict_incompatibilities(&schema),
        vec!["$.mode.oneOf"]
    );
    // Rewriting oneOf -> anyOf would tell the LLM "both branches matching is
    // fine" while apcore's validator still raises SCHEMA_UNION_AMBIGUOUS.
    assert_eq!(schema, snapshot);
}

#[test]
fn supported_formats_are_not_reported() {
    for fmt in [
        "date-time",
        "time",
        "date",
        "duration",
        "email",
        "hostname",
        "ipv4",
        "ipv6",
        "uuid",
    ] {
        let schema = json!({
            "type": "object",
            "properties": {"v": {"type": "string", "format": fmt}}
        });
        assert!(
            detect_openai_strict_incompatibilities(&schema).is_empty(),
            "format {fmt} must be accepted"
        );
    }
}

#[test]
fn unsupported_formats_are_reported() {
    let schema = json!({
        "type": "object",
        "properties": {"v": {"type": "string", "format": "uri"}}
    });
    assert_eq!(
        detect_openai_strict_incompatibilities(&schema),
        vec!["$.v.format=uri"]
    );
}

#[test]
fn supported_numeric_and_array_constraints_are_not_reported() {
    let schema = json!({
        "type": "object",
        "properties": {
            "n": {"type": "number", "minimum": 1, "maximum": 9, "multipleOf": 2},
            "l": {"type": "array", "minItems": 1, "maxItems": 3,
                  "items": {"type": "string", "pattern": "^a"}}
        }
    });
    assert!(detect_openai_strict_incompatibilities(&schema).is_empty());
}

#[test]
fn findings_are_sorted_and_deduplicated() {
    let schema = json!({
        "type": "object",
        "properties": {
            "zeta": {"type": "string", "minLength": 1},
            "alpha": {"type": "string", "minLength": 1}
        }
    });
    assert_eq!(
        detect_openai_strict_incompatibilities(&schema),
        vec!["$.alpha.minLength", "$.zeta.minLength"]
    );
}

#[test]
fn non_object_schema_is_tolerated() {
    assert!(detect_openai_strict_incompatibilities(&json!(true)).is_empty());
}

#[test]
fn assert_is_noop_for_compatible_schema() {
    let schema = json!({"type": "object", "properties": {}});
    assert!(assert_openai_strict_compatible(&schema, "m", None, None).is_ok());
}

#[test]
fn assert_reports_side_prefixed_features() {
    let schema = json!({
        "type": "object",
        "properties": {"s": {"type": "string", "minLength": 2}}
    });
    let err = assert_openai_strict_compatible(&schema, "demo.mod", Some("input"), Some("b.yaml"))
        .expect_err("must reject");

    assert_eq!(err.code, ErrorCode::BindingStrictSchemaIncompatible);
    assert_eq!(
        err.details.get("features_listed"),
        Some(&json!(["input:$.s.minLength"]))
    );
    assert!(err.message.contains("b.yaml: "));
    assert!(err
        .message
        .contains("binding 'demo.mod' uses auto_schema: strict"));
    assert!(err.message.contains("input:$.s.minLength"));
    assert!(err.message.contains("See protocol-spec §5.12.5"));
}

// ---------------------------------------------------------------------------
// Binding-path enforcement
// ---------------------------------------------------------------------------

/// `HashSet` derives `uniqueItems: true`, which OpenAI structured outputs
/// rejects.
#[derive(serde::Deserialize, schemars::JsonSchema)]
struct IncompatibleInput {
    #[allow(dead_code)]
    tags: std::collections::HashSet<String>,
}

/// `Option<String>` derives a nested nullable union and `String` a plain
/// string — both accepted by OpenAI.
#[derive(serde::Deserialize, schemars::JsonSchema)]
struct CompatibleInput {
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    note: Option<String>,
}

#[derive(serde::Serialize, schemars::JsonSchema)]
struct SimpleOutput {
    ok: bool,
}

fn write_binding(dir: &std::path::Path, target: &str, auto_schema: &str) -> std::path::PathBuf {
    let path = dir.join("t.binding.yaml");
    std::fs::write(
        &path,
        format!(
            "bindings:\n  - module_id: strict.case\n    target: \"{target}\"\n    auto_schema: {auto_schema}\n"
        ),
    )
    .unwrap();
    path
}

fn load(path: &std::path::Path) -> BindingLoader {
    let mut loader = BindingLoader::new();
    loader.load_from_yaml(path).unwrap();
    loader
}

#[test]
fn strict_rejects_incompatible_handler_schema() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_binding(dir.path(), "mod:run", "strict");
    let loader = load(&path);

    let mut handlers = HashMap::new();
    handlers.insert(
        "mod:run".to_string(),
        typed_handler::<IncompatibleInput, SimpleOutput>(|_| Ok(SimpleOutput { ok: true })),
    );

    let err = loader
        .register_into_with_typed_handlers(&Registry::new(), handlers)
        .expect_err("uniqueItems must be rejected under auto_schema: strict");

    assert_eq!(err.code, ErrorCode::BindingStrictSchemaIncompatible);
    let features = err.details.get("features_listed").unwrap();
    assert!(
        features.to_string().contains("input:$.tags.uniqueItems"),
        "unexpected features: {features}"
    );
}

#[test]
fn strict_accepts_optional_fields() {
    // Regression guard: a nullable union must not sink auto_schema: strict.
    let dir = tempfile::tempdir().unwrap();
    let path = write_binding(dir.path(), "mod:run", "strict");
    let loader = load(&path);

    let mut handlers = HashMap::new();
    handlers.insert(
        "mod:run".to_string(),
        typed_handler::<CompatibleInput, SimpleOutput>(|_| Ok(SimpleOutput { ok: true })),
    );

    assert_eq!(
        loader
            .register_into_with_typed_handlers(&Registry::new(), handlers)
            .unwrap(),
        1
    );
}

#[test]
fn permissive_mode_does_not_enforce() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_binding(dir.path(), "mod:run", "permissive");
    let loader = load(&path);

    let mut handlers = HashMap::new();
    handlers.insert(
        "mod:run".to_string(),
        typed_handler::<IncompatibleInput, SimpleOutput>(|_| Ok(SimpleOutput { ok: true })),
    );

    assert_eq!(
        loader
            .register_into_with_typed_handlers(&Registry::new(), handlers)
            .unwrap(),
        1
    );
}

#[test]
fn implicit_auto_schema_does_not_enforce() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.binding.yaml");
    std::fs::write(
        &path,
        "bindings:\n  - module_id: strict.implicit\n    target: \"mod:run\"\n",
    )
    .unwrap();
    let loader = load(&path);

    let mut handlers = HashMap::new();
    handlers.insert(
        "mod:run".to_string(),
        typed_handler::<IncompatibleInput, SimpleOutput>(|_| Ok(SimpleOutput { ok: true })),
    );

    assert_eq!(
        loader
            .register_into_with_typed_handlers(&Registry::new(), handlers)
            .unwrap(),
        1
    );
}
