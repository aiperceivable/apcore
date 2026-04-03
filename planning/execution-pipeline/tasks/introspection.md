# Task: Introspection

## Goal

Add introspection methods to the Executor for AI perceivability: list_strategies(), describe_pipeline(), register_strategy() (class-level), and current_strategy property. These enable AI agents to inspect available strategies and choose at call time.

## Files Involved

### Python SDK
- **Modify:** `apcore-python/src/apcore/executor.py` (add introspection methods)
- **Modify:** `apcore-python/tests/test_executor_pipeline.py` (add introspection tests)

### TypeScript SDK
- **Modify:** `apcore-typescript/src/executor.ts` (add introspection methods)
- **Modify:** `apcore-typescript/tests/test-executor-pipeline.test.ts` (add introspection tests)

### Rust SDK
- **Modify:** `apcore-rust/src/executor.rs` (add introspection methods)
- **Modify:** `apcore-rust/tests/test_executor_pipeline.rs` (add introspection tests)

## Steps (TDD)

### Step 1: Write failing tests (Python)

Test cases:
- `test_list_strategies_returns_strategy_infos` — returns list of StrategyInfo objects
- `test_list_strategies_includes_builtins` — contains standard, internal, testing, validate_only, performance
- `test_list_strategies_includes_registered` — after register_strategy("custom", s), custom appears
- `test_strategy_info_has_step_names` — each StrategyInfo has correct step_names list
- `test_strategy_info_has_step_count` — step_count matches len(step_names)
- `test_strategy_info_has_description` — description is auto-generated from step descriptions
- `test_describe_pipeline_returns_string` — returns human/AI-readable string
- `test_describe_pipeline_includes_step_names` — string includes all step names
- `test_describe_pipeline_includes_step_count` — string includes step count
- `test_current_strategy_returns_default` — returns the strategy set in constructor
- `test_current_strategy_after_construction` — Executor(strategy="internal") returns internal strategy
- `test_register_strategy_class_method` — Executor.register_strategy("my", strategy) is class-level
- `test_register_strategy_available_in_list` — registered strategy appears in list_strategies()
- `test_register_strategy_resolvable_by_name` — Executor(strategy="my") works after registration
- `test_strategy_not_found_raises` — Executor(strategy="nonexistent") raises StrategyNotFoundError

### Step 2: Implement Python introspection

```python
class Executor:
    _registered_strategies: ClassVar[dict[str, ExecutionStrategy]] = {}

    @classmethod
    def register_strategy(cls, name: str, strategy: ExecutionStrategy) -> None:
        cls._registered_strategies[name] = strategy

    @property
    def current_strategy(self) -> ExecutionStrategy:
        return self._strategy

    def list_strategies(self) -> list[StrategyInfo]:
        # Built-in presets + registered strategies
        ...

    def describe_pipeline(self) -> str:
        names = self._strategy.step_names()
        return f"{len(names)}-step pipeline: {' -> '.join(names)}"
```

### Step 3: Run Python tests, verify all pass

### Step 4-6: TypeScript

TS-specific:
- `static registerStrategy(name: string, strategy: ExecutionStrategy): void`
- `listStrategies(): StrategyInfo[]`
- `describePipeline(): string`
- `get currentStrategy(): ExecutionStrategy`

### Step 7-9: Rust

Rust-specific:
- `pub fn register_strategy(name: impl Into<String>, strategy: ExecutionStrategy)` (uses lazy_static or once_cell for global registry)
- `pub fn list_strategies(&self) -> Vec<StrategyInfo>`
- `pub fn describe_pipeline(&self) -> String`
- `pub fn current_strategy(&self) -> &ExecutionStrategy`

## Acceptance Criteria

- [ ] list_strategies() returns StrategyInfo for all built-in presets
- [ ] list_strategies() includes code-registered strategies
- [ ] StrategyInfo contains name, step_count, step_names, description
- [ ] describe_pipeline() returns AI-readable pipeline description string
- [ ] current_strategy returns the executor's active strategy
- [ ] register_strategy() is class-level/static, globally available
- [ ] Registered strategies resolvable by name in constructor and per-call override
- [ ] Unknown strategy name raises StrategyNotFoundError
- [ ] All implemented in all 3 SDKs

## Dependencies

- **Depends on:** preset-strategies, call-with-trace
- **Required by:** none (final task)

## Estimated Time

3 hours
