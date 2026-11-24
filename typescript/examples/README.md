# apcore-typescript — Examples

Runnable demos for the TypeScript SDK. Each top-level file is standalone.

## Quick start

Every example imports the package by its public name (`apcore-js`), which
self-resolves to `./dist/index.js`. `dist/` is gitignored and there is no
`prepare` build, so a fresh clone needs one build first:

```bash
# From the apcore-typescript repo root
pnpm install
pnpm build          # required — examples import 'apcore-js' -> ./dist
node examples/simple-client.ts
```

> **Node version:** these examples use top-level `await` and import a `.ts` file directly via Node's native TypeScript support (Node 22.6+ with `--experimental-strip-types`, or Node 23+ where it's the default). On older Node, use `npx tsx examples/<file>.ts`.

## All examples

| File | What it demonstrates | Run |
|---|---|---|
| [`simple-client.ts`](simple-client.ts) | Minimal `new APCore()` client with `client.module(...)` and `await client.call(...)`. | `node examples/simple-client.ts` |
| [`global-client.ts`](global-client.ts) | Module-level client pattern — minimal boilerplate. | `node examples/global-client.ts` |
| [`cancel-token.ts`](cancel-token.ts) | Cooperative cancellation: cancel a long-running module via `CancelToken`. | `node examples/cancel-token.ts` |
| [`pipeline-demo.ts`](pipeline-demo.ts) | The 11-step `ExecutionStrategy` pipeline — introspection, step-middleware tracing, and orchestration via `insertAfter` / `replace`. See note below. | `node examples/pipeline-demo.ts` |
| [`acl-agent-governance.ts`](acl-agent-governance.ts) | End-to-end AI-agent tool governance (issue #72): registers real tools, wires a default-deny ACL into `APCore`, has agents of different roles actually call the tools (allowed → real result, denied → `ACLDeniedError`), and prints the audit trail. Self-checks every decision against the cross-language contract. | `node examples/acl-agent-governance.ts` |
| [`acl-config-driven.ts`](acl-config-driven.ts) | Config-driven ACL discovery (D-64 / issue #74): writes a throwaway `apcore.yaml` with `acl.root` + a co-located policy file, then `new APCore({ config })` discovers and attaches the ACL **automatically** — no `setAcl`. Shows an allowed `@external` call and a denied inter-module call. The config-driven counterpart to `acl-agent-governance.ts` (manual `setAcl`). | `node examples/acl-config-driven.ts` |
| [`approval.ts`](approval.ts) | Human-in-the-loop approval gate: a `requiresApproval` tool, an `ApprovalHandler` that approves/rejects per request, calls that execute or throw `ApprovalDeniedError`. Companion to the ACL demo (ACL = who may call; approval = sensitive-op gate). | `node examples/approval.ts` |
| [`execution-policy.ts`](execution-policy.ts) | Execution-time governance policy (issue #76): an external `ExecutionPolicy` forces approval on naive, already-registered modules, makes `destructive` imply approval via `gateDestructive`, and fails **closed** with `strict: true` when a gated module has no handler. Companion to `approval.ts` (declared gate) — this governs modules from the outside. | `node examples/execution-policy.ts` |
| [`feature-toggle.ts`](feature-toggle.ts) | Runtime feature toggle: `disable()` / `enable()` a tool (blocked calls throw `ModuleDisabledError`), plus per-instance `ToggleState` isolation across two `APCore` instances (issue #71). | `node examples/feature-toggle.ts` |
| [`middleware.ts`](middleware.ts) | User-facing `useBefore` / `useAfter` middleware: a before hook augments inputs, an after hook transforms output, with an ordered trace proving hook order. | `node examples/middleware.ts` |
| [`events.ts`](events.ts) | Lifecycle event bus: enable `sys_modules.events`, subscribe via `on(...)`, and observe `apcore.registry.module_registered` / `apcore.module.toggled` events as the tool is registered, called, and toggled. | `node examples/events.ts` |
| [`v022-tour.ts`](v022-tour.ts) | Tour of the six v0.22.0 surfaces in one script: `ContextKey<T>`, `StreamingModule`, middleware duplicate detection, event retry + DLQ, registry async deferred-publish, and the reserved-namespace query API. | `node examples/v022-tour.ts` |

### Module reference files

The files under [`modules/`](modules/) are reusable module definitions, not standalone scripts.

| File | Pattern shown |
|---|---|
| [`modules/greet.ts`](modules/greet.ts) | Minimal module with TypeBox schemas. |
| [`modules/decorated-add.ts`](modules/decorated-add.ts) | The `module()` factory function. |
| [`modules/get-user.ts`](modules/get-user.ts) | Read-only module annotation. |
| [`modules/send-email.ts`](modules/send-email.ts) | Full-featured module: `createAnnotations()`, `ModuleExample`, `x-sensitive` field redaction, `ContextLogger`. |

> Annotations are built with `createAnnotations({ ... })`, not a bare object
> literal: `ModuleAnnotations` is a total interface, so a partial literal does
> not compile. `pnpm run typecheck:examples` enforces this.

### Bindings

The [`bindings/format-date/`](bindings/format-date/) directory shows the YAML-binding pattern:

| File | Role |
|---|---|
| [`bindings/format-date/binding.yaml`](bindings/format-date/binding.yaml) | Canonical binding definition. |
| [`bindings/format-date/format-date.ts`](bindings/format-date/format-date.ts) | Target function loaded by the binding — `(inputs, context)` signature plus the `inputSchema` / `outputSchema` exports `auto_schema: true` infers from. |
| [`bindings/format-date/run.ts`](bindings/format-date/run.ts) | Runnable driver: loads the binding and calls it through the `Executor`. |

```bash
node examples/bindings/format-date/run.ts
```

A binding `target` module path is handed to Node's `import()` verbatim, so it
must be a package specifier or an absolute path — `run.ts` rewrites the shipped
YAML's bare `format-date` to the absolute path of the co-located file before
loading. See the SDK README's [Bindings](../README.md#bindings) section for the
`BindingLoader` API.

## Pipeline demo — what to look for

`pipeline-demo.ts` is the deep-dive into the engine. One run prints three sections:

1. **Introspection** — the canonical 11 step names from `strategy.stepNames()` / `strategy.info()`.
2. **Middleware tracing** — a `StepMiddleware` that narrates every step of one call:
   ```
   [ 1/11] context_creation    — create execution context, set global deadline
           ✓   0.15 ms · caller=anonymous trace_id=…
   ...
   [11/11] return_result       — finalize and return output
           ✓   0.03 ms · returning {…}
   ```
3. **Orchestration** — `strategy.insertAfter("output_validation", auditLogStep)` adds a 12th step (rendered as `[  +  ]` to mark it as user-inserted), then `strategy.replace("audit_log", quietAuditLogStep)` swaps the implementation while keeping the position.

The `[N/11]` numbering stays pinned to the protocol's 11 standard steps; custom steps appear as `[  +  ]`. This makes the "11 standard + N custom" composition unmistakable in the trace output.
