# Task: Preset Strategies

## Goal

Implement factory functions for the 5 preset strategies: build_standard_strategy, build_internal_strategy, build_testing_strategy, build_validate_only_strategy, build_performance_strategy. Each is a factory function (not a constant) because built-in steps require runtime dependencies.

## Preset Definitions

| Strategy | Description | Modifications from STANDARD |
|----------|-------------|---------------------------|
| standard | Full 11-step pipeline | None (base) |
| internal | Skip ACL and approval for trusted internal calls | remove acl_check, remove approval_gate |
| testing | Minimal pipeline for tests | remove acl_check, remove approval_gate, remove safety_check |
| validate_only | Dry-run: validate inputs without executing | replace execute with BuiltinValidateOnly, remove middleware_before, remove middleware_after |
| performance | Skip middleware for performance-critical paths | remove middleware_before, remove middleware_after |

## Files Involved

### Python SDK
- **Create:** `apcore-python/src/apcore/strategies.py`
- **Create:** `apcore-python/tests/test_strategies.py`

### TypeScript SDK
- **Create:** `apcore-typescript/src/strategies.ts`
- **Create:** `apcore-typescript/tests/test-strategies.test.ts`

### Rust SDK
- **Create:** `apcore-rust/src/strategies.rs`
- **Modify:** `apcore-rust/src/lib.rs` (add `pub mod strategies;`)
- **Create:** `apcore-rust/tests/test_strategies.rs`

## Steps (TDD)

### Step 1: Write failing tests (Python)

Test cases:
- `test_build_standard_strategy_has_11_steps` — returns ExecutionStrategy with 11 steps in correct order
- `test_build_standard_strategy_step_names` — step_names() matches ["context_creation", "safety_check", "module_lookup", "acl_check", "approval_gate", "input_validation", "middleware_before", "execute", "output_validation", "middleware_after", "return_result"]
- `test_build_standard_strategy_name` — strategy.name == "standard"
- `test_build_internal_strategy_has_9_steps` — removes acl_check and approval_gate
- `test_build_internal_strategy_name` — strategy.name == "internal"
- `test_build_testing_strategy_has_8_steps` — removes acl_check, approval_gate, safety_check
- `test_build_testing_strategy_name` — strategy.name == "testing"
- `test_build_validate_only_strategy_replaces_execute` — execute step replaced with BuiltinValidateOnly
- `test_build_validate_only_strategy_no_middleware` — middleware_before and middleware_after removed
- `test_build_validate_only_strategy_name` — strategy.name == "validate_only"
- `test_build_performance_strategy_no_middleware` — middleware_before and middleware_after removed
- `test_build_performance_strategy_name` — strategy.name == "performance"
- `test_build_performance_strategy_has_9_steps` — 11 - 2 middleware steps = 9
- `test_all_presets_accept_same_kwargs` — all factory functions accept registry, config, acl, approval_handler, middlewares

### Step 2: Implement Python strategies

Also implement `BuiltinValidateOnly` step (used by validate_only strategy): a replaceable step with name="execute" that validates inputs without executing the module, setting ctx.output to a validation summary dict.

### Step 3: Run Python tests, verify all pass

### Step 4-6: TypeScript (mirror Python)

### Step 7-9: Rust (mirror Python)

## Acceptance Criteria

- [ ] build_standard_strategy() returns 11-step pipeline with correct order
- [ ] build_internal_strategy() removes acl_check and approval_gate (9 steps)
- [ ] build_testing_strategy() removes acl_check, approval_gate, safety_check (8 steps)
- [ ] build_validate_only_strategy() replaces execute, removes middleware (8 steps)
- [ ] build_performance_strategy() removes middleware_before and middleware_after (9 steps)
- [ ] All factory functions accept registry, config, acl, approval_handler, middlewares kwargs
- [ ] BuiltinValidateOnly step implemented for validate_only strategy
- [ ] Strategy names correct ("standard", "internal", "testing", "validate_only", "performance")
- [ ] All implemented in all 3 SDKs

## Dependencies

- **Depends on:** executor-refactor (which depends on builtin-steps + pipeline-engine)
- **Required by:** introspection

## Estimated Time

4 hours
