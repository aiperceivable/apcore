#![allow(clippy::result_large_err)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use apcore::{
    build_minimal_strategy, APCore, APCoreOptions, ApprovalHandler, ApprovalRequest,
    ApprovalResult, CallBudget, CancelToken, ChunkStream, Config, Context, ErrorCode, Executor,
    Module, ModuleAnnotations, ModuleError, Registry, RetryConfig, RetryMiddleware,
};
use async_trait::async_trait;
use futures_util::{stream, StreamExt};
use serde_json::{json, Value};

struct Metered {
    started: AtomicUsize,
    needs_approval: bool,
    streaming: bool,
}

#[async_trait]
impl Module for Metered {
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"actual":{"type":"integer"}}})
    }

    fn output_schema(&self) -> Value {
        json!({"type":"object"})
    }

    fn description(&self) -> &'static str {
        "Test invocation charges"
    }

    fn annotations(&self) -> ModuleAnnotations {
        ModuleAnnotations {
            requires_approval: self.needs_approval,
            ..ModuleAnnotations::default()
        }
    }

    async fn execute(&self, input: Value, ctx: &Context<Value>) -> Result<Value, ModuleError> {
        let index = self.started.fetch_add(1, Ordering::Relaxed);
        if let Some(actual) = input["actual"].as_u64() {
            ctx.record_cost(actual)?;
            if input["record_again"] == true {
                ctx.record_cost(actual)?;
            }
        }
        if input["retry_once"] == true && index == 0 {
            return Err(ModuleError::new(
                ErrorCode::GeneralInternalError,
                "retry this attempt",
            ));
        }
        Ok(json!({"started":index + 1}))
    }

    fn stream(&self, input: Value, ctx: &Context<Value>) -> Option<ChunkStream> {
        if !self.streaming {
            return None;
        }
        let context = ctx.clone();
        Some(Box::pin(stream::once(async move {
            context.record_cost(input["actual"].as_u64().unwrap())?;
            Ok(json!({"streamed":true}))
        })))
    }
}

fn register(client: &APCore, needs_approval: bool, streaming: bool) {
    client
        .register(
            "test.cost",
            Box::new(Metered {
                started: AtomicUsize::new(0),
                needs_approval,
                streaming,
            }),
        )
        .unwrap();
}

fn budget(max_calls: Option<u64>, max_cost: Option<u64>) -> Arc<CallBudget> {
    Arc::new(
        CallBudget::new(
            max_calls,
            max_cost,
            HashMap::from([("test.cost".into(), 3)]),
        )
        .unwrap(),
    )
}

#[tokio::test]
async fn call_budget_matches_the_shared_conformance_fixture() {
    let fixture_path = super::conformance_env::find_fixtures_root().join("call_budget.json");
    let fixture: Value =
        serde_json::from_str(&std::fs::read_to_string(fixture_path).unwrap()).unwrap();
    for test in fixture["test_cases"].as_array().unwrap() {
        let id = test["id"].as_str().unwrap();
        let module_id = test["module_id"].as_str().unwrap();
        let budget = Arc::new(
            CallBudget::new(
                test["max_calls"].as_u64(),
                test["max_cost"].as_u64(),
                HashMap::from([(
                    module_id.to_owned(),
                    test["estimated_cost"].as_u64().unwrap(),
                )]),
            )
            .unwrap(),
        );
        let context = Context::<Value>::anonymous().with_budget(Some(Arc::clone(&budget)));
        let executor = Executor::new(Registry::new(), Config::in_memory());
        executor
            .registry()
            .register_module(
                module_id,
                Box::new(Metered {
                    started: AtomicUsize::new(0),
                    needs_approval: false,
                    streaming: false,
                }),
            )
            .unwrap();
        for (index, call) in test["calls"].as_array().unwrap().iter().enumerate() {
            let inputs = call["actual_cost"]
                .as_u64()
                .map_or_else(|| json!({}), |actual| json!({"actual":actual}));
            let result = executor.call(module_id, inputs, Some(&context), None).await;
            let mut observed = serde_json::to_value(budget.snapshot()).unwrap();
            observed["error_code"] = result
                .err()
                .map_or(Value::Null, |error| json!(error.wire_code()));
            assert_eq!(observed, test["expected"][index], "{id} call {index}");
        }
    }
}

#[tokio::test]
async fn charges_only_started_calls_and_replaces_the_estimate_with_actual() {
    let client = APCore::new();
    register(&client, false, false);
    let budget = budget(Some(4), Some(5));
    let context = Context::<Value>::anonymous().with_budget(Some(Arc::clone(&budget)));
    assert!(CallBudget::new(Some(1u64 << 53), None, HashMap::new()).is_err());
    assert!(context.record_cost(2).is_err());
    assert_eq!(
        context.clone().serialize(),
        context.with_budget(None).serialize()
    );

    let preflight = client
        .validate("test.cost", &json!({}), Some(&context))
        .await
        .unwrap();
    assert!(preflight.valid);
    let _ = client
        .call("test.missing", json!({}), Some(&context), None)
        .await
        .unwrap_err();
    let _ = client
        .call(
            "test.cost",
            json!({"actual":"invalid"}),
            Some(&context),
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(budget.snapshot().calls_reserved, 0);
    assert_eq!(budget.snapshot().calls_used, 0);
    assert_eq!(budget.snapshot().cost_used, 0);

    client
        .call("test.cost", json!({"actual":0}), Some(&context), None)
        .await
        .unwrap();
    assert_eq!(budget.snapshot().cost_used, 0);
    let error = client
        .call("test.cost", json!({"actual":6}), Some(&context), None)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::CallBudgetExceeded);
    assert_eq!(error.details["dimension"], "cost");
    assert_eq!(error.retryable, Some(false));
    assert_eq!(budget.snapshot().calls_used, 2);
    assert_eq!(budget.snapshot().cost_used, 6);

    let denied = client
        .call("test.cost", json!({}), Some(&context), None)
        .await
        .unwrap_err();
    assert_eq!(denied.code, ErrorCode::CallBudgetExceeded);
    assert_eq!(denied.details["dimension"], "cost");
    assert_eq!(budget.snapshot().calls_used, 2);
    assert_eq!(budget.snapshot().calls_reserved, 0);
}

#[derive(Debug, Clone, Default)]
struct Pending {
    requested: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}

#[async_trait]
impl ApprovalHandler for Pending {
    async fn request_approval(&self, _: &ApprovalRequest) -> Result<ApprovalResult, ModuleError> {
        self.requested.notify_one();
        self.release.notified().await;
        let mut result = ApprovalResult::default();
        result.status = "pending".into();
        result.approval_id = Some("budget-approval".to_owned());
        Ok(result)
    }

    async fn check_approval(&self, _: &str) -> Result<ApprovalResult, ModuleError> {
        Ok(ApprovalResult::approved("budget-reviewer"))
    }
}

#[tokio::test]
async fn pending_and_cancelled_calls_release_a_concurrent_reservation() {
    let pending = Pending::default();
    let client = Arc::new(
        APCore::try_start(APCoreOptions {
            approval_handler: Some(Box::new(pending.clone())),
            ..APCoreOptions::default()
        })
        .unwrap(),
    );
    register(&client, true, false);
    let budget = budget(Some(1), None);
    let context = Context::<Value>::anonymous().with_budget(Some(Arc::clone(&budget)));

    let worker = tokio::spawn({
        let client = Arc::clone(&client);
        let context = context.clone();
        async move {
            client
                .call("test.cost", json!({}), Some(&context), None)
                .await
        }
    });
    pending.requested.notified().await;
    assert_eq!(budget.snapshot().calls_reserved, 1);
    let error = client
        .call("test.cost", json!({}), Some(&context), None)
        .await
        .unwrap_err();
    assert_eq!(error.details["dimension"], "calls");
    pending.release.notify_one();
    assert_eq!(
        worker.await.unwrap().unwrap_err().code,
        ErrorCode::ApprovalPending
    );
    assert_eq!(budget.snapshot().calls_used, 0);
    assert_eq!(budget.snapshot().calls_reserved, 0);

    let mut cancelled = context;
    let token = CancelToken::new();
    token.cancel();
    cancelled.cancel_token = Some(token);
    let error = client
        .call("test.cost", json!({}), Some(&cancelled), None)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ExecutionCancelled);
    assert_eq!(budget.snapshot().calls_reserved, 0);
}

#[tokio::test]
async fn retries_spend_once_per_started_attempt_and_streams_charge_once() {
    let client = APCore::new();
    register(&client, false, false);
    let mut retry = RetryConfig::default();
    retry.max_retries = 1;
    retry.base_delay_ms = 0;
    retry.jitter = false;
    client
        .use_middleware(Box::new(RetryMiddleware::new(retry)))
        .unwrap();
    let retry_budget = budget(None, None);
    let context = Context::<Value>::anonymous().with_budget(Some(Arc::clone(&retry_budget)));
    let output = client
        .call(
            "test.cost",
            json!({"retry_once":true}),
            Some(&context),
            None,
        )
        .await
        .unwrap();
    assert_eq!(output["started"], 2);
    assert_eq!(retry_budget.snapshot().calls_used, 2);
    assert_eq!(retry_budget.snapshot().cost_used, 6);
    let chunks = client
        .stream("test.cost", json!({"actual":4}), Some(&context), None)
        .collect::<Vec<_>>()
        .await;
    assert_eq!(chunks[0].as_ref().unwrap()["started"], 3);
    assert_eq!(retry_budget.snapshot().calls_used, 3);
    assert_eq!(retry_budget.snapshot().cost_used, 10);

    let streaming = APCore::new();
    register(&streaming, false, true);
    let streaming_budget = budget(Some(1), Some(10));
    let context = context.with_budget(Some(Arc::clone(&streaming_budget)));
    let chunks = streaming
        .stream("test.cost", json!({"actual":8}), Some(&context), None)
        .collect::<Vec<_>>()
        .await;
    assert_eq!(chunks[0].as_ref().unwrap()["streamed"], true);
    assert_eq!(streaming_budget.snapshot().calls_used, 1);
    assert_eq!(streaming_budget.snapshot().cost_used, 8);
    let denied = streaming
        .stream("test.cost", json!({"actual":1}), Some(&context), None)
        .next()
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(denied.code, ErrorCode::CallBudgetExceeded);
    assert_eq!(denied.details["dimension"], "calls");
}

#[tokio::test]
async fn redundant_cost_reporting_cannot_replace_a_charge_and_wide_totals_remain_portable() {
    let client = APCore::new();
    register(&client, false, false);
    let base = Context::<Value>::anonymous();
    assert!(base.record_cost(u64::MAX).is_ok());
    let budget = budget(None, None);
    let ctx = base.with_budget(Some(Arc::clone(&budget)));
    let duplicated = client
        .call(
            "test.cost",
            json!({"actual": 4, "record_again":true}),
            Some(&ctx),
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(duplicated.code, ErrorCode::GeneralInvalidInput);
    assert_eq!(budget.snapshot().cost_used, 4);
    let max_safe = (1u64 << 53) - 1;
    client
        .call("test.cost", json!({"actual":max_safe}), Some(&ctx), None)
        .await
        .unwrap();
    assert_eq!(budget.snapshot().cost_used, max_safe);
}

#[tokio::test]
async fn custom_strategy_without_a_stock_call_chain_guard_cannot_spend_budget() {
    let config = Config::in_memory();
    let executor =
        Executor::with_strategy(Registry::new(), config.clone(), build_minimal_strategy());
    let client = APCore::with_options(None, Some(executor), Some(config), None);
    register(&client, false, false);
    let budget = budget(Some(1), None);
    let ctx = Context::<Value>::anonymous().with_budget(Some(Arc::clone(&budget)));

    let denied = client
        .call("test.cost", json!({}), Some(&ctx), None)
        .await
        .unwrap_err();
    assert_eq!(denied.code, ErrorCode::PipelineConfigurationError);
    assert_eq!(budget.snapshot().calls_used, 0);
    assert_eq!(budget.snapshot().calls_reserved, 0);
    let approved = client
        .call("test.cost", json!({}), None, None)
        .await
        .unwrap();
    assert_eq!(approved["started"], 1);
}
