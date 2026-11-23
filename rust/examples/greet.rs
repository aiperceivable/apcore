//! Greet module — typed input/output structs with serde.

use apcore::context::{Context, Identity};
use apcore::errors::ModuleError;
use apcore::module::Module;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Typed schemas
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
struct GreetInput {
    name: String,
    #[serde(default = "default_greeting")]
    greeting: String,
}

fn default_greeting() -> String {
    "Hello".to_string()
}

#[derive(Debug, Serialize, Deserialize)]
struct GreetOutput {
    message: String,
}

// ---------------------------------------------------------------------------
// Module implementation
// ---------------------------------------------------------------------------

struct GreetModule;

#[async_trait]
impl Module for GreetModule {
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name":     { "type": "string", "description": "Name of the person to greet" },
                "greeting": { "type": "string", "description": "Custom greeting prefix", "default": "Hello" }
            },
            "required": ["name"]
        })
    }

    fn output_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "message": { "type": "string" }
            },
            "required": ["message"]
        })
    }

    fn description(&self) -> &'static str {
        "Greet a user by name"
    }

    async fn execute(&self, input: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        let req: GreetInput = serde_json::from_value(input).map_err(|e| {
            ModuleError::new(
                apcore::errors::ErrorCode::GeneralInvalidInput,
                e.to_string(),
            )
        })?;

        let output = GreetOutput {
            message: format!("{}, {}!", req.greeting, req.name),
        };

        Ok(serde_json::to_value(output).unwrap())
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    let identity = Identity::new(
        "agent-1".to_string(),
        "service".to_string(),
        vec!["assistant".to_string()],
        HashMap::new(),
    );
    let ctx: Context<Value> = Context::new(identity);
    let module = GreetModule;

    // With custom greeting
    let out = module
        .execute(json!({"name": "Alice", "greeting": "Good morning"}), &ctx)
        .await
        .unwrap();
    println!("{out}"); // {"message":"Good morning, Alice!"}

    // With default greeting
    let out = module.execute(json!({"name": "Bob"}), &ctx).await.unwrap();
    println!("{out}"); // {"message":"Hello, Bob!"}

    // Schema introspection
    println!(
        "\nInput schema:\n{}",
        serde_json::to_string_pretty(&module.input_schema()).unwrap()
    );

    // Validation error: missing required field
    let err = module
        .execute(json!({"greeting": "Hi"}), &ctx)
        .await
        .unwrap_err();
    println!("\nExpected error: {err}");
}
