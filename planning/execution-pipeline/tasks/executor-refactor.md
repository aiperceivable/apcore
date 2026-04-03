# Task: Executor Refactor

## Goal

Modify the Executor constructor to accept an optional `strategy` parameter and route call/call_async through PipelineEngine. When strategy is None, build STANDARD from legacy parameters (backward compatible). When strategy is provided, use it directly.

## BACKWARD COMPATIBILITY (Critical)

This is the most critical constraint of the entire feature. The following must continue to work identically:

```python
# Case 1: Minimal constructor — MUST still work exactly as before
executor = Executor(registry)

# Case 2: Full constructor — MUST still work exactly as before
executor = Executor(
    registry,
    middlewares=[LoggingMiddleware(), MetricsMiddleware()],
    acl=acl,
    approval_handler=handler,
    config=config,
)

# Case 3: All call patterns — MUST produce identical results
result = executor.call("module_id", inputs, context)
result = await executor.call_async("module_id", inputs, context)
async for chunk in executor.stream("module_id", inputs, context): ...

# Case 4: Validation — MUST still work
preflight = executor.validate("module_id", inputs)
```

The new `strategy` parameter is **keyword-only** and **optional**:
```python
# New usage (additive, does not break existing code)
executor = Executor(registry, strategy="internal")
executor = Executor(registry, strategy=my_custom_strategy)
```

When `strategy` is provided, `middlewares`/`acl`/`approval_handler` are ignored (steps in strategy contain their own dependencies).

## Files Involved

### Python SDK
- **Modify:** `apcore-python/src/apcore/executor.py`
- **Create:** `apcore-python/tests/test_executor_pipeline.py`

### TypeScript SDK
- **Modify:** `apcore-typescript/src/executor.ts`
- **Create:** `apcore-typescript/tests/test-executor-pipeline.test.ts`

### Rust SDK
- **Modify:** `apcore-rust/src/executor.rs`
- **Create:** `apcore-rust/tests/test_executor_pipeline.rs`

## Steps (TDD)

### Step 1: Write failing tests (Python)

Test cases:
- `test_executor_default_constructor_unchanged` — Executor(registry) works, produces same results
- `test_executor_full_constructor_unchanged` — Executor(registry, middlewares=..., acl=..., approval_handler=...) works
- `test_executor_call_async_produces_same_result` — call_async with default strategy matches current behavior
- `test_executor_call_sync_produces_same_result` — call() with default strategy matches current behavior
- `test_executor_stream_produces_same_result` — stream() with default strategy matches current behavior
- `test_executor_validate_still_works` — validate() unchanged
- `test_executor_with_strategy_object` — Executor(registry, strategy=my_strategy) uses provided strategy
- `test_executor_with_strategy_string` — Executor(registry, strategy="internal") resolves built-in preset
- `test_executor_strategy_ignores_legacy_params` — when strategy provided, middlewares/acl/approval_handler ignored
- `test_executor_per_call_strategy_override` — call_async(..., strategy="testing") overrides default
- `test_executor_per_call_strategy_object` — call_async(..., strategy=custom) uses provided strategy

### Step 2: Implement Python executor changes

Modify `__init__`:
```python
def __init__(self, registry, *, strategy=None, middlewares=None, acl=None,
             config=None, approval_handler=None):
    if strategy is None:
        self._strategy = build_standard_strategy(
            registry=registry, config=config, acl=acl,
            approval_handler=approval_handler, middlewares=middlewares,
        )
    elif isinstance(strategy, str):
        self._strategy = self._resolve_strategy_name(strategy)
    else:
        self._strategy = strategy
```

Route call_async through PipelineEngine:
```python
async def call_async(self, module_id, inputs, context=None, *, strategy=None):
    effective_strategy = self._resolve_call_strategy(strategy)
    ctx = PipelineContext(module_id=module_id, inputs=inputs, context=context, strategy=effective_strategy, ...)
    result, trace = await self._engine.run(effective_strategy, ctx)
    return result
```

### Step 3: Run Python tests AND full existing test suite, verify zero regressions

### Step 4: Write failing tests (TypeScript)

Mirror Python tests. TS-specific:
- Constructor uses options object: `new Executor({ registry, strategy: "internal" })`
- Per-call strategy via 4th param options: `executor.call(id, inputs, ctx, { strategy: "internal" })`

### Step 5: Implement TypeScript executor changes

### Step 6: Run TypeScript tests AND full existing test suite

### Step 7: Write failing tests (Rust)

Mirror tests. Rust-specific:
- `Executor::new(registry, config)` unchanged
- `Executor::with_strategy(registry, config, strategy)` new constructor
- `executor.call_with_strategy(id, inputs, ctx, &strategy)` for per-call override

### Step 8: Implement Rust executor changes

### Step 9: Run Rust tests AND full existing test suite

## Acceptance Criteria

- [ ] `Executor(registry)` works exactly as before (Python)
- [ ] `Executor(registry, middlewares=[...], acl=acl)` works exactly as before (Python)
- [ ] `new Executor({ registry })` works exactly as before (TypeScript)
- [ ] `Executor::new(registry, config)` works exactly as before (Rust)
- [ ] call/call_async/stream produce identical results with default strategy
- [ ] validate() is unchanged
- [ ] strategy parameter accepts ExecutionStrategy object or string name
- [ ] Per-call strategy override works for call/call_async
- [ ] When strategy provided, legacy params (middlewares/acl/approval_handler) are ignored
- [ ] String strategy resolves: built-in presets -> code-registered -> error
- [ ] Full existing test suites pass with zero regressions in all 3 SDKs

## Dependencies

- **Depends on:** builtin-steps, pipeline-engine
- **Required by:** preset-strategies, call-with-trace

## Estimated Time

8 hours
