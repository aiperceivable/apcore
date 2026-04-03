# Execution Pipeline Strategy — Implementation Plan

## Goal

Refactor the Executor from a hardcoded 11-step pipeline to a configurable ExecutionStrategy. Steps can be added, removed, replaced, and reordered while preserving backward compatibility and cross-language consistency across Python, TypeScript, and Rust SDKs.

## Task Order (dependency-aware)

| # | Task | Depends On | Languages |
|---|------|-----------|-----------|
| 1 | core-types | — | Py, TS, Rust |
| 2 | builtin-steps | core-types | Py, TS, Rust |
| 3 | pipeline-engine | core-types | Py, TS, Rust |
| 4 | executor-refactor | builtin-steps, pipeline-engine | Py, TS, Rust |
| 5 | preset-strategies | executor-refactor | Py, TS, Rust |
| 6 | call-with-trace | executor-refactor | Py, TS, Rust |
| 7 | introspection | preset-strategies, call-with-trace | Py, TS, Rust |

## Dependency Graph

```
core-types
  ├── builtin-steps
  └── pipeline-engine
        └── executor-refactor (also depends on builtin-steps)
              ├── preset-strategies ──┐
              └── call-with-trace ────┤
                                      └── introspection
```

## TDD Approach

Each task:
1. Write failing tests
2. Implement minimum code to pass
3. Run full suite, verify no regressions

## File Changes

### Python
- **NEW** `apcore-python/src/apcore/pipeline.py` — Step protocol, BaseStep, StepResult, PipelineContext, PipelineTrace, ExecutionStrategy, PipelineEngine, error types
- **NEW** `apcore-python/src/apcore/builtin_steps.py` — All 11 BuiltinStep implementations
- **NEW** `apcore-python/src/apcore/strategies.py` — Factory functions: build_standard_strategy, build_internal_strategy, build_testing_strategy, build_validate_only_strategy, build_performance_strategy
- **MOD** `apcore-python/src/apcore/executor.py` — Add strategy parameter, route call/call_async through PipelineEngine, add call_with_trace/call_async_with_trace, add introspection methods
- **NEW** `apcore-python/tests/test_pipeline.py` — Core types and engine tests
- **NEW** `apcore-python/tests/test_builtin_steps.py` — Individual step tests
- **NEW** `apcore-python/tests/test_strategies.py` — Strategy factory and preset tests
- **NEW** `apcore-python/tests/test_executor_pipeline.py` — Executor refactor, backward compat, introspection, call_with_trace tests

### TypeScript
- **NEW** `apcore-typescript/src/pipeline.ts` — Step interface, StepResult, PipelineContext, PipelineTrace, ExecutionStrategy, PipelineEngine, error types
- **NEW** `apcore-typescript/src/builtin-steps.ts` — All 11 BuiltinStep implementations
- **NEW** `apcore-typescript/src/strategies.ts` — Factory functions
- **MOD** `apcore-typescript/src/executor.ts` — Add strategy parameter, route call through PipelineEngine, add callWithTrace, add introspection methods
- **MOD** `apcore-typescript/src/index.ts` — Export new modules
- **NEW** `apcore-typescript/tests/test-pipeline.test.ts` — Core types and engine tests
- **NEW** `apcore-typescript/tests/test-builtin-steps.test.ts` — Individual step tests
- **NEW** `apcore-typescript/tests/test-strategies.test.ts` — Strategy factory and preset tests
- **NEW** `apcore-typescript/tests/test-executor-pipeline.test.ts` — Executor refactor, backward compat, introspection, callWithTrace tests

### Rust
- **NEW** `apcore-rust/src/pipeline.rs` — Step trait, StepResult, PipelineContext, PipelineTrace, ExecutionStrategy, PipelineEngine, error types
- **NEW** `apcore-rust/src/builtin_steps.rs` — All 11 BuiltinStep implementations
- **NEW** `apcore-rust/src/strategies.rs` — Factory functions
- **MOD** `apcore-rust/src/executor.rs` — Add with_strategy constructor, call_with_strategy, call_with_trace, introspection methods
- **MOD** `apcore-rust/src/lib.rs` — Add `pub mod pipeline; pub mod builtin_steps; pub mod strategies;`
- **NEW** `apcore-rust/tests/test_pipeline.rs` — Core types and engine tests
- **NEW** `apcore-rust/tests/test_builtin_steps.rs` — Individual step tests
- **NEW** `apcore-rust/tests/test_strategies.rs` — Strategy factory and preset tests
- **NEW** `apcore-rust/tests/test_executor_pipeline.rs` — Executor refactor, backward compat, introspection, call_with_trace tests
