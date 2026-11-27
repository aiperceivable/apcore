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

    let inbound = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
    let headers = HashMap::from([("traceparent".to_owned(), inbound.to_owned())]);
    let parsed = TraceContext::extract(&headers).unwrap();
    let contexts = [
        Context::create(None, Some(parsed.clone()), None, None, Value::Null, None),
        Context::builder()
            .trace_parent(Some(parsed.clone()))
            .build(),
    ];
    for context in &contexts {
        exporter.clear();
        let out = client
            .call("probe.inject", json!({}), Some(context), None)
            .await
            .unwrap();
        let traceparent = TraceContext::extract(&HashMap::from([(
            "traceparent".to_owned(),
            out["traceparent"].as_str().unwrap().to_owned(),
        )]))
        .unwrap();
        let spans = exporter.get_spans();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].trace_id, parsed.trace_id);
        assert_eq!(
            spans[0].parent_span_id.as_deref(),
            Some(parsed.parent_id.as_str())
        );
        assert_eq!(traceparent.parent_id, spans[0].span_id);
    }

    let mut restarted_parent = parsed;
    restarted_parent.trace_id = "f".repeat(32);
    let restarted = Context::builder()
        .trace_parent(Some(restarted_parent))
        .build();
    exporter.clear();
    client
        .call("probe.inject", json!({}), Some(&restarted), None)
        .await
        .unwrap();
    let spans = exporter.get_spans();
    assert_ne!(spans[0].trace_id, "f".repeat(32));
    assert!(spans[0].parent_span_id.is_none());
}

#[derive(Default)]
struct SpanProbeGate {
    started: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

impl SpanProbeGate {
    async fn observe(&self, fail: bool, ctx: &Context<Value>) -> Result<Value, ModuleError> {
        use apcore::trace_context::TraceContext;

        self.started.notify_one();
        self.release.notified().await;
        let traceparent = json!(TraceContext::inject(ctx)["traceparent"]);
        if fail {
            return Err(
                ModuleError::new(ErrorCode::ModuleExecuteError, "probe failure")
                    .with_details(HashMap::from([("traceparent".to_owned(), traceparent)])),
            );
        }
        Ok(json!({"traceparent": traceparent}))
    }
}

enum SpanProbe {
    Parent {
        a: Arc<SpanProbeGate>,
        b: Arc<SpanProbeGate>,
        a_finishes_first: bool,
    },
    Sibling(Arc<SpanProbeGate>),
}

#[async_trait]
impl Module for SpanProbe {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }

    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }

    fn description(&self) -> &'static str {
        "Runs a nested trace or reports its active span"
    }

    fn stream(&self, inputs: Value, ctx: &Context<Value>) -> Option<apcore::module::ChunkStream> {
        let Self::Sibling(gate) = self else {
            return None;
        };
        let gate = Arc::clone(gate);
        let ctx = ctx.clone();
        Some(Box::pin(futures_util::stream::once(async move {
            gate.observe(inputs["fail"] == true, &ctx).await
        })))
    }

    async fn execute(&self, inputs: Value, ctx: &Context<Value>) -> Result<Value, ModuleError> {
        use futures_util::StreamExt;

        match self {
            Self::Sibling(gate) => gate.observe(inputs["fail"] == true, ctx).await,
            Self::Parent {
                a,
                b,
                a_finishes_first,
            } => {
                let executor = ctx.executor().unwrap();
                let ctx_a = ctx.clone();
                let executor_a = Arc::clone(&executor);
                let first = tokio::spawn(async move {
                    executor_a
                        .call("probe.a", json!({}), Some(&ctx_a), None)
                        .await
                        .unwrap()
                });
                a.started.notified().await;
                // Separate the starts so a crossed timing association is observable.
                tokio::time::sleep(Duration::from_millis(40)).await;
                let ctx_b = ctx.clone();
                let fail = !*a_finishes_first;
                let second = tokio::spawn(async move {
                    let (mut chunks, _) = executor.stream_with_completion(
                        "probe.b",
                        json!({"fail": fail}),
                        Some(&ctx_b),
                        None,
                    );
                    let result = chunks.next().await.unwrap();
                    assert!(chunks.next().await.is_none());
                    if fail {
                        let error = result.unwrap_err();
                        assert_eq!(error.code, ErrorCode::ModuleExecuteError);
                        json!({"traceparent": error.details["traceparent"]})
                    } else {
                        result.unwrap()
                    }
                });
                b.started.notified().await;

                let (a_out, b_out) = if *a_finishes_first {
                    a.release.notify_one();
                    let a_out = first.await.unwrap();
                    b.release.notify_one();
                    (a_out, second.await.unwrap())
                } else {
                    b.release.notify_one();
                    let b_out = second.await.unwrap();
                    a.release.notify_one();
                    (first.await.unwrap(), b_out)
                };
                Ok(json!({"a": a_out, "b": b_out}))
            }
        }
    }
}

#[tokio::test]
async fn test_sibling_calls_keep_their_timing_and_span_parent_in_either_finish_order() {
    use apcore::observability::exporters::InMemoryExporter;
    use apcore::observability::logging::{ContextLogger, ObsLoggingMiddleware};
    use apcore::observability::metrics::{MetricsCollector, MetricsMiddleware};
    use apcore::observability::span::SpanStatus;
    use apcore::observability::tracing_middleware::TracingMiddleware;
    use apcore::observability::usage::{UsageCollector, UsageMiddleware};
    use apcore::trace_context::TraceContext;

    for a_finishes_first in [true, false] {
        let exporter = InMemoryExporter::new();
        let metrics = MetricsCollector::new();
        let usage = UsageCollector::new();
        let client = APCore::new();
        let mut logger = ContextLogger::new("probe");
        let capture = Capture::default();
        logger.set_writer(Box::new(capture.clone()));
        client
            .use_middleware(Box::new(TracingMiddleware::new(Box::new(exporter.clone()))))
            .unwrap();
        client
            .use_middleware(Box::new(MetricsMiddleware::new(metrics.clone())))
            .unwrap();
        client
            .use_middleware(Box::new(UsageMiddleware::new(usage.clone())))
            .unwrap();
        client
            .use_middleware(Box::new(ObsLoggingMiddleware::new(logger)))
            .unwrap();
        let a = Arc::new(SpanProbeGate::default());
        let b = Arc::new(SpanProbeGate::default());
        client
            .register("probe.a", Box::new(SpanProbe::Sibling(Arc::clone(&a))))
            .unwrap();
        client
            .register("probe.b", Box::new(SpanProbe::Sibling(Arc::clone(&b))))
            .unwrap();
        client
            .register(
                "probe.parent",
                Box::new(SpanProbe::Parent {
                    a,
                    b,
                    a_finishes_first,
                }),
            )
            .unwrap();
        let outputs = tokio::time::timeout(
            Duration::from_secs(3),
            client.call("probe.parent", json!({}), None, None),
        )
        .await
        .unwrap()
        .unwrap();
        let spans = exporter.get_spans();
        assert_eq!(spans.len(), 3);
        let expected_first = if a_finishes_first {
            "probe.a"
        } else {
            "probe.b"
        };
        assert_eq!(spans[0].attributes["module_id"], expected_first);
        let root = &spans[2];
        assert_eq!(root.attributes["module_id"], "probe.parent");
        for label in ["a", "b"] {
            let id = format!("probe.{label}");
            let span = spans
                .iter()
                .find(|span| span.attributes["module_id"] == id)
                .unwrap();
            let traceparent = TraceContext::extract(&HashMap::from([(
                "traceparent".to_owned(),
                outputs[label]["traceparent"].as_str().unwrap().to_owned(),
            )]))
            .unwrap();
            assert_eq!(span.parent_span_id.as_deref(), Some(root.span_id.as_str()));
            assert_eq!(traceparent.parent_id, span.span_id);
            let expected_status = if label == "b" && !a_finishes_first {
                SpanStatus::Error
            } else {
                SpanStatus::Ok
            };
            assert_eq!(span.status, expected_status);
        }

        let snapshot = metrics.snapshot();
        let duration = |id| {
            snapshot["histograms"][format!("apcore_module_duration_seconds|module_id=probe.{id}")]
                ["sum"]
                .as_f64()
                .unwrap()
        };
        assert!(duration("a") > duration("b"));
        let usage_a = usage.get_module_summary("probe.a").unwrap();
        let usage_b = usage.get_module_summary("probe.b").unwrap();
        assert_eq!(usage_b.error_count, u64::from(!a_finishes_first));
        assert!(usage_a.avg_latency_ms > usage_b.avg_latency_ms);
        let logged_durations: HashMap<String, f64> = capture
            .text()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter_map(|record| {
                let extra = &record["extra"];
                Some((
                    extra["module_id"].as_str()?.to_owned(),
                    extra["duration_ms"].as_f64()?,
                ))
            })
            .collect();
        assert!(logged_durations["probe.a"] > logged_durations["probe.b"]);
    }
}

#[cfg(feature = "events")]
#[tokio::test]
async fn otlp_posts_to_full_url_with_original_trace_and_parent_ids() {
    use apcore::observability::exporters::OTLPExporter;
    use apcore::observability::span::{Span, SpanExporter};
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = BufReader::new(stream);
        let mut request_line = String::new();
        stream.read_line(&mut request_line).await.unwrap();
        let mut content_length = None;
        loop {
            let mut line = String::new();
            assert!(stream.read_line(&mut line).await.unwrap() > 0);
            if line == "\r\n" {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                if name.eq_ignore_ascii_case("content-length") {
                    content_length = Some(value.trim().parse::<usize>().unwrap());
                }
            }
        }
        let mut body = vec![0; content_length.unwrap()];
        stream.read_exact(&mut body).await.unwrap();
        stream
            .get_mut()
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        (
            request_line,
            serde_json::from_slice::<Value>(&body).unwrap(),
        )
    });

    // The shape `observability.tracing.otlp_endpoint` and the default carry.
    let exporter = OTLPExporter::new(format!("http://127.0.0.1:{port}/v1/traces"));
    let mut span = Span::new("s", "4bf92f3577b34da6a3ce929d0e0e4736");
    span.parent_span_id = Some("00f067aa0ba902b7".to_owned());
    tokio::time::timeout(Duration::from_secs(2), exporter.export(&span))
        .await
        .unwrap()
        .unwrap();
    let (request, payload) = tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
    assert!(
        request.starts_with("POST /v1/traces HTTP/1.1"),
        "the endpoint must be posted to as given: {request}"
    );
    let exported = &payload["resourceSpans"][0]["scopeSpans"][0]["spans"][0];
    assert_eq!(exported["traceId"], span.trace_id);
    assert_eq!(exported["spanId"], span.span_id);
    assert_eq!(exported["parentSpanId"], span.parent_span_id.unwrap());
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

fn apcore_with_control_approval(config: Config, mut executor: Option<apcore::Executor>) -> APCore {
    if let Some(executor) = &mut executor {
        executor.set_approval_handler(Box::new(apcore::AutoApproveHandler));
    }
    APCore::try_start(apcore::APCoreOptions {
        config: Some(config),
        executor,
        approval_handler: Some(Box::new(apcore::AutoApproveHandler)),
        ..Default::default()
    })
    .unwrap()
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
    let client = apcore_with_control_approval(config, None);
    client.register("probe.toggled", fixed(json!({}))).unwrap();
    client.disable("probe.toggled", Some("test")).await.unwrap();

    let written = std::fs::read_to_string(&overrides).expect("overrides file written");
    assert!(written.contains("probe.toggled"), "{written}");
}

#[tokio::test]
async fn reload_module_can_reload_the_config_of_an_apcore_client() {
    let dir = tempfile::tempdir().unwrap();
    let client = apcore_with_control_approval(sys_config(dir.path(), ""), None);
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

#[tokio::test]
async fn test_update_config_rejects_empty_path_without_persisting_or_emitting() {
    let dir = tempfile::tempdir().unwrap();
    let mut client = apcore_with_control_approval(
        sys_config(
            dir.path(),
            "  control:\n    overrides_path: overrides.yaml\nextensions:\n  root: original\n",
        ),
        None,
    );
    let changes = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&changes);
    client
        .on("apcore.config.updated", move |_| {
            observed.fetch_add(1, Ordering::Relaxed);
        })
        .unwrap();

    let invalid = client
        .call(
            "system.control.update_config",
            json!({"key": "extensions.root", "value": "", "reason": "test"}),
            None,
            None,
        )
        .await;
    client.events().unwrap().flush_default().await.unwrap();
    assert_eq!(changes.load(Ordering::Relaxed), 0);
    assert!(!dir.path().join("overrides.yaml").exists());
    assert_eq!(invalid.unwrap_err().code, ErrorCode::ConfigInvalid);

    let valid = client
        .call(
            "system.control.update_config",
            json!({"key": "extensions.root", "value": "replacement", "reason": "test"}),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(valid["old_value"], "original");
    assert_eq!(valid["new_value"], "replacement");
}

struct TimeoutProbe;

#[async_trait]
impl Module for TimeoutProbe {
    fn input_schema(&self) -> Value {
        json!({ "type": "object" })
    }

    fn output_schema(&self) -> Value {
        json!({ "type": "object" })
    }

    fn description(&self) -> &'static str {
        "Await the executor's default timeout"
    }

    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        std::future::pending().await
    }
}

async fn expect_runtime_timeout(client: &APCore, timeout_ms: u64) {
    let error = client
        .call("probe.timeout", json!({}), None, None)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ModuleTimeout);
    assert_eq!(error.details["timeout_ms"], timeout_ms);
}

fn read_executor_config(path: &std::path::Path, timeout_ms: u64) -> Config {
    std::fs::write(
        path,
        format!(
            "version: '1.0'\nproject:\n  name: executor-probe\nexecutor:\n  default_timeout: {timeout_ms}\n"
        ),
    )
    .unwrap();
    Config::from_yaml_file(path).unwrap()
}

#[tokio::test]
async fn test_control_reload_refreshes_the_supplied_executor_and_rolls_back_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    let executor_source = dir.path().join("executor.yaml");
    let executor =
        apcore::Executor::new(Registry::new(), read_executor_config(&executor_source, 40));
    let mut client = apcore_with_control_approval(sys_config(dir.path(), ""), Some(executor));
    client
        .register("probe.timeout", Box::new(TimeoutProbe))
        .unwrap();
    let reloads = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&reloads);
    client
        .on("apcore.config.reloaded", move |_| {
            observed.fetch_add(1, Ordering::Relaxed);
        })
        .unwrap();
    expect_runtime_timeout(&client, 40).await;
    read_executor_config(&executor_source, 25);

    let input = json!({"reload_config": true, "reason": "test"});
    let result = client
        .call("system.control.reload_module", input.clone(), None, None)
        .await
        .unwrap();
    assert_eq!(
        result,
        json!({"success": true, "module_id": null, "config_reloaded": true})
    );
    client.events().unwrap().flush_default().await.unwrap();
    assert_eq!(reloads.load(Ordering::Relaxed), 1);
    expect_runtime_timeout(&client, 25).await;

    sys_config(dir.path(), "extensions:\n  root: changed\n");
    std::fs::remove_file(executor_source).unwrap();
    let result = client
        .call("system.control.reload_module", input, None, None)
        .await
        .unwrap();
    assert_eq!(
        result,
        json!({"success": false, "module_id": null, "config_reloaded": false})
    );
    client.events().unwrap().flush_default().await.unwrap();
    assert_eq!(reloads.load(Ordering::Relaxed), 1);
    expect_runtime_timeout(&client, 25).await;
    let result = client
        .call(
            "system.control.update_config",
            json!({"key": "extensions.root", "value": "another", "reason": "test"}),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(result["old_value"], "./extensions");
}

#[tokio::test]
async fn test_control_reload_reapplies_persisted_config_overrides() {
    let dir = tempfile::tempdir().unwrap();
    let config = sys_config(
        dir.path(),
        "  control:\n    overrides_path: overrides.yaml\nexecutor:\n  default_timeout: 45\n",
    );
    let client = apcore_with_control_approval(config, None);
    client
        .register("probe.timeout", Box::new(TimeoutProbe))
        .unwrap();
    let updated = client
        .call(
            "system.control.update_config",
            json!({"key": "executor.default_timeout", "value": 22, "reason": "test"}),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(updated["new_value"], 22);
    let key = "apcore.executor.default_timeout";
    let updated = client
        .call(
            "system.control.update_config",
            json!({"key": key, "value": 18, "reason": "test"}),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(updated["key"], key);
    assert_eq!(updated["old_value"], 22);
    assert_eq!(updated["new_value"], 18);
    let result = client
        .call(
            "system.control.reload_module",
            json!({"reload_config": true, "reason": "test"}),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(result["config_reloaded"], true);
    expect_runtime_timeout(&client, 18).await;
}

#[tokio::test]
async fn test_control_reload_removes_a_persisted_override_from_the_source_less_executor() {
    let dir = tempfile::tempdir().unwrap();
    let config = sys_config(
        dir.path(),
        "  control:\n    overrides_path: overrides.yaml\nexecutor:\n  default_timeout: 45\n",
    );
    let mut executor_config = Config::from_defaults();
    executor_config.set("executor.default_timeout", json!(35));
    let executor = apcore::Executor::new(Registry::new(), executor_config);
    let client = apcore_with_control_approval(config, Some(executor));
    client
        .register("probe.timeout", Box::new(TimeoutProbe))
        .unwrap();
    client
        .call(
            "system.control.update_config",
            json!({"key": "executor.default_timeout", "value": 22, "reason": "test"}),
            None,
            None,
        )
        .await
        .unwrap();
    expect_runtime_timeout(&client, 22).await;

    std::fs::remove_file(dir.path().join("overrides.yaml")).unwrap();
    let result = client
        .call(
            "system.control.reload_module",
            json!({"reload_config": true, "reason": "test"}),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(result["config_reloaded"], true);
    expect_runtime_timeout(&client, 35).await;
}

#[tokio::test]
async fn test_control_update_reports_the_bare_value_for_a_qualified_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("apcore.yaml");
    std::fs::write(
        &path,
        "apcore:\n  version: '1.0'\n  project:\n    name: path-probe\n  sys_modules:\n    enabled: true\n    events:\n      enabled: true\n",
    )
    .unwrap();
    let client = apcore_with_control_approval(Config::from_yaml_file(&path).unwrap(), None);
    let key = "apcore.extensions.root";
    let result = client
        .call(
            "system.control.update_config",
            json!({"key": key, "value": "replacement", "reason": "test"}),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        result,
        json!({"success": true, "key": key, "old_value": "./extensions", "new_value": "replacement"})
    );
    let result = client
        .call(
            "system.control.update_config",
            json!({"key": "extensions.root", "value": "another", "reason": "test"}),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(result["old_value"], "replacement");
}

#[tokio::test]
async fn test_control_update_reports_the_supplied_executor_value_for_a_qualified_key() {
    let dir = tempfile::tempdir().unwrap();
    let executor = apcore::Executor::new(
        Registry::new(),
        read_executor_config(&dir.path().join("executor.yaml"), 45),
    );
    let mut client = apcore_with_control_approval(
        sys_config(dir.path(), "executor:\n  default_timeout: 72\n"),
        Some(executor),
    );
    client
        .register("probe.timeout", Box::new(TimeoutProbe))
        .unwrap();
    let changes = Arc::new(StdMutex::new(Vec::new()));
    let observed = Arc::clone(&changes);
    client
        .on("apcore.config.updated", move |event| {
            observed.lock().unwrap().push(event.data.clone());
        })
        .unwrap();

    let key = "apcore.executor.default_timeout";
    let result = client
        .call(
            "system.control.update_config",
            json!({"key": key, "value": 20, "reason": "test"}),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        result,
        json!({"success": true, "key": key, "old_value": 45, "new_value": 20})
    );
    client.events().unwrap().flush_default().await.unwrap();
    let changes = changes.lock().unwrap().clone();
    assert_eq!(changes.len(), 1);
    for field in ["key", "old_value", "new_value"] {
        assert_eq!(changes[0][field], result[field]);
    }
    expect_runtime_timeout(&client, 20).await;
}

#[tokio::test]
async fn test_control_update_redacts_the_canonical_path_of_a_qualified_key() {
    let dir = tempfile::tempdir().unwrap();
    let mut client = apcore_with_control_approval(
        sys_config(
            dir.path(),
            "obs:\n  redaction:\n    sensitive_keys: ['executor.default_*']\n",
        ),
        None,
    );
    let changes = Arc::new(StdMutex::new(Vec::new()));
    let observed = Arc::clone(&changes);
    client
        .on("apcore.config.updated", move |event| {
            observed.lock().unwrap().push(event.data.clone());
        })
        .unwrap();

    let key = "apcore.executor.default_timeout";
    let result = client
        .call(
            "system.control.update_config",
            json!({"key": key, "value": 20, "reason": "test"}),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(result["key"], key);
    assert_eq!(result["old_value"], "***REDACTED***");
    assert_eq!(result["new_value"], "***REDACTED***");
    client.events().unwrap().flush_default().await.unwrap();
    let changes = changes.lock().unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["old_value"], "***REDACTED***");
    assert_eq!(changes[0]["new_value"], "***REDACTED***");
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
