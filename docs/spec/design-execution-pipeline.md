# Design: Execution Pipeline Strategy & AI Decision Support

> **Status:** Implemented (v0.17.0 – v0.18.0) | Authors: apcore maintainers | Updated: 2026-04-14

This document specifies apcore's configurable execution pipeline: a flat ordered list of steps with declarative metadata, AI decision support, and YAML-driven configuration. All design decisions apply equally to the Python, TypeScript, and Rust SDKs.

---

## 1. Problem Statement

### 1.1 Original State (pre-v0.17.0)

The Executor implemented a hardcoded 11-step pipeline with two problems:

**Step order error:** Input validation ran before middleware before-chain, meaning middleware input transformations were never validated:
```
6. Input Validation    ← validated raw inputs
7. Middleware Before   ← transformed inputs not re-validated
```

**Structural problems:**

| Problem | Impact |
|---------|--------|
| Cannot skip steps (ACL, validation) for internal calls | Performance overhead; workaround via `acl=None` |
| Cannot add steps (rate limiting, cost budgeting) | Features crammed into middleware with unclear semantics |
| Cannot replace step implementations (custom validator, OPA) | Locked to built-in implementations |
| validate() hardcoded to Steps 1–7 | User-added pipeline steps never ran during preflight |
| Steps had no declarative metadata | No per-step module filtering, error tolerance, or timeout |
| `safety_check` naming misleads | Users confused call-chain safety with transport-level rate limiting |
| Pipeline invisible to AI agents | AI cannot reason about safety checks or learn from traces |

### 1.2 Design Goals

1. **Corrected step order** — middleware transforms first, then validation checks the transformed result
2. **Configurable pipeline** — steps can be added, removed, replaced, reordered
3. **Declarative step metadata** — `pure`, `match_modules`, `ignore_errors`, `timeout_ms`
4. **Safety guarantees preserved** — core steps (context, lookup, execute, return) cannot be removed
5. **validate() via dry_run** — pure steps automatically participate; impure steps automatically skip
6. **AI perceivable** — pipeline structure and execution trace visible to AI
7. **AI participatory** — AI can implement any step (approval, risk assessment, semantic validation)
8. **Backward compatible** — default pipeline = corrected 11 steps; no migration for existing code
9. **Cross-language consistent** — identical API across Python, TypeScript, Rust
10. **YAML declarable** — pipeline strategy configurable in apcore.yaml

---

## 2. Core Concepts

### 2.1 Step

A single unit of work in the execution pipeline.

```
Step {
  name:           string         // unique identifier (e.g., "acl_check")
  description:    string         // AI-readable purpose description
  removable:      bool           // false = safety-critical, cannot be removed
  replaceable:    bool           // true = can swap implementation

  // Declarative metadata (v0.17.0)
  match_modules:  tuple[str] | None  // glob patterns; None = all modules
  ignore_errors:  bool               // true = failure logs warning and continues
  pure:           bool               // true = no side effects; safe in dry_run
  timeout_ms:     int                // per-step timeout; 0 = no limit

  execute(ctx: PipelineContext) -> StepResult
}
```

> **Configuration injection:** The Step protocol only defines `execute()`. Steps receive
> their configuration via constructor (e.g., `BuiltinACLCheck(acl=acl)`). The protocol
> does NOT constrain constructors — different steps need different configuration shapes.

> **Property vs attribute (Python):** `name`, `description`, `removable`, `replaceable`
> are declared as `@property` in the Protocol but implementations SHOULD use plain
> instance attributes (set in `__init__`), not class attributes.

### 2.2 StepResult

The outcome of a step, with AI-readable explanation.

```
StepResult {
  action:       "continue" | "skip_to" | "abort"
  skip_to:      string | nil             // target step name (for skip_to action)
  explanation:  string | nil             // AI/human-readable reason
  confidence:   float | nil              // AI decision confidence (0.0–1.0)
  alternatives: list[string] | nil       // suggested alternatives on abort
}
```

> **Data flow:** Steps do NOT pass data through `StepResult`. `StepResult` only controls
> flow (continue/skip/abort) and provides AI-readable metadata.
>
> **Two-tier data model:**
>
> | Tier | Storage | Written by | Read via | Example |
> |------|---------|-----------|----------|---------|
> | **Tier 1** | `PipelineContext` fields | Built-in steps | Direct field access | `ctx.module`, `ctx.output` |
> | **Tier 2** | `context.data` | Middleware, custom steps | `ContextKey[T]` (type-safe) | `TRACING_SPANS.set(ctx.context, [...])` |
>
> Built-in steps write pipeline-essential data to Tier 1. Middleware and custom steps
> store extension state in Tier 2. PipelineContext fields are NOT duplicated into
> `context.data`.

### 2.3 ExecutionStrategy

An ordered list of steps defining a complete pipeline.

```
ExecutionStrategy {
  name:  string
  steps: list[Step]

  insert_after(anchor: string, step: Step)
  insert_before(anchor: string, step: Step)
  remove(step_name: string)               // raises if step.removable == false
  replace(step_name: string, new: Step)   // raises if step.replaceable == false
}
```

**Invariant:** Step names MUST be unique within a strategy. `insert_after`/`insert_before`
MUST raise `StepNameDuplicateError` if a step with the same name already exists.

### 2.4 PipelineContext

The shared state flowing through all steps.

```
PipelineContext {
  // Input (set before pipeline starts)
  module_id:    string
  inputs:       map               // original inputs, may be mutated by middleware_before
  context:      Context           // apcore execution context

  // Resolved during pipeline (set by specific steps, nil until that step runs)
  module:       Module | nil      // set by module_lookup step
  validated_inputs: map | nil     // set by input_validation step
  output:       map | nil         // set by execute step
  validated_output: map | nil     // set by output_validation step

  // Streaming (set by PipelineEngine.run_stream before pipeline starts)
  stream:          bool = false
  output_stream:   AsyncGenerator | nil      // set by BuiltinExecute when stream=true

  // Execution control (v0.17.0)
  dry_run:          bool = false             // true during validate(); skips pure=false steps
  version_hint:     string | nil             // passed to module_lookup for version negotiation
  executed_middlewares: list = []            // tracks ran middleware for on_error recovery

  // Metadata
  strategy:     ExecutionStrategy
  trace:        PipelineTrace
}
```

> **Field availability:** Steps MUST check for nil before reading fields set by later steps.
> Standard guarantee: `module` after step 3, `validated_inputs` after step 7, `output`
> after step 8, `validated_output` after step 9.

### 2.5 PipelineTrace

Complete execution record, AI-readable. **Process-local** — returned to the caller of
`call_with_trace()`. NOT stored in `context.data` and NOT designed for cross-process
transmission.

```
PipelineTrace {
  module_id:         string
  strategy_name:     string
  steps:             list[StepTrace]
  total_duration_ms: float
  success:           bool
}

StepTrace {
  name:            string
  duration_ms:     float
  result:          StepResult       // includes explanation, confidence
  skipped:         bool
  decision_point:  bool             // true if step set result.confidence
  skip_reason:     string | nil     // "no_match" | "dry_run" | "error_ignored"
}
```

### 2.6 StrategyInfo

Returned by `list_strategies()` for AI introspection.

```
StrategyInfo {
  name:          string
  step_count:    int
  step_names:    list[string]
  description:   string         // auto-generated from step descriptions
}
```

---

## 3. Step Protocol

### 3.1 Python

```python
from typing import Protocol, runtime_checkable
from abc import ABC, abstractmethod

@runtime_checkable
class Step(Protocol):
    @property
    def name(self) -> str: ...
    @property
    def description(self) -> str: ...
    @property
    def removable(self) -> bool: ...
    @property
    def replaceable(self) -> bool: ...

    async def execute(self, ctx: PipelineContext) -> StepResult: ...

class BaseStep(ABC):
    def __init__(
        self,
        name: str,
        description: str = "",
        *,
        removable: bool = True,
        replaceable: bool = True,
        # Declarative metadata
        match_modules: tuple[str, ...] | None = None,
        ignore_errors: bool = False,
        pure: bool = False,
        timeout_ms: int = 0,
    ) -> None:
        self.name = name
        self.description = description
        self.removable = removable
        self.replaceable = replaceable
        self.match_modules = match_modules
        self.ignore_errors = ignore_errors
        self.pure = pure
        self.timeout_ms = timeout_ms

    @abstractmethod
    async def execute(self, ctx: PipelineContext) -> StepResult: ...
```

> **runtime_checkable note:** Python's `@runtime_checkable` Protocol cannot verify
> `execute` is a coroutine function. `PipelineEngine` MUST verify at registration:
> `assert inspect.iscoroutinefunction(step.execute)`.

> **Backward compat:** `PipelineEngine` reads the 4 new fields via `getattr()` with
> defaults, so third-party Step implementations that predate v0.17.0 continue to work.

### 3.2 TypeScript

```typescript
export interface Step {
  readonly name: string;
  readonly description: string;
  readonly removable: boolean;
  readonly replaceable: boolean;

  // Optional declarative metadata (default to no-op if absent)
  readonly matchModules?: readonly string[] | null;
  readonly ignoreErrors?: boolean;
  readonly pure?: boolean;
  readonly timeoutMs?: number;

  execute(ctx: PipelineContext): Promise<StepResult>;
}
```

### 3.3 Rust

```rust
#[async_trait]
pub trait Step: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn removable(&self) -> bool;
    fn replaceable(&self) -> bool;

    // Default implementations — override to customize
    fn match_modules(&self) -> Option<&[&str]> { None }
    fn ignore_errors(&self) -> bool { false }
    fn pure_step(&self) -> bool { false }
    fn timeout_ms(&self) -> u64 { 0 }

    async fn execute(&self, ctx: &mut PipelineContext) -> Result<StepResult, ModuleError>;
}
```

---

## 4. Built-in Steps

### 4.0 Core vs Optional Steps

The standard pipeline has 11 steps, but only **4 are mandatory** (non-removable):

```
 ┌─────────────────────────────────────────────────────────────────────┐
 │                    STANDARD PIPELINE (11 steps)                     │
 │                                                                     │
 │  ██ context_creation ──→ ░ call_chain_guard ──→ ██ module_lookup    │
 │                                                       │             │
 │  ░ acl_check ←────────────────────────────────────────┘             │
 │       │                                                             │
 │  ░ approval_gate ──→ ░ middleware_before ──→ ░ input_validation     │
 │                                                       │             │
 │  ██ execute ←─────────────────────────────────────────┘             │
 │       │                                                             │
 │  ░ output_validation ──→ ░ middleware_after ──→ ██ return_result    │
 │                                                                     │
 │  ██ = CORE (non-removable)    ░ = OPTIONAL (removable)              │
 └─────────────────────────────────────────────────────────────────────┘
```

**Core steps** (always present in every strategy):

| Step | Why it cannot be removed |
|------|-------------------------|
| `context_creation` | Every call needs an execution context with identity and trace ID |
| `module_lookup` | The module must be resolved from the registry before anything can use it |
| `execute` | Without execution, there is no output (replaceable for dry-run, not removable) |
| `return_result` | Pipeline must have a terminal step that finalizes the output |

The `minimal` preset strips all optional steps, leaving only the 4 core steps — suitable for pre-validated internal hot paths.

### 4.1 Step Inventory

| # | Name | pure | removable | replaceable | Description |
|---|------|------|-----------|-------------|-------------|
| 1 | `context_creation` | true | false | false | Create or inherit execution context, set global deadline |
| 2 | `call_chain_guard` | true | true | true | Validate call depth, module repeat limits, cancel token |
| 3 | `module_lookup` | true | false | false | Resolve module from registry by ID, apply version_hint |
| 4 | `acl_check` | true | true | true | Enforce access control rules |
| 5 | `approval_gate` | false | true | true | Handle human or AI approval flow (may call external systems) |
| 6 | `middleware_before` | false | true | false | Execute registered before-middleware chain |
| 7 | `input_validation` | true | true | true | Validate transformed inputs against schema, redact sensitive fields |
| 8 | `execute` | false | false | true | Invoke the module with timeout enforcement |
| 9 | `output_validation` | true | true | true | Validate outputs against schema, redact sensitive fields |
| 10 | `middleware_after` | false | true | false | Execute registered after-middleware chain |
| 11 | `return_result` | true | false | false | Finalize and return output |

!!! warning "Steps 6 and 7: Corrected Order (v0.17.0)"
    Middleware before-chain (step 6) now runs **before** input validation (step 7). This is
    the Kubernetes Mutating → Validating pattern: transformations happen first, then the
    transformed result is validated. The previous order (validate → middleware) silently
    discarded middleware modifications in the pipeline abstraction and never re-validated
    transformed inputs in production code.

!!! warning "Replacing the Execute Step"
    `strategy.replace("execute", custom_step)` bypasses built-in timeout enforcement,
    cancel token checks, global deadline clamping, and streaming detection. Your
    replacement MUST re-implement these if you rely on them. Treat this as an advanced
    escape hatch, not a casual customization point.

**Middleware steps (6, 10) are removable but not replaceable:** They can be removed
(e.g., `performance` preset), but their implementation wraps the existing middleware chain
contract. Replacing them would break the `Middleware` protocol ecosystem packages depend on.
Add a custom step before/after them instead.

### 4.2 Builtin Step Declarations

```python
# context_creation
pure=True,  removable=False, replaceable=False

# call_chain_guard  (renamed from safety_check in v0.17.0)
pure=True,  removable=True,  replaceable=True

# module_lookup
pure=True,  removable=False, replaceable=False

# acl_check
pure=True,  removable=True,  replaceable=True

# approval_gate
pure=False, removable=True,  replaceable=True
# pure=False: may call external approval system

# middleware_before
pure=False, removable=True,  replaceable=False
# pure=False: user middleware may have side effects
# replaceable=False: protects the onion model

# input_validation
pure=True,  removable=True,  replaceable=True

# execute
pure=False, removable=False, replaceable=True
# replaceable=True: enables sandbox/remote executors

# output_validation
pure=True,  removable=True,  replaceable=True

# middleware_after
pure=False, removable=True,  replaceable=False

# return_result
pure=True,  removable=False, replaceable=False
```

### 4.3 Data Flow Through Steps

```
ctx.inputs (raw, from caller)
    │
    ├─ Steps 1–5: READ-ONLY (don't modify inputs)
    │   context_creation:  writes ctx.context
    │   call_chain_guard:  reads ctx.context.call_chain
    │   module_lookup:     writes ctx.module, applies ctx.version_hint
    │   acl_check:         reads ctx.context.caller_id
    │   approval_gate:     reads/writes ctx.inputs (_approval_token removal)
    │
    ├─ Step 6: middleware_before
    │   WRITES ctx.inputs (middleware chain modifies in place)
    │   WRITES ctx.executed_middlewares (for on_error recovery chain)
    │
    ├─ Step 7: input_validation
    │   READS ctx.inputs (now includes middleware modifications)
    │   WRITES ctx.validated_inputs = ctx.inputs
    │   WRITES ctx.context.redacted_inputs
    │
    ├─ Step 8: execute
    │   READS ctx.validated_inputs (preferred) or ctx.inputs
    │   WRITES ctx.output
    │
    ├─ Step 9: output_validation
    │   READS ctx.output
    │   WRITES ctx.validated_output = ctx.output
    │   WRITES ctx.context[REDACTED_OUTPUT]
    │
    ├─ Step 10: middleware_after
    │   READS ctx.output
    │   WRITES ctx.output (middleware chain modifies)
    │
    └─ Step 11: return_result
        READS ctx.validated_output or ctx.output
        (PipelineEngine extracts final result)
```

### 4.4 When to Use Middleware vs Custom Steps

| Criterion | Middleware | Custom Step |
|-----------|-----------|-------------|
| **Use case** | Cross-cutting concerns (logging, retry, metrics, tracing) | Pipeline logic at a specific position (rate limiting, cost budgeting, custom auth) |
| **Execution position** | Fixed at steps 6 and 10 | Any position via `insert_before` / `insert_after` |
| **Lifecycle** | Paired: `before()` + `after()` + `on_error()` | Independent: single `execute()` method |
| **Error recovery** | `on_error()` can return recovery output | Return `StepResult(action="abort")` |
| **Registration** | `executor.use(middleware)` — dynamic, per-executor | `strategy.insert_after(anchor, step)` — per-strategy |
| **Observability** | Not individually traced | Each step appears in `PipelineTrace` |

**Rule of thumb:** If your logic needs to wrap execution (see both inputs and outputs as a pair), use Middleware. If your logic is a gate or transform at a specific pipeline position, use a Custom Step.

---

## 5. Execution Strategy

### 5.1 Standard Strategy

The `build_standard_strategy` factory function builds the corrected 11-step pipeline.
It is a **factory function**, not a pre-instantiated constant, because built-in steps
require runtime dependencies not available at import time:

```python
def build_standard_strategy(
    *,
    registry: Registry,
    config: Config | None = None,
    acl: ACL | None = None,
    approval_handler: ApprovalHandler | None = None,
    middlewares: list[Middleware] | None = None,
) -> ExecutionStrategy:
    return ExecutionStrategy(
        name="standard",
        steps=[
            BuiltinContextCreation(config=config),
            BuiltinCallChainGuard(config=config),
            BuiltinModuleLookup(registry=registry),
            BuiltinACLCheck(acl=acl),
            BuiltinApprovalGate(handler=approval_handler),
            BuiltinMiddlewareBefore(middlewares=middlewares or []),  # Step 6
            BuiltinInputValidation(),                                # Step 7 (after middleware)
            BuiltinExecute(config=config),
            BuiltinOutputValidation(),
            BuiltinMiddlewareAfter(middlewares=middlewares or []),
            BuiltinReturnResult(),
        ],
    )
```

### 5.2 Preset Strategies

Presets are also factory functions (not constants):

```python
def build_internal_strategy(**kwargs) -> ExecutionStrategy:
    """Standard minus ACL and approval — for trusted service-to-service calls."""
    s = build_standard_strategy(**kwargs); s.remove("acl_check"); s.remove("approval_gate")
    s._name = "internal"; return s

def build_testing_strategy(**kwargs) -> ExecutionStrategy:
    """No safety, ACL, or approval — for unit/integration tests."""
    s = build_standard_strategy(**kwargs)
    s.remove("acl_check"); s.remove("approval_gate"); s.remove("call_chain_guard")
    s._name = "testing"; return s

def build_performance_strategy(**kwargs) -> ExecutionStrategy:
    """Skip middleware — for latency-sensitive paths."""
    s = build_standard_strategy(**kwargs)
    s.remove("middleware_before"); s.remove("middleware_after")
    s._name = "performance"; return s

def build_minimal_strategy(**kwargs) -> ExecutionStrategy:
    """Core steps only — for pre-validated internal hot paths."""
    s = build_standard_strategy(**kwargs)
    for name in ["call_chain_guard", "acl_check", "approval_gate",
                 "middleware_before", "input_validation", "output_validation", "middleware_after"]:
        s.remove(name)
    s._name = "minimal"; return s
```

| Strategy | Steps | Removed from standard | Use case |
|----------|-------|----------------------|----------|
| `standard` | 11 | — | Default, full safety |
| `internal` | 9 | acl_check, approval_gate | Trusted service-to-service |
| `testing` | 8 | acl_check, approval_gate, call_chain_guard | Unit/integration tests |
| `performance` | 9 | middleware_before, middleware_after | Latency-sensitive paths |
| `minimal` | **4** | All optional steps | Pre-validated internal hot paths |

!!! note "dry_run replaces validate_only preset"
    The `validate()` method uses `dry_run=True`, which automatically skips `pure=False`
    steps in any strategy. A dedicated `validate_only` preset is unnecessary.

### 5.3 Custom Strategy

```python
my_strategy = ExecutionStrategy.from_standard(name="my_pipeline")
my_strategy.insert_after("acl_check", RateLimiterStep(max_rps=100))
my_strategy.insert_after("approval_gate", CostBudgetStep(max_cost=1.0))
# 13-step result:
# context_creation → call_chain_guard → module_lookup → acl_check →
# rate_limiter → approval_gate → cost_budget →
# middleware_before → input_validation → execute →
# output_validation → middleware_after → return_result
```

### 5.4 Strategy Modification API

```python
class ExecutionStrategy:
    def __init__(self, name: str, steps: list[Step]) -> None: ...

    @classmethod
    def from_standard(
        cls, name: str,
        remove: list[str] | None = None,
        replace: dict[str, Step] | None = None,
    ) -> "ExecutionStrategy":
        """Operation order: replace FIRST, then remove. Both respect step constraints."""
        ...

    def insert_after(self, anchor: str, step: Step) -> None: ...
    def insert_before(self, anchor: str, step: Step) -> None: ...
    def remove(self, step_name: str) -> None: ...      # raises StepNotRemovableError
    def replace(self, step_name: str, new: Step) -> None: ...  # raises StepNotReplaceableError
    def step_names(self) -> list[str]: ...             # for AI introspection
```

### 5.5 Accessing Context Data from Steps

```python
from apcore.context_keys import TRACING_SPANS, ContextKey

class MyCustomStep(BaseStep):
    async def execute(self, ctx: PipelineContext) -> StepResult:
        # Read Tier 2 data (middleware state) via ContextKey
        spans = TRACING_SPANS.get(ctx.context)

        # Write Tier 2 data for downstream middleware/steps
        MY_KEY = ContextKey[int]("myapp.processed_count")
        MY_KEY.set(ctx.context, 42)

        # Read Tier 1 data (pipeline state) via direct field access
        module = ctx.module  # set by module_lookup step

        return StepResult(action="continue")
```

---

## 6. Executor API

### 6.1 Constructor

```python
class Executor:
    def __init__(
        self,
        registry: Registry,
        *,
        strategy: ExecutionStrategy | str | None = None,
        middlewares: list[Middleware] | None = None,   # backward compat
        acl: ACL | None = None,                       # backward compat
        config: Config | None = None,
        approval_handler: ApprovalHandler | None = None,  # backward compat
    ) -> None:
        if strategy is None:
            self._strategy = build_standard_strategy(
                registry=registry, config=config, acl=acl,
                approval_handler=approval_handler, middlewares=middlewares,
            )
        elif isinstance(strategy, str):
            # 4-level lookup: built-in presets → code-registered → YAML (Python) → error
            self._strategy = self._resolve_strategy_name(strategy)
        else:
            self._strategy = strategy
```

When `strategy` is provided, `middlewares`/`acl`/`approval_handler` are ignored — steps in
the strategy contain their own dependencies.

### 6.2 Call Methods

```python
# Existing API unchanged
result = executor.call(module_id, inputs, context)
result = await executor.call_async(module_id, inputs, context)

# Per-call strategy override
result = executor.call(module_id, inputs, context, strategy="internal")
result = await executor.call_async(module_id, inputs, context, strategy=my_strategy)

# Call with trace
result, trace = executor.call_with_trace(module_id, inputs, context)
result, trace = await executor.call_async_with_trace(module_id, inputs, context, strategy="internal")
```

### 6.3 validate() — via dry_run

`validate()` replaces the previously hardcoded 7-step preflight check. It uses
`dry_run=True` so steps with `pure=False` (approval_gate, middleware, execute) are
automatically skipped. User-added steps with `pure=True` automatically participate:

```python
async def validate(self, module_id, inputs=None, context=None):
    ctx = PipelineContext(
        module_id=module_id,
        inputs=inputs or {},
        context=context,
        dry_run=True,
    )
    try:
        _, trace = await PipelineEngine().run(self._strategy, ctx)
    except PipelineAbortError as e:
        return PreflightResult(valid=False, checks=_trace_to_checks(e.pipeline_trace))
    return PreflightResult(valid=True, checks=_trace_to_checks(trace))
```

> **trace_to_checks mapping:** PreflightResult uses check names (`"module_id"`, `"call_chain"`,
> `"acl"`, `"schema"`) that differ from pipeline step names. `_trace_to_checks()` maintains
> a mapping table. Module ID format validation and `module.preflight()` have no separate
> pipeline steps — they run inside `module_lookup` and `input_validation` respectively.

### 6.4 Introspection

```python
strategies = executor.list_strategies()
# → [StrategyInfo(name="standard", step_count=11, step_names=[...], description="..."), ...]

strategy = executor.current_strategy
description = executor.describe_pipeline()
# → "11-step pipeline: context_creation → call_chain_guard → ..."

Executor.register_strategy("my_pipeline", my_strategy)  # class-level, global
```

### 6.5 Cross-Language Signatures

**TypeScript:**
```typescript
class Executor {
  constructor(options: {
    registry: Registry;
    strategy?: ExecutionStrategy | string | null;
    middlewares?: Middleware[] | null;
    acl?: ACL | null;
    config?: Config | null;
    approvalHandler?: ApprovalHandler | null;
  });

  async call(
    moduleId: string, inputs: Record<string, unknown>,
    context?: Context | null,
    options?: { strategy?: ExecutionStrategy | string },
  ): Promise<Record<string, unknown>>;

  async callWithTrace(
    moduleId: string, inputs: Record<string, unknown>,
    context?: Context | null,
    options?: { strategy?: ExecutionStrategy | string },
  ): Promise<[Record<string, unknown>, PipelineTrace]>;

  listStrategies(): StrategyInfo[];
  describePipeline(): string;
  static registerStrategy(name: string, strategy: ExecutionStrategy): void;
}
```

**Rust:**
```rust
impl Executor {
    pub fn new(registry: Registry, config: Config) -> Self;
    pub fn with_strategy(registry: Registry, config: Config, strategy: ExecutionStrategy) -> Self;

    pub async fn call(
        &self, module_id: &str, inputs: Value,
        ctx: Option<&Context<Value>>,
    ) -> Result<Value, ModuleError>;

    pub async fn call_with_strategy(
        &self, module_id: &str, inputs: Value,
        ctx: Option<&Context<Value>>,
        strategy: &ExecutionStrategy,
    ) -> Result<Value, ModuleError>;

    pub async fn call_with_trace(
        &self, module_id: &str, inputs: Value,
        ctx: Option<&Context<Value>>,
        strategy: Option<&ExecutionStrategy>,
    ) -> Result<(Value, PipelineTrace), ModuleError>;

    pub fn list_strategies(&self) -> Vec<StrategyInfo>;
    pub fn register_strategy(name: impl Into<String>, strategy: ExecutionStrategy);
}
```

---

## 7. Pipeline Execution Engine

### 7.1 Enhanced Engine Loop

The engine reads declarative metadata from each step and applies four filtering/routing
decisions before executing:

```python
async def run(self, strategy: ExecutionStrategy, ctx: PipelineContext) -> tuple[Any, PipelineTrace]:
    trace = PipelineTrace(module_id=ctx.module_id, strategy_name=strategy.name)
    start = time.monotonic()
    steps = strategy.steps
    i = 0

    while i < len(steps):
        step = steps[i]

        # Read declarations (getattr for backward compat with pre-v0.17.0 steps)
        match_modules = getattr(step, "match_modules", None)
        ignore_errors = getattr(step, "ignore_errors", False)
        pure          = getattr(step, "pure", False)
        timeout_ms    = getattr(step, "timeout_ms", 0)

        # ① match_modules filter: skip if module_id doesn't match any pattern
        if match_modules is not None and not _any_match(match_modules, ctx.module_id):
            trace.steps.append(StepTrace(
                name=step.name, duration_ms=0,
                result=StepResult(action="continue"),
                skipped=True, skip_reason="no_match",
            ))
            i += 1; continue

        # ② dry_run filter: skip steps with side effects during validate()
        if ctx.dry_run and not pure:
            trace.steps.append(StepTrace(
                name=step.name, duration_ms=0,
                result=StepResult(action="continue"),
                skipped=True, skip_reason="dry_run",
            ))
            i += 1; continue

        # ③ Execute with optional per-step timeout
        step_start = time.monotonic()
        try:
            if timeout_ms > 0:
                result = await asyncio.wait_for(step.execute(ctx), timeout=timeout_ms / 1000)
            else:
                result = await step.execute(ctx)
        except Exception as exc:
            duration = (time.monotonic() - step_start) * 1000
            # ④ ignore_errors: log warning and continue instead of aborting
            if ignore_errors:
                _logger.warning("Step '%s' failed (ignored): %s", step.name, exc)
                trace.steps.append(StepTrace(
                    name=step.name, duration_ms=duration,
                    result=StepResult(action="continue", explanation=str(exc)),
                    skip_reason="error_ignored",
                ))
                i += 1; continue
            # Not ignored: record and raise
            trace.steps.append(StepTrace(
                name=step.name, duration_ms=duration,
                result=StepResult(action="abort", explanation=str(exc)),
            ))
            trace.total_duration_ms = (time.monotonic() - start) * 1000
            raise

        duration = (time.monotonic() - step_start) * 1000
        trace.steps.append(StepTrace(
            name=step.name, duration_ms=duration,
            result=result, decision_point=result.confidence is not None,
        ))

        if result.action == "abort":
            trace.total_duration_ms = (time.monotonic() - start) * 1000
            raise PipelineAbortError(step=step.name, explanation=result.explanation,
                                     alternatives=result.alternatives, trace=trace)
        elif result.action == "skip_to":
            target = result.skip_to
            target_idx = None
            for j in range(i + 1, len(steps)):
                if steps[j].name == target:
                    target_idx = j; break
                trace.steps.append(StepTrace(
                    name=steps[j].name, duration_ms=0,
                    result=StepResult(action="continue"), skipped=True,
                ))
            if target_idx is None:
                raise StepNotFoundError(target)
            i = target_idx; continue

        i += 1

    trace.success = True
    trace.total_duration_ms = (time.monotonic() - start) * 1000
    return (ctx.validated_output if ctx.validated_output is not None else ctx.output), trace
```

**Pattern matching** (`_any_match`) uses Algorithm A09 — the same glob matching as ACL.
Patterns: `"api.*"` matches `api.users.list`, `"*.create"` matches `data.create`, `"*"` matches all.

### 7.2 Error Types

```python
class PipelineAbortError(ModuleError):
    step: str; explanation: str | None; alternatives: list[str] | None; trace: PipelineTrace

class StepNotFoundError(ModuleError): ...        # skip_to targets non-existent step
class StepNotRemovableError(ModuleError): ...    # remove() on non-removable step
class StepNotReplaceableError(ModuleError): ...  # replace() on non-replaceable step
class StrategyNotFoundError(ModuleError): ...    # unknown strategy name
class StepNameDuplicateError(ModuleError): ...   # insert with duplicate name
```

### 7.3 Streaming Support

Steps 1–7 are identical for streaming and non-streaming calls. The difference is in step 8:

When `PipelineEngine.run_stream()` is used, `ctx.stream = True` is set before the pipeline starts.
`BuiltinExecute` checks `ctx.stream`: if True, calls `module.stream()` and stores the async
generator in `ctx.output_stream`; if False, calls `module.execute()` and stores in `ctx.output`.

Steps 9–11 for streaming:
- Step 9 (`output_validation`): validates each chunk OR accumulated result after stream completes
- Step 10 (`middleware_after`): runs once after stream completes (on accumulated output), not per-chunk
- Step 11 (`return_result`): returns the async generator, not a dict

```python
class PipelineEngine:
    async def run(self, strategy, ctx) -> tuple[Any, PipelineTrace]: ...

    async def run_stream(self, strategy, ctx) -> tuple[AsyncGenerator, PipelineTrace]:
        """Steps 1–7 run identically. Step 8 yields chunks. Steps 9–11 run on accumulated output.
        
        Returns (async_generator, trace). trace is shared by reference:
        - Steps 1–7 traces populated BEFORE this method returns.
        - Step 8+ traces appended AS the generator is consumed.
        - trace.success set when generator exhausts.
        - Caller MUST NOT read trace.success until generator is fully consumed.
        """
```

---

## 8. AI Decision Support

### 8.1 AI as Step Implementor

Any step can be implemented by AI. The Step protocol does not distinguish AI from non-AI implementations.

```python
class AIRiskAssessment(BaseStep):
    def __init__(self, model: str = "gpt-4", threshold: float = 0.7) -> None:
        super().__init__(
            name="ai_risk_assessment",
            description="AI model evaluates execution risk before proceeding",
        )
        self._model = model; self._threshold = threshold

    async def execute(self, ctx: PipelineContext) -> StepResult:
        risk = await self._evaluate_risk(ctx)
        if risk.score > self._threshold:
            return StepResult(
                action="abort",
                explanation=f"High risk ({risk.score:.0%}): {risk.reason}",
                confidence=risk.score,
                alternatives=risk.suggested_modules,
            )
        return StepResult(action="continue", explanation=f"Risk acceptable ({risk.score:.0%})",
                          confidence=1.0 - risk.score)

class AIApprovalStep(BaseStep):
    def __init__(self, auto_approve_below: float = 0.3, require_human_above: float = 0.8) -> None:
        super().__init__(name="ai_approval",
                         description="AI evaluates whether to approve module execution")
        self._auto_approve = auto_approve_below; self._human_threshold = require_human_above

    async def execute(self, ctx: PipelineContext) -> StepResult:
        risk = await self._assess(ctx)
        if risk < self._auto_approve:
            return StepResult(action="continue", explanation="Auto-approved (low risk)",
                              confidence=1.0 - risk)
        if risk > self._human_threshold:
            return StepResult(action="abort", explanation="Requires human approval (high risk)",
                              confidence=risk)
        decision = await self._ai_decide(ctx, risk)
        return StepResult(action="continue" if decision.approved else "abort",
                          explanation=decision.reason, confidence=decision.confidence)
```

### 8.2 AI Strategy Selection

```python
strategies = executor.list_strategies()
# AI chooses the appropriate strategy based on call context
result = await executor.call_async("email.send", inputs, context, strategy="internal")
```

### 8.3 Pipeline Trace for AI Learning

```python
result, trace = await executor.call_async_with_trace("email.send", inputs, context)
# trace.steps contains per-step duration, result, explanation, confidence
# decision_point=True identifies steps where AI set a confidence score
```

### 8.4 AI Perceivability Guarantees

Each module invocation exposes a fully visible pipeline:

```json
{
  "module_id": "email.send",
  "strategy": "ai_governed",
  "pipeline": [
    {"step": "context_creation",      "removable": false, "pure": true},
    {"step": "call_chain_guard",      "removable": true,  "pure": true},
    {"step": "module_lookup",         "removable": false, "pure": true},
    {"step": "acl_check",             "removable": true,  "pure": true},
    {"step": "ai_risk_assessment",    "removable": true,  "pure": false},
    {"step": "approval_gate",         "removable": true,  "pure": false},
    {"step": "middleware_before",     "removable": true,  "pure": false},
    {"step": "input_validation",      "removable": true,  "pure": true},
    {"step": "ai_semantic_validation","removable": true,  "pure": true},
    {"step": "execute",               "removable": false, "pure": false},
    {"step": "output_validation",     "removable": true,  "pure": true},
    {"step": "middleware_after",      "removable": true,  "pure": false},
    {"step": "return_result",         "removable": false, "pure": true}
  ]
}
```

---

## 9. User Extension Examples

### 9.1 Security: IP Whitelist (admin modules only)

```python
class IPWhitelistStep(BaseStep):
    def __init__(self, allowed_ips):
        super().__init__(
            name="ip_whitelist",
            match_modules=("admin.*",),  # only runs for admin.* modules
            pure=True,                   # included in validate()
        )
        self._allowed_ips = set(allowed_ips)

    async def execute(self, ctx):
        ip = ctx.context.identity.attrs.get("ip")
        if ip not in self._allowed_ips:
            return StepResult(action="abort", explanation=f"IP {ip} not allowed")
        return StepResult(action="continue")

strategy.insert_after("acl_check", IPWhitelistStep(["10.0.0.0/8"]))
```

### 9.2 Optimization: Cache Hit (skip to return_result)

```python
class CacheCheckStep(BaseStep):
    def __init__(self, cache):
        super().__init__(name="cache_check", pure=True)
        self._cache = cache

    async def execute(self, ctx):
        cached = self._cache.get(ctx.module_id, ctx.inputs)
        if cached is not None:
            ctx.output = cached; ctx.validated_output = cached
            return StepResult(action="skip_to", skip_to="return_result")
        return StepResult(action="continue")

strategy.insert_after("acl_check", CacheCheckStep(my_cache))
```

### 9.3 Compliance: Fault-Tolerant Audit Log

```python
strategy.insert_before("return_result", BaseStep(
    name="audit_log",
    ignore_errors=True,   # audit failure doesn't block the result
    pure=False,           # not included in validate()
))
```

### 9.4 Input Enrichment (runs before validation, after middleware)

```python
class DefaultInjector(BaseStep):
    def __init__(self):
        super().__init__(name="inject_defaults", match_modules=("*.create",), pure=True)

    async def execute(self, ctx):
        ctx.inputs.setdefault("created_by", ctx.context.identity.id)
        ctx.inputs.setdefault("created_at", datetime.utcnow().isoformat())
        return StepResult(action="continue")

# Insert between middleware_before (6) and input_validation (7)
# The injected defaults will be validated by input_validation.
strategy.insert_before("input_validation", DefaultInjector())
```

---

## 10. Integration Contract

### 10.1 What Pipeline Handles

| Concern | Pipeline step | Notes |
|---------|--------------|-------|
| Context creation | `context_creation` | Identity comes from caller |
| Call chain safety | `call_chain_guard` | Depth, cycles, repeat |
| Module resolution | `module_lookup` | Version negotiation via hint |
| Access control | `acl_check` | Default-deny ACL |
| Human/AI approval | `approval_gate` | External handler protocol |
| Input transformation | `middleware_before` | User middleware chain |
| Input validation | `input_validation` | Schema validation + redaction |
| Execution | `execute` | With dual timeout |
| Output validation | `output_validation` | Schema validation + redaction |
| Output transformation | `middleware_after` | User middleware chain |
| Error propagation | `PipelineEngine` | Algorithm A11 wrapping |
| Sensitive redaction | `input/output_validation` | `x-sensitive` fields |

### 10.2 What Pipeline Does NOT Handle

| Concern | Who handles it | Why not pipeline |
|---------|---------------|-----------------|
| HTTP rate limiting | Transport (apcore-mcp, apcore-cli) | Connection-level, not module-level |
| Authentication | Transport (JWT, API key, OAuth) | Protocol-specific |
| Request routing | Transport (MCP tool→module mapping) | Protocol-specific |
| Database transactions | Module `execute()` internal | Business logic |
| External API calls | Module `execute()` internal | Business logic |
| Retry strategy | `RetryMiddleware` or Module | Cross-cutting or business logic |
| Result caching | Middleware or custom step | Application-specific |

### 10.3 Transport Layer Contract

Transport layers interact with the pipeline through exactly **one interface**:

```python
result = await executor.call_async(module_id, inputs, context)   # normal call
preflight = executor.validate(module_id, inputs, context)         # preflight check
async for chunk in executor.stream(module_id, inputs, context):   # streaming
    ...
```

Transport layers construct:
- `module_id` — from protocol mapping (MCP tool name, CLI command, FastAPI route)
- `inputs` — from protocol parsing (MCP arguments, CLI flags+stdin, HTTP body)
- `context` — from protocol auth (JWT→Identity, request.state.user→Identity, or None)

Transport layers handle protocol-specific error formatting and output formatting.
**Pipeline internals are invisible to transport layers.**

---

## 11. YAML Pipeline Configuration

### 11.1 Motivation

Same codebase, different pipeline per environment — without code changes:

```yaml
# prod.yaml — add rate limiting
pipeline:
  steps:
    - name: rate_limit
      type: rate_limit
      after: acl_check
      match_modules: ["api.*"]
      config: { max_per_minute: 100 }

# dev.yaml — remove access control
pipeline:
  remove: [acl_check, approval_gate]
```

### 11.2 Loading: Startup-Time Only

Pipeline YAML is loaded once at startup, same as Config. Strategy is immutable after construction. No hot-reload.

```
App start → Config.load("apcore.yaml") → build_strategy_from_config(pipeline_config) → App run
```

### 11.3 Step Resolution

Custom steps are resolved via two mechanisms, both at startup:

| Field | Mechanism | Python | TypeScript | Rust |
|-------|-----------|--------|------------|------|
| `handler` | Import class by path | ✓ MUST (`importlib`) | ✓ MUST (`import()`) | ✗ N/A |
| `type` | Look up pre-registered factory | ✓ MUST | ✓ MUST | ✓ MUST |

**Resolution order:** `type` first → `handler` fallback → error.

`handler` is the natural pattern for dynamic languages (no pre-registration needed,
consistent with Django MIDDLEWARE, NestJS providers). `type` is the natural pattern for
compiled languages where runtime import is impossible. Both fields may be present in
shared cross-language YAML; each SDK uses its preferred mechanism.

**When Rust encounters `handler` without `type`:**
```
Error: Pipeline step "rate_limit" has 'handler' but no 'type'.
       Rust SDK requires 'type' for step resolution.
       Register with: pipeline::register_step_type("rate_limit", factory_fn)
```

**Spec conformance:** `handler` — Python MUST, TypeScript MUST, Rust MAY (clear error if unsupported). `type` — all SDKs MUST support.

### 11.4 YAML Schema

```yaml
pipeline:
  # Remove builtin steps
  remove:
    - approval_gate

  # Configure existing step parameters
  configure:
    acl_check:
      timeout_ms: 3000
    call_chain_guard:
      timeout_ms: 2000

  # Add custom steps
  steps:
    - name: rate_limit
      type: rate_limit                            # primary: registered factory
      handler: myapp.steps:RateLimitStep          # fallback: import path (Python/TS)
      after: acl_check                            # insert_after("acl_check")
      match_modules: ["api.*"]
      pure: true
      ignore_errors: false
      timeout_ms: 3000
      config:
        max_per_minute: 100

    - name: audit_log
      type: audit_log
      before: return_result                       # insert_before("return_result")
      ignore_errors: true
      config:
        log_path: /var/log/apcore/audit.jsonl
```

### 11.5 Loading Flow

```
1. Config.load("apcore.yaml")
   └─ pipeline_config = config.get("pipeline")

2. build_strategy_from_config(pipeline_config, step_registry)
   ├─ Start with build_standard_strategy(...)
   ├─ Process "remove": strategy.remove(name) for each
   ├─ Process "configure": update step fields (timeout_ms, ignore_errors, etc.)
   └─ Process "steps":
       ├─ Resolve: step_registry.get(type) or import(handler)
       ├─ Instantiate with config dict
       ├─ Set metadata fields (match_modules, pure, ignore_errors, timeout_ms)
       └─ Insert: strategy.insert_after(after) or strategy.insert_before(before)

3. Executor(strategy=strategy)  — strategy immutable after this point
```

### 11.6 Cross-Language Support

| Capability | Python | TypeScript | Rust |
|------------|--------|------------|------|
| Config accepts `pipeline` key | ✓ dict passthrough | ✓ `registerNamespace` | ✓ `serde(flatten)` |
| `handler` (language-native import) | ✓ `importlib` | ✓ `import()` | ✗ N/A |
| `type` registry | ✓ | ✓ | ✓ |
| Startup-time loading | ✓ | ✓ | ✓ |
| Per-step timeout | ✓ `asyncio.wait_for` | ✓ `Promise.race` | ✓ `tokio::time::timeout` |
| Step backward compat | ✓ `getattr` | ✓ optional fields | ✓ trait defaults |

---

## 12. Migration

### 12.1 Backward Compatibility

The v0.17.0 pipeline redesign is **purely additive**. Existing code works without modification:

```python
# This continues to work exactly as before
executor = Executor(
    registry=registry,
    middlewares=[LoggingMiddleware(), MetricsMiddleware()],
    acl=acl,
    approval_handler=handler,
)
result = await executor.call_async("email.send", inputs, context)
```

| Package | Change required |
|---------|----------------|
| apcore-mcp, apcore-cli, apcore-a2a, fastapi-apcore | None |
| Custom middleware users | None |
| Code referencing `safety_check` step name | Rename to `call_chain_guard` |
| Middleware that relied on pre-validation inputs | Review: inputs are now validated AFTER middleware |

### 12.2 Breaking Changes (v0.17.0)

| Change | Impact | Migration |
|--------|--------|-----------|
| `safety_check` → `call_chain_guard` | Code referencing step name in `insert_before`/`insert_after`/`remove` calls | Find/replace |
| Step 6/7 order swap | Middleware modifications are now validated | Correct behavior; middleware that injected invalid fields may now fail validation (intended) |

---

## 13. Cross-Language Alignment

### 13.1 Current State (v0.18.0)

| Issue | Python | TypeScript | Rust | Resolution |
|-------|--------|-----------|------|------------|
| Middleware before sync/async | Async | Synchronous | Async | All async (Step protocol is async) |
| `call()` sync vs async | Sync `call()` + async `call_async()` | Async-only | Async-only | Keep both in Python for compat; TS/Rust async-only |
| Redaction key | `ctx.redacted_inputs` | `ctx.redactedInputs` | `ctx.redacted_inputs` | Built-in steps sync back to Context for backward compat |
| validate() return type | `PreflightResult` | `PreflightResult` | `PreflightResult` | All three SDKs unified as of v0.18.0 |
| `stream()` | Full implementation | Full implementation | Stub (returns Vec) | Rust full implementation deferred |

### 13.2 Step Execute is Always Async

- Python: `async def execute(self, ctx) -> StepResult`
- TypeScript: `execute(ctx): Promise<StepResult>`
- Rust: `async fn execute(&self, ctx: &mut PipelineContext) -> Result<StepResult, ModuleError>`

Sync steps simply return immediately without awaiting.

---

## 14. What This Design Does NOT Do

| Not doing | Why | Future path |
|-----------|-----|-------------|
| Remove the 11-step default pipeline | Backward compat; ecosystem depends on it | Never — it remains as `standard` strategy |
| Force users to define strategies | Default = current behavior | Strategy is opt-in |
| Built-in AI steps | AI model integration is application-level | Users implement Step protocol with their AI |
| Step-level middleware (per-step before/after) | Over-engineering; step replacement covers this | Re-evaluate if demand emerges |
| Distributed pipeline (steps across services) | Out of scope; apcore is in-process | Belongs to apflow |
| Phase/category system | Steps are a flat ordered list | Users position with insert_before/insert_after |
| Webhook steps | External calls belong in Module.execute() or Middleware | N/A |
| Transport-level concerns | Rate limiting, auth, routing stay in transport layers | N/A |

---

## 15. Relationship to Other Documents

| Document | Relationship |
|----------|-------------|
| `design-context-annotations-acl.md` | ACL condition handlers are used inside `BuiltinACLCheck`. `ContextKey` is used by steps to read Tier 2 data. Module Annotations `extra` is NOT step-specific. |
| `PROTOCOL_SPEC.md` §7.4 | This design extends §7.4 (Executor Integration) with the configurable pipeline. |
| `docs/features/config-bus.md` | Strategy names can be loaded from Config Bus (`executor.strategies` in YAML). |
| `docs/features/core-executor.md` | User-facing feature doc summarizing the pipeline. This document is the authoritative design reference. |
