---
description: "The configurable execution pipeline: the Step protocol, the 11 built-in steps, preset and custom strategies, apcore.yaml pipeline config, step middleware, run_until, dry-run validate() and tracing."
---

# Execution Pipeline

<!-- preamble-tier-doc -->
> **Type:** Feature reference. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md#516-pipeline-control-flow-requirements) §5.16 Pipeline Control Flow Requirements, §12.8 `validate()`, §7.4 approval gate, §6.6.3.2 and §6.6.5 governance state. Declarative configuration: [protocol-spec §5.16.1](../spec/protocol-spec.md#5161-declarative-pipeline-configuration-pipeline).

## Overview

Every `Executor.call()` runs through an **execution strategy**: a named, ordered list of
**steps**. A `PipelineEngine` walks the list, hands each step a shared `PipelineContext`, and
records a `PipelineTrace`. The standard strategy has eleven built-in steps; you can remove,
insert, replace or reconfigure steps in code or from `apcore.yaml`, select a preset strategy by
name, and observe each call step by step.

The strategy is the only thing that runs. An ACL or `ApprovalHandler` attached to the executor
does nothing unless the strategy contains the step that consults it — see
[Preset strategies](#preset-strategies).

## Core types

### Step

A step is one unit of work. Configuration reaches a step through its constructor; the protocol
itself only fixes these members:

| Member | Default | Meaning |
|---|---|---|
| `name` | required | Unique within a strategy. Anchors, `skip_to` and configuration address steps by name. |
| `description` | required | Human- and AI-readable purpose. |
| `removable` | required | `false` = `remove()` raises `StepNotRemovableError`. |
| `replaceable` | required | `false` = `replace()` / `configure_step()` raise `StepNotReplaceableError`. |
| `match_modules` | none (all modules) | Module-ID patterns (Algorithm A08: `*` only); the step is skipped for other modules. |
| `ignore_errors` | `false` | A failure is logged and the pipeline continues. |
| `pure` | `false` | Side-effect free. Only pure steps run during `validate()`. |
| `timeout_ms` | `0` | Per-step timeout; `0` = none. A timeout is an ordinary step failure. |
| `requires` / `provides` | empty | Capability tokens (for example `module`, `output`). Checked when the strategy is built. |
| `execute(ctx)` | required | Async. Reads and writes the `PipelineContext`, returns a `StepResult`. |

Per language:

- **Python** — `Step` is a runtime-checkable `Protocol`; subclass `BaseStep(name, description="", *, removable=True, replaceable=True, match_modules=None, ignore_errors=False, pure=False, timeout_ms=0, requires=(), provides=())` and implement `async def execute(self, ctx)`. `match_modules`, `requires` and `provides` are tuples.
- **TypeScript** — `Step` is an interface: `name`, `description`, `removable`, `replaceable`, the optional `matchModules`, `ignoreErrors`, `pure`, `timeoutMs`, `requires`, `provides`, and `execute(ctx): Promise<StepResult>`.
- **Rust** — `Step` is an `#[async_trait]` trait. `name`, `description`, `removable`, `replaceable` and `execute(&self, ctx: &mut PipelineContext) -> Result<StepResult, ModuleError>` are required; `match_modules`, `ignore_errors`, `pure`, `timeout_ms`, `requires`, `provides` and `builtin_gate` have defaults. `builtin_gate()` returns `Some(BuiltinGate::Acl | BuiltinGate::Approval)` only on the framework's own gates, which is how `governance_state()` recognises them by type rather than by name.

### StepResult

`StepResult` controls flow; it does not carry data between steps.

| Field | Meaning |
|---|---|
| `action` | `"continue"`, `"skip_to"` or `"abort"`. |
| `skip_to` | Target step name for `skip_to`. It must come later in the strategy. |
| `explanation` | Reason, surfaced in the trace and in the abort error. |
| `confidence` | Optional score in `[0, 1]`; a trace entry whose result sets it is marked `decision_point`. |
| `alternatives` | Optional suggestions carried on an abort. |

Python constructs `StepResult(action="continue")`, TypeScript returns an object literal
`{ action: 'continue' }`, and Rust uses `StepResult::continue_step()`, `StepResult::abort(explanation)`
and `StepResult::skip_to(target)`.

### PipelineContext

The mutable state every step sees. Fields set by a step are empty until that step has run, so a
custom step must check before reading a field that a later step fills.

| Field (Python / Rust; TypeScript is camelCase) | Set by |
|---|---|
| `module_id`, `inputs`, `context` | the caller; `inputs` may be rewritten by `approval_gate` and `middleware_before` |
| `module` | `module_lookup` (step 3) |
| `governance_projection` | `module_lookup`, read by `acl_check` |
| `acl_approval_required` | `acl_check`, read by `approval_gate` and `validate()` |
| `validated_inputs` | `input_validation` (step 7) |
| `output` | `execute` (step 8), possibly rewritten by `middleware_after` (step 10) |
| `validated_output` | `output_validation` (step 9) |
| `dry_run` | `validate()` sets it; impure steps are skipped |
| `version_hint` | the caller; used by `module_lookup` |
| `executed_middlewares` | `middleware_before`, for the `on_error` recovery chain |
| `trace` | the engine |

Language differences: Python and TypeScript also carry `stream` / `output_stream` (set by the
streaming path) and a `run_until` predicate. Rust carries neither; it passes a predicate through
`RunOptions`, and it carries the executor's resources — `registry`, `config`, `acl`,
`approval_handler`, `policy`, `middleware_manager` and others — which the executor injects before
each run. That is why Rust built-in steps are unit structs with no constructor arguments.

#### Two-tier data model

| Tier | Storage | Written by | Read through |
|---|---|---|---|
| 1 | `PipelineContext` fields | built-in steps | direct field access (`ctx.module`, `ctx.output`) |
| 2 | `context.data` | middleware and custom steps | a typed `ContextKey` |

Pipeline-essential data stays in Tier 1 and is not copied into `context.data`. Extension state
belongs in Tier 2; see [Context Object](./context-object.md).

### ExecutionStrategy

An ordered list of steps with a name. Construction and every insertion check two invariants:

- **Unique names.** A duplicate raises `StepNameDuplicateError`. Step lookups use a name-to-index
  map, never a scan (§5.16 requirement 2).
- **Satisfied dependencies.** Each step's `requires` must be covered by the `provides` of the
  steps before it, or construction raises `PipelineDependencyError` naming the step and the
  missing tokens. `remove()` does not re-run this check.

Python's constructor takes `validate_dependencies=False` and TypeScript's takes
`{ seedProvides: [...] }` for sub-strategies whose inputs are already populated; the streaming
path uses them for its post-stream segment.

### PipelineTrace

`PipelineTrace` has `module_id`, `strategy_name`, `steps` (a list of `StepTrace`),
`total_duration_ms` and `success`. Each `StepTrace` has `name`, `duration_ms`, `result`,
`skipped`, `decision_point` and `skip_reason`:

| `skip_reason` | Meaning |
|---|---|
| `"no_match"` | `match_modules` did not match the module ID. |
| `"dry_run"` | Impure step skipped during `validate()`. |
| `"error_ignored"` | The step failed and `ignore_errors` let the pipeline continue (not marked `skipped`). |
| none, with `skipped: true` | Passed over by a `skip_to`. |

A trace is process-local. `call_with_trace()` returns it; a `PipelineStepError` or
`PipelineAbortError` carries the trace up to the failure (`pipeline_trace` / `pipelineTrace`).

## Built-in steps

### The standard pipeline

| # | Step | Core | pure | removable | replaceable | What it does |
|---|---|---|---|---|---|---|
| 1 | `context_creation` | yes | yes | no | no | Create or inherit the execution context; set the global deadline. |
| 2 | `call_chain_guard` | | yes | yes | yes | Depth, circular-call and repeat limits ([Call Chain Guard](./call-chain-guard.md)). |
| 3 | `module_lookup` | yes | yes | no | no | Resolve the module (honouring `version_hint` and disabled toggles). |
| 4 | `acl_check` | | yes | yes | yes | Evaluate the ACL, if one is attached ([ACL System](./acl-system.md)). |
| 5 | `approval_gate` | | no | yes | yes | Request approval when governance requires it ([Approval System](./approval-system.md)). |
| 6 | `middleware_before` | | no | yes | no | Run the module-level `before` chain; may rewrite `inputs`. |
| 7 | `input_validation` | | yes | yes | yes | Validate the (possibly rewritten) inputs; record redacted inputs. |
| 8 | `execute` | yes | no | no | yes | Invoke the module under the per-module timeout and global deadline. |
| 9 | `output_validation` | | yes | yes | yes | Validate the output; record redacted output. |
| 10 | `middleware_after` | | no | yes | no | Run the module-level `after` chain; may rewrite `output`. |
| 11 | `return_result` | yes | yes | no | no | Finalise the output. |

The four **core** steps cannot be removed; every strategy contains them. The other seven are
optional.

- `middleware_before` runs **before** `input_validation`: middleware transforms first, then the
  transformed inputs are validated.
- The two middleware steps are removable but not replaceable: they run the executor's middleware
  chain, and `executor.use()` has no other way in. Removing them (the `performance` preset) means
  registered module middleware never runs. To add behaviour next to them, insert a step.
- `approval_gate`, the middleware steps and `execute` are impure, so `validate()` never runs them.

!!! warning "Replacing `execute`"
    `replace("execute", step)` drops the built-in timeout enforcement, cancel-token check,
    global-deadline clamp and streaming detection. A replacement that needs them must
    re-implement them.

### Data flow

```text
ctx.inputs (from the caller)
  │
  ├─ 1 context_creation   writes ctx.context
  ├─ 2 call_chain_guard   reads the call chain
  ├─ 3 module_lookup      writes ctx.module, ctx.governance_projection
  ├─ 4 acl_check          reads caller_id; writes ctx.acl_approval_required
  ├─ 5 approval_gate      removes _approval_token from ctx.inputs
  ├─ 6 middleware_before  rewrites ctx.inputs; writes ctx.executed_middlewares
  ├─ 7 input_validation   writes ctx.validated_inputs, context.redacted_inputs
  ├─ 8 execute            reads ctx.validated_inputs; writes ctx.output
  ├─ 9 output_validation  writes ctx.validated_output, context.redacted_output
  ├─ 10 middleware_after  rewrites ctx.output
  └─ 11 return_result     the engine returns ctx.output
```

### Middleware or a custom step?

| | Module middleware | Custom step |
|---|---|---|
| Position | Fixed, inside steps 6 and 10 | Anywhere, via `insert_before` / `insert_after` |
| Shape | Paired `before` / `after` / `on_error` around the call | One `execute(ctx)` |
| Scope | Per executor (`executor.use()`) | Per strategy |
| Stops a call by | Raising | Returning `abort`, or raising |
| Traced per step | No | Yes |

Use middleware for concerns that wrap the whole call (logging, retry, metrics); use a step for a
gate or transform at a specific position (rate limit, tenant check, cache short-circuit).

## Strategies

### Preset strategies

All three SDKs ship the same five presets, each a factory (`build_*_strategy` in Python and Rust,
`build*Strategy` in TypeScript), because built-in steps need runtime dependencies:

| Preset | Steps | Removed from `standard` | `acl_check` | `approval_gate` |
|---|---|---|---|---|
| `standard` | 11 | — | present | present |
| `internal` | 9 | `acl_check`, `approval_gate` | **removed** | **removed** |
| `testing` | 8 | `acl_check`, `approval_gate`, `call_chain_guard` | **removed** | **removed** |
| `performance` | 9 | `middleware_before`, `middleware_after` | present | present |
| `minimal` | 4 | all seven optional steps | **removed** | **removed** |

`internal`, `testing` and `minimal` run no ACL and no approval gate **even when an ACL or
`ApprovalHandler` is attached to the executor**. Check what is actually gating a registry with
`governance_state()` ([§6.6.5](../spec/protocol-spec.md#665-governance-state-query)), not by
testing whether an ACL object exists
([§6.6.3.2](../spec/protocol-spec.md#6632-a-configured-layer-is-not-necessarily-an-enforced-one)).

=== "Python"
    ```python
    from apcore import Executor, Registry

    executor = Executor(Registry(), strategy="internal")
    print(executor.describe_pipeline())
    # 9-step pipeline: context_creation → call_chain_guard → module_lookup → middleware_before → ...
    print(executor.governance_state().builtin_acl_gate_wired)  # False
    ```

=== "TypeScript"
    ```typescript
    import { Executor, Registry } from 'apcore-js';

    const executor = new Executor({ registry: new Registry(), strategy: 'internal' });
    console.log(executor.describePipeline().stepNames.join(' → '));
    // context_creation → call_chain_guard → module_lookup → middleware_before → ...
    console.log(executor.governanceState().builtinAclGateWired); // false
    ```

=== "Rust"
    ```rust
    use apcore::errors::ModuleError;
    use apcore::{Config, Executor, Registry};

    fn main() -> Result<(), ModuleError> {
        let executor = Executor::with_strategy_name(Registry::new(), Config::default(), "internal")?;
        println!("{}", executor.describe_pipeline());
        // 9-step pipeline: context_creation → call_chain_guard → module_lookup → middleware_before → ...
        println!("{}", executor.governance_state().builtin_acl_gate_wired); // false
        Ok(())
    }
    ```

### Selecting a strategy

| | Python | TypeScript | Rust |
|---|---|---|---|
| At construction | `Executor(registry, strategy=<ExecutionStrategy \| name>)` | `new Executor({ registry, strategy })` | `Executor::with_strategy(registry, config, strategy)`, `Executor::with_strategy_name(registry, config, name)` |
| Current strategy | `executor.current_strategy` | `executor.currentStrategy` | `executor.strategy()` |
| Per call | `call_with_trace(..., strategy=<strategy \| name>)`, `call_async_with_trace` | `callWithTrace(id, inputs, ctx, { strategy })` | `call_with_trace(id, inputs, ctx, version_hint, Some(&strategy))` |
| Register a name | `Executor.register_strategy(name, strategy)` | `Executor.registerStrategy(name, strategy)` | `register_strategy(info)`, `executor::register_strategy_by_name(name, &strategy)` — introspection only |

`call()` itself takes no strategy argument in any SDK. With no explicit strategy, the executor
builds `standard`, or the strategy the loaded configuration's `pipeline:` section describes; an
explicit strategy wins over configuration (D-73).

Name resolution differs by SDK. Python resolves preset names first, then registered names;
TypeScript resolves registered names first, so registering `standard` shadows the preset; Rust
resolves preset names only, and its strategy registry holds `StrategyInfo` for `list_strategies()`.
A registered strategy is shared by reference between every executor that selects it. An unknown
name raises `StrategyNotFoundError` (Python, TypeScript) or `GENERAL_INVALID_INPUT` (Rust).

!!! warning "A strategy instance brings its own dependencies (Python, TypeScript)"
    Python and TypeScript built-in steps capture the registry, ACL, approval handler and
    middleware manager when they are **constructed**. A strategy you build and pass in keeps
    those; the executor's own `acl`, `approval_handler` and `middlewares` arguments do not reach
    its steps, although `governance_state()` still reports the ACL as configured and the gate as
    wired. Prefer modifying `executor.current_strategy` / `executor.currentStrategy`, select presets
    by name, or call `set_acl()` / `setAcl()` after construction. Rust injects the executor's
    resources into every run, so a Rust strategy is wired by whichever executor runs it.

### Adding a custom step

A step that aborts `billing.*` calls whose caller has no `tenant` attribute. It is `pure`, so
`validate()` runs it too.

=== "Python"
    ```python
    from apcore import APCore, BaseStep, Context, Identity, PipelineContext, StepResult


    class TenantGuard(BaseStep):
        """Abort billing.* calls whose caller carries no tenant attribute."""

        def __init__(self) -> None:
            super().__init__(
                name="tenant_guard",
                description="Require a tenant attribute on billing.* callers",
                match_modules=("billing.*",),
                pure=True,  # side-effect free, so validate() runs it too
                requires=("context",),
            )

        async def execute(self, ctx: PipelineContext) -> StepResult:
            identity = ctx.context.identity
            if identity is None or "tenant" not in identity.attrs:
                return StepResult(action="abort", explanation="caller has no tenant attribute")
            return StepResult(action="continue")


    client = APCore()


    @client.module(id="billing.invoice", description="Create an invoice")
    def invoice(amount: int) -> dict:
        return {"amount": amount}


    # Mutate the executor's own strategy: its built-in steps are already wired
    # to the executor's registry, ACL, approval handler and middleware.
    client.executor.current_strategy.insert_after("acl_check", TenantGuard())

    ctx = Context.create(identity=Identity(id="u1", attrs={"tenant": "acme"}))
    result, trace = client.executor.call_with_trace("billing.invoice", {"amount": 5}, ctx)
    print(result)  # {'amount': 5}
    print(trace.strategy_name, [s.name for s in trace.steps if not s.skipped])
    ```

=== "TypeScript"
    ```typescript
    import { Type } from '@sinclair/typebox';
    import { APCore, Context, createIdentity } from 'apcore-js';
    import type { PipelineContext, Step, StepResult } from 'apcore-js';

    /** Abort billing.* calls whose caller carries no tenant attribute. */
    class TenantGuard implements Step {
      readonly name = 'tenant_guard';
      readonly description = 'Require a tenant attribute on billing.* callers';
      readonly removable = true;
      readonly replaceable = true;
      readonly matchModules = ['billing.*'];
      readonly pure = true; // side-effect free, so validate() runs it too
      readonly requires = ['context'];

      async execute(ctx: PipelineContext): Promise<StepResult> {
        const identity = ctx.context.identity;
        if (identity === null || !('tenant' in identity.attrs)) {
          return { action: 'abort', explanation: 'caller has no tenant attribute' };
        }
        return { action: 'continue' };
      }
    }

    const client = new APCore();
    client.module({
      id: 'billing.invoice',
      description: 'Create an invoice',
      inputSchema: Type.Object({ amount: Type.Integer() }),
      outputSchema: Type.Object({ amount: Type.Integer() }),
      execute: (inputs) => ({ amount: inputs.amount as number }),
    });

    // Mutate the executor's own strategy: its built-in steps are already wired
    // to the executor's registry, ACL, approval handler and middleware.
    client.executor.currentStrategy.insertAfter('acl_check', new TenantGuard());

    const ctx = Context.create(createIdentity('u1', 'user', [], { tenant: 'acme' }));
    const [result, trace] = await client.executor.callWithTrace('billing.invoice', { amount: 5 }, ctx);
    console.log(result); // { amount: 5 }
    console.log(trace.strategyName, trace.steps.filter((s) => !s.skipped).map((s) => s.name));
    ```

=== "Rust"
    ```rust
    use std::collections::HashMap;
    use std::sync::Arc;

    use apcore::context::{Context, Identity};
    use apcore::errors::ModuleError;
    use apcore::module::Module;
    use apcore::pipeline::{PipelineContext, Step, StepResult};
    use apcore::registry::registry::Registry;
    use apcore::{build_standard_strategy, Config, Executor};
    use async_trait::async_trait;
    use serde_json::{json, Value};

    struct Invoice;

    #[async_trait]
    impl Module for Invoice {
        fn input_schema(&self) -> Value {
            json!({"type": "object", "properties": {"amount": {"type": "integer"}}, "required": ["amount"]})
        }
        fn output_schema(&self) -> Value {
            json!({"type": "object", "properties": {"amount": {"type": "integer"}}})
        }
        fn description(&self) -> &str {
            "Create an invoice"
        }
        async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
            Ok(json!({"amount": inputs["amount"]}))
        }
    }

    /// Abort billing.* calls whose caller carries no tenant attribute.
    struct TenantGuard {
        match_modules: Vec<String>,
    }

    #[async_trait]
    impl Step for TenantGuard {
        fn name(&self) -> &str {
            "tenant_guard"
        }
        fn description(&self) -> &str {
            "Require a tenant attribute on billing.* callers"
        }
        fn removable(&self) -> bool {
            true
        }
        fn replaceable(&self) -> bool {
            true
        }
        fn match_modules(&self) -> Option<&[String]> {
            Some(&self.match_modules)
        }
        fn pure(&self) -> bool {
            true // side-effect free, so validate() runs it too
        }
        fn requires(&self) -> &[&str] {
            &["context"]
        }
        async fn execute(&self, ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
            let has_tenant = ctx
                .context
                .identity
                .as_ref()
                .is_some_and(|identity| identity.attrs().contains_key("tenant"));
            if has_tenant {
                Ok(StepResult::continue_step())
            } else {
                Ok(StepResult::abort("caller has no tenant attribute"))
            }
        }
    }

    #[tokio::main]
    async fn main() -> Result<(), ModuleError> {
        let registry = Arc::new(Registry::new());
        registry.register_module("billing.invoice", Box::new(Invoice))?;

        // Rust built-in steps read the registry, ACL, approval handler and
        // middleware from the PipelineContext the executor fills in, so a
        // strategy built here is fully wired by the Executor that runs it.
        let mut strategy = build_standard_strategy();
        strategy.insert_after(
            "acl_check",
            Box::new(TenantGuard { match_modules: vec!["billing.*".into()] }),
        )?;
        let executor = Executor::with_strategy(registry, Config::default(), strategy);

        let attrs = HashMap::from([("tenant".to_string(), json!("acme"))]);
        let ctx = Context::new(Identity::new("u1".into(), "user".into(), vec![], attrs));
        let (result, trace) = executor
            .call_with_trace("billing.invoice", json!({"amount": 5}), Some(&ctx), None, None)
            .await?;
        println!("{result}"); // {"amount":5}
        let ran: Vec<&str> = trace.steps.iter().filter(|s| !s.skipped).map(|s| s.name.as_str()).collect();
        println!("{} {ran:?}", trace.strategy_name);
        Ok(())
    }
    ```

When a custom step returns `abort`, the caller receives an error with code `PIPELINE_ABORT`
whose message carries the explanation. Aborts from the built-in steps are translated to their
typed errors (`ACL_DENIED`, `MODULE_NOT_FOUND`, schema validation errors) instead.

### Modifying a strategy

| Operation | Python | TypeScript | Rust |
|---|---|---|---|
| Insert after / before | `insert_after(anchor, step)`, `insert_before(anchor, step)` | `insertAfter`, `insertBefore` | `insert_after(anchor, Box<dyn Step>)`, `insert_before` → `Result` |
| Remove | `remove(name)` | `remove(name)` | `remove(name)` |
| Replace | `replace(name, step)` | `replace(name, step)` | `replace(name, step)` |
| Replace in place (idempotent) | `configure_step(name, step)` | `configureStep(name, step)` | `configure_step(name, step)` |
| Wrap the existing step | — | — | `replace_with(name, \|old\| new)` |
| Step names / summary | `step_names()`, `info()` | `stepNames()`, `info()` | `step_names()`, `info()` |
| Step middleware | `add_step_middleware(mw)` | on `PipelineEngine` — see [Step middleware](#step-middleware) | `add_step_middleware(Arc<dyn StepMiddleware>)` |

- `insert_*` raise `StepNameDuplicateError` for a taken name and `StepNotFoundError` for a
  missing anchor, then re-check dependencies.
- `remove()` raises `StepNotFoundError` or `StepNotRemovableError`.
- `replace()` and `configure_step()` keep the step's position and raise
  `StepNotReplaceableError` for a non-replaceable target; a missing target is
  `StepNotFoundError` for `replace()` and `PipelineStepNotFoundError` for `configure_step()`.
  Calling `configure_step()` twice leaves exactly one step under that name (§5.16 requirement 3).
  A replacement that takes a name held by a different step raises `StepNameDuplicateError`
  (TypeScript checks this in `configureStep()` only).
- Rust's `replace_with` hands the current step to a closure and does not check `replaceable`; the
  configuration loader uses it to overlay `configure:` fields.

Finish modifying a strategy before the first call; strategies are not synchronised for
concurrent mutation.

## Configuring the pipeline from apcore.yaml

When the loaded configuration carries a `pipeline:` section and no explicit strategy was given,
the executor builds its strategy from it (§5.16 requirement 6). The section is
`schemas/apcore-config.schema.json` `$defs/PipelineConfig`, with three keys, applied in this
order:

```yaml
pipeline:
  remove: [approval_gate]          # 1. remove built-in steps
  configure:                       # 2. override fields of existing steps
    call_chain_guard:
      timeout_ms: 2000
  steps:                           # 3. insert custom steps
    - name: rate_limit
      type: rate_limit             # a registered step type
      after: acl_check             # or before: <step>
      match_modules: ["api.*"]
      pure: true
      config:
        max_per_minute: 100        # constructor arguments for the factory
```

- **`configure`** accepts exactly `match_modules`, `ignore_errors`, `pure` and `timeout_ms`, on
  any step present, core steps included. Any other key — `requires`, `provides`, `name`, `type`,
  `handler`, `after`, `before`, `config` — fails the load with `PIPELINE_CONFIGURATION_ERROR`
  naming every offending key. The step keeps its position.
- **`steps`** entries accept `name` (required), `type`, `handler`, `config`, `match_modules`,
  `ignore_errors`, `pure`, `timeout_ms`, `after` and `before`. An unknown key, a missing anchor, or
  an entry with neither `after` nor `before` fails the load.
- **`remove`** of a step that is not present fails the load. Removing `acl_check` or
  `approval_gate` is allowed and logs a warning once per load (§5.16 requirement 7).
- The optional limits `validation.pipeline.step_name_max_length` and
  `validation.pipeline.timeout_ms_max` (unset by default) bound the names and timeouts the section
  declares ([§9.1.2](../spec/protocol-spec.md#912-declarative-validation-limits-validation)).

!!! danger "`configure` can switch a security gate off without a warning"
    `configure: {acl_check: {ignore_errors: true}}` turns an ACL denial into a logged warning and
    the call executes; `match_modules` on `acl_check` or `approval_gate` exempts every module it
    does not match. Only `remove` triggers the security-step warning. Treat `configure` entries for
    those two steps with the same care as removing them.

### Resolving custom steps

A `type` names a factory registered in code before the executor is built. `handler` is a
`"module:Name"` import path resolved at load time in Python and TypeScript.

=== "Python"
    ```python
    from apcore import APCore, BaseStep, Config, PipelineContext, StepResult, register_step_type


    class RateLimitStep(BaseStep):
        def __init__(self, max_per_minute: int = 60) -> None:
            super().__init__(name="rate_limit", description="Per-minute call budget")
            self.max_per_minute = max_per_minute

        async def execute(self, ctx: PipelineContext) -> StepResult:
            return StepResult(action="continue")


    # Register before the Executor is built: the pipeline section is applied then.
    register_step_type("rate_limit", RateLimitStep)  # a BaseStep class, or a callable(config) -> step

    client = APCore(config=Config.load("apcore.yaml"))
    print(client.executor.describe_pipeline())
    # 11-step pipeline: context_creation → call_chain_guard → module_lookup → acl_check → rate_limit → ...
    ```

=== "TypeScript"
    ```typescript
    import { APCore, Config, registerStepType } from 'apcore-js';
    import type { PipelineContext, Step, StepResult } from 'apcore-js';

    class RateLimitStep implements Step {
      readonly name = 'rate_limit';
      readonly description = 'Per-minute call budget';
      readonly removable = true;
      readonly replaceable = true;
      readonly maxPerMinute: number;

      constructor(maxPerMinute: number) {
        this.maxPerMinute = maxPerMinute;
      }

      async execute(_ctx: PipelineContext): Promise<StepResult> {
        return { action: 'continue' };
      }
    }

    // Register before the Executor is built: the pipeline section is applied then.
    registerStepType('rate_limit', (config) => new RateLimitStep(Number(config.max_per_minute ?? 60)));

    const client = new APCore({ config: Config.load('apcore.yaml') });
    console.log(client.executor.describePipeline().stepNames.join(' → '));
    // context_creation → call_chain_guard → module_lookup → acl_check → rate_limit → ...
    ```

=== "Rust"
    ```rust
    use std::path::Path;

    use apcore::errors::ModuleError;
    use apcore::pipeline::{PipelineContext, Step, StepResult};
    use apcore::{register_step_type, APCore, Config};
    use async_trait::async_trait;

    struct RateLimitStep {
        max_per_minute: u64,
    }

    #[async_trait]
    impl Step for RateLimitStep {
        fn name(&self) -> &str {
            "rate_limit"
        }
        fn description(&self) -> &str {
            "Per-minute call budget"
        }
        fn removable(&self) -> bool {
            true
        }
        fn replaceable(&self) -> bool {
            true
        }
        async fn execute(&self, _ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
            let _budget = self.max_per_minute;
            Ok(StepResult::continue_step())
        }
    }

    fn main() -> Result<(), ModuleError> {
        // Register before the Executor is built: the pipeline section is applied then.
        register_step_type(
            "rate_limit",
            Box::new(|config| {
                let max_per_minute = config["max_per_minute"].as_u64().unwrap_or(60);
                Ok(Box::new(RateLimitStep { max_per_minute }) as Box<dyn Step>)
            }),
        )?;

        let client = APCore::with_config(Config::load(Path::new("apcore.yaml"))?);
        println!("{}", client.executor().describe_pipeline());
        // 11-step pipeline: context_creation → call_chain_guard → module_lookup → acl_check → rate_limit → ...
        Ok(())
    }
    ```

The entry's `name`, `match_modules`, `ignore_errors`, `pure` and `timeout_ms` override what the
factory's step declares. An omitted metadata field keeps the step's own value in TypeScript but
resets to the default (all modules, `false`, `0`) in Python and Rust, so declare every field the
step relies on. What each SDK does with the resolution fields:

| Entry | Python | TypeScript | Rust |
|---|---|---|---|
| `type` only | registry lookup | registry lookup | registry lookup |
| `handler` only | `importlib` import; the target is called with `config` as keyword arguments | ESM `import()`; the export is called as `factory(config)`. **Not inserted** when the `Executor` constructor applies the section — it warns and skips the step; build with `await buildStrategyFromConfig()` and pass the strategy explicitly | `PIPELINE_HANDLER_NOT_SUPPORTED` |
| `type` and `handler` | `type` if registered, else `handler` | `type` if registered, else `handler` | `PIPELINE_HANDLER_NOT_SUPPORTED` |
| `after` and `before` | `after` wins | `after` wins | `after` wins |

The schema requires exactly one of `type` / `handler` and exactly one of `after` / `before`
([protocol-spec §5.16.1](../spec/protocol-spec.md#5161-declarative-pipeline-configuration-pipeline)); write
entries that way so the same file loads identically in every SDK.

When the section fails to build, Python and TypeScript raise from the `Executor` constructor. The
Rust constructors that apply the section (`Executor::new`, `Executor::with_options`) are
infallible: they log the error and run the **standard** pipeline,
so a declared custom step does not run. For fail-fast behaviour in Rust, build the strategy with
`apcore::pipeline_config::build_strategy_from_config_with_limits(&section, Some(&config))` and pass
it to `Executor::with_strategy`.

## The engine loop

For each step in order, `PipelineEngine` does the following:

1. **`match_modules`** — no match: record `skip_reason: "no_match"`, next step.
2. **Dry run** — `dry_run` set and the step is impure: record `skip_reason: "dry_run"`, next step.
3. **Step middleware `before_step`**, in registration order ([Step middleware](#step-middleware)).
4. **Execute**, under `timeout_ms` when it is non-zero.
5. **On failure** — step middleware `on_step_error` may supply a recovery value, which becomes
   `output` and the pipeline continues; otherwise, with `ignore_errors`, log a warning, record
   `skip_reason: "error_ignored"` and continue; otherwise stop and raise `PipelineStepError`
   carrying the step name and the original error (§5.16 requirement 1). A
   `MiddlewareChainError` propagates as itself.
6. **Step middleware `after_step`**, in reverse registration order, then record the step's output.
7. **Act on the result** — `abort` stops the pipeline with `PipelineAbortError` (Rust returns the
   trace with `success: false`, which the executor turns into `PIPELINE_ABORT`); `skip_to` jumps to
   the named later step, recording the steps in between as skipped, or raises `StepNotFoundError`
   when the target is not later in the strategy.
8. **`run_until`** — after a clean `continue`, evaluate the predicate; `true` ends the run.

The engine returns `ctx.output` and the trace. Callers of `Executor.call()` never see
`PipelineStepError`: the executor unwraps it to the original typed error before middleware
`on_error` recovery and error propagation run ([Core Executor](./core-executor.md)).

### run_until

A `run_until` predicate receives a `PipelineState` — `step_name`, `outputs` (step name → that
step's output, **including** the step that just completed) and `context` — after each step, and
returning `true` skips all remaining steps and returns what has accumulated (§5.16 requirement 4).
It is an engine option, not an `Executor.call()` option: Python passes it to
`PipelineEngine().run(strategy, ctx, run_until=...)` or sets `PipelineContext.run_until`,
TypeScript sets `runUntil` on the `PipelineContext`, and Rust calls
`PipelineEngine::run_until(&strategy, &mut ctx, predicate)` or `run_with_options` with
`RunOptions::run_until(predicate)`.

=== "Python"
    ```python
    import asyncio

    from apcore import APCore, Context, PipelineContext, PipelineEngine, PipelineState

    client = APCore()


    @client.module(id="math.add", description="Add two integers")
    def add(a: int, b: int) -> dict:
        return {"sum": a + b}


    def after_validation(state: PipelineState) -> bool:
        # Evaluated after each step completes; True stops the pipeline there.
        return state.step_name == "input_validation"


    async def main() -> None:
        ctx = PipelineContext(module_id="math.add", inputs={"a": 1, "b": 2}, context=Context.create())
        output, trace = await PipelineEngine().run(
            client.executor.current_strategy, ctx, run_until=after_validation
        )
        print(output, [s.name for s in trace.steps if not s.skipped])
        # None ['context_creation', ..., 'input_validation'] — execute never ran


    asyncio.run(main())
    ```

=== "TypeScript"
    ```typescript
    import { Type } from '@sinclair/typebox';
    import { APCore, Context, PipelineEngine } from 'apcore-js';
    import type { PipelineContext, PipelineState } from 'apcore-js';

    const client = new APCore();
    client.module({
      id: 'math.add',
      description: 'Add two integers',
      inputSchema: Type.Object({ a: Type.Integer(), b: Type.Integer() }),
      outputSchema: Type.Object({ sum: Type.Integer() }),
      execute: (inputs) => ({ sum: (inputs.a as number) + (inputs.b as number) }),
    });

    const ctx: PipelineContext = {
      moduleId: 'math.add',
      inputs: { a: 1, b: 2 },
      context: Context.create(),
      // Evaluated after each step completes; true stops the pipeline there.
      runUntil: (state: PipelineState) => state.stepName === 'input_validation',
    };
    const [output, trace] = await new PipelineEngine().run(client.executor.currentStrategy, ctx);
    console.log(output, trace.steps.filter((s) => !s.skipped).map((s) => s.name));
    // null [ 'context_creation', ..., 'input_validation' ] — execute never ran
    ```

=== "Rust"
    ```rust
    use std::sync::Arc;

    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::module::Module;
    use apcore::pipeline::{PipelineContext, PipelineEngine, PipelineState};
    use apcore::registry::registry::Registry;
    use apcore::{build_standard_strategy, Config};
    use async_trait::async_trait;
    use serde_json::{json, Value};

    struct Add;

    #[async_trait]
    impl Module for Add {
        fn input_schema(&self) -> Value {
            json!({"type": "object", "properties": {"a": {"type": "integer"}, "b": {"type": "integer"}}, "required": ["a", "b"]})
        }
        fn output_schema(&self) -> Value {
            json!({"type": "object", "properties": {"sum": {"type": "integer"}}})
        }
        fn description(&self) -> &str {
            "Add two integers"
        }
        async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
            Ok(json!({"sum": inputs["a"].as_i64().unwrap_or(0) + inputs["b"].as_i64().unwrap_or(0)}))
        }
    }

    #[tokio::main]
    async fn main() -> Result<(), ModuleError> {
        let registry = Arc::new(Registry::new());
        registry.register_module("math.add", Box::new(Add))?;
        let strategy = build_standard_strategy();

        // Outside an Executor, supply the resources the built-in steps read.
        let mut ctx = PipelineContext::new("math.add", json!({"a": 1, "b": 2}), Context::anonymous(), strategy.name());
        ctx.registry = Some(registry);
        ctx.config = Some(Arc::new(Config::default()));

        // Evaluated after each step completes; true stops the pipeline there.
        let (output, trace) = PipelineEngine::run_until(&strategy, &mut ctx, |state: &PipelineState| {
            state.step_name == "input_validation"
        })
        .await?;
        let ran: Vec<&str> = trace.steps.iter().filter(|s| !s.skipped).map(|s| s.name.as_str()).collect();
        println!("{output:?} {ran:?}"); // None [.., "input_validation"] — execute never ran
        Ok(())
    }
    ```

## Step middleware

A `StepMiddleware` wraps **every** step of a strategy with three optional hooks —
`before_step(step_name, state)`, `after_step(step_name, state, result)` and
`on_step_error(step_name, state, error)` — and filters on `step_name` itself. `before_step` runs in
registration order and the other two in reverse (onion). `before_step` and `after_step` only
observe; a non-null value from `on_step_error` recovers a failed step body. A `before_step`
failure is terminal: it raises `MiddlewareChainError` and no recovery applies. The full contract is
in [Middleware System § Pipeline Step Middleware](./middleware-system.md).

**Ordering against module middleware** (§5.16 requirement 5). The module-level chain runs inside
steps 6 and 10, so for the steps it encloses — `input_validation`, `execute` and
`output_validation` — the order is: module `before` → step `before_step` → step body → step
`after_step` → module `after`. Steps 1–5 and 11, and the two middleware steps themselves, are
wrapped by step middleware alone.

Where to attach it:

- **Python** — `executor.current_strategy.add_step_middleware(mw)`, which takes effect on the next call.
- **Rust** — `strategy.add_step_middleware(Arc::new(mw))` on a strategy you then pass to
  `Executor::with_strategy`; `Executor::strategy()` hands out only a shared reference.
- **TypeScript** — `PipelineEngine.addStepMiddleware(mw)`. The executor's engine is private, so
  step middleware applies only to runs of an engine you drive yourself, as in the
  [run_until](#run_until) example.

Complete examples in all three languages are in
[Middleware System § Pipeline Step Middleware](./middleware-system.md).

There is no `pipeline.step_middleware` configuration key; step middleware is registered in code.

## validate() and dry runs

`Executor.validate()` runs the executor's strategy with `dry_run` set
([§12.8](../spec/protocol-spec.md#128-executorvalidate-cross-language-implementation-guide)).
The engine skips every impure step — `approval_gate`, both middleware steps, `execute` — and runs
every pure one, built-in or custom. Steps 1–4 and 7 therefore run as checks, a pure custom step
such as `tenant_guard` above runs too, and an impure one is skipped. No module code and no
middleware runs.

Each pure step that ran becomes one entry in `PreflightResult.checks`. Built-in steps map to the
check names `context` (`context_creation`), `call_chain`, `module_lookup`, `acl` (`acl_check`)
and `schema` (`input_validation`); `output_validation` and `return_result` also run, and pass,
because there is no output yet; a custom step's check carries its step name. A `module_id` format
check runs before the pipeline. The engine is still fail-fast in a dry run, so steps after a
failing one report nothing.

- **Approval is reported, not requested.** `approval_gate` does not run and the `ApprovalHandler`
  is never invoked. `requires_approval` is computed from the same governance union the gate
  enforces: the module annotation and registry metadata, the `ExecutionPolicy`, and the ACL rule's
  `approval: required` ([§7.4](../spec/protocol-spec.md#74-executor-integration-step-5)).
- **Module introspection needs ACL permission.** The module's `preflight()` and `preview()` run
  only when the `acl` check did not fail; after a denial there is no `module_preflight` or
  `module_preview` check and `predicted_changes` is empty
  ([§12.8.5.1](../spec/protocol-spec.md#12851-module-level-preflight-check-7)).
- **`pure` is a promise.** A step marked `pure` runs during every `validate()`. Mark a step pure
  only if running it without executing the module has no side effects.

## Streaming through the pipeline

`Executor.stream()` runs the same strategy in three phases, fully implemented in all three SDKs:

1. **Before the module** — steps 1–7 run as for `call()`; any failure is raised before a chunk
   is produced. Python and TypeScript run the strategy with `stream` set so the `execute` step
   captures the module's stream; Rust runs the strategy up to `execute` and calls the module's
   `stream()` itself.
2. **Chunks** — each chunk is yielded as it arrives and deep-merged into an accumulated output.
3. **After the stream** — the strategy's `output_validation`, `middleware_after` and
   `return_result` steps (whichever are present) run once on the merged output. Chunks have
   already been delivered, so a failure here is not raised: it is logged and emitted as an
   `apcore.stream.post_validation_failed` event.

A module without `stream()` yields a single chunk produced by the full pipeline, with ordinary
error behaviour. See [Streaming](./streaming.md).

## Introspection

| | Python | TypeScript | Rust |
|---|---|---|---|
| Current pipeline | `describe_pipeline()` → `StrategyInfo` | `describePipeline()` | `describe_pipeline()` |
| Current + registered | `list_strategies()` | `listStrategies()` | `list_strategies()` |
| One strategy | `strategy.info()` | `strategy.info()` | `strategy.info()` |
| What is gating calls | `governance_state()` | `governanceState()` | `governance_state()` |

`list_strategies()` returns the executor's current strategy first, then each registered strategy,
de-duplicated by name (Python and TypeScript in name order, Rust in registration order).

### StrategyInfo

`StrategyInfo` carries `name`, `step_count`, `step_names` and `description`. Python's `str()` and
Rust's `Display` render `"N-step pipeline: a → b → …"`; TypeScript returns a plain object. The
`description` joins step names in Python and TypeScript and `name: description` pairs in Rust.

`StrategyInfo` carries **names only**. A custom step named `acl_check` looks identical to the
real gate, so never decide whether a registry is protected from it; `governance_state()` detects
the built-in gates by type
([§6.6.5.2](../spec/protocol-spec.md#6652-gate-detection-must-be-by-type-not-by-name)).

## Errors

| Python / TypeScript | Rust `ErrorCode` | Code | Raised when |
|---|---|---|---|
| `PipelineStepError` | `PipelineStepError` | `PIPELINE_STEP_ERROR` | A step failed and nothing recovered or ignored it. Carries the step name and cause; `Executor` unwraps it. |
| `PipelineAbortError` | `PipelineAbort` | `PIPELINE_ABORT` | A step returned `abort`. |
| `StepNotFoundError` | `StepNotFound` | `STEP_NOT_FOUND` | Missing anchor or target for `insert_*`, `remove`, `replace`; a `skip_to` target that is not later. |
| `StepNotRemovableError` | `StepNotRemovable` | `STEP_NOT_REMOVABLE` | `remove()` on a core step. |
| `StepNotReplaceableError` | `StepNotReplaceable` | `STEP_NOT_REPLACEABLE` | `replace()` / `configure_step()` on a non-replaceable step. |
| `StepNameDuplicateError` | `StepNameDuplicate` | `STEP_NAME_DUPLICATE` | A step name already in the strategy. |
| `PipelineStepNotFoundError` | `PipelineStepNotFound` | `PIPELINE_STEP_NOT_FOUND` | `configure_step()` on a missing step. |
| `PipelineDependencyError` | `PipelineDependencyError` | `PIPELINE_DEPENDENCY_ERROR` | A `requires` token no earlier step provides. |
| `StrategyNotFoundError` | `StrategyNotFound` | `STRATEGY_NOT_FOUND` | Unknown strategy name (Rust `with_strategy_name` reports `GENERAL_INVALID_INPUT`). |
| `ConfigurationError` | `PipelineConfigurationError` | `PIPELINE_CONFIGURATION_ERROR` | A structural error in the `pipeline:` section. |
| — | `PipelineHandlerNotSupported` | `PIPELINE_HANDLER_NOT_SUPPORTED` | A `handler:` step entry in Rust. |

An unregistered `type` also fails the load: Python raises `ValueError`, TypeScript
`ConfigurationError`, Rust `GENERAL_INVALID_INPUT`. See [Error System](./error-system.md).

## What the pipeline does not handle

| Concern | Where it belongs |
|---|---|
| Transport authentication, HTTP rate limiting, request routing | The adapter (apcore-mcp, apcore-cli, web frameworks), which builds the `module_id`, `inputs` and `Context` it passes to `call()`, `validate()` or `stream()` |
| Database transactions, external API calls | The module's `execute()` |
| Retries | `RetryMiddleware` or the module |
| Distribution of steps across processes | Out of scope; a strategy runs in one process |

## See also

- [Core Executor](./core-executor.md) — `call()`, `call_with_trace()`, timeouts, error propagation
- [Middleware System](./middleware-system.md) — module middleware and the `StepMiddleware` contract
- [Approval System](./approval-system.md) and [ACL System](./acl-system.md) — the two gate steps
- [Config Bus](./config-bus.md) — loading `apcore.yaml`
- [PROTOCOL_SPEC §5.16](../spec/protocol-spec.md#516-pipeline-control-flow-requirements) — the normative pipeline requirements
