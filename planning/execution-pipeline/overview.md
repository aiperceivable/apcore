# Execution Pipeline Strategy — Overview

## Goal

Refactor the Executor from a hardcoded 11-step pipeline to a configurable ExecutionStrategy with AI decision support. Steps can be added, removed, replaced, and reordered. Preserves full backward compatibility: `Executor(registry)` and `Executor(registry, middlewares=[...], acl=acl)` continue to work without changes.

## Scope

- **Core types** — Step protocol/interface/trait, StepResult, PipelineContext, PipelineTrace, ExecutionStrategy, StrategyInfo, BaseStep, error types
- **Built-in steps** — Extract current 11 executor steps into BuiltinStep implementations
- **Pipeline engine** — Index-based execution loop with skip_to, abort, trace accumulation
- **Executor refactor** — Accept strategy parameter, build STANDARD from legacy params when strategy=None
- **Preset strategies** — STANDARD, INTERNAL, TESTING, VALIDATE_ONLY, PERFORMANCE factory functions
- **Call with trace** — call_with_trace / call_async_with_trace methods
- **Introspection** — list_strategies, describe_pipeline, register_strategy, current_strategy

## Repos

- Python: `apcore-python/src/apcore/pipeline.py`, `builtin_steps.py`, `strategies.py`, `executor.py`
- TypeScript: `apcore-typescript/src/pipeline.ts`, `builtin-steps.ts`, `strategies.ts`, `executor.ts`
- Rust: `apcore-rust/src/pipeline.rs`, `builtin_steps.rs`, `strategies.rs`, `executor.rs`

## Key Design Decisions

1. Step.execute() is always async in all 3 SDKs
2. ExecutionStrategy is a factory function (not constant) because steps require runtime deps
3. PipelineContext uses two-tier data model: Tier 1 (direct fields) + Tier 2 (ContextKey)
4. Steps do NOT pass data through StepResult — StepResult only controls flow
5. Step names MUST be unique within a strategy (enforced at insert time)
6. Steps 1, 3, 8, 11 are non-removable; steps 7, 10 are non-replaceable
7. Zero breaking changes: strategy parameter is optional, defaults to STANDARD

## Task Execution Order

| # | Task | Status |
|---|------|--------|
| 1 | core-types | pending |
| 2 | builtin-steps | pending |
| 3 | pipeline-engine | pending |
| 4 | executor-refactor | pending |
| 5 | preset-strategies | pending |
| 6 | call-with-trace | pending |
| 7 | introspection | pending |
