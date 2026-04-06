# Design: Execution Pipeline Strategy & AI Decision Support

> Status: **Draft** | Author: apcore team | Date: 2026-04-02

This document specifies the redesign of apcore's execution pipeline from a hardcoded 11-step sequence to a configurable, extensible pipeline with AI decision support. All changes apply equally to Python, TypeScript, and Rust SDKs.

---

## 1. Problem Statement

### 1.1 Current State

The Executor implements a hardcoded 11-step pipeline:

```
1. Context Creation         (not skippable)
2. Safety Checks            (not skippable)
3. Module Lookup            (not skippable)
4. ACL Enforcement          (skipped if acl=None)
5. Approval Gate            (skipped if handler=None)
6. Input Validation         (always runs)
7. Middleware Before         (always runs)
8. Module Execution          (always runs)
9. Output Validation         (always runs)
10. Middleware After          (always runs)
11. Return Result            (always runs)
```

Every module invocation across the entire ecosystem goes through this pipeline:
- Framework integrations: FastAPI, Django, Flask, NestJS, Axum
- Surface adapters: MCP (Python/TS), CLI (Python/TS), A2A (Python/TS/Rust)
- User applications: via framework or adapter

### 1.2 Problems

| Problem | Impact |
|---------|--------|
| Cannot skip steps (ACL, validation) for internal calls | Performance overhead, workaround via acl=None |
| Cannot add steps (rate limiting, cost budgeting, circuit breaker) | Features crammed into middleware before/after with unclear semantics |
| Cannot replace step implementations (custom validator, OPA ACL) | Locked to built-in implementations |
| Pipeline is invisible to AI agents | AI cannot reason about what safety checks will run |
| No execution trace for AI learning | AI cannot learn from pipeline execution history |
| No AI decision points in pipeline | AI cannot participate in approval, risk assessment, routing |
| Cross-language inconsistency | TS middleware is sync, Python/Rust async; redaction storage paths differ |

### 1.3 Design Goals

1. **Configurable pipeline** — steps can be added, removed, replaced, reordered
2. **Safety guarantees preserved** — core steps (context, lookup, execute, return) cannot be removed
3. **AI perceivable** — pipeline structure visible to AI, execution trace available for learning
4. **AI participatory** — AI can be a step implementation (approval, risk assessment, semantic validation)
5. **Backward compatible** — default pipeline = current 11 steps, no migration required for existing code
6. **Cross-language consistent** — identical API across Python, TypeScript, Rust
7. **YAML declarable** — pipeline strategy can be configured in apcore.yaml

---

## 2. Core Concepts

### 2.1 Step

A single unit of work in the execution pipeline.

```
Step {
  name:        string         // unique identifier (e.g., "acl_check")
  description: string         // AI-readable purpose description
  removable:   bool           // false = safety-critical, cannot be removed
  replaceable: bool           // true = can swap implementation
  
  execute(ctx: PipelineContext) -> StepResult
}
```

> **Configuration injection:** The Step protocol only defines the `execute()` contract.
> Step implementations receive their configuration via constructor (e.g.,
> `BuiltinACLCheck(acl=acl)`, `BuiltinExecute(timeout_ms=30000)`). The protocol
> does NOT constrain constructors — this is intentional, as different steps need
> different configuration shapes.
>
> **Property vs attribute:** In Python, `name`, `description`, `removable`,
> `replaceable` are declared as `@property` in the Protocol but implementations
> SHOULD use plain instance attributes (set in `__init__`), not class attributes.
> This avoids the class-attribute-vs-property conflict in frozen dataclasses.

### 2.2 StepResult

The outcome of a step, with AI-readable explanation.

```
StepResult {
  action:       "continue" | "skip_to" | "abort"
  skip_to:      string | nil             // target step name (for skip_to action)
  explanation:  string | nil             // AI/human-readable reason
  confidence:   float | nil              // AI decision confidence (0.0-1.0)
  alternatives: list[string] | nil       // suggested alternatives on abort
}
```

> **Data flow:** Steps do NOT pass data through `StepResult`. `StepResult` only
> controls flow (continue/skip/abort) and provides AI-readable metadata.
>
> **Two-tier data model** (see also `design-context-annotations-acl.md` §1.6):
>
> | Tier | Storage | Written by | Read via | Example |
> |------|---------|-----------|----------|---------|
> | **Tier 1** | `PipelineContext` fields | Built-in steps | Direct field access | `ctx.module`, `ctx.output`, `ctx.validated_inputs` |
> | **Tier 2** | `context.data` | Middleware, custom steps | `ContextKey[T]` (type-safe) | `TRACING_SPANS.set(ctx.context, [...])` |
>
> **Rules:**
> - Built-in steps write pipeline-essential data to Tier 1 (PipelineContext fields).
> - Middleware and custom steps store extension state in Tier 2 (context.data via ContextKey).
> - Steps reading Tier 2 data SHOULD use ContextKey for type safety.
> - PipelineContext fields are NOT duplicated into context.data (single source of truth).
> - Built-in steps also sync Tier 1 data back to Context fields for backward compat
>   (e.g., `ctx.context.redacted_inputs = ctx.validated_inputs` after input_validation).

### 2.3 ExecutionStrategy

An ordered list of steps defining a complete pipeline.

```
ExecutionStrategy {
  name:  string
  steps: list[Step]
  
  insert_after(anchor: string, step: Step)
  insert_before(anchor: string, step: Step)
  remove(step_name: string)               // raises if step.removable == false
  replace(step_name: string, new: Step)    // raises if step.replaceable == false
}
```

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
  stream:          bool = false              // true when streaming mode requested
  output_stream:   AsyncGenerator | nil      // set by BuiltinExecute when stream is true
  
  // Metadata
  strategy:     ExecutionStrategy
  trace:        PipelineTrace     // accumulates step results
}
```

> **Field availability:** Steps MUST check for `nil` before reading fields set by
> other steps. For example, a custom step inserted before `module_lookup` will see
> `ctx.module == nil`. The standard pipeline guarantees field availability order:
> `module` available after step 3, `validated_inputs` after step 6, `output` after
> step 8, `validated_output` after step 9. Custom pipelines that reorder steps
> must handle nil fields accordingly.

### 2.5 StrategyInfo

Returned by `list_strategies()` for AI introspection.

```
StrategyInfo {
  name:          string
  step_count:    int
  step_names:    list[string]
  description:   string         // auto-generated from step descriptions
}
```

### 2.6 PipelineTrace

Complete execution record, AI-readable.

```
PipelineTrace {
  module_id:       string
  strategy_name:   string
  steps:           list[StepTrace]
  total_duration_ms: float
  success:         bool
}

StepTrace {
  name:            string
  duration_ms:     float
  result:          StepResult       // includes explanation, confidence
  skipped:         bool
  decision_point:  bool             // true if step is AI decision point
}
```

---

## 3. Step Protocol

### 3.1 Python

```python
from typing import Protocol, runtime_checkable
from abc import ABC, abstractmethod

# Protocol for structural typing (type checkers)
@runtime_checkable
class Step(Protocol):
    """A single step in the execution pipeline."""

    @property
    def name(self) -> str: ...

    @property
    def description(self) -> str: ...

    @property
    def removable(self) -> bool: ...

    @property
    def replaceable(self) -> bool: ...

    async def execute(self, ctx: PipelineContext) -> StepResult: ...

# Base class for implementation convenience (recommended but not required)
class BaseStep(ABC):
    """Convenience base class for step implementations."""

    def __init__(self, name: str, description: str, *,
                 removable: bool = True, replaceable: bool = True) -> None:
        self.name = name
        self.description = description
        self.removable = removable
        self.replaceable = replaceable

    @abstractmethod
    async def execute(self, ctx: PipelineContext) -> StepResult: ...
```

> **Note on runtime_checkable:** Python's `@runtime_checkable` Protocol cannot
> verify that `execute` is a coroutine function — it only checks the method exists.
> The `PipelineEngine` MUST additionally verify at registration time:
> `assert inspect.iscoroutinefunction(step.execute)`, or handle sync returns
> gracefully (wrap in coroutine).

### 3.2 TypeScript

```typescript
export interface Step {
  readonly name: string;
  readonly description: string;
  readonly removable: boolean;
  readonly replaceable: boolean;

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

    async fn execute(&self, ctx: &mut PipelineContext) -> Result<StepResult, ModuleError>;
}
```

---

## 4. Built-in Steps

### 4.0 Core vs Optional Steps

The standard pipeline has 11 steps, but only **4 are mandatory** (non-removable). The other 7 are optional and can be removed to build lighter pipelines:

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
| `execute` | Without execution, there is no output (replaceable for dry-run, but not removable) |
| `return_result` | Pipeline must have a terminal step that finalizes the output |

**Optional steps** form the safety, validation, and extensibility layers. Remove them when their guarantees are provided externally or are unnecessary for a specific call path.

The `minimal` preset strategy strips all optional steps, leaving only the 4 core steps — suitable for pre-validated internal hot paths where the caller has already verified ACL, inputs, and call safety.

### 4.1 Step Inventory

| # | Name | Removable | Replaceable | Description |
|---|------|-----------|-------------|-------------|
| 1 | `context_creation` | false | false | Create or inherit execution context, set global deadline |
| 2 | `call_chain_guard` | true | true | Validate call depth, module repeat limits, cancel token |
| 3 | `module_lookup` | false | false | Resolve module from registry by ID |
| 4 | `acl_check` | true | true | Enforce access control rules |
| 5 | `approval_gate` | true | true | Handle human or AI approval flow |
| 6 | `middleware_before` | true | false | Execute registered before-middleware chain |
| 7 | `input_validation` | true | true | Validate inputs against schema, redact sensitive fields |
| 8 | `execute` | false | true | Invoke the module with timeout enforcement |
| 9 | `output_validation` | true | true | Validate outputs against schema, redact sensitive fields |
| 10 | `middleware_after` | true | false | Execute registered after-middleware chain |
| 11 | `return_result` | false | false | Finalize and return output |

**Safety invariant:** Steps 1, 3, 8, 11 are never removable — context must exist, module must be found, execution must happen, result must return. Step 8 (`execute`) is replaceable (e.g., `ValidateOnlyStep` for dry-run) but not removable.

!!! warning "Replacing the Execute Step"
    `strategy.replace("execute", custom_step)` bypasses the built-in timeout enforcement, cancel token checks, global deadline clamping, and streaming detection. Your replacement MUST re-implement these if you rely on them. The built-in `BuiltinExecute` is the only step that enforces the dual-timeout model (per-module + global deadline). A replacement that calls `module.execute()` directly without timeout wrapping will allow unbounded execution time. This is intentional — it enables dry-run, mock, and custom execution strategies — but treat it as an advanced escape hatch, not a casual customization point.

**Middleware steps (6, 10) are removable but not replaceable:** They can be removed entirely (e.g., PERFORMANCE strategy), but their implementation wraps the existing middleware chain contract. Replacing them would break the `Middleware` protocol that ecosystem packages depend on. If you need custom pre/post logic, add a custom step before/after them instead.

### 4.2 When to Use Middleware vs Custom Steps

The pipeline offers two extension mechanisms. Use the right one for the job:

| Criterion | Middleware | Custom Step |
|-----------|-----------|-------------|
| **Purpose** | Cross-cutting concerns (logging, retry, metrics, tracing) | Pipeline logic extensions (rate limiting, cost budgeting, custom auth) |
| **Execution position** | Fixed at steps 6 and 10 (before/after execution) | Any position via `insert_before` / `insert_after` |
| **Lifecycle** | Paired: `before()` + `after()` + `on_error()` form an onion | Independent: single `execute()` method |
| **Error recovery** | `on_error()` can return recovery output | Return `StepResult(action="abort")` |
| **Registration** | `executor.use(middleware)` — dynamic, per-executor | `strategy.insert_after(anchor, step)` — per-strategy |
| **Observability** | Not individually traced | Each step appears in `PipelineTrace` |

**Rule of thumb:** If your logic needs to wrap execution (see both inputs and outputs as a pair), use Middleware. If your logic is a gate or transform at a specific pipeline position, use a Custom Step.

### 4.3 Built-in Step Implementations

Each built-in step wraps the existing executor logic. Example for `acl_check`:

```python
class BuiltinACLCheck(BaseStep):
    """Built-in ACL enforcement step."""

    def __init__(self, acl: ACL | None = None) -> None:
        super().__init__(
            name="acl_check",
            description="Enforce access control rules against caller identity and target module",
            removable=True,
            replaceable=True,
        )
        self._acl = acl

    async def execute(self, ctx: PipelineContext) -> StepResult:
        if self._acl is None:
            return StepResult(
                action="continue",
                explanation="ACL not configured, skipping",
            )
        # Use async_check if available (new API), fallback to sync check (old API).
        if hasattr(self._acl, "async_check"):
            allowed = await self._acl.async_check(
                ctx.context.caller_id, ctx.module_id, ctx.context,
            )
        else:
            allowed = self._acl.check(
                ctx.context.caller_id, ctx.module_id, ctx.context,
            )
        if not allowed:
            return StepResult(
                action="abort",
                explanation=f"ACL denied: caller '{ctx.context.caller_id}' cannot invoke '{ctx.module_id}'",
            )
        return StepResult(
            action="continue",
            explanation=f"ACL allowed: caller '{ctx.context.caller_id}' authorized",
        )
```

---

## 5. Execution Strategy

### 5.1 Default Strategy (STANDARD)

Equivalent to current 11-step pipeline. STANDARD is a **factory function**, not a
pre-instantiated constant, because built-in steps require runtime dependencies
(registry, acl, config, middlewares) that are not available at import time:

```python
def build_standard_strategy(
    *,
    registry: Registry,
    config: Config | None = None,
    acl: ACL | None = None,
    approval_handler: ApprovalHandler | None = None,
    middlewares: list[Middleware] | None = None,
) -> ExecutionStrategy:
    """Build the standard 11-step pipeline with injected dependencies."""
    return ExecutionStrategy(
        name="standard",
        steps=[
            BuiltinContextCreation(config=config),
            BuiltinSafetyCheck(config=config),
            BuiltinModuleLookup(registry=registry),
            BuiltinACLCheck(acl=acl),
            BuiltinApprovalGate(handler=approval_handler),
            BuiltinInputValidation(),
            BuiltinMiddlewareBefore(middlewares=middlewares or []),
            BuiltinExecute(config=config),
            BuiltinOutputValidation(),
            BuiltinMiddlewareAfter(middlewares=middlewares or []),
            BuiltinReturnResult(),
        ],
    )
```

> **Preset strategies (`INTERNAL`, `TESTING`, etc.) are also factory functions**,
> not constants. They call `build_standard_strategy()` internally then apply
> remove/replace modifications.

### 5.2 Preset Strategies

Presets are also **factory functions** (not constants), because they delegate to
`build_standard_strategy()` which requires runtime dependencies:

```python
def build_internal_strategy(**kwargs) -> ExecutionStrategy:
    """Standard pipeline minus ACL and approval."""
    strategy = build_standard_strategy(**kwargs)
    strategy.remove("acl_check")
    strategy.remove("approval_gate")
    strategy._name = "internal"
    return strategy

def build_testing_strategy(**kwargs) -> ExecutionStrategy:
    """Minimal pipeline for tests — no safety, ACL, or approval."""
    strategy = build_standard_strategy(**kwargs)
    strategy.remove("acl_check")
    strategy.remove("approval_gate")
    strategy.remove("call_chain_guard")
    strategy._name = "testing"
    return strategy

def build_performance_strategy(**kwargs) -> ExecutionStrategy:
    """Skip middleware for performance-critical paths."""
    strategy = build_standard_strategy(**kwargs)
    strategy.remove("middleware_before")
    strategy.remove("middleware_after")
    strategy._name = "performance"
    return strategy

def build_minimal_strategy(**kwargs) -> ExecutionStrategy:
    """Core steps only — no safety, ACL, approval, validation, or middleware."""
    strategy = build_standard_strategy(**kwargs)
    strategy.remove("call_chain_guard")
    strategy.remove("acl_check")
    strategy.remove("approval_gate")
    strategy.remove("middleware_before")
    strategy.remove("input_validation")
    strategy.remove("output_validation")
    strategy.remove("middleware_after")
    strategy._name = "minimal"
    return strategy
```

Summary of all preset strategies:

| Strategy | Steps | Removed from standard | Use case |
|----------|-------|----------------------|----------|
| `standard` | 11 | — | Default, full safety |
| `internal` | 9 | acl, approval | Trusted service-to-service calls |
| `testing` | 8 | acl, approval, guard | Unit/integration tests |
| `performance` | 9 | middleware_before/after | Latency-sensitive paths |
| `minimal` | **4** | All optional steps | Pre-validated internal hot paths |

!!! note "Dry-Run Validation"
    The `validate()` method (with `dry_run=True`) replaces the previously proposed `validate_only` strategy. The `dry_run` flag automatically skips non-pure steps in any strategy, making a dedicated preset unnecessary.

When the user passes `strategy="internal"` as a string, the Executor calls the
corresponding factory function with its own registry/config/acl/middlewares.

### 5.3 Custom Strategy with Additional Steps

```python
# User adds rate limiting and cost budgeting
my_strategy = ExecutionStrategy.from_standard(name="my_pipeline")
my_strategy.insert_after("acl_check", RateLimiterStep(max_rps=100))
my_strategy.insert_after("approval_gate", CostBudgetStep(max_cost=1.0))
# Resulting pipeline (13 steps):
#   context_creation → call_chain_guard → module_lookup → acl_check →
#   rate_limiter (new) → approval_gate → cost_budget (new) →
#   middleware_before → input_validation → execute →
#   output_validation → middleware_after → return_result
```

### 5.4 Strategy Modification API

```python
class ExecutionStrategy:
    def __init__(self, name: str, steps: list[Step]) -> None: ...

    @classmethod
    def from_standard(
        cls,
        name: str,
        remove: list[str] | None = None,
        replace: dict[str, Step] | None = None,
    ) -> "ExecutionStrategy":
        """Create a strategy by modifying the standard pipeline.
        
        Operation order: replace FIRST, then remove. Both operations
        respect step constraints: replace checks `replaceable`, remove
        checks `removable`. Attempting to remove a non-removable step
        raises StepNotRemovableError even in from_standard().
        """
        ...

    def insert_after(self, anchor: str, step: Step) -> None:
        """Insert step after the named anchor step.
        Raises StepNotFoundError if anchor doesn't exist."""
        ...

    def insert_before(self, anchor: str, step: Step) -> None:
        """Insert step before the named anchor step."""
        ...

    def remove(self, step_name: str) -> None:
        """Remove a step. Raises StepNotRemovableError if step.removable is False."""
        ...

    def replace(self, step_name: str, new_step: Step) -> None:
        """Replace a step. Raises StepNotReplaceableError if step.replaceable is False."""
        ...

    def step_names(self) -> list[str]:
        """Return ordered list of step names (for AI introspection)."""
        ...

# Note: from_standard(remove=..., replace=...) is a convenience for one-shot
# construction. It is equivalent to calling from_standard() then instance
# methods remove()/replace() in sequence. Both paths produce the same result.

# INVARIANT: Step names MUST be unique within a strategy.
# insert_after/insert_before MUST raise StepNameDuplicateError if a step
# with the same name already exists. This guarantees that remove(), replace(),
# and skip_to() always target exactly one step.
```

### 5.5 Accessing Context Data from Steps

Custom steps that need to read middleware state (e.g., tracing spans, metrics)
or write extension data should use `ContextKey` from the Context redesign:

```python
from apcore.context_keys import TRACING_SPANS, METRICS_STARTS, ContextKey

class MyCustomStep(BaseStep):
    def __init__(self) -> None:
        super().__init__(name="my_step", description="Custom processing step")
    
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

**Rule of thumb:** If the data is needed by the pipeline engine (module, inputs,
output) → use Tier 1 (PipelineContext field). If the data is middleware/extension
state → use Tier 2 (context.data via ContextKey).

### 5.6 YAML Declaration

```yaml
# apcore.yaml
executor:
  default_strategy: standard

  strategies:
    internal:
      base: standard
      remove: [acl_check, approval_gate]

    with_rate_limit:
      base: standard
      insert:
        - step: rate_limiter
          after: acl_check            # insert after this step
          class: myapp.steps.RateLimiterStep
          config:
            max_rps: 100
        # Also supported: "before: execute" to insert before a step

    # ai_governed: Python-only (uses class: for dynamic loading).
    # For TS/Rust, register the same strategy via code:
    #   Executor.registerStrategy("ai_governed", strategy)
    ai_governed:
      base: standard
      insert:
        - step: ai_risk_assessment
          after: acl_check
          class: myapp.steps.AIRiskAssessment   # Python-only: dynamic import
          config:
            model: gpt-4
            threshold: 0.7
        - step: ai_semantic_validation
          after: input_validation
          class: myapp.steps.AISemanticValidator  # Python-only
      replace:
        approval_gate:
          class: myapp.steps.AIApprovalStep       # Python-only
          config:
            auto_approve_below: 0.3
            require_human_above: 0.8
```

> **Cross-language YAML support:** Strategies using only `remove` and `replace`
> with built-in step names work in all three languages. Strategies using `class:`
> for custom steps work only in Python. TS/Rust equivalents:
> ```typescript
> // TypeScript — insert_after/replace are void, so no chaining.
> const aiGoverned = buildStandardStrategy({ registry, config });
> aiGoverned.insertAfter("acl_check", new AIRiskAssessment({ model: "gpt-4", threshold: 0.7 }));
> aiGoverned.replace("approval_gate", new AIApprovalStep({ autoApproveBelow: 0.3 }));
> Executor.registerStrategy("ai_governed", aiGoverned);
> ```

---

## 6. Executor API Changes

### 6.1 Constructor

```python
# Python
class Executor:
    def __init__(
        self,
        registry: Registry,
        *,
        strategy: ExecutionStrategy | str | None = None,  # NEW: strategy or strategy name
        middlewares: list[Middleware] | None = None,        # kept for backward compat
        acl: ACL | None = None,                            # kept for backward compat
        config: Config | None = None,
        approval_handler: ApprovalHandler | None = None,   # kept for backward compat
    ) -> None: ...
```

**Backward compatibility:** When `strategy` is None, constructs STANDARD strategy from `middlewares`, `acl`, and `approval_handler` parameters (current behavior). When `strategy` is provided, `middlewares`/`acl`/`approval_handler` are ignored (steps in strategy contain their own dependencies).

**Strategy name resolution:** When `strategy` is a `str`, the Executor resolves it in this order:
1. Built-in presets: `"standard"`, `"internal"`, `"testing"`, `"performance"`, `"minimal"`
2. Code-registered strategies: via `Executor.register_strategy(name, strategy)` class method
3. YAML-defined strategies (Python only): loaded from `executor.strategies` in Config
4. If not found: raises `StrategyNotFoundError`

> **YAML `class:` field and cross-language support:** YAML strategies with `class:` fields
> use dynamic class loading (`importlib.import_module` in Python). This is **Python-only**.
> TypeScript and Rust cannot dynamically load classes at runtime from a string.
> For TS/Rust, YAML strategies are limited to `remove` and `replace` with built-in step
> names (no custom classes). Custom steps in TS/Rust MUST be registered via code:
> ```typescript
> Executor.registerStrategy("my_pipeline", myStrategy);
> ```
> ```rust
> Executor::register_strategy("my_pipeline", my_strategy);
> ```

### 6.2 Call Methods

```python
# Existing API unchanged
result = executor.call(module_id, inputs, context)           # sync (Python only)
result = await executor.call_async(module_id, inputs, context)  # async

# NEW: per-call strategy override
result = executor.call(module_id, inputs, context, strategy="internal")
result = await executor.call_async(module_id, inputs, context, strategy=my_strategy)

# NEW: call with trace (also accepts strategy override)
result, trace = executor.call_with_trace(module_id, inputs, context)
result, trace = await executor.call_async_with_trace(module_id, inputs, context, strategy="internal")
```

> **Python sync `call()` with async pipeline:** The sync `call()` method uses the
> same approach as the current implementation — `asyncio.get_event_loop().run_until_complete()`
> or thread-based bridge to run the async pipeline. All Steps are async (the protocol
> requires it), so sync `call()` always bridges to async internally. This matches
> the current behavior where sync `call()` bridges async module execution.

### 6.3 Introspection

```python
# List available strategies
strategies = executor.list_strategies()
# → [{"name": "standard", "steps": ["context_creation", ...], "step_count": 11}, ...]

# Get current strategy
strategy = executor.current_strategy
# → ExecutionStrategy(name="standard", steps=[...])

# Describe pipeline (AI-readable)
description = executor.describe_pipeline()
# → "11-step pipeline: context_creation → call_chain_guard → module_lookup → ..."

# Register a custom strategy (class-level, global)
Executor.register_strategy("my_pipeline", my_strategy)
```

### 6.4 Cross-Language Signatures

**TypeScript:**
```typescript
class Executor {
  constructor(options: {
    registry: Registry;
    strategy?: ExecutionStrategy | string | null;
    middlewares?: Middleware[] | null;      // backward compat
    acl?: ACL | null;                      // backward compat
    config?: Config | null;
    approvalHandler?: ApprovalHandler | null; // backward compat
  });

  // strategy override via options object (4th param) — backward compat since
  // existing code passes at most 3 positional args (moduleId, inputs, context).
  async call(moduleId: string, inputs: Record<string, unknown>,
             context?: Context | null,
             options?: { strategy?: ExecutionStrategy | string }): Promise<Record<string, unknown>>;

  async callWithTrace(moduleId: string, inputs: Record<string, unknown>,
                      context?: Context | null,
                      options?: { strategy?: ExecutionStrategy | string }): Promise<[Record<string, unknown>, PipelineTrace]>;

  listStrategies(): StrategyInfo[];
  describePipeline(): string;

  // Class-level (static) — registers into global strategy registry.
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
        strategy: Option<&ExecutionStrategy>,  // None = use default
    ) -> Result<(Value, PipelineTrace), ModuleError>;

    pub fn list_strategies(&self) -> Vec<StrategyInfo>;

    /// Class-level (static) method. Registers into a global strategy registry
    /// (similar to Config's global namespace registry).
    pub fn register_strategy(name: impl Into<String>, strategy: ExecutionStrategy);
}
```

**StrategyInfo** (returned by `list_strategies()`):
```rust
pub struct StrategyInfo {
    pub name: String,
    pub step_count: usize,
    pub step_names: Vec<String>,
    pub description: String,  // auto-generated from step descriptions
}
```

---

## 7. AI Decision Support

### 7.1 AI as Step Implementor

Any step can be implemented by AI. The Step protocol does not distinguish AI from non-AI implementations.

```python
class AIRiskAssessment(BaseStep):
    """AI evaluates execution risk before proceeding."""

    def __init__(self, model: str = "gpt-4", threshold: float = 0.7) -> None:
        super().__init__(
            name="ai_risk_assessment",
            description="AI model evaluates the risk of executing this module with given inputs",
        )
        self._model = model
        self._threshold = threshold

    async def execute(self, ctx: PipelineContext) -> StepResult:
        risk = await self._evaluate_risk(ctx)
        if risk.score > self._threshold:
            return StepResult(
                action="abort",
                explanation=f"High risk ({risk.score:.0%}): {risk.reason}",
                confidence=risk.score,
                alternatives=risk.suggested_modules,
            )
        return StepResult(
            action="continue",
            explanation=f"Risk acceptable ({risk.score:.0%})",
            confidence=1.0 - risk.score,
        )

class AIApprovalStep(BaseStep):
    """AI decides approval based on risk, replacing human approval."""

    def __init__(self, auto_approve_below: float = 0.3, require_human_above: float = 0.8) -> None:
        super().__init__(
            name="ai_approval",
            description="AI evaluates whether to approve module execution based on risk analysis",
        )
        self._auto_approve = auto_approve_below
        self._human_threshold = require_human_above

    async def execute(self, ctx: PipelineContext) -> StepResult:
        risk = await self._assess(ctx)
        if risk < self._auto_approve:
            return StepResult(action="continue", explanation="Auto-approved (low risk)",
                              confidence=1.0 - risk)
        if risk > self._human_threshold:
            return StepResult(action="abort", explanation="Requires human approval (high risk)",
                              confidence=risk)
        # Medium risk: AI decides
        decision = await self._ai_decide(ctx, risk)
        return StepResult(
            action="continue" if decision.approved else "abort",
            explanation=decision.reason,
            confidence=decision.confidence,
        )
```

### 7.2 AI Strategy Selection

AI agents can inspect available strategies and choose at call time:

```python
# AI agent queries available strategies
strategies = executor.list_strategies()
# → [
#   StrategyInfo(name="standard", step_count=11,
#                description="Full safety pipeline with ACL, approval, validation"),
#   StrategyInfo(name="internal", step_count=9,
#                description="Skip ACL and approval for trusted internal calls"),
#   StrategyInfo(name="ai_governed", step_count=14,
#                description="AI risk assessment and approval with semantic validation"),
# ]

# AI chooses based on context
result = await executor.call_async(
    "email.send", inputs, context,
    strategy="internal",  # AI decided this is a trusted internal call
)
```

### 7.3 Pipeline Trace for AI Learning

```python
result, trace = await executor.call_async_with_trace("email.send", inputs, context)

# trace is AI-readable:
# PipelineTrace(
#   module_id="email.send",
#   strategy_name="ai_governed",
#   total_duration_ms=487.3,
#   success=True,
#   steps=[
#     StepTrace(name="context_creation", duration_ms=0.1, skipped=False,
#               result=StepResult(action="continue")),
#     StepTrace(name="acl_check", duration_ms=1.2, skipped=False,
#               result=StepResult(action="continue",
#                   explanation="Allowed: caller has 'email.operator' role")),
#     StepTrace(name="ai_risk_assessment", duration_ms=150, skipped=False,
#               decision_point=True,
#               result=StepResult(action="continue",
#                   explanation="Risk acceptable (12%)", confidence=0.88)),
#     StepTrace(name="execute", duration_ms=320, skipped=False,
#               result=StepResult(action="continue")),
#     ...
#   ]
# )
```

### 7.4 AI Perceivability Guarantees

Each module invocation has a fully visible pipeline:

```json
{
  "module_id": "email.send",
  "strategy": "ai_governed",
  "pipeline": [
    {"step": "context_creation", "removable": false, "description": "Create execution context"},
    {"step": "call_chain_guard", "removable": true, "description": "Check call depth and repeat limits"},
    {"step": "module_lookup", "removable": false, "description": "Resolve module from registry"},
    {"step": "acl_check", "removable": true, "description": "Enforce access control rules"},
    {"step": "ai_risk_assessment", "removable": true, "description": "AI evaluates execution risk"},
    {"step": "ai_approval", "removable": true, "description": "AI decides approval based on risk"},
    {"step": "middleware_before", "removable": true, "description": "Execute before-middleware chain"},
    {"step": "input_validation", "removable": true, "description": "Validate inputs against schema"},
    {"step": "ai_semantic_validation", "removable": true, "description": "AI validates input semantics"},
    {"step": "execute", "removable": false, "description": "Invoke the module"},
    {"step": "output_validation", "removable": true, "description": "Validate outputs against schema"},
    {"step": "middleware_after", "removable": true, "description": "Execute after-middleware chain"},
    {"step": "return_result", "removable": false, "description": "Return final output"}
  ]
}
```

AI sees this and knows: "This call goes through AI risk assessment and AI approval. If risk is high, it may be rejected. I should check the confidence score in the trace."

---

## 8. Pipeline Execution Engine

### 8.1 Core Loop

```python
class PipelineEngine:
    """Executes a pipeline strategy step by step."""

    async def run(
        self, strategy: ExecutionStrategy, ctx: PipelineContext,
    ) -> tuple[Any, PipelineTrace]:
        trace = PipelineTrace(
            module_id=ctx.module_id,
            strategy_name=strategy.name,
            steps=[],
            success=False,
        )
        start = time.monotonic()
        steps = strategy.steps
        i = 0

        # Index-based loop (not for-each) to support skip_to.
        while i < len(steps):
            step = steps[i]
            step_start = time.monotonic()
            try:
                result = await step.execute(ctx)
            except Exception as exc:
                trace.steps.append(StepTrace(
                    name=step.name,
                    duration_ms=(time.monotonic() - step_start) * 1000,
                    result=StepResult(action="abort", explanation=str(exc)),
                    skipped=False,
                    decision_point=False,
                ))
                trace.total_duration_ms = (time.monotonic() - start) * 1000
                raise

            step_trace = StepTrace(
                name=step.name,
                duration_ms=(time.monotonic() - step_start) * 1000,
                result=result,
                skipped=False,
                decision_point=result.confidence is not None,
            )
            trace.steps.append(step_trace)

            if result.action == "abort":
                trace.total_duration_ms = (time.monotonic() - start) * 1000
                raise PipelineAbortError(
                    step=step.name,
                    explanation=result.explanation,
                    alternatives=result.alternatives,
                    trace=trace,
                )
            elif result.action == "skip_to":
                # Fast-forward to the named step (from current position).
                target = result.skip_to
                target_idx = None
                for j in range(i + 1, len(steps)):
                    if steps[j].name == target:
                        target_idx = j
                        break
                    # Record skipped steps in trace
                    trace.steps.append(StepTrace(
                        name=steps[j].name,
                        duration_ms=0,
                        result=StepResult(action="continue"),
                        skipped=True,
                        decision_point=False,
                    ))
                if target_idx is None:
                    raise StepNotFoundError(target)
                i = target_idx  # Jump to target step (loop will execute it next)
                continue

            # action == "continue" → proceed to next step
            i += 1

        trace.success = True
        trace.total_duration_ms = (time.monotonic() - start) * 1000
        # Return the most-processed output available:
        # validated_output (if output_validation ran) > output (if only execute ran).
        # For VALIDATE_ONLY strategy: BuiltinValidateOnly sets ctx.output to a
        # validation summary dict (not None), so final_output is meaningful.
        # If both are None (degenerate pipeline), returns None.
        final_output = ctx.validated_output if ctx.validated_output is not None else ctx.output
        return final_output, trace
```

### 8.2 Error Types

```python
class PipelineAbortError(ModuleError):
    """Raised when a step aborts the pipeline."""
    step: str
    explanation: str | None
    alternatives: list[str] | None
    trace: PipelineTrace

class StepNotFoundError(ModuleError):
    """Raised when skip_to targets a non-existent step."""

class StepNotRemovableError(ModuleError):
    """Raised when trying to remove a non-removable step."""

class StepNotReplaceableError(ModuleError):
    """Raised when trying to replace a non-replaceable step."""

class StrategyNotFoundError(ModuleError):
    """Raised when a strategy name cannot be resolved."""

class StepNameDuplicateError(ModuleError):
    """Raised when inserting a step with a name that already exists in the strategy."""
```

### 8.3 PipelineTrace Scope and Serialization

`PipelineTrace` is **process-local** — it is returned to the caller of
`call_with_trace()` for inspection, logging, or AI learning within the same
process. It is NOT designed for cross-process transmission.

Rules:
- PipelineTrace is NOT stored in `context.data` (it lives on PipelineContext.trace)
- PipelineTrace does NOT have a `_context_version` field (not cross-process)
- PipelineTrace MAY be serialized to JSON for logging/persistence, but this is
  the caller's responsibility (no built-in wire format in v1)
- If cross-process trace transmission is needed in the future, add a versioned
  wire format at that time

### 8.4 Streaming Support

The current Executor has `stream()` / `call_stream_async()` for streaming output. The pipeline design handles streaming as follows:

**Steps 1-7 are identical** for streaming and non-streaming calls — context, safety, lookup, ACL, approval, validation, middleware-before all run the same way.

**Step 8 (execute) differs:** Streaming mode is determined by the **caller** (via `executor.stream()` or `PipelineEngine.run_stream()`), not by the execute step detecting module capabilities. When `run_stream()` is used, `ctx.stream` is set to `True` before the pipeline starts. The `BuiltinExecute` step checks `ctx.stream`: if True, it calls `module.stream()` and stores the async generator in `ctx.output_stream`; if False, it calls `module.execute()` and stores the result in `ctx.output`.

**Steps 9-11 differ for streaming:**
- Step 9 (output_validation): Validates each chunk individually OR validates the accumulated result after stream completes, depending on configuration.
- Step 10 (middleware_after): Runs once after stream completes (on accumulated output), NOT per-chunk.
- Step 11 (return_result): Returns the async generator, not a dict.

```python
class PipelineContext:
    # ... existing fields ...
    stream: bool = False                    # set by PipelineEngine.run_stream() BEFORE pipeline starts
    output_stream: AsyncGenerator | None = None  # set by BuiltinExecute step when ctx.stream is True
```

The `PipelineEngine` provides both methods:

```python
class PipelineEngine:
    async def run(self, strategy, ctx) -> tuple[Any, PipelineTrace]: ...
    
    async def run_stream(self, strategy, ctx) -> tuple[AsyncGenerator, PipelineTrace]:
        """Execute pipeline in streaming mode.
        
        Steps 1-7 run identically. Step 8 yields chunks.
        Steps 9-11 run on accumulated output after stream completes.
        
        Returns (async_generator, trace). The trace object is shared by reference:
        - Steps 1-7 traces are populated BEFORE this method returns.
        - Step 8+ traces are appended AS the generator is consumed.
        - trace.success and trace.total_duration_ms are set when generator exhausts.
        - Caller MUST NOT read trace.success until generator is fully consumed.
        """
        ...
```

---

## 9. Migration from Current Executor

### 9.1 Backward Compatibility

The current Executor constructor and call methods remain unchanged:

```python
# This still works exactly as before
executor = Executor(
    registry=registry,
    middlewares=[LoggingMiddleware(), MetricsMiddleware()],
    acl=acl,
    approval_handler=handler,
)
result = await executor.call_async("email.send", inputs, context)
```

Internally, the Executor constructs a STANDARD strategy from these parameters:

```python
def __init__(self, registry, *, strategy=None, middlewares=None, acl=None,
             config=None, approval_handler=None):
    if strategy is None:
        # Backward compat: build strategy from legacy parameters
        self._strategy = build_standard_strategy(
            registry=registry,
            config=config,
            acl=acl,
            approval_handler=approval_handler,
            middlewares=middlewares,
        )
    elif isinstance(strategy, str):
        # _resolve_strategy_name implements the 4-level lookup from §6.1:
        # 1. built-in presets → 2. code-registered → 3. YAML (Python) → 4. error
        self._strategy = self._resolve_strategy_name(strategy)
    else:
        self._strategy = strategy
```

### 9.2 Migration Path for Ecosystem Packages

| Package | Current | After | Breaking? |
|---------|---------|-------|-----------|
| apcore-mcp | `Executor(registry)` | No change (default strategy) | No |
| apcore-cli | `Executor(registry)` | No change | No |
| apcore-a2a | `Executor(registry)` | No change | No |
| fastapi-apcore | `Executor(registry)` | No change | No |
| All framework integrations | Use default constructor | No change | No |
| Custom middleware users | `Executor(registry, middlewares=[...])` | No change | No |

**Zero breaking changes for existing code.** Pipeline strategy is purely additive.

---

## 10. Cross-Language Alignment

### 10.1 Current Inconsistencies to Fix

| Issue | Python | TypeScript | Rust | Resolution |
|-------|--------|-----------|------|------------|
| Middleware before sync/async | Async-capable | Synchronous | Async | All async (Step protocol is async) |
| call() sync vs async | Sync `call()` + async `call_async()` | Async-only `call()` | Async-only `call()` | Keep both in Python for compat; TS/Rust async-only |
| Redaction key | `ctx.redacted_inputs` | `ctx.redactedInputs` | `ctx.redacted_inputs` | Steps write to PipelineContext fields. Built-in steps also sync back to Context (e.g., `context.redacted_inputs`) for backward compat with middleware that reads from Context. |
| validate() return type | `PreflightResult` | `PreflightResult` | `ValidationResult` | Keep existing `validate()` method unchanged — it returns `PreflightResult` (Python/TS) / `ValidationResult` (Rust). VALIDATE_ONLY strategy is separate (returns `(result, trace)` via `call_with_trace`). Long-term: unify return type to `PreflightResult` in all SDKs. |
| stream() | Full implementation | Full implementation | Stub (returns Vec) | Full implementation in Rust (deferred) |

### 10.2 Step Execute is Always Async

In all three languages, `Step.execute()` is async:
- Python: `async def execute(self, ctx) -> StepResult`
- TypeScript: `execute(ctx): Promise<StepResult>`
- Rust: `async fn execute(&self, ctx: &mut PipelineContext) -> Result<StepResult, ModuleError>`

Sync steps simply return immediately without awaiting anything.

---

## 11. Implementation Plan

### Phase 1: Core Infrastructure

| Step | Python | TypeScript | Rust |
|------|--------|-----------|------|
| 1.1 | Define `Step` protocol | Define `Step` interface | Define `Step` trait |
| 1.2 | Define `StepResult`, `PipelineContext`, `PipelineTrace` | Same | Same |
| 1.3 | Define `ExecutionStrategy` with insert/remove/replace | Same | Same |
| 1.4 | Implement `PipelineEngine.run()` | Same | Same |
| 1.5 | Define error types | Same | Same |

### Phase 2: Built-in Steps

| Step | All SDKs |
|------|----------|
| 2.1 | Extract current executor Step 1 (context creation) into `BuiltinContextCreation` |
| 2.2 | Extract Step 2 (safety check) into `BuiltinSafetyCheck` |
| 2.3 | Extract Step 3 (module lookup) into `BuiltinModuleLookup` |
| 2.4 | Extract Step 4 (ACL) into `BuiltinACLCheck` |
| 2.5 | Extract Step 5 (approval) into `BuiltinApprovalGate` |
| 2.6 | Extract Step 6 (input validation) into `BuiltinInputValidation` |
| 2.7 | Extract Step 7 (middleware before) into `BuiltinMiddlewareBefore` |
| 2.8 | Extract Step 8 (execute) into `BuiltinExecute` |
| 2.9 | Extract Step 9 (output validation) into `BuiltinOutputValidation` |
| 2.10 | Extract Step 10 (middleware after) into `BuiltinMiddlewareAfter` |
| 2.11 | Extract Step 11 (return result) into `BuiltinReturnResult` |

### Phase 3: Executor Refactor

| Step | All SDKs |
|------|----------|
| 3.1 | Add `strategy` parameter to Executor constructor |
| 3.2 | Build STANDARD strategy from legacy params when strategy=None |
| 3.3 | Route `call()`/`call_async()` through `PipelineEngine.run()` |
| 3.4 | Add `call_with_trace()` / `call_async_with_trace()` |
| 3.5 | Add `list_strategies()` / `describe_pipeline()` |

### Phase 4: Preset Strategies + YAML

| Step | All SDKs |
|------|----------|
| 4.1 | Implement STANDARD, INTERNAL, TESTING, VALIDATE_ONLY, PERFORMANCE presets |
| 4.2 | Load strategies from YAML (executor.strategies section) |
| 4.3 | Support strategy name in call(): `executor.call(..., strategy="internal")` |

### Phase 5: Tests

| Step | All SDKs |
|------|----------|
| 5.1 | Default strategy produces same results as current executor |
| 5.2 | Custom step insertion (11 → 13 steps) |
| 5.3 | Step removal with removable check |
| 5.3b | Duplicate step name insertion raises StepNameDuplicateError |
| 5.4 | Step replacement with replaceable check |
| 5.5 | PipelineTrace correctness |
| 5.6 | YAML strategy loading |
| 5.7 | Per-call strategy override |
| 5.8 | Backward compatibility (legacy constructor) |

---

## 12. Breaking Change Assessment

| Change | Breaking? | Migration |
|--------|-----------|-----------|
| New `Step` protocol/interface/trait | No | Pure addition |
| New `ExecutionStrategy` class | No | Pure addition |
| New `PipelineContext` / `PipelineTrace` | No | Pure addition |
| `strategy` parameter on Executor | No | Optional, defaults to None (backward compat) |
| `call_with_trace()` method | No | Pure addition |
| Executor internal refactor (11 steps → strategy) | No | Same behavior when strategy=None |
| Preset strategies (INTERNAL, TESTING, etc.) | No | Pure addition |
| YAML strategy declaration | No | Pure addition |

**Zero breaking changes.** The entire pipeline redesign is additive. Existing code continues to work without modification.

---

## 13. What This Design Does NOT Do

| Not doing | Why | Future path |
|-----------|-----|-------------|
| Remove the 11-step default pipeline | Backward compat; ecosystem depends on it | Never — it remains as STANDARD strategy |
| Force users to define strategies | Default strategy = current behavior | Strategy is opt-in |
| Built-in AI steps | AI model integration is application-level | Users implement Step protocol with their AI |
| Step-level middleware (per-step before/after) | Over-engineering; step replacement covers this | Re-evaluate if demand emerges |
| Distributed pipeline (steps across services) | Out of scope; apcore is in-process | Belongs to apflow |
| Pipeline versioning | Strategy name + step list is enough for now | Add if needed |

---

## 14. Relationship to Other Design Documents

| Document | Relationship |
|----------|-------------|
| `design-context-annotations-acl.md` | ACL condition handlers are used inside `BuiltinACLCheck` step. ContextKey is used by steps to read Tier 2 data from `context.data` (middleware/extension state). Module Annotations `extra` carries module-level metadata consumed by surface adapters — it is NOT step-specific. Step metadata belongs in Step implementation fields (name, description, removable, replaceable) and custom constructor args. |
| `PROTOCOL_SPEC.md` §7.4 | This design extends §7.4 (Executor Integration) with configurable pipeline. The spec will be updated to describe the Step protocol and ExecutionStrategy. |
| `docs/features/config-bus.md` | Strategy names can be loaded from Config Bus (executor.strategies in YAML). |
