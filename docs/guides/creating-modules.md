# Creating Modules Guide

> Build or upgrade modules to be AI-perceivable.

## Choose Your Integration Path

apcore supports four ways to create modules — choose the one that fits your scenario:

| Approach | Use Case | Code Intrusiveness | Jump To |
|------|---------|-----------|------|
| **Class-based** (Class Definition) | New module development | High (implements Module protocol) | [Quick Start](#quick-start) |
| **`@module` Decorator** | Functions where you can modify source code | Low (add one line decorator) | [module() Registration](#module-registration) |
| **`module()` Function Call** | Wrapping existing classes/methods | Very Low (no changes to original function) | [module() Registration](#module-registration) |
| **External Binding** (External Binding) | Zero-modification integration of existing apps | None (no source code changes) | [External Schema Binding](#external-schema-binding-yaml) |

These can also be grouped into two integration paths:

1.  **Native SDK (Recommended)**: Best for new projects. Full type safety and lifecycle control. (Class-based, `@module` Decorator)
2.  **Zero-Intrusion Patch**: Best for legacy code. Upgrade via function calls or YAML bindings **without rewriting business logic**. (`module()` Function Call, External Binding)

---

## Designing for the AI Lifecycle

To build a high-quality module, think through the **AI Collaboration Lifecycle**. Your module's metadata should guide the Agent through every stage of its task.

| Lifecycle Stage | Field / Tool | Purpose |
| :--- | :--- | :--- |
| **1. Discovery** | `description` | Helps the Agent find the right tool for its intent. |
| **2. Strategy** | `metadata` | Teaches the Agent *when* and *how* to use the tool correctly. |
| **3. Governance** | `requires_approval` | Sets the safety boundary for sensitive operations. |
| **4. Recovery** | `ai_guidance` | Provides a clear path for the Agent to fix errors autonomously. |

---

## Intent-Oriented Tips

### 1. Discovery: Focus on "What" not "How"
Your module's `description` is its **Identity**. It should answer: *"What problem does this solve?"* rather than *"What does the code do?"*
- **❌ Technical**: "Executes a SQL SELECT query on the users table."
- **✅ Intent-Oriented**: "Find a user profile by their email address or unique ID."

### 2. Strategy: Share Your Wisdom
Use `metadata` to give the Agent tactical guidance (the **Wisdom** layer):
- `x-when-to-use`: Describe the ideal scenario for this module.
- `x-when-not-to-use`: Explicitly warn the Agent of misuse to prevent hallucinations.
- `x-common-mistakes`: Warn the Agent about pitfalls others have encountered.

### 3. Governance: Set Guardrails
Use **Annotations** to define your module's **Personality**.
- For sensitive operations (spending money, deleting data), set `requires_approval: true`. This ensures a human always has the final word.

### 4. Recovery: Empower Self-Healing

Self-Healing enables **Self-Repair** and **Self-Evolution** (see [Design Philosophy](../concepts.md#11-the-concept-cognitive-interface) for definitions).

When an error occurs, use the `ai_guidance` field in your `ModuleError` to tell the Agent **exactly what to do next** — not what went wrong (that's `message`'s job).

| Field | Purpose | Example |
| :--- | :--- | :--- |
| `message` | What happened | `"Database connection failed"` |
| `ai_guidance` | What to do next | `"Retry after 5s. If persistent, ask user to check DB credentials."` |
| `suggestion` | Specific fix | `"Verify DB_HOST and DB_PORT environment variables"` |
| `user_fixable` | Can user fix? | `true` |

**Anti-patterns:**
- ❌ `ai_guidance="An error occurred while processing the request"` — restates the error, no action
- ❌ `ai_guidance="Please try again"` — too vague, no specificity

**Good patterns:**
- ✅ `ai_guidance="Email format is invalid. Ask the user for a valid email (user@domain.com)."`
- ✅ `ai_guidance="Retry after 5s. If still failing after 3 retries, ask user to check network connectivity."`
- ✅ `ai_guidance="File not found. Verify the path with the user. If correct, check read permissions."`

---

## Quick Start

### 1. Create Project Structure

```bash
my-project/
├── apcore.yaml           # Framework configuration
├── extensions/           # Extensions directory
│   └── executor/         # Execution layer
│       └── email/        # Email functionality
│           └── send_email.py # Python module OR
│           └── send_email.ts # TypeScript module
└── schemas/              # Schema definitions (optional)
```

### 2. Create Module File

!!! note "Module is a structural interface"
    In all languages, `Module` defines a structural contract — not a base class to inherit from. Python: `Module` is a `Protocol`, so `class MyModule:` without inheritance is valid. Rust: `Module` is a `trait`. TypeScript: any object matching the type shape satisfies `Module`. Explicit inheritance/implementation is convenient for IDE support but not required.

=== "Python"

    ```python
    # extensions/executor/email/send_email.py
    from pydantic import BaseModel, Field
    from apcore import Module, Context

    class SendEmailInput(BaseModel):
        to: str = Field(..., description="Recipient email address")
        subject: str = Field(..., description="Email subject")
        body: str = Field(..., description="Email body")

    class SendEmailOutput(BaseModel):
        success: bool = Field(..., description="Whether successful")
        message_id: str | None = Field(None, description="Message ID")

    class SendEmailModule(Module):
        """Send email module"""
        input_schema = SendEmailInput
        output_schema = SendEmailOutput

        def execute(self, inputs: dict, context: Context) -> dict:
            # Implement logic here
            return {"success": True, "message_id": "msg_123"}
    ```

=== "TypeScript"

    ```typescript
    // extensions/executor/email/sendEmail.ts
    import { Type } from '@sinclair/typebox';
    import { FunctionModule } from 'apcore-js';

    const SendEmailInput = Type.Object({
      to: Type.String({ description: 'Recipient email address' }),
      subject: Type.String({ description: 'Email subject' }),
      body: Type.String({ description: 'Email body' }),
    });

    const SendEmailOutput = Type.Object({
      success: Type.Boolean({ description: 'Whether successful' }),
      message_id: Type.Optional(Type.String({ description: 'Message ID' })),
    });

    export default new FunctionModule({
      moduleId: 'executor.email.send_email',
      description: 'Send email module',
      inputSchema: SendEmailInput,
      outputSchema: SendEmailOutput,
      execute: async (inputs) => {
        // Implement logic here
        return { success: true, message_id: 'msg_123' };
      },
    });
    ```

=== "Rust"

    ```rust
    // extensions/executor/email/send_email.rs
    use apcore::{Module, Context};
    use apcore::errors::{ErrorCode, ModuleError};
    use async_trait::async_trait;
    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    #[derive(Deserialize)]
    struct SendEmailInput {
        to: String,
        subject: String,
        body: String,
    }

    #[derive(Serialize)]
    struct SendEmailOutput {
        success: bool,
        message_id: Option<String>,
    }

    pub struct SendEmailModule;

    #[async_trait]
    impl Module for SendEmailModule {
        fn description(&self) -> &str {
            "Send email module"
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "to":      { "type": "string", "description": "Recipient email address" },
                    "subject": { "type": "string", "description": "Email subject line" },
                    "body":    { "type": "string", "description": "Email body content" }
                },
                "required": ["to", "subject", "body"],
                "additionalProperties": false
            })
        }

        fn output_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "success":    { "type": "boolean", "description": "Whether the email was sent" },
                    "message_id": { "type": ["string", "null"], "description": "Provider message ID" }
                },
                "required": ["success"],
                "additionalProperties": false
            })
        }

        async fn execute(
            &self,
            inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Value, ModuleError> {
            let input: SendEmailInput = serde_json::from_value(inputs)
                .map_err(|e| ModuleError::new(ErrorCode::GeneralInvalidInput, e.to_string()))?;
            // Implement logic here
            let _ = input;
            let output = SendEmailOutput {
                success: true,
                message_id: Some("msg_123".to_string()),
            };
            Ok(serde_json::to_value(output).unwrap())
        }
    }
    ```

### 3. Module ID Auto-Generation

```
File path: extensions/executor/email/send_email.py
Module ID:  executor.email.send_email
```

**No configuration needed - the file path is the ID.**

---

## Detailed Steps

### Step 1: Design Schema

**First think about module inputs and outputs:**

| Question | Example (Send Email) |
|------|------------------|
| What inputs are needed? | to, subject, body, cc |
| What outputs are returned? | success, message_id, error |
| What constraints exist? | to must be email format, subject max 200 chars |

**Define Input Schema:**

=== "Python"

    ```python
    from pydantic import BaseModel, Field
    from typing import Literal

    class SendEmailInput(BaseModel):
        """Input parameters - each field must have description."""

        to: str = Field(
            ...,                                  # ... means required
            description="Recipient email address",
            pattern=r"^[\w\.-]+@[\w\.-]+\.\w+$",  # Email format validation
        )

        subject: str = Field(
            ...,
            description="Email subject",
            max_length=200,                       # Length limit
        )

        body: str = Field(
            ...,
            description="Email body, supports plain text or HTML",
        )

        cc: list[str] = Field(
            default_factory=list,                 # Optional fields must have defaults
            description="CC list",
        )

        priority: Literal["low", "normal", "high"] = Field(
            default="normal",
            description="Email priority",
        )
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    // Each field carries a description; AI uses this to understand intent.
    export const SendEmailInput = Type.Object({
      to: Type.String({
        description: 'Recipient email address',
        pattern: '^[\\w\\.-]+@[\\w\\.-]+\\.\\w+$',
      }),
      subject: Type.String({
        description: 'Email subject',
        maxLength: 200,
      }),
      body: Type.String({
        description: 'Email body, supports plain text or HTML',
      }),
      cc: Type.Array(Type.String(), {
        description: 'CC list',
        default: [],
      }),
      priority: Type.Union(
        [Type.Literal('low'), Type.Literal('normal'), Type.Literal('high')],
        { description: 'Email priority', default: 'normal' },
      ),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    /// Returns the JSON Schema describing the email input. Each property
    /// carries a `description` so AI tooling can reason about intent.
    pub fn send_email_input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "to": {
                    "type": "string",
                    "description": "Recipient email address",
                    "pattern": r"^[\w\.-]+@[\w\.-]+\.\w+$"
                },
                "subject": {
                    "type": "string",
                    "description": "Email subject",
                    "maxLength": 200
                },
                "body": {
                    "type": "string",
                    "description": "Email body, supports plain text or HTML"
                },
                "cc": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "CC list",
                    "default": []
                },
                "priority": {
                    "type": "string",
                    "enum": ["low", "normal", "high"],
                    "description": "Email priority",
                    "default": "normal"
                }
            },
            "required": ["to", "subject", "body"],
            "additionalProperties": false
        })
    }
    ```

**Define Output Schema:**

=== "Python"

    ```python
    from pydantic import BaseModel, Field

    class SendEmailOutput(BaseModel):
        """Output result."""

        success: bool = Field(
            ...,
            description="Whether email was sent successfully",
        )

        message_id: str | None = Field(
            None,
            description="Message ID when send is successful",
        )

        error: str | None = Field(
            None,
            description="Error message when send fails",
        )

        sent_at: str | None = Field(
            None,
            description="Send time, ISO 8601 format",
        )
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    export const SendEmailOutput = Type.Object({
      success: Type.Boolean({
        description: 'Whether email was sent successfully',
      }),
      message_id: Type.Optional(
        Type.String({ description: 'Message ID when send is successful' }),
      ),
      error: Type.Optional(
        Type.String({ description: 'Error message when send fails' }),
      ),
      sent_at: Type.Optional(
        Type.String({ description: 'Send time, ISO 8601 format' }),
      ),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    pub fn send_email_output_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "success": {
                    "type": "boolean",
                    "description": "Whether email was sent successfully"
                },
                "message_id": {
                    "type": ["string", "null"],
                    "description": "Message ID when send is successful"
                },
                "error": {
                    "type": ["string", "null"],
                    "description": "Error message when send fails"
                },
                "sent_at": {
                    "type": ["string", "null"],
                    "description": "Send time, ISO 8601 format"
                }
            },
            "required": ["success"],
            "additionalProperties": false
        })
    }
    ```

---

### Step 2: Implement Module

=== "Python"

    ```python
    # extensions/executor/email/send_email.py
    from datetime import datetime, timezone
    from apcore import Module, Context

    class SendEmailModule(Module):
        """Send emails via SMTP or API, supports HTML format."""

        # Associate Schema
        input_schema = SendEmailInput
        output_schema = SendEmailOutput

        # Optional: Module metadata
        description = "Send an email message via SMTP or HTTP API"
        tags = ["email", "notification"]
        version = "1.0.0"

        def execute(self, inputs: dict, context: Context) -> dict:
            """Execute email sending.

            Args:
                inputs: Input parameters (already schema-validated)
                context: Call context (trace_id, caller_id, executor, ...)

            Returns:
                Send result matching ``SendEmailOutput``.
            """
            # Option 1: read from dict directly
            to = inputs["to"]
            subject = inputs["subject"]

            # Option 2: parse via Pydantic for typed access
            params = self.input_schema(**inputs)

            try:
                message_id = self._send_email(
                    to=params.to,
                    subject=params.subject,
                    body=params.body,
                    cc=params.cc,
                )
                return {
                    "success": True,
                    "message_id": message_id,
                    "error": None,
                    "sent_at": datetime.now(timezone.utc).isoformat(),
                }
            except Exception as e:  # noqa: BLE001 — convert to structured output
                return {
                    "success": False,
                    "message_id": None,
                    "error": str(e),
                    "sent_at": None,
                }

        def _send_email(self, to: str, subject: str, body: str, cc: list[str]) -> str:
            """Internal method: actual sending logic (e.g. smtplib)."""
            return "msg_" + datetime.now(timezone.utc).strftime("%Y%m%d%H%M%S")
    ```

=== "TypeScript"

    ```typescript
    // extensions/executor/email/send-email.ts
    import { FunctionModule } from 'apcore-js';
    import type { Context } from 'apcore-js';
    import { SendEmailInput, SendEmailOutput } from './schemas.js';

    async function sendEmail(
      to: string,
      subject: string,
      body: string,
      cc: string[],
    ): Promise<string> {
      // Implement specific sending logic here (e.g. nodemailer, fetch).
      return `msg_${Date.now()}`;
    }

    export default new FunctionModule({
      moduleId: 'executor.email.send_email',
      description: 'Send an email message via SMTP or HTTP API',
      inputSchema: SendEmailInput,
      outputSchema: SendEmailOutput,
      tags: ['email', 'notification'],
      version: '1.0.0',
      execute: async (inputs, _context: Context) => {
        const to = inputs.to as string;
        const subject = inputs.subject as string;
        const body = inputs.body as string;
        const cc = (inputs.cc as string[] | undefined) ?? [];

        try {
          const messageId = await sendEmail(to, subject, body, cc);
          return {
            success: true,
            message_id: messageId,
            error: null,
            sent_at: new Date().toISOString(),
          };
        } catch (e) {
          return {
            success: false,
            message_id: null,
            error: e instanceof Error ? e.message : String(e),
            sent_at: null,
          };
        }
      },
    });
    ```

=== "Rust"

    ```rust
    // extensions/executor/email/send_email.rs
    use apcore::{Context, Module};
    use apcore::errors::{ErrorCode, ModuleError};
    use async_trait::async_trait;
    use chrono::Utc;
    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    #[derive(Debug, Deserialize)]
    struct SendEmailInput {
        to: String,
        subject: String,
        body: String,
        #[serde(default)]
        cc: Vec<String>,
    }

    #[derive(Debug, Serialize)]
    struct SendEmailOutput {
        success: bool,
        message_id: Option<String>,
        error: Option<String>,
        sent_at: Option<String>,
    }

    pub struct SendEmailModule;

    #[async_trait]
    impl Module for SendEmailModule {
        fn input_schema(&self) -> Value {
            send_email_input_schema()
        }

        fn output_schema(&self) -> Value {
            send_email_output_schema()
        }

        fn description(&self) -> &str {
            "Send an email message via SMTP or HTTP API"
        }

        fn tags(&self) -> Vec<String> {
            vec!["email".into(), "notification".into()]
        }

        async fn execute(
            &self,
            inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Value, ModuleError> {
            let params: SendEmailInput = serde_json::from_value(inputs).map_err(|e| {
                ModuleError::new(ErrorCode::GeneralInvalidInput, e.to_string())
            })?;

            match send_email(&params.to, &params.subject, &params.body, &params.cc).await {
                Ok(message_id) => {
                    let out = SendEmailOutput {
                        success: true,
                        message_id: Some(message_id),
                        error: None,
                        sent_at: Some(Utc::now().to_rfc3339()),
                    };
                    Ok(serde_json::to_value(out).unwrap())
                }
                Err(e) => {
                    let out = SendEmailOutput {
                        success: false,
                        message_id: None,
                        error: Some(e.to_string()),
                        sent_at: None,
                    };
                    Ok(serde_json::to_value(out).unwrap())
                }
            }
        }
    }

    async fn send_email(
        _to: &str,
        _subject: &str,
        _body: &str,
        _cc: &[String],
    ) -> Result<String, std::io::Error> {
        // Implement specific sending logic here.
        Ok(format!("msg_{}", Utc::now().timestamp()))
    }
    ```

---

### Step 3: Place Files

**Organize directories by functional layers:**

```
extensions/
├── api/                    # API entry layer
│   └── handler/
│       └── user_api.py
│
├── orchestrator/           # Orchestration layer
│   └── workflow/
│       └── user_register.py
│
├── executor/               # Execution layer
│   ├── email/
│   │   ├── send_email.py       → executor.email.send_email
│   │   └── send_template.py    → executor.email.send_template
│   ├── sms/
│   │   └── send_sms.py         → executor.sms.send_sms
│   └── database/
│       └── query.py            → executor.database.query
│
└── common/                 # Common components
    └── util/
        └── validator.py        → common.util.validator
```

**Layer Recommendations:**

| Layer | Responsibility | Examples |
|---|------|------|
| `api` | External request entry | HTTP handler, GraphQL resolver |
| `orchestrator` | Business orchestration, flow control | Registration flow, order processing |
| `executor` | Concrete execution, external calls | Send email, call API, query database |
| `common` | Common utilities | Validators, formatters |

---

### Step 4: Use Module

=== "Python"

    ```python
    from apcore import Registry, Executor

    # 1. Create Registry and discover modules
    registry = Registry(extensions_dir="./extensions")
    registry.discover()

    # 2. Create Executor
    executor = Executor(registry)

    # 3. Call module
    result = executor.call(
        module_id="executor.email.send_email",
        inputs={
            "to": "user@example.com",
            "subject": "Hello",
            "body": "World"
        }
    )
    print(result)
    ```

=== "TypeScript"

    ```typescript
    import { Registry, Executor } from 'apcore-js';

    // 1. Create Registry and discover modules
    const registry = new Registry({ extensionsDir: './extensions' });
    await registry.discover();

    // 2. Create Executor
    const executor = new Executor({ registry });

    // 3. Call module
    const result = await executor.call(
      'executor.email.send_email',
      {
        to: 'user@example.com',
        subject: 'Hello',
        body: 'World'
      }
    );
    console.log(result);
    ```

=== "Rust"

    ```rust
    use apcore::{Registry, Executor};
    use serde_json::json;

    // 1. Create Registry and discover modules
    let registry = Registry::builder()
        .extensions_dir("./extensions")
        .build()?;
    registry.discover()?;

    // 2. Create Executor
    let executor = Executor::new(&registry);

    // 3. Call module
    let result = executor.call(
        "executor.email.send_email",
        json!({
            "to": "user@example.com",
            "subject": "Hello",
            "body": "World"
        }),
    )?;
    println!("{:?}", result);
    ```


---

## Advanced Usage

### Using Context

=== "Python"

    ```python
    from apcore import Module, Context

    class SendEmailModule(Module):
        def execute(self, inputs: dict, context: Context) -> dict:
            # Call chain information
            print(f"Trace ID:   {context.trace_id}")
            print(f"Caller:     {context.caller_id}")
            print(f"Call Chain: {context.call_chain}")

            # Identity information (if available)
            if context.identity:
                print(f"Identity: {context.identity.id} ({context.identity.type})")

            # Shared data along the call chain
            custom_data = context.data.get("my_data")

            # ... execute logic ...
            return {"success": True}
    ```

=== "TypeScript"

    ```typescript
    import { FunctionModule } from 'apcore-js';
    import type { Context } from 'apcore-js';

    export default new FunctionModule({
      moduleId: 'executor.email.send_email',
      description: 'Send email with context-aware logging',
      inputSchema: /* SendEmailInput */ undefined as never,
      outputSchema: /* SendEmailOutput */ undefined as never,
      execute: (inputs, context: Context) => {
        // Call chain information
        console.log(`Trace ID:   ${context.traceId}`);
        console.log(`Caller:     ${context.callerId}`);
        console.log(`Call Chain: ${context.callChain.join(' -> ')}`);

        // Identity information (if available)
        if (context.identity) {
          console.log(`Identity: ${context.identity.id} (${context.identity.type})`);
        }

        // Shared data along the call chain
        const customData = context.data['my_data'];

        return { success: true };
      },
    });
    ```

=== "Rust"

    ```rust
    use apcore::{Context, Module};
    use apcore::errors::ModuleError;
    use async_trait::async_trait;
    use serde_json::{json, Value};

    pub struct SendEmailModule;

    #[async_trait]
    impl Module for SendEmailModule {
        fn input_schema(&self) -> Value { json!({"type": "object"}) }
        fn output_schema(&self) -> Value { json!({"type": "object"}) }
        fn description(&self) -> &str { "Send email with context-aware logging" }

        async fn execute(
            &self,
            _inputs: Value,
            ctx: &Context<Value>,
        ) -> Result<Value, ModuleError> {
            // Call chain information
            println!("Trace ID:   {}", ctx.trace_id);
            println!("Caller:     {:?}", ctx.caller_id);
            println!("Call Chain: {:?}", ctx.call_chain);

            // Identity information (if available)
            if let Some(identity) = &ctx.identity {
                println!("Identity: {} ({})", identity.id(), identity.identity_type());
            }

            // Shared data along the call chain. Drop the guard before any await.
            let custom_data = ctx.data.read().get("my_data").cloned();
            let _ = custom_data;

            Ok(json!({"success": true}))
        }
    }
    ```

### Calling Other Modules

=== "Python"

    ```python
    from apcore import Module, Context

    class UserRegisterModule(Module):
        """User registration module."""

        def execute(self, inputs: dict, context: Context) -> dict:
            # Create user
            user_id = self._create_user(inputs)

            # Call the send-email module via the executor on the context
            email_result = context.executor.call(
                module_id="executor.email.send_email",
                inputs={
                    "to": inputs["email"],
                    "subject": "Welcome",
                    "body": "Welcome to the platform!",
                },
                context=context,  # Propagate context to keep the call chain
            )

            return {
                "user_id": user_id,
                "email_sent": email_result["success"],
            }

        def _create_user(self, inputs: dict) -> str:
            return "user_123"
    ```

=== "TypeScript"

    ```typescript
    import { FunctionModule, Executor } from 'apcore-js';
    import type { Context } from 'apcore-js';

    export default new FunctionModule({
      moduleId: 'orchestrator.user.register',
      description: 'Register a new user and send a welcome email',
      inputSchema: /* UserRegisterInput */ undefined as never,
      outputSchema: /* UserRegisterOutput */ undefined as never,
      execute: async (inputs, context: Context) => {
        // Create user
        const userId = 'user_123';

        // Call the send-email module via the executor stored on the context
        const executor = context.executor as Executor;
        const emailResult = await executor.call(
          'executor.email.send_email',
          {
            to: inputs.email as string,
            subject: 'Welcome',
            body: 'Welcome to the platform!',
          },
          context, // Propagate context to keep the call chain
        );

        return {
          user_id: userId,
          email_sent: emailResult.success as boolean,
        };
      },
    });
    ```

=== "Rust"

    ```rust
    use apcore::{Context, Executor, Module};
    use apcore::errors::ModuleError;
    use async_trait::async_trait;
    use serde_json::{json, Value};
    use std::sync::Arc;

    pub struct UserRegisterModule;

    #[async_trait]
    impl Module for UserRegisterModule {
        fn input_schema(&self) -> Value { json!({"type": "object"}) }
        fn output_schema(&self) -> Value { json!({"type": "object"}) }
        fn description(&self) -> &str { "Register a new user and send a welcome email" }

        async fn execute(
            &self,
            inputs: Value,
            ctx: &Context<Value>,
        ) -> Result<Value, ModuleError> {
            // Create user
            let user_id = "user_123".to_string();

            // Resolve the executor stashed on the context and invoke the next module.
            let executor = ctx
                .executor
                .as_ref()
                .and_then(|any| any.clone().downcast::<Executor>().ok())
                .ok_or_else(|| ModuleError::new(
                    apcore::errors::ErrorCode::GeneralInvalidInput,
                    "executor is not bound on context",
                ))?;

            let email_result = executor
                .call(
                    "executor.email.send_email",
                    json!({
                        "to": inputs["email"].as_str().unwrap_or_default(),
                        "subject": "Welcome",
                        "body": "Welcome to the platform!",
                    }),
                    Some(ctx),
                )
                .await?;

            Ok(json!({
                "user_id": user_id,
                "email_sent": email_result["success"].as_bool().unwrap_or(false),
            }))
        }
    }
    ```

### Async Modules

=== "Python"

    ```python
    import aiohttp
    from apcore import Module, Context

    class SendEmailModule(Module):
        """Send email module with async support."""

        input_schema = SendEmailInput
        output_schema = SendEmailOutput

        # Defining `execute` as `async def` is enough — the framework
        # auto-detects coroutines and drives them on the async path.
        async def execute(self, inputs: dict, context: Context) -> dict:
            params = self.input_schema(**inputs)
            async with aiohttp.ClientSession() as session:
                message_id = await self._send_async(session, params)
            return {
                "success": True,
                "message_id": message_id,
                "error": None,
            }

        async def _send_async(self, session, params) -> str:
            async with session.post(
                "https://api.example.com/email",
                json={"to": params.to, "subject": params.subject, "body": params.body},
            ) as resp:
                data = await resp.json()
                return data["id"]
    ```

=== "TypeScript"

    ```typescript
    // TypeScript modules are async by design — `execute` may return a Promise.
    import { FunctionModule } from 'apcore-js';
    import type { Context } from 'apcore-js';

    export default new FunctionModule({
      moduleId: 'executor.email.send_email',
      description: 'Send email with async transport',
      inputSchema: SendEmailInput,
      outputSchema: SendEmailOutput,
      execute: async (inputs, _context: Context) => {
        const resp = await fetch('https://api.example.com/email', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            to: inputs.to,
            subject: inputs.subject,
            body: inputs.body,
          }),
        });
        const data = (await resp.json()) as { id: string };
        return {
          success: true,
          message_id: data.id,
          error: null,
        };
      },
    });
    ```

=== "Rust"

    ```rust
    // Rust modules are async by design via `#[async_trait]`.
    use apcore::{Context, Module};
    use apcore::errors::{ErrorCode, ModuleError};
    use async_trait::async_trait;
    use serde_json::{json, Value};

    pub struct SendEmailModule {
        client: reqwest::Client,
    }

    #[async_trait]
    impl Module for SendEmailModule {
        fn input_schema(&self) -> Value { send_email_input_schema() }
        fn output_schema(&self) -> Value { send_email_output_schema() }
        fn description(&self) -> &str { "Send email with async transport" }

        async fn execute(
            &self,
            inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Value, ModuleError> {
            let resp = self
                .client
                .post("https://api.example.com/email")
                .json(&inputs)
                .send()
                .await
                .map_err(|e| ModuleError::new(ErrorCode::GeneralExecutionFailed, e.to_string()))?;

            let data: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| ModuleError::new(ErrorCode::GeneralExecutionFailed, e.to_string()))?;

            Ok(json!({
                "success": true,
                "message_id": data["id"],
                "error": null,
            }))
        }
    }
    ```

### Resource Management

=== "Python"

    ```python
    from typing import Any
    from apcore import Module, Context

    class DatabaseModule(Module):
        """Database module that manages a connection pool."""

        _pool: Any = None

        def on_load(self) -> None:
            """Create the connection pool when the module is registered."""
            self._pool = create_connection_pool(host="localhost", database="mydb")

        def on_unload(self) -> None:
            """Close the connection pool when the module is unregistered."""
            if self._pool:
                self._pool.close()

        def execute(self, inputs: dict, context: Context) -> dict:
            with self._pool.get_connection() as conn:
                rows = conn.execute(inputs["sql"])
                return {"rows": rows}
    ```

=== "TypeScript"

    ```typescript
    // TypeScript: keep resources on a closure or a wrapping class. Cleanup
    // is the host application's responsibility (e.g. on shutdown).
    import { FunctionModule } from 'apcore-js';
    import type { Context } from 'apcore-js';
    import type { Pool } from 'pg';
    import { createPool } from './db.js';

    let pool: Pool | null = null;
    function getPool(): Pool {
      if (!pool) pool = createPool({ host: 'localhost', database: 'mydb' });
      return pool;
    }

    export async function shutdown(): Promise<void> {
      if (pool) {
        await pool.end();
        pool = null;
      }
    }

    export default new FunctionModule({
      moduleId: 'executor.database.query',
      description: 'Execute a SQL query against the application database',
      inputSchema: /* QueryInput */ undefined as never,
      outputSchema: /* QueryOutput */ undefined as never,
      execute: async (inputs, _context: Context) => {
        const result = await getPool().query(inputs.sql as string);
        return { rows: result.rows };
      },
    });
    ```

=== "Rust"

    ```rust
    // Rust: own the pool on the module struct. Drop runs at unregistration.
    use apcore::{Context, Module};
    use apcore::errors::{ErrorCode, ModuleError};
    use async_trait::async_trait;
    use serde_json::{json, Value};
    use sqlx::postgres::{PgPool, PgPoolOptions};

    pub struct DatabaseModule {
        pool: PgPool,
    }

    impl DatabaseModule {
        pub async fn connect(url: &str) -> Result<Self, ModuleError> {
            let pool = PgPoolOptions::new()
                .max_connections(8)
                .connect(url)
                .await
                .map_err(|e| ModuleError::new(ErrorCode::GeneralExecutionFailed, e.to_string()))?;
            Ok(Self { pool })
        }
    }

    #[async_trait]
    impl Module for DatabaseModule {
        fn input_schema(&self) -> Value { json!({"type": "object"}) }
        fn output_schema(&self) -> Value { json!({"type": "object"}) }
        fn description(&self) -> &str { "Execute a SQL query against the application database" }

        async fn execute(
            &self,
            inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Value, ModuleError> {
            let sql = inputs["sql"].as_str().unwrap_or_default();
            let rows: Vec<(i64,)> = sqlx::query_as(sql)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| ModuleError::new(ErrorCode::GeneralExecutionFailed, e.to_string()))?;
            Ok(json!({ "rows": rows.len() }))
        }

        fn on_unload(&self) {
            // Pool is dropped along with the module struct; no manual close needed.
        }
    }
    ```

---

## Common Patterns

### Pattern 1: Simple Executor

=== "Python"

    ```python
    from typing import Literal
    from pydantic import BaseModel, Field
    from apcore import Module, Context

    class CalculatorModule(Module):
        """Simple calculator — no side effects."""

        class Input(BaseModel):
            a: float = Field(..., description="First number")
            b: float = Field(..., description="Second number")
            op: Literal["+", "-", "*", "/"] = Field(..., description="Operator")

        class Output(BaseModel):
            result: float = Field(..., description="Calculation result")

        input_schema = Input
        output_schema = Output
        description = "Perform basic arithmetic on two numbers"

        def execute(self, inputs: dict, context: Context) -> dict:
            a, b, op = inputs["a"], inputs["b"], inputs["op"]
            ops = {"+": a + b, "-": a - b, "*": a * b, "/": a / b}
            return {"result": ops[op]}
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';
    import { FunctionModule } from 'apcore-js';

    const Input = Type.Object({
      a: Type.Number({ description: 'First number' }),
      b: Type.Number({ description: 'Second number' }),
      op: Type.Union(
        [Type.Literal('+'), Type.Literal('-'), Type.Literal('*'), Type.Literal('/')],
        { description: 'Operator' },
      ),
    });

    const Output = Type.Object({
      result: Type.Number({ description: 'Calculation result' }),
    });

    export default new FunctionModule({
      moduleId: 'executor.math.calculator',
      description: 'Perform basic arithmetic on two numbers',
      inputSchema: Input,
      outputSchema: Output,
      execute: (inputs) => {
        const a = inputs.a as number;
        const b = inputs.b as number;
        const op = inputs.op as '+' | '-' | '*' | '/';
        const ops: Record<string, number> = {
          '+': a + b, '-': a - b, '*': a * b, '/': a / b,
        };
        return { result: ops[op] };
      },
    });
    ```

=== "Rust"

    ```rust
    use apcore::{Context, Module};
    use apcore::errors::{ErrorCode, ModuleError};
    use async_trait::async_trait;
    use serde_json::{json, Value};

    pub struct CalculatorModule;

    #[async_trait]
    impl Module for CalculatorModule {
        fn input_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {
                    "a":  { "type": "number", "description": "First number" },
                    "b":  { "type": "number", "description": "Second number" },
                    "op": {
                        "type": "string",
                        "enum": ["+", "-", "*", "/"],
                        "description": "Operator"
                    }
                },
                "required": ["a", "b", "op"]
            })
        }

        fn output_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {
                    "result": { "type": "number", "description": "Calculation result" }
                },
                "required": ["result"]
            })
        }

        fn description(&self) -> &str {
            "Perform basic arithmetic on two numbers"
        }

        async fn execute(
            &self,
            inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Value, ModuleError> {
            let a = inputs["a"].as_f64().unwrap_or(0.0);
            let b = inputs["b"].as_f64().unwrap_or(0.0);
            let op = inputs["op"].as_str().unwrap_or("+");
            let result = match op {
                "+" => a + b,
                "-" => a - b,
                "*" => a * b,
                "/" => a / b,
                _ => return Err(ModuleError::new(
                    ErrorCode::GeneralInvalidInput,
                    format!("unknown operator: {op}"),
                )),
            };
            Ok(json!({ "result": result }))
        }
    }
    ```

### Pattern 2: External API Call

=== "Python"

    ```python
    import httpx
    from pydantic import BaseModel, Field
    from apcore import Module, Context

    class WeatherModule(Module):
        """Fetch weather information from an external API."""

        class Input(BaseModel):
            city: str = Field(..., description="City name")

        class Output(BaseModel):
            temperature: float = Field(..., description="Temperature (Celsius)")
            description: str = Field(..., description="Weather description")

        input_schema = Input
        output_schema = Output
        description = "Get current weather for a city"

        async def execute(self, inputs: dict, context: Context) -> dict:
            async with httpx.AsyncClient(timeout=5.0) as client:
                resp = await client.get(
                    "https://api.weather.com/v1/current",
                    params={"city": inputs["city"]},
                )
                resp.raise_for_status()
                data = resp.json()
            return {
                "temperature": data["temp"],
                "description": data["desc"],
            }
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';
    import { FunctionModule } from 'apcore-js';

    const Input = Type.Object({
      city: Type.String({ description: 'City name' }),
    });
    const Output = Type.Object({
      temperature: Type.Number({ description: 'Temperature (Celsius)' }),
      description: Type.String({ description: 'Weather description' }),
    });

    export default new FunctionModule({
      moduleId: 'executor.weather.current',
      description: 'Get current weather for a city',
      inputSchema: Input,
      outputSchema: Output,
      execute: async (inputs) => {
        const url = new URL('https://api.weather.com/v1/current');
        url.searchParams.set('city', inputs.city as string);
        const resp = await fetch(url, { signal: AbortSignal.timeout(5_000) });
        if (!resp.ok) throw new Error(`weather API ${resp.status}`);
        const data = (await resp.json()) as { temp: number; desc: string };
        return {
          temperature: data.temp,
          description: data.desc,
        };
      },
    });
    ```

=== "Rust"

    ```rust
    use apcore::{Context, Module};
    use apcore::errors::{ErrorCode, ModuleError};
    use async_trait::async_trait;
    use serde::Deserialize;
    use serde_json::{json, Value};

    #[derive(Deserialize)]
    struct WeatherResp { temp: f64, desc: String }

    pub struct WeatherModule { client: reqwest::Client }

    #[async_trait]
    impl Module for WeatherModule {
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
                    "temperature": { "type": "number", "description": "Temperature (Celsius)" },
                    "description": { "type": "string", "description": "Weather description" }
                },
                "required": ["temperature", "description"]
            })
        }

        fn description(&self) -> &str { "Get current weather for a city" }

        async fn execute(
            &self,
            inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Value, ModuleError> {
            let city = inputs["city"].as_str().unwrap_or_default();
            let data: WeatherResp = self
                .client
                .get("https://api.weather.com/v1/current")
                .query(&[("city", city)])
                .send()
                .await
                .map_err(|e| ModuleError::new(ErrorCode::GeneralExecutionFailed, e.to_string()))?
                .error_for_status()
                .map_err(|e| ModuleError::new(ErrorCode::GeneralExecutionFailed, e.to_string()))?
                .json()
                .await
                .map_err(|e| ModuleError::new(ErrorCode::GeneralExecutionFailed, e.to_string()))?;
            Ok(json!({ "temperature": data.temp, "description": data.desc }))
        }
    }
    ```

### Pattern 3: Data Validator

=== "Python"

    ```python
    import re
    from pydantic import BaseModel, Field
    from apcore import Module, Context

    EMAIL_RE = re.compile(r"^[\w\.-]+@[\w\.-]+\.\w+$")

    class EmailValidatorModule(Module):
        """Email format validator."""

        class Input(BaseModel):
            email: str = Field(..., description="Email to validate")

        class Output(BaseModel):
            valid: bool = Field(..., description="Whether the email is valid")
            reason: str | None = Field(None, description="Reason if invalid")

        input_schema = Input
        output_schema = Output
        description = "Validate that a string looks like an email address"

        def execute(self, inputs: dict, context: Context) -> dict:
            if EMAIL_RE.match(inputs["email"]):
                return {"valid": True, "reason": None}
            return {"valid": False, "reason": "Invalid email format"}
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';
    import { FunctionModule } from 'apcore-js';

    const EMAIL_RE = /^[\w.-]+@[\w.-]+\.\w+$/;

    const Input = Type.Object({
      email: Type.String({ description: 'Email to validate' }),
    });
    const Output = Type.Object({
      valid: Type.Boolean({ description: 'Whether the email is valid' }),
      reason: Type.Union(
        [Type.String(), Type.Null()],
        { description: 'Reason if invalid' },
      ),
    });

    export default new FunctionModule({
      moduleId: 'common.validator.email',
      description: 'Validate that a string looks like an email address',
      inputSchema: Input,
      outputSchema: Output,
      execute: (inputs) => {
        const email = inputs.email as string;
        if (EMAIL_RE.test(email)) {
          return { valid: true, reason: null };
        }
        return { valid: false, reason: 'Invalid email format' };
      },
    });
    ```

=== "Rust"

    ```rust
    use apcore::{Context, Module};
    use apcore::errors::ModuleError;
    use async_trait::async_trait;
    use once_cell::sync::Lazy;
    use regex::Regex;
    use serde_json::{json, Value};

    static EMAIL_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"^[\w\.-]+@[\w\.-]+\.\w+$").unwrap());

    pub struct EmailValidatorModule;

    #[async_trait]
    impl Module for EmailValidatorModule {
        fn input_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {
                    "email": { "type": "string", "description": "Email to validate" }
                },
                "required": ["email"]
            })
        }

        fn output_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {
                    "valid":  { "type": "boolean", "description": "Whether the email is valid" },
                    "reason": { "type": ["string", "null"], "description": "Reason if invalid" }
                },
                "required": ["valid"]
            })
        }

        fn description(&self) -> &str {
            "Validate that a string looks like an email address"
        }

        async fn execute(
            &self,
            inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Value, ModuleError> {
            let email = inputs["email"].as_str().unwrap_or_default();
            if EMAIL_RE.is_match(email) {
                Ok(json!({ "valid": true, "reason": null }))
            } else {
                Ok(json!({ "valid": false, "reason": "Invalid email format" }))
            }
        }
    }
    ```

### Pattern 4: Executor with Retry

=== "Python"

    ```python
    from tenacity import retry, stop_after_attempt, wait_exponential
    from apcore import Module, Context

    class ReliableSendModule(Module):
        """Wrap an external send call with bounded exponential-backoff retries."""

        input_schema = SendEmailInput
        output_schema = SendEmailOutput
        description = "Send email with retry on transient failures"

        def execute(self, inputs: dict, context: Context) -> dict:
            try:
                return self._execute_with_retry(inputs)
            except Exception as e:  # noqa: BLE001
                return {"success": False, "message_id": None, "error": str(e)}

        @retry(stop=stop_after_attempt(3), wait=wait_exponential(min=1, max=10))
        def _execute_with_retry(self, inputs: dict) -> dict:
            message_id = self._send(inputs)
            return {"success": True, "message_id": message_id, "error": None}

        def _send(self, inputs: dict) -> str:
            return "msg_42"
    ```

=== "TypeScript"

    ```typescript
    import { FunctionModule } from 'apcore-js';
    import type { Context } from 'apcore-js';

    async function withRetry<T>(
      fn: () => Promise<T>,
      attempts = 3,
      baseDelayMs = 1_000,
    ): Promise<T> {
      let lastErr: unknown;
      for (let i = 0; i < attempts; i++) {
        try {
          return await fn();
        } catch (e) {
          lastErr = e;
          const delay = Math.min(baseDelayMs * 2 ** i, 10_000);
          await new Promise((r) => setTimeout(r, delay));
        }
      }
      throw lastErr;
    }

    export default new FunctionModule({
      moduleId: 'executor.email.reliable_send',
      description: 'Send email with retry on transient failures',
      inputSchema: SendEmailInput,
      outputSchema: SendEmailOutput,
      execute: async (inputs, _context: Context) => {
        try {
          const messageId = await withRetry(() => sendEmail(inputs));
          return { success: true, message_id: messageId, error: null };
        } catch (e) {
          return {
            success: false,
            message_id: null,
            error: e instanceof Error ? e.message : String(e),
          };
        }
      },
    });

    async function sendEmail(_inputs: Record<string, unknown>): Promise<string> {
      return 'msg_42';
    }
    ```

=== "Rust"

    ```rust
    use apcore::{Context, Module};
    use apcore::errors::ModuleError;
    use async_trait::async_trait;
    use serde_json::{json, Value};
    use std::time::Duration;

    pub struct ReliableSendModule;

    async fn send_email(_inputs: &Value) -> Result<String, std::io::Error> {
        Ok("msg_42".to_string())
    }

    async fn with_retry<F, Fut, T, E>(mut f: F, attempts: u32, base: Duration) -> Result<T, E>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
    {
        let mut last_err: Option<E> = None;
        for i in 0..attempts {
            match f().await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    last_err = Some(e);
                    let delay = base * 2u32.pow(i);
                    tokio::time::sleep(delay.min(Duration::from_secs(10))).await;
                }
            }
        }
        Err(last_err.expect("at least one attempt"))
    }

    #[async_trait]
    impl Module for ReliableSendModule {
        fn input_schema(&self) -> Value { send_email_input_schema() }
        fn output_schema(&self) -> Value { send_email_output_schema() }
        fn description(&self) -> &str { "Send email with retry on transient failures" }

        async fn execute(
            &self,
            inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Value, ModuleError> {
            match with_retry(|| send_email(&inputs), 3, Duration::from_secs(1)).await {
                Ok(message_id) => Ok(json!({
                    "success": true,
                    "message_id": message_id,
                    "error": null,
                })),
                Err(e) => Ok(json!({
                    "success": false,
                    "message_id": null,
                    "error": e.to_string(),
                })),
            }
        }
    }
    ```

---

## Best Practices

### 1. Schema Design

=== "Python"

    ```python
    from pydantic import BaseModel, Field

    # Good design: fields have descriptions and constraints.
    class GoodInput(BaseModel):
        email: str = Field(
            ..., description="User email", pattern=r"^[\w\.-]+@[\w\.-]+\.\w+$"
        )
        age: int = Field(..., description="User age", ge=0, le=150)

    # Bad design: missing descriptions and constraints.
    class BadInput(BaseModel):
        email: str
        age: int
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    // Good design: fields have descriptions and constraints.
    export const GoodInput = Type.Object({
      email: Type.String({
        description: 'User email',
        pattern: '^[\\w\\.-]+@[\\w\\.-]+\\.\\w+$',
      }),
      age: Type.Integer({
        description: 'User age',
        minimum: 0,
        maximum: 150,
      }),
    });

    // Bad design: missing descriptions and constraints.
    export const BadInput = Type.Object({
      email: Type.String(),
      age: Type.Integer(),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    // Good design: properties carry descriptions and constraints.
    pub fn good_input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "email": {
                    "type": "string",
                    "description": "User email",
                    "pattern": r"^[\w\.-]+@[\w\.-]+\.\w+$"
                },
                "age": {
                    "type": "integer",
                    "description": "User age",
                    "minimum": 0,
                    "maximum": 150
                }
            },
            "required": ["email", "age"]
        })
    }

    // Bad design: missing descriptions and constraints.
    pub fn bad_input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "email": { "type": "string" },
                "age":   { "type": "integer" }
            },
            "required": ["email", "age"]
        })
    }
    ```

### 2. Error Handling

=== "Python"

    ```python
    from apcore import Module, Context
    from apcore.errors import ModuleError

    # Good practice: convert business outcomes into structured outputs and
    # raise ModuleError (with ai_guidance) for unrecoverable failures.
    class GoodModule(Module):
        def execute(self, inputs: dict, context: Context) -> dict:
            try:
                result = self._do_work(inputs)
                return {"success": True, "data": result, "error": None}
            except ValueError as e:
                raise ModuleError(
                    code="EMAIL_INVALID",
                    message=f"Parameter error: {e}",
                    ai_guidance="Ask the user for a valid email (user@domain.com).",
                    user_fixable=True,
                ) from e

        def _do_work(self, inputs: dict) -> dict:
            return {}

    # Bad practice: let unstructured exceptions propagate up.
    class BadModule(Module):
        def execute(self, inputs: dict, context: Context) -> dict:
            return self._do_work(inputs)  # Raw exception leaks framework details
    ```

=== "TypeScript"

    ```typescript
    import { FunctionModule, ModuleError } from 'apcore-js';
    import type { Context } from 'apcore-js';

    // Good practice: throw ModuleError with aiGuidance for unrecoverable cases.
    export const goodModule = new FunctionModule({
      moduleId: 'executor.example.good',
      description: 'Demonstrates structured error handling',
      inputSchema: /* ... */ undefined as never,
      outputSchema: /* ... */ undefined as never,
      execute: async (inputs, _ctx: Context) => {
        try {
          const data = await doWork(inputs);
          return { success: true, data, error: null };
        } catch (e) {
          throw new ModuleError(
            'EMAIL_INVALID',
            e instanceof Error ? e.message : String(e),
            {},
            e instanceof Error ? e : undefined,
            undefined,
            undefined,
            'Ask the user for a valid email (user@domain.com).',
            true,
          );
        }
      },
    });

    async function doWork(_inputs: Record<string, unknown>): Promise<unknown> {
      return {};
    }
    ```

=== "Rust"

    ```rust
    use apcore::{Context, Module};
    use apcore::errors::{ErrorCode, ModuleError};
    use async_trait::async_trait;
    use serde_json::{json, Value};

    // Good practice: convert errors into ModuleError with AI guidance.
    pub struct GoodModule;

    #[async_trait]
    impl Module for GoodModule {
        fn input_schema(&self) -> Value { json!({"type": "object"}) }
        fn output_schema(&self) -> Value { json!({"type": "object"}) }
        fn description(&self) -> &str { "Demonstrates structured error handling" }

        async fn execute(
            &self,
            inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Value, ModuleError> {
            match do_work(&inputs).await {
                Ok(data) => Ok(json!({ "success": true, "data": data, "error": null })),
                Err(e) => Err(ModuleError::new(
                    ErrorCode::GeneralInvalidInput,
                    format!("Parameter error: {e}"),
                )
                .with_ai_guidance("Ask the user for a valid email (user@domain.com)."))
            }
        }
    }

    async fn do_work(_inputs: &Value) -> Result<Value, std::io::Error> {
        Ok(json!({}))
    }
    ```

### 3. Single Responsibility

=== "Python"

    ```python
    from apcore import Module, Context

    # Good design: each module does one thing.
    class SendEmailModule(Module):       # Only sends email
        ...
    class ValidateEmailModule(Module):   # Only validates email
        ...
    class RenderTemplateModule(Module):  # Only renders template
        ...

    # Bad design: one module does too much (validation + rendering + sending).
    class EmailModule(Module):
        def execute(self, inputs: dict, context: Context) -> dict:
            self._validate(inputs)
            html = self._render(inputs)
            return self._send(html)
    ```

=== "TypeScript"

    ```typescript
    // Good design: one module per responsibility, composed by an orchestrator.
    export { default as sendEmailModule } from './send-email.js';        // Only sends email
    export { default as validateEmailModule } from './validate-email.js'; // Only validates
    export { default as renderTemplateModule } from './render-template.js'; // Only renders

    // Bad design: a single module that mixes concerns.
    import { FunctionModule } from 'apcore-js';
    export const emailModule = new FunctionModule({
      moduleId: 'executor.email.everything',
      description: 'Validate, render and send email all at once (avoid)',
      inputSchema: /* ... */ undefined as never,
      outputSchema: /* ... */ undefined as never,
      execute: async (inputs) => {
        // validate, render, then send — three responsibilities in one module.
        return {};
      },
    });
    ```

=== "Rust"

    ```rust
    // Good design: separate types per responsibility.
    pub struct SendEmailModule;        // Only sends email
    pub struct ValidateEmailModule;    // Only validates email
    pub struct RenderTemplateModule;   // Only renders template

    // Bad design: one module that mixes concerns. Prefer to split it.
    use apcore::{Context, Module};
    use apcore::errors::ModuleError;
    use async_trait::async_trait;
    use serde_json::{json, Value};

    pub struct EmailModule;

    #[async_trait]
    impl Module for EmailModule {
        fn input_schema(&self) -> Value { json!({"type": "object"}) }
        fn output_schema(&self) -> Value { json!({"type": "object"}) }
        fn description(&self) -> &str { "Validate, render and send email (avoid)" }

        async fn execute(
            &self,
            _inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Value, ModuleError> {
            // validate, render, then send — three responsibilities in one module.
            Ok(json!({}))
        }
    }
    ```

---

## Testing Guide

### Basic Module Testing

=== "Python"

    ```python
    # test_send_email.py
    import pytest
    from unittest.mock import MagicMock
    from apcore import Context
    from apcore.context import Identity

    def create_test_context(**kwargs) -> Context:
        """Create a test Context with sensible defaults."""
        return Context(
            trace_id="test-trace-id",
            caller_id=kwargs.get("caller_id"),
            call_chain=kwargs.get("call_chain", []),
            executor=kwargs.get("executor", MagicMock()),
            identity=kwargs.get("identity", Identity(id="test", type="user")),
            data=kwargs.get("data", {}),
        )

    class TestSendEmailModule:
        def setup_method(self):
            self.module = SendEmailModule()
            self.context = create_test_context()

        def test_successful_send(self):
            result = self.module.execute(
                inputs={
                    "to": "user@example.com",
                    "subject": "Test",
                    "body": "Hello",
                },
                context=self.context,
            )
            assert result["success"] is True

        def test_invalid_input(self):
            with pytest.raises(Exception):
                self.module.execute(
                    inputs={"to": "", "subject": ""},
                    context=self.context,
                )

        def test_calls_other_module(self):
            mock_executor = MagicMock()
            mock_executor.call.return_value = {"result": "ok"}
            context = create_test_context(executor=mock_executor)

            self.module.execute(
                inputs={"to": "u@example.com", "subject": "s", "body": "b"},
                context=context,
            )

            mock_executor.call.assert_called_once()
    ```

=== "TypeScript"

    ```typescript
    // send-email.test.ts (vitest)
    import { describe, it, expect, vi } from 'vitest';
    import { Context, createIdentity } from 'apcore-js';
    import sendEmailModule from './send-email.js';

    function createTestContext(overrides: Partial<{
      callerId: string | null;
      executor: unknown;
      data: Record<string, unknown>;
    }> = {}): Context {
      return new Context(
        'test-trace-id',
        overrides.callerId ?? null,
        [],
        overrides.executor ?? { call: vi.fn() },
        createIdentity('test', 'user'),
        null,
        overrides.data ?? {},
      );
    }

    describe('sendEmailModule', () => {
      it('returns success on valid inputs', async () => {
        const ctx = createTestContext();
        const result = await sendEmailModule.execute(
          { to: 'user@example.com', subject: 'Test', body: 'Hello' },
          ctx,
        );
        expect(result.success).toBe(true);
      });

      it('forwards calls to executor when nesting', async () => {
        const call = vi.fn().mockResolvedValue({ result: 'ok' });
        const ctx = createTestContext({ executor: { call } });
        await sendEmailModule.execute(
          { to: 'u@example.com', subject: 's', body: 'b' },
          ctx,
        );
        // Assert here when the module under test forwards a call.
      });
    });
    ```

=== "Rust"

    ```rust
    // tests/send_email.rs
    use apcore::context::{Context, Identity};
    use apcore::module::Module;
    use serde_json::{json, Value};
    use std::collections::HashMap;

    fn test_context() -> Context<Value> {
        let identity = Identity::new(
            "test".to_string(),
            "user".to_string(),
            vec![],
            HashMap::new(),
        );
        Context::new(identity)
    }

    #[tokio::test]
    async fn returns_success_on_valid_inputs() {
        let module = SendEmailModule;
        let ctx = test_context();
        let result = module
            .execute(
                json!({
                    "to": "user@example.com",
                    "subject": "Test",
                    "body": "Hello",
                }),
                &ctx,
            )
            .await
            .expect("execute");
        assert_eq!(result["success"], json!(true));
    }

    #[tokio::test]
    async fn rejects_invalid_inputs() {
        let module = SendEmailModule;
        let ctx = test_context();
        let result = module
            .execute(json!({ "to": "", "subject": "" }), &ctx)
            .await;
        assert!(result.is_err() || result.unwrap()["success"] == json!(false));
    }
    ```

### Debugging Tips

1. **Use trace_id to track call chain**: Search for `trace_id` in logs to trace complete call path
2. **Check call_chain**: `context.call_chain` shows the complete path of current call
3. **Pre-validate Schema**: Use `executor.validate()` to check if inputs are valid before execution
4. **Middleware debugging**: Add `LoggingMiddleware` to view inputs/outputs of each call

### Performance Guide

| Recommendation | Explanation |
|------|------|
| Reuse connections | Create connection pool in `on_load()`, close in `on_unload()` |
| Avoid blocking | Long operations **should** use `async def execute()` |
| Control data size | `context.data` is shared along call chain, avoid storing large amounts of data |
| Set timeouts | External calls **must** set reasonable timeout |
| Idempotent design | Modules marked as `idempotent=True` should ensure repeated calls are safe |

---

## module() Registration

> For existing functions or methods, wrap them as standard apcore modules using the `@module` decorator or `module()` function call. See [PROTOCOL_SPEC §5.11](../spec/protocol-spec.md) for detailed specification.

### @module Decorator (Simple Example)

**Before (regular function):**

=== "Python"

    ```python
    def send_email(to: str, subject: str, body: str) -> dict:
        """Send email."""
        # Business logic...
        return {"success": True, "message_id": "msg_123"}
    ```

=== "TypeScript"

    ```typescript
    export function sendEmail(
      to: string,
      subject: string,
      body: string,
    ): { success: boolean; message_id: string } {
      // Business logic...
      return { success: true, message_id: 'msg_123' };
    }
    ```

=== "Rust"

    ```rust
    pub fn send_email(_to: &str, _subject: &str, _body: &str)
        -> serde_json::Value
    {
        // Business logic...
        serde_json::json!({ "success": true, "message_id": "msg_123" })
    }
    ```

**After (apcore module):**

=== "Python"

    ```python
    from apcore import module

    @module(id="email.send", tags=["email", "notification"])
    def send_email(to: str, subject: str, body: str) -> dict:
        """Send email."""
        # Business logic completely unchanged.
        return {"success": True, "message_id": "msg_123"}
    ```

=== "TypeScript"

    ```typescript
    // TypeScript has no decorator equivalent of Python's @module — the
    // SDK exposes `module({...})` / `client.module({...})` instead. Schemas
    // must be supplied explicitly because TypeScript types are erased at
    // runtime (see PROTOCOL_SPEC §5.11.6 / decorator.ts).
    import { Type } from '@sinclair/typebox';
    import { APCore } from 'apcore-js';

    const client = new APCore();

    client.module({
      id: 'email.send',
      description: 'Send email',
      tags: ['email', 'notification'],
      inputSchema: Type.Object({
        to: Type.String(),
        subject: Type.String(),
        body: Type.String(),
      }),
      outputSchema: Type.Object({
        success: Type.Boolean(),
        message_id: Type.String(),
      }),
      execute: (inputs) => ({
        success: true,
        message_id: 'msg_123',
      }),
    });
    ```

=== "Rust"

    ```rust
    // Rust has no attribute-macro equivalent — use `client.module(...)` or
    // build a `FunctionModule::with_description` and register it. Schemas
    // must be supplied explicitly.
    use apcore::APCore;
    use serde_json::json;

    fn register(client: &mut APCore) -> Result<(), apcore::errors::ModuleError> {
        client.module(
            "email.send",
            "Send email",
            json!({
                "type": "object",
                "properties": {
                    "to":      { "type": "string" },
                    "subject": { "type": "string" },
                    "body":    { "type": "string" }
                },
                "required": ["to", "subject", "body"]
            }),
            json!({
                "type": "object",
                "properties": {
                    "success":    { "type": "boolean" },
                    "message_id": { "type": "string" }
                },
                "required": ["success", "message_id"]
            }),
            None,
            vec!["email".into(), "notification".into()],
            None,
            None,
            vec![],
            None,
            |_inputs, _ctx| {
                Box::pin(async move {
                    Ok(json!({ "success": true, "message_id": "msg_123" }))
                })
            },
        )?;
        Ok(())
    }
    ```

In Python, adding one `@module` line turns the function into an apcore module:

- Schema is auto-generated from type annotations
- Description is auto-extracted from the docstring
- The module is auto-registered to the active Registry

In TypeScript and Rust the schemas are explicit because runtime type
information is unavailable — the trade-off is more code, but the resulting
module surface is identical.

### module() Function Call (Register existing class methods)

=== "Python"

    ```python
    from apcore import module

    # Existing business code (no modifications)
    class EmailService:
        def send(self, to: str, subject: str, body: str) -> dict:
            """Send email."""
            return {"success": True}

        def send_template(self, template_id: str, data: dict) -> dict:
            """Send using template."""
            return {"success": True}

    # Register via module() without changing the original code
    service = EmailService()
    module(service.send, id="email.send")
    module(service.send_template, id="email.send_template")
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';
    import { APCore } from 'apcore-js';

    // Existing business code (no modifications)
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

    // Wrap each method as an apcore module by passing a closure to execute.
    client.module({
      id: 'email.send',
      description: 'Send email',
      inputSchema: Type.Object({
        to: Type.String(),
        subject: Type.String(),
        body: Type.String(),
      }),
      outputSchema: Type.Object({ success: Type.Boolean() }),
      execute: (inputs) =>
        service.send(
          inputs.to as string,
          inputs.subject as string,
          inputs.body as string,
        ),
    });

    client.module({
      id: 'email.send_template',
      description: 'Send email using a template',
      inputSchema: Type.Object({
        template_id: Type.String(),
        data: Type.Record(Type.String(), Type.Unknown()),
      }),
      outputSchema: Type.Object({ success: Type.Boolean() }),
      execute: (inputs) =>
        service.sendTemplate(
          inputs.template_id as string,
          inputs.data as Record<string, unknown>,
        ),
    });
    ```

=== "Rust"

    ```rust
    use apcore::APCore;
    use serde_json::{json, Value};
    use std::sync::Arc;

    // Existing business code (no modifications)
    pub struct EmailService;
    impl EmailService {
        pub fn send(&self, _to: &str, _subject: &str, _body: &str) -> Value {
            json!({ "success": true })
        }
        pub fn send_template(&self, _template_id: &str, _data: &Value) -> Value {
            json!({ "success": true })
        }
    }

    pub fn register(client: &mut APCore) -> Result<(), apcore::errors::ModuleError> {
        let service = Arc::new(EmailService);

        let s = Arc::clone(&service);
        client.module(
            "email.send",
            "Send email",
            json!({
                "type": "object",
                "properties": {
                    "to":      { "type": "string" },
                    "subject": { "type": "string" },
                    "body":    { "type": "string" }
                },
                "required": ["to", "subject", "body"]
            }),
            json!({"type": "object", "properties": {"success": {"type": "boolean"}}}),
            None, vec![], None, None, vec![], None,
            move |inputs, _ctx| {
                let s = Arc::clone(&s);
                Box::pin(async move {
                    Ok(s.send(
                        inputs["to"].as_str().unwrap_or_default(),
                        inputs["subject"].as_str().unwrap_or_default(),
                        inputs["body"].as_str().unwrap_or_default(),
                    ))
                })
            },
        )?;

        let s = Arc::clone(&service);
        client.module(
            "email.send_template",
            "Send email using a template",
            json!({
                "type": "object",
                "properties": {
                    "template_id": { "type": "string" },
                    "data":        { "type": "object" }
                },
                "required": ["template_id", "data"]
            }),
            json!({"type": "object", "properties": {"success": {"type": "boolean"}}}),
            None, vec![], None, None, vec![], None,
            move |inputs, _ctx| {
                let s = Arc::clone(&s);
                Box::pin(async move {
                    Ok(s.send_template(
                        inputs["template_id"].as_str().unwrap_or_default(),
                        &inputs["data"],
                    ))
                })
            },
        )?;
        Ok(())
    }
    ```

### Advanced Example (Annotated + async)

=== "Python"

    ```python
    from apcore import module, Context
    from apcore.module import ModuleAnnotations
    from typing import Annotated
    from pydantic import Field

    @module(
        id="email.send",
        annotations=ModuleAnnotations(open_world=True, idempotent=False),
        tags=["email"],
    )
    async def send_email(
        to: Annotated[str, Field(description="Recipient email", pattern=r"^[\w\.-]+@[\w\.-]+\.\w+$")],
        subject: Annotated[str, Field(description="Email subject", max_length=200)],
        body: Annotated[str, Field(description="Email body")],
        cc: Annotated[list[str], Field(description="CC list")] = [],
        context: Context | None = None,
    ) -> dict:
        """Send email module — async via SMTP."""
        # `async def` is auto-detected by the framework's async execution path.
        if context is not None:
            print(f"trace_id: {context.trace_id}")
        return {"success": True, "message_id": "msg_123"}
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';
    import { APCore, createAnnotations } from 'apcore-js';
    import type { Context } from 'apcore-js';

    const client = new APCore();

    const SendEmailInput = Type.Object({
      to: Type.String({
        description: 'Recipient email',
        pattern: '^[\\w\\.-]+@[\\w\\.-]+\\.\\w+$',
      }),
      subject: Type.String({ description: 'Email subject', maxLength: 200 }),
      body: Type.String({ description: 'Email body' }),
      cc: Type.Array(Type.String(), { description: 'CC list', default: [] }),
    });

    const SendEmailOutput = Type.Object({
      success: Type.Boolean(),
      message_id: Type.String(),
    });

    client.module({
      id: 'email.send',
      description: 'Send email module — async via SMTP',
      tags: ['email'],
      annotations: createAnnotations({
        openWorld: true,
        idempotent: false,
      }),
      inputSchema: SendEmailInput,
      outputSchema: SendEmailOutput,
      execute: async (inputs, context: Context) => {
        // `execute` is async by design.
        console.log(`trace_id: ${context.traceId}`);
        return { success: true, message_id: 'msg_123' };
      },
    });
    ```

=== "Rust"

    ```rust
    use apcore::{APCore, ModuleAnnotations};
    use apcore::errors::ModuleError;
    use serde_json::json;

    pub fn register(client: &mut APCore) -> Result<(), ModuleError> {
        let _annotations = ModuleAnnotations {
            open_world: true,
            idempotent: false,
            ..Default::default()
        };

        client.module(
            "email.send",
            "Send email module — async via SMTP",
            json!({
                "type": "object",
                "properties": {
                    "to": {
                        "type": "string",
                        "description": "Recipient email",
                        "pattern": r"^[\w\.-]+@[\w\.-]+\.\w+$"
                    },
                    "subject": {
                        "type": "string",
                        "description": "Email subject",
                        "maxLength": 200
                    },
                    "body": { "type": "string", "description": "Email body" },
                    "cc": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "CC list",
                        "default": []
                    }
                },
                "required": ["to", "subject", "body"]
            }),
            json!({
                "type": "object",
                "properties": {
                    "success":    { "type": "boolean" },
                    "message_id": { "type": "string" }
                },
                "required": ["success", "message_id"]
            }),
            None,
            vec!["email".into()],
            None,
            None,
            vec![],
            None,
            |_inputs, ctx| {
                let trace_id = ctx.trace_id.clone();
                Box::pin(async move {
                    println!("trace_id: {trace_id}");
                    Ok(json!({ "success": true, "message_id": "msg_123" }))
                })
            },
        )?;
        Ok(())
    }
    ```

### ID Generation Rules

- When `id` parameter is specified: use it directly
- When not specified: auto-generate from function's `__module__` + `__qualname__`

### Description Extraction

1. `description` parameter (highest priority)
2. First line of function docstring
3. Default description generated from function name

### Limitations

| Feature | Class-based | module() |
|------|------------|----------|
| Lifecycle hooks (on_load/on_unload) | Supported | Not supported |
| Custom validate() | Supported | Not supported |
| Schema source | Pydantic Model | Auto-generated from type annotations |
| Execution context | `self` + `context` | `context` parameter injection |

---

## External Schema Binding (YAML)

> For scenarios where you cannot modify existing source code at all, use YAML binding files to map functions to apcore modules. See [PROTOCOL_SPEC §5.12](../spec/protocol-spec.md) for detailed specification.

### Complete Binding File Example

```yaml
# bindings/email.binding.yaml
bindings:
  - module_id: "email.send"
    target: "myapp.services.email:send_email"
    description: "Send email"
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
        message_id:
          type: string
      required: [success]

  - module_id: "email.send_template"
    target: "myapp.services.email:EmailService.send_template"
    description: "Send email using template"
    auto_schema: true
```

### auto_schema Mode

When the target function has complete type annotations, you can use `auto_schema: true` to auto-generate Schema:

```yaml
bindings:
  - module_id: "email.send"
    target: "myapp.services.email:send_email"
    auto_schema: true    # Auto-generate from send_email's type annotations
```

Equivalent to `module(send_email, id="email.send")`, but requires no source code modifications.

### Discovery Mechanism Configuration

```yaml
# apcore.yaml
bindings:
  dir: "./bindings"              # Scan directory (default)
  pattern: "*.binding.yaml"      # File matching pattern
  # Or specify file list
  files:
    - "./bindings/email.binding.yaml"
    - "./bindings/payment.binding.yaml"
```

### Multiple Binding Files Management

```
my-project/
├── bindings/
│   ├── email.binding.yaml       # Email-related modules
│   ├── payment.binding.yaml     # Payment-related modules
│   └── user.binding.yaml        # User-related modules
└── apcore.yaml
```

Each binding file is organized by business domain. The framework automatically scans all `*.binding.yaml` files in the `bindings/` directory.

---

## Approach Selection Comparison

| Consideration | Class-based | `@module` Decorator | `module()` Function Call | External Binding |
|------|------------|-----------------|-------------------|-----------------|
| **New development** | Recommended | Usable | Usable | Not recommended |
| **Wrap existing functions** | Not recommended (requires rewrite) | Recommended | Recommended | Usable |
| **Cannot modify source** | Impossible | Impossible | Impossible | Recommended |
| **Need lifecycle management** | Recommended | Not supported | Not supported | Not supported |
| **Cross-language unified config** | Not applicable | Not applicable | Partially applicable | Recommended |
| **Schema flexibility** | Highest (Pydantic) | Medium (type annotations) | Medium (type annotations) | High (hand-written YAML) |

---

## Next Steps

- [Schema Definition Details](./schema-definition.md) - Complete Schema usage
- [ACL Configuration Guide](./acl-configuration.md) - Configure module access permissions
- [Module Interface](../features/module-interface.md) - Module Protocol contract
- [Adapter Development Guide](./adapter-development.md) - Framework adapter development
