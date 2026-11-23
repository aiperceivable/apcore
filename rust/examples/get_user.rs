//! GetUser module — readonly + idempotent behavioral annotations.

use apcore::context::{Context, Identity};
use apcore::errors::ModuleError;
use apcore::module::{Module, ModuleAnnotations};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Typed schemas
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
struct GetUserInput {
    user_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct GetUserOutput {
    id: String,
    name: String,
    email: String,
}

// ---------------------------------------------------------------------------
// Module implementation
// ---------------------------------------------------------------------------

struct GetUserModule;

impl GetUserModule {
    fn users() -> HashMap<&'static str, GetUserOutput> {
        let mut m = HashMap::new();
        m.insert(
            "user-1",
            GetUserOutput {
                id: "user-1".into(),
                name: "Alice".into(),
                email: "alice@example.com".into(),
            },
        );
        m.insert(
            "user-2",
            GetUserOutput {
                id: "user-2".into(),
                name: "Bob".into(),
                email: "bob@example.com".into(),
            },
        );
        m
    }
}

#[async_trait]
impl Module for GetUserModule {
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "user_id": { "type": "string", "description": "The user's unique identifier" }
            },
            "required": ["user_id"]
        })
    }

    fn output_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id":    { "type": "string" },
                "name":  { "type": "string" },
                "email": { "type": "string" }
            },
            "required": ["id", "name", "email"]
        })
    }

    fn description(&self) -> &'static str {
        "Look up a user by ID (readonly, idempotent)"
    }

    async fn execute(&self, input: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        let req: GetUserInput = serde_json::from_value(input).map_err(|e| {
            ModuleError::new(
                apcore::errors::ErrorCode::GeneralInvalidInput,
                e.to_string(),
            )
        })?;

        let users = Self::users();
        let output = match users.get(req.user_id.as_str()) {
            Some(u) => GetUserOutput {
                id: u.id.clone(),
                name: u.name.clone(),
                email: u.email.clone(),
            },
            None => GetUserOutput {
                id: req.user_id.clone(),
                name: "Unknown".into(),
                email: "unknown@example.com".into(),
            },
        };

        Ok(serde_json::to_value(output).unwrap())
    }
}

// ---------------------------------------------------------------------------
// Helper — build annotations for this module
// ---------------------------------------------------------------------------

fn get_user_annotations() -> ModuleAnnotations {
    ModuleAnnotations {
        readonly: true,
        idempotent: true,
        cacheable: true,
        cache_ttl: 60,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    let identity = Identity::new(
        "service-account".to_string(),
        "service".to_string(),
        vec!["reader".to_string()],
        HashMap::new(),
    );
    let ctx: Context<Value> = Context::new(identity);
    let module = GetUserModule;

    for user_id in ["user-1", "user-2", "user-999"] {
        let out = module
            .execute(json!({"user_id": user_id}), &ctx)
            .await
            .unwrap();
        println!("{user_id}: {out}");
    }

    // Show annotations
    let annotations = get_user_annotations();
    println!(
        "\nAnnotations: {}",
        serde_json::to_string_pretty(&annotations).unwrap()
    );
}
