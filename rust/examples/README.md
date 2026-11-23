# apcore-rust — Examples

Runnable demos for the Rust SDK. Examples follow Cargo's convention: every top-level `examples/*.rs` file is auto-registered and runnable via `cargo run --example <name>`.

## Quick start

```bash
# From the apcore-rust repo root
cargo run --example simple_client
```

## All examples

| File | What it demonstrates | Run |
|---|---|---|
| [`simple_client.rs`](simple_client.rs) | Implement the `Module` trait directly and call `module.execute()` with a `Context`. | `cargo run --example simple_client` |
| [`global_client.rs`](global_client.rs) | Use the global `APCore` registry — register and call without an explicit client variable. | `cargo run --example global_client` |
| [`decorated_add.rs`](decorated_add.rs) | The `FunctionModule` adapter — turn a plain async fn into a registered module. | `cargo run --example decorated_add` |
| [`greet.rs`](greet.rs) | Minimal module: input/output schemas + `execute`. | `cargo run --example greet` |
| [`get_user.rs`](get_user.rs) | Read-only module annotation. | `cargo run --example get_user` |
| [`send_email.rs`](send_email.rs) | Full-featured module: `ModuleAnnotations`, `ModuleExample`, sensitive-field redaction. | `cargo run --example send_email` |
| [`cancel_token.rs`](cancel_token.rs) | Cooperative cancellation: cancel a long-running module via `CancelToken`. | `cargo run --example cancel_token` |
| [`pipeline_demo.rs`](pipeline_demo.rs) | The 11-step `ExecutionStrategy` pipeline — introspection, step-middleware tracing, and orchestration via `insert_after` / `replace`. See note below. | `cargo run --example pipeline_demo` |
| [`acl_agent_governance.rs`](acl_agent_governance.rs) | End-to-end AI-agent tool governance (issue #72): registers real tools, wires a default-deny ACL into the `Executor`, has agents of different roles actually call the tools (allowed → real result, denied → `ACLDenied`), and prints the audit trail. Self-checks every decision against the cross-language contract. | `cargo run --example acl_agent_governance` |
| [`acl_config_driven.rs`](acl_config_driven.rs) | Config-driven ACL discovery (D-64, issue #74): writes an `apcore.yaml` with `acl: { root: ./acl }` and a co-located `acl/global_acl.yaml`, then `APCore::from_path` auto-discovers and enforces it — no manual `set_acl` (contrast with `acl_agent_governance.rs`). An `@external` greet call is allowed, a vault call is blocked by default-deny (`ACLDenied`). | `cargo run --example acl_config_driven` |
| [`approval.rs`](approval.rs) | Human-in-the-loop approval gate: a `requires_approval` tool, an `ApprovalHandler` that approves/rejects per request, calls that execute or fail with `ErrorCode::ApprovalDenied`. Companion to the ACL demo (ACL = who may call; approval = sensitive-op gate). | `cargo run --example approval` |
| [`feature_toggle.rs`](feature_toggle.rs) | Runtime feature toggle: `disable()` / `enable()` a tool (blocked calls fail with `ErrorCode::ModuleDisabled`), plus per-instance `ToggleState` isolation across two `APCore` instances (issue #71). | `cargo run --example feature_toggle` |
| [`middleware.rs`](middleware.rs) | User-facing `use_before` / `use_after` middleware: a before hook augments inputs, an after hook transforms output, with an ordered trace proving hook order. | `cargo run --example middleware` |
| [`events.rs`](events.rs) | Lifecycle event bus: subscribe via `on(...)` and observe `apcore.registry.module_registered` / `apcore.module.toggled` events (see the in-file note on the Rust local-emitter vs sys-bus split). | `cargo run --example events` |

### Bindings

The [`bindings/`](bindings/) directory shows the YAML-binding pattern:

| File | Role |
|---|---|
| [`bindings/format_date.binding.yaml`](bindings/format_date.binding.yaml) | Canonical binding definition. |
| [`bindings/format_date.rs`](bindings/format_date.rs) | Target function loaded by the binding. |

Because this file lives in a sub-directory, Cargo does not auto-register it as an example. This repository already declares the required `[[example]]` target in `Cargo.toml`, so run it with `cargo run --example format_date`; copy the loader pattern into your own program when adapting it.

## Pipeline demo — what to look for

`pipeline_demo.rs` is the deep-dive into the engine. One run prints three sections:

1. **Introspection** — the canonical 11 step names from `strategy.step_names()` / `strategy.info()`.
2. **Middleware tracing** — a `StepMiddleware` that narrates every step of one call:
   ```
   [ 1/11] context_creation    — create execution context, set global deadline
           ✓   0.16 ms · caller=anonymous trace_id=…
   ...
   [11/11] return_result       — finalize and return output
           ✓   0.00 ms · returning {…}
   ```
3. **Orchestration** — `strategy.insert_after("output_validation", Box::new(AuditLogStep))?` adds a 12th step (rendered as `[  +  ]` to mark it as user-inserted), then `strategy.replace("audit_log", Box::new(QuietAuditLogStep))?` swaps the implementation while keeping the position.

The `[N/11]` numbering stays pinned to the protocol's 11 standard steps; custom steps appear as `[  +  ]`. This makes the "11 standard + N custom" composition unmistakable in the trace output.
