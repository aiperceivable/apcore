# Task: Core Types

## Goal

Define the foundational types for the execution pipeline in all 3 SDKs: Step protocol/interface/trait, BaseStep convenience class, StepResult, PipelineContext, PipelineTrace, StepTrace, ExecutionStrategy, StrategyInfo, and all error types (PipelineAbortError, StepNotFoundError, StepNotRemovableError, StepNotReplaceableError, StrategyNotFoundError, StepNameDuplicateError).

## Files Involved

### Python SDK
- **Create:** `apcore-python/src/apcore/pipeline.py`
- **Create:** `apcore-python/tests/test_pipeline.py`

### TypeScript SDK
- **Create:** `apcore-typescript/src/pipeline.ts`
- **Modify:** `apcore-typescript/src/index.ts` (export pipeline types)
- **Create:** `apcore-typescript/tests/test-pipeline.test.ts`

### Rust SDK
- **Create:** `apcore-rust/src/pipeline.rs`
- **Modify:** `apcore-rust/src/lib.rs` (add `pub mod pipeline;`)
- **Create:** `apcore-rust/tests/test_pipeline.rs`

## Steps (TDD)

### Step 1: Write failing tests (Python)

Test cases for `test_pipeline.py`:
- `test_step_result_continue` — StepResult with action="continue" is constructible
- `test_step_result_skip_to` — StepResult with action="skip_to" and skip_to target
- `test_step_result_abort` — StepResult with action="abort", explanation, alternatives
- `test_step_result_confidence` — StepResult stores confidence float
- `test_pipeline_context_creation` — PipelineContext holds module_id, inputs, context, strategy, trace
- `test_pipeline_context_resolved_fields_initially_none` — module, validated_inputs, output, validated_output are None
- `test_pipeline_trace_creation` — PipelineTrace with module_id, strategy_name, steps, success
- `test_step_trace_creation` — StepTrace with name, duration_ms, result, skipped, decision_point
- `test_execution_strategy_creation` — ExecutionStrategy(name, steps) stores name and steps
- `test_execution_strategy_step_names` — step_names() returns ordered list
- `test_execution_strategy_insert_after` — insert_after adds step at correct position
- `test_execution_strategy_insert_before` — insert_before adds step at correct position
- `test_execution_strategy_remove` — remove() removes step by name
- `test_execution_strategy_remove_non_removable_raises` — remove() raises StepNotRemovableError
- `test_execution_strategy_replace` — replace() swaps step implementation
- `test_execution_strategy_replace_non_replaceable_raises` — replace() raises StepNotReplaceableError
- `test_execution_strategy_insert_duplicate_raises` — insert raises StepNameDuplicateError
- `test_execution_strategy_remove_not_found_raises` — remove() raises StepNotFoundError
- `test_strategy_info_creation` — StrategyInfo holds name, step_count, step_names, description
- `test_base_step_subclass` — BaseStep subclass implements execute, has name/description/removable/replaceable
- `test_step_protocol_structural_typing` — Any class with correct attributes satisfies Step protocol
- `test_error_types_extend_module_error` — All pipeline errors extend ModuleError
- `test_pipeline_abort_error_carries_trace` — PipelineAbortError has step, explanation, alternatives, trace

### Step 2: Implement Python core types

In `apcore-python/src/apcore/pipeline.py`:
- `Step` — `@runtime_checkable` Protocol with properties (name, description, removable, replaceable) and `async execute(ctx) -> StepResult`
- `BaseStep` — ABC with `__init__(name, description, removable=True, replaceable=True)` using instance attributes, abstract `execute`
- `StepResult` — dataclass with action (Literal["continue", "skip_to", "abort"]), skip_to, explanation, confidence, alternatives
- `PipelineContext` — dataclass with module_id, inputs, context, strategy, trace, module, validated_inputs, output, validated_output, stream, output_stream
- `PipelineTrace` — dataclass with module_id, strategy_name, steps (list[StepTrace]), total_duration_ms, success
- `StepTrace` — dataclass with name, duration_ms, result, skipped, decision_point
- `ExecutionStrategy` — class with name, steps, insert_after, insert_before, remove, replace, step_names
- `StrategyInfo` — dataclass with name, step_count, step_names, description
- Error types: PipelineAbortError, StepNotFoundError, StepNotRemovableError, StepNotReplaceableError, StrategyNotFoundError, StepNameDuplicateError (all extend ModuleError)

### Step 3: Run Python tests, verify all pass

### Step 4: Write failing tests (TypeScript)

Mirror Python test cases with TypeScript/vitest syntax. Key differences:
- Step is an interface (not protocol)
- StepResult uses string literal union for action
- PipelineContext uses `null` instead of `None`

### Step 5: Implement TypeScript core types

### Step 6: Run TypeScript tests, verify all pass

### Step 7: Write failing tests (Rust)

Mirror test cases with Rust conventions. Key differences:
- Step is an `#[async_trait]` trait with `Send + Sync` bounds
- StepResult uses enum for action
- PipelineContext takes `&mut` reference
- Error types use `thiserror`

### Step 8: Implement Rust core types

### Step 9: Run Rust tests, verify all pass

## Acceptance Criteria

- [ ] Step protocol/interface/trait defined in all 3 SDKs with execute() returning StepResult
- [ ] BaseStep convenience class in all 3 SDKs
- [ ] StepResult supports continue, skip_to, abort actions with optional explanation/confidence/alternatives
- [ ] PipelineContext holds all pipeline state (input, resolved, streaming, metadata)
- [ ] PipelineTrace and StepTrace record execution history
- [ ] ExecutionStrategy supports insert_after, insert_before, remove, replace with safety checks
- [ ] Unique step name invariant enforced (StepNameDuplicateError on duplicate insert)
- [ ] Non-removable steps cannot be removed (StepNotRemovableError)
- [ ] Non-replaceable steps cannot be replaced (StepNotReplaceableError)
- [ ] StrategyInfo for AI introspection
- [ ] All 6 error types defined and extend ModuleError
- [ ] PipelineAbortError carries trace, step name, explanation, alternatives

## Dependencies

- **Depends on:** none (uses existing Context, ModuleError from each SDK)
- **Required by:** builtin-steps, pipeline-engine

## Estimated Time

6 hours
