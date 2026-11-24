//! Cross-language coverage for the JSON representation of typed module results.

use apcore::{
    bindings::typed_handler, ChunkStream, Config, Context, ErrorCode, Executor, FunctionModule,
    Module, ModuleAnnotations, ModuleError, Registry, StreamingModule,
};
use async_trait::async_trait;
use futures_util::TryStreamExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::conformance_env::find_fixtures_root;

#[derive(Clone, Deserialize, JsonSchema, Serialize)]
struct Item {
    id: String,
}

#[derive(Clone, Deserialize, JsonSchema, Serialize)]
struct Output {
    item: Item,
    updated_at: String,
}

#[derive(Deserialize, JsonSchema)]
struct Input {}

struct NativeModule {
    output: Output,
    streaming: bool,
}

impl NativeModule {
    fn serialized_output(&self) -> Value {
        serde_json::to_value(&self.output).expect("serialize the typed fixture output")
    }
}

#[async_trait]
impl Module for NativeModule {
    fn description(&self) -> &str {
        "Return a typed object"
    }

    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }

    fn output_schema(&self) -> Value {
        serde_json::to_value(schemars::schema_for!(Output)).expect("typed output schema")
    }

    async fn execute(&self, _: Value, _: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(self.serialized_output())
    }

    fn stream(&self, inputs: Value, context: &Context<Value>) -> Option<ChunkStream> {
        self.as_streaming()
            .map(|streaming| streaming.stream_typed(inputs, context))
    }

    fn as_streaming(&self) -> Option<&dyn StreamingModule> {
        self.streaming.then_some(self)
    }
}

impl StreamingModule for NativeModule {
    fn stream_typed(&self, _: Value, _: &Context<Value>) -> ChunkStream {
        Box::pin(futures_util::stream::once(std::future::ready(Ok(
            self.serialized_output()
        ))))
    }
}

#[tokio::test]
async fn typed_module_outputs() {
    let path = find_fixtures_root().join("module_output_normalization.json");
    let data = std::fs::read_to_string(path).expect("read fixture");
    let fixture: Value = serde_json::from_str(&data).expect("parse fixture");

    for case in fixture["test_cases"].as_array().expect("test_cases array") {
        let output: Output = serde_json::from_value(case["input"].clone()).expect("typed fixture");
        let typed_output = output.clone();
        let typed = typed_handler::<Input, Output>(move |_| Ok(typed_output.clone()));
        let handler = typed.handler;
        let wrapped = FunctionModule::with_description(
            ModuleAnnotations::default(),
            typed.input_schema.expect("typed input schema"),
            typed.output_schema.expect("typed output schema"),
            "Return a typed object",
            None,
            vec![],
            "1.0.0",
            Default::default(),
            vec![],
            move |inputs, context| handler(inputs, context),
        );
        let registry = Registry::new();
        let native = |streaming| {
            Box::new(NativeModule {
                output: output.clone(),
                streaming,
            })
        };
        registry
            .register_module("test.class", native(false))
            .unwrap();
        registry
            .register_module("test.wrapped", Box::new(wrapped))
            .unwrap();
        registry
            .register_module("test.streaming", native(true))
            .unwrap();
        let executor = Executor::new(registry, Config::from_defaults());

        for module_id in ["test.class", "test.wrapped", "test.streaming"] {
            let result = executor
                .call(module_id, json!({}), None, None)
                .await
                .unwrap();
            assert_eq!(result, case["expected"]["output"], "{module_id}");

            let chunks: Vec<Value> = executor
                .stream(module_id, json!({}), None, None)
                .try_collect()
                .await
                .unwrap();
            assert_eq!(
                Value::Array(chunks),
                case["expected"]["chunks"],
                "{module_id}"
            );
        }
    }
}

#[tokio::test]
async fn execute_rejects_a_non_map_result() {
    let registry = Registry::new();
    let invalid = FunctionModule::with_description(
        ModuleAnnotations::default(),
        json!({}),
        json!({}),
        "Return a non-map value",
        None,
        vec![],
        "1.0.0",
        Default::default(),
        vec![],
        |_, _| Box::pin(async { Ok(json!(["not an object"])) }),
    );
    let module_id = "test.invalid_output";
    registry
        .register_module(module_id, Box::new(invalid))
        .unwrap();
    let executor = Executor::new(registry, Config::from_defaults());

    let result = executor.call(module_id, json!({}), None, None).await;
    assert_eq!(result.unwrap_err().code, ErrorCode::ModuleExecuteError);
    let chunks: Result<Vec<_>, _> = executor
        .stream(module_id, json!({}), None, None)
        .try_collect()
        .await;
    assert_eq!(chunks.unwrap_err().code, ErrorCode::ModuleExecuteError);
}
