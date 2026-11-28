#![allow(clippy::result_large_err)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use apcore::{
    APCore, ApprovalHandler, ApprovalRequest, ApprovalResult, Config, Context, ErrorCode, Executor,
    ModuleError, ModulePack, PolicyEffect, PolicyEvaluationRequest, PolicyEvaluator, PolicyVerdict,
    Registry, ACL,
};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const SCHEMA_DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";
static STARTS: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct TypedInput {
    name: String,
    #[serde(default = "default_limit")]
    limit: u8,
    selection: Selection,
}

fn default_limit() -> u8 {
    3
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Selection {
    position: (u32, String),
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
struct TypedOutput {
    label: String,
    limit: u8,
}

/// Return a typed label
/// and its defaulted limit.
///
/// The execution context supplies the current module ID.
#[apcore::module(id = "demo.typed", requires_approval = true)]
async fn declared(
    input: self::TypedInput,
    context: &Context<Value>,
) -> Result<self::TypedOutput, ModuleError> {
    STARTS.fetch_add(1, Ordering::Relaxed);
    let current = context.call_chain.last().unwrap();
    Ok(TypedOutput {
        label: format!("{}:{current}", input.name),
        limit: input.limit,
    })
}

mod sibling {
    #[apcore::module(id = "demo.echo", description = "Echo a typed label", readonly = true)]
    pub(super) async fn echo(
        input: super::TypedOutput,
    ) -> Result<super::TypedOutput, apcore::ModuleError> {
        Ok(input)
    }
}

const PACK: ModulePack = ModulePack::new(&[declared::DECLARATION, sibling::echo::DECLARATION]);

fn raw_input() -> Value {
    json!({"name": "label", "selection": {"position": [2, "here"]}})
}

#[derive(Debug, Default)]
struct Gate {
    policy: Mutex<Vec<Value>>,
    approvals: Mutex<Vec<ApprovalRequest>>,
}

#[async_trait]
impl PolicyEvaluator for Gate {
    async fn evaluate(
        &self,
        request: &PolicyEvaluationRequest,
    ) -> Result<PolicyVerdict, ModuleError> {
        self.policy.lock().unwrap().push(request.arguments.clone());
        Ok(PolicyVerdict {
            decision: if request.arguments["name"] == "denied" {
                PolicyEffect::Deny
            } else {
                PolicyEffect::Allow
            },
            rule_id: None,
        })
    }
}

#[async_trait]
impl ApprovalHandler for Gate {
    async fn request_approval(
        &self,
        request: &ApprovalRequest,
    ) -> Result<ApprovalResult, ModuleError> {
        self.approvals.lock().unwrap().push(request.clone());
        Ok(ApprovalResult::approved("reviewer"))
    }

    async fn check_approval(&self, _: &str) -> Result<ApprovalResult, ModuleError> {
        panic!("an immediate grant must not query a token");
    }
}

fn client_with_gate(gate: Arc<Gate>, hosted: bool) -> APCore {
    let config = Config::in_memory();
    let mut executor = Executor::with_options(Registry::new(), config.clone(), None, None, None);
    if hosted {
        executor.set_policy_evaluator(Some(gate.clone())).unwrap();
    }
    executor.set_approval_handler_shared(gate);
    let client = APCore::with_options(None, Some(executor), Some(config), None);
    PACK.register(&client).unwrap();
    client
}

#[tokio::test]
async fn registration_preserves_descriptor_and_builtin_call_guards() {
    let client = APCore::new();
    let typed = || serde_json::from_value::<TypedInput>(raw_input()).unwrap();
    assert_eq!(declared::HANDLE.module_id(), "demo.typed");
    assert_eq!(declared::DECLARATION.module_id(), "demo.typed");
    let err = declared::HANDLE
        .invoke(&client, typed(), None)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::ModuleNotFound);

    PACK.register(&client).unwrap();
    let descriptor = client
        .registry()
        .get_definition("demo.typed")
        .unwrap()
        .unwrap();
    assert_eq!(
        descriptor.description,
        "Return a typed label and its defaulted limit."
    );
    assert!(descriptor
        .documentation
        .unwrap()
        .contains("execution context"));
    assert!(descriptor.annotations.unwrap().requires_approval);
    assert_eq!(descriptor.input_schema["$schema"], SCHEMA_DIALECT);
    assert_eq!(descriptor.output_schema["$schema"], SCHEMA_DIALECT);
    assert!(jsonschema::draft202012::meta::is_valid(
        &descriptor.input_schema
    ));
    assert!(jsonschema::draft202012::meta::is_valid(
        &descriptor.output_schema
    ));
    assert_eq!(
        descriptor.input_schema["properties"]["selection"]["$ref"],
        "#/$defs/Selection"
    );
    let tuple = &descriptor.input_schema["$defs"]["Selection"]["properties"]["position"];
    assert_eq!(tuple["prefixItems"][0]["type"], "integer");
    assert_eq!(tuple["prefixItems"][1]["type"], "string");
    assert_eq!(client.list_modules(None, None), ["demo.echo", "demo.typed"]);

    let err = declared::HANDLE
        .invoke(&client, typed(), None)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::ApprovalDenied);
    let config = Config::in_memory();
    let executor = Executor::with_options(
        Registry::new(),
        config.clone(),
        None,
        Some(ACL::try_new(vec![], "deny", None).unwrap()),
        None,
    );
    let denied = APCore::with_options(None, Some(executor), Some(config), None);
    declared::DECLARATION.register(&denied).unwrap();
    let err = declared::HANDLE
        .invoke(&denied, typed(), None)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::ACLDenied);
}

fn malformed_input() -> Value {
    let mut invalid = raw_input();
    invalid["selection"]["position"][1] = json!(2);
    invalid
}

#[tokio::test]
async fn legacy_approval_still_runs_before_input_validation() {
    let legacy = Arc::new(Gate::default());
    let client = client_with_gate(legacy.clone(), false);
    let invalid = malformed_input();
    let err = client
        .call("demo.typed", invalid.clone(), None, None)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::SchemaValidationError);
    assert_eq!(legacy.approvals.lock().unwrap()[0].arguments, invalid);
}

#[tokio::test]
async fn normalizes_typed_input_before_hosted_policy_and_final_approval() {
    let input = raw_input();
    let typed = || serde_json::from_value::<TypedInput>(input.clone()).unwrap();
    let gate = Arc::new(Gate::default());
    let client = client_with_gate(gate.clone(), true);
    let err = client
        .call("demo.typed", malformed_input(), None, None)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::SchemaValidationError);
    assert!(gate.policy.lock().unwrap().is_empty());
    assert!(gate.approvals.lock().unwrap().is_empty());

    let mut rejected = typed();
    rejected.name = "denied".to_owned();
    let err = declared::HANDLE
        .invoke(&client, rejected, None)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::PolicyDenied);
    assert!(gate.approvals.lock().unwrap().is_empty());
    assert_eq!(STARTS.load(Ordering::Relaxed), 0);

    assert_eq!(
        client
            .call("demo.typed", input.clone(), None, None)
            .await
            .unwrap(),
        json!({"label": "label:demo.typed", "limit": 3})
    );
    let output = declared::HANDLE
        .invoke(&client, typed(), None)
        .await
        .unwrap();
    assert_eq!(output.limit, 3);
    assert_eq!(output.label, "label:demo.typed");
    assert_eq!(STARTS.load(Ordering::Relaxed), 2);
    {
        let observations = gate.policy.lock().unwrap();
        let approvals = gate.approvals.lock().unwrap();
        let mut expected = input;
        expected["limit"] = json!(3);
        assert_eq!(observations.len(), 3);
        assert_eq!(approvals.len(), 2);
        assert_eq!(observations[1], expected);
        assert_eq!(observations[2], expected);
        assert_eq!(approvals[0].arguments, expected);
        assert_eq!(approvals[1].arguments, expected);
        assert!(approvals[0].canonical_args_digest.is_some());
        assert_eq!(
            approvals[0].canonical_args_digest,
            approvals[1].canonical_args_digest
        );
    }
    let echoed = sibling::echo::HANDLE
        .invoke(&client, output, None)
        .await
        .unwrap();
    assert_eq!(echoed.label, "label:demo.typed");
}

#[test]
fn unsupported_annotation_has_a_compile_time_diagnostic() {
    trybuild::TestCases::new().compile_fail("tests/ui/typed_module_invalid_annotation.rs");
}
