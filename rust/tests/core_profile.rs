//! Public in-memory assembly exercised independently of optional extensions.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use apcore::{
    ACLRule, APCore, APCoreOptions, ApprovalRequirement, ChunkStream, Config, Context,
    ExecutionPolicy, Executor, Module, ModuleAnnotations, ModuleError, Registry, StreamEndReason,
    ACL,
};
use async_trait::async_trait;
use futures_util::{stream, StreamExt};
use serde_json::{json, Value};

mod call_budget;
mod capability_discovery;
#[path = "conformance_env.rs"]
mod conformance_env;
mod post_validation_policy;
mod realtime_context;

#[derive(Default)]
struct Report {
    annotations: ModuleAnnotations,
    executions: Arc<AtomicUsize>,
}

#[async_trait]
impl Module for Report {
    fn description(&self) -> &'static str {
        "Return a synthetic report"
    }

    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }

    fn output_schema(&self) -> Value {
        json!({"type": "object", "required": ["report"]})
    }

    fn annotations(&self) -> ModuleAnnotations {
        self.annotations.clone()
    }

    async fn execute(
        &self,
        inputs: Value,
        _context: &Context<Value>,
    ) -> Result<Value, ModuleError> {
        self.executions.fetch_add(1, Ordering::Relaxed);
        Ok(json!({"report": inputs}))
    }

    fn stream(&self, _inputs: Value, _context: &Context<Value>) -> Option<ChunkStream> {
        Some(Box::pin(stream::iter([
            Ok(json!({"report": 1})),
            Ok(json!({"report": 2})),
        ])))
    }
}

#[tokio::test]
async fn in_memory_assembly_invokes_streams_and_shuts_down() {
    let client = APCore::new();
    assert_eq!(client.config.project_root(), std::path::Path::new("."));
    assert_eq!(client.list_modules(None, None), Vec::<String>::new());
    assert!(!client.executor().governance_state().acl_configured);
    client
        .register("demo.report", Box::new(Report::default()))
        .unwrap();

    let value = client
        .call("demo.report", json!({"value": 1}), None, None)
        .await
        .unwrap();
    assert_eq!(value, json!({"report": {"value": 1}}));

    let (mut chunks, completion) = client.stream_with_completion("demo.report", None, None, None);
    assert_eq!(chunks.next().await.unwrap().unwrap(), json!({"report": 1}));
    assert_eq!(chunks.next().await.unwrap().unwrap(), json!({"report": 2}));
    assert!(chunks.next().await.is_none());
    let completed = completion.await;
    assert_eq!(completed.reason, StreamEndReason::Completed);
    assert_eq!(completed.output.unwrap(), json!({"report": 2}));

    client.shutdown().await.unwrap();
    assert_eq!(client.list_modules(None, None), Vec::<String>::new());
}

#[cfg(not(feature = "config"))]
#[tokio::test]
async fn missing_file_configuration_feature_disables_configured_acl_calls() {
    use apcore::{APCoreStartupError, ErrorCode};

    let mut options = Config::in_memory();
    options.set("acl.root", json!("acl.yaml"));
    let startup = APCore::try_start(APCoreOptions {
        config: Some(options.clone()),
        ..APCoreOptions::default()
    });
    assert!(matches!(
        startup,
        Err(APCoreStartupError::AclLoadFailed { .. })
    ));

    let client = APCore::with_config(options);
    client
        .register("demo.report", Box::new(Report::default()))
        .unwrap();
    let result = client.call("demo.report", json!({}), None, None).await;
    assert_eq!(result.unwrap_err().code, ErrorCode::ConfigInvalid);
}

#[tokio::test]
async fn core_profile_governance_matches_shared_fixture() {
    let path = conformance_env::find_fixtures_root().join("core_profile_governance.json");
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    for case in fixture["test_cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let input = &case["input"];
        let expected = &case["expected"];
        let client = governance_client(input);
        let executions = Arc::new(AtomicUsize::new(0));
        client
            .register(
                "demo.report",
                Box::new(Report {
                    annotations: ModuleAnnotations {
                        requires_approval: input["approval_source"] == "annotation",
                        destructive: input["approval_source"] == "destructive",
                        ..ModuleAnnotations::default()
                    },
                    executions: Arc::clone(&executions),
                }),
            )
            .unwrap();

        let preflight = client.validate("demo.report", None, None).await.unwrap();
        let result = client.call("demo.report", json!({}), None, None).await;
        let code = result.err().map_or(Value::Null, |error| json!(error.code));
        assert_eq!(
            json!(preflight.requires_approval),
            expected["requires_approval"],
            "{id}"
        );
        assert_eq!(code, expected["error_code"], "{id}");
        assert_eq!(
            json!(executions.load(Ordering::Relaxed)),
            expected["executions"],
            "{id}"
        );
    }
}

fn governance_client(input: &Value) -> APCore {
    let mut config = Config::in_memory();
    config.set("acl.root", Value::Null);
    let policy = if input["approval_source"] == "policy_rule" {
        Some(json!({"rules": [{"pattern": "demo.report", "requires_approval": true}]}))
    } else {
        input.get("execution_policy").cloned()
    }
    .map(|policy| ExecutionPolicy::from_value(policy).unwrap());
    let acl = (input["approval_source"] == "acl").then(|| {
        let mut rule = ACLRule::new(vec!["*".into()], vec!["demo.report".into()], "allow");
        rule.approval = Some(ApprovalRequirement::Required);
        ACL::try_new(vec![rule], "deny", None).unwrap()
    });

    if acl.is_some() || input["client"] == "injected_executor" {
        let executor = Executor::with_options(Registry::new(), config.clone(), None, acl, None);
        APCore::with_options(None, Some(executor), Some(config), None)
    } else if policy.is_some() {
        APCore::try_start(APCoreOptions {
            config: Some(config),
            policy,
            ..APCoreOptions::default()
        })
        .unwrap()
    } else {
        APCore::new()
    }
}

#[test]
fn a_custom_strategy_without_the_builtin_gate_reports_that_boundary() {
    use apcore::build_minimal_strategy;

    let executor = Executor::with_strategy(
        Registry::new(),
        Config::in_memory(),
        build_minimal_strategy(),
    );
    let state = executor.governance_state();
    assert!(!state.policy_strict);
    assert!(!state.builtin_approval_gate_wired);
}
