# RFC: Pipeline v2 — Declarative Step Metadata

**Status:** Draft
**Authors:** apcore maintainers
**Date:** 2026-04-05
**Spec Version:** Targets 0.17.0

## 1. Motivation

### 1.1 Current State

The Executor has **two parallel implementations** of the pipeline:

- `executor.py` — Production code. `call()`, `call_async()`, `stream()` each **inline 11 steps** (~120 lines duplicated 3 times). Includes version negotiation, sensitive field redaction, middleware on_error recovery, cancel token checks, propagate_error (A11).
- `pipeline.py` + `builtin_steps.py` — Abstract framework. Step protocol, ExecutionStrategy, PipelineEngine. But builtin_steps.py is annotated: *"simplified logic sufficient for integration testing"*. Missing: redaction, on_error recovery, cancel token, version_hint, stream support.

### 1.2 Problems

**P1: Middleware transforms are never validated.**

In `executor.py` (production code), the order is: validate inputs → middleware transforms → execute. Middleware modifications ARE used for execution, but the **transformed inputs are never re-validated against the schema**. A middleware that injects an invalid field will pass through to execute unchecked.

In `builtin_steps.py` (pipeline abstraction), the problem is worse: execute prefers `ctx.validated_inputs` (set before middleware) over `ctx.inputs` (modified by middleware), so middleware modifications are **silently discarded entirely**.

Both problems are fixed by swapping the order: middleware transforms first → validation checks the transformed result. This aligns with Kubernetes: Mutating Admission → Schema Validation → Validating Admission.

**P2: validate() is hardcoded to Steps 1–7.**

`executor.py` validate() (line 441–560) manually reimplements 7 checks inline. User-added pipeline steps are never included in validation. If a user inserts a `rate_limit` step after `acl_check`, it won't execute during `validate()`.

**P3: Steps have no declarative metadata.**

Steps cannot declare:
- Which modules they apply to (all steps run for all calls)
- Whether they can be safely called during validate() (no side-effects declaration)
- Whether their failure should abort the pipeline (no ignore option)
- Per-step timeout (only global/module timeout exists)

**P4: `safety_check` naming misleads.**

The step performs call-chain safety checks (depth, cycles, repeat limits). Documentation calls it "frequency throttling," leading users to confuse it with transport-level rate limiting.

### 1.3 Validated by Real Integrations

| Integration | How it consumes the pipeline | What it needs |
|-------------|------------------------------|---------------|
| **apcore-mcp** | `ExecutionRouter` calls `executor.call_async(tool_name, arguments, context)`. Pre-validation optional via `executor.validate()`. Error mapped through `ErrorMapper.to_mcp_error()`. | validate() should cover user-defined guards (P2). MCP auth → Identity is clean; pipeline doesn't need to know about MCP. |
| **apcore-cli** | `executor.execute(moduleId, inputData)` via Sandbox wrapper. CLI does schema→flags parsing, ref resolution, STDIN merge. Approval via TTY prompt (separate from pipeline's ApprovalGate). | Pipeline step errors must map to CLI exit codes. CLI does NOT pass Context (executor creates its own). |
| **fastapi-apcore** | `FastAPIContextFactory.create_context(request)` → `executor.call(module_id, inputs, context)`. Scans FastAPI routes → registers as modules. Middleware/ACL/tracing injected via ExtensionManager. | Context factory is outside pipeline (correct boundary). Observability middleware should participate in traces. |

**Key boundary observation:** All three integrations treat the pipeline as a black box: they construct inputs + context, call the executor, and handle the result/error. The pipeline's internal steps are invisible to transport layers. **This boundary is correct and should be preserved.**

## 2. Design

### 2.1 Principles

```
1. No new abstraction layers (no Phase enum, no StepConfig wrapper)
2. No new types for 2-value concepts (bool, not enum)
3. Pipeline is a flat ordered list of steps
4. Steps declare behavior via simple fields on BaseStep
5. PipelineEngine uses declarations to make smart decisions
6. Transport layers remain unaware of pipeline internals
```

### 2.2 BaseStep — Add 4 Fields

```python
class BaseStep(ABC):
    def __init__(
        self,
        name: str,
        description: str = "",
        *,
        removable: bool = True,
        replaceable: bool = True,
        # ── New: declarative metadata ──
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

| Field | Type | Default | Meaning |
|-------|------|---------|---------|
| `match_modules` | `tuple[str, ...] \| None` | `None` | Glob patterns for module IDs this step applies to. `None` = all. |
| `ignore_errors` | `bool` | `False` | `True` = step failure logs warning and continues. `False` = step failure aborts pipeline. |
| `pure` | `bool` | `False` | `True` = no side effects. Safe to run during `validate()` (dry_run mode). |
| `timeout_ms` | `int` | `0` | Per-step timeout in milliseconds. `0` = no per-step timeout (use global deadline only). |

**Step protocol unchanged.** PipelineEngine reads new fields via `getattr()` with defaults, so third-party Step implementations that don't extend BaseStep continue to work.

### 2.3 PipelineContext — Add Fields

```python
@dataclass
class PipelineContext:
    # ── Existing ──
    module_id: str
    inputs: dict[str, Any]
    context: Any
    module: Any | None = None
    validated_inputs: dict[str, Any] | None = None
    output: dict[str, Any] | None = None
    validated_output: dict[str, Any] | None = None
    stream: bool = False
    output_stream: Any | None = None
    strategy: ExecutionStrategy | None = None
    trace: PipelineTrace | None = None

    # ── New ──
    dry_run: bool = False
    version_hint: str | None = None
    executed_middlewares: list[Any] = field(default_factory=list)
```

| Field | Purpose |
|-------|---------|
| `dry_run` | `True` during `validate()`. PipelineEngine skips steps with `pure=False`. |
| `version_hint` | Passed through to module_lookup for version negotiation. |
| `executed_middlewares` | Tracks which middleware ran, enabling on_error recovery chain. |

### 2.4 StepTrace — Add skip_reason

```python
@dataclass
class StepTrace:
    name: str
    duration_ms: float
    result: StepResult
    skipped: bool = False
    decision_point: bool = False
    skip_reason: str | None = None    # "no_match", "dry_run", "error_ignored"
```

### 2.5 Standard Pipeline — Corrected Order

```
 #  Step               pure   removable  replaceable  Notes
 1  context_creation    true   false      false        Creates/inherits Context, sets global_deadline
 2  call_chain_guard    true   true       true         RENAMED from safety_check
 3  module_lookup       true   false      false        Resolves module, applies version_hint
 4  acl_check           true   true       true         Access control
 5  approval_gate       false  true       true         May call external systems
 6  middleware_before    false  true       false        Input transformation chain
 7  input_validation    true   true       true         MOVED AFTER middleware (was Step 6)
 8  execute             false  false      true         Module execution with timeout
 9  output_validation   true   true       true         Output schema check
10  middleware_after     false  true       false        Output transformation chain
11  return_result       true   false      false        Finalize output
```

**Key change: Steps 6 and 7 swapped.** Middleware transforms inputs first, then validation checks the transformed result. Aligns with Kubernetes Mutating → Validating order.

### 2.6 PipelineEngine.run() — Enhanced Loop

```python
async def run(self, strategy, ctx):
    trace = PipelineTrace(module_id=ctx.module_id, strategy_name=strategy.name)
    start = time.monotonic()
    steps = strategy.steps
    i = 0

    while i < len(steps):
        step = steps[i]

        # ── Read declarations (getattr for backward compat) ──
        match_modules = getattr(step, "match_modules", None)
        ignore_errors = getattr(step, "ignore_errors", False)
        pure = getattr(step, "pure", False)
        timeout_ms = getattr(step, "timeout_ms", 0)

        # ① match_modules filter
        if match_modules is not None and not _any_match(match_modules, ctx.module_id):
            trace.steps.append(StepTrace(
                name=step.name, duration_ms=0,
                result=StepResult(action="continue"),
                skipped=True, skip_reason="no_match",
            ))
            i += 1
            continue

        # ② dry_run filter: skip steps with side effects
        if ctx.dry_run and not pure:
            trace.steps.append(StepTrace(
                name=step.name, duration_ms=0,
                result=StepResult(action="continue"),
                skipped=True, skip_reason="dry_run",
            ))
            i += 1
            continue

        # ③ Execute with per-step timeout
        step_start = time.monotonic()
        try:
            if timeout_ms > 0:
                result = await asyncio.wait_for(
                    step.execute(ctx), timeout=timeout_ms / 1000
                )
            else:
                result = await step.execute(ctx)
        except Exception as exc:
            duration = (time.monotonic() - step_start) * 1000
            # ④ ignore_errors: log and continue
            if ignore_errors:
                _logger.warning("Step '%s' failed (ignored): %s", step.name, exc)
                trace.steps.append(StepTrace(
                    name=step.name, duration_ms=duration,
                    result=StepResult(action="continue", explanation=str(exc)),
                    skip_reason="error_ignored",
                ))
                i += 1
                continue
            # Not ignored: record and raise
            trace.steps.append(StepTrace(
                name=step.name, duration_ms=duration,
                result=StepResult(action="abort", explanation=str(exc)),
            ))
            trace.total_duration_ms = (time.monotonic() - start) * 1000
            raise

        # ⑤ Record trace
        duration = (time.monotonic() - step_start) * 1000
        trace.steps.append(StepTrace(
            name=step.name, duration_ms=duration,
            result=result,
            decision_point=result.confidence is not None,
        ))

        # ⑥ Handle abort / skip_to (unchanged from current)
        if result.action == "abort":
            trace.total_duration_ms = (time.monotonic() - start) * 1000
            raise PipelineAbortError(step=step.name, explanation=result.explanation,
                                      alternatives=result.alternatives, trace=trace)
        elif result.action == "skip_to":
            # ... existing skip_to logic unchanged ...
            pass

        i += 1

    trace.success = True
    trace.total_duration_ms = (time.monotonic() - start) * 1000
    return (ctx.validated_output or ctx.output), trace
```

### 2.7 validate() — Use dry_run

Replace the 120-line hardcoded validate() in executor.py with:

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

User-added steps with `pure=True` automatically participate. Steps with `pure=False` (approval_gate, middleware, execute) are automatically skipped.

### 2.8 Data Flow Through Steps

```
ctx.inputs (raw, from caller)
    │
    ├─ Steps 1-5: READ-ONLY (don't modify inputs)
    │   context_creation:  writes ctx.context
    │   call_chain_guard:  reads ctx.context.call_chain
    │   module_lookup:     writes ctx.module
    │   acl_check:         reads ctx.context.caller_id
    │   approval_gate:     reads/writes ctx.inputs (_approval_token removal)
    │
    ├─ Step 6: middleware_before
    │   WRITES ctx.inputs (middleware chain modifies in place)
    │   WRITES ctx.executed_middlewares (for on_error recovery)
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

### 2.9 Builtin Step Declarations

```python
# context_creation
pure=True, removable=False, replaceable=False

# call_chain_guard (renamed from safety_check)
pure=True, removable=True, replaceable=True

# module_lookup
pure=True, removable=False, replaceable=False

# acl_check
pure=True, removable=True, replaceable=True

# approval_gate
pure=False, removable=True, replaceable=True
# pure=False because may call external approval system

# middleware_before
pure=False, removable=True, replaceable=False
# pure=False because middleware may have side effects
# replaceable=False to protect onion model

# input_validation
pure=True, removable=True, replaceable=True

# execute
pure=False, removable=False, replaceable=True
# replaceable=True for sandbox/remote executors

# output_validation
pure=True, removable=True, replaceable=True

# middleware_after
pure=False, removable=True, replaceable=False

# return_result
pure=True, removable=False, replaceable=False
```

### 2.10 Open Design Points

The following require detailed design during implementation:

**stream() mode:**
PipelineEngine.run() executes steps sequentially. But stream() needs to yield chunks mid-execution (during Step 8). Design options:

- **Option A (recommended):** Pipeline runs Steps 1-7. BuiltinExecute in stream mode returns `StepResult(action="continue")` and sets `ctx.output_stream`. Executor.stream() then iterates `ctx.output_stream`, accumulating chunks. After stream completes, executor runs Steps 9-11 via a **second mini-strategy** (output_validation + middleware_after + return_result). This splits the pipeline into two runs for stream mode.
- **Option B:** BuiltinExecute blocks until all chunks are accumulated, then returns. Simpler but no incremental streaming — defeats the purpose.

**validate() → PreflightResult mapping:**
Current PreflightResult uses check names ("module_id", "call_chain", "acl", "schema") that differ from pipeline step names ("module_lookup", "call_chain_guard", "acl_check", "input_validation"). The `_trace_to_checks()` function must maintain a mapping table. Additionally, "module_id" format validation and "module_preflight" have no corresponding pipeline steps and must be handled by existing builtin steps (module_lookup validates format before lookup; input_validation runs module.preflight() if available).

**Error propagation (A11):**
Current executor wraps all exceptions with `propagate_error(module_id, ctx)`. In the pipeline-driven model, this wrapping should happen in PipelineEngine.run() catch block — wrapping any exception that escapes the pipeline into a ModuleError with trace context attached.

**match_modules pattern syntax:**
Uses `apcore.utils.match_pattern()` (Algorithm A09) — the same glob matching used by ACL. Patterns: `"api.*"` matches `api.users.list`, `"*.create"` matches `data.create`, `"*"` matches all.

**TypeScript step name prefix:**
TypeScript builtin steps currently use `builtin.` prefix (e.g., `builtin.context_creation`). After rename, Python/Rust use `call_chain_guard`, TypeScript uses `builtin.call_chain_guard`. This prefix should be **removed** in v0.17 for cross-SDK consistency. TypeScript YAML configs and strategy code referencing `builtin.*` names will need migration.

## 3. Boundary Clarification

### 3.1 What Pipeline Handles

| Concern | Pipeline step | Notes |
|---------|--------------|-------|
| Context creation | context_creation | Identity comes from caller |
| Call chain safety | call_chain_guard | Depth, cycles, repeat |
| Module resolution | module_lookup | Version negotiation via hint |
| Access control | acl_check | Default-deny ACL |
| Human approval | approval_gate | External handler protocol |
| Input transformation | middleware_before | User middleware chain |
| Input validation | input_validation | Schema validation (Pydantic) |
| Execution | execute | With timeout |
| Output validation | output_validation | Schema validation |
| Output transformation | middleware_after | User middleware chain |
| Error propagation | PipelineEngine | A11 wrapping |
| Sensitive redaction | input/output_validation | x-sensitive fields |

### 3.2 What Pipeline Does NOT Handle

| Concern | Who handles it | Why not pipeline |
|---------|---------------|-----------------|
| HTTP rate limiting | Transport (apcore-mcp, apcore-cli) | Connection-level, not module-level |
| Authentication | Transport (JWT, API key, OAuth) | Protocol-specific |
| Request routing | Transport (MCP tool→module mapping) | Protocol-specific |
| Database transactions | Module (execute() internal) | Business logic |
| External API calls | Module (execute() internal) | Business logic |
| Retry strategy | RetryMiddleware or Module | Cross-cutting or business |
| Caching (result) | Middleware or user step (skip_to) | Application-specific |
| Message queues | Module internal | Infrastructure |

### 3.3 Integration Contract

Transport layers interact with the pipeline through exactly **one interface**:

```python
# This is the ONLY thing transport layers call:
result = await executor.call_async(module_id, inputs, context)

# Or for preflight:
preflight = executor.validate(module_id, inputs, context)

# Or for streaming:
async for chunk in executor.stream(module_id, inputs, context):
    ...
```

Transport layers construct:
- `module_id` — from protocol mapping (MCP tool name, CLI command, FastAPI route)
- `inputs` — from protocol parsing (MCP arguments, CLI flags+stdin, HTTP body)
- `context` — from protocol auth (JWT→Identity, request.state.user→Identity, or None)

Transport layers handle:
- Protocol-specific error formatting (MCP ErrorContent, CLI exit codes, HTTP status)
- Protocol-specific output formatting (MCP TextContent, CLI table/JSON, HTTP response)

**Pipeline internals are invisible to transport layers. This is correct and preserved.**

## 4. Migration

### 4.1 Breaking Changes

| Change | Impact | Migration |
|--------|--------|-----------|
| `safety_check` → `call_chain_guard` | Code referencing step name | Find/replace in insert_before/after/remove calls |
| Step 6/7 order swap | Middleware modifications now validated | Correct behavior; middleware that relied on pre-validation input should be reviewed |

### 4.2 Implementation — All in v0.17.0

All changes ship in a single version bump across all 3 SDKs. One breaking change, one migration. Implementation order within v0.17:

**PR 1: Pipeline framework enhancement (Python)**
- BaseStep + 4 new fields (match_modules, ignore_errors, pure, timeout_ms)
- PipelineContext + dry_run, version_hint, executed_middlewares
- PipelineEngine + match/dry_run/timeout/ignore logic
- Builtin steps production parity: redaction, on_error recovery, cancel_token, version_hint
- Rename safety_check → call_chain_guard
- Swap middleware_before ↔ input_validation order
- Tests for all new behavior

**PR 2: Executor → Pipeline delegation (Python)**
- executor.call() delegates to PipelineEngine.run(strategy, ctx)
- executor.call_async() same
- executor.stream() — Steps 1-7 via pipeline, streaming loop in executor, Steps 9-11 via pipeline
- executor.validate() — PipelineEngine.run() with dry_run=True
- Delete ~360 lines of duplicated inline step code

**PR 3: YAML pipeline configuration (Python)**
- `register_step_type()` API + step factory registry
- `build_strategy_from_config()` — startup-time YAML → Strategy builder
- YAML `pipeline` section: remove, configure, steps
- Step resolution: `type` (registry) + `handler` (import path)

**PR 4: Spec + documentation**
- PROTOCOL_SPEC.md: Update pipeline order, add step metadata spec, add pipeline YAML config
- docs/features/core-executor.md: Update pipeline description
- docs/api/executor-api.md: Update step order and new fields

**PR 5: TypeScript SDK sync**
- Step interface + 4 optional fields
- PipelineEngine enhancement
- Rename, order swap, builtin step parity
- YAML pipeline config with `handler` (dynamic import) + `type` (registry)

**PR 6: Rust SDK sync**
- Step trait + 4 default methods
- PipelineEngine enhancement
- Rename, order swap, builtin step parity
- YAML pipeline config with `type` (registry only)

## 5. User Extension Examples

### 5.1 Security: IP Whitelist (only for admin modules)

```python
class IPWhitelistStep(BaseStep):
    def __init__(self, allowed_ips):
        super().__init__(
            name="ip_whitelist",
            match_modules=("admin.*",),
            pure=True,           # included in validate()
        )
        self._allowed_ips = set(allowed_ips)

    async def execute(self, ctx):
        ip = ctx.context.identity.attrs.get("ip")
        if ip not in self._allowed_ips:
            return StepResult(action="abort", explanation=f"IP {ip} not allowed")
        return StepResult(action="continue")

strategy.insert_after("acl_check", IPWhitelistStep(["10.0.0.0/8"]))
```

### 5.2 Optimization: Cache Hit

```python
class CacheCheckStep(BaseStep):
    def __init__(self, cache):
        super().__init__(name="cache_check", pure=True)
        self._cache = cache

    async def execute(self, ctx):
        cached = self._cache.get(ctx.module_id, ctx.inputs)
        if cached is not None:
            ctx.output = cached
            ctx.validated_output = cached
            return StepResult(action="skip_to", skip_to="return_result")
        return StepResult(action="continue")

strategy.insert_after("acl_check", CacheCheckStep(my_cache))
```

### 5.3 Compliance: Audit Log (failure tolerant)

```python
strategy.insert_before("return_result", BaseStep(
    name="audit_log",
    ignore_errors=True,      # audit failure doesn't block result
    pure=False,              # not included in validate()
))
```

### 5.4 Input Enrichment (before validation)

```python
class DefaultInjector(BaseStep):
    def __init__(self):
        super().__init__(
            name="inject_defaults",
            match_modules=("*.create",),
            pure=True,
        )

    async def execute(self, ctx):
        ctx.inputs.setdefault("created_by", ctx.context.identity.id)
        ctx.inputs.setdefault("created_at", datetime.utcnow().isoformat())
        return StepResult(action="continue")

# Insert BETWEEN middleware_before and input_validation
strategy.insert_before("input_validation", DefaultInjector())
```

This works correctly because input_validation is now AFTER middleware_before.

## 6. YAML Pipeline Configuration

### 6.1 Motivation

Same codebase, different pipeline per environment — without code changes:

```yaml
# prod.yaml
pipeline:
  steps:
    - name: rate_limit
      type: rate_limit
      after: acl_check
      match_modules: ["api.*"]
      config: { max_per_minute: 100 }

# dev.yaml
pipeline:
  remove: [acl_check, approval_gate]
```

### 6.2 Design: Startup Loading (Not Runtime)

Pipeline YAML is loaded once at startup, same as Config:

```
App start → Config.load("apcore.yaml") → build strategy from YAML → App run
                                          ↑ strategy is immutable after this
```

No runtime dynamic loading. No hot-reload of pipeline steps.

### 6.3 Step Resolution: Two Mechanisms, Each Language's Strength

Custom steps are resolved via two mechanisms, both at **startup time** (not runtime):

| Field | Mechanism | Python | TypeScript | Rust |
|-------|-----------|--------|------------|------|
| `handler` | Import class by module path | ✓ MUST (`importlib`) | ✓ MUST (`import()`) | ✗ N/A |
| `type` | Look up pre-registered factory by name | ✓ MUST | ✓ MUST | ✓ MUST |

**Resolution order:** `type` first → `handler` fallback → error.

**`handler` — language-native import (Python/TypeScript):**

The natural pattern for dynamic languages. No pre-registration needed. Consistent with each ecosystem's conventions:

```python
# Python: handler path works like Django MIDDLEWARE setting
# YAML: handler: myapp.steps:RateLimitStep
# Framework calls: importlib.import_module("myapp.steps").RateLimitStep
# Zero extra code needed from the user.
```

```typescript
// TypeScript: handler path works like NestJS provider import
// YAML: handler: ./steps/rate-limit.js:RateLimitStep
// Framework calls: (await import("./steps/rate-limit.js")).RateLimitStep
// Zero extra code needed from the user.
```

This reuses existing infrastructure:
- Python: `bindings.py` `resolve_target()` (line 129, importlib-based)
- TypeScript: `bindings.ts` `resolveTarget()` (line 125, ESM dynamic import)

**`type` — registry lookup (all SDKs, required for Rust):**

The natural pattern for compiled languages. Type-safe, explicit. Also useful for cross-language teams sharing YAML config:

```rust
// Rust: register at init (the only option — Rust has no runtime import)
pipeline::register_step_type("rate_limit", |config| {
    Box::new(RateLimitStep::from_config(config))
});
// Follows existing Rust SDK patterns:
// - register_subscriber_type() in events
// - register_condition() in ACL
```

```python
# Python: type registry also available (optional, for cross-language portability)
register_step_type("rate_limit", RateLimitStep)
```

**When Rust encounters `handler` without `type`:**

```
Error: Pipeline step "rate_limit" has 'handler' but no 'type'.
       Rust SDK requires 'type' for step resolution.
       Register with: pipeline::register_step_type("rate_limit", factory_fn)
```

**Spec conformance levels:**

```
handler field:  Python MUST, TypeScript MUST, Rust MAY (clear error if unsupported)
type field:     All SDKs MUST support
```

**Cross-language team pattern (both fields):**

```yaml
# Shared apcore.yaml — works in both Python and Rust services
pipeline:
  steps:
    - name: rate_limit
      type: rate_limit                        # Rust uses this
      handler: myapp.steps:RateLimitStep      # Python uses this (if type not registered)
      after: acl_check
      config:
        max_per_minute: 100
```

### 6.4 YAML Schema

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
      type: rate_limit                            # primary: lookup registered factory
      handler: myapp.steps:RateLimitStep          # fallback: import path (Python/TS)
      after: acl_check                            # position: insert_after("acl_check")
      match_modules: ["api.*"]
      pure: true
      ignore_errors: false
      timeout_ms: 3000
      config:                                     # passed to factory/constructor
        max_per_minute: 100

    - name: audit_log
      type: audit_log
      before: return_result                       # position: insert_before("return_result")
      ignore_errors: true
      config:
        log_path: /var/log/apcore/audit.jsonl
```

### 6.5 Loading Flow (All SDKs)

```
1. Config.load("apcore.yaml")
   └─ pipeline_config = config.get("pipeline")

2. build_strategy_from_config(pipeline_config, step_registry)
   ├─ Start with build_standard_strategy(...)
   ├─ Process "remove": strategy.remove(name) for each
   ├─ Process "configure": update step fields (timeout_ms, ignore_errors, etc.)
   └─ Process "steps":
       ├─ Resolve step: step_registry.get(type) or import(handler)
       ├─ Instantiate with config dict
       ├─ Set metadata fields (match_modules, pure, ignore_errors, timeout_ms)
       └─ Insert: strategy.insert_after(after) or strategy.insert_before(before)

3. Executor(strategy=strategy)
   └─ Strategy is immutable after construction
```

### 6.6 Cross-Language Verification

| Capability | Python | TypeScript | Rust |
|------------|--------|------------|------|
| Config accepts `pipeline` key | ✓ dict passthrough | ✓ `registerNamespace` | ✓ `serde(flatten)` |
| `handler` import (language-native) | ✓ `bindings.py` `importlib` | ✓ `bindings.ts` `import()` | ✗ N/A |
| `type` registry (cross-language) | ✓ `ACL.register_condition` | ✓ `ACL.registerCondition` | ✓ `register_subscriber_type` |
| Startup-time loading | ✓ same as Config | ✓ same as Config | ✓ same as Config |
| Per-step timeout | ✓ `asyncio.wait_for` | ✓ `Promise.race` | ✓ `tokio::time::timeout` |
| Step backward compat | ✓ `getattr` | ✓ optional fields | ✓ trait defaults |

### 6.7 Decision Summary

| Question | Answer |
|----------|--------|
| Why not unify to `type` only? | Forces Python/TS users to pre-register (anti-pattern in those ecosystems: Django, Flask, NestJS all use import paths) |
| Why not unify to `handler` only? | Rust has no runtime import — `type` registry is the idiomatic Rust pattern |
| Why support both? | Each language uses its natural pattern; cross-language teams use both fields in shared YAML |
| Is this inconsistent? | No — the YAML format is the same; resolution strategy adapts to language capabilities |

## 7. Middleware Relationship

Middleware and Pipeline Steps coexist. They are NOT redundant:

| | Pipeline Step | Middleware |
|-|--------------|-----------|
| **Use case** | Single-point logic at a specific position | Paired before/after/on_error wrapping execution |
| **Example** | ACL check, cache lookup, audit log | Logging, tracing, metrics, retry |
| **Lifecycle** | Independent steps, no pairing guarantee | before+after+on_error in one class |
| **Error recovery** | PipelineEngine abort/ignore | Dedicated on_error chain with recovery |
| **Integration** | BuiltinMiddlewareBefore/After are pipeline steps that invoke the middleware chain | Middleware runs inside these two pipeline steps |

Middleware is **not replaced** by pipeline enhancements. The two systems serve different purposes.

## 8. Non-Goals

- **No Phase/category system.** Steps are a flat ordered list. Users position with insert_before/insert_after.
- **No Webhook steps.** External service calls belong in Module.execute() or Middleware, not pipeline infrastructure.
- **No transport-level concerns.** Rate limiting, authentication, request routing stay in transport layers.
- **No database transaction management.** Module.execute() owns its resources.
