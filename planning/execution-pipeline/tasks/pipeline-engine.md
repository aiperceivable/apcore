# Task: Pipeline Engine

## Goal

Implement PipelineEngine with `run()` and `run_stream()` methods that execute an ExecutionStrategy step by step using an index-based loop with skip_to support, trace accumulation, and abort handling.

## Files Involved

### Python SDK
- **Modify:** `apcore-python/src/apcore/pipeline.py` (add PipelineEngine class)
- **Modify:** `apcore-python/tests/test_pipeline.py` (add engine tests)

### TypeScript SDK
- **Modify:** `apcore-typescript/src/pipeline.ts` (add PipelineEngine class)
- **Modify:** `apcore-typescript/tests/test-pipeline.test.ts` (add engine tests)

### Rust SDK
- **Modify:** `apcore-rust/src/pipeline.rs` (add PipelineEngine impl)
- **Modify:** `apcore-rust/tests/test_pipeline.rs` (add engine tests)

## Steps (TDD)

### Step 1: Write failing tests (Python)

Test cases for PipelineEngine.run():
- `test_engine_runs_all_steps_in_order` — 3 mock steps all return continue, trace has 3 entries
- `test_engine_abort_stops_pipeline` — step 2 returns abort, raises PipelineAbortError with trace
- `test_engine_abort_error_carries_trace` — PipelineAbortError.trace has steps up to abort point
- `test_engine_skip_to_jumps_forward` — step 1 returns skip_to="step3", step2 is marked skipped in trace
- `test_engine_skip_to_nonexistent_raises` — skip_to targets missing step, raises StepNotFoundError
- `test_engine_skip_to_only_forward` — skip_to can only target steps after current position
- `test_engine_trace_records_durations` — each StepTrace has duration_ms > 0
- `test_engine_trace_records_decision_points` — steps with confidence != None have decision_point=True
- `test_engine_exception_in_step_propagates` — step raises arbitrary exception, trace records abort
- `test_engine_returns_validated_output_when_available` — returns ctx.validated_output over ctx.output
- `test_engine_returns_output_when_no_validation` — returns ctx.output when validated_output is None
- `test_engine_empty_strategy` — strategy with 0 steps returns (None, trace) with success=True
- `test_engine_trace_success_flag` — trace.success is True on completion, False on abort

Test cases for PipelineEngine.run_stream():
- `test_engine_stream_sets_ctx_stream_flag` — ctx.stream is True during streaming execution
- `test_engine_stream_returns_async_generator` — returns (generator, trace) tuple
- `test_engine_stream_trace_populated_progressively` — steps 1-7 traces populated before return

### Step 2: Implement Python PipelineEngine

```python
class PipelineEngine:
    async def run(self, strategy: ExecutionStrategy, ctx: PipelineContext) -> tuple[Any, PipelineTrace]: ...
    async def run_stream(self, strategy: ExecutionStrategy, ctx: PipelineContext) -> tuple[AsyncGenerator, PipelineTrace]: ...
```

Core loop: index-based while loop (not for-each) to support skip_to. On each iteration:
1. Execute step, measure duration
2. On continue: i += 1
3. On skip_to: find target index forward from current, record skipped steps in trace, jump
4. On abort: raise PipelineAbortError with accumulated trace
5. On exception: record in trace, re-raise

### Step 3: Run Python tests, verify all pass

### Step 4: Write failing tests (TypeScript)

Mirror Python tests with vitest syntax.

### Step 5: Implement TypeScript PipelineEngine

### Step 6: Run TypeScript tests, verify all pass

### Step 7: Write failing tests (Rust)

Mirror tests with Rust/tokio conventions.

### Step 8: Implement Rust PipelineEngine

### Step 9: Run Rust tests, verify all pass

## Acceptance Criteria

- [ ] PipelineEngine.run() executes all steps in order when all return continue
- [ ] Abort action raises PipelineAbortError with accumulated trace
- [ ] skip_to action jumps forward, records skipped steps in trace
- [ ] skip_to to nonexistent step raises StepNotFoundError
- [ ] Step exceptions are recorded in trace before re-raising
- [ ] Trace records duration_ms per step and total_duration_ms
- [ ] decision_point is True when step result has confidence != None
- [ ] Returns validated_output when available, falls back to output
- [ ] trace.success is True only on full completion
- [ ] run_stream() sets ctx.stream=True and returns (generator, trace)
- [ ] All implemented in all 3 SDKs

## Dependencies

- **Depends on:** core-types (Step, StepResult, PipelineContext, PipelineTrace, ExecutionStrategy, error types)
- **Required by:** executor-refactor

## Estimated Time

5 hours
