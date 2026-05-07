# Module Interface

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../../PROTOCOL_SPEC.md) §5.6 Module Interface Protocol.

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
| `streaming` | `False` | Supports chunk-by-chunk output via `stream()` |
| `cacheable` | `False` | Output can be cached for identical inputs |
| `cache_ttl` | `0` | Cache duration in seconds (0 = no cache) |
| `cache_key_fields` | `None` | Tuple of input fields for cache key (None = all fields). Lists are auto-converted to tuples. |
| `paginated` | `False` | Returns paginated results |
| `pagination_style` | `"cursor"` | `"cursor"`, `"offset"`, or `"page"` |
| `extra` | `{}` | Ecosystem extension metadata not covered by standard fields |

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
| `preflight(inputs, context) -> list[str]` | Advisory warnings emitted during `Executor.validate()`. Does NOT block execution. |
| `describe() -> dict` | Module metadata for introspection. Used by `system.manifest`. Default returns `{description, input_schema, output_schema, annotations}`. |
| `stream(inputs, context) -> AsyncIterator[dict]` | Streaming output. When defined, `Executor.stream()` calls this instead of `execute()`. Modules implementing `stream()` SHOULD set `annotations.streaming = True`. |

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

For full grammar details and decorator semantics, see [PROTOCOL_SPEC §5.11](../../PROTOCOL_SPEC.md) and [Decorator & YAML Bindings](./decorator-bindings.md).

## Contract: Module conformance

Normative behavioral contract. All SDK implementations MUST satisfy these guarantees.

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
    import { z } from "zod";
    import { Module, Context, ModuleAnnotations } from "apcore";

    const SendEmailInput = z.object({
      to: z.string().email().describe("Recipient email address"),
      subject: z.string().max(200).describe("Email subject"),
      body: z.string().describe("Email body"),
      cc: z.array(z.string()).default([]).describe("CC list"),
    });

    const SendEmailOutput = z.object({
      success: z.boolean().describe("Whether sending was successful"),
      messageId: z.string().nullable().describe("Message ID"),
      error: z.string().nullable().describe("Error message"),
    });

    export class SendEmailModule implements Module {
      static description = "Send email to specified recipient. Uses SMTP protocol.";
      static inputSchema = SendEmailInput;
      static outputSchema = SendEmailOutput;
      static tags = ["email", "notification"];
      static annotations: ModuleAnnotations = { openWorld: true };

      onLoad() { this.smtp = this.connect(); }
      onUnload() { this.smtp.close(); }

      async execute(inputs: z.infer<typeof SendEmailInput>, ctx: Context) {
        try {
          const messageId = await this.smtp.send(inputs.to, inputs.subject, inputs.body);
          return { success: true, messageId, error: null };
        } catch (e) {
          return { success: false, messageId: null, error: String(e) };
        }
      }
    }
    ```

=== "Rust"

    ```rust
    use apcore::{Module, Context, ModuleAnnotations, ModuleResult};
    use serde::{Deserialize, Serialize};
    use schemars::JsonSchema;

    #[derive(Debug, Deserialize, JsonSchema)]
    pub struct SendEmailInput {
        /// Recipient email address
        pub to: String,
        /// Email subject
        pub subject: String,
        /// Email body
        pub body: String,
        /// CC list
        #[serde(default)]
        pub cc: Vec<String>,
    }

    #[derive(Debug, Serialize, JsonSchema)]
    pub struct SendEmailOutput {
        /// Whether sending was successful
        pub success: bool,
        /// Message ID
        pub message_id: Option<String>,
        /// Error message
        pub error: Option<String>,
    }

    /// Send email to specified recipient. Uses SMTP protocol.
    pub struct SendEmailModule {
        smtp: SmtpClient,
    }

    impl Module for SendEmailModule {
        type Input = SendEmailInput;
        type Output = SendEmailOutput;

        fn annotations() -> ModuleAnnotations {
            ModuleAnnotations { open_world: true, ..Default::default() }
        }

        fn execute(&self, input: SendEmailInput, _ctx: &Context) -> ModuleResult<SendEmailOutput> {
            match self.smtp.send(&input.to, &input.subject, &input.body) {
                Ok(message_id) => Ok(SendEmailOutput { success: true, message_id: Some(message_id), error: None }),
                Err(e) => Ok(SendEmailOutput { success: false, message_id: None, error: Some(e.to_string()) }),
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
    import { module } from "apcore";

    export const sendEmail = module(
      {
        id: "email.send",
        description: "Send email to specified recipient",
        tags: ["email"],
      },
      async (input: { to: string; subject: string; body: string }, ctx) => {
        return { success: true, messageId: "msg_123", error: null };
      }
    );
    ```

=== "Rust"

    ```rust
    use apcore::module;

    #[module(id = "email.send", tags = ["email"])]
    /// Send email to specified recipient.
    pub fn send_email(input: SendEmailInput, ctx: &Context) -> ModuleResult<SendEmailOutput> {
        Ok(SendEmailOutput { success: true, message_id: Some("msg_123".into()), error: None })
    }
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
