# Task: Built-in Steps

## Goal

Extract the current executor's 11 hardcoded pipeline steps into individual BuiltinStep implementations in all 3 SDKs. Each step wraps the existing executor logic, receiving its dependencies via constructor injection.

## 11 Built-in Steps to Extract

| # | Class Name | Wraps | Removable | Replaceable | Constructor Deps |
|---|------------|-------|-----------|-------------|------------------|
| 1 | BuiltinContextCreation | Context creation + global deadline | false | false | config |
| 2 | BuiltinSafetyCheck | Call chain guard (depth, repeat limits, cancel token) | true | true | config |
| 3 | BuiltinModuleLookup | registry.get() | false | false | registry |
| 4 | BuiltinACLCheck | acl.check() / async_check() | true | true | acl |
| 5 | BuiltinApprovalGate | Approval handler flow | true | true | handler |
| 6 | BuiltinInputValidation | Schema validation + redaction | true | true | — |
| 7 | BuiltinMiddlewareBefore | MiddlewareManager execute_before | true | false | middlewares |
| 8 | BuiltinExecute | Module execution with timeout | false | true | config |
| 9 | BuiltinOutputValidation | Output schema validation + redaction | true | true | — |
| 10 | BuiltinMiddlewareAfter | MiddlewareManager execute_after | true | false | middlewares |
| 11 | BuiltinReturnResult | Result finalization | false | false | — |

## Files Involved

### Python SDK
- **Create:** `apcore-python/src/apcore/builtin_steps.py`
- **Create:** `apcore-python/tests/test_builtin_steps.py`
- **Read:** `apcore-python/src/apcore/executor.py` (extract logic from)

### TypeScript SDK
- **Create:** `apcore-typescript/src/builtin-steps.ts`
- **Create:** `apcore-typescript/tests/test-builtin-steps.test.ts`
- **Read:** `apcore-typescript/src/executor.ts` (extract logic from)

### Rust SDK
- **Create:** `apcore-rust/src/builtin_steps.rs`
- **Modify:** `apcore-rust/src/lib.rs` (add `pub mod builtin_steps;`)
- **Create:** `apcore-rust/tests/test_builtin_steps.rs`
- **Read:** `apcore-rust/src/executor.rs` (extract logic from)

## Steps (TDD)

### Step 1: Write failing tests (Python)

For each of the 11 steps, write tests that verify:
- Correct step metadata (name, description, removable, replaceable)
- Happy path returns StepResult(action="continue")
- Error/denial returns StepResult(action="abort") with explanation
- Side effects on PipelineContext (e.g., BuiltinModuleLookup sets ctx.module)

Key test cases per step:
1. **BuiltinContextCreation** — creates Context if none provided, inherits existing context, sets global deadline
2. **BuiltinSafetyCheck** — passes when call depth OK, aborts on max depth exceeded, aborts on cancel token
3. **BuiltinModuleLookup** — sets ctx.module on found, aborts on module not found
4. **BuiltinACLCheck** — continues when acl=None, continues when allowed, aborts when denied, supports async_check
5. **BuiltinApprovalGate** — continues when handler=None, continues when approved, aborts when denied, handles timeout
6. **BuiltinInputValidation** — validates inputs against schema, redacts sensitive fields, sets ctx.validated_inputs
7. **BuiltinMiddlewareBefore** — executes middleware chain, handles empty middleware list, handles chain errors
8. **BuiltinExecute** — executes module, enforces timeout, sets ctx.output, handles streaming mode (ctx.stream)
9. **BuiltinOutputValidation** — validates output against schema, redacts sensitive fields, sets ctx.validated_output
10. **BuiltinMiddlewareAfter** — executes after-middleware chain, handles empty list
11. **BuiltinReturnResult** — finalizes result (no-op continue, output already on ctx)

### Step 2: Implement Python built-in steps

Extract logic from `executor.py` into `builtin_steps.py`. Each class extends BaseStep and wraps the corresponding section of the current executor's call_async method.

### Step 3: Run Python tests, verify all pass

### Step 4: Write failing tests (TypeScript)

Mirror Python tests. Key TS-specific differences:
- All steps return Promise<StepResult>
- BuiltinExecute uses Promise.race for timeout instead of threading

### Step 5: Implement TypeScript built-in steps

### Step 6: Run TypeScript tests, verify all pass

### Step 7: Write failing tests (Rust)

Mirror test cases. Key Rust-specific differences:
- Steps implement `#[async_trait] Step` trait
- BuiltinExecute uses `tokio::time::timeout`
- Error handling via Result<StepResult, ModuleError>

### Step 8: Implement Rust built-in steps

### Step 9: Run Rust tests, verify all pass

## Acceptance Criteria

- [ ] All 11 BuiltinStep classes implemented in all 3 SDKs
- [ ] Each step has correct name, description, removable, replaceable metadata
- [ ] BuiltinContextCreation creates/inherits context and sets global deadline
- [ ] BuiltinSafetyCheck validates call depth, repeat limits, cancel token
- [ ] BuiltinModuleLookup resolves module from registry, sets ctx.module
- [ ] BuiltinACLCheck supports both sync and async ACL check paths
- [ ] BuiltinApprovalGate handles approve/deny/timeout/pending flows
- [ ] BuiltinInputValidation validates and redacts inputs, sets ctx.validated_inputs
- [ ] BuiltinMiddlewareBefore/After execute middleware chain via MiddlewareManager
- [ ] BuiltinExecute invokes module with timeout, sets ctx.output, handles streaming
- [ ] BuiltinOutputValidation validates and redacts output, sets ctx.validated_output
- [ ] BuiltinReturnResult finalizes pipeline (continues, output already on ctx)
- [ ] Logic extracted is behaviorally identical to current executor code
- [ ] Each step independently testable with mock PipelineContext

## Dependencies

- **Depends on:** core-types (Step, BaseStep, StepResult, PipelineContext)
- **Required by:** executor-refactor, preset-strategies

## Estimated Time

10 hours
