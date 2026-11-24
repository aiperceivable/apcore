// APCore Protocol — Tracing middleware
// Spec reference: Automatic span creation around module execution

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use super::span::{Span, SpanExporter, SpanStatus};
use crate::context::Context;
use crate::errors::ModuleError;
use crate::middleware::base::Middleware;

/// Strategy for deciding which spans to sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SamplingStrategy {
    /// Sample all spans (serializes as "full").
    #[serde(rename = "full")]
    Always,
    /// Sample spans at the configured rate (serializes as "proportional").
    #[serde(rename = "proportional")]
    Probabilistic,
    /// Sample errors always, success at rate (serializes as "`error_first`").
    #[serde(rename = "error_first")]
    ErrorFirst,
    /// Never sample spans (serializes as "off").
    #[serde(rename = "off")]
    Never,
}

/// Combined state for span stacks and sampling decisions, protected by a single mutex
/// to avoid TOCTOU races during cleanup.
#[derive(Debug, Default)]
struct TraceState {
    spans: HashMap<String, Vec<Span>>,
    sampling: HashMap<String, bool>,
}

/// Middleware that creates tracing spans around module execution.
///
/// Uses a stack-based approach (Vec of Spans per `trace_id`) to correctly
/// handle nested module-to-module calls with proper parent-child span
/// relationships.
///
/// Lock ordering: always acquire `ctx.data` before `self.state` to prevent
/// deadlocks.
#[derive(Debug)]
pub struct TracingMiddleware {
    /// The active span exporter. Wrapped in a `Mutex<Arc<…>>` for interior
    /// mutability so `set_exporter` can swap it on an already-registered
    /// middleware (the middleware chain holds `Arc<dyn Middleware>`, which is
    /// shared and immutable). Sync finding A-D-18: span_exporter extensions
    /// reconfigure an existing TracingMiddleware rather than appending a new one.
    exporter: Mutex<Arc<dyn SpanExporter>>,
    pub sampling_strategy: SamplingStrategy,
    pub sampling_rate: f64,
    /// Combined span stacks and sampling decisions, protected by a single mutex.
    state: Mutex<TraceState>,
}

impl TracingMiddleware {
    /// Create a new tracing middleware with the given exporter (Always sampling).
    #[must_use]
    pub fn new(exporter: Box<dyn SpanExporter>) -> Self {
        Self {
            exporter: Mutex::new(Arc::from(exporter)),
            sampling_strategy: SamplingStrategy::Always,
            sampling_rate: 1.0,
            state: Mutex::new(TraceState::default()),
        }
    }

    /// Create with explicit sampling configuration.
    #[must_use]
    pub fn with_sampling(
        exporter: Box<dyn SpanExporter>,
        strategy: SamplingStrategy,
        rate: f64,
    ) -> Self {
        Self {
            exporter: Mutex::new(Arc::from(exporter)),
            sampling_strategy: strategy,
            sampling_rate: rate.clamp(0.0, 1.0),
            state: Mutex::new(TraceState::default()),
        }
    }

    /// Replace the active span exporter. Used by the extension-application
    /// path to reconfigure a TracingMiddleware that is already registered in
    /// the executor's middleware chain (sync finding A-D-18). Mirrors
    /// apcore-python `TracingMiddleware.set_exporter`.
    pub fn set_exporter(&self, exporter: Box<dyn SpanExporter>) {
        self.set_exporter_shared(Arc::from(exporter));
    }

    /// Install an already-shared exporter handle.
    ///
    /// The counterpart to [`Self::set_exporter`] for callers that keep their
    /// own reference — [`ExtensionManager::apply`](crate::extensions::ExtensionManager::apply)
    /// retains its registrations (D-78), so it wires a clone rather than the
    /// only copy.
    pub fn set_exporter_shared(&self, exporter: Arc<dyn SpanExporter>) {
        *self.exporter.lock() = exporter;
    }

    /// Snapshot the current exporter handle (clones the `Arc`, not the
    /// exporter) so the export call can `.await` without holding the lock.
    fn exporter_handle(&self) -> Arc<dyn SpanExporter> {
        Arc::clone(&self.exporter.lock())
    }

    /// Determine if this request should be sampled, inheriting from parent if available.
    fn should_sample(&self, ctx: &Context<serde_json::Value>) -> bool {
        // Check for inherited decision first (from context.data)
        if let Some(sampled) = ctx.data.read().get("_apcore.mw.tracing.sampled").cloned() {
            if let Some(b) = sampled.as_bool() {
                return b;
            }
        }

        let mut state = self.state.lock();

        // Check cached decision for this trace
        if let Some(&decision) = state.sampling.get(&ctx.trace_id) {
            return decision;
        }

        // Make a new decision
        let decision = match self.sampling_strategy {
            SamplingStrategy::Always => true,
            SamplingStrategy::Never => false,
            SamplingStrategy::Probabilistic | SamplingStrategy::ErrorFirst => {
                #[allow(clippy::cast_precision_loss)]
                // intentional: sampling ratio; minor precision loss is acceptable
                let random_val = uuid::Uuid::new_v4().as_u128() as f64 / u128::MAX as f64;
                random_val < self.sampling_rate
            }
        };

        // Cache the decision
        state.sampling.insert(ctx.trace_id.clone(), decision);

        decision
    }
}

/// Mirror the span stack into `context.data` under `_apcore.mw.tracing.spans`,
/// the key apcore-python and apcore-typescript keep their stack under, so
/// [`TraceContext::inject`](crate::trace_context::TraceContext::inject) takes
/// the current span as the outbound parent. The context's data map is shared
/// by the whole call tree, as the stack is.
fn publish_span_stack(ctx: &Context<serde_json::Value>, stack: &[Span]) {
    if stack.is_empty() {
        crate::context_keys::TRACING_SPANS.delete(ctx);
        return;
    }
    let spans = stack
        .iter()
        .filter_map(|span| serde_json::to_value(span).ok())
        .collect();
    crate::context_keys::TRACING_SPANS.set(ctx, spans);
}

#[async_trait]
impl Middleware for TracingMiddleware {
    fn name(&self) -> &'static str {
        "tracing"
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    async fn before(
        &self,
        module_id: &str,
        _inputs: serde_json::Value,
        ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        self.should_sample(ctx);

        let mut span = Span::new("apcore.module.execute", &ctx.trace_id);

        span.set_attribute("module_id".to_string(), serde_json::json!(module_id));
        span.set_attribute("method".to_string(), serde_json::json!("execute"));
        if let Some(ref caller_id) = ctx.caller_id {
            span.set_attribute("caller_id".to_string(), serde_json::json!(caller_id));
        }

        // Single lock scope: read parent span_id and push the new span
        let snapshot = {
            let mut state = self.state.lock();
            let stack = state.spans.entry(ctx.trace_id.clone()).or_default();
            if let Some(parent) = stack.last() {
                span.parent_span_id = Some(parent.span_id.clone());
            }
            stack.push(span);
            stack.clone()
        };
        publish_span_stack(ctx, &snapshot);

        Ok(None)
    }

    async fn after(
        &self,
        _module_id: &str,
        _inputs: serde_json::Value,
        _output: serde_json::Value,
        ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        // Single lock scope: pop span and clean up empty stacks
        let (span, should_clean_sampling, remaining) = {
            let mut state = self.state.lock();
            let popped = state
                .spans
                .get_mut(&ctx.trace_id)
                .and_then(std::vec::Vec::pop);

            let should_clean = if let Some(stack) = state.spans.get(&ctx.trace_id) {
                if stack.is_empty() {
                    state.spans.remove(&ctx.trace_id);
                    true
                } else {
                    false
                }
            } else {
                false
            };
            let remaining = state.spans.get(&ctx.trace_id).cloned().unwrap_or_default();

            (popped, should_clean, remaining)
        };
        publish_span_stack(ctx, &remaining);

        if let Some(mut span) = span {
            span.status = SpanStatus::Ok;
            span.end();
            let duration_ms = span
                .end_time
                .map_or(0.0, |e| (e - span.start_time) * 1000.0);
            span.set_attribute("duration_ms".to_string(), serde_json::json!(duration_ms));
            span.set_attribute("success".to_string(), serde_json::json!(true));

            if self.should_sample(ctx) {
                let exporter = self.exporter_handle();
                let _ = exporter.export(&span).await;
            }
        }

        // Clean up sampling decision after export (needs separate lock since
        // should_sample/export happened between the two lock scopes)
        if should_clean_sampling {
            let mut state = self.state.lock();
            state.sampling.remove(&ctx.trace_id);
        }

        Ok(None)
    }

    async fn on_error(
        &self,
        _module_id: &str,
        _inputs: serde_json::Value,
        error: &ModuleError,
        ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        // Single lock scope: pop span and clean up empty stacks
        let (span, should_clean_sampling, remaining) = {
            let mut state = self.state.lock();
            let popped = state
                .spans
                .get_mut(&ctx.trace_id)
                .and_then(std::vec::Vec::pop);

            let should_clean = if let Some(stack) = state.spans.get(&ctx.trace_id) {
                if stack.is_empty() {
                    state.spans.remove(&ctx.trace_id);
                    true
                } else {
                    false
                }
            } else {
                false
            };
            let remaining = state.spans.get(&ctx.trace_id).cloned().unwrap_or_default();

            (popped, should_clean, remaining)
        };
        publish_span_stack(ctx, &remaining);

        if let Some(mut span) = span {
            span.status = SpanStatus::Error;
            span.end();
            let duration_ms = span
                .end_time
                .map_or(0.0, |e| (e - span.start_time) * 1000.0);
            span.set_attribute("duration_ms".to_string(), serde_json::json!(duration_ms));
            span.set_attribute("success".to_string(), serde_json::json!(false));
            // ERR-001: the canonical SCREAMING_SNAKE wire code, never the Rust
            // enum's PascalCase `Debug` name. apcore-python (`tracing.py`:
            // `getattr(error, "code", ...)`) and apcore-typescript
            // (`tracing.ts`: `error['code']`) both write the protocol code
            // here, so an OTLP query or alert rule keyed on it matched two SDKs
            // and never this one.
            span.set_attribute(
                "error_code".to_string(),
                serde_json::json!(error.code.wire_str()),
            );
            span.set_attribute(
                "error.message".to_string(),
                serde_json::json!(error.message),
            );
            span.add_event("exception");

            // For ErrorFirst strategy, always export errors regardless of sampling decision
            let should_export = match self.sampling_strategy {
                SamplingStrategy::ErrorFirst => true,
                _ => self.should_sample(ctx),
            };

            if should_export {
                let exporter = self.exporter_handle();
                let _ = exporter.export(&span).await;
            }
        }

        // Clean up sampling decision after export
        if should_clean_sampling {
            let mut state = self.state.lock();
            state.sampling.remove(&ctx.trace_id);
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Identity;
    use crate::observability::exporters::InMemoryExporter;

    fn make_ctx(trace_id: &str) -> Context<serde_json::Value> {
        Context::create(
            Some(Identity::new(
                "test-user".to_string(),
                "user".to_string(),
                vec![],
                HashMap::new(),
            )),
            None,
            None,
            None,
            serde_json::Value::Null,
            None,
        )
        .tap_trace_id(trace_id)
    }

    /// Helper trait to set trace_id in a builder-like fashion for tests.
    trait TapTraceId {
        fn tap_trace_id(self, trace_id: &str) -> Self;
    }

    impl TapTraceId for Context<serde_json::Value> {
        fn tap_trace_id(mut self, trace_id: &str) -> Self {
            self.trace_id = trace_id.to_string();
            self
        }
    }

    #[tokio::test]
    async fn test_nested_spans_parent_child_linking() {
        let exporter = InMemoryExporter::new();
        let mw = TracingMiddleware::new(Box::new(exporter.clone()));
        let ctx = make_ctx("trace-1");

        // before("a") -> before("b") -> after("b") -> after("a")
        mw.before("mod_a", serde_json::json!({}), &ctx)
            .await
            .unwrap();
        mw.before("mod_b", serde_json::json!({}), &ctx)
            .await
            .unwrap();
        mw.after("mod_b", serde_json::json!({}), serde_json::json!({}), &ctx)
            .await
            .unwrap();
        mw.after("mod_a", serde_json::json!({}), serde_json::json!({}), &ctx)
            .await
            .unwrap();

        let spans = exporter.get_spans();
        assert_eq!(spans.len(), 2, "expected 2 exported spans");

        // First exported span is mod_b (inner), second is mod_a (outer)
        let span_b = &spans[0];
        let span_a = &spans[1];

        assert_eq!(
            span_b.attributes.get("module_id").unwrap(),
            &serde_json::json!("mod_b")
        );
        assert_eq!(
            span_a.attributes.get("module_id").unwrap(),
            &serde_json::json!("mod_a")
        );

        // mod_b's parent should be mod_a's span_id
        assert_eq!(
            span_b.parent_span_id.as_ref().unwrap(),
            &span_a.span_id,
            "inner span should reference outer span as parent"
        );

        // mod_a should have no parent
        assert!(
            span_a.parent_span_id.is_none(),
            "root span should have no parent"
        );
    }

    #[tokio::test]
    async fn test_nested_spans_cleanup_after_all_pops() {
        let exporter = InMemoryExporter::new();
        let mw = TracingMiddleware::new(Box::new(exporter.clone()));
        let ctx = make_ctx("trace-cleanup");

        // Push two, pop two
        mw.before("mod_a", serde_json::json!({}), &ctx)
            .await
            .unwrap();
        mw.before("mod_b", serde_json::json!({}), &ctx)
            .await
            .unwrap();
        mw.after("mod_b", serde_json::json!({}), serde_json::json!({}), &ctx)
            .await
            .unwrap();
        mw.after("mod_a", serde_json::json!({}), serde_json::json!({}), &ctx)
            .await
            .unwrap();

        // State should be fully cleaned up — no leftover entries
        let state = mw.state.lock();
        assert!(
            !state.spans.contains_key("trace-cleanup"),
            "span stack should be removed after all spans are popped"
        );
        assert!(
            !state.sampling.contains_key("trace-cleanup"),
            "sampling decision should be removed after trace completes"
        );
    }

    #[tokio::test]
    async fn test_sampling_decision_inherited_from_parent() {
        // Use "Always" strategy so sampling is deterministically true
        let exporter = InMemoryExporter::new();
        let mw = TracingMiddleware::with_sampling(
            Box::new(exporter.clone()),
            SamplingStrategy::Always,
            1.0,
        );
        let ctx = make_ctx("trace-inherit");

        // First call caches the sampling decision
        mw.before("mod_a", serde_json::json!({}), &ctx)
            .await
            .unwrap();

        // Verify the decision is cached
        {
            let state = mw.state.lock();
            assert_eq!(
                state.sampling.get("trace-inherit"),
                Some(&true),
                "sampling decision should be cached after first before()"
            );
        }

        // Nested call should inherit the same decision (not create a new one)
        mw.before("mod_b", serde_json::json!({}), &ctx)
            .await
            .unwrap();

        {
            let state = mw.state.lock();
            assert_eq!(
                state.sampling.get("trace-inherit"),
                Some(&true),
                "sampling decision should remain the same for nested calls"
            );
        }

        // Clean up
        mw.after("mod_b", serde_json::json!({}), serde_json::json!({}), &ctx)
            .await
            .unwrap();
        mw.after("mod_a", serde_json::json!({}), serde_json::json!({}), &ctx)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_sampling_never_does_not_export() {
        let exporter = InMemoryExporter::new();
        let mw = TracingMiddleware::with_sampling(
            Box::new(exporter.clone()),
            SamplingStrategy::Never,
            0.0,
        );
        let ctx = make_ctx("trace-never");

        mw.before("mod_a", serde_json::json!({}), &ctx)
            .await
            .unwrap();
        mw.after("mod_a", serde_json::json!({}), serde_json::json!({}), &ctx)
            .await
            .unwrap();

        let spans = exporter.get_spans();
        assert!(
            spans.is_empty(),
            "Never strategy should not export any spans"
        );
    }
}
