//! Direct coverage for `FunctionModule` — the runtime-side support type for
//! function-based module registration (the Rust equivalent of Python's
//! `@module` decorator). Exercises both constructors and verifies that
//! metadata added in the v0.18.0 refactor (documentation, tags, version,
//! metadata, examples) round-trips correctly.

use apcore::context::Context;
use apcore::decorator::FunctionModule;
use apcore::errors::ModuleError;
use apcore::module::{Module, ModuleAnnotations, ModuleExample};
use serde_json::{json, Value};
use std::collections::HashMap;

type EchoFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, ModuleError>> + Send + 'a>>;

#[allow(clippy::type_complexity)] // Handler must match FunctionModule's async trait shape.
fn echo_handler(
) -> impl for<'a> Fn(Value, &'a Context<Value>) -> EchoFuture<'a> + Send + Sync + 'static {
    |inputs: Value, _ctx: &Context<Value>| Box::pin(async move { Ok(json!({ "echoed": inputs })) })
}

#[tokio::test]
async fn test_function_module_minimal_with_description() {
    let fm = FunctionModule::with_description(
        ModuleAnnotations::default(),
        json!({}),
        json!({}),
        "",
        None,
        vec![],
        "0.1.0",
        HashMap::new(),
        vec![],
        echo_handler(),
    );

    assert_eq!(fm.description(), "");
    assert_eq!(fm.version, "0.1.0");
    assert!(fm.tags.is_empty());
    assert!(fm.documentation.is_none());
    assert!(fm.metadata.is_empty());
    assert!(fm.examples.is_empty());
}

#[tokio::test]
async fn test_function_module_with_description_preserves_metadata() {
    let mut metadata = HashMap::new();
    metadata.insert("owner".to_string(), json!("platform-team"));

    let mut ex = ModuleExample::default();
    ex.title = "basic".to_string();
    ex.inputs = json!({ "x": 1 });
    ex.output = json!({ "echoed": { "x": 1 } });
    let examples = vec![ex];

    let fm = FunctionModule::with_description(
        ModuleAnnotations::default(),
        json!({"type": "object"}),
        json!({"type": "object"}),
        "Echoes inputs back to the caller",
        Some("Extended docs about the echo module".to_string()),
        vec!["demo".to_string(), "utility".to_string()],
        "1.2.3",
        metadata,
        examples,
        echo_handler(),
    );

    assert_eq!(fm.description(), "Echoes inputs back to the caller");
    assert_eq!(
        fm.documentation.as_deref(),
        Some("Extended docs about the echo module")
    );
    assert_eq!(fm.tags, vec!["demo".to_string(), "utility".to_string()]);
    assert_eq!(fm.version, "1.2.3");
    assert_eq!(fm.metadata.get("owner"), Some(&json!("platform-team")));
    assert_eq!(fm.examples.len(), 1);
    assert_eq!(fm.examples[0].title, "basic");
}

#[tokio::test]
async fn test_function_module_execute_runs_handler() {
    let fm = FunctionModule::with_description(
        ModuleAnnotations::default(),
        json!({}),
        json!({}),
        "",
        None,
        vec![],
        "0.1.0",
        HashMap::new(),
        vec![],
        echo_handler(),
    );

    let ctx: Context<Value> = Context::anonymous();
    let result = fm
        .execute(json!({ "hello": "world" }), &ctx)
        .await
        .expect("handler should succeed");

    assert_eq!(result["echoed"]["hello"], "world");
}

#[tokio::test]
async fn test_function_module_schemas_round_trip() {
    let input_schema = json!({
        "type": "object",
        "properties": { "name": { "type": "string" } },
        "required": ["name"]
    });
    let output_schema = json!({ "type": "object" });

    let fm = FunctionModule::with_description(
        ModuleAnnotations::default(),
        input_schema.clone(),
        output_schema.clone(),
        "",
        None,
        vec![],
        "0.1.0",
        HashMap::new(),
        vec![],
        echo_handler(),
    );

    assert_eq!(fm.input_schema(), input_schema);
    assert_eq!(fm.output_schema(), output_schema);
}
