//! Single-SDK defects tracked in aiperceivable/apcore#123.
//!
//! Each test reproduces one defect an audit found by reading this crate's
//! source, against the behaviour apcore-python and apcore-typescript already
//! have. They are grouped by subsystem, not by finding number.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use apcore::config::Config;
use apcore::context::Context;
use apcore::errors::{ErrorCode, ModuleError};
use apcore::module::Module;
use apcore::registry::registry::Registry;
use apcore::APCore;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct Capture(Arc<StdMutex<Vec<u8>>>);

impl Capture {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl std::io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Capture;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// A module answering with a fixed value, so a test can tell instances apart.
struct Fixed {
    value: Value,
}

#[async_trait]
impl Module for Fixed {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "fixed"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(self.value.clone())
    }
}

fn fixed(value: Value) -> Box<dyn Module> {
    Box::new(Fixed { value })
}

// ---------------------------------------------------------------------------
// Observability
// ---------------------------------------------------------------------------

#[tokio::test]
async fn batch_processor_keeps_running_when_a_clone_is_dropped() {
    use apcore::observability::exporters::InMemoryExporter;
    use apcore::observability::processor::{BatchSpanProcessor, SpanProcessor};
    use apcore::observability::span::Span;

    let exporter = InMemoryExporter::new();
    let processor = BatchSpanProcessor::builder(Arc::new(exporter.clone()))
        .schedule_delay_ms(10)
        .build();
    // The documented wiring hands a clone to the middleware and lets the
    // original go out of scope.
    let handed_out = processor.clone();
    drop(processor);
    // Give the worker time to act on anything the drop signalled.
    tokio::time::sleep(Duration::from_millis(50)).await;

    handed_out
        .on_span_end(Span::new("after-drop", "trace-1"))
        .await;
    for _ in 0..100 {
        if !exporter.get_spans().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        exporter.get_spans().len(),
        1,
        "the worker must keep exporting while any handle is alive"
    );
}

#[test]
fn collectors_attach_the_in_memory_storage_backend_by_default() {
    use apcore::observability::error_history::ErrorHistory;
    use apcore::observability::metrics::MetricsCollector;
    use apcore::observability::usage::UsageCollector;

    assert!(ErrorHistory::new(10).storage_backend().is_some());
    assert!(ErrorHistory::with_limits(10, 100)
        .storage_backend()
        .is_some());
    assert!(MetricsCollector::new().storage_backend().is_some());
    assert!(UsageCollector::new().storage_backend().is_some());
}

#[test]
fn error_history_new_keeps_the_default_total_limit() {
    use apcore::observability::error_history::ErrorHistory;

    // One entry per module, 150 modules: the total limit is the default 1000,
    // not a multiple of the per-module limit.
    let history = ErrorHistory::new(1);
    for i in 0..150 {
        history.record(
            &format!("mod.m{i}"),
            &ModuleError::new(ErrorCode::GeneralInternalError, "boom"),
        );
    }
    assert_eq!(history.count(), 150);
}

#[test]
fn error_history_get_orders_by_creation_newest_first() {
    use apcore::observability::error_history::ErrorHistory;

    let history = ErrorHistory::new(50);
    let t0 = chrono::Utc::now();
    let first = ModuleError::new(ErrorCode::GeneralInternalError, "first");
    let second = ModuleError::new(ErrorCode::GeneralInternalError, "second");
    history.record_at("mod.a", &first, t0);
    history.record_at("mod.a", &second, t0 + chrono::Duration::seconds(1));
    // A repeat of the first error updates `last_occurred` but not its place.
    history.record_at("mod.a", &first, t0 + chrono::Duration::seconds(2));

    let messages: Vec<String> = history
        .get("mod.a", None)
        .into_iter()
        .map(|e| e.message)
        .collect();
    assert_eq!(messages, vec!["second".to_string(), "first".to_string()]);
}

#[tokio::test]
async fn in_memory_exporter_default_capacity_is_ten_thousand() {
    use apcore::observability::exporters::InMemoryExporter;
    use apcore::observability::span::{Span, SpanExporter};

    let exporter = InMemoryExporter::new();
    for i in 0..1500 {
        exporter
            .export(&Span::new(format!("s{i}"), "trace"))
            .await
            .unwrap();
    }
    assert_eq!(exporter.get_spans().len(), 1500);
}

#[test]
fn context_logger_applies_the_default_redaction_rules() {
    use apcore::observability::logging::ContextLogger;

    let capture = Capture::default();
    let mut logger = ContextLogger::new("probe");
    logger.set_writer(Box::new(capture.clone()));
    let mut extra = HashMap::new();
    extra.insert("password".to_string(), json!("hunter2"));
    extra.insert("nested".to_string(), json!({"api_key": "k-123", "ok": 1}));
    extra.insert("trace_id".to_string(), json!("abc"));
    logger.emit("info", "hello", Some(&extra));

    let out = capture.text();
    assert!(!out.contains("hunter2"), "password leaked: {out}");
    assert!(!out.contains("k-123"), "nested api_key leaked: {out}");
    assert!(out.contains("\"trace_id\":\"abc\""), "{out}");
    assert!(out.contains("\"ok\":1"), "{out}");
}

#[test]
fn context_logger_applies_a_configured_redaction_config() {
    use apcore::observability::logging::ContextLogger;
    use apcore::observability::redaction::RedactionConfig;

    let capture = Capture::default();
    let mut logger = ContextLogger::new("probe");
    logger.set_writer(Box::new(capture.clone()));
    logger.set_redaction_config(
        RedactionConfig::builder()
            .sensitive_keys(["customer"])
            .value_patterns([r"sk-[0-9]+"])
            .build(),
    );
    let mut extra = HashMap::new();
    extra.insert("customer_name".to_string(), json!("Ada"));
    extra.insert("note".to_string(), json!("SK-999"));
    logger.emit("info", "hello", Some(&extra));

    let out = capture.text();
    assert!(!out.contains("Ada"), "{out}");
    assert!(!out.contains("SK-999"), "{out}");
}

#[test]
fn builder_value_patterns_are_case_insensitive_like_from_config() {
    use apcore::observability::redaction::RedactionConfig;

    let built = RedactionConfig::builder()
        .value_patterns([r"secret-\d+"])
        .build();
    assert!(built.value_matches("SECRET-42"));

    let mut config = Config::default();
    config.set("obs.redaction.regex_patterns", json!([r"secret-\d+"]));
    assert!(RedactionConfig::from_config(&config).value_matches("SECRET-42"));
}

#[tokio::test]
async fn trace_context_inject_uses_the_current_span() {
    use apcore::observability::exporters::InMemoryExporter;
    use apcore::observability::tracing_middleware::TracingMiddleware;
    use apcore::trace_context::TraceContext;

    struct Injecting;

    #[async_trait]
    impl Module for Injecting {
        fn input_schema(&self) -> Value {
            json!({"type": "object"})
        }
        fn output_schema(&self) -> Value {
            json!({"type": "object"})
        }
        fn description(&self) -> &'static str {
            "injects"
        }
        async fn execute(
            &self,
            _inputs: Value,
            ctx: &Context<Value>,
        ) -> Result<Value, ModuleError> {
            let headers = TraceContext::inject(ctx);
            Ok(json!({"traceparent": headers["traceparent"]}))
        }
    }

    let exporter = InMemoryExporter::new();
    let client = APCore::new();
    client
        .use_middleware(Box::new(TracingMiddleware::new(Box::new(exporter.clone()))))
        .unwrap();
    client
        .register("probe.inject", Box::new(Injecting))
        .unwrap();

    let out = client
        .call("probe.inject", json!({}), None, None)
        .await
        .unwrap();
    let traceparent = out["traceparent"].as_str().unwrap().to_string();
    let parent_id = traceparent.split('-').nth(2).unwrap().to_string();
    let spans = exporter.get_spans();
    assert_eq!(spans.len(), 1);
    assert_eq!(
        parent_id, spans[0].span_id,
        "the outbound parent must be the span wrapping the call"
    );
}

#[cfg(feature = "events")]
#[tokio::test]
async fn otlp_endpoint_is_the_full_url() {
    use apcore::observability::exporters::OTLPExporter;
    use apcore::observability::span::{Span, SpanExporter};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 256];
        let n = stream.read(&mut buf).await.unwrap();
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        String::from_utf8_lossy(&buf[..n]).into_owned()
    });

    // The shape `observability.tracing.otlp_endpoint` and the default carry.
    let exporter = OTLPExporter::new(format!("http://127.0.0.1:{port}/v1/traces"));
    exporter.export(&Span::new("s", "t")).await.unwrap();
    let request = tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
    assert!(
        request.starts_with("POST /v1/traces HTTP/1.1"),
        "the endpoint must be posted to as given: {request}"
    );
}

// ---------------------------------------------------------------------------
// Nested calls through APCore
// ---------------------------------------------------------------------------

struct Outer;

#[async_trait]
impl Module for Outer {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "calls probe.inner"
    }
    async fn execute(&self, _inputs: Value, ctx: &Context<Value>) -> Result<Value, ModuleError> {
        let executor = ctx.executor().ok_or_else(|| {
            ModuleError::new(
                ErrorCode::GeneralInternalError,
                "no executor on the context",
            )
        })?;
        let inner = executor
            .call("probe.inner", json!({}), Some(ctx), None)
            .await?;
        Ok(json!({"inner": inner}))
    }
}

struct Inner;

#[async_trait]
impl Module for Inner {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "reports its call chain"
    }
    async fn execute(&self, _inputs: Value, ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({"call_chain": ctx.call_chain, "caller_id": ctx.caller_id}))
    }
}

#[tokio::test]
async fn a_module_registered_through_apcore_can_call_another() {
    let client = APCore::new();
    client.register("probe.outer", Box::new(Outer)).unwrap();
    client.register("probe.inner", Box::new(Inner)).unwrap();

    let out = client
        .call("probe.outer", json!({}), None, None)
        .await
        .unwrap();
    assert_eq!(
        out["inner"]["call_chain"],
        json!(["probe.outer", "probe.inner"])
    );
    assert_eq!(out["inner"]["caller_id"], json!("probe.outer"));
}

#[tokio::test]
async fn an_executor_placed_in_an_arc_is_reachable_from_the_context() {
    use apcore::executor::Executor;

    let registry = Arc::new(Registry::new());
    registry
        .register_module("probe.outer", Box::new(Outer))
        .unwrap();
    registry
        .register_module("probe.inner", Box::new(Inner))
        .unwrap();
    let executor = Executor::new(Arc::clone(&registry), Arc::new(Config::default())).into_shared();

    let out = executor
        .call("probe.outer", json!({}), None, None)
        .await
        .unwrap();
    assert_eq!(out["inner"]["caller_id"], json!("probe.outer"));
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

fn counting_factory(counter: Arc<AtomicUsize>) -> apcore::ModuleFactory {
    Arc::new(move |_file, _entry| {
        let generation = counter.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(Some(Arc::new(Fixed {
            value: json!({"generation": generation}),
        }) as Arc<dyn Module>))
    })
}

#[tokio::test]
async fn registry_discover_passes_the_extension_roots() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("greet.rs"), "// stub").unwrap();

    let registry = Registry::new();
    registry.set_extension_roots(vec![tmp.path().to_string_lossy().into_owned()]);
    let discoverer =
        apcore::DefaultDiscoverer::new().with_factory(counting_factory(Arc::default()));
    let count = registry.discover(&discoverer).await.unwrap();
    assert_eq!(count, 1);
    assert!(registry.has("greet"));
}

#[tokio::test]
async fn a_default_discoverer_without_a_factory_does_not_discover_nothing_silently() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("greet.rs"), "// stub").unwrap();

    let registry = Registry::new();
    registry.set_extension_roots(vec![tmp.path().to_string_lossy().into_owned()]);
    let err = registry
        .discover(&apcore::DefaultDiscoverer::new())
        .await
        .expect_err("a file no factory can instantiate must be reported");
    assert_eq!(err.code, ErrorCode::ModuleLoadError);
}

#[tokio::test]
async fn watch_replaces_a_changed_module() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("greet.rs");
    std::fs::write(&file, "// v1").unwrap();

    let registry = Arc::new(Registry::new());
    registry.set_extension_roots(vec![tmp.path().to_string_lossy().into_owned()]);
    registry.set_discoverer(Box::new(
        apcore::DefaultDiscoverer::new().with_factory(counting_factory(Arc::default())),
    ));
    registry.discover_internal().await.unwrap();

    let generation = |registry: &Registry| {
        let module = registry.get("greet").unwrap().expect("registered");
        async move {
            module
                .execute(json!({}), &Context::anonymous())
                .await
                .unwrap()["generation"]
                .clone()
        }
    };
    assert_eq!(generation(&registry).await, json!(1));

    registry.watch().await.unwrap();
    // Let the platform watcher settle before the change it must observe.
    tokio::time::sleep(Duration::from_millis(300)).await;
    std::fs::write(&file, "// v2").unwrap();

    let mut replaced = false;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if registry.has("greet") && generation(&registry).await != json!(1) {
            replaced = true;
            break;
        }
    }
    registry.unwatch();
    assert!(
        replaced,
        "a changed module file must replace the registered module"
    );
}

#[test]
fn register_versioned_accepts_a_second_version_and_get_returns_the_latest() {
    let registry = Registry::new();
    registry
        .register_versioned("probe.v", fixed(json!({"v": 1})), Some("1.0.0"), None)
        .unwrap();
    registry
        .register_versioned("probe.v", fixed(json!({"v": 2})), Some("2.0.0"), None)
        .expect("a second version of the same module is multi-version registration");
    assert_eq!(
        registry.get_definition("probe.v").unwrap().unwrap().version,
        "2.0.0"
    );

    // An older version added later does not displace the latest.
    registry
        .register_versioned("probe.v", fixed(json!({"v": 15})), Some("1.5.0"), None)
        .unwrap();
    assert_eq!(
        registry.get_definition("probe.v").unwrap().unwrap().version,
        "2.0.0"
    );

    // The same version twice is still a duplicate.
    let err = registry
        .register_versioned("probe.v", fixed(json!({})), Some("2.0.0"), None)
        .expect_err("same id and version");
    assert_eq!(err.code, ErrorCode::DuplicateModuleId);

    // Unregister removes every version.
    assert!(registry.unregister("probe.v").unwrap());
    assert!(registry.get("probe.v").unwrap().is_none());
    registry
        .register_versioned("probe.v", fixed(json!({})), Some("0.1.0"), None)
        .unwrap();
    assert_eq!(
        registry.get_definition("probe.v").unwrap().unwrap().version,
        "0.1.0"
    );
}

#[test]
fn a_custom_validator_rejection_is_invalid_input() {
    use apcore::module::ValidationResult;
    use apcore::registry::registry::ModuleValidator;

    struct RejectAll;
    impl ModuleValidator for RejectAll {
        fn validate(
            &self,
            _module: &dyn Module,
            _descriptor: Option<&apcore::registry::registry::ModuleDescriptor>,
        ) -> ValidationResult {
            let mut result = ValidationResult::default();
            result.valid = false;
            result
        }
    }

    let registry = Registry::new();
    registry.set_validator(Box::new(RejectAll));
    let err = registry
        .register_module("probe.rejected", fixed(json!({})))
        .expect_err("validator rejects");
    assert_eq!(err.code, ErrorCode::GeneralInvalidInput);
}

// ---------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------

#[test]
fn an_unknown_strategy_name_is_strategy_not_found() {
    let err =
        apcore::executor::resolve_strategy_by_name("no-such-strategy").expect_err("unknown name");
    assert_eq!(err.code, ErrorCode::StrategyNotFound);
}

#[tokio::test]
async fn stream_fallback_runs_a_replaced_execute_step() {
    use apcore::executor::Executor;
    use apcore::pipeline::{PipelineContext, Step, StepResult};
    use futures_util::StreamExt;

    struct ReplacedExecute;

    #[async_trait]
    impl Step for ReplacedExecute {
        fn name(&self) -> &'static str {
            "execute"
        }
        fn description(&self) -> &'static str {
            "replaced"
        }
        fn removable(&self) -> bool {
            false
        }
        fn replaceable(&self) -> bool {
            true
        }
        async fn execute(&self, ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
            ctx.output = Some(json!({"replaced": true}));
            Ok(StepResult::continue_step())
        }
    }

    let registry = Arc::new(Registry::new());
    registry
        .register_module("probe.unary", fixed(json!({"replaced": false})))
        .unwrap();
    let mut strategy = apcore::builtin_steps::build_standard_strategy();
    strategy
        .replace("execute", Box::new(ReplacedExecute))
        .unwrap();
    let executor = Executor::with_strategy(registry, Arc::new(Config::default()), strategy);

    let chunks: Vec<Value> = executor
        .stream("probe.unary", json!({}), None, None)
        .map(|c| c.unwrap())
        .collect()
        .await;
    assert_eq!(chunks, vec![json!({"replaced": true})]);
}

#[tokio::test]
async fn apcore_use_middleware_detects_a_duplicate_identity() {
    use apcore::middleware::base::Middleware;

    #[derive(Debug)]
    struct Named;

    #[async_trait]
    impl Middleware for Named {
        fn name(&self) -> &'static str {
            "probe-duplicate"
        }
        async fn before(
            &self,
            _: &str,
            _: Value,
            _: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            Ok(None)
        }
        async fn after(
            &self,
            _: &str,
            _: Value,
            _: Value,
            _: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            Ok(None)
        }
        async fn on_error(
            &self,
            _: &str,
            _: Value,
            _: &ModuleError,
            _: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            Ok(None)
        }
    }

    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture.clone())
        .with_ansi(false)
        .finish();
    let client = APCore::new();
    tracing::subscriber::with_default(subscriber, || {
        client.use_middleware(Box::new(Named)).unwrap();
        client.use_middleware(Box::new(Named)).unwrap();
    });
    assert!(
        capture.text().contains("duplicate middleware registration"),
        "{}",
        capture.text()
    );
}

// ---------------------------------------------------------------------------
// System modules
// ---------------------------------------------------------------------------

fn sys_config(dir: &std::path::Path, extra: &str) -> Config {
    let path = dir.join("apcore.yaml");
    std::fs::write(
        &path,
        format!(
            "version: \"1.0\"\nproject:\n  name: sys-probe\nsys_modules:\n  enabled: true\n  events:\n    enabled: true\n{extra}"
        ),
    )
    .unwrap();
    Config::from_yaml_file(&path).expect("config loads")
}

#[tokio::test]
async fn apcore_persists_a_toggle_to_the_configured_overrides_path() {
    let dir = tempfile::tempdir().unwrap();
    let overrides = dir.path().join("overrides.yaml");
    let config = sys_config(
        dir.path(),
        &format!(
            "  control:\n    overrides_path: {}\n",
            serde_json::to_string(&overrides.to_string_lossy()).unwrap()
        ),
    );
    let client = APCore::with_config(config);
    client.register("probe.toggled", fixed(json!({}))).unwrap();
    client.disable("probe.toggled", Some("test")).await.unwrap();

    let written = std::fs::read_to_string(&overrides).expect("overrides file written");
    assert!(written.contains("probe.toggled"), "{written}");
}

#[tokio::test]
async fn reload_module_can_reload_the_config_of_an_apcore_client() {
    let dir = tempfile::tempdir().unwrap();
    let client = APCore::with_config(sys_config(dir.path(), ""));
    let out = client
        .call(
            "system.control.reload_module",
            json!({"reason": "test", "reload_config": true}),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(out["config_reloaded"], json!(true), "{out}");
}

#[test]
fn toggle_feature_is_declared_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let client = APCore::with_config(sys_config(dir.path(), ""));
    let annotations = |id: &str| {
        client
            .registry()
            .get_definition(id)
            .unwrap()
            .unwrap()
            .annotations
            .unwrap()
    };
    assert!(annotations("system.control.toggle_feature").idempotent);
    assert!(!annotations("system.control.update_config").idempotent);
    assert!(!annotations("system.control.reload_module").idempotent);
}

// ---------------------------------------------------------------------------
// Async tasks
// ---------------------------------------------------------------------------

#[tokio::test]
async fn async_task_manager_lists_and_cleans_up_asynchronously() {
    use apcore::async_task::{AsyncTaskManager, TaskInfo, TaskStatus, TaskStore};

    /// A store whose every call yields once, like a network-backed one.
    struct Yielding(apcore::async_task::InMemoryTaskStore);

    #[async_trait]
    impl TaskStore for Yielding {
        async fn save(&self, task: &TaskInfo) -> Result<(), ModuleError> {
            tokio::task::yield_now().await;
            self.0.save(task).await
        }
        async fn get(&self, id: &str) -> Result<Option<TaskInfo>, ModuleError> {
            tokio::task::yield_now().await;
            self.0.get(id).await
        }
        async fn list(&self, status: Option<TaskStatus>) -> Result<Vec<TaskInfo>, ModuleError> {
            tokio::task::yield_now().await;
            self.0.list(status).await
        }
        async fn delete(&self, id: &str) -> Result<(), ModuleError> {
            tokio::task::yield_now().await;
            self.0.delete(id).await
        }
        async fn list_expired(&self, before: f64) -> Result<Vec<TaskInfo>, ModuleError> {
            tokio::task::yield_now().await;
            self.0.list_expired(before).await
        }
        // No `store_type_name`: the trait supplies a default.
    }

    let registry = Arc::new(Registry::new());
    registry
        .register_module("probe.task", fixed(json!({})))
        .unwrap();
    let executor = Arc::new(apcore::executor::Executor::new(
        registry,
        Arc::new(Config::default()),
    ));
    let manager = AsyncTaskManager::with_store(
        executor,
        4,
        100,
        Arc::new(Yielding(apcore::async_task::InMemoryTaskStore::new())),
    );
    assert!(!manager.store_type_name().is_empty());

    let id = manager.submit("probe.task", json!({}), None).await.unwrap();
    for _ in 0..100 {
        if manager
            .get_status_async(&id)
            .await
            .unwrap()
            .map(|t| t.status)
            == Some(TaskStatus::Completed)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(manager.list_tasks_async(None).await.unwrap().len(), 1);
    assert_eq!(manager.cleanup_async(0.0).await.unwrap(), 1);
    assert!(manager.list_tasks_async(None).await.unwrap().is_empty());
}
