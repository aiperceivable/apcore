// Integration tests for pipeline tasks:
// executor-refactor, preset-strategies, call-with-trace, introspection

use apcore::config::Config;
use apcore::errors::ModuleError;
use apcore::pipeline::{ExecutionStrategy, PipelineContext, Step, StepResult};
use apcore::registry::registry::Registry;
use apcore::{
    build_internal_strategy, build_performance_strategy, build_standard_strategy,
    build_testing_strategy, list_strategies, register_strategy, Executor,
};
use async_trait::async_trait;
use serde_json::Value;

// ---------------------------------------------------------------------------
// Test helper: step that sets ctx.output
// ---------------------------------------------------------------------------

struct OutputStep {
    output: Value,
}

impl OutputStep {
    fn new(output: Value) -> Self {
        Self { output }
    }
}

#[async_trait]
impl Step for OutputStep {
    fn name(&self) -> &'static str {
        "output_step"
    }
    fn description(&self) -> &'static str {
        "Sets output on the pipeline context"
    }
    fn removable(&self) -> bool {
        true
    }
    fn replaceable(&self) -> bool {
        true
    }
    async fn execute(&self, ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        ctx.output = Some(self.output.clone());
        Ok(StepResult::continue_step())
    }
}

// ---------------------------------------------------------------------------
// Task 1: executor-refactor — with_strategy stores strategy
// ---------------------------------------------------------------------------

#[test]
fn test_executor_with_strategy_stores_strategy() {
    let registry = Registry::new();
    let config = Config::default();
    let strategy = build_testing_strategy();

    let executor = Executor::with_strategy(registry, config, strategy);

    let stored = executor.strategy();
    assert_eq!(stored.name(), "testing");
}

#[test]
fn test_executor_new_has_standard_strategy() {
    let registry = Registry::new();
    let config = Config::default();
    let executor = Executor::new(registry, config);
    assert_eq!(executor.strategy().name(), "standard");
}

// ---------------------------------------------------------------------------
// Task 2: preset-strategies
// ---------------------------------------------------------------------------

#[test]
fn test_preset_internal_strategy() {
    let strategy = build_internal_strategy();
    assert_eq!(strategy.name(), "internal");
    assert_eq!(strategy.steps().len(), 9);
    // Internal skips ACL and approval
    let names = strategy.step_names();
    assert!(names.contains(&"context_creation".to_string()));
    assert!(names.contains(&"execute".to_string()));
    assert!(!names.contains(&"acl_check".to_string()));
    assert!(!names.contains(&"approval_gate".to_string()));
}

#[test]
fn test_preset_testing_strategy() {
    let strategy = build_testing_strategy();
    assert_eq!(strategy.name(), "testing");
    assert_eq!(strategy.steps().len(), 8);
    let names = strategy.step_names();
    assert!(names.contains(&"context_creation".to_string()));
    assert!(names.contains(&"execute".to_string()));
    assert!(!names.contains(&"call_chain_guard".to_string()));
    assert!(!names.contains(&"acl_check".to_string()));
    assert!(!names.contains(&"approval_gate".to_string()));
}

#[test]
fn test_preset_performance_strategy() {
    let strategy = build_performance_strategy();
    assert_eq!(strategy.name(), "performance");
    assert_eq!(strategy.steps().len(), 9);
    let names = strategy.step_names();
    assert!(names.contains(&"context_creation".to_string()));
    assert!(names.contains(&"execute".to_string()));
    assert!(!names.contains(&"middleware_before".to_string()));
    assert!(!names.contains(&"middleware_after".to_string()));
}

// ---------------------------------------------------------------------------
// Task 3: call_with_trace
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_call_with_trace_returns_output_and_trace() {
    let registry = Registry::new();
    let config = Config::default();

    let strategy = ExecutionStrategy::new(
        "simple",
        vec![Box::new(OutputStep::new(serde_json::json!({"result": 42})))],
    )
    .unwrap();

    let executor = Executor::with_strategy(registry, config, strategy);
    let (output, trace) = executor
        .call_with_trace("test_mod", serde_json::json!({}), None, None, None)
        .await
        .unwrap();

    assert_eq!(output, serde_json::json!({"result": 42}));
    assert!(trace.success);
    assert_eq!(trace.steps.len(), 1);
    assert_eq!(trace.steps[0].name, "output_step");
    assert_eq!(trace.strategy_name, "simple");
}

// Sync finding A-D-005 (D-19): call_with_trace accepts and forwards
// version_hint like call(), so the trace variant shares call()'s
// version-negotiation semantics.
#[tokio::test]
async fn test_call_with_trace_accepts_version_hint() {
    let registry = Registry::new();
    let config = Config::default();

    let strategy = ExecutionStrategy::new(
        "simple",
        vec![Box::new(OutputStep::new(serde_json::json!({"result": 7})))],
    )
    .unwrap();

    let executor = Executor::with_strategy(registry, config, strategy);
    let (output, trace) = executor
        .call_with_trace("test_mod", serde_json::json!({}), None, Some("1.0.0"), None)
        .await
        .unwrap();

    assert_eq!(output, serde_json::json!({"result": 7}));
    assert!(trace.success);
}

#[tokio::test]
async fn test_call_with_trace_strategy_override() {
    let registry = Registry::new();
    let config = Config::default();

    let default_strategy = ExecutionStrategy::new(
        "default",
        vec![Box::new(OutputStep::new(
            serde_json::json!({"from": "default"}),
        ))],
    )
    .unwrap();

    let override_strategy = ExecutionStrategy::new(
        "override",
        vec![Box::new(OutputStep::new(
            serde_json::json!({"from": "override"}),
        ))],
    )
    .unwrap();

    let executor = Executor::with_strategy(registry, config, default_strategy);

    // Override should take precedence
    let (output, trace) = executor
        .call_with_trace(
            "mod",
            serde_json::json!({}),
            None,
            None,
            Some(&override_strategy),
        )
        .await
        .unwrap();

    assert_eq!(output, serde_json::json!({"from": "override"}));
    assert_eq!(trace.strategy_name, "override");
}

#[tokio::test]
async fn test_call_with_trace_no_override_uses_default_strategy() {
    let registry = Registry::new();
    let config = Config::default();
    let executor = Executor::new(registry, config);

    // Passing None for strategy uses the executor's default strategy.
    // Module lookup will fail since the registry is empty, but the
    // strategy itself is always available.
    let result = executor
        .call_with_trace("nonexistent", serde_json::json!({}), None, None, None)
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code, apcore::errors::ErrorCode::ModuleNotFound);
}

// ---------------------------------------------------------------------------
// Task 4: introspection — register_strategy, list_strategies, describe_pipeline
// ---------------------------------------------------------------------------

#[test]
fn test_register_and_list_strategies() {
    let strategy = build_internal_strategy();
    let info = strategy.info();
    assert_eq!(info.name, "internal");
    assert_eq!(info.step_count, 9);

    register_strategy(info);

    let all = list_strategies();
    assert!(
        all.iter().any(|s| s.name == "internal"),
        "internal strategy should be in the registry"
    );
}

#[test]
fn test_executor_list_strategies_includes_the_current_strategy() {
    // The module-level `list_strategies()` reports the global registry, which
    // is not pre-seeded with the built-ins — so before this fix a default
    // executor answered the AI-strategy-selection query with an empty menu.
    // `core-executor.md` requires "the executor's current strategy plus one
    // for every strategy registered".
    let registry = Registry::new();
    let config = Config::default();
    let executor = Executor::new(registry, config);

    let all = executor.list_strategies();
    assert!(
        !all.is_empty(),
        "a default executor must report at least its own strategy"
    );
    assert_eq!(
        all[0].name,
        executor.strategy().info().name,
        "the current strategy must come first"
    );

    // A registered strategy joins it, without displacing the current one.
    register_strategy(build_internal_strategy().info());
    let all = executor.list_strategies();
    assert_eq!(all[0].name, executor.strategy().info().name);
    assert!(all.iter().any(|s| s.name == "internal"));

    // Names are unique — the current strategy is not repeated when it is also
    // present in the registry.
    let mut names: Vec<&str> = all.iter().map(|s| s.name.as_str()).collect();
    let before = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(before, names.len(), "strategy names must be deduped");
}

#[test]
fn test_describe_pipeline() {
    let strategy = build_standard_strategy();
    let info = strategy.info();
    assert_eq!(info.name, "standard");
    assert_eq!(info.step_count, 11);
    assert!(info.description.contains("context_creation"));
    assert!(info.description.contains("execute"));
}
