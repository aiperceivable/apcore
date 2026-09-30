---
description: "Four ways to create apcore modules in Python, TypeScript, and Rust — class-based modules, module() registration of functions, wrapping existing methods, and YAML bindings that need no source changes."
---

# Creating Modules Guide

> Build modules that people and AI agents can discover, call, and govern.

## Choose Your Integration Path

apcore offers four ways to create a module. Pick the one that fits your situation:

| Approach | Use case | Changes to your code | Jump to |
|------|---------|-----------|------|
| **Class-based module** | New modules | You implement the module interface | [Quick Start](#quick-start) |
| **`module()` on a function** | Functions you own and can annotate | One decorator (Python) or one registration call (TypeScript, Rust) | [module() Registration](#module-registration) |
| **`module()` on an existing callable** | Existing functions or methods you import but do not edit | One registration call beside, not inside, the original code | [module() Registration](#module-registration) |
| **External binding (YAML)** | Existing code you cannot or will not change | None — a YAML file maps a module ID to a callable | [External Schema Binding](#external-schema-binding-yaml) |

The first two suit new code: you get full control over schemas and lifecycle. The last two are zero-intrusion: they wrap existing business logic **without rewriting it**.

---

## Designing for the AI Lifecycle

A module's metadata should guide an AI agent through each stage of using it:

| Stage | Field | Purpose |
| :--- | :--- | :--- |
| **1. Discovery** | `description` | Helps the agent find the right module for its intent |
| **2. Strategy** | `metadata` (`x-when-to-use`, …) | Tells the agent *when* and *how* to use the module |
| **3. Governance** | annotations (`requires_approval`, `destructive`, …) | Declares the safety boundary |
| **4. Recovery** | `ModuleError.ai_guidance` | Tells the agent how to recover from a failure |

**1. Discovery — describe the intent, not the implementation.** The `description` should answer "what problem does this solve?"

- Technical: "Executes a SQL SELECT query on the users table."
- Intent-oriented: "Find a user profile by email address or user ID."

**2. Strategy — share usage guidance in `metadata`.**

- `x-when-to-use`: the situations this module is the right choice for
- `x-when-not-to-use`: situations where another module should be used
- `x-common-mistakes`: pitfalls callers run into
- `x-workflow-hints`: steps that usually come before or after

**3. Governance — declare the risk.** Set `destructive: true` for irreversible operations and `requires_approval: true` for operations a human must confirm (spending money, deleting data). The executor's approval gate then asks your `ApprovalHandler` before the module runs; with no handler configured the gate is skipped with a warning. See the [Approval Flow cookbook](./cookbook-approval-flow.md).

**4. Recovery — say what to do next.** When a module fails, `message` says what happened and `ai_guidance` says what the agent should do about it:

| Field | Purpose | Example |
| :--- | :--- | :--- |
| `message` | What happened | `"Database connection failed"` |
| `ai_guidance` | What to do next | `"Retry after 5s. If it keeps failing, ask the user to check the DB credentials."` |
| `suggestion` | A concrete fix for a human | `"Verify DB_HOST and DB_PORT"` |
| `user_fixable` | Whether the end user can fix it | `true` |

Weak guidance restates the error (`"An error occurred while processing the request"`) or is too vague (`"Please try again"`). Good guidance is specific: `"Email format is invalid. Ask the user for an address like user@domain.com."`

---

## Quick Start

### 1. Project Layout

```text
my-project/
├── apcore.yaml                  # framework configuration (optional for this quick start)
├── extensions/                  # discovery root (extensions.root, default ./extensions)
│   └── executor/
│       └── email/
│           ├── send_email.py    # Python module, or
│           └── send_email.ts    # TypeScript module
└── schemas/                     # shared YAML schemas (optional)
```

Rust modules are compiled into your binary rather than discovered from files, so a Rust project keeps them in `src/` and registers them explicitly (step 4).

### 2. Write the Module

!!! note "What each SDK needs to recognize a module"
    - **Python** — a class defined in the file, with class attributes `input_schema` and `output_schema` that are Pydantic `BaseModel` subclasses, a non-empty `description` string, and an `execute(inputs, context)` method. Exactly one such class per file. `Module` is a `Protocol`, so inheriting from it is optional.
    - **TypeScript** — the file's default export (or its only matching named export) is an object with TypeBox `inputSchema` / `outputSchema`, a `description` string, and an `execute()` function — for example a `FunctionModule`.
    - **Rust** — a type that implements the `Module` trait, registered with `client.register()`.

=== "Python"

    ```python
    # extensions/executor/email/send_email.py
    from pydantic import BaseModel, Field

    from apcore import Context, Module


    class SendEmailInput(BaseModel):
        to: str = Field(..., description="Recipient email address")
        subject: str = Field(..., description="Email subject")
        body: str = Field(..., description="Email body")


    class SendEmailOutput(BaseModel):
        success: bool = Field(..., description="Whether the email was accepted for delivery")
        message_id: str | None = Field(None, description="Provider message ID")


    class SendEmailModule(Module):
        description = "Send an email to one recipient"
        input_schema = SendEmailInput
        output_schema = SendEmailOutput

        def execute(self, inputs: dict, context: Context) -> dict:
            # Implement the sending logic here
            return {"success": True, "message_id": "msg_123"}
    ```

=== "TypeScript"

    ```typescript
    // extensions/executor/email/send_email.ts
    import { Type } from '@sinclair/typebox';
    import { FunctionModule } from 'apcore-js';

    const SendEmailInput = Type.Object({
      to: Type.String({ description: 'Recipient email address' }),
      subject: Type.String({ description: 'Email subject' }),
      body: Type.String({ description: 'Email body' }),
    });

    const SendEmailOutput = Type.Object({
      success: Type.Boolean({ description: 'Whether the email was accepted for delivery' }),
      message_id: Type.Optional(Type.String({ description: 'Provider message ID' })),
    });

    export default new FunctionModule({
      moduleId: 'executor.email.send_email',
      description: 'Send an email to one recipient',
      inputSchema: SendEmailInput,
      outputSchema: SendEmailOutput,
      execute: async () => {
        // Implement the sending logic here
        return { success: true, message_id: 'msg_123' };
      },
    });
    ```

=== "Rust"

    ```rust
    // src/modules/send_email.rs
    use apcore::errors::{ErrorCode, ModuleError};
    use apcore::{Context, Module};
    use async_trait::async_trait;
    use serde::Deserialize;
    use serde_json::{json, Value};

    #[derive(Deserialize)]
    struct SendEmailInput {
        to: String,
        subject: String,
        body: String,
    }

    pub struct SendEmailModule;

    #[async_trait]
    impl Module for SendEmailModule {
        fn description(&self) -> &str {
            "Send an email to one recipient"
        }

        fn input_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {
                    "to":      { "type": "string", "description": "Recipient email address" },
                    "subject": { "type": "string", "description": "Email subject" },
                    "body":    { "type": "string", "description": "Email body" }
                },
                "required": ["to", "subject", "body"],
                "additionalProperties": false
            })
        }

        fn output_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {
                    "success":    { "type": "boolean", "description": "Whether the email was accepted for delivery" },
                    "message_id": { "type": ["string", "null"], "description": "Provider message ID" }
                },
                "required": ["success"]
            })
        }

        async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
            let input: SendEmailInput = serde_json::from_value(inputs)
                .map_err(|e| ModuleError::new(ErrorCode::GeneralInvalidInput, e.to_string()))?;
            // Implement the sending logic here
            let _ = (input.to, input.subject, input.body);
            Ok(json!({ "success": true, "message_id": "msg_123" }))
        }
    }
    ```

### 3. The Module ID Comes from the Path

```text
File:      extensions/executor/email/send_email.py   (or send_email.ts)
Module ID: executor.email.send_email
```

No configuration is needed — the path under the discovery root is the ID. Name files in snake_case: the TypeScript scanner uses the file name as-is, so `sendEmail.ts` would yield `executor.email.sendEmail`, which is not a legal ID and is skipped. In Rust, the ID is the one you pass to `register()`. The rules are in [protocol-spec §2.1](../spec/protocol-spec.md#21-directory-as-id-core-rule); to rename a file's ID without moving it, see [ID Map Configuration](./multi-language.md#5-id-map-configuration).

### 4. Call the Module

=== "Python"

    ```python
    from apcore import APCore

    client = APCore()      # discovery root defaults to ./extensions
    client.discover()

    result = client.call(
        "executor.email.send_email",
        {"to": "user@example.com", "subject": "Hello", "body": "World"},
    )
    print(result)  # {'success': True, 'message_id': 'msg_123'}
    ```

=== "TypeScript"

    ```typescript
    import { APCore } from 'apcore-js';

    const client = new APCore(); // discovery root defaults to ./extensions
    await client.discover();

    const result = await client.call('executor.email.send_email', {
      to: 'user@example.com',
      subject: 'Hello',
      body: 'World',
    });
    console.log(result); // { success: true, message_id: 'msg_123' }
    ```

    Discovery imports the `.ts` files at runtime, so run under a runtime that can load TypeScript (Node.js with type stripping, or `tsx`), or point discovery at your compiled `.js` output.

=== "Rust"

    ```rust
    use apcore::errors::ModuleError;
    use apcore::APCore;
    use serde_json::json;

    mod modules {
        pub mod send_email;
    }
    use modules::send_email::SendEmailModule;

    #[tokio::main]
    async fn main() -> Result<(), ModuleError> {
        let client = APCore::new();
        client.register("executor.email.send_email", Box::new(SendEmailModule))?;

        // call(module_id, inputs, context, version_hint)
        let result = client
            .call(
                "executor.email.send_email",
                json!({ "to": "user@example.com", "subject": "Hello", "body": "World" }),
                None,
                None,
            )
            .await?;
        println!("{result}");
        Ok(())
    }
    ```

To change the discovery root or other settings, put them in `apcore.yaml` and pass the loaded config: `APCore(config=Config.load("apcore.yaml"))`, `new APCore({ config: Config.load('apcore.yaml') })`, or `APCore::from_path("apcore.yaml")?`. See [Getting Started](../getting-started.md).

---

## Detailed Steps

### Step 1: Design the Schema

Decide the inputs and outputs before writing code:

| Question | Example (send email) |
|------|------------------|
| What inputs are needed? | `to`, `subject`, `body`, `cc`, `priority` |
| What does it return? | `success`, `message_id`, `error`, `sent_at` |
| What constraints apply? | `to` looks like an email address; `subject` is at most 200 characters |

Every field gets a `description` — it is what an AI agent reads to fill the field. Constrain values with `pattern`, `maxLength`, `enum`, and ranges rather than describing the limits in prose. The [Schema Definition Guide](./schema-definition.md) covers every field type and constraint.

### Step 2: Implement the Module

A complete module file with the schema from Step 1:

=== "Python"

    ```python
    # extensions/executor/email/send_email.py
    from datetime import datetime, timezone
    from typing import Literal

    from pydantic import BaseModel, Field

    from apcore import Context, Module


    class SendEmailInput(BaseModel):
        to: str = Field(..., description="Recipient email address", pattern=r"^[\w\.-]+@[\w\.-]+\.\w+$")
        subject: str = Field(..., description="Email subject", max_length=200)
        body: str = Field(..., description="Email body, plain text or HTML")
        cc: list[str] = Field(default_factory=list, description="CC addresses")
        priority: Literal["low", "normal", "high"] = Field("normal", description="Delivery priority")


    class SendEmailOutput(BaseModel):
        success: bool = Field(..., description="Whether the email was accepted for delivery")
        message_id: str | None = Field(None, description="Provider message ID when successful")
        error: str | None = Field(None, description="Error message when sending failed")
        sent_at: str | None = Field(None, description="Send time, ISO 8601")


    class SendEmailModule(Module):
        description = "Send an email via SMTP or an HTTP email API"
        input_schema = SendEmailInput
        output_schema = SendEmailOutput
        tags = ["email", "notification"]
        version = "1.0.0"

        def execute(self, inputs: dict, context: Context) -> dict:
            # `inputs` has already been validated against SendEmailInput.
            # model_validate() gives typed access and fills in defaults.
            params = SendEmailInput.model_validate(inputs)
            try:
                message_id = self._send(params)
            except OSError as exc:  # delivery failure is a business outcome, not a crash
                return {"success": False, "message_id": None, "error": str(exc), "sent_at": None}
            return {
                "success": True,
                "message_id": message_id,
                "error": None,
                "sent_at": datetime.now(timezone.utc).isoformat(),
            }

        def _send(self, params: SendEmailInput) -> str:
            # Replace with smtplib or your provider's client.
            return "msg_" + datetime.now(timezone.utc).strftime("%Y%m%d%H%M%S")
    ```

=== "TypeScript"

    ```typescript
    // extensions/executor/email/send_email.ts
    import { Type, type Static } from '@sinclair/typebox';
    import { FunctionModule } from 'apcore-js';

    const SendEmailInput = Type.Object({
      to: Type.String({ description: 'Recipient email address', pattern: '^[\\w\\.-]+@[\\w\\.-]+\\.\\w+$' }),
      subject: Type.String({ description: 'Email subject', maxLength: 200 }),
      body: Type.String({ description: 'Email body, plain text or HTML' }),
      cc: Type.Optional(Type.Array(Type.String(), { description: 'CC addresses', default: [] })),
      priority: Type.Optional(
        Type.Union([Type.Literal('low'), Type.Literal('normal'), Type.Literal('high')], {
          description: 'Delivery priority',
          default: 'normal',
        }),
      ),
    });

    const SendEmailOutput = Type.Object({
      success: Type.Boolean({ description: 'Whether the email was accepted for delivery' }),
      message_id: Type.Union([Type.String(), Type.Null()], { description: 'Provider message ID when successful' }),
      error: Type.Union([Type.String(), Type.Null()], { description: 'Error message when sending failed' }),
      sent_at: Type.Union([Type.String(), Type.Null()], { description: 'Send time, ISO 8601' }),
    });

    async function send(params: Static<typeof SendEmailInput>): Promise<string> {
      // Replace with nodemailer or your provider's client.
      return `msg_${Date.now()}`;
    }

    export default new FunctionModule({
      moduleId: 'executor.email.send_email',
      description: 'Send an email via SMTP or an HTTP email API',
      inputSchema: SendEmailInput,
      outputSchema: SendEmailOutput,
      tags: ['email', 'notification'],
      version: '1.0.0',
      execute: async (inputs) => {
        // `inputs` has already been validated against SendEmailInput.
        const params = inputs as Static<typeof SendEmailInput>;
        try {
          const messageId = await send(params);
          return { success: true, message_id: messageId, error: null, sent_at: new Date().toISOString() };
        } catch (e) {
          // Delivery failure is a business outcome, not a crash.
          return { success: false, message_id: null, error: e instanceof Error ? e.message : String(e), sent_at: null };
        }
      },
    });
    ```

=== "Rust"

    ```rust
    // src/modules/send_email.rs
    use apcore::errors::{ErrorCode, ModuleError};
    use apcore::{Context, Module};
    use async_trait::async_trait;
    use chrono::Utc;
    use serde::Deserialize;
    use serde_json::{json, Value};

    #[derive(Debug, Deserialize)]
    struct SendEmailInput {
        to: String,
        subject: String,
        body: String,
        #[serde(default)]
        cc: Vec<String>,
    }

    pub struct SendEmailModule;

    #[async_trait]
    impl Module for SendEmailModule {
        fn description(&self) -> &str {
            "Send an email via SMTP or an HTTP email API"
        }

        fn tags(&self) -> Vec<String> {
            vec!["email".into(), "notification".into()]
        }

        fn input_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {
                    "to":       { "type": "string", "description": "Recipient email address", "pattern": r"^[\w\.-]+@[\w\.-]+\.\w+$" },
                    "subject":  { "type": "string", "description": "Email subject", "maxLength": 200 },
                    "body":     { "type": "string", "description": "Email body, plain text or HTML" },
                    "cc":       { "type": "array", "items": { "type": "string" }, "description": "CC addresses", "default": [] },
                    "priority": { "type": "string", "enum": ["low", "normal", "high"], "description": "Delivery priority", "default": "normal" }
                },
                "required": ["to", "subject", "body"],
                "additionalProperties": false
            })
        }

        fn output_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {
                    "success":    { "type": "boolean", "description": "Whether the email was accepted for delivery" },
                    "message_id": { "type": ["string", "null"], "description": "Provider message ID when successful" },
                    "error":      { "type": ["string", "null"], "description": "Error message when sending failed" },
                    "sent_at":    { "type": ["string", "null"], "description": "Send time, ISO 8601" }
                },
                "required": ["success"],
                "additionalProperties": false
            })
        }

        async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
            // `inputs` has already been validated against input_schema().
            let params: SendEmailInput = serde_json::from_value(inputs)
                .map_err(|e| ModuleError::new(ErrorCode::GeneralInvalidInput, e.to_string()))?;
            match send(&params).await {
                Ok(message_id) => Ok(json!({
                    "success": true,
                    "message_id": message_id,
                    "error": null,
                    "sent_at": Utc::now().to_rfc3339(),
                })),
                // Delivery failure is a business outcome, not a crash.
                Err(e) => Ok(json!({ "success": false, "message_id": null, "error": e.to_string(), "sent_at": null })),
            }
        }
    }

    async fn send(params: &SendEmailInput) -> Result<String, std::io::Error> {
        // Replace with lettre or your provider's client.
        let _ = (&params.to, &params.subject, &params.body, &params.cc);
        Ok(format!("msg_{}", Utc::now().timestamp()))
    }
    ```

Python's input validation is strict and passes `execute()` the inputs as sent, which is why the example calls `model_validate()` for typed access; see [Schema Definition § 2.1](./schema-definition.md#21-in-code-recommended).

### Step 3: Organize by Layer

Group modules into the four layers; calls go downward only (`api` → `orchestrator` → `executor` → `common`):

```text
extensions/
├── api/                          # external entry points
│   └── handler/
│       └── user_api.py           → api.handler.user_api
├── orchestrator/                 # business flows
│   └── workflow/
│       └── user_register.py      → orchestrator.workflow.user_register
├── executor/                     # concrete actions and external calls
│   ├── email/
│   │   ├── send_email.py         → executor.email.send_email
│   │   └── send_template.py      → executor.email.send_template
│   └── database/
│       └── query.py              → executor.database.query
└── common/                       # shared utilities
    └── util/
        └── validator.py          → common.util.validator
```

| Layer | Responsibility | Examples |
|---|------|------|
| `api` | External request entry | HTTP handler, GraphQL resolver |
| `orchestrator` | Business orchestration, flow control | Registration flow, order processing |
| `executor` | Concrete execution, external calls | Send email, call an API, query a database |
| `common` | Shared utilities | Validators, formatters |

---

## Advanced Usage

### Using Context

Every call receives a `Context` carrying the trace ID, the caller, the call chain, the caller's identity, and a `data` map shared along the call chain.

=== "Python"

    ```python
    from pydantic import BaseModel, Field

    from apcore import Context


    class Empty(BaseModel):
        pass


    class Result(BaseModel):
        success: bool = Field(..., description="Always true")


    class ContextAwareModule:
        description = "Show what a module can read from its context"
        input_schema = Empty
        output_schema = Result

        def execute(self, inputs: dict, context: Context) -> dict:
            print(f"Trace ID:   {context.trace_id}")
            print(f"Caller:     {context.caller_id}")      # None for a top-level call
            print(f"Call chain: {context.call_chain}")

            if context.identity is not None:
                print(f"Identity:   {context.identity.id} ({context.identity.type})")

            custom = context.data.get("ext.myapp.tenant")   # shared along the call chain
            print(f"Tenant:     {custom}")
            return {"success": True}
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';
    import { FunctionModule, type Context } from 'apcore-js';

    export default new FunctionModule({
      moduleId: 'executor.demo.context_aware',
      description: 'Show what a module can read from its context',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ success: Type.Boolean({ description: 'Always true' }) }),
      execute: (_inputs, context: Context) => {
        console.log(`Trace ID:   ${context.traceId}`);
        console.log(`Caller:     ${context.callerId}`); // null for a top-level call
        console.log(`Call chain: ${context.callChain.join(' -> ')}`);

        if (context.identity) {
          console.log(`Identity:   ${context.identity.id} (${context.identity.type})`);
        }

        const tenant = context.data['ext.myapp.tenant']; // shared along the call chain
        console.log(`Tenant:     ${String(tenant)}`);
        return { success: true };
      },
    });
    ```

=== "Rust"

    ```rust
    use apcore::errors::ModuleError;
    use apcore::{Context, Module};
    use async_trait::async_trait;
    use serde_json::{json, Value};

    pub struct ContextAwareModule;

    #[async_trait]
    impl Module for ContextAwareModule {
        fn description(&self) -> &str {
            "Show what a module can read from its context"
        }

        fn input_schema(&self) -> Value {
            json!({ "type": "object", "properties": {} })
        }

        fn output_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": { "success": { "type": "boolean", "description": "Always true" } },
                "required": ["success"]
            })
        }

        async fn execute(&self, _inputs: Value, ctx: &Context<Value>) -> Result<Value, ModuleError> {
            println!("Trace ID:   {}", ctx.trace_id);
            println!("Caller:     {:?}", ctx.caller_id); // None for a top-level call
            println!("Call chain: {:?}", ctx.call_chain);

            if let Some(identity) = &ctx.identity {
                println!("Identity:   {} ({})", identity.id(), identity.identity_type());
            }

            // Shared along the call chain. Clone the value so the lock guard
            // is dropped before any .await.
            let tenant = ctx.data.read().get("ext.myapp.tenant").cloned();
            println!("Tenant:     {tenant:?}");
            Ok(json!({ "success": true }))
        }
    }
    ```

Keys your application writes to `context.data` use the `ext.<vendor>.<field>` namespace; `_apcore.*` keys belong to the framework. `context.data` is visible to every module and middleware along the call chain, and `x-sensitive` marks schema fields rather than `data` entries, so keep secrets out of it — see [Context Object](../features/context-object.md).

### Calling Other Modules

An orchestrator module calls other modules through the executor, passing its own context so the nested call extends the call chain and is subject to ACL, approval, and call-depth checks.

=== "Python"

    ```python
    from pydantic import BaseModel, Field

    from apcore import Context


    class RegisterInput(BaseModel):
        email: str = Field(..., description="New user's email address")


    class RegisterOutput(BaseModel):
        user_id: str = Field(..., description="Created user ID")
        email_sent: bool = Field(..., description="Whether the welcome email was sent")


    class UserRegisterModule:
        description = "Register a new user and send a welcome email"
        input_schema = RegisterInput
        output_schema = RegisterOutput

        def execute(self, inputs: dict, context: Context) -> dict:
            user_id = "user_123"  # create the user here

            email_result = context.executor.call(
                "executor.email.send_email",
                {"to": inputs["email"], "subject": "Welcome", "body": "Welcome to the platform!"},
                context,  # propagate the context to keep the call chain
            )
            return {"user_id": user_id, "email_sent": email_result["success"]}
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';
    import { FunctionModule, type Context, type Executor } from 'apcore-js';

    export default new FunctionModule({
      moduleId: 'orchestrator.user.register',
      description: 'Register a new user and send a welcome email',
      inputSchema: Type.Object({
        email: Type.String({ description: "New user's email address" }),
      }),
      outputSchema: Type.Object({
        user_id: Type.String({ description: 'Created user ID' }),
        email_sent: Type.Boolean({ description: 'Whether the welcome email was sent' }),
      }),
      execute: async (inputs, context: Context) => {
        const userId = 'user_123'; // create the user here

        const executor = context.executor as Executor;
        const emailResult = await executor.call(
          'executor.email.send_email',
          { to: inputs.email as string, subject: 'Welcome', body: 'Welcome to the platform!' },
          context, // propagate the context to keep the call chain
        );
        return { user_id: userId, email_sent: emailResult.success === true };
      },
    });
    ```

=== "Rust"

    ```rust
    use std::sync::{Arc, Weak};

    use apcore::errors::{ErrorCode, ModuleError};
    use apcore::{Config, Context, Executor, Module, Registry};
    use async_trait::async_trait;
    use serde_json::{json, Value};

    /// A module that calls another module holds a handle to the Executor it
    /// runs under. `Weak` avoids a reference cycle (registry → module → executor).
    pub struct UserRegisterModule {
        executor: Weak<Executor>,
    }

    #[async_trait]
    impl Module for UserRegisterModule {
        fn description(&self) -> &str {
            "Register a new user and send a welcome email"
        }

        fn input_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": { "email": { "type": "string", "description": "New user's email address" } },
                "required": ["email"]
            })
        }

        fn output_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {
                    "user_id":    { "type": "string", "description": "Created user ID" },
                    "email_sent": { "type": "boolean", "description": "Whether the welcome email was sent" }
                },
                "required": ["user_id", "email_sent"]
            })
        }

        async fn execute(&self, inputs: Value, ctx: &Context<Value>) -> Result<Value, ModuleError> {
            let executor = self.executor.upgrade().ok_or_else(|| {
                ModuleError::new(ErrorCode::GeneralInternalError, "executor has been dropped")
            })?;
            let user_id = "user_123"; // create the user here

            let email_result = executor
                .call(
                    "executor.email.send_email",
                    json!({ "to": inputs["email"], "subject": "Welcome", "body": "Welcome to the platform!" }),
                    Some(ctx), // propagate the context to keep the call chain
                    None,
                )
                .await?;
            Ok(json!({
                "user_id": user_id,
                "email_sent": email_result["success"].as_bool().unwrap_or(false),
            }))
        }
    }

    /// Wiring: build the Registry and Executor yourself so the module can hold
    /// a handle to the same Executor that runs it.
    pub fn build(send_email: Box<dyn Module>) -> Result<Arc<Executor>, ModuleError> {
        let registry = Arc::new(Registry::new());
        let executor = Arc::new(Executor::new(Arc::clone(&registry), Config::default()));
        registry.register_module("executor.email.send_email", send_email)?;
        registry.register_module(
            "orchestrator.user.register",
            Box::new(UserRegisterModule { executor: Arc::downgrade(&executor) }),
        )?;
        Ok(executor)
    }
    ```

    In Rust `ctx.executor` is an identity handle, not a callable executor, so a module that makes nested calls keeps its own `Weak<Executor>`. Nested calls must go through the same Executor that is running the parent call.

### Async Modules

- **Python** — declare `async def execute(...)`; the executor detects coroutines and awaits them. Synchronous `execute` methods are fine for CPU-bound or blocking work.
- **TypeScript** — `execute` may return a value or a `Promise`.
- **Rust** — `execute` is always `async` (`#[async_trait]`).

[Pattern 1](#pattern-1-external-api-call) shows an asynchronous module in all three languages.

### Resource Management

Open long-lived resources when the module is registered and release them when it is unregistered. `on_load()` runs before the module becomes callable — raising from it aborts registration — and `on_unload()` runs after it is removed.

=== "Python"

    ```python
    import json
    from typing import TextIO

    from pydantic import BaseModel, Field

    from apcore import Context


    class AppendInput(BaseModel):
        event: str = Field(..., description="Event name")
        detail: str = Field("", description="Free-text detail")


    class AppendOutput(BaseModel):
        written: bool = Field(..., description="Whether the record was written")


    class AppendRecordModule:
        description = "Append an audit record to the local audit log"
        input_schema = AppendInput
        output_schema = AppendOutput

        def __init__(self) -> None:
            self._file: TextIO | None = None

        def on_load(self) -> None:
            # Synchronous; runs when the module is registered.
            self._file = open("audit.jsonl", "a", encoding="utf-8")

        def on_unload(self) -> None:
            if self._file is not None:
                self._file.close()
                self._file = None

        def execute(self, inputs: dict, context: Context) -> dict:
            if self._file is None:
                raise RuntimeError("audit log is not open")
            self._file.write(json.dumps({"trace_id": context.trace_id, **inputs}) + "\n")
            self._file.flush()
            return {"written": True}
    ```

=== "TypeScript"

    ```typescript
    import { open, type FileHandle } from 'node:fs/promises';
    import { Type } from '@sinclair/typebox';
    import type { Context, Module } from 'apcore-js';

    class AppendRecordModule implements Module {
      readonly description = 'Append an audit record to the local audit log';
      readonly inputSchema = Type.Object({
        event: Type.String({ description: 'Event name' }),
        detail: Type.Optional(Type.String({ description: 'Free-text detail', default: '' })),
      });
      readonly outputSchema = Type.Object({
        written: Type.Boolean({ description: 'Whether the record was written' }),
      });

      private file: FileHandle | null = null;

      async onLoad(): Promise<void> {
        this.file = await open('audit.jsonl', 'a');
      }

      async onUnload(): Promise<void> {
        await this.file?.close();
        this.file = null;
      }

      async execute(inputs: Record<string, unknown>, context: Context): Promise<Record<string, unknown>> {
        if (!this.file) throw new Error('audit log is not open');
        await this.file.appendFile(`${JSON.stringify({ trace_id: context.traceId, ...inputs })}\n`);
        return { written: true };
      }
    }

    export default new AppendRecordModule();
    ```

=== "Rust"

    ```rust
    use std::fs::{File, OpenOptions};
    use std::io::Write;
    use std::sync::Mutex;

    use apcore::errors::{ErrorCode, ModuleError};
    use apcore::{Context, Module};
    use async_trait::async_trait;
    use serde_json::{json, Value};

    pub struct AppendRecordModule {
        file: Mutex<Option<File>>,
    }

    impl AppendRecordModule {
        pub fn new() -> Self {
            Self { file: Mutex::new(None) }
        }
    }

    #[async_trait]
    impl Module for AppendRecordModule {
        fn description(&self) -> &str {
            "Append an audit record to the local audit log"
        }

        fn input_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {
                    "event":  { "type": "string", "description": "Event name" },
                    "detail": { "type": "string", "description": "Free-text detail", "default": "" }
                },
                "required": ["event"]
            })
        }

        fn output_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": { "written": { "type": "boolean", "description": "Whether the record was written" } },
                "required": ["written"]
            })
        }

        fn on_load(&self) -> Result<(), ModuleError> {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open("audit.jsonl")
                .map_err(|e| ModuleError::new(ErrorCode::ModuleLoadError, e.to_string()))?;
            *self.file.lock().unwrap() = Some(file);
            Ok(())
        }

        fn on_unload(&self) {
            // Dropping the File closes it.
            self.file.lock().unwrap().take();
        }

        async fn execute(&self, inputs: Value, ctx: &Context<Value>) -> Result<Value, ModuleError> {
            let mut guard = self.file.lock().unwrap(); // no .await while the guard is held
            let file = guard.as_mut().ok_or_else(|| {
                ModuleError::new(ErrorCode::GeneralInternalError, "audit log is not open")
            })?;
            let record = json!({ "trace_id": ctx.trace_id, "event": inputs["event"], "detail": inputs["detail"] });
            writeln!(file, "{record}")
                .map_err(|e| ModuleError::new(ErrorCode::ModuleExecuteError, e.to_string()))?;
            Ok(json!({ "written": true }))
        }
    }
    ```

---

## Common Patterns

### Pattern 1: External API Call

Always put a timeout on outbound calls. The executor also enforces `executor.default_timeout` per call.

=== "Python"

    ```python
    import httpx
    from pydantic import BaseModel, Field

    from apcore import Context


    class WeatherInput(BaseModel):
        city: str = Field(..., description="City name")


    class WeatherOutput(BaseModel):
        temperature: float = Field(..., description="Temperature in Celsius")
        summary: str = Field(..., description="Weather description")


    class WeatherModule:
        description = "Get the current weather for a city"
        input_schema = WeatherInput
        output_schema = WeatherOutput

        async def execute(self, inputs: dict, context: Context) -> dict:
            async with httpx.AsyncClient(timeout=5.0) as client:
                resp = await client.get("https://api.weather.example/v1/current", params={"city": inputs["city"]})
                resp.raise_for_status()
                data = resp.json()
            return {"temperature": data["temp"], "summary": data["desc"]}
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';
    import { FunctionModule } from 'apcore-js';

    export default new FunctionModule({
      moduleId: 'executor.weather.current',
      description: 'Get the current weather for a city',
      inputSchema: Type.Object({ city: Type.String({ description: 'City name' }) }),
      outputSchema: Type.Object({
        temperature: Type.Number({ description: 'Temperature in Celsius' }),
        summary: Type.String({ description: 'Weather description' }),
      }),
      execute: async (inputs) => {
        const url = new URL('https://api.weather.example/v1/current');
        url.searchParams.set('city', inputs.city as string);
        const resp = await fetch(url, { signal: AbortSignal.timeout(5_000) });
        if (!resp.ok) throw new Error(`weather API returned ${resp.status}`);
        const data = (await resp.json()) as { temp: number; desc: string };
        return { temperature: data.temp, summary: data.desc };
      },
    });
    ```

=== "Rust"

    ```rust
    use std::time::Duration;

    use apcore::errors::{ErrorCode, ModuleError};
    use apcore::{Context, Module};
    use async_trait::async_trait;
    use serde::Deserialize;
    use serde_json::{json, Value};

    #[derive(Deserialize)]
    struct WeatherResponse {
        temp: f64,
        desc: String,
    }

    pub struct WeatherModule {
        client: reqwest::Client,
    }

    impl WeatherModule {
        pub fn new() -> Result<Self, reqwest::Error> {
            let client = reqwest::Client::builder().timeout(Duration::from_secs(5)).build()?;
            Ok(Self { client })
        }
    }

    fn upstream(e: reqwest::Error) -> ModuleError {
        ModuleError::new(ErrorCode::ModuleExecuteError, e.to_string())
    }

    #[async_trait]
    impl Module for WeatherModule {
        fn description(&self) -> &str {
            "Get the current weather for a city"
        }

        fn input_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": { "city": { "type": "string", "description": "City name" } },
                "required": ["city"]
            })
        }

        fn output_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {
                    "temperature": { "type": "number", "description": "Temperature in Celsius" },
                    "summary":     { "type": "string", "description": "Weather description" }
                },
                "required": ["temperature", "summary"]
            })
        }

        async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
            let city = inputs["city"].as_str().unwrap_or_default();
            let data: WeatherResponse = self
                .client
                .get("https://api.weather.example/v1/current")
                .query(&[("city", city)])
                .send()
                .await
                .map_err(upstream)?
                .error_for_status()
                .map_err(upstream)?
                .json()
                .await
                .map_err(upstream)?;
            Ok(json!({ "temperature": data.temp, "summary": data.desc }))
        }
    }
    ```

### Pattern 2: Retrying a Flaky Dependency

Retry transient failures of an outbound call inside the module, with a bounded number of attempts and exponential backoff. (To retry whole module calls instead, use the built-in `RetryMiddleware` — see [Writing Middleware](./middleware.md).)

=== "Python"

    ```python
    import time

    from pydantic import BaseModel, Field

    from apcore import Context


    class NotifyInput(BaseModel):
        to: str = Field(..., description="Recipient address")
        text: str = Field(..., description="Message text")


    class NotifyOutput(BaseModel):
        success: bool = Field(..., description="Whether the message was delivered")
        error: str | None = Field(None, description="Last error when delivery failed")


    class ReliableNotifyModule:
        description = "Send a notification, retrying transient failures up to three times"
        input_schema = NotifyInput
        output_schema = NotifyOutput

        def execute(self, inputs: dict, context: Context) -> dict:
            last_error: Exception | None = None
            for attempt in range(3):
                try:
                    self._send(inputs)
                    return {"success": True, "error": None}
                except ConnectionError as exc:
                    last_error = exc
                    time.sleep(min(2**attempt, 10))  # 1s, 2s, 4s
            return {"success": False, "error": str(last_error)}

        def _send(self, inputs: dict) -> None:
            """Call the notification provider here."""
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';
    import { FunctionModule } from 'apcore-js';

    async function withRetry<T>(fn: () => Promise<T>, attempts = 3, baseDelayMs = 1_000): Promise<T> {
      let lastError: unknown;
      for (let i = 0; i < attempts; i++) {
        try {
          return await fn();
        } catch (e) {
          lastError = e;
          await new Promise((resolve) => setTimeout(resolve, Math.min(baseDelayMs * 2 ** i, 10_000)));
        }
      }
      throw lastError;
    }

    async function send(_to: string, _text: string): Promise<void> {
      // Call the notification provider here.
    }

    export default new FunctionModule({
      moduleId: 'executor.notify.reliable_send',
      description: 'Send a notification, retrying transient failures up to three times',
      inputSchema: Type.Object({
        to: Type.String({ description: 'Recipient address' }),
        text: Type.String({ description: 'Message text' }),
      }),
      outputSchema: Type.Object({
        success: Type.Boolean({ description: 'Whether the message was delivered' }),
        error: Type.Union([Type.String(), Type.Null()], { description: 'Last error when delivery failed' }),
      }),
      execute: async (inputs) => {
        try {
          await withRetry(() => send(inputs.to as string, inputs.text as string));
          return { success: true, error: null };
        } catch (e) {
          return { success: false, error: e instanceof Error ? e.message : String(e) };
        }
      },
    });
    ```

=== "Rust"

    ```rust
    use std::future::Future;
    use std::time::Duration;

    use apcore::errors::ModuleError;
    use apcore::{Context, Module};
    use async_trait::async_trait;
    use serde_json::{json, Value};

    async fn with_retry<F, Fut, T, E>(mut f: F, attempts: u32, base: Duration) -> Result<T, E>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let mut last_err = None;
        for i in 0..attempts {
            match f().await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    last_err = Some(e);
                    tokio::time::sleep((base * 2u32.pow(i)).min(Duration::from_secs(10))).await;
                }
            }
        }
        Err(last_err.expect("attempts must be at least 1"))
    }

    async fn send(_to: &str, _text: &str) -> Result<(), std::io::Error> {
        // Call the notification provider here.
        Ok(())
    }

    pub struct ReliableNotifyModule;

    #[async_trait]
    impl Module for ReliableNotifyModule {
        fn description(&self) -> &str {
            "Send a notification, retrying transient failures up to three times"
        }

        fn input_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {
                    "to":   { "type": "string", "description": "Recipient address" },
                    "text": { "type": "string", "description": "Message text" }
                },
                "required": ["to", "text"]
            })
        }

        fn output_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {
                    "success": { "type": "boolean", "description": "Whether the message was delivered" },
                    "error":   { "type": ["string", "null"], "description": "Last error when delivery failed" }
                },
                "required": ["success", "error"]
            })
        }

        async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
            let to = inputs["to"].as_str().unwrap_or_default();
            let text = inputs["text"].as_str().unwrap_or_default();
            match with_retry(|| send(to, text), 3, Duration::from_secs(1)).await {
                Ok(()) => Ok(json!({ "success": true, "error": null })),
                Err(e) => Ok(json!({ "success": false, "error": e.to_string() })),
            }
        }
    }
    ```

---

## Best Practices

### Schema Design

Give every field a `description` and real constraints; prefer enums to free text. The [Schema Definition Guide](./schema-definition.md#9-best-practices) has good/bad pairs for each SDK.

### Error Handling

Return expected business outcomes (a rejected payment, an undeliverable email) as normal output. Raise a `ModuleError` with `ai_guidance` when the call cannot succeed as asked. Other exceptions are wrapped by the executor as `MODULE_EXECUTE_ERROR`, which gives the caller nothing to act on.

=== "Python"

    ```python
    from pydantic import BaseModel, Field

    from apcore import Context
    from apcore.errors import ModuleError


    class LookupInput(BaseModel):
        email: str = Field(..., description="Email address of the user to find")


    class LookupOutput(BaseModel):
        user_id: str = Field(..., description="Matching user ID")


    class FindUserModule:
        description = "Find a user by email address"
        input_schema = LookupInput
        output_schema = LookupOutput

        def execute(self, inputs: dict, context: Context) -> dict:
            email = inputs["email"]
            if "@" not in email:
                raise ModuleError(
                    code="USER_EMAIL_INVALID",
                    message=f"'{email}' is not an email address",
                    ai_guidance="Ask the user for an email address like user@domain.com, then retry.",
                    user_fixable=True,
                )
            return {"user_id": "user_123"}
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';
    import { FunctionModule, ModuleError } from 'apcore-js';

    export default new FunctionModule({
      moduleId: 'executor.user.find_by_email',
      description: 'Find a user by email address',
      inputSchema: Type.Object({ email: Type.String({ description: 'Email address of the user to find' }) }),
      outputSchema: Type.Object({ user_id: Type.String({ description: 'Matching user ID' }) }),
      execute: (inputs) => {
        const email = inputs.email as string;
        if (!email.includes('@')) {
          // (code, message, details, cause, traceId, retryable, aiGuidance, userFixable)
          throw new ModuleError(
            'USER_EMAIL_INVALID',
            `'${email}' is not an email address`,
            {},
            undefined,
            undefined,
            false,
            'Ask the user for an email address like user@domain.com, then retry.',
            true,
          );
        }
        return { user_id: 'user_123' };
      },
    });
    ```

=== "Rust"

    ```rust
    use apcore::errors::{ErrorCode, ModuleError};
    use apcore::{Context, Module};
    use async_trait::async_trait;
    use serde_json::{json, Value};

    pub struct FindUserModule;

    #[async_trait]
    impl Module for FindUserModule {
        fn description(&self) -> &str {
            "Find a user by email address"
        }

        fn input_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": { "email": { "type": "string", "description": "Email address of the user to find" } },
                "required": ["email"]
            })
        }

        fn output_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": { "user_id": { "type": "string", "description": "Matching user ID" } },
                "required": ["user_id"]
            })
        }

        async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
            let email = inputs["email"].as_str().unwrap_or_default();
            if !email.contains('@') {
                return Err(ModuleError::new(
                    ErrorCode::GeneralInvalidInput,
                    format!("'{email}' is not an email address"),
                )
                .with_ai_guidance("Ask the user for an email address like user@domain.com, then retry.")
                .with_user_fixable(true));
            }
            Ok(json!({ "user_id": "user_123" }))
        }
    }
    ```

Python and TypeScript modules may use their own error codes (avoid the framework's prefixes); Rust modules choose from `ErrorCode`. The error model is described in [Error System](../features/error-system.md).

### Single Responsibility

Give each module one job — validate an email, render a template, send a message — and compose them in an `orchestrator.*` module. Small modules are easier for an agent to choose between, easier to govern with ACL rules, and easier to test. A single `email.everything` module that validates, renders, and sends makes all three harder.

---

## Testing

Modules are plain objects, so you can call `execute()` directly with a test context, or run them through a real client to exercise validation, ACL, and middleware. The [Testing Modules Guide](./testing-modules.md) covers unit tests, schema tests, and integration tests in pytest, Vitest, and Rust. ACL rules are tested as described in [ACL Configuration](./acl-configuration.md).

### Debugging Tips

1. **Follow the trace ID.** Every log line and span for one request carries the same `trace_id`.
2. **Read the call chain.** `context.call_chain` shows how a nested call was reached.
3. **Dry-run a call.** `client.validate(module_id, inputs)` runs the pre-execution checks — call-chain guard, module lookup, ACL, approval requirement, input schema — plus the module's own `preflight()`, without executing anything.
4. **Log calls.** Add the built-in `LoggingMiddleware` with `client.use(...)` to log inputs and outputs of every call.

### Performance Tips

| Recommendation | Explanation |
|------|------|
| Reuse connections | Open pools and clients in `on_load()` and close them in `on_unload()` |
| Don't block the event loop | Use `async def execute()` in Python for I/O-bound work |
| Keep `context.data` small | It is shared along the whole call chain |
| Set timeouts | Give outbound calls their own timeout; `executor.default_timeout` bounds the whole call |
| Make retried work idempotent | Mark a module `idempotent` only when repeating a call is safe |

---

## module() Registration

`module()` turns a function into a module without writing a class. Python infers the schemas from type hints; TypeScript and Rust take them explicitly because type information is not available at runtime. The contract is in [protocol-spec §5.11](../spec/protocol-spec.md#511-function-based-module-definition-function-based-module-definition).

### Registering a Function

Given an ordinary `send_email(to, subject, body)` function, one registration turns it into a module:

=== "Python"

    ```python
    from apcore import APCore

    client = APCore()


    @client.module(id="email.send", tags=["email", "notification"])
    def send_email(to: str, subject: str, body: str) -> dict:
        """Send an email."""
        # Business logic unchanged.
        return {"success": True, "message_id": "msg_123"}


    print(client.call("email.send", {"to": "user@example.com", "subject": "Hi", "body": "Hello"}))
    ```

    The input schema comes from the parameter types, the output schema from the return type, and the description from the first line of the docstring. `@apcore.module(...)` does the same on the process-wide default client.

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';
    import { APCore } from 'apcore-js';

    const client = new APCore();

    client.module({
      id: 'email.send',
      description: 'Send an email',
      tags: ['email', 'notification'],
      inputSchema: Type.Object({
        to: Type.String({ description: 'Recipient email address' }),
        subject: Type.String({ description: 'Email subject' }),
        body: Type.String({ description: 'Email body' }),
      }),
      outputSchema: Type.Object({
        success: Type.Boolean({ description: 'Whether the email was accepted' }),
        message_id: Type.String({ description: 'Provider message ID' }),
      }),
      execute: () => ({ success: true, message_id: 'msg_123' }),
    });

    console.log(await client.call('email.send', { to: 'user@example.com', subject: 'Hi', body: 'Hello' }));
    ```

=== "Rust"

    ```rust
    use apcore::errors::ModuleError;
    use apcore::APCore;
    use serde_json::json;

    pub fn register(client: &mut APCore) -> Result<(), ModuleError> {
        client.module(
            "email.send",
            "Send an email",
            json!({
                "type": "object",
                "properties": {
                    "to":      { "type": "string", "description": "Recipient email address" },
                    "subject": { "type": "string", "description": "Email subject" },
                    "body":    { "type": "string", "description": "Email body" }
                },
                "required": ["to", "subject", "body"]
            }),
            json!({
                "type": "object",
                "properties": {
                    "success":    { "type": "boolean", "description": "Whether the email was accepted" },
                    "message_id": { "type": "string", "description": "Provider message ID" }
                },
                "required": ["success", "message_id"]
            }),
            None,                                        // documentation
            vec!["email".into(), "notification".into()], // tags
            None,                                        // version (default "1.0.0")
            None,                                        // metadata
            vec![],                                      // examples
            None,                                        // display
            |_inputs, _ctx| Box::pin(async move { Ok(json!({ "success": true, "message_id": "msg_123" })) }),
        )?;
        Ok(())
    }
    ```

### Wrapping Existing Methods

To register functions or methods you do not want to touch, call the function form beside them. In Python that is `apcore.decorator.module(func, id=...)`, which returns a module object you register yourself.

=== "Python"

    ```python
    from apcore import APCore
    from apcore.decorator import module


    # Existing business code, unchanged
    class EmailService:
        def send(self, to: str, subject: str, body: str) -> dict:
            """Send an email."""
            return {"success": True}

        def send_template(self, template_id: str, data: dict) -> dict:
            """Send an email rendered from a template."""
            return {"success": True}


    service = EmailService()
    client = APCore()

    # module(func, id=...) returns a FunctionModule; the method itself is not modified.
    client.register("email.send", module(service.send, id="email.send"))
    client.register("email.send_template", module(service.send_template, id="email.send_template"))
    ```

    Use this function form for bound methods. The decorator form (`@client.module(...)`) attaches the module to the function object, which a bound method does not allow.

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';
    import { APCore } from 'apcore-js';

    // Existing business code, unchanged
    class EmailService {
      send(to: string, subject: string, body: string) {
        return { success: true };
      }
      sendTemplate(templateId: string, data: Record<string, unknown>) {
        return { success: true };
      }
    }

    const service = new EmailService();
    const client = new APCore();

    client.module({
      id: 'email.send',
      description: 'Send an email',
      inputSchema: Type.Object({
        to: Type.String({ description: 'Recipient email address' }),
        subject: Type.String({ description: 'Email subject' }),
        body: Type.String({ description: 'Email body' }),
      }),
      outputSchema: Type.Object({ success: Type.Boolean({ description: 'Whether the email was accepted' }) }),
      execute: (inputs) => service.send(inputs.to as string, inputs.subject as string, inputs.body as string),
    });

    client.module({
      id: 'email.send_template',
      description: 'Send an email rendered from a template',
      inputSchema: Type.Object({
        template_id: Type.String({ description: 'Template ID' }),
        data: Type.Record(Type.String(), Type.Unknown(), { description: 'Template variables' }),
      }),
      outputSchema: Type.Object({ success: Type.Boolean({ description: 'Whether the email was accepted' }) }),
      execute: (inputs) =>
        service.sendTemplate(inputs.template_id as string, inputs.data as Record<string, unknown>),
    });
    ```

=== "Rust"

    ```rust
    use std::sync::Arc;

    use apcore::errors::ModuleError;
    use apcore::APCore;
    use serde_json::{json, Value};

    // Existing business code, unchanged
    pub struct EmailService;

    impl EmailService {
        pub fn send(&self, _to: &str, _subject: &str, _body: &str) -> Value {
            json!({ "success": true })
        }
    }

    pub fn register(client: &mut APCore, service: Arc<EmailService>) -> Result<(), ModuleError> {
        client.module(
            "email.send",
            "Send an email",
            json!({
                "type": "object",
                "properties": {
                    "to":      { "type": "string", "description": "Recipient email address" },
                    "subject": { "type": "string", "description": "Email subject" },
                    "body":    { "type": "string", "description": "Email body" }
                },
                "required": ["to", "subject", "body"]
            }),
            json!({
                "type": "object",
                "properties": { "success": { "type": "boolean", "description": "Whether the email was accepted" } }
            }),
            None,
            vec![],
            None,
            None,
            vec![],
            None,
            move |inputs, _ctx| {
                let service = Arc::clone(&service);
                Box::pin(async move {
                    Ok(service.send(
                        inputs["to"].as_str().unwrap_or_default(),
                        inputs["subject"].as_str().unwrap_or_default(),
                        inputs["body"].as_str().unwrap_or_default(),
                    ))
                })
            },
        )?;
        Ok(())
    }
    ```

### Annotations, Context, and Async

=== "Python"

    ```python
    from typing import Annotated

    from pydantic import Field

    from apcore import APCore, Context

    client = APCore()


    @client.module(
        id="email.send",
        annotations={"open_world": True, "idempotent": False},
        tags=["email"],
    )
    async def send_email(
        to: Annotated[str, Field(description="Recipient email", pattern=r"^[\w\.-]+@[\w\.-]+\.\w+$")],
        subject: Annotated[str, Field(description="Email subject", max_length=200)],
        body: Annotated[str, Field(description="Email body")],
        context: Context,
        cc: Annotated[list[str] | None, Field(description="CC list")] = None,
    ) -> dict:
        """Send an email over SMTP."""
        print(f"trace_id: {context.trace_id}")
        return {"success": True, "message_id": "msg_123"}
    ```

    A parameter annotated exactly `Context` receives the call context and is left out of the input schema. It must be `Context`, not `Context | None`. `async def` functions are awaited automatically.

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';
    import { APCore, createAnnotations, type Context } from 'apcore-js';

    const client = new APCore();

    client.module({
      id: 'email.send',
      description: 'Send an email over SMTP',
      tags: ['email'],
      annotations: createAnnotations({ openWorld: true, idempotent: false }),
      inputSchema: Type.Object({
        to: Type.String({ description: 'Recipient email', pattern: '^[\\w\\.-]+@[\\w\\.-]+\\.\\w+$' }),
        subject: Type.String({ description: 'Email subject', maxLength: 200 }),
        body: Type.String({ description: 'Email body' }),
        cc: Type.Optional(Type.Array(Type.String(), { description: 'CC list' })),
      }),
      outputSchema: Type.Object({
        success: Type.Boolean({ description: 'Whether the email was accepted' }),
        message_id: Type.String({ description: 'Provider message ID' }),
      }),
      execute: async (_inputs, context: Context) => {
        console.log(`trace_id: ${context.traceId}`);
        return { success: true, message_id: 'msg_123' };
      },
    });
    ```

=== "Rust"

    ```rust
    use std::collections::HashMap;

    use apcore::errors::ModuleError;
    use apcore::{APCore, FunctionModule, ModuleAnnotations};
    use serde_json::json;

    // `client.module()` registers default annotations, so build a
    // FunctionModule when a module needs its own.
    pub fn register(client: &APCore) -> Result<(), ModuleError> {
        let annotations = ModuleAnnotations {
            open_world: true,
            idempotent: false,
            ..Default::default()
        };

        let module = FunctionModule::with_description(
            annotations,
            json!({
                "type": "object",
                "properties": {
                    "to":      { "type": "string", "description": "Recipient email", "pattern": r"^[\w\.-]+@[\w\.-]+\.\w+$" },
                    "subject": { "type": "string", "description": "Email subject", "maxLength": 200 },
                    "body":    { "type": "string", "description": "Email body" },
                    "cc":      { "type": "array", "items": { "type": "string" }, "description": "CC list" }
                },
                "required": ["to", "subject", "body"]
            }),
            json!({
                "type": "object",
                "properties": {
                    "success":    { "type": "boolean", "description": "Whether the email was accepted" },
                    "message_id": { "type": "string", "description": "Provider message ID" }
                },
                "required": ["success", "message_id"]
            }),
            "Send an email over SMTP",
            None,                // documentation
            vec!["email".into()], // tags
            "1.0.0",             // version
            HashMap::new(),      // metadata
            vec![],              // examples
            |_inputs, ctx| {
                let trace_id = ctx.trace_id.clone();
                Box::pin(async move {
                    println!("trace_id: {trace_id}");
                    Ok(json!({ "success": true, "message_id": "msg_123" }))
                })
            },
        );
        client.register("email.send", Box::new(module))
    }
    ```

### How IDs and Descriptions Are Chosen

| | Python | TypeScript | Rust |
|---|---|---|---|
| **Module ID** | `id=` if given; otherwise generated from the function's `__module__` and `__qualname__` | `id` is required | The `module_id` argument |
| **Description** | `description=` if given; otherwise the docstring's first line; otherwise `"Module <function name>"` | `description` if given; otherwise `"Module <id>"` | The `description` argument |
| **Schemas** | Inferred from type hints (every parameter and the return value need one) | `inputSchema` / `outputSchema` (TypeBox) | `input_schema` / `output_schema` (JSON values) |

### Class-Based Modules vs. module()

| Capability | Class-based module | `module()` |
|------|------------|----------|
| Lifecycle hooks (`on_load` / `on_unload`) | Supported | Not available |
| Advisory checks during `validate()` (`preflight()`) | Supported | Not available |
| Schema source | Declared on the class (Pydantic, TypeBox, or JSON values) | Python: inferred from type hints. TypeScript/Rust: passed explicitly |
| State between calls | Instance fields | Closure captures |
| Access to the call context | `execute(inputs, context)` | Python: a `Context`-typed parameter. TypeScript/Rust: the handler's second argument |

---

## External Schema Binding (YAML)

When you cannot change the source at all, a binding file maps a module ID to an existing callable. The format is specified in [protocol-spec §5.12](../spec/protocol-spec.md#512-external-schema-binding-external-schema-binding).

### Binding File

```yaml
# bindings/email.binding.yaml
spec_version: "1.0"
bindings:
  - module_id: "email.send"
    target: "myapp.services.email:send_email"
    description: "Send an email"
    tags: ["email", "notification"]
    annotations:
      open_world: true
      idempotent: false
    input_schema:
      type: object
      properties:
        to:
          type: string
          description: "Recipient email"
        subject:
          type: string
          description: "Email subject"
        body:
          type: string
          description: "Email body"
      required: [to, subject, body]
    output_schema:
      type: object
      properties:
        success:
          type: boolean
          description: "Whether the email was accepted"
        message_id:
          type: string
          description: "Provider message ID"
      required: [success]

  - module_id: "email.send_template"
    target: "myapp.services.email:EmailService.send_template"
    description: "Send an email rendered from a template"
    schema_ref: "../schemas/email/send_template.schema.yaml"
```

`target` is `<module path>:<callable>` — a Python import path or a TypeScript module specifier, then the function or `Class.method` name. In Rust, `target` is the key under which you supply a handler (below).

### Where the Schemas Come From

Each binding uses exactly one of these:

| Mode | How | Notes |
|------|------|------|
| Inline | `input_schema` + `output_schema` | Both are required together |
| Shared file | `schema_ref: <path>` | Path is relative to the binding file; the referenced file holds `input_schema` / `output_schema` |
| Inferred | `auto_schema: true` (or no schema at all) | Python reads the target's type hints. TypeScript reads `inputSchema`/`outputSchema` (or `<name>InputSchema`/`<name>OutputSchema`) exported beside the target. Rust cannot infer a schema from a string target and falls back to a permissive object schema with a warning, so give Rust bindings explicit schemas |

`auto_schema: strict` additionally requires the inferred schema to be compatible with OpenAI/Anthropic strict mode. Setting `auto_schema: false` without another mode is an error.

### Loading Binding Files

Binding files are loaded with `BindingLoader`, which scans `bindings.dir` (default `./bindings`) for files matching `bindings.pattern` (default `*.binding.yaml`; `*` and `?` wildcards, matched against file names, not recursive):

```yaml
# apcore.yaml
bindings:
  dir: "./bindings"
  pattern: "*.binding.yaml"
```

```text
my-project/
├── bindings/
│   ├── email.binding.yaml       # one file per business domain
│   ├── payment.binding.yaml
│   └── user.binding.yaml
└── apcore.yaml
```

=== "Python"

    ```python
    from apcore import APCore, BindingLoader, Config

    config = Config.load("apcore.yaml")
    client = APCore(config=config)

    # dir and pattern come from bindings.dir / bindings.pattern
    BindingLoader().load_binding_dir(registry=client.registry, config=config)
    ```

=== "TypeScript"

    ```typescript
    import { APCore, BindingLoader, Config } from 'apcore-js';

    const config = Config.load('apcore.yaml');
    const client = new APCore({ config });

    // dir and pattern come from bindings.dir / bindings.pattern
    await new BindingLoader().loadBindingDir(undefined, client.registry, undefined, config);
    ```

=== "Rust"

    ```rust
    use std::collections::HashMap;
    use std::sync::Arc;

    use apcore::errors::ModuleError;
    use apcore::{APCore, BindingHandler, BindingLoader, Config};
    use serde_json::json;

    pub fn load_bindings() -> Result<APCore, ModuleError> {
        let config = Config::load(std::path::Path::new("apcore.yaml"))?;

        // dir and pattern come from bindings.dir / bindings.pattern
        let mut loader = BindingLoader::new();
        loader.load_binding_dir_with_config(None, None, Some(&config))?;

        // Rust cannot import a callable by name, so supply a handler for every
        // `target` in the loaded files; a missing one fails registration.
        let mut handlers: HashMap<String, BindingHandler> = HashMap::new();
        handlers.insert(
            "myapp.services.email:send_email".to_string(),
            Arc::new(|_inputs, _ctx| Box::pin(async move { Ok(json!({ "success": true, "message_id": "msg_123" })) })),
        );
        handlers.insert(
            "myapp.services.email:EmailService.send_template".to_string(),
            Arc::new(|_inputs, _ctx| Box::pin(async move { Ok(json!({ "success": true })) })),
        );

        let client = APCore::with_config(config);
        loader.register_into_with_handlers(client.registry(), handlers)?;
        Ok(client)
    }
    ```

---

## Approach Selection Comparison

| Consideration | Class-based | `module()` on a function | `module()` on an existing callable | External binding |
|------|------------|-----------------|-------------------|-----------------|
| **New development** | Recommended | Recommended for small modules | Usable | Not recommended |
| **Wrapping existing functions** | Requires a rewrite | Requires editing the function (Python decorator) | Recommended | Usable |
| **Cannot modify the source** | Not possible | Not possible | Recommended (registration lives elsewhere) | Recommended |
| **Needs lifecycle hooks** | Recommended | Not supported | Not supported | Not supported |
| **Schema lives in YAML shared across SDKs** | Via `SchemaLoader` | No | No | Via `schema_ref` |
| **Schema control** | Full | Python: from type hints; TypeScript/Rust: explicit | Same as `module()` | Full (hand-written YAML) |

---

## Next Steps

- [Schema Definition Guide](./schema-definition.md) — Complete schema usage
- [Testing Modules Guide](./testing-modules.md) — Testing your modules
- [ACL Configuration Guide](./acl-configuration.md) — Configure who may call which module
- [Module Interface](../features/module-interface.md) — The module contract
- [Adapter Development Guide](./adapter-development.md) — Exposing modules through web frameworks
