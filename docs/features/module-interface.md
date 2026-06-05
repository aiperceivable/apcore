---
description: "Structural duck-typed module contract: required input_schema/output_schema/description and execute(); optional validate/preflight/describe/stream, lifecycle hooks; class/function forms."
---

# Module Interface

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §5.6 Module Interface Protocol.

## Overview

The Module Interface is the central contract every apcore module must satisfy. It defines the required schema attributes (`input_schema`, `output_schema`, `description`), the `execute()` entry point, and a set of optional attributes, lifecycle hooks, and methods that modules MAY implement to express richer behavior to the framework and AI callers. The interface is **structural** — apcore validates conformance via duck typing, not ABC inheritance — which lets each language SDK express the contract using the most idiomatic construct available (Python `Protocol`, TypeScript interface, Rust trait).

apcore also accepts a function-based form (`@module` decorator or `module(callable, ...)`) that produces a Module object equivalent to the class form. The two forms are interchangeable in Registry, Executor, and Schema export behavior.

## Requirements

- Every module MUST declare an `input_schema` and `output_schema` (Pydantic `BaseModel` in Python, Zod schema in TypeScript, serde-derived struct in Rust).
- Every module MUST provide a `description` (≤200 chars, plain text) sourced from a docstring or explicit attribute.
- Every module MUST implement an `execute(inputs, context) -> outputs` method (sync or async — the framework auto-detects).
- Modules MAY implement `validate()`, `preflight()`, `describe()`, `stream()` for additional capabilities.
- Modules MAY define lifecycle hooks (`on_load`, `on_unload`, `on_suspend`, `on_resume`) for resource and state management, including hot-reload state preservation.
- Modules MAY annotate behavior via `ModuleAnnotations` (read-only / destructive / idempotent / requires_approval / open_world / streaming / cacheable / paginated, plus an `extra` dict for ecosystem extensions).
- Modules MAY supply `examples`, `tags`, `name`, `version`, and a free-form `metadata` dict.
- Function-based modules MUST have type annotations on all parameters and the return value; the framework auto-generates the schemas from the signature.
- The framework MUST validate structural conformance at registration time and reject modules missing required attributes.

## Technical Design

### Structural Typing

Modules MUST NOT inherit from an ABC. The framework checks for the required attributes and methods (`input_schema`, `output_schema`, `description`, `execute`) using language-appropriate structural mechanisms:

- **Python** — `typing.Protocol` with `@runtime_checkable`.
- **TypeScript** — Structural interfaces; conformance verified at registration via attribute checks.
- **Rust** — `Module` trait with required methods; the compiler enforces conformance.

### Required Attributes

| Attribute | Constraint |
|-----------|-----------|
| `input_schema` | Schema type (Pydantic `BaseModel` / Zod schema / serde struct). Every field MUST have a `description`. |
| `output_schema` | Same constraints as `input_schema`. The `execute()` return value MUST validate against it. |
| `description` | Plain text, ≤200 chars. Sourced from class docstring or explicit attribute. Describes "what / when / key features". |
| `documentation` | Optional. Markdown, ≤5000 chars. Used for richer AI-facing documentation. |
| `execute(inputs, context)` | Required method. May be sync or async. The framework auto-detects. |

### Optional Attributes

| Attribute | Default | Purpose |
|-----------|---------|---------|
| `name` | Generated from class name | Human-readable name |
| `tags` | `[]` | Categorization for `Registry.list(tags=...)` |
| `version` | `"1.0.0"` | SemVer |
| `annotations` | `ModuleAnnotations()` defaults | Behavior hints for AI callers (see below) |
| `examples` | `[]` | `ModuleExample` list — recommended when input has unions, >5 required fields, or 2+ levels of nesting |
| `metadata` | `{}` | Free-form extension dict (cost hints, side effects, owner, etc.) |

### ModuleAnnotations

Behavior annotations help AI/LLM callers make invocation decisions. All fields are optional and default to safe values.

| Field | Default | Meaning |
|-------|---------|---------|
| `readonly` | `False` | Does not modify any state |
| `destructive` | `False` | May delete or overwrite data |
| `idempotent` | `False` | Repeated calls have no additional side effects |
| `requires_approval` | `False` | Requires human confirmation before execution. Enforced at runtime by [Approval System](./approval-system.md). |
| `open_world` | `True` | Connects to external systems |
| `discoverable` | `True` | Whether the module appears in manifests and tool discovery. When `false`, it remains callable by ID but is hidden from discovery. |
| `streaming` | `False` | Supports chunk-by-chunk output via `stream()` |
| `cacheable` | `False` | Output can be cached for identical inputs |
| `cache_ttl` | `0` | Cache duration in seconds (0 = no cache) |
| `cache_key_fields` | `None` | Tuple of input fields for cache key (None = all fields). Lists are auto-converted to tuples. |
| `paginated` | `False` | Returns paginated results |
| `pagination_style` | `"cursor"` | `"cursor"`, `"offset"`, or `"page"` |
| `extra` | `{}` | Ecosystem extension metadata not covered by standard fields |

### Metadata Conventions

The `metadata` dict MAY carry ecosystem-standard keys to aid AI orchestration. SDKs SHOULD pass these keys through to discovery manifests.

| Key | Type | Meaning |
|-----|------|---------|
| `x-reasoning-demand` | `int` (1–5) | Indicates the expected reasoning complexity (1 = simple tool, 5 = high-agency task). |
| `x-required-context-keys` | `list[str]` | Keys that MUST be present in `context.data` for the module to function. |
| `x-supports-dry-run` | `bool` | Indicates the module supports a non-destructive dry-run mode via an input flag. |

### Lifecycle Hooks

| Hook | Purpose |
|------|---------|
| `on_load()` | Called once when the module is registered. Initialize resources here (DB connections, pools). |
| `on_unload()` | Called when the module is removed or the application shuts down. Clean up resources. |
| `on_suspend() -> dict \| None` | Called before hot-reload. Return JSON-serializable state to preserve. |
| `on_resume(state: dict)` | Called after hot-reload with the dict returned by `on_suspend()`. |

**Hot-reload sequence:**

```text
old.on_suspend() → state          ← export state
old.on_unload()                   ← release resources
  (reload module code from disk)
new.__init__()
new.on_load()                     ← acquire resources
new.on_resume(state)              ← restore state (only if state is not None)
```

`on_suspend()` return values MUST be JSON-serializable. `on_resume()` MUST tolerate missing or extra keys (versions may differ). Hook exceptions are logged but do not block reload.

### Optional Methods

| Method | Purpose |
|--------|---------|
| `validate(inputs) -> ValidationResult` | Custom input validation without execution. Should be side-effect free. |
| `preview(inputs, context) -> dict` | Returns a "low-fidelity" result without performing side effects. Used by AI agents to verify their plan before execution. |
| `preflight(inputs, context) -> list[str]` | Advisory warnings emitted during `Executor.validate()`. Does NOT block execution. |
| `describe() -> dict` | Module metadata for introspection. Used by `system.manifest`. Default returns `{description, input_schema, output_schema, annotations}`. |
| `stream(inputs, context) -> AsyncIterator[dict]` | Streaming output. When defined, `Executor.stream()` calls this instead of `execute()`. Modules implementing `stream()` MUST satisfy the [`StreamingModule` interface](./streaming.md#streaming-module-interface-issue-62) for their target language (Python Protocol with `@runtime_checkable`; TypeScript interface + `Symbol.for("apcore.streaming")` marker; Rust `trait StreamingModule: Module`). Modules implementing `stream()` SHOULD set `annotations.streaming = True`. |
| `as_streaming() -> Optional[StreamingModule]` (Rust only) | Trait-object accessor on the base `Module` trait for adapter / bridge code that needs a typed `&dyn StreamingModule` handle. Default returns `None`; streaming modules override to return `Some(self)`. MUST stay consistent with `Module::stream()` — both return `Some(_)` or both return `None` per module. See [Streaming Module Interface](./streaming.md#streaming-module-interface-issue-62). |

### Sync / Async Execution

Modules define **one** `execute()` method, either `def` or `async def`. The framework detects which and dispatches accordingly. Both `Executor.call()` and `Executor.call_async()` handle both forms.

### Function-Based Modules

The `@module(...)` decorator (or the call form `module(callable, ...)`) lets unmodifiable code (class methods, third-party functions) be registered without writing a Module class. The framework derives equivalents:

| Class attribute | Function-based equivalent |
|-----------------|---------------------------|
| `input_schema` | Auto-generated from parameter type annotations |
| `output_schema` | Auto-generated from return type annotation |
| `description` | First line of docstring, or `description` parameter |
| `documentation` | `documentation` parameter |
| `execute()` | The function itself |
| `name`, `tags`, `version`, `annotations`, `metadata` | Decorator parameters |
| `on_load`, `on_unload`, `on_suspend`, `on_resume` | Not supported in function form |

If a parameter is declared as `context: Context`, the framework auto-injects the Context object. The `context` parameter is excluded from the generated `input_schema`.

For full grammar details and decorator semantics, see [PROTOCOL_SPEC §5.11](../spec/protocol-spec.md) and [Decorator & YAML Bindings](./decorator-bindings.md).

## Contract: Module conformance

Normative behavioral contract. All SDK implementations MUST satisfy these guarantees.

### Inputs

A module is supplied to the framework either as a class implementing the module surface or, in function form, as a decorated callable from which the surface is derived (see the function-based equivalence table above). The behavioral contract is exercised through `execute()`, which receives:

- `inputs` (dict / object / map, required) — the call arguments. MUST validate against `input_schema` at execution time; a validation failure is surfaced as a schema error before the module body runs.
- `context` (`Context`, required) — the per-call context carrying identity, `data`, cancellation signal, and deadline. In function form, a parameter declared as `context: Context` is auto-injected and excluded from the generated `input_schema`.

### Required surface

- `input_schema` — schema type. MUST exist; MUST validate inputs at execution time.
- `output_schema` — schema type. MUST exist; MUST validate outputs at execution time.
- `description` — string, ≤200 chars, MUST exist (docstring or attribute).
- `execute(inputs, context) -> dict` — MUST exist as `def` or `async def`.

### Optional surface

| Surface | Level | Contract |
|---------|-------|----------|
| `documentation` | MAY | ≤5000 chars, Markdown allowed |
| `validate()` | MAY | No side effects |
| `preflight()` | MAY | Returns advisory warnings; MUST NOT block execution |
| `describe()` | MAY | Returns introspection dict |
| `stream()` | MAY | Async iterator yielding partial dicts; framework deep-merges chunks |
| `on_load`, `on_unload` | MAY | Exceptions MUST NOT block other modules from loading |
| `on_suspend`, `on_resume` | MAY | Used during hot-reload only; return value MUST be JSON-serializable |
| `name` | MAY | Defaults to derived from class name |
| `tags` | MAY | Defaults to `[]` |
| `version` | MAY | Defaults to `"1.0.0"`; MUST conform to SemVer |
| `annotations` | MAY | Defaults applied where unset |
| `examples` | MAY | Each `inputs` MUST validate against `input_schema` |
| `metadata` | MAY | Defaults to `{}` |

### Timeout semantics

- Module execution SHOULD complete within the configured timeout (`resources.timeout`, default 30 000 ms; global `executor.global_timeout`, default 60 000 ms).
- After timeout the framework MUST raise `MODULE_TIMEOUT`.
- Modules SHOULD support cooperative cancellation by polling the cancellation signal exposed via `context`.

### Thread safety

- Module instances MUST tolerate concurrent `execute()` invocations.
- Modules MUST NOT mutate class-level (`ClassVar`) attributes inside `execute()`.
- Shared state MUST use thread-safe constructs.

### Return-value constraints

- `execute()` MUST return a dict (or language-equivalent map).
- The return value MUST validate against `output_schema`.
- The return value MUST NOT contain non-serializable objects (functions, open connections, etc.).

### Errors

- `MissingRequiredAttribute` — module lacks `input_schema`, `output_schema`, `description`, or `execute`.
- `InvalidSchemaType` — schema attribute is not a recognized schema type.
- `DescriptionTooLong` — `description` exceeds 200 chars.
- `DocumentationTooLong` — `documentation` exceeds 5000 chars.
- `InvalidAnnotations` — `annotations` not of type `ModuleAnnotations`.
- `InvalidExample` — entry in `examples` missing `title` or `inputs`.

### Returns

`execute()` MUST return a dict (or language-equivalent map) that validates against `output_schema`, subject to the constraints in *Return-value constraints* above (JSON-serializable; no functions or open connections). The optional surface methods return as follows:

- `validate()` — no return value; raises on a conformance violation (see *Errors*).
- `preflight()` — advisory warnings; MUST NOT block execution.
- `describe()` — an introspection dict.
- `stream()` — an async iterator yielding partial dicts that the framework deep-merges into the final output.

### Properties

- async: `execute()` MAY be synchronous or asynchronous (`def` or `async def`); the framework supports both.
- thread_safe: required — instances MUST tolerate concurrent `execute()` invocations and MUST NOT mutate class-level (`ClassVar`) attributes during execution (see *Thread safety* above).
- pure: not required — modules MAY perform side effects; lifecycle hooks (`on_load`, `on_unload`, `on_suspend`, `on_resume`) exist precisely to manage external state.
- timeout-bound: execution SHOULD complete within `resources.timeout` (default 30 000 ms) / `executor.global_timeout` (default 60 000 ms); on expiry the framework MUST raise `MODULE_TIMEOUT`. Modules SHOULD support cooperative cancellation by polling the signal exposed via `context`.

The full required and optional method/attribute surface — including conformance levels (MUST / MAY) and per-member constraints — is enumerated in the *Required surface* and *Optional surface* tables above.

## Usage

=== "Python"

    ```python
    from typing import Any, ClassVar, Type
    from pydantic import BaseModel, Field
    from apcore import Module, Context, ModuleAnnotations, ModuleExample


    class SendEmailInput(BaseModel):
        to: str = Field(..., description="Recipient email address",
                        pattern=r"^[\w\.-]+@[\w\.-]+\.\w+$")
        subject: str = Field(..., description="Email subject", max_length=200)
        body: str = Field(..., description="Email body")
        cc: list[str] = Field(default=[], description="CC list")


    class SendEmailOutput(BaseModel):
        success: bool = Field(..., description="Whether sending was successful")
        message_id: str | None = Field(None, description="Message ID")
        error: str | None = Field(None, description="Error message")


    class SendEmailModule(Module):
        """Send email to specified recipient. Uses SMTP protocol."""

        input_schema: ClassVar[Type[BaseModel]] = SendEmailInput
        output_schema: ClassVar[Type[BaseModel]] = SendEmailOutput

        tags: ClassVar[list[str]] = ["email", "notification"]
        annotations = ModuleAnnotations(open_world=True)

        examples = [
            ModuleExample(
                title="Send plain text email",
                inputs={"to": "user@example.com", "subject": "Hello", "body": "World"},
                output={"success": True, "message_id": "msg_123", "error": None},
            ),
        ]

        def on_load(self) -> None:
            self._smtp = self._connect()

        def on_unload(self) -> None:
            self._smtp.close()

        def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
            params = SendEmailInput(**inputs)
            try:
                msg_id = self._smtp.send(params.to, params.subject, params.body)
                return {"success": True, "message_id": msg_id, "error": None}
            except Exception as e:
                return {"success": False, "message_id": None, "error": str(e)}
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';
    import { APCore, createAnnotations, type Context } from 'apcore-js';

    const SendEmailInput = Type.Object({
      to: Type.String({ format: 'email', description: 'Recipient email address' }),
      subject: Type.String({ maxLength: 200, description: 'Email subject' }),
      body: Type.String({ description: 'Email body' }),
      cc: Type.Array(Type.String(), { default: [], description: 'CC list' }),
    });

    const SendEmailOutput = Type.Object({
      success: Type.Boolean({ description: 'Whether sending was successful' }),
      messageId: Type.Union([Type.String(), Type.Null()], { description: 'Message ID' }),
      error: Type.Union([Type.String(), Type.Null()], { description: 'Error message' }),
    });

    const client = new APCore();
    client.module({
      id: 'email.send_email',
      description: 'Send email to specified recipient. Uses SMTP protocol.',
      tags: ['email', 'notification'],
      inputSchema: SendEmailInput,
      outputSchema: SendEmailOutput,
      annotations: createAnnotations({ openWorld: true }),
      async execute(inputs: { to: string; subject: string; body: string }, _ctx: Context) {
        try {
          const messageId = await sendEmail(inputs.to, inputs.subject, inputs.body);
          return { success: true, messageId, error: null };
        } catch (e) {
          return { success: false, messageId: null, error: String(e) };
        }
      },
    });
    ```

=== "Rust"

    ```rust
    use apcore::{Context, Module};
    use apcore::errors::{ErrorCode, ModuleError};
    use async_trait::async_trait;
    use serde::{Deserialize, Serialize};
    use serde_json::{json, Value};

    #[derive(Debug, Deserialize)]
    pub struct SendEmailInput {
        pub to: String,
        pub subject: String,
        pub body: String,
        #[serde(default)]
        pub cc: Vec<String>,
    }

    #[derive(Debug, Serialize)]
    pub struct SendEmailOutput {
        pub success: bool,
        pub message_id: Option<String>,
        pub error: Option<String>,
    }

    /// Send email to specified recipient. Uses SMTP protocol.
    pub struct SendEmailModule {
        smtp: SmtpClient,
    }

    #[async_trait]
    impl Module for SendEmailModule {
        fn description(&self) -> &str {
            "Send email to specified recipient. Uses SMTP protocol."
        }

        fn input_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {
                    "to":      { "type": "string", "format": "email", "description": "Recipient email address" },
                    "subject": { "type": "string", "maxLength": 200, "description": "Email subject" },
                    "body":    { "type": "string", "description": "Email body" },
                    "cc":      { "type": "array", "items": { "type": "string" }, "default": [], "description": "CC list" }
                },
                "required": ["to", "subject", "body"],
                "additionalProperties": false
            })
        }

        fn output_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {
                    "success":    { "type": "boolean", "description": "Whether sending was successful" },
                    "message_id": { "type": ["string", "null"], "description": "Message ID" },
                    "error":      { "type": ["string", "null"], "description": "Error message" }
                },
                "required": ["success"],
                "additionalProperties": false
            })
        }

        fn tags(&self) -> Vec<String> {
            vec!["email".into(), "notification".into()]
        }

        async fn execute(
            &self,
            inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Value, ModuleError> {
            let input: SendEmailInput = serde_json::from_value(inputs)
                .map_err(|e| ModuleError::new(ErrorCode::GeneralInvalidInput, e.to_string()))?;
            match self.smtp.send(&input.to, &input.subject, &input.body) {
                Ok(message_id) => Ok(json!({
                    "success": true, "message_id": message_id, "error": null
                })),
                Err(e) => Ok(json!({
                    "success": false, "message_id": null, "error": e.to_string()
                })),
            }
        }
    }
    ```

### Function-based form

=== "Python"

    ```python
    from apcore import module, Context

    @module(id="email.send", tags=["email"])
    def send_email(to: str, subject: str, body: str, context: Context) -> dict:
        """Send email to specified recipient."""
        # context is auto-injected; not part of input_schema
        return {"success": True, "message_id": "msg_123", "error": None}
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';
    import { APCore } from 'apcore-js';

    const client = new APCore();
    client.module({
      id: 'email.send',
      description: 'Send email to specified recipient',
      tags: ['email'],
      inputSchema: Type.Object({
        to: Type.String(),
        subject: Type.String(),
        body: Type.String(),
      }),
      outputSchema: Type.Object({
        success: Type.Boolean(),
        messageId: Type.Union([Type.String(), Type.Null()]),
        error: Type.Union([Type.String(), Type.Null()]),
      }),
      execute: async (_inputs, _ctx) => ({
        success: true,
        messageId: 'msg_123',
        error: null,
      }),
    });
    ```

=== "Rust"

    ```rust
    // Rust has no #[module] proc macro; register the function via APCore::module().
    use apcore::{APCore, Context};
    use apcore::errors::ModuleError;
    use serde_json::{json, Value};

    let mut client = APCore::new();
    client.module(
        "email.send",
        "Send email to specified recipient",
        json!({ "type": "object", "properties": {
            "to": { "type": "string" }, "subject": { "type": "string" }, "body": { "type": "string" }
        }, "required": ["to", "subject", "body"] }),
        json!({ "type": "object", "properties": {
            "success": { "type": "boolean" },
            "message_id": { "type": ["string", "null"] },
            "error": { "type": ["string", "null"] }
        }, "required": ["success"] }),
        None,
        vec!["email".into()],
        None,
        None,
        vec![],
        None,
        |_inputs: Value, _ctx: &Context<Value>| {
            Box::pin(async move {
                Ok::<Value, ModuleError>(json!({
                    "success": true, "message_id": "msg_123", "error": null
                }))
            })
        },
    )?;
    ```

For YAML-based bindings and decorator details, see [Decorator & YAML Bindings](./decorator-bindings.md).

## Dependencies

- [Schema System](./schema-system.md) — input/output schema loading and validation.
- [Core Executor](./core-executor.md) — invokes `execute()`, `validate()`, `preflight()`, `stream()`.
- [Registry System](./registry-system.md) — performs structural conformance checks at registration.
- [Identity System](./identity-system.md) — Context delivery into `execute()`.

## Testing Strategy

- Conformance tests assert that minimal modules with only required surface register and execute successfully.
- Negative tests assert that missing `input_schema`/`output_schema`/`description`/`execute` produces clear errors at registration time.
- Lifecycle tests cover `on_load`/`on_unload` ordering, `on_suspend`/`on_resume` round-trip, and exception isolation.
- Sync/async dispatch tests cover both `def` and `async def` modules invoked through `call()` and `call_async()`.
- Function-based form tests verify schema generation parity with class form.

## Mapping to AI Protocols

| Standard apcore field | Anthropic | A2A | MCP |
|-----------------------|-----------|-----|-----|
| `description` | `description` | `AgentSkill.description` | `description` |
| `examples` | `input_examples` | `AgentSkill.examples` | Placed in `_meta` |
| `annotations.requires_approval` | Tool-use confirmation | Skill consent flag | `requires_approval` annotation |

For full type mapping across languages, see [Type Mapping](../spec/type-mapping.md).

## Next Steps

- [Creating Modules Guide](../guides/creating-modules.md) — full tutorial.
- [Decorator & YAML Bindings](./decorator-bindings.md) — `@module` grammar and YAML form.
- [Schema System](./schema-system.md) — schema rules and `$ref` resolution.
- [Core Executor](./core-executor.md) — how `execute()` is invoked.
