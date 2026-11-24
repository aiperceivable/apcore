//! Regression tests for spec v1.50.0 (D-103 – D-107) and the verified D11
//! deep-chain findings (DEC-002, STR-1, STR-2, MW-002, ERR-001, ERR-003,
//! ERR-004, OBS-002, SYS-*).
//!
//! Every test here pins a behaviour where apcore-rust answered differently from
//! apcore-python and apcore-typescript while all three test suites were green:
//! the divergence was in the code path taken, never in the signature.

#![allow(clippy::pedantic)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_stream::stream;
use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};

use apcore::errors::retryable_for_code;
use apcore::registry::registry::{DependencyInfo, DiscoveredModule, Discoverer, ModuleDescriptor};
use apcore::sys_modules::control::ReloadModule;
use apcore::sys_modules::{HealthModule, HealthSummaryModule, ManifestFullModule, ManifestModule};
use apcore::{build_minimal_strategy, ACLConditionHandler, ACLRule, ErrorHistory};
use apcore::{
    derive_module_ids, ApCoreEvent, BindingHandler, BindingLoader, ChunkStream, Config, Context,
    DiscoveredClass, DiscoveryConfig, ErrorCode, EventEmitter, EventSubscriber, Executor,
    InMemoryExporter, MetricsCollector, Middleware, Module, ModuleAnnotations, ModuleError,
    Registry, SchemaLoader, TracingMiddleware, ACL,
};

// ---------------------------------------------------------------------------
// Shared fixtures
// ---------------------------------------------------------------------------

/// Echoes its input; declares no schema so nothing filters the output.
struct EchoModule;

#[async_trait]
impl Module for EchoModule {
    fn input_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn output_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn description(&self) -> &'static str {
        "echo"
    }
    async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({ "echo": inputs }))
    }
}

fn registry_with(module_id: &str, module: Box<dyn Module>) -> Arc<Registry> {
    let registry = Registry::new();
    registry
        .register_module(module_id, module)
        .expect("register module");
    Arc::new(registry)
}

fn fresh_context() -> Context<Value> {
    Context::<Value>::create(None, None, None, None, Value::Null, None)
}

fn code_of(err: &ModuleError) -> String {
    err.to_dict()
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

// ===========================================================================
// 1. D-105 — the executor's ACL step MUST take the async path
// ===========================================================================

/// Resolvable only on the async path: it suspends before answering, so the
/// synchronous evaluator can only report UNEVALUABLE (§6.1.3 row 1).
struct AsyncOnlyTrue;

#[async_trait]
impl ACLConditionHandler for AsyncOnlyTrue {
    async fn evaluate(&self, _value: &Value, _ctx: &Context<Value>) -> bool {
        tokio::task::yield_now().await;
        true
    }
}

// D-105: an `allow` rule carrying a condition registered through
// `register_async_condition` MUST grant. On the sync path the key is
// "async only" -> UNEVALUABLE -> §6.1.1 makes the allow rule stop granting and
// the caller is denied, which made the whole async condition registry dead in
// the only path that enforces.
#[tokio::test]
async fn acl_step_resolves_async_only_conditions() {
    ACL::init_builtin_handlers();
    ACL::register_async_condition("__d105_async_only__", Arc::new(AsyncOnlyTrue));

    let mut rule = ACLRule::new(vec!["*".to_string()], vec!["*".to_string()], "allow");
    rule.conditions = Some(json!({ "__d105_async_only__": true }));
    let acl = ACL::try_new(vec![rule], "deny", None).expect("valid acl");

    let executor = Executor::with_options(
        registry_with("executor.d105.echo", Box::new(EchoModule)),
        Config::default(),
        None,
        Some(acl),
        None,
    );

    let result = executor
        .call("executor.d105.echo", json!({ "a": 1 }), None, None)
        .await;

    assert!(
        result.is_ok(),
        "an async-only condition on an allow rule MUST grant (D-105); got {:?}",
        result.err().map(|e| (code_of(&e), e.message))
    );
}

// ===========================================================================
// 2. DEC-002 — the binding annotation parser covers all 13 keys
// ===========================================================================

const FULL_ANNOTATIONS_BINDING: &str = r#"
spec_version: "1.0"
bindings:
  - module_id: executor.dec002.report
    target: "reports:build"
    input_schema: {type: object}
    output_schema: {type: object}
    description: "Build a report"
    annotations:
      readonly: true
      destructive: false
      idempotent: true
      requires_approval: true
      open_world: true
      streaming: false
      cacheable: true
      cache_ttl: 900
      cache_key_fields: ["tenant_id", "period"]
      paginated: true
      pagination_style: "offset"
      discoverable: false
      extra:
        mcp.category: "reports"
"#;

const STREAMING_BINDING: &str = r#"
spec_version: "1.0"
bindings:
  - module_id: executor.dec002.stream_claim
    target: "reports:stream"
    input_schema: {type: object}
    output_schema: {type: object}
    description: "Claims to stream"
    annotations:
      streaming: true
"#;

fn echo_handler() -> BindingHandler {
    Arc::new(|inputs: Value, _ctx: &Context<Value>| Box::pin(async move { Ok(inputs) }))
}

fn register_binding_doc(
    registry: &Registry,
    file_name: &str,
    yaml: &str,
    target: &str,
) -> Result<usize, ModuleError> {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(file_name);
    std::fs::write(&path, yaml).expect("write binding yaml");

    let mut loader = BindingLoader::new();
    loader.load_from_yaml(&path).expect("load binding yaml");

    let mut handlers: HashMap<String, BindingHandler> = HashMap::new();
    handlers.insert(target.to_string(), echo_handler());
    loader.register_into_with_handlers(registry, handlers)
}

// DEC-002: the hand-rolled `annotations_from_value` matched five of the twelve
// declared keys; every other one — including the literal `extra` key — fell
// into the catch-all and was nested under its own name, violating §4.4.1.
#[test]
fn binding_annotations_round_trip_every_declared_key() {
    let registry = Registry::new();
    register_binding_doc(
        &registry,
        "reports.binding.yaml",
        FULL_ANNOTATIONS_BINDING,
        "reports:build",
    )
    .expect("register binding");

    let descriptor = registry
        .get_definition("executor.dec002.report")
        .expect("get_definition")
        .expect("registered");
    let ann = descriptor
        .annotations
        .as_ref()
        .expect("descriptor carries annotations");

    assert!(ann.readonly, "readonly");
    assert!(!ann.destructive, "destructive");
    assert!(ann.idempotent, "idempotent");
    assert!(ann.requires_approval, "requires_approval");
    assert!(ann.open_world, "open_world");
    assert!(!ann.streaming, "streaming");
    assert!(ann.cacheable, "cacheable");
    assert_eq!(ann.cache_ttl, 900, "cache_ttl");
    assert_eq!(
        ann.cache_key_fields,
        Some(vec!["tenant_id".to_string(), "period".to_string()]),
        "cache_key_fields"
    );
    assert!(ann.paginated, "paginated");
    assert_eq!(ann.pagination_style, "offset", "pagination_style");
    assert!(!ann.discoverable, "discoverable");

    // §4.4.1 rule 1/2: extension data lives in a nested `extra` object, and the
    // literal key `extra` MUST NOT be nested inside itself.
    assert_eq!(
        ann.extra.get("mcp.category").and_then(Value::as_str),
        Some("reports"),
        "extra must be read as the nested extension map, got {:?}",
        ann.extra
    );
    assert!(
        !ann.extra.contains_key("extra"),
        "`extra` must never be nested under its own key: {:?}",
        ann.extra
    );
}

// DEC-002 (sharpest observable): a binding declaring `streaming: true` for a
// handler that cannot stream is rejected at registration by apcore-python and
// apcore-typescript. It registered cleanly here because `streaming` never
// reached the annotations at all.
#[test]
fn binding_streaming_true_without_stream_is_rejected() {
    let registry = Registry::new();
    let err = register_binding_doc(
        &registry,
        "stream.binding.yaml",
        STREAMING_BINDING,
        "reports:stream",
    )
    .expect_err("a streaming: true binding with no stream() MUST be rejected");
    assert_eq!(
        code_of(&err),
        "STREAMING_INTERFACE_MISMATCH",
        "got {}: {}",
        code_of(&err),
        err.message
    );
}

// ===========================================================================
// 3. D-103 — a null identity stays null
// ===========================================================================

/// Records whether the module saw an identity, and which caller_id.
struct IdentityProbe {
    saw_identity: Arc<AtomicBool>,
    caller_id: Arc<Mutex<Option<String>>>,
}

#[async_trait]
impl Module for IdentityProbe {
    fn input_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn output_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn description(&self) -> &'static str {
        "identity probe"
    }
    async fn execute(&self, _inputs: Value, ctx: &Context<Value>) -> Result<Value, ModuleError> {
        self.saw_identity
            .store(ctx.identity.is_some(), Ordering::SeqCst);
        *self.caller_id.lock().unwrap() = ctx.caller_id.clone();
        Ok(json!({}))
    }
}

// D-103: `@external` is the caller-side ACL sentinel for a null `caller_id`,
// not a principal. An implementation MUST NOT synthesize an `Identity` for a
// call that supplied none — a module written to the spec's own example
// (`if not context.identity: raise "Authentication required"`) must reject the
// call here exactly as it does on the peers.
#[tokio::test]
async fn unauthenticated_call_keeps_identity_null() {
    let saw_identity = Arc::new(AtomicBool::new(false));
    let caller_id = Arc::new(Mutex::new(None));
    let module = IdentityProbe {
        saw_identity: Arc::clone(&saw_identity),
        caller_id: Arc::clone(&caller_id),
    };

    let executor = Executor::new(
        registry_with("executor.d103.probe", Box::new(module)),
        Config::default(),
    );

    executor
        .call("executor.d103.probe", json!({}), None, None)
        .await
        .expect("call succeeds");

    assert!(
        !saw_identity.load(Ordering::SeqCst),
        "a call supplying no identity MUST reach the module with identity == None (D-103)"
    );
    assert_eq!(
        caller_id.lock().unwrap().as_deref(),
        None,
        "a top-level Context carries caller_id = null (§Context.create); `@external` is the \
         ACL's substitution for it, not a value stamped onto the Context — apcore-typescript \
         says so in builtin-steps.ts and apcore-python never writes it either"
    );
}

// ===========================================================================
// 4. D-106 — p99 falls back to the largest finite bucket
// ===========================================================================

// D-106: when every observation overflows the largest finite bucket the
// estimate MUST be that bucket bound, not 0.0. Returning zero reported the
// fastest possible latency for the slowest modules, which disabled latency
// alerting for exactly the modules that should fire it.
#[tokio::test]
async fn p99_falls_back_to_the_largest_finite_bucket() {
    let metrics = MetricsCollector::new();
    // The default bucket ladder tops out at 60s.
    for _ in 0..5 {
        metrics.observe_duration("executor.d106.slow", 120.0);
    }

    let registry = registry_with("executor.d106.slow", Box::new(EchoModule));
    let health = HealthModule::new(
        Arc::clone(&registry),
        Some(metrics.clone()),
        ErrorHistory::new(10),
    );

    let out = health
        .execute(
            json!({ "module_id": "executor.d106.slow" }),
            &fresh_context(),
        )
        .await
        .expect("health.module");

    let p99 = out["p99_latency_ms"].as_f64().expect("p99_latency_ms");
    assert!(
        (p99 - 60_000.0).abs() < 1.0,
        "p99 beyond the top bucket MUST report the largest finite bound (60s -> 60000ms), got {p99}"
    );
}

// D-106 (second half): the snapshot SHOULD carry the `+Inf` bucket so a
// consumer can tell "no data" from "all overflow".
#[test]
fn histogram_snapshot_emits_the_inf_bucket() {
    let metrics = MetricsCollector::new();
    metrics.observe_duration("executor.d106.inf", 120.0);

    let snapshot = metrics.snapshot();
    let histograms = snapshot["histograms"]
        .as_object()
        .expect("histograms object");
    let (_, data) = histograms
        .iter()
        .find(|(k, _)| k.contains("executor.d106.inf"))
        .expect("the module's histogram");
    let buckets = data["buckets"].as_array().expect("buckets array");

    let inf = buckets
        .iter()
        .find(|b| b["le"].as_str() == Some("+Inf"))
        .expect("snapshot MUST carry the +Inf bucket (D-106)");
    assert_eq!(
        inf["count"].as_u64(),
        Some(1),
        "the +Inf bucket counts every observation"
    );
}

// ===========================================================================
// 5. D-104 — a local `#/…` ref resolves against the file root, then the node
// ===========================================================================

const LAYOUT_A: &str = r##"
module_id: a
description: "Layout A — definitions beside input_schema, at the file root"
definitions:
  User:
    type: object
    properties:
      id: { type: string }
input_schema:
  type: object
  properties:
    user:
      $ref: "#/definitions/User"
output_schema:
  type: object
"##;
const LAYOUT_B: &str = r##"
module_id: b
description: "Layout B — $defs nested inside input_schema"
input_schema:
  type: object
  $defs:
    User:
      type: object
      properties:
        id: { type: string }
  properties:
    user:
      $ref: "#/$defs/User"
output_schema:
  type: object
"##;

fn load_schema(dir: &std::path::Path, module_id: &str) -> Result<Value, ModuleError> {
    let mut config = Config::default();
    config.set("schema.root", json!(dir.to_string_lossy()));
    let mut loader = SchemaLoader::with_config(&config, Some(dir));
    loader
        .load(module_id)
        .map(|def| serde_json::to_value(def).expect("serialize SchemaDefinition"))
}

// D-104: BOTH layouts are normative — file root first, falling back to the
// schema node being resolved. Layout B is what apcore-python and
// apcore-typescript accept and is presumably in the wild; it raised
// SCHEMA_NOT_FOUND here.
#[test]
fn both_local_ref_layouts_load() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("a.schema.yaml"), LAYOUT_A).expect("write a");
    std::fs::write(dir.path().join("b.schema.yaml"), LAYOUT_B).expect("write b");

    let a = load_schema(dir.path(), "a").expect("Layout A (file root) must load");
    assert_eq!(
        a["input_schema"]["properties"]["user"]["type"].as_str(),
        Some("object"),
        "Layout A's #/definitions/User must be inlined: {a}"
    );

    let b = load_schema(dir.path(), "b").expect("Layout B (schema node) must load (D-104)");
    assert_eq!(
        b["input_schema"]["properties"]["user"]["type"].as_str(),
        Some("object"),
        "Layout B's nested #/$defs/User must be inlined: {b}"
    );
}

// ===========================================================================
// 6. D-107 — per-class markers are the only multi-class opt-in
// ===========================================================================

// D-107: `DiscoveryConfig.multi_class` was a FILE-LEVEL toggle citing a config
// key decision-log D-06 removed. Per-class markers are authoritative: a file
// whose classes carry the marker derives one ID per class no matter what the
// (retained, inert) DiscoveryConfig says.
#[test]
fn multi_class_opt_in_comes_from_the_per_class_marker() {
    let path = std::path::PathBuf::from("extensions/math/ops.rs");

    let marked = vec![
        DiscoveredClass::new("Addition", true).with_multi_class(true),
        DiscoveredClass::new("Subtraction", true).with_multi_class(true),
    ];
    let mut ids = derive_module_ids(&path, "extensions", &marked, &DiscoveryConfig::default())
        .expect("marked classes derive per-class IDs");
    ids.sort();
    assert_eq!(
        ids,
        vec![
            "math.ops.addition".to_string(),
            "math.ops.subtraction".to_string()
        ],
        "per-class markers opt the file in, with the file-level config left at its default"
    );

    // A single marked class still gets the bare base_id (single-class identity
    // guarantee), and unmarked classes never opt the file in — even when the
    // retired file-level flag is on.
    let unmarked = vec![
        DiscoveredClass::new("Addition", true),
        DiscoveredClass::new("Subtraction", true),
    ];
    let ids = derive_module_ids(
        &path,
        "extensions",
        &unmarked,
        &DiscoveryConfig::with_multi_class(),
    )
    .expect("unmarked classes stay single-class");
    assert_eq!(
        ids,
        vec!["math.ops".to_string()],
        "the file-level toggle MUST NOT gate multi-class discovery (D-107)"
    );
}

// ===========================================================================
// 7 + 8. STR-1 / STR-2 / MW-002 — the streaming tail
// ===========================================================================

/// Non-streaming module whose output violates its own `output_schema`.
struct BadOutputPlainModule;

#[async_trait]
impl Module for BadOutputPlainModule {
    fn input_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn output_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["value"],
            "properties": { "value": { "type": "integer" } }
        })
    }
    fn description(&self) -> &'static str {
        "bad output, no stream()"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({ "value": "not-an-integer" }))
    }
}

/// Non-streaming module producing a schema-valid output.
struct PlainModule;

#[async_trait]
impl Module for PlainModule {
    fn input_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn output_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn description(&self) -> &'static str {
        "no stream()"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({ "value": 1 }))
    }
}

/// Streaming module with an `x-sensitive` output field, used for MW-002.
struct SensitiveStreamModule;

#[async_trait]
impl Module for SensitiveStreamModule {
    fn input_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn output_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "token": { "type": "string", "x-sensitive": true },
                "value": { "type": "integer" }
            }
        })
    }
    fn description(&self) -> &'static str {
        "streams a sensitive field"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({ "token": "s3cret", "value": 1 }))
    }
    fn stream(&self, _inputs: Value, _ctx: &Context<Value>) -> Option<ChunkStream> {
        Some(Box::pin(stream! {
            yield Ok(json!({ "token": "s3cret" }));
            yield Ok(json!({ "value": 1 }));
        }))
    }
}

/// Rewrites the output in `after()`, and records what it saw.
#[derive(Debug)]
struct AfterMiddleware {
    after_calls: Arc<AtomicUsize>,
    redacted_output: Arc<Mutex<Option<Value>>>,
}

#[async_trait]
impl Middleware for AfterMiddleware {
    fn name(&self) -> &str {
        "after-probe"
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
        output: Value,
        ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        self.after_calls.fetch_add(1, Ordering::SeqCst);
        *self.redacted_output.lock().unwrap() = ctx
            .redacted_output
            .as_ref()
            .map(|m| Value::Object(m.clone().into_iter().collect()));
        let mut out = output;
        out["decorated"] = json!(true);
        Ok(Some(out))
    }
    async fn on_error(
        &self,
        _module_id: &str,
        _inputs: Value,
        _error: &ModuleError,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        Ok(None)
    }
}

async fn drain(
    executor: &Executor,
    module_id: &str,
    inputs: Value,
) -> Vec<Result<Value, ModuleError>> {
    let mut items = Vec::new();
    let mut s = executor.stream(module_id, inputs, None, None);
    while let Some(item) = s.next().await {
        items.push(item);
    }
    items
}

// STR-1: for a module with NO `stream()`, the fallback runs `execute()` and
// must then run the SAME pipeline tail `call()` runs. apcore-python and
// apcore-typescript run the full strategy in Phase 1, so an output violating
// `output_schema` fails the call and yields nothing. The "chunks already
// delivered, cannot un-send" rationale does not apply here — nothing has been
// delivered yet.
#[tokio::test]
async fn stream_fallback_validates_output() {
    let executor = Executor::new(
        registry_with("executor.str1.bad", Box::new(BadOutputPlainModule)),
        Config::default(),
    );

    let items = drain(&executor, "executor.str1.bad", json!({})).await;

    assert!(
        items.iter().all(std::result::Result::is_err),
        "an output violating output_schema MUST fail the call and yield no chunk (STR-1); got {:?}",
        items
            .iter()
            .map(|i| i.as_ref().map(std::string::ToString::to_string))
            .collect::<Vec<_>>()
    );
    let err = items
        .into_iter()
        .find_map(std::result::Result::err)
        .expect("an error item");
    assert_eq!(code_of(&err), "SCHEMA_VALIDATION_ERROR", "{}", err.message);
}

// STR-1 (second half): an after-middleware that transforms the output is
// reflected in what the fallback yields, because the tail actually runs.
#[tokio::test]
async fn stream_fallback_yields_the_after_middleware_output() {
    let after_calls = Arc::new(AtomicUsize::new(0));
    let executor = Executor::new(
        registry_with("executor.str1.plain", Box::new(PlainModule)),
        Config::default(),
    );
    executor
        .use_middleware(Box::new(AfterMiddleware {
            after_calls: Arc::clone(&after_calls),
            redacted_output: Arc::new(Mutex::new(None)),
        }))
        .expect("add middleware");

    let items = drain(&executor, "executor.str1.plain", json!({})).await;
    assert_eq!(items.len(), 1, "one fallback chunk");
    let chunk = items[0].as_ref().expect("chunk").clone();
    assert_eq!(
        chunk["decorated"].as_bool(),
        Some(true),
        "the fallback MUST yield the after-middleware's output (STR-1): {chunk}"
    );
    assert_eq!(
        after_calls.load(Ordering::SeqCst),
        1,
        "after() runs exactly once on the fallback path"
    );
}

// STR-2: Phase 3 must run the ACTUAL strategy's post steps. On the `minimal`
// preset both `output_validation` and `middleware_after` are removed, so a
// streamed call must neither validate nor fire after() — `call()` on the same
// preset skips both, and the two paths may not disagree.
#[tokio::test]
async fn stream_phase3_honours_the_strategy() {
    let after_calls = Arc::new(AtomicUsize::new(0));
    let executor = Executor::with_strategy(
        registry_with("executor.str2.stream", Box::new(SensitiveStreamModule)),
        Config::default(),
        build_minimal_strategy(),
    );
    executor
        .use_middleware(Box::new(AfterMiddleware {
            after_calls: Arc::clone(&after_calls),
            redacted_output: Arc::new(Mutex::new(None)),
        }))
        .expect("add middleware");

    let items = drain(&executor, "executor.str2.stream", json!({})).await;
    assert!(
        items.iter().all(std::result::Result::is_ok),
        "the stream itself still succeeds"
    );
    assert_eq!(
        after_calls.load(Ordering::SeqCst),
        0,
        "`minimal` removes middleware_after, so a streamed call MUST NOT fire after() (STR-2)"
    );
}

// MW-002: on the standard strategy, the streamed call's post steps populate
// `context.redacted_output`, so a streamed call's audit record carries the same
// output projection its non-streamed twin does.
#[tokio::test]
async fn streamed_call_populates_redacted_output() {
    let redacted = Arc::new(Mutex::new(None));
    let executor = Executor::new(
        registry_with("executor.mw002.stream", Box::new(SensitiveStreamModule)),
        Config::default(),
    );
    executor
        .use_middleware(Box::new(AfterMiddleware {
            after_calls: Arc::new(AtomicUsize::new(0)),
            redacted_output: Arc::clone(&redacted),
        }))
        .expect("add middleware");

    let items = drain(&executor, "executor.mw002.stream", json!({})).await;
    assert!(items.iter().all(std::result::Result::is_ok), "{items:?}");

    let seen = redacted.lock().unwrap().clone();
    let seen = seen.expect("a streamed call MUST populate context.redacted_output (MW-002)");
    assert_eq!(
        seen["token"].as_str(),
        Some(apcore::REDACTED_VALUE),
        "the x-sensitive field is projected out: {seen}"
    );
}

// ===========================================================================
// 9. ERR-001 — the span attribute carries the canonical wire code
// ===========================================================================

/// Fails with a fixed, recognisable error code.
struct FailingModule;

#[async_trait]
impl Module for FailingModule {
    fn input_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn output_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn description(&self) -> &'static str {
        "always fails"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Err(ModuleError::new(ErrorCode::ModuleTimeout, "too slow"))
    }
}

// ERR-001: apcore-python (`tracing.py`) and apcore-typescript (`tracing.ts`)
// both write `error.code` — the protocol's SCREAMING_SNAKE wire code — into the
// span's `error_code` attribute. Rust wrote the Rust enum's PascalCase `Debug`
// name, so an OTLP query keyed on the protocol code matched two SDKs and never
// this one.
#[tokio::test]
async fn span_error_code_is_the_wire_code() {
    let exporter = InMemoryExporter::new();
    let executor = Executor::new(
        registry_with("executor.err001.fail", Box::new(FailingModule)),
        Config::default(),
    );
    executor
        .use_middleware(Box::new(TracingMiddleware::new(Box::new(exporter.clone()))))
        .expect("add tracing middleware");

    let _ = executor
        .call("executor.err001.fail", json!({}), None, None)
        .await;

    let spans = exporter.get_spans();
    let span = spans.last().expect("a span was exported for the failure");
    assert_eq!(
        span.attributes.get("error_code").and_then(Value::as_str),
        Some("MODULE_TIMEOUT"),
        "the span attribute MUST carry the wire code, not the Rust Debug name: {:?}",
        span.attributes.get("error_code")
    );
}

// ===========================================================================
// 10. OBS-002 — Prometheus label values are escaped
// ===========================================================================

// OBS-002: a label value containing `"` emits a malformed exposition line,
// which makes Prometheus reject the ENTIRE scrape — dropping every other metric
// with it. apcore-python (`_escape_label_value`) and apcore-typescript
// (`escapeLabelValue`) both escape backslash, double-quote and newline.
#[test]
fn prometheus_label_values_are_escaped() {
    let metrics = MetricsCollector::new();
    let mut labels = HashMap::new();
    labels.insert(
        "module_id".to_string(),
        "quote\"back\\slash\nnewline".to_string(),
    );
    metrics.increment("apcore_module_calls_total", labels, 1.0);

    let text = metrics.export_prometheus();
    let line = text
        .lines()
        .find(|l| l.starts_with("apcore_module_calls_total{"))
        .expect("the counter line");

    assert!(
        line.contains("quote\\\"back\\\\slash\\nnewline"),
        "backslash, double-quote and newline MUST be escaped (OBS-002): {line}"
    );
    assert_eq!(
        line.matches('"').count() - line.matches("\\\"").count(),
        2,
        "exactly one unescaped quote pair delimits the label value: {line}"
    );
}

// ===========================================================================
// 11. ERR-003 / ERR-004 — recovery metadata
// ===========================================================================

// ERR-003: `TASK_LIMIT_EXCEEDED` resolves to `retryable: true` in apcore-python
// (`_default_retryable = True`) and apcore-typescript (`DEFAULT_RETRYABLE`).
// It was in neither arm here, so it fell to `None` and `RetryMiddleware` — which
// gates on `retryable == Some(true)` in all three — never retried a full task
// pool that the peers do retry.
#[test]
fn task_limit_exceeded_is_retryable() {
    assert_eq!(
        retryable_for_code(ErrorCode::TaskLimitExceeded),
        Some(true),
        "TASK_LIMIT_EXCEEDED MUST resolve to retryable: true (ERR-003)"
    );
}

// ERR-004: apcore-python and apcore-typescript attach a default `ai_guidance`
// to these five codes; Rust built all five with a bare `ModuleError::new`,
// while already supplying guidance for eight other codes.
#[test]
fn the_five_recovery_codes_carry_ai_guidance() {
    let cases = vec![
        ModuleError::module_not_found("executor.missing"),
        ModuleError::module_disabled("executor.off"),
        ModuleError::module_timeout("executor.slow", 1_000),
        ModuleError::dependency_not_found("executor.a", "executor.b"),
        ModuleError::dependency_version_mismatch("executor.a", "executor.b", ">=2.0.0", "1.0.0"),
    ];
    for err in cases {
        let guidance = err
            .ai_guidance
            .as_deref()
            .unwrap_or_else(|| panic!("{:?} MUST carry ai_guidance (ERR-004)", err.code));
        assert!(
            !guidance.is_empty(),
            "{:?} guidance must not be empty",
            err.code
        );
    }
}

// ERR-004 (routed): the raise sites go through the builders, so a real lookup
// failure carries the guidance rather than only the hand-built error doing so.
#[test]
fn registry_lookup_failure_carries_ai_guidance() {
    let registry = Registry::new();
    let err = registry
        .describe("executor.nope")
        .expect_err("unknown module");
    assert_eq!(code_of(&err), "MODULE_NOT_FOUND");
    assert!(
        err.ai_guidance
            .as_deref()
            .is_some_and(|g| g.contains("registry")),
        "Registry::describe's MODULE_NOT_FOUND MUST carry guidance: {:?}",
        err.ai_guidance
    );
}

// ===========================================================================
// 12. SYS-* — manifest, health and control parity
// ===========================================================================

struct TaggedModule;

#[async_trait]
impl Module for TaggedModule {
    fn input_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn output_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn description(&self) -> &'static str {
        "tagged"
    }
    fn tags(&self) -> Vec<String> {
        vec!["instance-tag".to_string()]
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
}

fn descriptor_for(module_id: &str) -> ModuleDescriptor {
    ModuleDescriptor {
        module_id: module_id.to_string(),
        name: None,
        description: "documented module".to_string(),
        documentation: Some("Long-form **docs**.".to_string()),
        input_schema: json!({ "type": "object" }),
        output_schema: json!({ "type": "object" }),
        version: "1.0.0".to_string(),
        tags: vec!["reports".to_string()],
        annotations: Some(ModuleAnnotations::default()),
        examples: vec![],
        metadata: HashMap::from([("owner".to_string(), json!("platform"))]),
        display: None,
        sunset_date: None,
        dependencies: vec![DependencyInfo {
            module_id: "common.helpers".to_string(),
            version_constraint: ">=1.0.0".to_string(),
            optional: false,
        }],
        enabled: true,
    }
}

fn manifest_registry(module_id: &str) -> Arc<Registry> {
    let registry = Registry::new();
    registry
        .register(module_id, Box::new(EchoModule), descriptor_for(module_id))
        .expect("register with descriptor");
    Arc::new(registry)
}

fn empty_config() -> Arc<tokio::sync::Mutex<Config>> {
    Arc::new(tokio::sync::Mutex::new(Config::default()))
}

// SYS-6 / SYS-7 / SYS-12: `manifest.module` reads the DESCRIPTOR, which carries
// documentation and metadata; `source_path` is null when `project.source_root`
// is unset (the schema types it `["string","null"]`); and a dependency's
// version key is spelled `version`, as in the peers and the metadata YAML.
#[tokio::test]
async fn manifest_module_reports_documentation_metadata_and_null_source_path() {
    let registry = manifest_registry("executor.sys.doc");
    let manifest = ManifestModule::new(registry, empty_config());

    let out = manifest
        .execute(json!({ "module_id": "executor.sys.doc" }), &fresh_context())
        .await
        .expect("manifest.module");

    assert_eq!(
        out["documentation"].as_str(),
        Some("Long-form **docs**."),
        "documentation comes from the descriptor (SYS-6): {out}"
    );
    assert_eq!(
        out["metadata"]["owner"].as_str(),
        Some("platform"),
        "metadata comes from the descriptor (SYS-6): {out}"
    );
    assert!(
        out["source_path"].is_null(),
        "source_path is null when project.source_root is unset (SYS-7): {out}"
    );
    let dep = &out["dependencies"][0];
    assert_eq!(
        dep["version"].as_str(),
        Some(">=1.0.0"),
        "a dependency's constraint is spelled `version` (SYS-12): {dep}"
    );
    assert!(
        dep.get("version_constraint").is_none(),
        "the Rust-internal field name MUST NOT reach the wire (SYS-12): {dep}"
    );
}

// SYS-8 / SYS-9: a `manifest.full` entry is field-for-field `manifest.module`'s
// output (sys-manifest-full.schema.json asserts it by `$ref`), and the
// omittable keys are emitted as null rather than dropped.
#[tokio::test]
async fn manifest_full_entries_match_manifest_module() {
    let registry = manifest_registry("executor.sys.full");
    let full = ManifestFullModule::new(registry, empty_config());

    let out = full
        .execute(json!({}), &fresh_context())
        .await
        .expect("manifest.full");
    let entry = &out["modules"][0];
    assert_eq!(
        entry["documentation"].as_str(),
        Some("Long-form **docs**."),
        "manifest.full entries carry documentation (SYS-8): {entry}"
    );
    assert_eq!(
        entry["metadata"]["owner"].as_str(),
        Some("platform"),
        "manifest.full entries carry metadata (SYS-8): {entry}"
    );

    let out = full
        .execute(
            json!({ "include_schemas": false, "include_source_paths": false }),
            &fresh_context(),
        )
        .await
        .expect("manifest.full");
    let entry = &out["modules"][0];
    for key in ["input_schema", "output_schema", "source_path"] {
        assert!(
            entry.get(key).is_some() && entry[key].is_null(),
            "{key} MUST be emitted as null when excluded (SYS-9): {entry}"
        );
    }
}

// SYS-11: tag filtering delegates to `Registry::list(tags)`, which unions the
// descriptor's tags with the live instance's `tags()`. Re-implementing the
// filter against `descriptor.tags` bypassed that union.
#[tokio::test]
async fn manifest_full_tag_filter_sees_instance_tags() {
    let registry = Registry::new();
    // Registered with an explicit descriptor that declares NO tags, while the
    // live instance declares one. `Registry::list` unions the two (D11-003);
    // a filter written against `descriptor.tags` sees nothing.
    let mut descriptor = descriptor_for("executor.sys.tagged");
    descriptor.tags = vec![];
    registry
        .register("executor.sys.tagged", Box::new(TaggedModule), descriptor)
        .expect("register");
    let full = ManifestFullModule::new(Arc::new(registry), empty_config());

    let out = full
        .execute(json!({ "tags": ["instance-tag"] }), &fresh_context())
        .await
        .expect("manifest.full");

    assert_eq!(
        out["module_count"].as_u64(),
        Some(1),
        "a module declaring tags() on the instance MUST be matched (SYS-11): {out}"
    );
}

// SYS-4: `top_error` is the most FREQUENT error, as in apcore-python
// (`max(entries, key=lambda e: e.count)`) — not the most recent. The field is
// named top_error.
#[tokio::test]
async fn health_summary_top_error_is_the_most_frequent() {
    let history = ErrorHistory::new(10);
    let frequent = ModuleError::new(ErrorCode::ModuleTimeout, "frequent failure");
    for _ in 0..3 {
        history.record("executor.sys.health", &frequent);
    }
    // Recorded last, so it is the most RECENT but not the most frequent.
    history.record(
        "executor.sys.health",
        &ModuleError::new(ErrorCode::GeneralInvalidInput, "one-off"),
    );

    let registry = registry_with("executor.sys.health", Box::new(EchoModule));
    let summary = HealthSummaryModule::new(registry, None, history, empty_config());

    let out = summary
        .execute(json!({}), &fresh_context())
        .await
        .expect("health.summary");
    let entry = out["modules"]
        .as_array()
        .expect("modules")
        .iter()
        .find(|m| m["module_id"].as_str() == Some("executor.sys.health"))
        .expect("the module's entry");

    assert_eq!(
        entry["top_error"]["message"].as_str(),
        Some("frequent failure"),
        "top_error MUST be the most frequent error (SYS-4): {entry}"
    );
    assert_eq!(entry["top_error"]["count"].as_u64(), Some(3));
}

// SYS-24: the sys modules' `output_schema` MUST declare the full field
// contract (PROTOCOL_SPEC §6.7.1.6), as the canonical schemas do and as
// `usage.rs` already did.
#[test]
fn sys_module_output_schemas_declare_their_contract() {
    let registry = Arc::new(Registry::new());
    let cases: Vec<(&str, Value, Vec<&str>)> = vec![
        (
            "system.health.summary",
            HealthSummaryModule::new(
                Arc::clone(&registry),
                None,
                ErrorHistory::new(10),
                empty_config(),
            )
            .output_schema(),
            vec!["project", "summary", "modules"],
        ),
        (
            "system.health.module",
            HealthModule::new(Arc::clone(&registry), None, ErrorHistory::new(10)).output_schema(),
            vec![
                "module_id",
                "status",
                "total_calls",
                "error_count",
                "error_rate",
            ],
        ),
        (
            "system.manifest.module",
            ManifestModule::new(Arc::clone(&registry), empty_config()).output_schema(),
            vec!["module_id", "description"],
        ),
        (
            "system.manifest.full",
            ManifestFullModule::new(Arc::clone(&registry), empty_config()).output_schema(),
            vec!["project_name", "module_count", "modules"],
        ),
    ];

    for (name, schema, required) in cases {
        let props = schema["properties"]
            .as_object()
            .unwrap_or_else(|| panic!("{name} output_schema MUST declare properties (SYS-24)"));
        for key in required {
            assert!(
                props.contains_key(key),
                "{name} output_schema must declare `{key}` (SYS-24)"
            );
        }
    }
}

// SYS-15: the bulk reload path emitted `apcore.module.reloaded` with the
// literal "unknown" for both version fields, while its own single-module path
// emitted real versions.
#[derive(Debug)]
struct ReloadCapture {
    events: Arc<Mutex<Vec<Value>>>,
}

#[async_trait]
impl EventSubscriber for ReloadCapture {
    fn subscriber_id(&self) -> &str {
        "sys15-capture"
    }
    fn event_pattern(&self) -> &str {
        "apcore.module.reloaded"
    }
    async fn on_event(&self, event: &ApCoreEvent) -> Result<(), ModuleError> {
        self.events.lock().unwrap().push(event.data.clone());
        Ok(())
    }
}

struct VersionedDiscoverer {
    module_id: String,
    version: String,
}

#[async_trait]
impl Discoverer for VersionedDiscoverer {
    async fn discover(&self, _roots: &[String]) -> Result<Vec<DiscoveredModule>, ModuleError> {
        let mut descriptor = descriptor_for(&self.module_id);
        descriptor.version = self.version.clone();
        Ok(vec![DiscoveredModule {
            name: self.module_id.clone(),
            source: "test".to_string(),
            descriptor,
            module: Arc::new(EchoModule),
        }])
    }
}

#[tokio::test]
async fn bulk_reload_emits_real_versions() {
    let registry = Arc::new(Registry::new());
    registry
        .register_internal(
            "executor.sys.reload",
            Box::new(EchoModule),
            descriptor_for("executor.sys.reload"),
        )
        .expect("register");
    registry.set_discoverer(Box::new(VersionedDiscoverer {
        module_id: "executor.sys.reload".to_string(),
        version: "2.0.0".to_string(),
    }));

    let emitter = Arc::new(EventEmitter::new());
    let events = Arc::new(Mutex::new(Vec::new()));
    emitter.subscribe(Box::new(ReloadCapture {
        events: Arc::clone(&events),
    }));

    let reload = ReloadModule::new(Arc::clone(&registry), Arc::clone(&emitter));
    reload
        .execute(
            json!({ "path_filter": "executor.sys.*", "reason": "bulk" }),
            &fresh_context(),
        )
        .await
        .expect("bulk reload");
    // `EventEmitter::emit` spawns per-subscriber delivery tasks.
    emitter.flush_default().await.expect("flush events");

    let captured = events.lock().unwrap().clone();
    let event = captured
        .first()
        .expect("apcore.module.reloaded was emitted");
    assert_eq!(
        event["previous_version"].as_str(),
        Some("1.0.0"),
        "the bulk path MUST carry the pre-unregister version (SYS-15): {event}"
    );
    assert_eq!(
        event["new_version"].as_str(),
        Some("2.0.0"),
        "the bulk path MUST carry the re-discovered version (SYS-15): {event}"
    );
}
