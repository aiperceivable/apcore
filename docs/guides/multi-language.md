---
description: "Developing apcore modules across the Python, TypeScript, and Rust SDKs: shared YAML schema files, canonical module IDs, ID maps, and cross-language pitfalls."
---

# Multi-Language Development Guide

> Use one YAML schema and one module ID to implement or call the same module from Python, TypeScript, and Rust.

## 1. Overview

apcore has three SDKs — Python, TypeScript, and Rust. A module written in one of them is described by the same things in all of them:

- a **canonical module ID** (`executor.email.send_email`),
- an **input and output JSON Schema** (Draft 2020-12),
- **annotations** (`readonly`, `destructive`, `requires_approval`, …),
- **error codes** from the shared error catalogue.

Keeping those four in one place is what lets a Python service, a TypeScript tool server, and a Rust worker agree on what `executor.email.send_email` accepts and returns.

```text
            schemas/executor/email/send_email.schema.yaml
                        (shared contract)
                               │
           ┌───────────────────┼───────────────────┐
           ▼                   ▼                   ▼
      apcore-python     apcore-typescript      apcore-rust
           │                   │                   │
           └───────────────────┼───────────────────┘
                               ▼
                 executor.email.send_email
                    (canonical module ID)
```

| Principle | What it means in practice |
|------|------|
| **The schema file is the source of truth** | Input/output structure lives in a `*.schema.yaml` file that each SDK loads at runtime |
| **The canonical ID is the address** | Every SDK calls the module by the same dot-separated snake_case ID |
| **Types map by a published table** | JSON Schema types map to native types as described in the [Type Mapping Specification](../spec/type-mapping.md) |
| **Behaviour travels as annotations** | `readonly`, `destructive`, `requires_approval`, and the rest mean the same thing in every SDK |

---

## 2. Canonical Module IDs

A canonical ID is a dot-separated sequence of lowercase snake_case segments:

```text
Pattern:    ^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$
Max length: 192 characters

Examples:
  executor.validator.db_params
  orchestrator.engine.task_flow
  api.handler.task_submit
  common.util.sql_parser
```

Every SDK calls a module by that ID:

=== "Python"

    ```python
    from apcore import APCore

    client = APCore()
    client.discover()

    result = client.call(
        "executor.email.send_email",
        {"to": "user@example.com", "subject": "Hello", "body": "World"},
    )
    ```

=== "TypeScript"

    ```typescript
    import { APCore } from 'apcore-js';

    const client = new APCore();
    await client.discover();

    const result = await client.call('executor.email.send_email', {
      to: 'user@example.com',
      subject: 'Hello',
      body: 'World',
    });
    ```

=== "Rust"

    ```rust
    use apcore::errors::ModuleError;
    use apcore::APCore;
    use serde_json::json;

    async fn send(client: &APCore) -> Result<serde_json::Value, ModuleError> {
        // call(module_id, inputs, context, version_hint)
        client
            .call(
                "executor.email.send_email",
                json!({ "to": "user@example.com", "subject": "Hello", "body": "World" }),
                None,
                None,
            )
            .await
    }
    ```

### 2.1 How each SDK derives an ID from a file

| SDK | Rule | Example |
|------|------|------|
| Python | Path under the extensions root, separators → `.`, `.py` dropped | `extensions/executor/validator/db_params.py` → `executor.validator.db_params` |
| TypeScript | Same rule, `.ts` / `.js` dropped. The file name is used as-is, so it must already be snake_case — `dbParams.ts` yields `executor.validator.dbParams`, which is not a legal ID and is skipped with a warning | `extensions/executor/validator/db_params.ts` → `executor.validator.db_params` |
| Rust | Modules are compiled into the binary, so they are usually registered with an explicit ID: `client.register("executor.validator.db_params", Box::new(DbParamsValidator))` | — |

Class, struct, and function names never affect a discovered ID. See [protocol-spec §2.1](../spec/protocol-spec.md#21-directory-as-id-core-rule) for the derivation algorithm and [§2.7](../spec/protocol-spec.md#27-id-formal-grammar) for the grammar.

---

## 3. Sharing a YAML Schema

### 3.1 The schema file

Schema files live under `schema.root` (default `./schemas`) at a path mirroring the module ID:

```text
schemas/
├── executor/
│   ├── email/
│   │   └── send_email.schema.yaml      → executor.email.send_email
│   └── validator/
│       └── db_params.schema.yaml       → executor.validator.db_params
└── common/
    └── pagination.schema.yaml          → reusable definitions for $ref
```

```yaml
# schemas/executor/email/send_email.schema.yaml
$schema: "https://apcore.dev/schema/v1"
version: "1.0.0"
module_id: "executor.email.send_email"
description: "Send an email through the configured SMTP relay"

input_schema:
  type: object
  properties:
    to:
      type: string
      description: "Recipient email address"
    subject:
      type: string
      maxLength: 200
      description: "Email subject"
    body:
      type: string
      description: "Plain-text email body"
    smtp_port:
      type: integer
      description: "SMTP server port"
      default: 587
  required: [to, subject, body]
  additionalProperties: false

output_schema:
  type: object
  properties:
    success:
      type: boolean
      description: "Whether the send was accepted by the relay"
    message_id:
      type: string
      description: "Relay-assigned message ID"
  required: [success]
```

`description`, `input_schema`, and `output_schema` are required. The file format is defined in [protocol-spec §4.2](../spec/protocol-spec.md#42-schema-format).

### 3.2 Loading it at runtime

All three SDKs read schema files at runtime with `SchemaLoader` — there is no code-generation step. The loader reads `schema.root`, `schema.strategy`, and `schema.max_ref_depth` from the `Config` you give it and resolves `$ref`s when it loads the file.

=== "Python"

    ```python
    from apcore import APCore, Config, Context, SchemaLoader

    loader = SchemaLoader(Config.from_defaults())
    schema_def = loader.load("executor.email.send_email")
    input_schema, output_schema = loader.get_schema("executor.email.send_email")


    class SendEmailModule:
        description = schema_def.description
        input_schema = input_schema.model      # Pydantic model generated from the YAML
        output_schema = output_schema.model

        def execute(self, inputs: dict, context: Context) -> dict:
            return {"success": True, "message_id": "msg_123"}


    client = APCore()
    client.register("executor.email.send_email", SendEmailModule())
    ```

=== "TypeScript"

    ```typescript
    import { APCore, Config, FunctionModule, SchemaLoader } from 'apcore-js';

    const loader = new SchemaLoader(Config.fromDefaults());
    const schemaDef = loader.load('executor.email.send_email');
    const [input, output] = loader.getSchema('executor.email.send_email');

    const client = new APCore();
    client.register(
      'executor.email.send_email',
      new FunctionModule({
        moduleId: 'executor.email.send_email',
        description: schemaDef.description,
        inputSchema: input.schema, // TypeBox schema built from the YAML
        outputSchema: output.schema,
        execute: async () => ({ success: true, message_id: 'msg_123' }),
      }),
    );
    ```

=== "Rust"

    ```rust
    use apcore::errors::ModuleError;
    use apcore::{APCore, Config, Context, Module, SchemaDefinition, SchemaLoader};
    use async_trait::async_trait;
    use serde_json::{json, Value};

    pub struct SendEmailModule {
        schema: SchemaDefinition,
    }

    #[async_trait]
    impl Module for SendEmailModule {
        fn input_schema(&self) -> Value {
            self.schema.input_schema.clone()
        }

        fn output_schema(&self) -> Value {
            self.schema.output_schema.clone()
        }

        fn description(&self) -> &str {
            &self.schema.description
        }

        async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
            Ok(json!({ "success": true, "message_id": "msg_123" }))
        }
    }

    #[tokio::main]
    async fn main() -> Result<(), ModuleError> {
        let config = Config::default();
        // Reads <schema.root>/executor/email/send_email.schema.yaml at startup.
        let schema = SchemaLoader::with_config(&config, None).load("executor.email.send_email")?;

        let client = APCore::with_config(config);
        client.register("executor.email.send_email", Box::new(SendEmailModule { schema }))?;
        Ok(())
    }
    ```

`schema.strategy` decides what wins when a module has both a YAML file and a schema written in code (`yaml_first`, `native_first`, or `yaml_only`); see [Schema Definition § Schema Loading Strategy](./schema-definition.md#8-schema-loading-strategy).

A [binding file](./creating-modules.md#external-schema-binding-yaml) can also point at a shared schema with `schema_ref`, which is the zero-code way to reuse the same contract.

---

## 4. What Every SDK Provides

SDK-specific idioms (Pydantic, TypeBox, `serde_json`) are covered in each SDK's own repository. The shared surface is:

| Area | What it means |
|------|------|
| Module interface | `execute()`, `description`, `input_schema`, `output_schema`, optional `annotations` — see [Module Interface](../features/module-interface.md) |
| Schema validation | Inputs are validated against `input_schema` before `execute()`, outputs against `output_schema` after |
| Canonical IDs | Directory-derived IDs, plus the ID map below for files whose names are not already canonical |
| Errors | Structured errors with the shared error codes — see [Error System](../features/error-system.md) |

---

## 5. ID Map Configuration

### 5.1 When you need one

Discovery derives an ID from the file path. You need an ID map only when a file's path does **not** produce the ID you want — for example a legacy file name that is not snake_case, or a module you are moving without changing its public ID. Projects whose file names are already snake_case need no ID map.

### 5.2 Configuration

`id_map.overrides` is the only ID-map setting that has an effect. Its value is the path of an ID-map file, resolved like `extensions.root`:

```yaml
# apcore.yaml
extensions:
  root: ./extensions
id_map:
  overrides: ./id_map.yaml
```

```yaml
# id_map.yaml
mappings:
  - file: executor/validator/DbParams.py     # path relative to the extensions root
    id: executor.validator.db_params         # ID to register instead of the derived one
  - file: legacy/sendMail.ts
    id: executor.email.send_email
```

Each entry replaces the ID that discovery would have derived for that file. The map is applied during directory discovery in all three SDKs; modules registered explicitly with `register()` keep the ID you pass. A constructor argument (`Registry(id_map_path=…)` in Python, `idMapPath` in TypeScript, `DefaultDiscoverer::with_id_map` in Rust) takes precedence over the config key.

---

## 6. Type Mapping

How JSON Schema types map to Python, TypeScript, and Rust types — including nullable types, enums, `format`, large integers, and round-trip fidelity — is specified in one place: the [Type Mapping Specification](../spec/type-mapping.md). Note that `format` is an annotation, not a type binding: no SDK turns `format: date-time` into a native date type ([type-mapping §11.1](../spec/type-mapping.md#111-format-keyword)).

---

## 7. Common Pitfalls

### 7.1 Integer precision (JavaScript's 53-bit limit)

JavaScript numbers are IEEE 754 doubles, so integers above `2^53 - 1` (`9007199254740991`) lose precision when parsed:

```javascript
JSON.parse('{"order_id": 9007199254740993}');
// → { order_id: 9007199254740992 }
```

If an ID or counter can exceed that range, transmit it as a string:

```yaml
properties:
  order_id:
    type: string
    pattern: "^[0-9]+$"
    description: "Order ID (string, to avoid precision loss)"
```

See [type-mapping §14.1](../spec/type-mapping.md#141-large-integer-precision-loss).

### 7.2 Date-times and time zones

| Rule | Why |
|------|------|
| Store and transmit in UTC | Languages default to different local-time behaviour |
| Always include a zone offset | "Naive" times are ambiguous across services |
| Use ISO 8601 (`2026-02-07T10:30:00Z`) | Every SDK can parse it |

```yaml
properties:
  created_at:
    type: string
    format: date-time
    description: "Creation time (ISO 8601, UTC)"
    x-examples: ["2026-02-07T10:30:00Z"]
```

### 7.3 Unicode normalization

`"café"` (e + combining accent) and `"café"` (precomposed é) look identical but are different strings. apcore passes strings through unchanged, so normalize (for example to NFC) at your own boundary before comparing, hashing, or de-duplicating text. You can record the expectation for callers with `x-constraints`:

```yaml
properties:
  name:
    type: string
    description: "Display name"
    x-constraints: "Send NFC-normalized text"
```

### 7.4 `null` vs. empty vs. missing

```json
{"name": null}
{"name": ""}
{}
```

These are three different inputs: present-and-null, present-and-empty, and absent. Express which ones you accept with `required` and the `type` list:

```yaml
properties:
  required_field:
    type: string                # must be present, must be a string
  optional_field:
    type: string
    default: "default value"    # may be absent; a string when present
  nullable_field:
    type: ["string", "null"]    # must be present, may be null
required: [required_field, nullable_field]
```

### 7.5 Floating-point precision

`0.1 + 0.2` is `0.30000000000000004` in every language that uses IEEE 754. For money, use integer minor units or a decimal string:

```yaml
properties:
  amount_cents:
    type: integer
    minimum: 0
    description: "Amount in cents, e.g. 1999 for 19.99"
  amount:
    type: string
    pattern: "^\\d+\\.\\d{2}$"
    description: "Amount as a decimal string, e.g. \"19.99\""
```

### 7.6 Enum values

Languages have different enum naming conventions. On the wire, always send the exact string the schema lists — typically snake_case — and map to your language's enum type inside the module:

```yaml
properties:
  status:
    type: string
    enum: ["pending", "in_progress", "completed", "failed"]
```

---

## Next Steps

- [Schema Definition Guide](./schema-definition.md) — Writing schemas in each SDK
- [Creating Modules Guide](./creating-modules.md) — Module creation tutorial
- [Testing Modules Guide](./testing-modules.md) — Testing strategies
- [Type Mapping Specification](../spec/type-mapping.md) — Canonical type mapping
- [Architecture Design](../architecture.md) — Overall system architecture
