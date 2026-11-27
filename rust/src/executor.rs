// APCore Protocol — Executor
// Spec reference: Module execution engine

mod schema_redaction;

use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::{Arc, LazyLock};

use futures_core::Stream;
use futures_util::StreamExt;
use serde_json::Value;

use crate::acl::ACL;
use crate::approval::ApprovalHandler;
use crate::builtin_steps::{
    build_internal_strategy, build_minimal_strategy, build_performance_strategy,
    build_standard_strategy, build_testing_strategy, BuiltinPostValidationPolicy,
};
use crate::cancel::CancelToken;
use crate::config::Config;
use crate::context::Context;
use crate::errors::{ErrorCode, ModuleError};
use crate::events::emitter::{ApCoreEvent, EventEmitter};
use crate::middleware::adapters::{AfterMiddleware, BeforeMiddleware};
use crate::middleware::base::{Middleware, OnErrorOutcome};
use crate::middleware::manager::{MiddlewareHandle, MiddlewareManager};
use crate::module::PreflightCheckResult as PfCheck;
use crate::module::{PreflightCheckResult, PreflightResult};
use crate::observability::redaction::RedactionConfig;
use crate::pipeline::{
    BuiltinGate, ExecutionStrategy, PipelineContext, PipelineEngine, PipelineTrace, RunOptions,
    StepTrace, StrategyInfo,
};
use crate::policy::{ExecutionPolicy, PolicyEffect, PolicyEvaluator};
use crate::registry::registry::{module_id_pattern, ModuleExecutionLease, Registry};
use crate::runtime::background_runtime;
use crate::stream_completion::{StreamChunks, StreamCompletionHandle, StreamRecorder};
use crate::utils::propagate_module_error;

/// Maximum nesting depth for deep_merge_value to prevent stack overflow on
/// adversarial or pathological inputs.
///
/// Aligned with apcore-python and apcore-typescript (32) per spec (sync STREAM-001).
const DEEP_MERGE_MAX_DEPTH: usize = 32;

/// Resolve `stream.max_merge_depth` (PROTOCOL_SPEC §5; canonical default 32).
///
/// §5 calls 32 the *canonical default*, which implies an override — and until
/// now there was none: the key was declared in all three key surfaces, carried
/// a canonical default in `defaults.schema.json`, and was read by no code path
/// (apcore#118), so the constant WAS the contract.
///
/// A non-positive or non-integer value falls back to the canonical default
/// rather than disabling the cap. The cap exists to prevent stack exhaustion
/// from adversarial chunk shapes, so a misconfiguration must not remove it.
#[must_use]
pub fn resolve_merge_depth(config: &Config) -> usize {
    config
        .get("stream.max_merge_depth")
        .and_then(|v| v.as_u64())
        .filter(|n| *n >= 1)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(DEEP_MERGE_MAX_DEPTH)
}

/// Deep-merge a list of JSON Value chunks into a single accumulated Value.
///
/// Retained for tests of the unchecked merge behavior. The streaming pipeline
/// checks each chunk's shape before incrementally merging it (D-58).
#[cfg(test)]
fn deep_merge_chunks(chunks: &[Value]) -> Value {
    let mut acc = Value::Null;
    for chunk in chunks {
        deep_merge_value(&mut acc, chunk, 0, DEEP_MERGE_MAX_DEPTH);
    }
    acc
}

/// Sync finding D-58: deep-merge chunks while *enforcing* that every chunk is
/// a JSON object. A non-object chunk (string, number, array, etc.) yields
/// `ModuleError::GeneralInvalidInput` with `details["code"] =
/// "STREAM_CHUNK_NOT_OBJECT"`, mirroring Python's `_deep_merge` AttributeError
/// and TypeScript's TypeError. The previous Rust path silently replaced the
/// accumulator with a non-object, leaving downstream consumers staring at
/// surprise scalars.
///
/// `chunks` may be empty; in that case the function returns an empty object
/// (mirroring Python's `accumulated: dict[str, Any] = {}` initial state).
pub fn deep_merge_chunks_checked(chunks: &[Value]) -> Result<Value, ModuleError> {
    deep_merge_chunks_checked_with_depth(chunks, DEEP_MERGE_MAX_DEPTH)
}

/// [`deep_merge_chunks_checked`] with an explicit depth cap.
///
/// The cap is `stream.max_merge_depth` (PROTOCOL_SPEC §5), resolved by
/// [`resolve_merge_depth`]. The no-cap entry point above keeps the canonical
/// default so every existing caller — tests and the conformance driver
/// included — is unaffected.
pub fn deep_merge_chunks_checked_with_depth(
    chunks: &[Value],
    max_depth: usize,
) -> Result<Value, ModuleError> {
    let mut acc: Value = Value::Object(serde_json::Map::new());
    for (idx, chunk) in chunks.iter().enumerate() {
        if !chunk.is_object() {
            return Err(stream_chunk_not_object_error(idx, chunk));
        }
        deep_merge_value(&mut acc, chunk, 0, max_depth);
    }
    Ok(acc)
}

/// Build the canonical "chunk is not a JSON object" error (audit D10-001 /
/// sync finding D-58). Shared by Phase 2 reject-before-deliver in
/// [`Executor::stream`] and by [`deep_merge_chunks_checked`] so both surface an
/// identical `code` / `message` / `details` shape that cross-language consumers
/// can match (mirrors Python's `_deep_merge` AttributeError and TS's TypeError).
pub fn stream_chunk_not_object_error(idx: usize, chunk: &Value) -> ModuleError {
    let mut details = std::collections::HashMap::new();
    details.insert(
        "code".to_string(),
        Value::String("STREAM_CHUNK_NOT_OBJECT".to_string()),
    );
    details.insert(
        "chunk_index".to_string(),
        Value::Number(serde_json::Number::from(idx)),
    );
    details.insert(
        "actual_type".to_string(),
        Value::String(json_type_name(chunk).to_string()),
    );
    ModuleError::new(
        ErrorCode::GeneralInvalidInput,
        format!(
            "Streaming chunk at index {idx} is not a JSON object \
             (got {}); chunks must be objects so deep_merge can \
             accumulate them.",
            json_type_name(chunk)
        ),
    )
    .with_details(details)
}

fn json_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn deep_merge_value(base: &mut Value, overlay: &Value, depth: usize, max_depth: usize) {
    if depth >= max_depth {
        // At the depth limit, stop recursing (stack safety) but still merge
        // SHALLOWLY: assign each overlay key onto base rather than replacing
        // the whole node. Replacing dropped every base-only key, so streaming
        // chunk A `{a:{a:…{x:1}}}` followed by chunk B `{a:{a:…{y:2}}}` at
        // depth 32 accumulated to `{y:2}` in Rust where apcore-python
        // (`executor.py` `_deep_merge`) and apcore-typescript
        // (`executor.ts` `deepMerge`) both produce `{x:1,y:2}`. Machine-generated
        // deeply nested payloads are exactly where streaming accumulators hit
        // this cap.
        match (base, overlay) {
            (Value::Object(base_map), Value::Object(overlay_map)) => {
                for (k, v) in overlay_map {
                    base_map.insert(k.clone(), v.clone());
                }
            }
            (base, overlay) => {
                *base = overlay.clone();
            }
        }
        return;
    }
    match (base, overlay) {
        (Value::Object(base_map), Value::Object(overlay_map)) => {
            for (k, v) in overlay_map {
                let entry = base_map.entry(k.clone()).or_insert(Value::Null);
                deep_merge_value(entry, v, depth + 1, max_depth);
            }
        }
        (base, overlay) => {
            *base = overlay.clone();
        }
    }
}

/// Resolve a preset strategy by name.
///
/// Built-in names: `"standard"`, `"internal"`, `"testing"`, `"performance"`, `"minimal"`.
pub fn resolve_strategy_by_name(name: &str) -> Result<ExecutionStrategy, ModuleError> {
    match name {
        "standard" => Ok(build_standard_strategy()),
        "internal" => Ok(build_internal_strategy()),
        "testing" => Ok(build_testing_strategy()),
        "performance" => Ok(build_performance_strategy()),
        "minimal" => Ok(build_minimal_strategy()),
        _ => Err(ModuleError::strategy_not_found(format!(
            "Unknown strategy name '{name}'. Built-in presets: standard, internal, testing, performance, minimal"
        ))),
    }
}

/// Map pipeline step names to PreflightResult check names.
fn step_to_check_name(step_name: &str) -> &str {
    match step_name {
        "context_creation" => "context",
        "call_chain_guard" => "call_chain",
        "module_lookup" => "module_lookup",
        "acl_check" => crate::module::ACL_CHECK_NAME,
        "approval_gate" => "approval",
        "middleware_before" => "middleware",
        "input_validation" => "schema",
        other => other,
    }
}

/// Convert `PipelineTrace` steps into `PreflightCheckResult` entries.
fn trace_to_checks(
    steps: &[StepTrace],
    preflight_errors: &[(String, ModuleError)],
) -> Vec<PfCheck> {
    steps
        .iter()
        .filter(|st| !st.skipped)
        .map(|st| {
            let check_name = step_to_check_name(&st.name).to_string();
            let passed = st.result.action != "abort";
            let error = if passed {
                None
            } else {
                preflight_errors
                    .iter()
                    .find(|(step, _)| step == &st.name)
                    .map(|(_, error)| error.to_dict())
                    .or_else(|| {
                        st.result.explanation.as_ref().map(|msg| {
                            serde_json::json!({
                                "code": format!("STEP_{}_FAILED", st.name.to_uppercase()),
                                "message": msg,
                            })
                        })
                    })
            };
            PfCheck {
                check: check_name,
                passed,
                error,
                warnings: vec![],
            }
        })
        .collect()
}

/// Returns true if the schema is non-trivial (not null and not an empty object).
pub fn has_schema(schema: &Value) -> bool {
    if schema.is_null() {
        return false;
    }
    if let Some(obj) = schema.as_object() {
        return !obj.is_empty();
    }
    true
}

/// Sentinel value used to replace sensitive fields in redacted output.
pub const REDACTED_VALUE: &str = "***REDACTED***";

/// Internal: context carried from stream setup through post-processing.
struct StreamSetup {
    module: Arc<dyn crate::module::Module>,
    /// The same pipeline context serves every phase and retains its module lease.
    pipe_ctx: PipelineContext,
    /// The target module, for decorating errors raised after Phase 1.
    module_id: String,
    /// `stream.max_merge_depth` resolved once at setup (PROTOCOL_SPEC §5).
    /// Carried on the setup rather than re-read in Phase 3 so one stream uses
    /// one cap even if the configuration is mutated mid-stream.
    merge_depth: usize,
}

impl StreamSetup {
    fn context(&self) -> &Context<Value> {
        &self.pipe_ctx.context
    }

    fn event_emitter(&self) -> Option<&Arc<EventEmitter>> {
        self.pipe_ctx.event_emitter.as_ref()
    }

    fn settle_retiring_producer(&self, producer: crate::module::ChunkStream, error: &ModuleError) {
        let Some(token) = self.context().cancel_token.as_ref().filter(|token| {
            error.code == ErrorCode::ExecutionCancelled && token.retirement_requested()
        }) else {
            return;
        };
        // A retirement owner uses this time-enabled worker to signal the token.
        let Ok(runtime) = background_runtime() else {
            return;
        };
        let execution = RetiringProducer {
            producer,
            _lease: self.pipe_ctx.module_lease.clone(),
        };
        token.spawn_tracked_on(&runtime, execution.settle());
    }
}

struct RetiringProducer {
    // Drop the producer before releasing the generation's lease.
    producer: crate::module::ChunkStream,
    _lease: Option<Arc<ModuleExecutionLease>>,
}

impl RetiringProducer {
    async fn settle(mut self) {
        while self.producer.next().await.is_some() {
            // Cleanup may produce output. The cancelled caller has already left.
            tokio::task::yield_now().await;
        }
    }
}

/// Internal: outcome of `Executor::prepare_stream`. A Phase-1 (pre-execute)
/// pipeline failure can be intercepted by a middleware `on_error` handler that
/// supplies a recovery value — mirroring `call()` and the Python/TypeScript
/// SDKs. In that case the streaming body must YIELD the recovery value as the
/// stream's chunk rather than surfacing the error (sync finding A-D-001).
enum StreamPrep {
    /// Phase 1 succeeded; proceed with the selected strategy.
    Setup(Box<StreamSetup>),
    /// A middleware `on_error` handler recovered from a Phase-1 failure; yield
    /// this value as the single stream chunk, then end the stream.
    Recovery(Value, Box<ModuleError>),
}

/// Validate a JSON value against a JSON Schema.
///
/// This is the module-invocation boundary — `builtin_steps.rs` calls it for both
/// input and output. It performs **no type coercion**, under any host
/// configuration: a contract that declares `integer` receives an integer, and
/// `"42"` is a type error (TYPE_MAPPING §17.3). `SchemaValidator::new()` agrees;
/// its `coerce_types` default is `false` for exactly that reason.
///
/// Returns `Ok(())` when the value is valid. On failure the [`ModuleError`]
/// carries `SCHEMA_VALIDATION_ERROR` when the *value* is at fault, or
/// `SCHEMA_PARSE_ERROR` when the *schema* could not be compiled; both spell
/// their failures out in `details.errors`.
pub fn validate_against_schema(
    value: &Value,
    schema: &Value,
    direction: &str,
) -> Result<(), ModuleError> {
    static VALIDATOR: LazyLock<crate::schema::SchemaValidator> =
        LazyLock::new(crate::schema::SchemaValidator::new);
    // If schema is null/empty, skip validation
    if !has_schema(schema) {
        return Ok(());
    }

    // Compiled through the same builder `SchemaValidator` uses, so the executor
    // and the validator can never reach opposite verdicts on one input: the
    // document's own draft is honoured and `format` stays an annotation.
    let validator = match VALIDATOR.compile_cached(schema) {
        Ok(v) => v,
        Err(e) => {
            // The *schema* is broken, not the value. SCHEMA_VALIDATION_ERROR is
            // flagged caller-fixable (`user_fixable_for_code`), which would send
            // the caller off to edit arguments that were never at fault, so this
            // branch reports SCHEMA_PARSE_ERROR — the same code
            // `SchemaValidator::validate_detailed_raw` uses for a failed compile.
            let message = format!("{direction} schema is invalid: {e}");
            let mut entry = HashMap::new();
            entry.insert("field".to_string(), String::new());
            entry.insert("message".to_string(), message.clone());
            let mut details = HashMap::new();
            details.insert(
                "errors".to_string(),
                Value::Array(vec![
                    // INVARIANT: HashMap<String, String> always serializes to a JSON object.
                    serde_json::to_value(&entry)
                        .expect("HashMap<String, String> serialization is infallible"),
                ]),
            );
            return Err(ModuleError::new(ErrorCode::SchemaParseError, message)
                .with_details(details)
                .with_ai_guidance(format!(
                    "The {direction} schema itself could not be compiled. Fix the module's schema declaration; the call arguments are not the cause."
                )));
        }
    };

    if validator.is_valid(value) {
        return Ok(());
    }

    let error_list: Vec<HashMap<String, String>> = validator
        .iter_errors(value)
        .map(|e| {
            let mut map = HashMap::new();
            map.insert("path".to_string(), e.instance_path.to_string());
            map.insert(
                "keyword".to_string(),
                crate::schema::validator::constraint_name(&e.kind).unwrap_or_else(|| {
                    e.schema_path
                        .to_string()
                        .rsplit('/')
                        .next()
                        .unwrap_or("schema")
                        .to_string()
                }),
            );
            map.insert(
                "message".to_string(),
                crate::schema::validator::safe_validation_message(&e),
            );
            map
        })
        .collect();

    let errors_json: Vec<Value> = error_list
        .iter()
        .map(|e| {
            // INVARIANT: HashMap<String, String> always serializes to a JSON object; to_value is infallible.
            serde_json::to_value(e).expect("HashMap<String, String> serialization is infallible")
        })
        .collect();
    let mut details = HashMap::new();
    details.insert("errors".to_string(), Value::Array(errors_json));

    Err(ModuleError::new(
        ErrorCode::SchemaValidationError,
        format!("{direction} validation failed"),
    )
    .with_details(details)
    .with_ai_guidance(format!(
        "{direction} failed schema validation. Check the 'errors' field in details for specific validation failures."
    )))
}

async fn next_stream_chunk(
    inner: &mut crate::module::ChunkStream,
    setup: &StreamSetup,
) -> Result<Option<Result<Value, ModuleError>>, ModuleError> {
    let context = setup.context();
    tokio::select! {
        biased;
        error = wait_stream_interruption(
            context.cancel_token.as_ref(), context.global_deadline, &setup.module_id,
            setup.module.as_ref(),
        ) => Err(error),
        item = inner.next() => Ok(item),
    }
}

fn stream_deadline_remaining(deadline: f64) -> Option<std::time::Duration> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    std::time::Duration::try_from_secs_f64((deadline - now).max(0.0)).ok()
}

fn stream_timeout_error(
    token: Option<&CancelToken>,
    module_id: &str,
    module: &dyn crate::module::Module,
) -> ModuleError {
    let mut error =
        ModuleError::module_timeout(module_id, 0).with_retryable(module.annotations().idempotent);
    error.message = format!("Module '{module_id}' streaming aborted: global deadline exceeded");
    error.details.remove("timeout_ms");
    if let Some(token) = token {
        token.cancel_after_timeout(&error);
    }
    error
}

pub(crate) async fn open_stream(
    ctx: &PipelineContext,
    module: &Arc<dyn crate::module::Module>,
) -> Result<Option<crate::module::ChunkStream>, ModuleError> {
    let context = ctx.context.clone();
    let token = context.cancel_token.clone();
    let deadline = context.global_deadline;
    let opening_module = Arc::clone(module);
    let module_id = ctx.module_id.clone();
    let inputs = ctx.inputs.clone();
    let lease = ctx.module_lease.clone();
    let span = tracing::Span::current();
    let open = move || {
        let _lease = lease;
        let _span = span.enter();
        if let Some(token) = context.cancel_token.as_ref() {
            token.check_for(&module_id)?;
        }
        if deadline
            .and_then(stream_deadline_remaining)
            .is_some_and(|remaining| remaining.is_zero())
        {
            return Err(stream_timeout_error(
                context.cancel_token.as_ref(),
                &module_id,
                opening_module.as_ref(),
            ));
        }
        let producer = match opening_module.as_streaming() {
            Some(typed) => Some(typed.stream_typed(inputs, &context)),
            None => opening_module.stream(inputs, &context),
        };
        Ok(producer)
    };
    let opening = match token.as_ref() {
        Some(token) => token.spawn_blocking_tracked(open),
        None => tokio::task::spawn_blocking(open),
    };
    tokio::select! {
        biased;
        error = wait_stream_interruption(
            token.as_ref(), deadline, &ctx.module_id, module.as_ref()
        ) => Err(error),
        result = opening => result.map_err(|error| {
            ModuleError::new(ErrorCode::GeneralInternalError, format!("Opening the stream failed: {error}"))
        })?,
    }
}

async fn wait_stream_interruption(
    token: Option<&CancelToken>,
    global_deadline: Option<f64>,
    module_id: &str,
    module: &dyn crate::module::Module,
) -> ModuleError {
    let cancellation = async {
        match token {
            Some(token) => token.cancelled().await,
            None => std::future::pending().await,
        }
    };
    let timeout = async {
        let remaining = global_deadline.and_then(stream_deadline_remaining);
        match remaining {
            Some(delta) if delta.is_zero() => {}
            Some(delta) => tokio::time::sleep(delta).await,
            None => std::future::pending().await,
        }
    };
    tokio::select! {
        biased;
        () = cancellation => {
            // The unary fallback's execute step signals its token on timeout.
            // Preserve that error if the pipeline is still unwinding hooks.
            token.and_then(CancelToken::timeout_error).cloned().unwrap_or_else(|| {
                crate::cancel::ExecutionCancelledError::new(
                    module_id, "Execution was cancelled",
                ).into()
            })
        }
        () = timeout => stream_timeout_error(token, module_id, module),
    }
}

/// Redact fields marked with `x-sensitive: true` in the schema.
///
/// Returns a deep copy of `data` with sensitive values replaced by `"***REDACTED***"`.
/// Also redacts any keys starting with `_secret_` regardless of schema.
/// Local `$ref` definitions, JSON Pointers, anchors and root `$id` references
/// participate in redaction. Recursive schemas are traversed without reference
/// loops; external references are never fetched.
///
/// Applies §10.6's schema rule ONLY. The executor's capture point must apply
/// the configured `obs.redaction.*` rules as well — use
/// [`redact_sensitive_with`] there. This signature is kept because it is the
/// published §10.6 algorithm and public API.
#[must_use]
pub fn redact_sensitive(data: &Value, schema: &Value) -> Value {
    let mut redacted = data.clone();
    schema_redaction::redact(&mut redacted, schema);
    match &mut redacted {
        Value::Object(object) => redact_secret_prefix(object),
        Value::Array(items) => redact_secret_prefix_in_list(items),
        _ => {}
    }
    redacted
}

pub(crate) fn schema_sensitive_fields(data: &Value, schema: &Value) -> Vec<String> {
    schema_redaction::sensitive_fields(data, schema)
}

/// Redact for the executor's input/output capture point.
///
/// PROTOCOL_SPEC §10.6.1 "Where the rules apply": the union of §10.6's
/// `x-sensitive` rule and the two configured `obs.redaction.*` rules MUST hold
/// at BOTH log emission and this capture point, with the same rules at each.
/// Until `rules` existed the capture point had no parameter for them at all —
/// the signature was `(data, schema)`, faithfully mirroring §10.6's published
/// algorithm — so an operator's `regex_patterns` entry redacted a bearer token
/// in the log line they were watching and stored it in the audit record they
/// were not (aiperceivable/apcore#120).
///
/// `rules` of `None` means the spec DEFAULTS, per requirement 3: "no
/// configuration" means the defaults, never no redaction. That is what this
/// path did before, so an unconfigured caller sees no change.
#[must_use]
pub fn redact_sensitive_with(
    data: &Value,
    schema: &Value,
    rules: Option<&RedactionConfig>,
) -> Value {
    let mut redacted = data.clone();
    schema_redaction::redact(&mut redacted, schema);
    match rules {
        Some(cfg) => cfg.redact(&mut redacted),
        None => RedactionConfig::defaults().redact(&mut redacted),
    }
    redacted
}

/// The value a built-in logging middleware writes for a call's inputs or output
/// (PROTOCOL_SPEC §10.6.1 requirement 5, D-131).
///
/// `captured` is `context.redacted_inputs` / `context.redacted_output`, filled
/// by the pipeline's capture point with the `x-sensitive` rule and the
/// configured rules already applied — that is what gets logged, never `raw`.
/// When the middleware is driven outside an executor, apply the configured
/// rules to `raw` before logging. Absent rules use the defaults per
/// requirement 3.
#[must_use]
pub(crate) fn loggable_value(
    captured: Option<&HashMap<String, Value>>,
    raw: &Value,
    rules: Option<&RedactionConfig>,
    within_executor: bool,
) -> Value {
    match captured {
        Some(map) => Value::Object(map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
        None if within_executor => Value::Null,
        None => redact_sensitive_with(raw, &Value::Null, rules),
    }
}

/// In-place redaction of keys starting with `_secret_`, at any depth.
///
/// A-D-003: recurses into both nested objects AND array elements so sensitive
/// keys inside objects nested in arrays are redacted. Mirrors apcore-python
/// `utils/redaction.py` (`_redact_by_keys_and_regex` + `_redact_in_list`).
fn redact_secret_prefix(data: &mut serde_json::Map<String, Value>) {
    let keys: Vec<String> = data.keys().cloned().collect();
    for key in keys {
        if key.starts_with("_secret_") {
            if let Some(val) = data.get(&key) {
                if !val.is_null() {
                    data.insert(key, Value::String(REDACTED_VALUE.to_string()));
                }
            }
        } else if let Some(val) = data.get_mut(&key) {
            match val {
                Value::Object(obj) => redact_secret_prefix(obj),
                Value::Array(arr) => redact_secret_prefix_in_list(arr),
                _ => {}
            }
        }
    }
}

/// Traverse a list, redacting `_secret_`-prefixed keys in dict children and
/// recursing into nested lists. Mirrors apcore-python `_redact_in_list`.
fn redact_secret_prefix_in_list(items: &mut [Value]) {
    for item in items.iter_mut() {
        match item {
            Value::Object(obj) => redact_secret_prefix(obj),
            Value::Array(nested) => redact_secret_prefix_in_list(nested),
            _ => {}
        }
    }
}

// PreflightResult is re-exported from module.rs — used as the return type for Executor::validate().

// ---------------------------------------------------------------------------
// Global strategy registry (introspection)
// ---------------------------------------------------------------------------

/// Global registry of named execution strategies for introspection.
static STRATEGY_REGISTRY: LazyLock<RwLock<Vec<StrategyInfo>>> =
    LazyLock::new(|| RwLock::new(Vec::new()));

/// Register a strategy's info in the global registry for introspection.
///
/// Replaces any existing entry with the same name.
///
/// Cross-language parity: apcore-python and apcore-typescript accept
/// `(name, strategy)` to the equivalent `Executor.register_strategy()` class
/// method. Use [`register_strategy_from_executor`] for that form
/// (sync finding A-015).
pub fn register_strategy(info: StrategyInfo) {
    let mut registry = STRATEGY_REGISTRY.write();
    // Replace existing entry with same name, or append.
    if let Some(existing) = registry.iter_mut().find(|s| s.name == info.name) {
        *existing = info;
    } else {
        registry.push(info);
    }
}

/// Register a strategy by (name, strategy) pair — cross-language parity form.
///
/// Builds a `StrategyInfo` from the given `ExecutionStrategy` and registers
/// it. Mirrors apcore-python `Executor.register_strategy(name, strategy)`
/// and apcore-typescript `Executor.registerStrategy(name, strategy)`
/// (sync finding A-015).
pub fn register_strategy_by_name(name: impl Into<String>, strategy: &ExecutionStrategy) {
    let name = name.into();
    let mut info = strategy.info();
    info.name = name;
    register_strategy(info);
}

/// List all registered strategy summaries.
pub fn list_strategies() -> Vec<StrategyInfo> {
    STRATEGY_REGISTRY.read().clone()
}

/// What is actually gating an executor's registry (PROTOCOL_SPEC 6.6.5).
///
/// Eight plain observations plus one derived flag. `acl.is_some()` is not the
/// answer to "what is gating this registry": the ACL and approval gates are
/// pipeline *steps*, and the `internal`, `testing` and `minimal` strategies all
/// remove them — so an executor can hold an ACL that no step ever consults.
///
/// Returned by [`Executor::governance_state`]. Pure data: reading it enforces
/// nothing and changes nothing. The existing public `acl` / `approval_handler`
/// / `policy` fields are unaffected; this answers a different question.
// Nine booleans is the shape PROTOCOL_SPEC 6.6.5.1 mandates, field for field,
// so that an adapter reads the same facts from all three SDKs. Collapsing any
// pair into an enum would make apcore-rust the only SDK where the accessor
// answers a different set of questions.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GovernanceState {
    /// At least one `system.control.*` module is in the registry.
    pub control_modules_registered: bool,
    /// At least one read-only `system.*` module is in the registry.
    pub read_modules_registered: bool,
    /// An ACL is in force: attached to the executor, which is what the
    /// built-in ACL gate reads on every call (PROTOCOL_SPEC 6.6.5.5).
    pub acl_configured: bool,
    /// The running strategy contains the built-in ACL gate, matched by capability.
    pub builtin_acl_gate_wired: bool,
    /// An `ApprovalHandler` is in force: attached to the executor, which is
    /// what the built-in approval gate reads on every call.
    pub approval_handler_configured: bool,
    /// The running strategy contains the built-in approval gate, matched by capability.
    pub builtin_approval_gate_wired: bool,
    /// The effective policy uses strict approval enforcement.
    pub policy_strict: bool,
    /// Every registered `system.control.*` module declares `requires_approval`.
    ///
    /// Required by the derived flag because the two gates are not symmetric
    /// (PROTOCOL_SPEC 6.6.5.1.1): `acl_check` evaluates every call, but
    /// `approval_gate` resolves per module and returns before consulting the
    /// handler when the module does not need approval. `false` when no control
    /// module is registered.
    pub all_control_modules_require_approval: bool,
    /// Control modules are registered and no recognised built-in gate engages.
    ///
    /// Reports the **absence of a gate**, never the presence of protection: a
    /// wired ACL that permits every call still yields `false`. And `true` does
    /// not mean the call will succeed — a custom step, custom middleware or an
    /// upstream gateway is invisible here by construction.
    pub unprotected_control_surface: bool,
}

/// The payload of an Executor's identity handle.
///
/// `Context::executor` is type-erased so `Context` stays independent of the
/// Executor type; its identity is the handle's allocation. The weak reference
/// is what makes the handle usable for a nested call without a reference
/// cycle: an Executor holds its handle, a Context holds the handle, and
/// neither keeps the Executor alive.
#[derive(Debug, Default)]
pub struct ExecutorHandle {
    executor: std::sync::OnceLock<std::sync::Weak<Executor>>,
}

impl ExecutorHandle {
    /// The Executor this handle identifies, if it is shared and still alive.
    #[must_use]
    pub fn executor(&self) -> Option<Arc<Executor>> {
        self.executor.get().and_then(std::sync::Weak::upgrade)
    }
}

#[derive(Debug)]
struct RuntimeConfig {
    source: Arc<tokio::sync::Mutex<Config>>,
    snapshot: parking_lot::Mutex<Option<ConfigSnapshot>>,
}

#[derive(Clone, Debug)]
struct ConfigSnapshot {
    config: Arc<Config>,
    redaction: Arc<RedactionConfig>,
}

impl RuntimeConfig {
    async fn snapshot(&self) -> ConfigSnapshot {
        let source = self.source.lock().await;
        let mut snapshot = self.snapshot.lock();
        if let Some(current) = snapshot.as_ref() {
            if current.config.generation == source.generation {
                return current.clone();
            }
        }
        let current = ConfigSnapshot {
            config: Arc::new(source.clone()),
            redaction: Arc::new(RedactionConfig::from_config(&source)),
        };
        *snapshot = Some(current.clone());
        current
    }
}

/// Responsible for executing modules with middleware, ACL, and context management.
#[derive(Debug)]
pub struct Executor {
    pub registry: Arc<Registry>,
    /// Configuration supplied when this executor was constructed.
    pub config: Arc<Config>,
    runtime_config: parking_lot::RwLock<Option<Arc<RuntimeConfig>>>,
    pub acl: Option<Arc<ACL>>,
    pub(crate) acl_load_error: Option<ModuleError>,
    pub(crate) pipeline_load_error: Option<ModuleError>,
    pub approval_handler: Option<Arc<dyn ApprovalHandler>>,
    /// Execution-time governance policy consulted by the Step-5 approval gate
    /// (apcore#76 RFC pilot). Flows to the gate via `PipelineContext`.
    pub policy: Option<Arc<ExecutionPolicy>>,
    policy_evaluator: Option<Arc<dyn PolicyEvaluator>>,
    /// Event emitter for governance events (apcore#77). When set, the ACL and
    /// approval-gate steps publish `apcore.acl.denied` /
    /// `apcore.approval.decision` / `apcore.policy.override` on the bus.
    pub event_emitter: Option<Arc<EventEmitter>>,
    /// Dedup set backing the approval gate's fail-loud warn-once-per-module
    /// governance warnings (apcore#76). Executor-owned so dedup persists across
    /// calls; threaded into each `PipelineContext` by `inject_resources`.
    governance_warned: Arc<parking_lot::Mutex<std::collections::HashSet<String>>>,
    /// The `obs.redaction.*` rules the capture point applies, built ONCE from
    /// `config` (PROTOCOL_SPEC §10.6.1 requirement 5, aiperceivable/apcore#120).
    ///
    /// Lazy rather than eager so an `Executor` constructed and never used pays
    /// nothing, and so a `Config` mutated between construction and first call
    /// is still read — `Config::set` is public and callers do use it.
    redaction: std::sync::OnceLock<Arc<RedactionConfig>>,
    pub middleware_manager: Arc<MiddlewareManager>,
    /// Execution strategy — all calls go through PipelineEngine.
    strategy: ExecutionStrategy,
    /// Unique per-Executor identity handle bound to `context.executor` at
    /// pipeline entry (apcore Issue #66 / `core-executor.md` §"Contract:
    /// Executor binding to Context"). Two `Executor` instances are
    /// distinguished by `Arc::ptr_eq` on this handle; cloning an `Executor`
    /// would produce a fresh handle by design — Executors are meant to be
    /// shared via `Arc<Executor>`, not duplicated.
    ///
    /// Once the Executor is shared (see [`Executor::into_shared`]) the handle
    /// also leads back to it, which is how a module reaches the Executor for a
    /// nested call through [`Context::executor`].
    instance_handle: Arc<ExecutorHandle>,
    /// Per-instance toggle store consulted by the `module_lookup` step to
    /// reject disabled modules (issue #71). Defaults to the process-global
    /// store so executors built outside an `APCore` keep working; `APCore`
    /// rebinds this to its own store via [`Executor::set_toggle_state`] so the
    /// read path (pipeline lookup) and the write path
    /// (`ToggleFeatureModule`) share one instance-scoped store.
    toggle_state: Arc<crate::sys_modules::ToggleState>,
}

// ---------------------------------------------------------------------------
// Config-driven pipeline (PROTOCOL_SPEC §5.16 requirements 6 and 7)
// ---------------------------------------------------------------------------

/// Steps whose removal withdraws a protection rather than an ordinary stage.
const SECURITY_STEPS: [&str; 2] = ["acl_check", "approval_gate"];

/// PROTOCOL_SPEC §5.16 requirement 7 — removing `acl_check` or `approval_gate`
/// emits a diagnostic, once per configuration load.
///
/// The reason is the transition, not the steady state. Requirement 6 makes a
/// previously ignored section take effect, so a configuration that has been
/// carrying `remove: [acl_check]` while ACL was enforced anyway starts having
/// ACL genuinely removed. That is the operator getting what they asked for, and
/// it is also the one direction in which honouring configuration can withdraw a
/// protection that was in place a moment before. It is a notice, not a refusal.
fn warn_removed_security_steps(section: &Value) {
    let Some(remove_list) = section.get("remove").and_then(Value::as_array) else {
        return;
    };
    let removed: Vec<&str> = remove_list
        .iter()
        .filter_map(Value::as_str)
        .filter(|name| SECURITY_STEPS.contains(name))
        .collect();
    if removed.is_empty() {
        return;
    }
    tracing::warn!(
        steps = %removed.join(", "),
        "pipeline.remove takes security step(s) out of the execution pipeline. \
         Calls will no longer be checked against the ACL or held for approval. \
         This is what the configuration asks for; remove the entry to restore it."
    );
}

/// Build the configured strategy. Infallible constructors retain the failure
/// and reject execution before any strategy step can run.
fn strategy_from_config(
    config: &Config,
    toggle_state: &Arc<crate::sys_modules::ToggleState>,
) -> (ExecutionStrategy, Option<ModuleError>) {
    let standard =
        || crate::builtin_steps::build_standard_strategy_with_toggle(Arc::clone(toggle_state));
    let Some(section) = config.get("pipeline") else {
        return (standard(), None);
    };
    if section.as_object().is_some_and(serde_json::Map::is_empty) {
        return (standard(), None);
    }
    match crate::pipeline_config::build_strategy_from_config_with_toggle(
        &section,
        Some(config),
        Arc::clone(toggle_state),
    ) {
        Ok(strategy) => {
            warn_removed_security_steps(&section);
            (strategy, None)
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                "The `pipeline:` section failed to build; module execution is disabled"
            );
            (standard(), Some(e))
        }
    }
}

fn has_policy_compatible_layout(strategy: &ExecutionStrategy) -> bool {
    fn step_types(strategy: &ExecutionStrategy) -> impl Iterator<Item = std::any::TypeId> + '_ {
        strategy.steps().iter().map(|step| step.as_ref().type_id())
    }

    step_types(strategy).eq(step_types(&build_standard_strategy()))
        || step_types(strategy).eq(step_types(&build_performance_strategy()))
}

impl Executor {
    /// Create a new executor with the given (shared) registry and config.
    ///
    /// Builds a standard execution strategy — all calls go through PipelineEngine.
    /// Accepts either an owned `Registry`/`Config` (convenient for tests) or a
    /// pre-shared `Arc<Registry>`/`Arc<Config>` (required for runtime wiring).
    pub fn new(registry: impl Into<Arc<Registry>>, config: impl Into<Arc<Config>>) -> Self {
        let toggle_state = crate::sys_modules::global_toggle_state_arc();
        let config = config.into();
        // §5.16 requirement 6: a `pipeline:` section in the loaded config is
        // what runs. It used to be parsed, validated and then ignored.
        let (strategy, pipeline_load_error) = strategy_from_config(&config, &toggle_state);
        Self {
            registry: registry.into(),
            config,
            runtime_config: parking_lot::RwLock::new(None),
            acl: None,
            acl_load_error: None,
            pipeline_load_error,
            approval_handler: None,
            policy: None,
            policy_evaluator: None,
            event_emitter: None,
            governance_warned: Arc::new(parking_lot::Mutex::new(std::collections::HashSet::new())),
            redaction: std::sync::OnceLock::new(),
            middleware_manager: Arc::new(MiddlewareManager::new()),
            strategy,
            instance_handle: Arc::new(ExecutorHandle::default()),
            toggle_state,
        }
    }

    /// Create a new executor with a strategy resolved by name.
    ///
    /// Built-in preset names: `"standard"`, `"internal"`, `"testing"`,
    /// `"performance"`, `"minimal"`.
    pub fn with_strategy_name(
        registry: impl Into<Arc<Registry>>,
        config: impl Into<Arc<Config>>,
        name: &str,
    ) -> Result<Self, ModuleError> {
        let strategy = resolve_strategy_by_name(name)?;
        Ok(Self {
            registry: registry.into(),
            config: config.into(),
            runtime_config: parking_lot::RwLock::new(None),
            acl: None,
            acl_load_error: None,
            pipeline_load_error: None,
            approval_handler: None,
            policy: None,
            policy_evaluator: None,
            event_emitter: None,
            governance_warned: Arc::new(parking_lot::Mutex::new(std::collections::HashSet::new())),
            redaction: std::sync::OnceLock::new(),
            middleware_manager: Arc::new(MiddlewareManager::new()),
            strategy,
            instance_handle: Arc::new(ExecutorHandle::default()),
            toggle_state: crate::sys_modules::global_toggle_state_arc(),
        })
    }

    /// Create a new executor with a custom execution strategy.
    pub fn with_strategy(
        registry: impl Into<Arc<Registry>>,
        config: impl Into<Arc<Config>>,
        strategy: ExecutionStrategy,
    ) -> Self {
        Self {
            registry: registry.into(),
            config: config.into(),
            runtime_config: parking_lot::RwLock::new(None),
            acl: None,
            acl_load_error: None,
            pipeline_load_error: None,
            approval_handler: None,
            policy: None,
            policy_evaluator: None,
            event_emitter: None,
            governance_warned: Arc::new(parking_lot::Mutex::new(std::collections::HashSet::new())),
            redaction: std::sync::OnceLock::new(),
            middleware_manager: Arc::new(MiddlewareManager::new()),
            strategy,
            instance_handle: Arc::new(ExecutorHandle::default()),
            toggle_state: crate::sys_modules::global_toggle_state_arc(),
        }
    }

    /// Create a new executor with all optional parameters.
    pub fn with_options(
        registry: impl Into<Arc<Registry>>,
        config: impl Into<Arc<Config>>,
        middlewares: Option<Vec<Box<dyn Middleware>>>,
        acl: Option<ACL>,
        approval_handler: Option<Box<dyn ApprovalHandler>>,
    ) -> Self {
        let middleware_manager = MiddlewareManager::new();
        if let Some(mws) = middlewares {
            for mw in mws {
                // Middleware provided at construction time is trusted; log and
                // skip if priority is out of range rather than failing the build.
                if let Err(e) = middleware_manager.add(mw) {
                    tracing::warn!("Skipping middleware during executor construction: {}", e);
                }
            }
        }
        let toggle_state = crate::sys_modules::global_toggle_state_arc();
        let config = config.into();
        // §5.16 requirement 6 — see `Executor::new`.
        let (strategy, pipeline_load_error) = strategy_from_config(&config, &toggle_state);
        Self {
            registry: registry.into(),
            config,
            runtime_config: parking_lot::RwLock::new(None),
            acl: acl.map(Arc::new),
            acl_load_error: None,
            pipeline_load_error,
            approval_handler: approval_handler.map(|h| Arc::from(h) as Arc<dyn ApprovalHandler>),
            policy: None,
            policy_evaluator: None,
            event_emitter: None,
            governance_warned: Arc::new(parking_lot::Mutex::new(std::collections::HashSet::new())),
            redaction: std::sync::OnceLock::new(),
            middleware_manager: Arc::new(middleware_manager),
            strategy,
            instance_handle: Arc::new(ExecutorHandle::default()),
            toggle_state,
        }
    }

    /// Get a reference to the registry.
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Get the names of all middlewares in pipeline order.
    pub fn middlewares(&self) -> Vec<String> {
        self.middleware_manager.snapshot()
    }

    /// Find the first middleware in the chain whose name matches, as a shared
    /// handle. Used by extension application to locate an existing
    /// `TracingMiddleware` and reconfigure it (sync finding A-D-18).
    pub fn find_middleware(&self, name: &str) -> Option<Arc<dyn Middleware>> {
        self.middleware_manager.find_by_name(name)
    }

    /// Rebind the per-instance toggle store consulted by the `module_lookup`
    /// step (issue #71).
    ///
    /// `APCore::with_options` calls this so the pipeline read path and the
    /// `ToggleFeatureModule` write path share one instance-scoped
    /// [`ToggleState`](crate::sys_modules::ToggleState).
    ///
    /// Every built-in preset (`standard`, `internal`, `testing`,
    /// `performance`, `minimal`) is derived from `build_standard_strategy()`,
    /// which binds the *process-global* toggle store, so all five need the
    /// `module_lookup` step rebound — not just `standard`. Rebinding only the
    /// `standard` preset meant `apcore.disable(module)` on one instance was
    /// silently ignored by any executor running another preset (issue #71).
    /// apcore-python honours the per-instance store in all five presets.
    ///
    /// A user-supplied custom strategy is left untouched: those callers are
    /// expected to wire the store themselves via
    /// [`build_standard_strategy_with_toggle`](crate::builtin_steps::build_standard_strategy_with_toggle).
    pub fn set_toggle_state(&mut self, toggle_state: Arc<crate::sys_modules::ToggleState>) {
        const BUILTIN_PRESETS: &[&str] =
            &["standard", "internal", "testing", "performance", "minimal"];
        if BUILTIN_PRESETS.contains(&self.strategy.name()) {
            let bound = Arc::clone(&toggle_state);
            // `replace_with` bypasses the `replaceable` flag (module_lookup is
            // not user-replaceable) and preserves the step's position.
            // `minimal` and friends all keep `module_lookup`, but ignore the
            // error if a preset ever drops it.
            self.strategy
                .replace_with("module_lookup", move |_old| {
                    Box::new(crate::builtin_steps::BuiltinModuleLookup::new(bound))
                })
                .ok();
        }
        self.toggle_state = toggle_state;
    }

    /// Shared handle to this executor's per-instance toggle store (issue #71).
    #[must_use]
    pub fn toggle_state(&self) -> Arc<crate::sys_modules::ToggleState> {
        Arc::clone(&self.toggle_state)
    }

    pub(crate) fn bind_client_toggle_state(
        &mut self,
        reuse_existing: bool,
    ) -> Arc<crate::sys_modules::ToggleState> {
        let isolated = if reuse_existing {
            let state = self.toggle_state();
            if !Arc::ptr_eq(&state, &crate::sys_modules::global_toggle_state_arc()) {
                return state;
            }
            // Preserve decisions made by a standalone executor while isolating
            // future client toggles from the process-wide default store.
            Arc::new(state.snapshot())
        } else {
            Arc::new(crate::sys_modules::ToggleState::new())
        };
        self.set_toggle_state(Arc::clone(&isolated));
        isolated
    }

    /// Set the ACL for access control.
    pub fn set_acl(&mut self, acl: ACL) {
        self.set_acl_shared(Arc::new(acl));
    }

    /// Install an already-shared ACL handle.
    ///
    /// The counterpart to [`Self::set_acl`] for callers that keep their own
    /// reference — [`ExtensionManager::apply`](crate::extensions::ExtensionManager::apply)
    /// retains its registrations (D-78), so it wires a clone rather than the
    /// only copy.
    pub fn set_acl_shared(&mut self, acl: Arc<ACL>) {
        self.acl = Some(acl);
        self.acl_load_error = None;
    }

    fn ensure_startup_configuration(&self) -> Result<(), ModuleError> {
        if let Some(error) = &self.pipeline_load_error {
            return Err(error.clone());
        }
        if self.acl_load_error.is_some() {
            return Err(ModuleError::new(
                ErrorCode::ConfigInvalid,
                "ACL configuration failed to load",
            ));
        }
        Ok(())
    }

    /// Set the approval handler.
    pub fn set_approval_handler(&mut self, handler: Box<dyn ApprovalHandler>) {
        self.set_approval_handler_shared(Arc::from(handler));
    }

    /// Install an already-shared approval-handler handle (D-78).
    pub fn set_approval_handler_shared(&mut self, handler: Arc<dyn ApprovalHandler>) {
        self.approval_handler = Some(handler);
    }

    /// Set (or clear) the execution-time governance policy consulted by the
    /// Step-5 approval gate (apcore#76 RFC pilot). Mirrors
    /// [`Self::set_approval_handler`]; the policy flows to the gate via
    /// `PipelineContext` at each invocation, so this takes effect immediately
    /// for subsequent calls. Pass `None` to restore the default policy.
    pub fn set_policy(&mut self, policy: Option<ExecutionPolicy>) {
        self.policy = policy.map(Arc::new);
    }

    /// Install or clear the Executor's post-validation evaluator.
    ///
    /// Installation requires the stock `standard` or `performance` layout.
    /// Step middleware may observe those steps. Before middleware on the
    /// opted-in path must perform side-effect-free normalization.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::PipelineConfigurationError`] for an incompatible strategy.
    pub fn set_policy_evaluator(
        &mut self,
        evaluator: Option<Arc<dyn PolicyEvaluator>>,
    ) -> Result<(), ModuleError> {
        match (self.policy_evaluator.is_some(), evaluator.is_some()) {
            (false, true) => {
                if !has_policy_compatible_layout(&self.strategy) {
                    return Err(ModuleError::new(ErrorCode::PipelineConfigurationError,
                        "Post-validation policy requires the stock standard or performance strategy"));
                }
                self.strategy
                    .insert_after("input_validation", Box::new(BuiltinPostValidationPolicy))?;
            }
            (true, false) => self.strategy.remove("post_validation_policy")?,
            _ => {}
        }
        self.policy_evaluator = evaluator;
        Ok(())
    }

    /// Set (or clear) the event emitter used to publish governance events
    /// (apcore#77): `apcore.acl.denied`, `apcore.approval.decision`, and
    /// `apcore.policy.override`.
    pub fn set_event_emitter(&mut self, emitter: Option<Arc<EventEmitter>>) {
        self.event_emitter = emitter;
    }

    /// Add a middleware to the pipeline.
    ///
    /// Returns an error if the middleware's priority exceeds the allowed range.
    ///
    /// Takes `&self` — `MiddlewareManager` uses interior mutability, so the
    /// executor can be held behind a shared reference and still have
    /// middleware added after construction. This removes the previous
    /// `Arc::get_mut` hack that panicked once the middleware manager was
    /// cloned into a pipeline context.
    ///
    /// Returns the [`MiddlewareHandle`] for this registration. Keep it to remove
    /// exactly this middleware later via [`Self::remove_handle`]; `remove(name)`
    /// cannot, because duplicate registration only warns and two instances
    /// answering the same `name()` is a reachable state (sync finding A-C-001).
    /// Discarding the handle with `?;` stays valid, so existing callers are
    /// unaffected.
    ///
    /// A second middleware with the same `name()` is registered and warned
    /// about, as apcore-python and apcore-typescript warn on a duplicate
    /// identity (see [`MiddlewareManager::add_detecting_duplicates`]).
    #[track_caller]
    pub fn use_middleware(
        &self,
        middleware: Box<dyn Middleware>,
    ) -> Result<MiddlewareHandle, ModuleError> {
        self.middleware_manager.add_detecting_duplicates(middleware)
    }

    /// Add an already-shared middleware handle to the pipeline.
    ///
    /// The counterpart to [`Self::use_middleware`] for callers that keep their
    /// own reference (D-78).
    ///
    /// # Errors
    ///
    /// Returns [`ModuleError`] if the middleware's priority exceeds the
    /// allowed range.
    pub fn use_middleware_shared(
        &self,
        middleware: Arc<dyn Middleware>,
    ) -> Result<MiddlewareHandle, ModuleError> {
        self.middleware_manager.add_shared(middleware)
    }

    /// Remove a middleware by name.
    ///
    /// Removes the FIRST match in pipeline order. When the caller means one
    /// specific instance, use [`Self::remove_handle`] — that is the
    /// identity-based removal `Contract: APCore.remove` describes, which
    /// apcore-python and apcore-typescript get from comparing the object itself.
    pub fn remove(&self, name: &str) -> bool {
        self.middleware_manager.remove(name)
    }

    /// Remove exactly the middleware that [`Self::use_middleware`] returned
    /// `handle` for. Returns `false` if it is no longer registered.
    pub fn remove_handle(&self, handle: MiddlewareHandle) -> bool {
        self.middleware_manager.remove_handle(handle)
    }

    /// Remove a middleware by name (legacy alias).
    pub fn remove_middleware(&self, name: &str) -> bool {
        self.remove(name)
    }

    /// Validate module_id format before pipeline setup.
    ///
    /// Mirrors `apcore-python.Executor._validate_module_id` and
    /// `apcore-typescript.Executor.validateModuleId`. Fails fast with
    /// `InvalidInput` rather than letting the pipeline's module-lookup step
    /// produce a generic "not found" error for a malformed ID.
    fn validate_module_id(module_id: &str) -> Result<(), ModuleError> {
        if module_id.is_empty() || !module_id_pattern().is_match(module_id) {
            return Err(ModuleError::new(
                ErrorCode::InvalidModuleId,
                format!(
                    "Invalid module ID: '{}'. Must match pattern: {}",
                    module_id,
                    crate::registry::registry::MODULE_ID_PATTERN,
                ),
            ));
        }
        // `Contract: Executor.call` requires over-length IDs to be rejected
        // here, before the PipelineContext is built (D-75). The registry
        // enforces the same bound one step later; without this check a
        // 300-character well-formed ID reached the pipeline and came back as
        // MODULE_NOT_FOUND instead of INVALID_MODULE_ID.
        if module_id.len() > crate::registry::registry::MAX_MODULE_ID_LENGTH {
            return Err(ModuleError::new(
                ErrorCode::InvalidModuleId,
                format!(
                    "Module ID exceeds maximum length of {}: {}",
                    crate::registry::registry::MAX_MODULE_ID_LENGTH,
                    module_id.len()
                ),
            ));
        }
        Ok(())
    }

    /// Execute (call) a module by ID with the given inputs and context.
    ///
    /// Delegates to `PipelineEngine::run()` using the configured strategy.
    pub async fn call(
        &self,
        module_id: &str,
        inputs: serde_json::Value,
        ctx: Option<&Context<serde_json::Value>>,
        version_hint: Option<&str>,
    ) -> Result<serde_json::Value, ModuleError> {
        self.execute_call(module_id, inputs, ctx, version_hint, &self.strategy, false)
            .await
            .map(|(output, _)| output)
    }

    async fn execute_call(
        &self,
        module_id: &str,
        mut inputs: Value,
        context: Option<&Context<Value>>,
        version_hint: Option<&str>,
        strategy: &ExecutionStrategy,
        record_attempts: bool,
    ) -> Result<(Value, PipelineTrace), ModuleError> {
        self.ensure_startup_configuration()?;
        if self.policy_evaluator.is_some() && !std::ptr::eq(strategy, self.strategy()) {
            return Err(ModuleError::new(
                ErrorCode::PipelineConfigurationError,
                "Per-call strategy overrides cannot replace the active policy strategy",
            ));
        }
        Self::validate_module_id(module_id)?;
        let mut parent = context.cloned().unwrap_or_else(Context::anonymous);
        parent.bind_executor(self.instance_handle())?;
        let started = std::time::Instant::now();
        let mut previous_steps = Vec::new();

        loop {
            let mut attempt =
                PipelineContext::new(module_id, inputs, parent.clone(), strategy.name());
            attempt.validate_approval_credential()?;
            attempt.version_hint = version_hint.map(str::to_owned);
            self.inject_resources(&mut attempt).await;

            let (output, mut trace) = match PipelineEngine::run(strategy, &mut attempt).await {
                Ok((output, trace)) => {
                    Self::reject_aborted_trace(&trace)?;
                    (output.unwrap_or(Value::Null), trace)
                }
                Err(error) => {
                    let error = Self::call_error(module_id, &attempt, error);
                    match Self::recover_call(module_id, &attempt, &error).await {
                        Some(OnErrorOutcome::Recovery(output)) => (output, attempt.trace),
                        Some(OnErrorOutcome::Retry(signal)) => {
                            // Preserve the call's deadline and shared state. Each
                            // attempt derives its context from the original caller.
                            parent.global_deadline = attempt.context.global_deadline;
                            inputs = signal.inputs;
                            if record_attempts {
                                previous_steps.extend(attempt.trace.steps);
                            }
                            continue;
                        }
                        None => return Err(error),
                    }
                }
            };
            if !previous_steps.is_empty() {
                previous_steps.append(&mut trace.steps);
                trace.steps = previous_steps;
                trace.total_duration_ms = started.elapsed().as_secs_f64() * 1000.0;
            }
            return Ok((output, trace));
        }
    }

    fn call_error(module_id: &str, ctx: &PipelineContext, error: ModuleError) -> ModuleError {
        let error = error.unwrap_pipeline_step_error().unwrap_or(error);
        let error = error.unwrap_middleware_chain_error().unwrap_or(error);
        propagate_module_error(error, module_id, &ctx.context)
    }

    async fn recover_call(
        module_id: &str,
        ctx: &PipelineContext,
        error: &ModuleError,
    ) -> Option<OnErrorOutcome> {
        if ctx.blocks_error_recovery()
            || error.code == ErrorCode::ExecutionCancelled
            || ctx.executed_middlewares.is_empty()
        {
            return None;
        }
        MiddlewareManager::execute_on_error_outcome_snapshot(
            ctx.middleware_snapshot.as_deref()?,
            module_id,
            ctx.inputs.clone(),
            error,
            &ctx.context,
            &ctx.executed_middlewares,
        )
        .await
    }

    /// Validate module inputs without executing (steps 1-7, spec §12.3).
    ///
    /// Runs the pipeline in `dry_run` mode — pure steps only, side-effecting
    /// steps are skipped automatically.
    ///
    /// `ctx` is the optional execution context. When provided, call-chain
    /// checks (depth limit, circular-call detection) and ACL caller-identity
    /// matching can run against real caller state. When omitted, an anonymous
    /// `@external` context is synthesized for backward compatibility, in which
    /// case call-chain checks are no-ops.
    ///
    /// Aligned with `apcore-python.Executor.validate(module_id, inputs, context=None)`
    /// and `apcore-typescript.Executor.validate(moduleId, inputs?, context?)` per
    /// PROTOCOL_SPEC §12.2.
    #[allow(clippy::too_many_lines)]
    pub async fn validate(
        &self,
        module_id: &str,
        inputs: &serde_json::Value,
        ctx: Option<&Context<serde_json::Value>>,
    ) -> Result<PreflightResult, ModuleError> {
        self.ensure_startup_configuration()?;
        // Sync finding A-D-010: validate() is a NON-THROWING preflight —
        // a malformed module_id MUST be reported as a failed check, not
        // bubble up as a `ModuleError`. apcore-python and apcore-typescript
        // return a structured PreflightResult with `valid=false` for the
        // module_id check; previously Rust used `?` on validate_module_id
        // which broke that contract for callers expecting validate() to
        // never throw on input-shape errors.
        if let Err(err) = Self::validate_module_id(module_id) {
            let check = PreflightCheckResult {
                check: "module_id".to_string(),
                passed: false,
                error: Some(err.to_dict()),
                warnings: vec![],
            };
            return Ok(PreflightResult {
                valid: false,
                checks: vec![check],
                requires_approval: false,
                predicted_changes: vec![],
            });
        }
        // D-103: no synthetic principal for a call that supplied none.
        let context = ctx
            .cloned()
            .unwrap_or_else(Context::<serde_json::Value>::anonymous);
        let mut pipe_ctx =
            PipelineContext::new(module_id, inputs.clone(), context, self.strategy.name());
        let business_inputs = pipe_ctx
            .discard_approval_credential()
            .then(|| pipe_ctx.inputs.clone());
        let inputs = business_inputs.as_ref().unwrap_or(inputs);
        pipe_ctx.dry_run = true;
        self.inject_resources(&mut pipe_ctx).await;
        // validate() is non-throwing per A-D-010: bind errors surface as a
        // failed `executor_binding` check rather than bubbling up.
        if let Err(err) = self.bind_to_context(&mut pipe_ctx) {
            let check = PreflightCheckResult {
                check: "executor_binding".to_string(),
                passed: false,
                error: Some(err.to_dict()),
                warnings: vec![],
            };
            return Ok(PreflightResult {
                valid: false,
                checks: vec![check],
                requires_approval: false,
                predicted_changes: vec![],
            });
        }

        let mut checks: Vec<PreflightCheckResult> = Vec::new();
        let trace_result = PipelineEngine::run(&self.strategy, &mut pipe_ctx).await;
        match trace_result {
            Ok((_output, trace)) => {
                checks.extend(trace_to_checks(&trace.steps, &pipe_ctx.preflight_errors));
            }
            Err(e) => {
                // Pipeline step raised an error; convert to a failed check.
                //
                // The trace's own entry for the aborting step is dropped: the
                // typed entry pushed below supersedes it, carrying the WIRE
                // error code and the real message where the trace carries only
                // `STEP_<NAME>_FAILED`. Keeping both put the SAME failure in
                // `checks` twice, so `errors()` reported two problems where one
                // existed and §12.8.4's one-entry-per-check shape did not hold.
                let steps = pipe_ctx.trace.steps.as_slice();
                let completed = match steps.split_last() {
                    Some((last, completed))
                        if last.result.action == "abort"
                            && !pipe_ctx
                                .preflight_errors
                                .iter()
                                .any(|(name, _)| name == &last.name) =>
                    {
                        completed
                    }
                    _ => steps,
                };
                checks.extend(trace_to_checks(completed, &pipe_ctx.preflight_errors));
                // §1.1 fail-fast: unwrap `PipelineStepError` to the original
                // typed cause for preflight categorization (mirrors Python).
                let underlying = e.unwrap_pipeline_step_error().unwrap_or(e);
                let check_name = match underlying.code {
                    ErrorCode::ModuleNotFound => "module_lookup",
                    ErrorCode::ACLDenied => crate::module::ACL_CHECK_NAME,
                    ErrorCode::SchemaValidationError | ErrorCode::GeneralInvalidInput => "schema",
                    ErrorCode::CallDepthExceeded | ErrorCode::CircularCall => "call_chain",
                    _ => "unknown",
                };
                checks.push(PreflightCheckResult {
                    check: check_name.to_string(),
                    passed: false,
                    error: Some(serde_json::json!({
                        // WIRE code, never `Debug` (errors.rs `wire_str`, sync
                        // finding A-D-14). apcore-python builds this dict from
                        // `underlying_e.to_dict()`, whose `code` is the wire
                        // string (executor.py:604-607 — the
                        // `type(e).__name__` line above it is only the fallback
                        // for a non-apcore exception), and apcore-typescript
                        // emits `{ code: e.code }` (executor.ts:880). `Debug`
                        // here yielded the PascalCase variant name, which is a
                        // code no registry contains.
                        "code": underlying.wire_code(),
                        "message": underlying.message,
                    })),
                    warnings: vec![],
                });
            }
        }

        // Detect requires_approval from module annotations. Under an
        // ExecutionPolicy (apcore#76) the preflight MUST report the
        // policy-effective verdict (`PolicyDecision::needs_approval`) so it
        // matches what the Step-5 gate will actually enforce.
        //
        // Read the SAME governance source `BuiltinApprovalGate` reads: the
        // D-96 union of the live instance's annotations and the registry
        // descriptor's (`ModuleAnnotations::governance_union`). A preflight that
        // consults a narrower source than the gate reports a verdict the gate
        // will not honour, which is the one thing §7.9.5 exists to prevent.
        //
        // The descriptor is a real governance source here and not merely a
        // fallback for an unresolved module: this SDK accepts a caller-supplied
        // `ModuleDescriptor`. Taking it only when the live module is absent —
        // an `or_else` rather than a union — meant a descriptor-only
        // `requires_approval: true` was reported as "no approval needed" for
        // every module that DID resolve, and the caller then hit the gate.
        let module_annotations = crate::module::ModuleAnnotations::governance_union(
            pipe_ctx
                .module
                .as_ref()
                .map(|module| module.annotations())
                .as_ref(),
            pipe_ctx.declared_module_annotations().as_ref(),
        );
        // Resolve preflight through the same default and call-site-aware
        // policy entry point as the Step-5 gate (§7.9.6).
        let policy_effective_approval = ExecutionPolicy::effective(self.policy.as_deref())
            .resolve_with_call_site(module_id, module_annotations.as_ref(), Some(inputs), ctx)
            .needs_approval;
        // PROTOCOL_SPEC §7.9.5: report the GOVERNANCE-effective requirement —
        // the union of §6.9 rows 3-5 for this call site — not merely the
        // policy-effective one. Since v1.28.0 that union includes an ACL rule
        // carrying `approval` (§6.1.6), recorded on the pipeline context by the
        // Step-4 `acl_check` that just ran (it is `pure`, so a dry run executes
        // it). Reporting only the policy-effective value would tell a caller no
        // approval is needed for a call the Step-5 gate will stop.
        let requires_approval = policy_effective_approval
            || pipe_ctx.acl_approval_required
            || pipe_ctx.policy_effect == Some(PolicyEffect::RequireApproval);

        // Module-level introspection is gated on TWO conditions, not one.
        //
        // 1. The PIPELINE resolved a module — `pipe_ctx.module`, exactly the
        //    handle apcore-python and apcore-typescript guard on. This used to
        //    re-query `self.registry.get(module_id)` independently, which is a
        //    different question: the registry still holds a module the pipeline
        //    deliberately refused to resolve. `module_lookup` raises
        //    ModuleDisabled for a module toggled off via
        //    `system.control.toggle_feature` BEFORE assigning `ctx.module`, and
        //    `call_chain_guard` runs before `module_lookup` at all — so a
        //    disabled module, or one rejected by the depth/cycle guard, still
        //    had its `preflight()` and `preview()` executed here and its
        //    `predicted_changes` returned, where the peers emit no
        //    `module_preflight` / `module_preview` check at all.
        // 2. The ACL did not deny the call — PROTOCOL_SPEC §12.8.5.1.
        //
        // Condition 2 is the security half. `preflight()` and `preview()` are
        // module-authored code, and what they return names what the call would
        // do: the resolved binary and argv of a command-wrapping module, the
        // target of a write. Module lookup is Step 3 and the ACL check is Step
        // 4, so gating on lookup alone runs module code for a caller the ACL
        // just denied and hands back what it said. `apcore-mcp-rust` had to
        // filter these three check names out of `__apcore_module_preview` by
        // string to keep argv away from a denied caller; the gate belongs here.
        //
        // Scoped to authorization deliberately: a failed `schema` check does
        // NOT suppress introspection, because a caller the ACL permits is
        // entitled to the module's account of what would happen even when its
        // inputs are malformed.
        let acl_denied = checks
            .iter()
            .any(|c| c.check == crate::module::ACL_CHECK_NAME && !c.passed);
        let acl_required = pipe_ctx.acl.is_some()
            && self
                .strategy
                .steps()
                .iter()
                .any(|step| step.builtin_gate() == Some(BuiltinGate::Acl));
        let acl_permitted = pipe_ctx
            .acl_decision
            .as_ref()
            .is_some_and(crate::acl::AccessDecision::is_allowed);
        let mut predicted_changes: Vec<crate::module::Change> = Vec::new();
        let policy_permitted = pipe_ctx.policy_evaluator.is_none()
            || pipe_ctx.policy_effect == Some(PolicyEffect::Allow);
        if let (true, Some(module)) = (
            !acl_denied && (!acl_required || acl_permitted) && policy_permitted,
            pipe_ctx.module.clone(),
        ) {
            let warnings = module.preflight(inputs, ctx);
            checks.push(PreflightCheckResult {
                check: crate::module::MODULE_PREFLIGHT_CHECK_NAME.to_string(),
                passed: true,
                error: None,
                warnings,
            });

            // Module-level preview() (protocol-spec §5.6 / §12.8.5.1).
            // Invoked after preflight in cross-language alignment with
            // apcore-python and apcore-typescript. Panics are treated as
            // advisory and do NOT fail validation.
            //
            // Module trait methods are not UnwindSafe in general (they take
            // `&self`); we deliberately use AssertUnwindSafe because:
            // 1. preview() is contractually side-effect-free per RFC.
            // 2. Even if a module unwinds across borrowed state, the
            //    Registry's Arc<dyn Module> stays valid afterwards — the
            //    panic only crosses the catch_unwind boundary.
            let preview_panic_msg: Option<String> = {
                use std::panic::{catch_unwind, AssertUnwindSafe};
                let module_ref = module.as_ref();
                let result = catch_unwind(AssertUnwindSafe(|| module_ref.preview(inputs, ctx)));
                match result {
                    Ok(Some(preview_result)) => {
                        if !preview_result.changes.is_empty() {
                            predicted_changes = preview_result.changes;
                        }
                        checks.push(PreflightCheckResult {
                            check: crate::module::MODULE_PREVIEW_CHECK_NAME.to_string(),
                            passed: true,
                            error: None,
                            warnings: vec![],
                        });
                        None
                    }
                    Ok(None) => {
                        // Default trait impl returns None — record nothing.
                        // This matches apcore-python's "method absent → no
                        // module_preview check" semantics and keeps the
                        // checks list small for modules that don't preview.
                        None
                    }
                    Err(panic_payload) => {
                        // Extract the panic message from the payload.
                        let msg = if let Some(s) = panic_payload.downcast_ref::<&str>() {
                            (*s).to_string()
                        } else if let Some(s) = panic_payload.downcast_ref::<String>() {
                            s.clone()
                        } else {
                            "<non-string panic payload>".to_string()
                        };
                        Some(msg)
                    }
                }
            };
            if let Some(msg) = preview_panic_msg {
                checks.push(PreflightCheckResult {
                    check: crate::module::MODULE_PREVIEW_CHECK_NAME.to_string(),
                    passed: true,
                    error: None,
                    warnings: vec![format!("preview() panicked: {msg}")],
                });
            }
        }

        let valid = checks.iter().all(|c| c.passed);
        Ok(PreflightResult {
            valid,
            checks,
            requires_approval,
            predicted_changes,
        })
    }

    /// Share this Executor and make it reachable from the contexts it binds.
    ///
    /// A module makes a nested call with the Executor running it
    /// (PROTOCOL_SPEC §5.7): `ctx.executor()` returns it, and passing `ctx` on
    /// propagates the call chain, identity, deadline and cancel token. That
    /// needs a shared Executor, and this is where one becomes shared.
    /// [`APCore`](crate::APCore) does this for the Executor it owns.
    #[must_use]
    pub fn into_shared(self) -> Arc<Self> {
        let shared = Arc::new(self);
        shared.enable_nested_calls();
        shared
    }

    /// Make an Executor already held in an `Arc` reachable from its contexts.
    ///
    /// The counterpart of [`Self::into_shared`] for a caller that built the
    /// `Arc` itself. Idempotent; an Executor can only live in one `Arc`.
    pub fn enable_nested_calls(self: &Arc<Self>) {
        let _ = self.instance_handle.executor.set(Arc::downgrade(self));
    }

    /// Create an executor from a registry and config.
    pub fn from_registry(
        registry: impl Into<Arc<Registry>>,
        config: impl Into<Arc<Config>>,
    ) -> Self {
        Self::new(registry, config)
    }

    /// Stream execution of a module.
    ///
    /// Returns an async `Stream` of output chunks. Each chunk is delivered to
    /// the caller *as soon as it is produced* by the underlying module — no
    /// buffering — so this is true incremental streaming.
    ///
    /// Pipeline phases:
    /// - **Phase 1 (pre-stream):** context creation, call-chain guard, module
    ///   lookup, ACL check, approval gate, before-middleware, input validation.
    ///   Any failure surfaces as the first (and only) `Err` item in the stream.
    /// - **Phase 2 (body):** open the producer through the configured execute
    ///   step, forward each chunk as it arrives, and accumulate it for Phase 3.
    /// - **Phase 3 (post-stream):** after the inner stream is exhausted,
    ///   deep-merge the accumulated chunks and run the strategy's post-stream
    ///   steps (output validation, after-middleware) over the merged result.
    ///   The chunks are already delivered. Ordinary failures are logged and
    ///   published as `apcore.stream.post_validation_failed`; cancellation is yielded.
    ///
    /// When no producer opens, the strategy continues normally. The stream
    /// yields the strategy's output as a single chunk or yields its error.
    pub fn stream<'a>(
        &'a self,
        module_id: &str,
        inputs: Value,
        ctx: Option<&Context<Value>>,
        version_hint: Option<&str>,
    ) -> Pin<Box<dyn Stream<Item = Result<Value, ModuleError>> + Send + 'a>> {
        self.stream_impl(module_id, inputs, ctx.cloned(), version_hint, None)
    }

    /// Stream an invocation with a separate completion result.
    ///
    /// Drive the returned chunks to exhaustion or call `close()` before
    /// awaiting the handle. Dropping the chunks also closes the invocation
    /// and signals its own cancellation scope.
    pub fn stream_with_completion<'a>(
        &'a self,
        module_id: &str,
        inputs: Value,
        ctx: Option<&Context<Value>>,
        version_hint: Option<&str>,
    ) -> (StreamChunks<'a>, StreamCompletionHandle) {
        let mut context = ctx.cloned().unwrap_or_else(Context::anonymous);
        let token = context
            .cancel_token
            .as_ref()
            .map_or_else(CancelToken::new, CancelToken::child);
        context.cancel_token = Some(token.clone());
        let names = self.strategy.step_names();
        let configured = |name: &str| names.iter().any(|step| step == name);
        let (recorder, completion) = StreamRecorder::new(
            configured("output_validation"),
            configured("middleware_after"),
        );
        let stream = self.stream_impl(
            module_id,
            inputs,
            Some(context),
            version_hint,
            Some(recorder.clone()),
        );
        (StreamChunks::new(stream, recorder, token), completion)
    }

    fn stream_impl<'a>(
        &'a self,
        module_id: &str,
        inputs: Value,
        initial_context: Option<Context<Value>>,
        version_hint: Option<&str>,
        recorder: Option<StreamRecorder>,
    ) -> Pin<Box<dyn Stream<Item = Result<Value, ModuleError>> + Send + 'a>> {
        // Capture by value so the returned Stream is `'a` (only borrowing &self).
        let module_id_owned = module_id.to_string();
        let version_hint_owned = version_hint.map(str::to_string);

        Box::pin(async_stream::try_stream! {
            // Fail fast for invalid module IDs before setting up any pipeline state.
            Self::validate_module_id(&module_id_owned)?;

            // Phase 1: pre-stream setup. A pre-execute failure runs the
            // middleware on_error recovery chain (A-D-001): a recovery value is
            // yielded as the stream's chunk; otherwise the error short-circuits
            // the whole stream.
            let mut setup = match self
                .prepare_stream(
                    &module_id_owned,
                    inputs,
                    initial_context,
                    version_hint_owned.as_deref(),
                    recorder.as_ref(),
                )
                .await?
            {
                StreamPrep::Setup(setup) => *setup,
                StreamPrep::Recovery(value, error) => {
                    if let Some(recorder) = recorder.as_ref() {
                        recorder.record_failure(&error);
                    }
                    yield value;
                    return;
                }
            };

            self.run_stream_execute(&mut setup, recorder.as_ref()).await?;
            let mut accumulated = Value::Object(serde_json::Map::new());

            let producer = setup.pipe_ctx.stream.as_mut().and_then(|stream| stream.producer.take());
            if let Some(producer) = producer {
                let mut inner = producer.into_inner();
                let mut chunk_index = 0;
                loop {
                    let chunk_result = match next_stream_chunk(&mut inner, &setup).await {
                        Ok(Some(chunk)) => chunk,
                        Ok(None) => break,
                        Err(error) => {
                            setup.settle_retiring_producer(inner, &error);
                            Err(error)?;
                            return;
                        }
                    };
                    let chunk = match chunk_result {
                        Ok(c) => c,
                        Err(chunk_err) => {
                            // A-D-015: a non-cancellation error raised while
                            // iterating chunks runs the middleware on_error
                            // recovery chain over the Phase-1 executed
                            // middlewares; a recovery value is yielded as the
                            // stream's next chunk before the stream ends.
                            // Mirrors apcore-python (executor.py:1069) and
                            // apcore-typescript. Cancellation short-circuits
                            // (D-20): it bypasses on_error and surfaces raw.
                            let decorated = propagate_module_error(
                                chunk_err,
                                &module_id_owned,
                                setup.context(),
                            );
                            // `recover_stream_chunk_error` returns Some(recovery)
                            // to yield, or None to surface `decorated` raw
                            // (cancellation, no middleware, no/Retry outcome).
                            if let Some(recovery) =
                                Self::recover_stream_chunk_error(&setup, &decorated, &module_id_owned).await
                            {
                                if let Some(recorder) = recorder.as_ref() {
                                    recorder.record_failure(&decorated);
                                }
                                yield recovery;
                                return;
                            }
                            Err(decorated)?;
                            return;
                        }
                    };
                    // Audit D10-001: reject a non-object chunk BEFORE delivering
                    // it. The canonical contract is "reject before deliver +
                    // raise": a chunk is valid iff it is a JSON object. On the
                    // first invalid chunk we surface a structured error and do
                    // NOT yield the bad chunk. We mirror the exact error shape
                    // produced by `deep_merge_chunks_checked` (same code,
                    // message, and details) so cross-language consumers match.
                    if !chunk.is_object() {
                        Err(stream_chunk_not_object_error(chunk_index, &chunk))?;
                        return;
                    }
                    deep_merge_value(&mut accumulated, &chunk, 0, setup.merge_depth);
                    chunk_index += 1;
                    yield chunk;
                }
            } else {
                let output = setup.pipe_ctx.output.take().unwrap_or(Value::Null);
                if let Some(recorder) = recorder.as_ref() {
                    recorder.capture_if_missing(&output);
                }
                yield output;
                return;
            }

            // Phase 3: post-stream validation + middleware_after on the merged
            // output. Ordinary failures are reported out of band.
            Self::run_stream_phase3(&self.strategy, &mut setup, accumulated, &module_id_owned, recorder.as_ref()).await?;
        })
    }

    /// Step names from `execute` to the end of `strategy`, in strategy order.
    ///
    /// Empty when the strategy has no `execute` step — nothing then runs the
    /// module, exactly as on the `call()` path.
    fn execute_and_later_step_names(strategy: &ExecutionStrategy) -> Vec<String> {
        let names = strategy.step_names();
        names
            .iter()
            .position(|n| n == "execute")
            .map_or_else(Vec::new, |idx| names[idx..].to_vec())
    }

    /// Continue the configured strategy, pausing after execute only if it
    /// opened a native producer. The ordinary fallback runs to completion.
    async fn run_stream_execute(
        &self,
        setup: &mut StreamSetup,
        recorder: Option<&StreamRecorder>,
    ) -> Result<(), ModuleError> {
        let steps = Self::execute_and_later_step_names(&self.strategy);
        // A skip_to may already have crossed execute and processed the tail.
        if steps.is_empty()
            || setup
                .pipe_ctx
                .trace
                .steps
                .iter()
                .any(|step| step.name == "execute")
        {
            return Ok(());
        }
        let mut options = RunOptions::run_until(|state| {
            state
                .context
                .stream
                .as_ref()
                .is_some_and(|stream| stream.producer.is_some())
        });
        options.only_steps = Some(steps);
        let result =
            Self::run_stream_segment(&self.strategy, &mut setup.pipe_ctx, options, recorder).await;
        let raw_error = match result {
            Ok((_, trace)) => return Self::reject_aborted_trace(&trace),
            Err(error) => error,
        };
        // A failed after-step hook can leave an opened producer in the context.
        if let Some(stream) = &mut setup.pipe_ctx.stream {
            stream.producer = None;
        }
        let interrupted = matches!(
            raw_error.code,
            ErrorCode::ExecutionCancelled | ErrorCode::ModuleTimeout
        );
        let error = Self::call_error(&setup.module_id, &setup.pipe_ctx, raw_error);
        if interrupted {
            return Err(error);
        }
        let context = setup.context();
        let recovery = tokio::select! {
            biased;
            outcome = Self::recover_call(&setup.module_id, &setup.pipe_ctx, &error) => outcome,
            interruption = wait_stream_interruption(
                context.cancel_token.as_ref(), context.global_deadline,
                &setup.module_id, setup.module.as_ref(),
            ) => return Err(interruption),
        };
        if let Some(OnErrorOutcome::Recovery(value)) = recovery {
            if let Some(recorder) = recorder {
                recorder.record_failure(&error);
            }
            setup.pipe_ctx.output = Some(value);
            return Ok(());
        }
        Err(error)
    }

    /// The post-stream steps of `strategy`, in strategy order.
    ///
    /// STR-2: the same filter apcore-python applies when it assembles its
    /// `post_stream` sub-strategy (`s.name in ("output_validation",
    /// "middleware_after", "return_result")`) and apcore-typescript applies
    /// inline. Reading them off the ACTUAL strategy is the point: on the
    /// `minimal` preset, which removes `output_validation` and
    /// `middleware_after`, this list is empty and a streamed call does neither —
    /// matching what `call()` does on that preset. A step REPLACED under one of
    /// these names runs in its predecessor's place, as it does everywhere else.
    fn post_stream_step_names(strategy: &ExecutionStrategy) -> Vec<String> {
        const POST_STREAM_STEPS: [&str; 3] =
            ["output_validation", "middleware_after", "return_result"];
        strategy
            .step_names()
            .into_iter()
            .filter(|name| POST_STREAM_STEPS.contains(&name.as_str()))
            .collect()
    }

    /// Run the strategy's post-stream steps over the accumulated result.
    /// Report ordinary failures as diagnostics and propagate cancellation to the consumer.
    ///
    /// The steps are the strategy's own, not a hardcoded "validate + fire
    /// after()" pair (STR-2). Running the real `output_validation` step is also
    /// what populates `context.redacted_output` for a streamed call, so its
    /// audit record carries the same output projection its non-streamed twin
    /// does (MW-002).
    async fn run_stream_phase3(
        strategy: &ExecutionStrategy,
        setup: &mut StreamSetup,
        accumulated: Value,
        module_id: &str,
        recorder: Option<&StreamRecorder>,
    ) -> Result<(), ModuleError> {
        let post_steps = Self::post_stream_step_names(strategy);
        if post_steps.is_empty() {
            if let Some(recorder) = recorder {
                recorder.capture_if_missing(&accumulated);
            }
            return Ok(());
        }
        setup.pipe_ctx.output = Some(accumulated);
        let prior_steps = setup.pipe_ctx.trace.steps.len();
        let result = Self::run_stream_segment(
            strategy,
            &mut setup.pipe_ctx,
            RunOptions::only_steps(post_steps),
            recorder,
        )
        .await;
        // A fatal step outside the public output-step fields (for example
        // return_result) stays observable even when legacy streams end normally.
        let unrepresented = recorder.filter(|_| {
            !setup.pipe_ctx.trace.steps[prior_steps..]
                .last()
                .is_some_and(|step| {
                    step.result.action == "abort"
                        && matches!(step.name.as_str(), "output_validation" | "middleware_after")
                })
        });
        match result {
            Ok((_, trace)) => {
                if trace.success {
                    if let (Some(recorder), Some(output)) =
                        (recorder, setup.pipe_ctx.output.as_ref())
                    {
                        recorder.capture_if_missing(output);
                    }
                }
                if let Some(recorder) = unrepresented {
                    if let Err(error) = Self::reject_aborted_trace(&trace) {
                        recorder.record_failure(&error);
                    }
                }
            }
            Err(e) => {
                let underlying = e.unwrap_pipeline_step_error().unwrap_or(e);
                let underlying = underlying
                    .unwrap_middleware_chain_error()
                    .unwrap_or(underlying);
                if matches!(
                    underlying.code,
                    ErrorCode::ExecutionCancelled | ErrorCode::ModuleTimeout
                ) {
                    return Err(underlying);
                }
                if let Some(recorder) = unrepresented {
                    recorder.record_failure(&underlying);
                }
                tracing::warn!(
                    module_id = %module_id,
                    error = %underlying.message,
                    "stream phase-3 post steps failed (chunks already delivered, swallowed)"
                );
                Self::emit_post_stream_failure(setup, module_id, &underlying).await;
            }
        }
        Ok(())
    }

    async fn run_stream_segment(
        strategy: &ExecutionStrategy,
        ctx: &mut PipelineContext,
        options: RunOptions,
        recorder: Option<&StreamRecorder>,
    ) -> Result<(Option<Value>, PipelineTrace), ModuleError> {
        let token = ctx.context.cancel_token.clone();
        let deadline = ctx.context.global_deadline;
        let module = ctx.module.clone();
        let module_id = ctx.module_id.clone();
        let prior_steps = ctx.trace.steps.len();
        let mut observe_input = |name: &str, ctx: &PipelineContext| {
            if let Some(recorder) = recorder {
                recorder.observe_step_input(name, ctx.output.as_ref());
            }
        };
        let pipeline = PipelineEngine::run_with_step_input(
            strategy,
            ctx,
            options,
            if recorder.is_some() {
                Some(&mut observe_input)
            } else {
                None
            },
        );
        let result = if let Some(module) = module {
            tokio::select! {
                biased;
                error = wait_stream_interruption(token.as_ref(), deadline, &module_id, module.as_ref()) => Err(error),
                result = pipeline => result,
            }
        } else {
            pipeline.await
        };
        if let Some(recorder) = recorder {
            recorder.record_steps(&ctx.trace.steps[prior_steps..]);
        }
        result
    }

    /// Publish `apcore.stream.post_validation_failed` when a post-stream failure
    /// is swallowed (chunks already delivered, cannot be un-sent). Best-effort:
    /// a no-op when no event emitter is configured. Mirrors apcore-python's
    /// `_emit_post_stream_failure` payload (`error_type`, `message`, `trace_id`).
    async fn emit_post_stream_failure(setup: &StreamSetup, module_id: &str, error: &ModuleError) {
        let Some(emitter) = setup.event_emitter() else {
            return;
        };
        let event = ApCoreEvent::with_module(
            "apcore.stream.post_validation_failed",
            serde_json::json!({
                // `error_type`, NOT `code` — deliberately a class name rather
                // than a wire code, matching apcore-python
                // (`type(exc).__name__`, executor.py:1168) and
                // apcore-typescript (`cause.constructor?.name`,
                // executor.ts:818). Rust has no per-code error class, so the
                // `ErrorCode` variant name is the closest analogue and `Debug`
                // is correct here — unlike a `code` field, which MUST use
                // `wire_str()`.
                "error_type": format!("{:?}", error.code),
                "message": error.message.clone(),
                "trace_id": setup.context().trace_id.clone(),
            }),
            module_id,
            "error",
        );
        emitter.emit(&event).await;
    }

    /// Run the middleware on_error recovery chain for a Phase-2 (mid-stream)
    /// chunk error. Returns `Some(recovery_value)` to yield as the stream's
    /// next chunk, or `None` when the original (decorated) error should be
    /// surfaced raw — i.e. there is no middleware manager, no middleware ran,
    /// the outcome was `None`, or the outcome was `Retry` (not meaningful
    /// mid-stream; re-raise per Python/TS). A-D-015.
    async fn recover_stream_chunk_error(
        setup: &StreamSetup,
        decorated: &ModuleError,
        module_id: &str,
    ) -> Option<Value> {
        let outcome = Self::recover_call(module_id, &setup.pipe_ctx, decorated).await;
        match outcome {
            Some(crate::middleware::base::OnErrorOutcome::Recovery(value)) => Some(value),
            Some(crate::middleware::base::OnErrorOutcome::Retry(_)) => {
                tracing::warn!(
                    module_id = %module_id,
                    "Retry requested during stream phase 2 — ignored; surfacing original error"
                );
                None
            }
            None => None,
        }
    }

    /// Run Phase 1 of the streaming pipeline: every step up to (but not
    /// including) `execute`. Returns the resolved module and the Phase-1
    /// pipeline context, or a recovery value from the on_error chain.
    ///
    /// Drives the shared [`PipelineEngine::run_until_step`] so every per-step
    /// declaration (`match_modules`, `ignore_errors`, `timeout_ms`, `dry_run`
    /// purity filtering, `skip_to` targets) behaves identically to the
    /// non-streaming `call()` path.
    async fn prepare_stream(
        &self,
        module_id: &str,
        inputs: Value,
        ctx: Option<Context<Value>>,
        version_hint: Option<&str>,
        recorder: Option<&StreamRecorder>,
    ) -> Result<StreamPrep, ModuleError> {
        self.ensure_startup_configuration()?;
        // D-103: no synthetic principal for a call that supplied none.
        let context = ctx.unwrap_or_else(Context::<Value>::anonymous);

        let mut pipe_ctx = PipelineContext::new(module_id, inputs, context, self.strategy.name());
        pipe_ctx.validate_approval_credential()?;
        pipe_ctx.stream = Some(crate::pipeline::StreamPipelineState::default());
        if let Some(hint) = version_hint {
            pipe_ctx.version_hint = Some(hint.to_string());
        }
        self.inject_resources(&mut pipe_ctx).await;
        self.bind_to_context(&mut pipe_ctx)?;

        // Drive the shared pipeline engine up to — but not including — the
        // `execute` step. This inherits all per-step metadata handling from
        // `PipelineEngine::run`, so streaming and non-streaming never diverge.
        let (_output, trace) = match Self::run_stream_segment(
            &self.strategy,
            &mut pipe_ctx,
            RunOptions::stop_before("execute"),
            recorder,
        )
        .await
        {
            Ok(pair) => pair,
            Err(e) => {
                let underlying = Self::call_error(module_id, &pipe_ctx, e);
                match Self::recover_call(module_id, &pipe_ctx, &underlying).await {
                    Some(OnErrorOutcome::Recovery(value)) => {
                        return Ok(StreamPrep::Recovery(value, Box::new(underlying)));
                    }
                    Some(OnErrorOutcome::Retry(_)) => {
                        tracing::warn!(
                            module_id = %module_id,
                            "Retry requested during stream phase 1; surfacing original error"
                        );
                    }
                    None => {}
                }
                return Err(underlying);
            }
        };

        // `run_until` returns `Ok` for pipeline-level aborts (so the caller
        // can observe the trace). Streaming requires a resolved module, so
        // an aborted pre-execute phase is an error for us.
        Self::reject_aborted_trace(&trace)?;

        let module = pipe_ctx.module.clone().ok_or_else(|| {
            ModuleError::new(
                ErrorCode::ModuleNotFound,
                format!("Module '{module_id}' was not resolved during pre-stream setup"),
            )
        })?;

        Ok(StreamPrep::Setup(Box::new(
            self.build_stream_setup(module, module_id, pipe_ctx),
        )))
    }

    /// Assemble the [`StreamSetup`] Phase 2 and Phase 3 run from.
    ///
    /// Extracted from `prepare_stream` so that function stays inside clippy's
    /// `too_many_lines` bound; it is pure assembly and holds no logic of its
    /// own. The configuration read lives here because it is resolved ONCE per
    /// stream: a configuration mutated mid-stream must not change the merge
    /// cap of a stream already in flight.
    fn build_stream_setup(
        &self,
        module: Arc<dyn crate::module::Module>,
        module_id: &str,
        pipe_ctx: PipelineContext,
    ) -> StreamSetup {
        let config = pipe_ctx.config.as_deref().unwrap_or(self.config.as_ref());
        StreamSetup {
            module,
            module_id: module_id.to_string(),
            merge_depth: resolve_merge_depth(config),
            pipe_ctx,
        }
    }

    /// Get a reference to the executor's execution strategy.
    pub fn strategy(&self) -> &ExecutionStrategy {
        &self.strategy
    }

    /// List registered module IDs with an approval declaration the built-in gate would skip.
    ///
    /// Use after registration to report the default strategy's approval posture.
    /// Includes hidden modules and evaluates policy without a call site. Caller-specific
    /// ACL requirements are evaluated during each invocation.
    #[must_use]
    pub fn ungated_approval_modules(&self) -> Vec<String> {
        let policy = ExecutionPolicy::effective(self.policy.as_deref());
        let enforcing_gate = (self.approval_handler.is_some() || policy.strict)
            && self
                .strategy
                .steps()
                .iter()
                .any(|step| step.builtin_gate() == Some(BuiltinGate::Approval));

        self.registry
            .module_ids_full(true)
            .into_iter()
            .filter(|id| {
                let Some(annotations) = self.registry.governance_annotations(id) else {
                    return false;
                };
                let needs_approval = policy.resolve(id, Some(&annotations)).needs_approval;
                (annotations.requires_approval || needs_approval)
                    && !(enforcing_gate && needs_approval)
            })
            .collect()
    }

    /// Return what is actually gating this executor (PROTOCOL_SPEC 6.6.5).
    ///
    /// A **pure read**: it never enforces, warns, panics or mutates. What to do
    /// about an unprotected control surface belongs to the caller — a
    /// serve-time adapter may warn or refuse, a test may assert, a health
    /// endpoint may report. Putting the reaction here would make it
    /// unavoidable and untestable.
    ///
    /// Computed fresh on every call, so attaching an ACL or swapping the
    /// strategy is visible in the next one.
    #[must_use]
    pub fn governance_state(&self) -> GovernanceState {
        // Gate detection is by CAPABILITY, never by step name
        // (PROTOCOL_SPEC 6.6.5.2). `Step::builtin_gate` defaults to `None`, so
        // a custom step calling itself `acl_check` cannot be mistaken for the
        // gate — reporting a gate that is not there is the one direction this
        // accessor must never fail in.
        let builtin_acl_gate_wired = self
            .strategy
            .steps()
            .iter()
            .any(|step| step.builtin_gate() == Some(BuiltinGate::Acl));
        let builtin_approval_gate_wired = self
            .strategy
            .steps()
            .iter()
            .any(|step| step.builtin_gate() == Some(BuiltinGate::Approval));

        // `visibility` includes hidden: the accessor reports what is
        // REGISTERED, and the default is public-only. A control module
        // registered with `discoverable: false` is still callable by ID, so
        // omitting it would under-report the write surface.
        let all_ids = self.registry.list(None, None, Some(&["public", "hidden"]));
        let control_ids: Vec<&String> = all_ids
            .iter()
            .filter(|id| id.starts_with("system.control."))
            .collect();
        let read_modules_registered = all_ids.iter().any(|id| {
            id.starts_with("system.health.")
                || id.starts_with("system.usage.")
                || id.starts_with("system.manifest.")
        });

        // Read governance through `governance_union` — the same source the
        // approval gate reads when it decides whether to fire. A second reading
        // here would be a second way for the accessor to disagree with the
        // pipeline it describes, and reading the descriptor alone (as this did)
        // is exactly that: a `system.control.*` module declaring
        // `requires_approval` on its INSTANCE, with a descriptor that omits it,
        // is gated by the pipeline and was reported here as ungated.
        let all_control_modules_require_approval = !control_ids.is_empty()
            && control_ids.iter().all(|id| {
                self.registry
                    .governance_annotations(id)
                    .is_some_and(|a| a.requires_approval)
            });

        // PROTOCOL_SPEC 6.6.5.5 (D-129) requires these to report what the
        // running built-in gate holds. In this SDK the gate steps hold no
        // provider: `inject_resources` hands every call the executor's ACL,
        // handler and policy, and the gates read them from the
        // `PipelineContext`. However the strategy was supplied, the executor
        // fields ARE what the running gate enforces, so reading them here
        // cannot drift from the gate.
        let acl_configured = self.acl.is_some();
        let approval_handler_configured = self.approval_handler.is_some();
        let policy_strict = if builtin_approval_gate_wired {
            ExecutionPolicy::effective(self.policy.as_deref()).strict
        } else {
            self.policy.as_deref().is_some_and(|policy| policy.strict)
        };

        // PROTOCOL_SPEC 6.6.5.1. The approval conjunct carries
        // `all_control_modules_require_approval` because `approval_gate` is
        // per-module conditional while `acl_check` is not (6.6.5.1.1).
        // Written to mirror PROTOCOL_SPEC 6.6.5.1 term for term. clippy offers a
        // De Morgan rewrite; taking it would leave the code and the normative
        // formula looking different, which is how a transcription error hides.
        #[allow(clippy::nonminimal_bool)]
        let unprotected_control_surface = !control_ids.is_empty()
            && !(acl_configured && builtin_acl_gate_wired)
            && !(builtin_approval_gate_wired
                && all_control_modules_require_approval
                && (approval_handler_configured || policy_strict));

        GovernanceState {
            control_modules_registered: !control_ids.is_empty(),
            read_modules_registered,
            acl_configured,
            builtin_acl_gate_wired,
            approval_handler_configured,
            builtin_approval_gate_wired,
            policy_strict,
            all_control_modules_require_approval,
            unprotected_control_surface,
        }
    }

    /// Return structured info about the configured pipeline.
    ///
    /// Returns a [`StrategyInfo`] describing the strategy name, step count,
    /// step names, and auto-generated description. This matches the spec and
    /// aligns with the Python and TypeScript SDK return types.
    ///
    /// Use `.to_string()` on the result for a human-readable summary.
    pub fn describe_pipeline(&self) -> StrategyInfo {
        self.strategy.info()
    }

    /// Strategy summaries for this executor: its current strategy first, then
    /// every strategy registered via [`register_strategy`].
    ///
    /// Per `core-executor.md` "Contract: Executor.list_strategies" the result
    /// carries "one entry for the executor's current strategy plus one for
    /// every strategy registered". The module-level [`list_strategies`]
    /// function answers a different question — it reports the global registry
    /// only, and `STRATEGY_REGISTRY` is not pre-seeded with the built-ins, so
    /// it returns an empty Vec for a default executor. `design-execution-
    /// pipeline.md` publishes this instance-scoped signature; AI strategy
    /// selection reads it, and an empty menu is not a usable answer.
    pub fn list_strategies(&self) -> Vec<StrategyInfo> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut result: Vec<StrategyInfo> = Vec::new();

        // Current strategy first, matching apcore-python's ordering.
        let current = self.strategy.info();
        seen.insert(current.name.clone());
        result.push(current);

        for info in list_strategies() {
            if seen.insert(info.name.clone()) {
                result.push(info);
            }
        }
        result
    }

    /// Execute a module through the pipeline engine, returning both the output
    /// and a full execution trace.
    ///
    /// Uses the provided `strategy` override, or the executor's default strategy.
    ///
    /// Shares the canonical error-recovery semantics of [`Self::call`] per
    /// spec docs/features/core-executor.md §Trace Variants (D-19) —
    /// middleware `on_error` recovery applies, the
    /// `ExecutionCancelled` short-circuit (D-20) applies, and the
    /// `MiddlewareChainError` unwrap rule (D-22) applies. On successful
    /// recovery the returned `PipelineTrace` is the trace captured during
    /// the failing pipeline run (including any `on_error` events recorded
    /// inside it).
    /// Convert a pipeline trace carrying `success == false` into the
    /// `PIPELINE_ABORT` error a caller expects.
    ///
    /// `PipelineEngine::run` reports a step-returned `action: "abort"` as a
    /// successful return with the flag cleared, not as an `Err`. Shared by
    /// `call` and `call_with_trace` so the two cannot drift on it again.
    fn reject_aborted_trace(trace: &crate::pipeline::PipelineTrace) -> Result<(), ModuleError> {
        if trace.success {
            return Ok(());
        }
        let (aborted_step, explanation) = trace
            .steps
            .iter()
            .find_map(|s| {
                if s.result.action == "abort" {
                    Some((s.name.as_str(), s.result.explanation.as_deref()))
                } else {
                    None
                }
            })
            .unwrap_or(("unknown", None));
        Err(ModuleError::pipeline_abort(aborted_step, explanation))
    }

    pub async fn call_with_trace(
        &self,
        module_id: &str,
        inputs: Value,
        ctx: Option<&Context<Value>>,
        version_hint: Option<&str>,
        strategy: Option<&ExecutionStrategy>,
    ) -> Result<(Value, PipelineTrace), ModuleError> {
        self.execute_call(
            module_id,
            inputs,
            ctx,
            version_hint,
            strategy.unwrap_or(&self.strategy),
            true,
        )
        .await
    }

    #[cfg(feature = "sys-modules")]
    pub(crate) fn set_runtime_config(&self, config: Arc<tokio::sync::Mutex<Config>>) {
        *self.runtime_config.write() = Some(Arc::new(RuntimeConfig {
            source: config,
            snapshot: parking_lot::Mutex::new(None),
        }));
    }

    /// The resolved capture-point redaction rules, built on first use.
    fn redaction(&self) -> &Arc<RedactionConfig> {
        self.redaction
            .get_or_init(|| Arc::new(RedactionConfig::from_config(&self.config)))
    }

    /// Capture one configuration snapshot for the pipeline, including its redaction rules.
    async fn inject_resources(&self, ctx: &mut PipelineContext) {
        let runtime_config = self.runtime_config.read().clone();
        let snapshot = if let Some(runtime_config) = runtime_config {
            runtime_config.snapshot().await
        } else {
            ConfigSnapshot {
                config: Arc::clone(&self.config),
                redaction: Arc::clone(self.redaction()),
            }
        };
        ctx.registry = Some(Arc::clone(&self.registry));
        ctx.config = Some(snapshot.config);
        ctx.redaction = Some(snapshot.redaction);
        ctx.acl = self.acl.as_ref().map(Arc::clone);
        ctx.approval_handler = self.approval_handler.as_ref().map(Arc::clone);
        ctx.policy = self.policy.as_ref().map(Arc::clone);
        ctx.policy_evaluator = self.policy_evaluator.as_ref().map(Arc::clone);
        ctx.event_emitter = self.event_emitter.as_ref().map(Arc::clone);
        ctx.governance_warned = Some(Arc::clone(&self.governance_warned));
        ctx.middleware_manager = Some(Arc::clone(&self.middleware_manager));
    }

    /// Bind this Executor's identity handle to `ctx.context.executor` per
    /// apcore Issue #66 / `core-executor.md` §"Contract: Executor binding to
    /// Context". Called before pipeline step 1 from every top-level entry
    /// point (`call`, `call_with_trace`, `validate`, `stream` via
    /// `prepare_stream`).
    ///
    /// Errors out with [`ErrorCode::ContextBindingError`] when the supplied
    /// Context is already bound to a different Executor instance.
    fn bind_to_context(&self, ctx: &mut PipelineContext) -> Result<(), ModuleError> {
        ctx.context.bind_executor(self.instance_handle())
    }

    /// Type-erased Executor identity handle.
    ///
    /// Returned as `Arc<dyn Any + Send + Sync>` because `Context::executor` is
    /// non-generic over the Executor type. Two Executors are considered the
    /// same instance iff the underlying `Arc`s point to the same allocation
    /// (`Arc::ptr_eq`). The pointed-to payload is opaque to callers — only
    /// the identity matters.
    ///
    /// Marked `doc(hidden)` because it exists to support binding-aware
    /// integrations (e.g. test fixtures, distributed runtimes injecting a
    /// pre-bound Context); application code SHOULD let the Executor bind
    /// itself implicitly at pipeline entry instead.
    #[doc(hidden)]
    #[must_use]
    pub fn instance_handle(&self) -> Arc<dyn std::any::Any + Send + Sync> {
        // Coerce `Arc<()>` → `Arc<dyn Any + Send + Sync>` via the implicit
        // unsized cast performed by `let` annotation; this preserves identity
        // because `Arc::clone` increments the same strong-count.
        let h: Arc<dyn std::any::Any + Send + Sync> = self.instance_handle.clone();
        h
    }

    /// Add a before middleware.
    pub fn use_before(
        &self,
        middleware: Box<dyn BeforeMiddleware>,
    ) -> Result<MiddlewareHandle, ModuleError> {
        self.middleware_manager
            .add(Box::new(BoxedBeforeMiddlewareAdapter(middleware)))
    }

    /// Add an after middleware.
    pub fn use_after(
        &self,
        middleware: Box<dyn AfterMiddleware>,
    ) -> Result<MiddlewareHandle, ModuleError> {
        self.middleware_manager
            .add(Box::new(BoxedAfterMiddlewareAdapter(middleware)))
    }
}

// These boxed adapters wrap `Box<dyn BeforeMiddleware>` / `Box<dyn AfterMiddleware>`
// (unsized trait objects) into the full `Middleware` trait. They are private to
// this module because they are only needed by `Executor::use_before` /
// `Executor::use_after`.

/// Wraps a boxed BeforeMiddleware into a full Middleware trait object.
struct BoxedBeforeMiddlewareAdapter(Box<dyn BeforeMiddleware>);

impl std::fmt::Debug for BoxedBeforeMiddlewareAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoxedBeforeMiddlewareAdapter")
            .field("name", &self.0.name())
            .finish()
    }
}

#[async_trait::async_trait]
impl Middleware for BoxedBeforeMiddlewareAdapter {
    fn name(&self) -> &str {
        self.0.name()
    }

    async fn before(
        &self,
        module_id: &str,
        inputs: serde_json::Value,
        ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        self.0.before(module_id, inputs, ctx).await
    }

    async fn after(
        &self,
        _module_id: &str,
        _inputs: serde_json::Value,
        _output: serde_json::Value,
        _ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        Ok(None)
    }

    async fn on_error(
        &self,
        _module_id: &str,
        _inputs: serde_json::Value,
        _error: &ModuleError,
        _ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        Ok(None)
    }
}

/// Wraps a boxed AfterMiddleware into a full Middleware trait object.
struct BoxedAfterMiddlewareAdapter(Box<dyn AfterMiddleware>);

impl std::fmt::Debug for BoxedAfterMiddlewareAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoxedAfterMiddlewareAdapter")
            .field("name", &self.0.name())
            .finish()
    }
}

#[async_trait::async_trait]
impl Middleware for BoxedAfterMiddlewareAdapter {
    fn name(&self) -> &str {
        self.0.name()
    }

    async fn before(
        &self,
        _module_id: &str,
        _inputs: serde_json::Value,
        _ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        Ok(None)
    }

    async fn after(
        &self,
        module_id: &str,
        inputs: serde_json::Value,
        output: serde_json::Value,
        ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        self.0.after(module_id, inputs, output, ctx).await
    }

    async fn on_error(
        &self,
        _module_id: &str,
        _inputs: serde_json::Value,
        _error: &ModuleError,
        _ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approval::{ApprovalHandler, ApprovalRequest, ApprovalResult};
    use crate::config::Config;
    use crate::context::Context;
    use crate::errors::ErrorCode;
    use crate::module::{Module, ModuleAnnotations};
    use crate::registry::registry::{ModuleDescriptor, Registry};
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::Mutex;

    // ── Mock module ──────────────────────────────────────────────────

    struct MockModule {
        input_schema: Value,
        output_schema: Value,
        output: Value,
    }

    impl MockModule {
        fn new(input_schema: Value, output_schema: Value, output: Value) -> Self {
            Self {
                input_schema,
                output_schema,
                output,
            }
        }

        fn echo() -> Self {
            Self::new(json!({}), json!({}), json!({"ok": true}))
        }
    }

    #[async_trait]
    impl Module for MockModule {
        fn input_schema(&self) -> Value {
            self.input_schema.clone()
        }
        fn output_schema(&self) -> Value {
            self.output_schema.clone()
        }
        fn description(&self) -> &'static str {
            "mock module"
        }
        async fn execute(
            &self,
            _inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Value, ModuleError> {
            Ok(self.output.clone())
        }
    }

    /// A module that returns its own inputs, so a test can assert exactly what
    /// reached `execute()`.
    struct EchoInputsModule {
        input_schema: Value,
    }

    #[async_trait]
    impl Module for EchoInputsModule {
        fn input_schema(&self) -> Value {
            self.input_schema.clone()
        }
        fn output_schema(&self) -> Value {
            json!({})
        }
        fn description(&self) -> &'static str {
            "echo inputs module"
        }
        async fn execute(
            &self,
            inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Value, ModuleError> {
            Ok(inputs)
        }

        fn preflight(&self, inputs: &Value, _ctx: Option<&Context<Value>>) -> Vec<String> {
            if inputs.get("_approval_token").is_some() {
                return vec!["preflight observed a reserved input".to_owned()];
            }
            vec![]
        }
    }

    // ── Mock approval handler ────────────────────────────────────────

    /// Tracks which method was called (request vs check) and returns a
    /// configurable ApprovalResult.
    #[derive(Debug)]
    struct MockApprovalHandler {
        /// Result returned by both request_approval and check_approval.
        result: ApprovalResult,
        /// Records "request" or "check:<token>" for each call.
        calls: Mutex<Vec<String>>,
    }

    impl MockApprovalHandler {
        fn with_status(status: &str) -> Self {
            Self {
                result: ApprovalResult {
                    status: status.to_string(),
                    approved_by: None,
                    reason: Some(format!("mock-{status}")),
                    approval_id: None,
                    audit_id: None,
                    metadata: None,
                },
                calls: Mutex::new(vec![]),
            }
        }
    }

    #[async_trait]
    impl ApprovalHandler for MockApprovalHandler {
        async fn request_approval(
            &self,
            _request: &ApprovalRequest,
        ) -> Result<ApprovalResult, ModuleError> {
            self.calls.lock().unwrap().push("request".to_string());
            Ok(self.result.clone())
        }

        async fn check_approval(&self, _approval_id: &str) -> Result<ApprovalResult, ModuleError> {
            panic!("The status query cannot authorize execution");
        }

        async fn check_approval_for_call(
            &self,
            approval_id: &str,
            request: &ApprovalRequest,
        ) -> Result<ApprovalResult, ModuleError> {
            assert!(request.arguments.get("_approval_token").is_none());
            assert_eq!(request.module_id, "test_mod");
            self.calls
                .lock()
                .unwrap()
                .push(format!("check:{approval_id}"));
            Ok(self.result.clone())
        }
    }

    // ── Helper: build executor with a registered module ──────────────

    /// Wraps a test module so its LIVE `annotations()` reports the declared
    /// governance values. `BuiltinApprovalGate` and `validate()` both read
    /// `module.annotations()` (PROTOCOL_SPEC §7.4 Step 5), so putting the
    /// annotations only on the descriptor would declare nothing they consult.
    struct AnnotatedModule<M: Module> {
        inner: M,
        annotations: ModuleAnnotations,
    }

    #[async_trait]
    impl<M: Module> Module for AnnotatedModule<M> {
        fn input_schema(&self) -> Value {
            self.inner.input_schema()
        }
        fn output_schema(&self) -> Value {
            self.inner.output_schema()
        }
        fn description(&self) -> &str {
            self.inner.description()
        }
        fn annotations(&self) -> ModuleAnnotations {
            self.annotations.clone()
        }
        async fn execute(&self, inputs: Value, ctx: &Context<Value>) -> Result<Value, ModuleError> {
            self.inner.execute(inputs, ctx).await
        }
    }

    fn build_executor_with_module<M: Module + 'static>(
        module: M,
        annotations: ModuleAnnotations,
    ) -> Executor {
        let registry = Registry::new();
        let module = AnnotatedModule {
            inner: module,
            annotations: annotations.clone(),
        };
        let descriptor = ModuleDescriptor {
            module_id: "test_mod".to_string(),
            name: None,
            description: module.description().to_string(),
            documentation: None,
            input_schema: module.input_schema(),
            output_schema: module.output_schema(),
            version: "1.0.0".to_string(),
            tags: vec![],
            annotations: Some(annotations),
            examples: vec![],
            metadata: std::collections::HashMap::new(),
            display: None,
            sunset_date: None,
            dependencies: vec![],
            enabled: true,
        };
        registry
            .register("test_mod", Box::new(module), descriptor)
            .unwrap();
        Executor::new(registry, Config::default())
    }

    // ═══════════════════════════════════════════════════════════════════
    // deep_merge depth cap (sync STREAM-001)
    // ═══════════════════════════════════════════════════════════════════

    /// Build a chain of N nested objects: {"a": {"a": {"a": {... value}}}}.
    fn build_nested(depth: usize, leaf_key: &str, leaf_val: &serde_json::Value) -> Value {
        let mut current = serde_json::json!({ leaf_key: leaf_val });
        for _ in 0..depth {
            current = serde_json::json!({ "a": current });
        }
        current
    }

    #[test]
    fn test_deep_merge_depth_cap_is_32_aligned_with_python_typescript() {
        // Confirm constant value matches spec (sync STREAM-001).
        assert_eq!(DEEP_MERGE_MAX_DEPTH, 32);
    }

    #[test]
    fn test_deep_merge_caps_recursion_at_max_depth() {
        // Two chunks, each 33 levels deep, with conflicting leaves. Once the
        // merge crosses DEEP_MERGE_MAX_DEPTH (32), the second chunk's value
        // replaces the first wholesale instead of recursing further. This
        // guarantees no stack overflow on adversarial inputs.
        let chunk_a = build_nested(33, "x", &serde_json::json!(1));
        let chunk_b = build_nested(33, "y", &serde_json::json!(2));
        // Should not stack-overflow; the deeper merge replaces rather than recurses.
        let merged = deep_merge_chunks(&[chunk_a, chunk_b]);
        // Walking 32 levels of "a" should still yield an object (the cap node).
        let mut cursor = &merged;
        for _ in 0..32 {
            cursor = cursor.get("a").expect("nested 'a' should exist within cap");
        }
        // At/above the cap, the second chunk replaces the first verbatim, so
        // the leaf key from chunk_b dominates.
        let inner_str = cursor.to_string();
        assert!(
            inner_str.contains("\"y\""),
            "at depth >= cap, the later chunk should fully replace the earlier one; got {inner_str}"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // validate_against_schema
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn test_validate_against_schema_valid_input_passes() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"}
            },
            "required": ["name"]
        });
        let value = json!({"name": "Alice"});
        assert!(validate_against_schema(&value, &schema, "Input").is_ok());
    }

    #[test]
    fn test_validate_against_schema_invalid_input_returns_error_with_details() {
        let schema = json!({
            "type": "object",
            "properties": {
                "age": {"type": "integer"}
            },
            "required": ["age"]
        });
        let value = json!({"age": "not-a-number"});
        let err = validate_against_schema(&value, &schema, "Input").unwrap_err();
        assert_eq!(err.code, ErrorCode::SchemaValidationError);
        let detail = &err.to_dict()["details"]["errors"][0];
        assert_eq!(detail["path"], "/age");
        assert_eq!(detail["keyword"], "type");
        assert_eq!(detail["message"], "value is not of type \"integer\"");
    }

    #[test]
    fn test_validate_against_schema_null_schema_skips() {
        let value = json!({"anything": 123});
        assert!(validate_against_schema(&value, &Value::Null, "Input").is_ok());
    }

    #[test]
    fn test_validate_against_schema_empty_object_schema_skips() {
        let value = json!({"anything": 123});
        assert!(validate_against_schema(&value, &json!({}), "Input").is_ok());
    }

    // ═══════════════════════════════════════════════════════════════════
    // redact_sensitive
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn configured_capture_uses_the_same_key_rules_as_structured_logging() {
        let input = json!({"_secret_example": "placeholder", "display_name": "example"});
        let rules = RedactionConfig::builder()
            .sensitive_keys(["display_name"])
            .build();

        let mut logged = input.clone();
        rules.redact(&mut logged);
        assert_eq!(
            logged,
            json!({"_secret_example": "placeholder", "display_name": REDACTED_VALUE})
        );
        assert_eq!(
            redact_sensitive_with(&input, &Value::Null, Some(&rules)),
            logged
        );
        assert_eq!(
            redact_sensitive_with(&input, &Value::Null, None)["_secret_example"],
            REDACTED_VALUE
        );
    }

    #[test]
    fn test_redact_sensitive_preserves_object_root_with_sensitive_ref_and_composition() {
        let data = json!({"public":"visible","nested":{"private":"canary"}});
        let schema = json!({"x-sensitive":true,"$ref":"#/$defs/root", "$defs":{"root":{
            "x-sensitive":true,"allOf":[{"properties":{"nested":{"x-sensitive":true}}}]}}});
        let expected = json!({"public":"visible","nested":REDACTED_VALUE});
        assert_eq!(redact_sensitive(&data, &schema), expected);
        assert_eq!(redact_sensitive_with(&data, &schema, None), expected);
        assert_eq!(data["nested"]["private"], "canary");
    }

    #[test]
    fn test_redact_sensitive_ignores_prefix_items_in_legacy_dialects() {
        for dialect in [
            "http://json-schema.org/draft-07/schema#",
            "https://json-schema.org/draft/2019-09/schema",
        ] {
            let schema = json!({"$schema":dialect,"properties":{"rows":{"items":{
                "prefixItems":[{"type":"string"}], "items":{"x-sensitive":true}}}}});
            assert_eq!(
                redact_sensitive(&json!({"rows":[["canary","canary"]]}), &schema),
                json!({"rows":[[REDACTED_VALUE,REDACTED_VALUE]]})
            );
        }
    }

    #[test]
    fn test_redact_sensitive_ignores_unknown_schema_keyword_anchor_shadows() {
        for keyword in ["definitions", "dependencies"] {
            let mut schema = json!({"properties":{"schemaField":{"$anchor":"private","x-sensitive":true},
                "value":{"$ref":"#private"}}});
            schema[keyword] = json!({"opaque":{"$anchor":"private","x-sensitive":false}});
            assert_eq!(
                redact_sensitive(&json!({"value":"canary"}), &schema),
                json!({"value":REDACTED_VALUE})
            );
        }
    }

    #[test]
    fn test_redact_sensitive_resolves_refs_and_escaped_pointers() {
        let schema = json!({"$defs":{
            "Secret/Value~":{"x-sensitive":true}, "Space Name":{"x-sensitive":true}},
            "properties":{"token":{"$ref":"#/$defs/Secret~1Value~0"},
                "encoded":{"$ref":"#/$defs/Space%20Name"}, "public":{"type":"string"}}});
        let data = json!({"token":"private","encoded":"private","public":"visible"});
        let original_schema = schema.clone();
        let result = redact_sensitive(&data, &schema);
        assert_eq!(result["token"], REDACTED_VALUE);
        assert_eq!(result["encoded"], REDACTED_VALUE);
        assert_eq!(result["public"], "visible");
        assert_eq!(data["token"], "private");
        assert_eq!(schema, original_schema);
    }

    #[test]
    fn test_redact_sensitive_resolves_anchors_root_ids_and_siblings() {
        let schema = json!({"$id":"https://local.example/schema", "$defs":{
            "Secret":{"$anchor":"secret","x-sensitive":true}}, "properties":{
            "anchor":{"$ref":"#secret"}, "absolute":{"$ref":"https://local.example/schema#secret"},
            "sibling":{"$ref":"#/$defs/Missing","x-sensitive":true},
            "external":{"$ref":"file:///unavailable/schema"}}});
        let data = json!({"anchor":"private","absolute":"private","sibling":"private","external":"visible"});
        let result = redact_sensitive(&data, &schema);
        for name in ["anchor", "absolute", "sibling"] {
            assert_eq!(result[name], REDACTED_VALUE);
        }
        assert_eq!(result["external"], "visible");
    }

    #[test]
    fn test_redact_sensitive_handles_recursive_refs_and_compositions() {
        let schema = json!({"$ref":"#/$defs/Node", "$defs":{
            "Secret":{"x-sensitive":true},
            "LoopA":{"$ref":"#/$defs/LoopB"},
            "LoopB":{"anyOf":[{"$ref":"#/$defs/LoopA"},{"x-sensitive":true}]},
            "Node":{"allOf":[{"properties":{"token":{"$ref":"#/$defs/Secret"}}},
                {"properties":{"child":{"$ref":"#/$defs/Node"},"loop":{"$ref":"#/$defs/LoopA"}}}]}}});
        let data = json!({"token":"private","child":{"token":"nested","child":null},"loop":"private","public":"visible"});
        let result = redact_sensitive(&data, &schema);
        assert_eq!(result["token"], REDACTED_VALUE);
        assert_eq!(result["child"]["token"], REDACTED_VALUE);
        assert!(result["child"]["child"].is_null());
        assert_eq!(result["loop"], REDACTED_VALUE);
        assert_eq!(result["public"], "visible");
    }

    #[test]
    fn test_redact_sensitive_handles_referenced_root_arrays_and_scalars() {
        let schema =
            json!({"$defs":{"Secret":{"x-sensitive":true}},"items":{"$ref":"#/$defs/Secret"}});
        assert_eq!(
            redact_sensitive(&json!(["private", null]), &schema),
            json!([REDACTED_VALUE, null])
        );
        let schema = json!({"$defs":{"Secret":{"x-sensitive":true}},"$ref":"#/$defs/Secret"});
        assert_eq!(
            redact_sensitive(&json!("private"), &schema),
            json!(REDACTED_VALUE)
        );
        assert!(redact_sensitive(&Value::Null, &schema).is_null());
        assert_eq!(
            redact_sensitive(
                &json!([{"_secret_token":"private","public":"visible"}]),
                &json!({})
            ),
            json!([{"_secret_token":REDACTED_VALUE,"public":"visible"}])
        );
    }

    #[test]
    fn test_redact_sensitive_with_refs_preserves_configured_rules() {
        let schema = json!({"$defs":{"Credentials":{"properties":{"token":{"x-sensitive":true}}}},
            "properties":{"profile":{"$ref":"#/$defs/Credentials"}}});
        let data = json!({"profile":{"token":"private"},"password":"configured","_secret_other":"prefix","public":"visible"});
        let result = redact_sensitive_with(&data, &schema, None);
        assert_eq!(result["profile"]["token"], REDACTED_VALUE);
        assert_eq!(result["password"], REDACTED_VALUE);
        assert_eq!(result["_secret_other"], REDACTED_VALUE);
        assert_eq!(result["public"], "visible");
        assert_eq!(data["profile"]["token"], "private");
    }

    #[test]
    fn test_redact_sensitive_resolves_embedded_resource_ids() {
        let schema = json!({"$id":"https://example.test/root", "$defs":{
            "child":{"$id":"child","$anchor":"secret","type":"object",
                "$defs":{"Sensitive":{"type":"string","x-sensitive":true}},
                "properties":{"value":{"$ref":"#/$defs/Sensitive"}}}},
            "properties":{"payload":{"$ref":"child#secret"}}});
        let data = json!({"payload":{"value":"scope-canary"},"public":"visible"});
        let result = redact_sensitive(&data, &schema);
        assert_eq!(result["payload"]["value"], REDACTED_VALUE);
        assert_eq!(result["public"], "visible");
        assert_eq!(data["payload"]["value"], "scope-canary");
    }

    #[test]
    fn test_redact_sensitive_keeps_embedded_anchor_scopes_distinct() {
        let schema = json!({"$id":"https://example.test/root", "$defs":{
            "first":{"$id":"one", "$anchor":"same", "$defs":{"Secret":{"x-sensitive":true}},
                "properties":{"value":{"$ref":"#/$defs/Secret"}}},
            "second":{"$id":"nested/two", "$anchor":"same", "properties":{"value":{"type":"string"}}}},
            "properties":{"first":{"$ref":"one#same"},"second":{"$ref":"nested/two#same"}},
            "default":{"$id":"one","$anchor":"same","$ref":"https://unavailable.invalid/data"}});
        let result = redact_sensitive(
            &json!({"first":{"value":"private"},"second":{"value":"visible"}}),
            &schema,
        );
        assert_eq!(result["first"]["value"], REDACTED_VALUE);
        assert_eq!(result["second"]["value"], "visible");
    }

    #[test]
    fn test_redact_sensitive_resolves_modern_positional_and_tail_refs() {
        let schema = json!({"$schema":"https://json-schema.org/draft/2020-12/schema",
            "$defs":{"Sensitive":{"type":"string","x-sensitive":true}},
            "properties":{"values":{"type":"array","prefixItems":[{"$ref":"#/$defs/Sensitive"},{"type":"string"}],
                "items":{"$ref":"#/$defs/Sensitive"}}}});
        let data = json!({"values":["position-canary","visible","tail-canary"]});
        let original_schema = schema.clone();
        assert_eq!(
            redact_sensitive(&data, &schema),
            json!({"values":[REDACTED_VALUE,"visible",REDACTED_VALUE]})
        );
        assert_eq!(data["values"][0], "position-canary");
        assert_eq!(schema, original_schema);
    }

    #[test]
    fn test_redact_sensitive_resolves_legacy_tuple_and_additional_refs() {
        let schema = json!({"$schema":"http://json-schema.org/draft-07/schema#",
            "definitions":{"Sensitive":{"type":"string","x-sensitive":true}},
            "properties":{"values":{"type":"array","items":[{"$ref":"#/definitions/Sensitive"},{"type":"string"}],
                "additionalItems":{"$ref":"#/definitions/Sensitive"}}}});
        let data = json!({"values":["position-canary","visible","tail-canary"]});
        let original_schema = schema.clone();
        assert_eq!(
            redact_sensitive(&data, &schema),
            json!({"values":[REDACTED_VALUE,"visible",REDACTED_VALUE]})
        );
        assert_eq!(data["values"][0], "position-canary");
        assert_eq!(schema, original_schema);
    }

    #[test]
    fn test_redact_sensitive_basic_field() {
        let schema = json!({
            "properties": {
                "password": {"type": "string", "x-sensitive": true},
                "username": {"type": "string"}
            }
        });
        let data = json!({"password": "s3cret", "username": "alice"});
        let result = redact_sensitive(&data, &schema);
        assert_eq!(result["password"], REDACTED_VALUE);
        assert_eq!(result["username"], "alice");
    }

    #[test]
    fn test_redact_sensitive_nested_object() {
        let schema = json!({
            "properties": {
                "credentials": {
                    "type": "object",
                    "properties": {
                        "token": {"type": "string", "x-sensitive": true},
                        "scope": {"type": "string"}
                    }
                }
            }
        });
        let data = json!({"credentials": {"token": "abc123", "scope": "read"}});
        let result = redact_sensitive(&data, &schema);
        assert_eq!(result["credentials"]["token"], REDACTED_VALUE);
        assert_eq!(result["credentials"]["scope"], "read");
    }

    #[test]
    fn test_redact_sensitive_array_items() {
        let schema = json!({
            "properties": {
                "tokens": {
                    "type": "array",
                    "items": {"type": "string", "x-sensitive": true}
                }
            }
        });
        let data = json!({"tokens": ["a", "b", "c"]});
        let result = redact_sensitive(&data, &schema);
        let arr = result["tokens"].as_array().unwrap();
        for item in arr {
            assert_eq!(item, REDACTED_VALUE);
        }
    }

    #[test]
    fn test_redact_sensitive_secret_prefix_keys() {
        let schema = json!({});
        let data = json!({
            "_secret_api_key": "key123",
            "public_field": "visible"
        });
        let result = redact_sensitive(&data, &schema);
        assert_eq!(result["_secret_api_key"], REDACTED_VALUE);
        assert_eq!(result["public_field"], "visible");
    }

    #[test]
    fn test_redact_secret_prefix_recurses_into_arrays() {
        // A-D-003: the _secret_ prefix walk must recurse into ARRAY elements,
        // not only objects — mirroring apcore-python `_redact_in_list`.
        let schema = json!({});
        let data = json!({
            "items": [
                { "_secret_api_key": "k", "name": "ok" },
                { "nested": { "_secret_token": "t" } }
            ]
        });
        let result = redact_sensitive(&data, &schema);
        let arr = result["items"].as_array().unwrap();
        assert_eq!(arr[0]["_secret_api_key"], REDACTED_VALUE);
        assert_eq!(arr[0]["name"], "ok");
        assert_eq!(arr[1]["nested"]["_secret_token"], REDACTED_VALUE);
    }

    #[test]
    fn test_redact_secret_prefix_recurses_into_nested_arrays() {
        // Arrays nested inside arrays must also be traversed.
        let schema = json!({});
        let data = json!({ "outer": [[{ "_secret_x": "v" }]] });
        let result = redact_sensitive(&data, &schema);
        assert_eq!(result["outer"][0][0]["_secret_x"], REDACTED_VALUE);
    }

    #[test]
    fn test_redact_sensitive_null_values_preserved() {
        let schema = json!({
            "properties": {
                "password": {"type": "string", "x-sensitive": true}
            }
        });
        let data = json!({"password": null});
        let result = redact_sensitive(&data, &schema);
        assert!(result["password"].is_null());
    }

    #[test]
    fn test_redact_sensitive_no_schema_no_redaction() {
        let data = json!({"password": "s3cret"});
        let result = redact_sensitive(&data, &Value::Null);
        assert_eq!(result, data);
    }

    // ═══════════════════════════════════════════════════════════════════
    // _approval_token Phase B
    // ═══════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_approval_token_stripped_from_inputs_and_check_called() {
        let handler = MockApprovalHandler::with_status("approved");
        let module = MockModule::echo();
        let annotations = ModuleAnnotations {
            requires_approval: true,
            ..Default::default()
        };
        let mut executor = build_executor_with_module(module, annotations);
        executor.set_approval_handler(Box::new(handler));

        let inputs = json!({"_approval_token": "tok-123", "data": "hello"});
        let result = executor.call("test_mod", inputs, None, None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_approval_no_token_calls_request_approval() {
        let handler = MockApprovalHandler::with_status("approved");
        let module = MockModule::echo();
        let annotations = ModuleAnnotations {
            requires_approval: true,
            ..Default::default()
        };
        let mut executor = build_executor_with_module(module, annotations);
        executor.set_approval_handler(Box::new(handler));

        // No _approval_token -> should call request_approval
        let inputs = json!({"data": "hello"});
        let result = executor.call("test_mod", inputs, None, None).await;
        assert!(result.is_ok());
    }

    /// Strict input schema that rejects any key it does not declare — the
    /// shape that makes a leaked `_approval_token` observable in Step 7.
    fn closed_input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {"data": {"type": "string"}},
            "additionalProperties": false
        })
    }

    #[tokio::test]
    async fn test_approval_token_stripped_when_nonstrict_and_no_handler_configured() {
        // PROTOCOL_SPEC §7.4 is unconditional: `_approval_token` MUST be
        // removed from the arguments before they reach a subsequent step. The
        // no-handler skip returns from Step 5 early, so the token used to
        // survive into Step 7 where `additionalProperties: false` rejects it as
        // an undeclared key — and then into the module's own execute().
        let module = EchoInputsModule {
            input_schema: closed_input_schema(),
        };
        let annotations = ModuleAnnotations {
            requires_approval: true,
            ..Default::default()
        };
        let mut executor = build_executor_with_module(module, annotations);
        executor.set_policy(Some(ExecutionPolicy::default().with_strict(false)));

        let result = executor
            .call(
                "test_mod",
                json!({"_approval_token": "tok-123", "data": "hello"}),
                None,
                None,
            )
            .await
            .expect("the token must be stripped before input validation");
        assert_eq!(result, json!({"data": "hello"}));
    }

    #[tokio::test]
    async fn test_approval_token_stripped_when_module_not_gated() {
        // The other early return from Step 5: the module needs no approval at
        // all, so the gate continues immediately. §7.4 still applies.
        let module = EchoInputsModule {
            input_schema: closed_input_schema(),
        };
        let executor = build_executor_with_module(module, ModuleAnnotations::default());

        let result = executor
            .call(
                "test_mod",
                json!({"_approval_token": "tok-123", "data": "hello"}),
                None,
                None,
            )
            .await
            .expect("the token must be stripped before input validation");
        assert_eq!(result, json!({"data": "hello"}));
    }

    #[tokio::test]
    async fn test_approval_token_non_string_rejected_on_ungated_path() {
        // The non-string rejection runs ahead of every early return too, so a
        // malformed token never slips through an ungated call.
        let module = EchoInputsModule {
            input_schema: json!({}),
        };
        let executor = build_executor_with_module(module, ModuleAnnotations::default());

        let err = executor
            .call("test_mod", json!({"_approval_token": 123}), None, None)
            .await
            .expect_err("a non-string _approval_token must be rejected");
        assert_eq!(err.code, ErrorCode::GeneralInvalidInput);
        assert!(err.message.contains("must be a string"), "{}", err.message);
    }

    #[tokio::test]
    async fn test_validate_notes_requires_approval_without_gating() {
        // validate() per spec §12.8 MUST NOT actually request approval,
        // it only reports requires_approval = true.
        let handler = MockApprovalHandler::with_status("timeout");
        let module = MockModule::echo();
        let annotations = ModuleAnnotations {
            requires_approval: true,
            ..Default::default()
        };
        let mut executor = build_executor_with_module(module, annotations);
        executor.set_approval_handler(Box::new(handler));

        let result = executor
            .validate("test_mod", &json!({}), None)
            .await
            .unwrap();
        assert!(result.valid);
        assert!(result.requires_approval);
    }

    #[tokio::test]
    async fn approval_token_isolated_without_a_gate_on_call_and_stream() {
        let mut executor = build_executor_with_module(
            EchoInputsModule {
                input_schema: closed_input_schema(),
            },
            ModuleAnnotations::default(),
        );
        executor.strategy.remove("approval_gate").unwrap();
        let inputs = json!({"_approval_token": "review-example", "data": "hello"});
        let output = json!({"data": "hello"});

        assert_eq!(
            executor
                .call("test_mod", inputs.clone(), None, None)
                .await
                .unwrap(),
            output
        );
        let mut chunks = executor.stream("test_mod", inputs, None, None);
        assert_eq!(chunks.next().await.unwrap().unwrap(), output);
        assert!(chunks.next().await.is_none());
    }

    #[tokio::test]
    async fn preflight_ignores_approval_token_and_reports_required_approval() {
        let mut executor = build_executor_with_module(
            EchoInputsModule {
                input_schema: closed_input_schema(),
            },
            ModuleAnnotations {
                requires_approval: true,
                ..Default::default()
            },
        );
        executor.set_approval_handler(Box::new(MockApprovalHandler::with_status("rejected")));
        let result = executor
            .validate(
                "test_mod",
                &json!({"_approval_token": false, "data": "hello"}),
                None,
            )
            .await
            .unwrap();
        assert!(result.valid);
        assert!(result.requires_approval);
        assert!(result.checks.iter().all(|check| check.warnings.is_empty()));
    }

    #[derive(Default)]
    struct ObserveApprovalError(Mutex<Vec<ModuleError>>);

    #[async_trait]
    impl crate::pipeline::StepMiddleware for ObserveApprovalError {
        async fn on_step_error(
            &self,
            _step_name: &str,
            _state: &crate::pipeline::PipelineState<'_>,
            error: &ModuleError,
        ) -> Result<Option<Value>, ModuleError> {
            self.0.lock().unwrap().push(error.clone());
            Ok(None)
        }
    }

    #[tokio::test]
    async fn approval_errors_preserve_full_result_for_caller_only() {
        for (status, code) in [
            ("rejected", ErrorCode::ApprovalDenied),
            ("timeout", ErrorCode::ApprovalTimeout),
            ("pending", ErrorCode::ApprovalPending),
            ("unknown", ErrorCode::ApprovalDenied),
        ] {
            let mut executor = build_executor_with_module(
                MockModule::echo(),
                ModuleAnnotations {
                    requires_approval: true,
                    ..Default::default()
                },
            );
            let observer = Arc::new(ObserveApprovalError::default());
            executor.strategy.add_step_middleware(observer.clone());
            let mut handler = MockApprovalHandler::with_status(status);
            handler.result.approval_id = Some("review-example".into());
            handler.result.audit_id = Some("review-audit".into());
            handler.result.metadata = Some(HashMap::from([("source".into(), json!("test"))]));
            let expected_result = json!(handler.result);
            executor.set_approval_handler(Box::new(handler));

            let error = executor
                .call("test_mod", json!({}), None, None)
                .await
                .unwrap_err();
            assert_eq!(error.code, code);
            assert_eq!(error.details["result"], expected_result);
            if status == "pending" {
                assert_eq!(error.details["approval_id"], json!("review-example"));
            }
            let reported_errors = observer.0.lock().unwrap();
            assert_eq!(reported_errors.len(), 1);
            assert_eq!(reported_errors[0].code, code);
            assert!(!reported_errors[0].details.contains_key("result"));
            assert!(!reported_errors[0].details.contains_key("approval_id"));
        }
    }

    #[tokio::test]
    async fn strict_approval_without_handler_reports_rejected_result() {
        let mut executor = build_executor_with_module(
            MockModule::echo(),
            ModuleAnnotations {
                requires_approval: true,
                ..Default::default()
            },
        );
        executor.set_policy(Some(ExecutionPolicy::new(vec![]).with_strict(true)));
        let error = executor
            .call("test_mod", json!({}), None, None)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::ApprovalDenied);
        assert_eq!(error.details["result"]["status"], "rejected");
    }
}
