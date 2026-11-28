use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use apcore::{
    APCore, CancelToken, Context, ElicitationHandler, ElicitationResult, ErrorCode, Module,
    ModuleError, ProgressReporter,
};
use async_trait::async_trait;
use serde_json::{json, Value};

struct InteractionModule;

#[async_trait]
impl Module for InteractionModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }

    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }

    fn description(&self) -> &'static str {
        "Test execution-local interaction"
    }

    async fn execute(&self, _inputs: Value, ctx: &Context<Value>) -> Result<Value, ModuleError> {
        ctx.report_progress(1.0, Some(2.0), Some("starting"))
            .await?;
        let result = ctx.elicit("Continue?", json!({"type": "object"})).await?;
        Ok(json!({"answer": result}))
    }
}

#[tokio::test]
async fn callbacks_are_derived_and_local_to_rust_contexts() {
    let base = Context::<Value>::anonymous();
    let calls = Arc::new(AtomicUsize::new(0));
    let reporter: ProgressReporter = Arc::new({
        let calls = Arc::clone(&calls);
        move |update| {
            let calls = Arc::clone(&calls);
            Box::pin(async move {
                assert_eq!(update.progress.to_bits(), 1.0f64.to_bits());
                assert_eq!(update.total.map(f64::to_bits), Some(2.0f64.to_bits()));
                assert_eq!(update.message.as_deref(), Some("starting"));
                calls.fetch_add(1, Ordering::Relaxed);
                Ok(())
            })
        }
    });
    let handler: ElicitationHandler = Arc::new(|request| {
        Box::pin(async move {
            assert_eq!(request.message, "Continue?");
            assert_eq!(request.requested_schema.as_ref().unwrap()["type"], "object");
            Ok(ElicitationResult::Accept {
                content: serde_json::Map::from_iter([("confirm".into(), json!(true))]),
            })
        })
    });
    let interactive = base
        .with_progress_reporter(Some(reporter))
        .with_elicitation_handler(Some(handler));
    assert_eq!(base.serialize(), interactive.serialize());
    let mut wire = serde_json::to_value(&interactive).unwrap();
    assert_eq!(wire, serde_json::to_value(&base).unwrap());
    wire["runtime"] = json!({"progress_reporter":"untrusted"});
    let decoded: Context<Value> = serde_json::from_value(wire).unwrap();

    let client = APCore::new();
    client
        .register("test.interaction", Box::new(InteractionModule))
        .unwrap();
    let output = client
        .call("test.interaction", json!({}), Some(&interactive), None)
        .await
        .unwrap();
    assert_eq!(
        output,
        json!({"answer": {"action":"accept","content":{"confirm":true}}})
    );
    for context in [
        base,
        decoded,
        interactive
            .with_progress_reporter(None)
            .with_elicitation_handler(None),
    ] {
        let output = client
            .call("test.interaction", json!({}), Some(&context), None)
            .await
            .unwrap();
        assert_eq!(output, json!({"answer": null}));
    }
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn interruption_of_a_pending_callback_preserves_cancellation_and_timeout_errors() {
    let token = CancelToken::new();
    let started = Arc::new(tokio::sync::Notify::new());
    let reporter: ProgressReporter = Arc::new({
        let started = Arc::clone(&started);
        move |_| {
            started.notify_one();
            Box::pin(std::future::pending())
        }
    });
    let context = Context::create(None, None, Some(token.clone()), None, Value::Null, None)
        .with_progress_reporter(Some(reporter));
    let worker = tokio::spawn(async move { context.report_progress(1.0, None, None).await });
    started.notified().await;
    token.cancel();
    let error = worker.await.unwrap().unwrap_err();
    assert_eq!(error.code, ErrorCode::ExecutionCancelled);

    let deadline = (SystemTime::now() + Duration::from_millis(20))
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let handler: ElicitationHandler = Arc::new(|_| Box::pin(std::future::pending()));
    let context = Context::create(None, None, None, None, Value::Null, Some(deadline))
        .with_elicitation_handler(Some(handler));
    let error = context
        .elicit("Continue?", json!({"type":"object"}))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ModuleTimeout);
}
