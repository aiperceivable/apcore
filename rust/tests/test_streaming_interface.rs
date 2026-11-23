// Issue #62 — StreamingModule trait and Module::as_streaming accessor
// Tests the typed streaming interface, invariant enforcement, and registry validation.

use apcore::context::Context;
use apcore::errors::ErrorCode;
use apcore::module::{Module, ModuleAnnotations, StreamingModule};
use apcore::registry::registry::{ModuleDescriptor, Registry};
use apcore::ChunkStream;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;

fn make_descriptor(id: &str, streaming: bool) -> ModuleDescriptor {
    let ann = ModuleAnnotations {
        streaming,
        ..ModuleAnnotations::default()
    };
    ModuleDescriptor {
        module_id: id.to_string(),
        name: None,
        description: String::new(),
        documentation: None,
        input_schema: json!({}),
        output_schema: json!({}),
        version: "1.0.0".to_string(),
        tags: vec![],
        annotations: Some(ann),
        examples: vec![],
        metadata: HashMap::new(),
        display: None,
        sunset_date: None,
        dependencies: vec![],
        enabled: true,
    }
}

// -- non-streaming module --
struct PlainModule;

#[async_trait]
impl Module for PlainModule {
    #[allow(clippy::unnecessary_literal_bound)]
    fn description(&self) -> &str {
        "plain"
    }
    fn input_schema(&self) -> Value {
        json!({})
    }
    fn output_schema(&self) -> Value {
        json!({})
    }
    async fn execute(
        &self,
        _: Value,
        _: &Context<Value>,
    ) -> Result<Value, apcore::errors::ModuleError> {
        Ok(json!({}))
    }
    // as_streaming() not overridden → returns None (default)
}

// -- streaming module --
struct MyStreamingModule;

#[async_trait]
impl Module for MyStreamingModule {
    #[allow(clippy::unnecessary_literal_bound)]
    fn description(&self) -> &str {
        "streaming"
    }
    fn input_schema(&self) -> Value {
        json!({})
    }
    fn output_schema(&self) -> Value {
        json!({})
    }
    async fn execute(
        &self,
        _: Value,
        _: &Context<Value>,
    ) -> Result<Value, apcore::errors::ModuleError> {
        Ok(json!({}))
    }
    fn stream(&self, inputs: Value, ctx: &Context<Value>) -> Option<ChunkStream> {
        Some(self.stream_typed(inputs, ctx))
    }
    fn as_streaming(&self) -> Option<&dyn StreamingModule> {
        Some(self)
    }
}

impl StreamingModule for MyStreamingModule {
    fn stream_typed(&self, _inputs: Value, _ctx: &Context<Value>) -> ChunkStream {
        use async_stream::stream;
        let s = stream! {
            yield Ok(json!({"chunk": 1}));
        };
        Box::pin(s)
    }
}

// -- faulty module: streaming annotation but no StreamingModule impl --
struct FaultyStreamingModule;

#[async_trait]
impl Module for FaultyStreamingModule {
    #[allow(clippy::unnecessary_literal_bound)]
    fn description(&self) -> &str {
        "faulty"
    }
    fn input_schema(&self) -> Value {
        json!({})
    }
    fn output_schema(&self) -> Value {
        json!({})
    }
    async fn execute(
        &self,
        _: Value,
        _: &Context<Value>,
    ) -> Result<Value, apcore::errors::ModuleError> {
        Ok(json!({}))
    }
    // No as_streaming() override → returns None (default)
    // This creates the mismatch: streaming=true annotation but no StreamingModule impl.
}

#[test]
fn non_streaming_module_returns_none_from_as_streaming() {
    let m = PlainModule;
    assert!(m.as_streaming().is_none());
}

#[test]
fn streaming_module_returns_some_from_as_streaming() {
    let m = MyStreamingModule;
    assert!(m.as_streaming().is_some());
}

#[test]
fn streaming_invariant_both_some_or_both_none_for_streaming_module() {
    let m = MyStreamingModule;
    let ctx = Context::anonymous();
    let has_stream = m.stream(json!({}), &ctx).is_some();
    let has_streaming = m.as_streaming().is_some();
    assert_eq!(
        has_stream, has_streaming,
        "invariant: stream() and as_streaming() must agree"
    );
}

#[test]
fn non_streaming_module_invariant_both_none() {
    let m = PlainModule;
    let ctx = Context::anonymous();
    let has_stream = m.stream(json!({}), &ctx).is_some();
    let has_streaming = m.as_streaming().is_some();
    assert_eq!(
        has_stream, has_streaming,
        "invariant: stream() and as_streaming() must both be None for plain modules"
    );
}

#[test]
fn registration_rejects_module_with_streaming_annotation_but_no_impl() {
    let registry = Registry::new();
    let result = registry.register(
        "test.faulty",
        Box::new(FaultyStreamingModule),
        make_descriptor("test.faulty", true),
    );
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code, ErrorCode::StreamingInterfaceMismatch);
    assert!(err.message.contains("test.faulty"));
}

#[test]
fn registration_accepts_streaming_module_with_impl() {
    let registry = Registry::new();
    let result = registry.register(
        "test.streaming",
        Box::new(MyStreamingModule),
        make_descriptor("test.streaming", true),
    );
    assert!(result.is_ok());
    // Verify it's actually registered and accessible
    assert!(registry.get("test.streaming").unwrap().is_some());
}

#[test]
fn registration_accepts_non_streaming_module_with_streaming_false() {
    let registry = Registry::new();
    let result = registry.register(
        "test.plain",
        Box::new(PlainModule),
        make_descriptor("test.plain", false),
    );
    assert!(result.is_ok());
}

#[test]
fn streaming_error_carries_module_id_in_details() {
    let registry = Registry::new();
    let result = registry.register(
        "test.mismatch",
        Box::new(FaultyStreamingModule),
        make_descriptor("test.mismatch", true),
    );
    let err = result.unwrap_err();
    assert_eq!(err.code, ErrorCode::StreamingInterfaceMismatch);
    let module_id_detail = err.details.get("module_id").and_then(|v| v.as_str());
    assert_eq!(module_id_detail, Some("test.mismatch"));
    let reason_detail = err.details.get("mismatch_reason").and_then(|v| v.as_str());
    assert!(reason_detail.is_some());
    assert!(reason_detail.unwrap().contains("missing_marker"));
}
