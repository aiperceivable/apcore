use std::sync::{Arc, Mutex};

use apcore::{
    APCore, ApprovalHandler, ApprovalRequest, ApprovalResult, Config, Executor, ModuleError,
    ModulePack, PolicyEffect, PolicyEvaluationRequest, PolicyEvaluator, PolicyVerdict, Registry,
};
use apcore_mcp::{async_serve, AsyncServeConfig};
use async_trait::async_trait;
use axum::body::{to_bytes, Body};
use axum::http::Request;
use axum::Router;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tower::ServiceExt;

#[derive(Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
struct Note {
    note_id: String,
    #[serde(default)]
    archived: bool,
}

/// Look up a note.
#[apcore::module(id = "notes.lookup", readonly = true)]
async fn lookup(note: Note) -> Result<Note, ModuleError> {
    Ok(note)
}

const NOTES: ModulePack = ModulePack::new(&[lookup::DECLARATION]);

#[derive(Debug, Default)]
struct Review(Mutex<Vec<Value>>);

#[async_trait]
impl PolicyEvaluator for Review {
    async fn evaluate(
        &self,
        request: &PolicyEvaluationRequest,
    ) -> Result<PolicyVerdict, ModuleError> {
        let decision = if request.arguments["note_id"] == "private" {
            PolicyEffect::Deny
        } else {
            PolicyEffect::RequireApproval
        };
        Ok(PolicyVerdict {
            decision,
            rule_id: None,
        })
    }
}

#[async_trait]
impl ApprovalHandler for Review {
    async fn request_approval(
        &self,
        request: &ApprovalRequest,
    ) -> Result<ApprovalResult, ModuleError> {
        self.0.lock().unwrap().push(request.arguments.clone());
        Ok(ApprovalResult::approved("example-reviewer"))
    }

    async fn check_approval(&self, _: &str) -> Result<ApprovalResult, ModuleError> {
        unreachable!("this example returns approval immediately")
    }
}

async fn request(app: &Router, method: &str, params: Value) -> Value {
    let request = Request::post("/mcp")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "jsonrpc": "2.0", "id": 1, "method": method, "params": params
            })
            .to_string(),
        ))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert!(response.status().is_success());
    serde_json::from_slice(&to_bytes(response.into_body(), 65_536).await.unwrap()).unwrap()
}

#[tokio::test]
async fn typed_pack_and_mcp_use_the_same_governed_executor() {
    let config = Config::in_memory();
    let review = Arc::new(Review::default());
    let mut executor = Executor::new(Registry::new(), config.clone());
    executor.set_policy_evaluator(Some(review.clone())).unwrap();
    executor.set_approval_handler_shared(review.clone());
    let core = APCore::with_options(None, Some(executor), Some(config), None);
    NOTES.register(&core).unwrap();

    let available = Note {
        note_id: "public".into(),
        archived: false,
    };
    let expected = serde_json::to_value(&available).unwrap();
    assert_eq!(
        lookup::HANDLE
            .invoke(&core, available, None)
            .await
            .unwrap()
            .note_id,
        "public"
    );
    let private = Note {
        note_id: "private".into(),
        archived: false,
    };
    assert!(lookup::HANDLE.invoke(&core, private, None).await.is_err());

    let app = async_serve(
        core.executor_arc(),
        AsyncServeConfig {
            require_auth: Some(false),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let listed = request(&app, "tools/list", json!({})).await;
    let tools = listed["result"]["tools"].as_array().unwrap();
    let tool = tools
        .iter()
        .find(|tool| tool["name"] == "notes.lookup")
        .unwrap();
    assert_eq!(tool["inputSchema"]["required"], json!(["note_id"]));
    let reply = request(
        &app,
        "tools/call",
        json!({
            "name": "notes.lookup", "arguments": {"note_id": "public"},
        }),
    )
    .await;
    assert_ne!(reply["result"]["isError"], true, "{reply}");
    let text = reply["result"]["content"][0]["text"].as_str().unwrap();
    assert_eq!(serde_json::from_str::<Value>(text).unwrap(), expected);
    for arguments in [json!({"note_id": "private"}), json!({})] {
        let denied = request(
            &app,
            "tools/call",
            json!({
                "name": "notes.lookup", "arguments": arguments,
            }),
        )
        .await;
        assert_eq!(denied["result"]["isError"], true, "{denied}");
    }
    assert_eq!(*review.0.lock().unwrap(), vec![expected.clone(), expected]);
}
