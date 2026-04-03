# Task: Call With Trace

## Goal

Add call_with_trace / call_async_with_trace methods to the Executor in all 3 SDKs. These methods return both the result and the PipelineTrace, enabling AI learning and execution introspection.

## Files Involved

### Python SDK
- **Modify:** `apcore-python/src/apcore/executor.py` (add call_with_trace, call_async_with_trace)
- **Modify:** `apcore-python/tests/test_executor_pipeline.py` (add trace tests)

### TypeScript SDK
- **Modify:** `apcore-typescript/src/executor.ts` (add callWithTrace)
- **Modify:** `apcore-typescript/tests/test-executor-pipeline.test.ts` (add trace tests)

### Rust SDK
- **Modify:** `apcore-rust/src/executor.rs` (add call_with_trace)
- **Modify:** `apcore-rust/tests/test_executor_pipeline.rs` (add trace tests)

## Steps (TDD)

### Step 1: Write failing tests (Python)

Test cases:
- `test_call_with_trace_returns_tuple` — returns (result, trace) tuple
- `test_call_async_with_trace_returns_tuple` — async version returns (result, trace)
- `test_trace_has_correct_module_id` — trace.module_id matches called module
- `test_trace_has_correct_strategy_name` — trace.strategy_name matches used strategy
- `test_trace_has_all_steps` — trace.steps has entry for each step in strategy
- `test_trace_success_on_completion` — trace.success is True on successful execution
- `test_trace_success_false_on_abort` — trace.success is False when pipeline aborts
- `test_trace_total_duration_positive` — trace.total_duration_ms > 0
- `test_trace_step_durations_positive` — each step trace has duration_ms >= 0
- `test_trace_with_strategy_override` — call_with_trace(..., strategy="testing") uses override strategy
- `test_trace_records_skipped_steps` — when step returns skip_to, skipped steps marked in trace
- `test_trace_decision_points` — steps with confidence have decision_point=True
- `test_trace_abort_explanation` — PipelineAbortError from call_with_trace has explanation

### Step 2: Implement Python call_with_trace

```python
# Sync wrapper
def call_with_trace(self, module_id, inputs, context=None, *, strategy=None):
    return asyncio.get_event_loop().run_until_complete(
        self.call_async_with_trace(module_id, inputs, context, strategy=strategy)
    )

# Async implementation
async def call_async_with_trace(self, module_id, inputs, context=None, *, strategy=None):
    effective_strategy = self._resolve_call_strategy(strategy)
    ctx = self._build_pipeline_context(module_id, inputs, context, effective_strategy)
    result, trace = await self._engine.run(effective_strategy, ctx)
    return result, trace
```

### Step 3: Run Python tests, verify all pass

### Step 4-6: TypeScript

TypeScript signature:
```typescript
async callWithTrace(
    moduleId: string, inputs: Record<string, unknown>,
    context?: Context | null,
    options?: { strategy?: ExecutionStrategy | string }
): Promise<[Record<string, unknown>, PipelineTrace]>
```

### Step 7-9: Rust

Rust signature:
```rust
pub async fn call_with_trace(
    &self, module_id: &str, inputs: Value,
    ctx: Option<&Context<Value>>,
    strategy: Option<&ExecutionStrategy>,
) -> Result<(Value, PipelineTrace), ModuleError>
```

## Acceptance Criteria

- [ ] call_with_trace returns (result, PipelineTrace) in Python (sync)
- [ ] call_async_with_trace returns (result, PipelineTrace) in Python (async)
- [ ] callWithTrace returns [result, PipelineTrace] in TypeScript
- [ ] call_with_trace returns Result<(Value, PipelineTrace), ModuleError> in Rust
- [ ] Trace contains correct module_id, strategy_name, step entries
- [ ] Trace records success/failure, durations, decision points
- [ ] Trace records skipped steps on skip_to
- [ ] Strategy override parameter works with trace methods
- [ ] PipelineAbortError carries trace when abort occurs

## Dependencies

- **Depends on:** executor-refactor
- **Required by:** introspection

## Estimated Time

3 hours
