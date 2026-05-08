# Glossary

> **Type:** Reference. **Normative spec:** [PROTOCOL_SPEC](./spec/protocol-spec.md) (terms appear in their respective normative sections).

A single-page reference for terminology used across the apcore protocol, the three SDKs, and the ecosystem. Where a definition is normative, the linked PROTOCOL_SPEC section is authoritative — this page exists to disambiguate quickly. Cross-language naming conventions (snake_case in Python/Rust, camelCase in TypeScript) are noted only when they differ in spelling.

## A

**ACL (Access Control List)** — A list of `callers → targets → effect` rules evaluated first-match-wins to determine whether one module is allowed to invoke another. Default effect is **always `deny`** in production. Conditions support identity types, roles, call depth, and the `$or` / `$not` compound operators. See [PROTOCOL_SPEC §6](./spec/protocol-spec.md#6-acl-specification) and [features/acl-system.md](./features/acl-system.md).

**Adapter** — A separate package that bridges apcore modules to a host framework (FastAPI, Flask, Django, NestJS, Axum, …). Adapters live outside the apcore core and SDK repos — see e.g. `fastapi-apcore`, `nestjs-apcore`, `axum-apcore`. They scan apcore modules and surface them as routes, queue jobs, or RPC methods.

**Annotations** — Optional behavioral hints on a module (`requires_approval`, `tags`, `version`, `extra` extensible fields). Distinct from the schema's input/output type info. The wire format and `extra` semantics are normative — see [PROTOCOL_SPEC §4.4.1](./spec/protocol-spec.md#441-annotations-extension-field-extra-wire-format).

**APCore Client** — The user-facing client SDK class (`APCore` in all three SDKs) that wires together a `Registry`, `Executor`, optional `ACL`, optional `ApprovalHandler`, middleware, and config. Decorator binding (`@client.module`) and `client.call()` / `client.stream()` / `client.validate()` are exposed here.

**Approval Gate** — Pipeline Step 5. Invokes the registered `ApprovalHandler` when the target module declares `requires_approval=true`. Skipped entirely when no handler is configured or the annotation is absent. See [PROTOCOL_SPEC §7](./spec/protocol-spec.md#7-approval-system).

**ApprovalHandler** — Pluggable interface that the executor calls to obtain an `ApprovalResult` (`approved` / `rejected` / `timeout` / `pending`). Phase A is sync-blocking; Phase B uses `pending` + `_approval_token` for async resume.

## B

**Binding** — A YAML or decorator declaration that maps a Canonical ID to an actual callable (Python function, TypeScript function, Rust async fn). Two flavors: function-based (auto-schema from type hints) and external-schema-binding (schema lives in a separate YAML file). See [PROTOCOL_SPEC §5.11–§5.12](./spec/protocol-spec.md#5-module-specification).

## C

**Caller / `caller_id`** — The Canonical ID of the module (or the literal `@external`) initiating an invocation. Always referred to as `caller_id` in normative text and conformance fixtures — never bare "caller".

**Call Chain** — The ordered list of `caller_id`s representing the active call stack, propagated via `Context.call_chain`. Used by the Call Chain Guard (Step 2) to detect circular invocations and enforce maximum call depth.

**Call Chain Guard** — Pipeline Step 2. Inspects `context.call_chain` for circular calls (target already present) and over-depth (chain length exceeds policy). Raises `CIRCULAR_CALL` or `CALL_DEPTH_EXCEEDED` on violation.

**Canonical ID** — The dotted-path identifier for a module derived from its filesystem path (e.g. `executor.email.send_email` from `<root>/executor/email/send_email.py`). Algorithm A01 in [PROTOCOL_SPEC §2.1](./spec/protocol-spec.md#2-naming-specification) is normative; A02 normalizes IDs across language casing conventions.

**Cancellation** — Cooperative termination of a running invocation, surfaced through the Context (`context.cancel_token` / `context.is_cancelled()`). Modules that participate must check the token between long operations. See [features/cancellation.md](./features/cancellation.md).

**Conformance Level** — One of `Level 0 (Core)`, `Level 1 (Standard)`, `Level 2 (Full)` defined by [docs/spec/conformance.md](./spec/conformance.md). An SDK declares the level it satisfies and lists known deviations.

**Context (`Context` object)** — Per-invocation state carrying `trace_id`, `caller_id`, `call_chain`, `executor`, `identity`, and a free-form `data` map. Spec'd in [PROTOCOL_SPEC §5.7](./spec/protocol-spec.md#5-module-specification); reference in [features/context-object.md](./features/context-object.md). Must be JSON-serializable for cross-language transport.

## D

**Declarative Config** — The unified YAML configuration surface (`apcore.yaml` plus optional bindings YAML and pipeline YAML) consumed identically by all three SDKs. Specified in [docs/spec/DECLARATIVE_CONFIG_SPEC.md](./spec/DECLARATIVE_CONFIG_SPEC.md).

**Default Effect** — The fallback decision (`allow` / `deny`) applied when no ACL rule matches a `(caller_id, target_id)` pair. **Production deployments MUST set `deny`.**

## E

**Executor** — The component that runs a module invocation through the 11-step pipeline (Context Creation → Call Chain Guard → Module Lookup → ACL Check → Approval Gate → Middleware Before → Input Validation → Execute → Output Validation → Middleware After → Return). See [features/core-executor.md](./features/core-executor.md).

**External Module / `@external`** — The literal caller pattern matching invocations that originated outside apcore (e.g., a public HTTP entry point). Used in ACL rules instead of a Canonical ID.

**Extension (module extension)** — Older synonym for "module" used in some early docs. Prefer **module** in new writing.

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

**PreflightResult** — The return type of `Executor.validate()` carrying per-check status, a `requires_approval` flag, and `.valid`/`.errors` properties for backward compatibility with the legacy `ValidationResult`. Six pipeline checks plus an optional module-level preflight; see [PROTOCOL_SPEC §12.8](./spec/protocol-spec.md#12-sdk-implementation-guide).

## R

**Registry** — In-memory catalog mapping Canonical ID → registered module. Provides `discover()` (directory scan), `register()`, `get()`, `list()`. Multiple registries may coexist; the APCore Client owns one by default.

**Reload (hot-reload)** — Re-scanning the extension directory and updating the registry without restarting the host process. Optional (Level 2 feature) and gated by safety checks against in-flight calls.

## S

**`x-sensitive`** — Schema-level annotation marking a property as containing sensitive data (PII, credentials, etc.). Logging middleware and the audit pipeline **MUST** redact fields tagged `x-sensitive: true` before emitting them. Combined with `obs.redaction.sensitive_keys` (canonical defaults shipped in all 3 SDKs — see fixture `sensitive_keys_default`).

**Schema** — JSON Schema Draft 2020-12 document describing module input or output. Specified in [PROTOCOL_SPEC §4](./spec/protocol-spec.md#4-schema-specification). Three layers: **Core** (required: `input_schema`, `output_schema`, `description`), **Annotation** (optional behavioral hints), **Extension** (`x-*` open-ended).

**Stream / Streaming Module** — A module whose `execute()` yields successive partial outputs (chunks). The Executor merges chunks via recursive deep-merge (depth-capped at 32), validates the final accumulated output against the output schema, and emits per-chunk events for observability. See [features/streaming.md](./features/streaming.md).

**System Modules (`sys.*`)** — Reserved namespace for framework-provided control-plane modules: `sys.health.*`, `sys.manifest.*`, `sys.control.*`. Authorization is enforced by the same ACL system as user modules. See [features/system-modules.md](./features/system-modules.md).

## T

**Trace ID (`trace_id`)** — 32-character lowercase hex string compatible with W3C Trace Context, generated at the entry to a call tree and propagated unchanged to all child invocations via `Context.trace_id`. Externally provided unvalidated values **MUST NOT** be accepted; either accept verbatim after validation or replace with a fresh value (see [PROTOCOL_SPEC §10.5](./spec/protocol-spec.md#10-observability-specification)).

**Target / `target_id`** — The Canonical ID of the module being invoked. Always `target_id` in normative text — never bare "target".

## See also

- [Concept Index](./site-map.md#concept-index) — quick navigation table in the documentation map.
- [PROTOCOL_SPEC §1.6](./spec/protocol-spec.md#1-overview) — full normative terminology section.
- [docs/spec/conformance.md](./spec/conformance.md) — fixture catalog cross-referenced by feature.
