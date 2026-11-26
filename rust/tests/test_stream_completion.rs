use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use apcore::{
    build_strategy_from_config, APCore, CancelToken, Config, Context, ErrorCode, Executor,
    Middleware, Module, ModuleError, PipelineState, Registry, StepMiddleware, StreamEndReason,
};
use async_stream::stream;
use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::sync::Notify;
use tokio::time::timeout;

mod conformance_env;

const MODULE_ID: &str = "test.completion";
const TEST_TIMEOUT: Duration = Duration::from_secs(2);

struct FixtureModule {
    input: Value,
    execution_count: Arc<AtomicUsize>,
    observed_cancel_on_drop: Option<Arc<AtomicBool>>,
}

struct CancellationOnDrop {
    token: CancelToken,
    observation: Arc<AtomicBool>,
}

impl Drop for CancellationOnDrop {
    fn drop(&mut self) {
        self.observation
            .store(self.token.is_cancelled(), Ordering::SeqCst);
    }
}

#[async_trait]
impl Module for FixtureModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }

    fn output_schema(&self) -> Value {
        match self.input["validation"].as_str() {
            Some("invalid" | "ignored" | "recovered" | "no_match") => {
                json!({"type": "object", "required": ["value"], "properties": {"value": {"type": "integer"}}})
            }
            _ => json!({"type": "object"}),
        }
    }

    fn description(&self) -> &'static str {
        "Stream completion fixture module"
    }

    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        assert_eq!(self.input["mode"], "fallback");
        self.execution_count.fetch_add(1, Ordering::SeqCst);
        if self.input["action"] == "timeout_with_step_error_hook" {
            return std::future::pending().await;
        }
        Ok(self.input["chunks"][0].clone())
    }

    fn stream(&self, _inputs: Value, ctx: &Context<Value>) -> Option<apcore::ChunkStream> {
        if self.input["mode"] == "fallback" {
            return None;
        }
        self.execution_count.fetch_add(1, Ordering::SeqCst);
        let input = self.input.clone();
        let cancellation_on_drop =
            self.observed_cancel_on_drop
                .clone()
                .map(|observation| CancellationOnDrop {
                    token: ctx.cancel_token.clone().unwrap(),
                    observation,
                });
        Some(Box::pin(stream! {
            let _cancellation_on_drop = cancellation_on_drop;
            for chunk in input["chunks"].as_array().unwrap() {
                yield Ok(chunk.clone());
            }
            match input["action"].as_str() {
                Some("fail_after_first" | "recover_after_first") => {
                    yield Err(ModuleError::new(ErrorCode::ModuleExecuteError, "producer failed"));
                }
                Some("timeout_after_first" | "block_after_first") => std::future::pending().await,
                _ => {}
            }
        }))
    }
}

#[derive(Debug)]
struct FixtureMiddleware {
    after: String,
    recover: bool,
    after_entered: Arc<Notify>,
}

#[async_trait]
impl Middleware for FixtureMiddleware {
    fn name(&self) -> &'static str {
        "FixtureMiddleware"
    }

    async fn before(
        &self,
        _module_id: &str,
        _inputs: Value,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        Ok(None)
    }

    async fn after(
        &self,
        _module_id: &str,
        _inputs: Value,
        mut output: Value,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        match self.after.as_str() {
            "mutate" => {
                output["value"] = json!(2);
                Ok(Some(output))
            }
            "fail" => Err(ModuleError::new(
                ErrorCode::ModuleExecuteError,
                "after failed",
            )),
            "cancel" => Err(ModuleError::new(
                ErrorCode::ExecutionCancelled,
                "after cancelled",
            )),
            "wait" | "block" => {
                self.after_entered.notify_one();
                std::future::pending().await
            }
            _ => Ok(None),
        }
    }

    async fn on_error(
        &self,
        _module_id: &str,
        _inputs: Value,
        _error: &ModuleError,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        Ok(self.recover.then(|| json!({"recovered": true})))
    }
}

struct RecoverValidation;

#[async_trait]
impl StepMiddleware for RecoverValidation {
    async fn on_step_error(
        &self,
        name: &str,
        _state: &PipelineState<'_>,
        _error: &ModuleError,
    ) -> Result<Option<Value>, ModuleError> {
        Ok((name == "output_validation").then(|| json!({"value": 2})))
    }
}

struct FailBeforeStep(&'static str);

#[async_trait]
impl StepMiddleware for FailBeforeStep {
    async fn before_step(&self, name: &str, _state: &PipelineState<'_>) -> Result<(), ModuleError> {
        if name == self.0 {
            return Err(ModuleError::new(
                ErrorCode::ModuleExecuteError,
                "return hook failed",
            ));
        }
        Ok(())
    }
}

struct WaitOnExecuteError;

#[async_trait]
impl StepMiddleware for WaitOnExecuteError {
    async fn on_step_error(
        &self,
        name: &str,
        _state: &PipelineState<'_>,
        _error: &ModuleError,
    ) -> Result<Option<Value>, ModuleError> {
        if name == "execute" {
            return std::future::pending().await;
        }
        Ok(None)
    }
}

fn client_for_case(input: &Value, calls: Arc<AtomicUsize>) -> (APCore, Arc<Notify>) {
    let mut config = Config::default();
    if let Some(depth) = input.get("max_merge_depth") {
        config.set("stream.max_merge_depth", depth.clone());
    }
    let pipeline = match input["validation"].as_str().unwrap() {
        "removed" => json!({"remove": ["output_validation"]}),
        "no_match" => json!({"configure": {"output_validation": {"match_modules": ["other.*"]}}}),
        "ignored" => json!({"configure": {"output_validation": {"ignore_errors": true}}}),
        _ => json!({}),
    };
    let mut strategy = build_strategy_from_config(&pipeline).unwrap();
    if input["validation"] == "recovered" {
        strategy.add_step_middleware(Arc::new(RecoverValidation));
    }
    if input["action"] == "fail_return_result" {
        strategy.add_step_middleware(Arc::new(FailBeforeStep("return_result")));
    }
    if input["action"] == "fail_before_validation" {
        strategy.add_step_middleware(Arc::new(FailBeforeStep("output_validation")));
    }
    if input["action"] == "timeout_with_step_error_hook" {
        config.set("executor.default_timeout", json!(25));
        strategy.add_step_middleware(Arc::new(WaitOnExecuteError));
    }
    let executor = Executor::with_strategy(Registry::new(), config, strategy);
    let client = APCore::with_options(None, Some(executor), None, None);
    client
        .register(
            MODULE_ID,
            Box::new(FixtureModule {
                input: input.clone(),
                execution_count: calls,
                observed_cancel_on_drop: None,
            }),
        )
        .unwrap();
    let after_entered = Arc::new(Notify::new());
    client
        .use_middleware(Box::new(FixtureMiddleware {
            after: input["after"].as_str().unwrap().to_string(),
            recover: matches!(
                input["action"].as_str(),
                Some("recover_after_first" | "recover_fallback")
            ),
            after_entered: Arc::clone(&after_entered),
        }))
        .unwrap();
    (client, after_entered)
}

#[tokio::test]
async fn stream_completion_conformance() {
    let path = conformance_env::find_fixtures_root().join("stream_completion.json");
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let cases = fixture["test_cases"].as_array().unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        timeout(TEST_TIMEOUT, run_fixture_case(case))
            .await
            .unwrap_or_else(|_| panic!("completion timed out for {}", case["id"]));
    }
}

async fn run_fixture_case(case: &Value) {
    let input = &case["input"];
    let expected = &case["expected"];
    let calls = Arc::new(AtomicUsize::new(0));
    let (client, after_entered) = client_for_case(input, Arc::clone(&calls));
    let token = CancelToken::new();
    let mut context = Context::anonymous();
    context.cancel_token = Some(token.clone());
    let action = input["action"].as_str().unwrap();
    if matches!(action, "timeout_after_first" | "timeout_during_after") {
        context.global_deadline = Some(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs_f64()
                + 0.25,
        );
    }
    let (mut chunks, completion) =
        client.stream_with_completion(MODULE_ID, None, Some(&context), None);
    let mut received = Vec::new();
    match action {
        "close_before_first" => chunks.close(),
        "close_after_first" | "cancel_after_first" => {
            received.push(chunks.next().await.unwrap().unwrap());
            if action == "close_after_first" {
                chunks.close();
            } else {
                token.cancel();
            }
        }
        "cancel_during_after" | "timeout_during_after" => {
            if input["mode"] == "native" {
                received.push(chunks.next().await.unwrap().unwrap());
            }
            tokio::select! {
                biased;
                () = after_entered.notified() => {}
                next = chunks.next() => panic!("stream terminated before after middleware: {next:?}"),
            }
            if action == "cancel_during_after" {
                token.cancel();
            }
        }
        _ => {}
    }

    let mut error = None;
    while let Some(chunk) = chunks.next().await {
        match chunk {
            Ok(value) => received.push(value),
            Err(err) => error = Some(err.code.wire_str()),
        }
    }
    chunks.close();
    let result = serde_json::to_value(completion.await).unwrap();
    let id = &case["id"];
    assert_eq!(result["reason"], expected["reason"], "{id}");
    assert_eq!(result["output"], expected["output"], "{id}");
    assert_eq!(
        result["output_validation"], expected["output_validation"],
        "{id}"
    );
    assert_eq!(
        result["middleware_after"], expected["middleware_after"],
        "{id}"
    );
    assert_eq!(result["error_code"], expected["error_code"], "{id}");
    if action != "recover_after_first" {
        assert_eq!(json!(error), expected["error_code"], "{id}");
    }
    assert_eq!(json!(received), expected["chunks"], "{id}");
    assert_eq!(
        json!(calls.load(Ordering::SeqCst)),
        expected["execution_count"],
        "{id}"
    );
}

#[tokio::test]
async fn unrepresented_post_failure_keeps_legacy_chunks_and_fails_completion() {
    let input = json!({
        "mode": "native", "validation": "pass", "after": "pass",
        "action": "fail_return_result", "chunks": [{"value": 1}]
    });
    let calls = Arc::new(AtomicUsize::new(0));
    let (client, _) = client_for_case(&input, Arc::clone(&calls));

    let (mut chunks, completion) = client.stream_with_completion(MODULE_ID, None, None, None);
    assert_eq!(chunks.next().await.unwrap().unwrap(), json!({"value": 1}));
    assert!(chunks.next().await.is_none());
    let result = completion.await;
    assert_eq!(result.reason, StreamEndReason::Failed);
    assert_eq!(result.error_code.as_deref(), Some("MODULE_EXECUTE_ERROR"));
    assert_eq!(result.output, Some(json!({"value": 1})));
    assert_eq!(result.output_validation, apcore::StreamStepOutcome::Passed);
    assert_eq!(result.middleware_after, apcore::StreamStepOutcome::Passed);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn native_post_stream_steps_observe_cancellation_and_deadline() {
    let input = json!({
        "mode": "native", "validation": "pass", "after": "wait",
        "action": "none", "chunks": [{"value": 1}]
    });
    let (client, _) = client_for_case(&input, Arc::new(AtomicUsize::new(0)));
    for (code, reason) in [
        (ErrorCode::ExecutionCancelled, StreamEndReason::Cancelled),
        (ErrorCode::ModuleTimeout, StreamEndReason::TimedOut),
    ] {
        let token = CancelToken::new();
        let mut context = Context::anonymous();
        context.cancel_token = Some(token.clone());
        if code == ErrorCode::ModuleTimeout {
            context.global_deadline = Some(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs_f64()
                    + 0.25,
            );
        }
        let (mut chunks, completion) =
            client.stream_with_completion(MODULE_ID, None, Some(&context), None);
        assert_eq!(chunks.next().await.unwrap().unwrap(), json!({"value": 1}));
        assert!(futures_util::poll!(chunks.next()).is_pending());
        if code == ErrorCode::ExecutionCancelled {
            token.cancel();
        }
        let error = timeout(TEST_TIMEOUT, chunks.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.code, code);
        let result = completion.await;
        assert_eq!(result.reason, reason);
        assert_eq!(result.output, Some(json!({"value": 1})));
        assert_eq!(result.output_validation, apcore::StreamStepOutcome::Passed);
        assert_eq!(
            result.middleware_after,
            apcore::StreamStepOutcome::Skipped {
                reason: apcore::StreamSkipReason::NotReached
            }
        );
    }
}

#[tokio::test]
async fn test_fallback_execute_timeout_keeps_its_error_during_hook_cleanup() {
    let input = json!({
        "mode": "fallback", "validation": "pass", "after": "pass",
        "action": "timeout_with_step_error_hook", "chunks": []
    });
    let calls = Arc::new(AtomicUsize::new(0));
    let (client, _) = client_for_case(&input, Arc::clone(&calls));
    let token = CancelToken::new();
    let mut context = Context::anonymous();
    context.cancel_token = Some(token.clone());
    let (mut chunks, completion) =
        client.stream_with_completion(MODULE_ID, None, Some(&context), None);
    let error = timeout(TEST_TIMEOUT, chunks.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::ModuleTimeout);
    assert_eq!(error.details.get("timeout_ms"), Some(&json!(25)));
    let result = completion.await;
    assert_eq!(result.reason, StreamEndReason::TimedOut);
    assert_eq!(result.output, None);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(!token.is_cancelled());
}

#[tokio::test]
async fn completion_snapshot_is_bound_to_the_actual_validation_entry() {
    for (mode, after, action, chunk, output) in [
        (
            "native",
            "pass",
            "fail_before_validation",
            json!({"value": 1}),
            None,
        ),
        (
            "fallback",
            "fail",
            "recover_fallback",
            json!({"recovered": true}),
            Some(json!({"value": 1})),
        ),
    ] {
        let input = json!({
            "mode": mode, "validation": "pass", "after": after,
            "action": action, "chunks": [{"value": 1}]
        });
        let (client, _) = client_for_case(&input, Arc::new(AtomicUsize::new(0)));
        let (mut chunks, completion) = client.stream_with_completion(MODULE_ID, None, None, None);
        assert_eq!(chunks.next().await.unwrap().unwrap(), chunk);
        assert!(chunks.next().await.is_none());
        assert_eq!(completion.await.output, output, "{action}");
    }
}

#[tokio::test]
async fn dropping_completion_chunks_cancels_producer_without_cancelling_parent() {
    let client = APCore::new();
    let cancel_seen = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    client
        .register(
            MODULE_ID,
            Box::new(FixtureModule {
                input: json!({"mode": "native", "action": "block_after_first", "chunks": [{"value": 1}]}),
                execution_count: Arc::clone(&calls),
                observed_cancel_on_drop: Some(Arc::clone(&cancel_seen)),
            }),
        )
        .unwrap();
    let parent = CancelToken::new();
    let mut context = Context::anonymous();
    context.cancel_token = Some(parent.clone());

    let (chunks, completion) = client.stream_with_completion(MODULE_ID, None, Some(&context), None);
    drop(chunks);
    assert_eq!(
        timeout(TEST_TIMEOUT, completion).await.unwrap().reason,
        StreamEndReason::ConsumerClosed
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    let (mut chunks, completion) =
        client.stream_with_completion(MODULE_ID, None, Some(&context), None);
    assert_eq!(chunks.next().await.unwrap().unwrap(), json!({"value": 1}));
    assert!(futures_util::poll!(chunks.next()).is_pending());
    drop(chunks);
    assert_eq!(
        timeout(TEST_TIMEOUT, completion).await.unwrap().reason,
        StreamEndReason::ConsumerClosed
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(cancel_seen.load(Ordering::SeqCst));
    assert!(!parent.is_cancelled());
}
