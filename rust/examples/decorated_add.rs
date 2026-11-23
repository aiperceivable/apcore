//! Decorated function example: add two numbers.
//!
//! Demonstrates creating a simple module using `FunctionModule` with typed schemas.

use apcore::{Context, FunctionModule, Identity, Module, ModuleAnnotations};
use serde_json::{json, Value};
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let module = FunctionModule::with_description(
        ModuleAnnotations::default(),
        json!({
            "type": "object",
            "properties": {
                "a": { "type": "integer" },
                "b": { "type": "integer" }
            },
            "required": ["a", "b"]
        }),
        json!({
            "type": "object",
            "properties": {
                "result": { "type": "integer" }
            },
            "required": ["result"]
        }),
        "Add two integers",
        None,
        vec!["math".to_string(), "utility".to_string()],
        "1.0.0",
        HashMap::default(),
        vec![],
        |inputs: Value, _ctx: &Context<Value>| {
            Box::pin(async move {
                let a = inputs["a"].as_i64().unwrap_or(0);
                let b = inputs["b"].as_i64().unwrap_or(0);
                Ok(json!({ "result": a + b }))
            })
        },
    );

    let ctx = Context::new(Identity::new(
        "user:1".into(),
        "user".into(),
        vec![],
        HashMap::new(),
    ));

    let result = module.execute(json!({"a": 10, "b": 5}), &ctx).await?;
    println!("Result: {result}"); // {"result": 15}

    Ok(())
}
