---
description: "Single-page glossary of apcore terminology (ACL, annotations, canonical ID, context, approval, conformance levels) cross-referenced to authoritative PROTOCOL_SPEC sections."
---

# Glossary

> **Type:** Reference. **Normative spec:** [PROTOCOL_SPEC](./spec/protocol-spec.md) (terms appear in their respective normative sections).

A single-page reference for terminology used across the apcore protocol, the three SDKs, and the ecosystem. Where a definition is normative, the linked PROTOCOL_SPEC section is authoritative — this page exists to disambiguate quickly. Cross-language naming conventions (snake_case in Python/Rust, camelCase in TypeScript) are noted only when they differ in spelling.

## A

**ACL (Access Control List)** — A list of `callers → targets → effect` rules evaluated first-match-wins to determine whether one module is allowed to invoke another. Default effect is **always `deny`** in production. Conditions support identity types, roles, call depth, and the `$or` / `$not` compound operators. See [PROTOCOL_SPEC §6](./spec/protocol-spec.md#6-acl-specification) and [features/acl-system.md](./features/acl-system.md).

**Adapter** — A separate, independently versioned package that projects apcore modules onto a surface. *Surface adapters* (`apcore-mcp`, `apcore-a2a`, `apcore-cli`) expose modules as MCP tools, A2A skills or CLI commands; *framework integrations* (`fastapi-apcore`, `django-apcore`, `flask-apcore`, `nestjs-apcore`, `express-apcore`, `hono-apcore`, `axum-apcore`) bind HTTP endpoints to modules. See [Adapter Development](./guides/adapter-development.md).

**Annotations** — `ModuleAnnotations`: optional behavioral hints on a module — `readonly`, `destructive`, `idempotent`, `requires_approval`, `open_world`, `streaming`, `cacheable`, `cache_ttl`, `cache_key_fields`, `paginated`, `pagination_style`, `discoverable`, plus the open `extra` map. Distinct from the input/output schemas and from `tags`/`version`, which are module metadata. See [protocol-spec §4.4](./spec/protocol-spec.md#44-module-behavior-annotations-annotations) and [§4.4.1](./spec/protocol-spec.md#441-annotations-extension-field-extra-wire-format) for the `extra` wire format.

**APCore Client** — The user-facing client SDK class (`APCore` in all three SDKs) that wires together a `Registry`, `Executor`, optional `ACL`, optional `ApprovalHandler`, middleware, and config. Decorator binding (`@client.module`) and `client.call()` / `client.stream()` / `client.validate()` are exposed here.

**Approval Gate** — Pipeline Step 5. Invokes the configured `ApprovalHandler` when approval is required — by the module's `requires_approval` annotation, its registry metadata, a matching ACL rule with `approval: required`, or an `ExecutionPolicy` that gates destructive modules. With no handler configured the gate is skipped with a warning, unless `ExecutionPolicy(strict=true)` makes it fail closed. See [protocol-spec §7](./spec/protocol-spec.md#7-approval-system).

**ApprovalHandler** — Pluggable interface that the executor calls to obtain an `ApprovalResult` (`approved` / `rejected` / `timeout` / `pending`). A handler either decides immediately (block until decided) or returns `pending`; the caller then resumes by retrying with `_approval_token` in the inputs. See [Approval System](./features/approval-system.md).

## B

**Binding** — A YAML or decorator declaration that maps a Canonical ID to an actual callable (Python function, TypeScript function, Rust async fn). Two flavors: function-based (auto-schema from type hints) and external-schema-binding (schema lives in a separate YAML file). See [PROTOCOL_SPEC §5.11–§5.12](./spec/protocol-spec.md#5-module-specification).

## C

**Caller / `caller_id`** — The Canonical ID of the module (or the literal `@external`) initiating an invocation. Always referred to as `caller_id` in normative text and conformance fixtures — never bare "caller".

**Call Chain** — The ordered list of `caller_id`s representing the active call stack, propagated via `Context.call_chain`. Used by the Call Chain Guard (Step 2) to detect circular invocations and enforce maximum call depth.

**Call Chain Guard** — Pipeline Step 2. Enforces `executor.max_call_depth`, detects circular calls, and limits how often one module may repeat in a chain (`executor.max_module_repeat`). Raises `CALL_DEPTH_EXCEEDED`, `CIRCULAR_CALL` or `CALL_FREQUENCY_EXCEEDED`. See [Call Chain Guard](./features/call-chain-guard.md).

**Canonical ID** — The dotted-path identifier for a module derived from its filesystem path (e.g. `executor.email.send_email` from `<root>/executor/email/send_email.py`). Algorithm A01 in [PROTOCOL_SPEC §2.1](./spec/protocol-spec.md#2-naming-specification) is normative; A02 normalizes IDs across language casing conventions.

**Cancellation** — Cooperative termination of a running invocation, surfaced through the Context (`context.cancel_token` / `context.is_cancelled()`). Modules that participate must check the token between long operations. See [features/cancellation.md](./features/cancellation.md).

**Conformance Level** — One of `Level 0 (Core)`, `Level 1 (Standard)`, `Level 2 (Full)` defined by [docs/spec/conformance.md](./spec/conformance.md). An SDK declares the level it satisfies and lists known deviations.

**Context (`Context` object)** — Per-invocation state carrying `trace_id`, `caller_id`, `call_chain`, `executor`, `identity`, and a free-form `data` map. Spec'd in [PROTOCOL_SPEC §5.7](./spec/protocol-spec.md#5-module-specification); reference in [features/context-object.md](./features/context-object.md). Must be JSON-serializable for cross-language transport.

## D

**Config Bus** — The namespaced configuration system: one `apcore.yaml` (or project file) holds sections for apcore and for ecosystem packages, each registered as a namespace, with per-namespace environment-variable overrides. See [Config Bus](./features/config-bus.md) and [protocol-spec §9](./spec/protocol-spec.md#9-configuration-specification).

**Decision Register** — The index of every recorded design decision (`D-xx`) with its status and the spec version that carries it. Decision records explain *why* a rule exists; the specification defines *what* it is. See [Decision register](./spec/decision-register.md).

**Default Effect** — The fallback decision (`allow` / `deny`) an ACL file applies when no rule matches a `(caller_id, target_id)` pair. Set it in the ACL file; always use `deny` in production.

**Display Overlay** — The optional `display` section of a binding entry that gives a module a per-surface alias, description, guidance and tags (MCP, A2A, CLI) without changing its canonical ID. See [protocol-spec §5.13](./spec/protocol-spec.md#513-display-overlay-surface-facing-presentation).

## E

**Ephemeral Module** — A module registered at runtime under the reserved `ephemeral.*` namespace (for example, a tool synthesized by an agent). Such modules normally set `discoverable: false` so they are callable by ID but hidden from listings. See [protocol-spec §2.5](./spec/protocol-spec.md#25-reserved-words).

**Executor** — The component that runs a module invocation through the 11-step pipeline (Context Creation → Call Chain Guard → Module Lookup → ACL Check → Approval Gate → Middleware Before → Input Validation → Execute → Output Validation → Middleware After → Return). See [features/core-executor.md](./features/core-executor.md).

**Execution Pipeline / Strategy** — The ordered list of steps a call runs through. The standard strategy has eleven built-in steps; preset and custom strategies can remove, insert or replace steps. See [Execution Pipeline](./features/execution-pipeline.md).

**Execution Policy** — `ExecutionPolicy`: executor-level governance settings — `strict` (fail closed when approval is required but no handler is configured), gating of destructive modules, and external policy overrides. See [protocol-spec §7.9](./spec/protocol-spec.md#79-execution-policy-v190-76).

**External Module / `@external`** — The literal caller pattern matching invocations that originated outside apcore (e.g., a public HTTP entry point). Used in ACL rules instead of a Canonical ID.

**Extension (`x-*`) Fields** — Schema and annotation fields prefixed with `x-` reserved for forward-compatible additions. Implementations **MUST** silently ignore unknown `x-*` keys. Notable examples: `x-llm-description`, `x-examples`, `x-sensitive`.

## I

**Identity** — Sub-object on `Context.identity` carrying `id`, `type` (`user` / `service` / `system` / etc.), `roles`, and `attrs`. Used by ACL conditional rules and by audit logging.

## M

**Manifest** — The runtime catalog of registered modules and their schemas, exposed by the Registry and via `system.manifest.*` system modules. Used by adapters to render UIs, OpenAPI specs, or LLM tool catalogs.

**MCP (Model Context Protocol)** — A separate transport protocol for LLM tool invocation. apcore-mcp bridges expose apcore modules as MCP tools. apcore is the **module standard**; MCP is one of several **transport protocols** apcore can be exposed over — they are not synonyms.

**Middleware** — A class or function with `before` / `after` / `on_error` hooks invoked by the Executor in onion order (before 1→N around the module body, after N→1). See [features/middleware-system.md](./features/middleware-system.md). Distinguish from **Step Middleware**, which wraps individual pipeline steps rather than the whole module call.

**Module** — A unit of executable behavior with an input schema, output schema, description, and an `execute()` callable. The smallest deployable artifact in apcore. See [PROTOCOL_SPEC §5](./spec/protocol-spec.md#5-module-specification).

## O

**Orchestrator** — A logical layer name (api → orchestrator → executor → common) used in the layered architecture diagrams. Modules at the `orchestrator.*` namespace coordinate cross-domain workflows; downward calls (`orchestrator → executor`) are allowed, upward calls are not. Not a separate SDK class — a naming convention.

## P

**Preview** — Optional `Module.preview()`: returns the changes a call would make without making them. Its result appears as `predicted_changes` in a `PreflightResult`; `preview()` runs only when the ACL check passes. See [Module Interface](./features/module-interface.md).

**PreflightResult** — The return type of `validate()`: per-check status for pipeline Steps 1–5 and 7 (no middleware, no execution), a `requires_approval` flag, `predicted_changes` from an optional `Module.preview()`, and `.valid` / `.errors` summaries. See [protocol-spec §12.8](./spec/protocol-spec.md#128-executorvalidate-cross-language-implementation-guide).

## R

**Registry** — In-memory catalog mapping Canonical ID → registered module. Provides `discover()` (directory scan), `register()`, `get()`, `list()`. Multiple registries may coexist; the APCore Client owns one by default.

**Reload (hot-reload)** — Re-scanning the extension directory and updating the registry without restarting the host process. Optional (Level 2 feature) and gated by safety checks against in-flight calls.

## S

**`x-sensitive`** — Schema-level annotation marking a property as containing sensitive data (PII, credentials, etc.). Logging middleware and the audit pipeline **MUST** redact fields tagged `x-sensitive: true` before emitting them. Combined with `obs.redaction.sensitive_keys` (canonical defaults shipped in all 3 SDKs — see fixture `sensitive_keys_default`).

**Schema** — JSON Schema Draft 2020-12 document describing module input or output. Specified in [PROTOCOL_SPEC §4](./spec/protocol-spec.md#4-schema-specification). Three layers: **Core** (required: `input_schema`, `output_schema`, `description`), **Annotation** (optional behavioral hints), **Extension** (`x-*` open-ended).

**Stream / Streaming Module** — A module that implements `stream()` and yields partial outputs (chunks). The executor deep-merges the chunks (depth limit `stream.max_merge_depth`, default 32), validates the accumulated output against the output schema, and falls back to `execute()` for modules without `stream()`. See [Streaming](./features/streaming.md).

**System Modules (`system.*`)** — Reserved namespace for built-in introspection and control modules: `system.health.*`, `system.usage.*`, `system.manifest.*`, `system.control.*`. Registered when `sys_modules.enabled` is true (control modules additionally need `sys_modules.control.enabled`) and governed by the same ACL and approval rules as user modules. See [System Modules](./features/system-modules.md).

## T

**Trace ID (`trace_id`)** — 32-character lowercase hex string compatible with W3C Trace Context, generated at the entry to a call tree and propagated unchanged to all child invocations via `Context.trace_id`. Externally provided unvalidated values **MUST NOT** be accepted; either accept verbatim after validation or replace with a fresh value (see [PROTOCOL_SPEC §10.5](./spec/protocol-spec.md#10-observability-specification)).

**Target / `target_id`** — The Canonical ID of the module being invoked. Always `target_id` in normative text — never bare "target".

## See also

- [PROTOCOL_SPEC §1.6](./spec/protocol-spec.md#1-overview) — full normative terminology section.
- [Conformance](./spec/conformance.md) — conformance levels and the cross-language fixture suite.
- [Decision register](./spec/decision-register.md) — the recorded reason behind each normative rule.
