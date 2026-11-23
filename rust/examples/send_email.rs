//! SendEmail module — destructive action with sensitive fields and examples.

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
struct SendEmailInput {
    to: String,
    subject: String,
    body: String,
    /// Sensitive field — should be redacted in logs.
    api_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct SendEmailOutput {
    status: String,
    message_id: String,
}

// ---------------------------------------------------------------------------
// Module implementation
// ---------------------------------------------------------------------------

struct SendEmailModule;

#[async_trait]
impl Module for SendEmailModule {
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "to":      { "type": "string", "description": "Recipient email address" },
                "subject": { "type": "string" },
                "body":    { "type": "string" },
                "api_key": { "type": "string", "x-sensitive": true }
            },
            "required": ["to", "subject", "body", "api_key"]
        })
    }

    fn output_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "status":     { "type": "string", "enum": ["sent", "failed"] },
                "message_id": { "type": "string" }
            },
            "required": ["status", "message_id"]
        })
    }

    fn description(&self) -> &'static str {
        "Send an email message via external API (destructive)"
    }

    async fn execute(&self, input: Value, ctx: &Context<Value>) -> Result<Value, ModuleError> {
        let req: SendEmailInput = serde_json::from_value(input).map_err(|e| {
            ModuleError::new(
                apcore::errors::ErrorCode::GeneralInvalidInput,
                e.to_string(),
            )
        })?;

        // Simulate sending (real impl would call SMTP/API)
        println!(
            "[{}] Sending email to '{}' | subject: '{}'",
            ctx.trace_id, req.to, req.subject
        );

        let message_id = format!("msg-{:05}", req.to.len() * 1000 % 100_000);

        let output = SendEmailOutput {
            status: "sent".to_string(),
            message_id,
        };

        Ok(serde_json::to_value(output).unwrap())
    }
}

// ---------------------------------------------------------------------------
// Helper — build annotations for this module
// ---------------------------------------------------------------------------

fn send_email_annotations() -> ModuleAnnotations {
    ModuleAnnotations {
        destructive: true,
        requires_approval: true,
        open_world: true,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    let identity = Identity::new(
        "admin-1".to_string(),
        "user".to_string(),
        vec!["admin".to_string()],
        HashMap::new(),
    );
    let ctx: Context<Value> = Context::new(identity);
    let module = SendEmailModule;

    let out = module
        .execute(
            json!({
                "to":      "alice@example.com",
                "subject": "Hello from apcore",
                "body":    "This is a test email.",
                "api_key": "sk-secret"
            }),
            &ctx,
        )
        .await
        .unwrap();
    println!("Result: {out}");

    // Show annotations
    let annotations = send_email_annotations();
    println!(
        "\nAnnotations:\n{}",
        serde_json::to_string_pretty(&annotations).unwrap()
    );

    // Schema — note x-sensitive on api_key
    println!(
        "\nInput schema:\n{}",
        serde_json::to_string_pretty(&module.input_schema()).unwrap()
    );
}
