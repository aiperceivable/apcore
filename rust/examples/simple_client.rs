//! Simple client example — implement the Module trait and execute it directly.

use apcore::context::{Context, Identity};
use apcore::errors::ModuleError;
use apcore::module::Module;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Module definition
// ---------------------------------------------------------------------------

struct AddModule;

#[async_trait]
impl Module for AddModule {
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "a": { "type": "integer" },
                "b": { "type": "integer" }
            },
            "required": ["a", "b"]
        })
    }

    fn output_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "result": { "type": "integer" }
            }
        })
    }

    fn description(&self) -> &'static str {
        "Add two integers"
    }

    async fn execute(&self, input: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        let a = input["a"].as_i64().unwrap_or(0);
        let b = input["b"].as_i64().unwrap_or(0);
        Ok(json!({ "result": a + b }))
    }
}

// ---------------------------------------------------------------------------
// Another module: greet
// ---------------------------------------------------------------------------

struct GreetModule;

#[async_trait]
impl Module for GreetModule {
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name":     { "type": "string" },
                "greeting": { "type": "string" }
            },
            "required": ["name"]
        })
    }

    fn output_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "message": { "type": "string" }
            }
        })
    }

    fn description(&self) -> &'static str {
        "Greet a user"
    }

    async fn execute(&self, input: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        let name = input["name"].as_str().unwrap_or("World");
        let greeting = input["greeting"].as_str().unwrap_or("Hello");
        Ok(json!({ "message": format!("{}, {}!", greeting, name) }))
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    // Build a caller identity
    let identity = Identity::new(
        "user-1".to_string(),
        "user".to_string(),
        vec!["user".to_string()],
        HashMap::new(),
    );

    // Create an execution context
    let ctx: Context<Value> = Context::new(identity);

    // Instantiate modules
    let add_module = AddModule;
    let greet_module = GreetModule;

    // Execute math.add
    let result = add_module
        .execute(json!({"a": 10, "b": 5}), &ctx)
        .await
        .unwrap();
    println!("math.add result:  {result}"); // {"result":15}

    // Execute greet
    let result = greet_module
        .execute(json!({"name": "Alice", "greeting": "Hi"}), &ctx)
        .await
        .unwrap();
    println!("greet result:     {result}"); // {"message":"Hi, Alice!"}

    // Default greeting
    let result = greet_module
        .execute(json!({"name": "Bob"}), &ctx)
        .await
        .unwrap();
    println!("greet (default):  {result}"); // {"message":"Hello, Bob!"}
}
