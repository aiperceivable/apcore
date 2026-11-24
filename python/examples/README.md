# apcore-python — Examples

Runnable demos for the Python SDK. Each top-level file is a standalone script — no setup beyond `pip install -e .` (or `pip install apcore`) at the repo root.

## Quick start

```bash
# From the apcore-python repo root
python examples/simple_client.py
```

## All examples

| File | What it demonstrates | Run |
|---|---|---|
| [`simple_client.py`](simple_client.py) | Minimal `APCore()` client with `@client.module` decorator and a sync `client.call()`. | `python examples/simple_client.py` |
| [`global_client.py`](global_client.py) | Module-level `apcore.module` / `apcore.call` — no explicit client. | `python examples/global_client.py` |
| [`cancel_token.py`](cancel_token.py) | Cooperative cancellation: cancel a long-running module via `CancelToken` mid-flight. | `python examples/cancel_token.py` |
| [`pipeline_demo.py`](pipeline_demo.py) | The 11-step `ExecutionStrategy` pipeline — introspection, step-middleware tracing, and orchestration via `insert_after` / `replace`. See note below. | `python examples/pipeline_demo.py` |
| [`acl_agent_governance.py`](acl_agent_governance.py) | End-to-end AI-agent tool governance (issue #72): registers real tools, wires a default-deny ACL into `APCore`, has agents of different roles actually call the tools (allowed → real result, denied → `ACLDeniedError`), and prints the audit trail. Self-checks every decision against the cross-language contract. | `python examples/acl_agent_governance.py` |
| [`acl_config_driven.py`](acl_config_driven.py) | Config-driven ACL discovery (D-64, issue #74): declares `acl.root` in `apcore.yaml` + a default-deny `acl/global_acl.yaml`, and `APCore(config=...)` auto-wires enforcement via `ACL.discover` — no manual `set_acl`. Allowed call returns a result; denied call raises `ACLDeniedError`. Contrast with the manual path in `acl_agent_governance.py`. | `python examples/acl_config_driven.py` |
| [`approval.py`](approval.py) | Human-in-the-loop approval gate: a `requires_approval` tool, an `ApprovalHandler` that approves/rejects per request, calls that execute or raise `ApprovalDeniedError`. Companion to the ACL demo (ACL = who may call; approval = sensitive-op gate). | `python examples/approval.py` |
| [`execution_policy.py`](execution_policy.py) | Execution-time governance policy (issue #76): an external `ExecutionPolicy` forces approval on naive, already-registered modules, makes `destructive` imply approval via `gate_destructive`, and fails **closed** with `strict=True` when a gated module has no handler. Companion to `approval.py` (declared gate) — this governs modules from the outside. | `python examples/execution_policy.py` |
| [`feature_toggle.py`](feature_toggle.py) | Runtime feature toggle: `disable()` / `enable()` a tool (blocked calls raise `ModuleDisabledError`), plus per-instance `ToggleState` isolation across two `APCore` instances (issue #71). | `python examples/feature_toggle.py` |
| [`middleware.py`](middleware.py) | User-facing `use_before` / `use_after` middleware: a before hook augments inputs, an after hook transforms output, with an ordered trace proving hook order. | `python examples/middleware.py` |
| [`events.py`](events.py) | Lifecycle event bus: enable `sys_modules.events`, subscribe via `on(...)`, and observe `apcore.registry.module_registered` / `apcore.module.toggled` events as the tool is registered, called, and toggled. | `python examples/events.py` |
| [`bindings/format_date/run.py`](bindings/format_date/run.py) | Loading a YAML binding (`format_date.binding.yaml` → `format_date.py`) via `BindingLoader` and calling it through `Executor`. | `python examples/bindings/format_date/run.py` |

### Module reference files

The files under [`modules/`](modules/) are reusable module definitions kept as reference patterns. No example script imports them; their only automated consumer is [`tests/examples/test_example_modules.py`](../tests/examples/test_example_modules.py), which loads each one and exercises its `execute()`. They are not meant to be run standalone.

| File | Pattern shown |
|---|---|
| [`modules/greet.py`](modules/greet.py) | Minimal duck-typed module (`input_schema` + `output_schema` + `execute`). |
| [`modules/decorated_add.py`](modules/decorated_add.py) | The `@module` decorator from `apcore.decorator`. |
| [`modules/get_user.py`](modules/get_user.py) | Read-only module annotation. |
| [`modules/send_email.py`](modules/send_email.py) | Full-featured module: `ModuleAnnotations`, `ModuleExample`, `x-sensitive` redaction, `ContextLogger`. |

## Pipeline demo — what to look for

`pipeline_demo.py` is the deep-dive into the engine. One run prints three sections:

1. **Introspection** — the canonical 11 step names from `strategy.step_names()` / `strategy.info()`.
2. **Middleware tracing** — a `StepMiddleware` that narrates every step of one call:
   ```
   [ 1/11] context_creation    — create execution context, set global deadline
           ✓   0.07 ms · caller=anonymous trace_id=…
   ...
   [11/11] return_result       — finalize and return output
           ✓   0.01 ms · returning {…}
   ```
3. **Orchestration** — `strategy.insert_after("output_validation", AuditLogStep())` adds a 12th step (rendered as `[  +  ]` to mark it as user-inserted), then `strategy.replace("audit_log", QuietAuditLogStep())` swaps the implementation while keeping the position.

The `[N/11]` numbering stays pinned to the protocol's 11 standard steps; custom steps appear as `[  +  ]`. This makes the "11 standard + N custom" composition unmistakable in the trace output.
