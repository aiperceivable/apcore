//! Global client example — module-level APCore instance using OnceLock.
//!
//! Demonstrates the Rust equivalent of Python's `import apcore; apcore.call(...)` pattern.

use std::sync::OnceLock;

use apcore::{APCore, Context, Identity, Module};
use serde_json::{json, Value};

static CLIENT: OnceLock<APCore> = OnceLock::new();

/// Return the global APCore instance, initializing on first call.
fn client() -> &'static APCore {
    CLIENT.get_or_init(APCore::new)
}

// ---------------------------------------------------------------------------
// A simple add module registered on the global client
// ---------------------------------------------------------------------------

struct AddModule;

#[async_trait::async_trait]
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
            },
            "required": ["result"]
        })
    }

    fn description(&self) -> &'static str {
        "Add two integers"
    }

    async fn execute(
        &self,
        inputs: Value,
        _ctx: &Context<Value>,
    ) -> Result<Value, apcore::errors::ModuleError> {
        let a = inputs["a"].as_i64().unwrap_or(0);
        let b = inputs["b"].as_i64().unwrap_or(0);
        Ok(json!({ "result": a + b }))
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Register a module on the global client via APCore::register.
    // APCore::register takes &self (not &mut self), so this is safe to call
    // on the shared static reference.
    client().register("math.add", Box::new(AddModule))?;

    // Build a caller context for the call.
    let ctx = Context::new(Identity::new(
        "user:1".into(),
        "user".into(),
        vec![],
        std::collections::HashMap::new(),
    ));

    // Call the module through the global client.
    let result = client()
        .call("math.add", json!({"a": 10, "b": 5}), Some(&ctx), None)
        .await?;
    println!("Global call result: {result}"); // {"result":15}

    // Calling again via the client() helper to confirm the same instance.
    let result2 = client()
        .call("math.add", json!({"a": 100, "b": 200}), None, None)
        .await?;
    println!("Second call result: {result2}"); // {"result":300}

    // List all registered modules.
    let modules = client().list_modules(None, None);
    println!("Registered modules: {modules:?}");

    Ok(())
}
