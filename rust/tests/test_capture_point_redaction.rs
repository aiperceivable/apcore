//! PROTOCOL_SPEC §10.6.1 "Where the rules apply" — the configured redaction
//! rules must reach the executor's capture point, not only log emission
//! (aiperceivable/apcore#120).
//!
//! Every case builds a real `Config`, constructs a real `Executor`, executes a
//! real module, and reads `context.redacted_inputs` / `redacted_output`. That
//! shape is the acceptance condition the issue names, and it is the shape no
//! pre-existing test had: every redaction test in the three SDKs drove either
//! the config object or the logger, which is why a MUST written in
//! `docs/features/observability.md` went unimplemented in all three for the
//! entire life of the keys.
//!
//! The fields are read through a MIDDLEWARE rather than the `Context` handed to
//! `call`: the pipeline derives a child context and the capture point writes to
//! that, so an assertion against the caller's object tests the wrong object.
//! Middleware is also where `ObsLoggingMiddleware` reads these fields, so it is
//! the surface the contract is about.

use apcore::config::Config;
use apcore::context::{Context, Identity};
use apcore::errors::ModuleError;
use apcore::executor::Executor;
use apcore::middleware::base::Middleware;
use apcore::module::Module;
use apcore::registry::{ModuleDescriptor, Registry};
use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

const SECRET: &str = "sk-abcdef123456";
const MODULE_ID: &str = "executor.test.echo";

/// Echoes `note` back, so the OUTPUT capture point has something to redact.
#[derive(Debug)]
struct Echo;

#[async_trait]
impl Module for Echo {
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "note": {"type": "string"},
                "label": {"type": "string"},
                "amount": {"type": "number"}
            }
        })
    }
    fn output_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"echoed": {"type": "string"}, "label": {"type": "string"}}
        })
    }
    fn description(&self) -> &'static str {
        "Echo the note back so the output capture point has something to redact"
    }
    async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({ "echoed": inputs["note"], "label": inputs["label"] }))
    }
}

#[derive(Debug, Default)]
struct Capture {
    inputs: Mutex<Option<HashMap<String, Value>>>,
    output: Mutex<Option<HashMap<String, Value>>>,
}

#[async_trait]
impl Middleware for Capture {
    fn name(&self) -> &'static str {
        "capture"
    }
    async fn before(
        &self,
        _id: &str,
        _inputs: Value,
        ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        (*self.inputs.lock()).clone_from(&ctx.redacted_inputs);
        Ok(None)
    }
    async fn after(
        &self,
        _id: &str,
        _inputs: Value,
        _output: Value,
        ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        (*self.inputs.lock()).clone_from(&ctx.redacted_inputs);
        (*self.output.lock()).clone_from(&ctx.redacted_output);
        Ok(None)
    }
    async fn on_error(
        &self,
        _id: &str,
        _inputs: Value,
        _error: &ModuleError,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        Ok(None)
    }
}

fn descriptor(module_id: &str, input_schema: Value, output_schema: Value) -> ModuleDescriptor {
    ModuleDescriptor {
        module_id: module_id.to_string(),
        name: None,
        description: "capture-point probe".to_string(),
        documentation: None,
        input_schema,
        output_schema,
        version: "1.0.0".to_string(),
        tags: vec![],
        annotations: None,
        examples: vec![],
        metadata: HashMap::new(),
        display: None,
        sunset_date: None,
        dependencies: vec![],
        enabled: true,
    }
}

fn config_with(redaction: Option<Value>) -> Config {
    let mut raw = json!({"version": "1.0", "project": {"name": "capture-point"}});
    if let Some(block) = redaction {
        raw["obs"] = json!({ "redaction": block });
    }
    serde_json::from_value(raw).expect("the probe configuration parses")
}

/// Run `MODULE_ID` under `redaction` and hand back what a middleware saw.
async fn run(redaction: Option<Value>) -> Arc<Capture> {
    let registry = Arc::new(Registry::new());
    let module = Echo;
    let d = descriptor(MODULE_ID, module.input_schema(), module.output_schema());
    registry.register(MODULE_ID, Box::new(Echo), d).unwrap();

    let executor = Executor::new(registry, Arc::new(config_with(redaction)));
    let seen = Arc::new(Capture::default());
    executor
        .use_middleware(Box::new(CaptureHandle(Arc::clone(&seen))))
        .expect("middleware registers");

    let ctx = Context::<Value>::new(Identity::new(
        "test".to_string(),
        "user".to_string(),
        vec![],
        HashMap::new(),
    ));
    executor
        .call(
            MODULE_ID,
            json!({"note": SECRET, "label": "public", "amount": 42}),
            Some(&ctx),
            None,
        )
        .await
        .expect("the probe module cannot fail");

    assert!(
        seen.inputs.lock().is_some(),
        "the capture point must have run at all"
    );
    seen
}

/// `Middleware` needs a `Box`, and the assertions need the same object, so the
/// registered value is a thin handle over the shared one.
#[derive(Debug)]
struct CaptureHandle(Arc<Capture>);

#[async_trait]
impl Middleware for CaptureHandle {
    fn name(&self) -> &str {
        self.0.name()
    }
    async fn before(
        &self,
        id: &str,
        inputs: Value,
        ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        self.0.before(id, inputs, ctx).await
    }
    async fn after(
        &self,
        id: &str,
        inputs: Value,
        output: Value,
        ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        self.0.after(id, inputs, output, ctx).await
    }
    async fn on_error(
        &self,
        id: &str,
        inputs: Value,
        error: &ModuleError,
        ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        self.0.on_error(id, inputs, error, ctx).await
    }
}

fn value_rule() -> Value {
    json!({"sensitive_keys": [], "regex_patterns": ["sk-[A-Za-z0-9]{6,}"]})
}

#[tokio::test]
async fn a_configured_regex_redacts_the_captured_input() {
    let seen = run(Some(value_rule())).await;
    let inputs = seen.inputs.lock().clone().unwrap();
    assert_eq!(inputs["note"], json!("***REDACTED***"));
    // The discriminating half: a field the rule does NOT match must survive, or
    // "redacted everything" would satisfy the assertion above.
    assert_eq!(inputs["label"], json!("public"));
}

#[tokio::test]
async fn a_configured_regex_redacts_the_captured_output() {
    let seen = run(Some(value_rule())).await;
    let output = seen.output.lock().clone().unwrap();
    assert_eq!(output["echoed"], json!("***REDACTED***"));
    assert_eq!(output["label"], json!("public"));
}

#[tokio::test]
async fn a_configured_sensitive_key_redacts_the_captured_input() {
    // `label` matches nothing in the shipped default list, so this can only
    // pass if the OPERATOR's list was read.
    let seen = run(Some(json!({"sensitive_keys": ["label"]}))).await;
    let inputs = seen.inputs.lock().clone().unwrap();
    assert_eq!(inputs["label"], json!("***REDACTED***"));
    assert_eq!(inputs["note"], json!(SECRET));
}

#[tokio::test]
async fn a_configured_replacement_token_is_used() {
    let seen = run(Some(
        json!({"sensitive_keys": ["label"], "replacement": "<<GONE>>"}),
    ))
    .await;
    assert_eq!(
        seen.inputs.lock().clone().unwrap()["label"],
        json!("<<GONE>>")
    );
}

#[tokio::test]
async fn a_non_string_value_is_still_not_tested() {
    // §10.6.1 requirement 2 holds at this surface too, not only at logging.
    let seen = run(Some(
        json!({"sensitive_keys": [], "regex_patterns": ["[0-9]+"]}),
    ))
    .await;
    assert_eq!(seen.inputs.lock().clone().unwrap()["amount"], json!(42));
}

#[tokio::test]
async fn an_explicitly_narrowed_rule_set_is_honoured() {
    // Requirement 3 is about the ABSENT case; an operator who writes an empty
    // list has configured something, and it must be obeyed.
    let seen = run(Some(json!({"sensitive_keys": []}))).await;
    assert_eq!(seen.inputs.lock().clone().unwrap()["note"], json!(SECRET));
}

#[tokio::test]
async fn with_no_redaction_configured_the_default_list_applies() {
    #[derive(Debug)]
    struct Secret;
    #[async_trait]
    impl Module for Secret {
        fn input_schema(&self) -> Value {
            json!({"type": "object", "properties": {
                "password": {"type": "string"}, "keep": {"type": "string"}}})
        }
        fn output_schema(&self) -> Value {
            json!({"type": "object", "properties": {"ok": {"type": "boolean"}}})
        }
        fn description(&self) -> &'static str {
            "Input carries a field the default list covers"
        }
        async fn execute(&self, _i: Value, _c: &Context<Value>) -> Result<Value, ModuleError> {
            Ok(json!({"ok": true}))
        }
    }
    // Requirement 3: "no configuration" means the DEFAULTS, never no redaction.
    //
    // This is a BEHAVIOUR CHANGE in this SDK and in apcore-typescript, and the
    // fourth divergence found at this surface: the capture point applied only
    // `x-sensitive` and the `_secret_` prefix here, while apcore-python passed
    // `sensitive_keys=None` into `redact_sensitive`, which resolves to the
    // canonical 16-entry default list. So `password` was redacted in the
    // captured input by one SDK and stored in plaintext by two.
    let registry = Arc::new(Registry::new());

    let module = Secret;
    let d = descriptor(
        "executor.test.secret",
        module.input_schema(),
        module.output_schema(),
    );
    registry
        .register("executor.test.secret", Box::new(Secret), d)
        .unwrap();

    let executor = Executor::new(registry, Arc::new(config_with(None)));
    let seen = Arc::new(Capture::default());
    executor
        .use_middleware(Box::new(CaptureHandle(Arc::clone(&seen))))
        .expect("middleware registers");

    let ctx = Context::<Value>::new(Identity::new(
        "test".to_string(),
        "user".to_string(),
        vec![],
        HashMap::new(),
    ));
    executor
        .call(
            "executor.test.secret",
            json!({"password": "hunter2", "keep": "v"}),
            Some(&ctx),
            None,
        )
        .await
        .expect("cannot fail");

    let inputs = seen.inputs.lock().clone().unwrap();
    assert_eq!(inputs["password"], json!("***REDACTED***"));
    assert_eq!(inputs["keep"], json!("v"));
}
