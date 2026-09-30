<div align="center">
  <img src="./apcore-logo.svg" alt="apcore logo" width="200"/>
</div>

# apcore — AI-Perceivable Core

> **Define a governed capability once. Expose it through any supported surface.**

**[📖 Documentation](https://apcore.aiperceivable.com/)** · [Getting Started](https://apcore.aiperceivable.com/getting-started/) · [Protocol Specification](./docs/spec/protocol-spec.md)

[![License](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![OpenSSF Best Practices](https://www.bestpractices.dev/projects/12294/badge)](https://www.bestpractices.dev/projects/12294)
[![Python Version](https://img.shields.io/badge/python-3.11%2B-blue)](https://github.com/aiperceivable/apcore-python)
[![TypeScript Version](https://img.shields.io/badge/TypeScript-Node_18%2B-blue)](https://github.com/aiperceivable/apcore-typescript)
[![Rust Version](https://img.shields.io/badge/Rust-1.75%2B-blue)](https://github.com/aiperceivable/apcore-rust)

apcore is a **governed, protocol-neutral runtime and module standard** for application capabilities that agents or code can call.

You define a capability once — a description, an input schema, an output schema and behavioral annotations. The runtime then applies identity, access control, approval, validation, middleware, execution, structured errors and trace context on every call. Surface adapters project the same capability to MCP, A2A, CLI, HTTP or direct code.

MCP tells an agent what it *can* call. apcore decides whether *this* call — with these arguments, by this identity — should run at all, and leaves evidence that it did.

This repository holds the **protocol specification**, JSON Schemas and cross-language conformance fixtures. The runtimes live in separate repositories:

| SDK | Language | Install | Repository |
|-----|----------|---------|------------|
| `apcore` | Python 3.11+ | `pip install apcore` | [apcore-python](https://github.com/aiperceivable/apcore-python) |
| `apcore-js` | TypeScript (Node 18+) | `npm install apcore-js` | [apcore-typescript](https://github.com/aiperceivable/apcore-typescript) |
| `apcore` | Rust 1.75+ | `cargo add apcore` | [apcore-rust](https://github.com/aiperceivable/apcore-rust) |

Current versions are listed on the [documentation home](./docs/index.md).

---

## Quick Start

### Python

```python
from apcore import APCore

client = APCore()

@client.module(id="math.add", description="Add two numbers")
def add(a: int, b: int) -> dict:
    return {"sum": a + b}

print(client.call("math.add", {"a": 10, "b": 5}))  # {'sum': 15}
```

### TypeScript

```typescript
import { Type } from '@sinclair/typebox';
import { APCore } from 'apcore-js';

const client = new APCore();

client.module({
  id: 'math.add',
  description: 'Add two numbers',
  inputSchema: Type.Object({ a: Type.Number(), b: Type.Number() }),
  outputSchema: Type.Object({ sum: Type.Number() }),
  execute: (inputs) => ({ sum: (inputs.a as number) + (inputs.b as number) }),
});

console.log(await client.call('math.add', { a: 10, b: 5 })); // { sum: 15 }
```

### Rust

```rust
use apcore::errors::ModuleError;
use apcore::{APCore, Context, Module};
use async_trait::async_trait;
use serde_json::{json, Value};

struct AddModule;

#[async_trait]
impl Module for AddModule {
    fn description(&self) -> &str { "Add two numbers" }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"a": {"type": "integer"}, "b": {"type": "integer"}},
            "required": ["a", "b"]
        })
    }

    fn output_schema(&self) -> Value {
        json!({"type": "object", "properties": {"sum": {"type": "integer"}}})
    }

    async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        let a = inputs["a"].as_i64().unwrap_or(0);
        let b = inputs["b"].as_i64().unwrap_or(0);
        Ok(json!({"sum": a + b}))
    }
}

#[tokio::main]
async fn main() -> Result<(), ModuleError> {
    let client = APCore::new();
    client.register("math.add", Box::new(AddModule))?;
    let result = client.call("math.add", json!({"a": 10, "b": 5}), None, None).await?;
    println!("{result}"); // {"sum":15}
    Ok(())
}
```

The [Getting Started guide](https://apcore.aiperceivable.com/getting-started/) continues with preflight validation, streaming, directory-based discovery, middleware and system modules.

---

## What the Runtime Enforces

| Concern | Mechanism | Reference |
|---|---|---|
| **Contract** | Every module declares `description`, `input_schema` and `output_schema` (JSON Schema Draft 2020-12); inputs and outputs are validated on every call | [Schema System](./docs/features/schema-system.md) |
| **Behavior** | Annotations such as `readonly`, `destructive`, `idempotent`, `requires_approval`, `open_world`, `streaming`, `cacheable`, `paginated` | [Module Interface](./docs/features/module-interface.md) |
| **Access** | Pattern-based ACL rules evaluated first-match-wins, with identity, role and call-depth conditions; `default_effect: deny` | [ACL System](./docs/features/acl-system.md) |
| **Approval** | A human-in-the-loop gate for modules that require approval, through a pluggable `ApprovalHandler` | [Approval System](./docs/features/approval-system.md) |
| **Safety** | Call-depth limits and circular-call detection across nested calls | [Call Chain Guard](./docs/features/call-chain-guard.md) |
| **Extension** | Onion-model middleware and a configurable step pipeline | [Middleware](./docs/features/middleware-system.md) · [Execution Pipeline](./docs/features/execution-pipeline.md) |
| **Evidence** | W3C trace context, tracing, metrics, usage, redaction of `x-sensitive` fields, events and audit | [Observability](./docs/features/observability.md) |
| **Recovery** | Structured errors with `retryable`, `ai_guidance`, `user_fixable` and `suggestion` | [Error System](./docs/features/error-system.md) |

Every call runs the same eleven-step pipeline:

```text
 1. context_creation    trace_id, caller_id, call_chain, identity
 2. call_chain_guard    depth limit and circular-call detection
 3. module_lookup       resolve the module in the registry
 4. acl_check           caller → target permission
 5. approval_gate       human approval when required
 6. middleware_before   before() hooks, in order
 7. input_validation    validate inputs against input_schema
 8. execute             module.execute() (or stream())
 9. output_validation   validate output against output_schema
10. middleware_after    after() hooks, in reverse order
11. return_result
    on error: on_error() hooks in reverse order
```

Module IDs derive from the file path under the extensions root — `extensions/executor/email/send_email.py` becomes `executor.email.send_email` — so the same directory layout yields the same IDs in every language.

---

## Surface Adapters

Adapters are independently versioned and available for Python, TypeScript and Rust under the same package name.

| Adapter | Projects modules as | Repository |
|---|---|---|
| `apcore-mcp` | MCP tools (with a Tool Explorer UI) | [apcore-mcp](https://github.com/aiperceivable/apcore-mcp) |
| `apcore-a2a` | A2A skills and Agent Card metadata | [apcore-a2a](https://github.com/aiperceivable/apcore-a2a) |
| `apcore-cli` | CLI commands and arguments | [apcore-cli](https://github.com/aiperceivable/apcore-cli) |
| `apcore-toolkit` | Shared scanners, schema extraction and output writers for adapter authors | [apcore-toolkit](https://github.com/aiperceivable/apcore-toolkit) |

Framework integrations bind HTTP endpoints to modules: `fastapi-apcore`, `django-apcore`, `flask-apcore`, `nestjs-apcore`, `express-apcore`, `hono-apcore`, `axum-apcore`. To write your own, see the [Adapter Development guide](./docs/guides/adapter-development.md).

Use a protocol SDK directly when a single protocol server is all you need. Use apcore when validation, access, approval and audit rules must stay the same across more than one caller or surface — see [Positioning](./docs/POSITIONING.md).

---

## Documentation

| Start here | Build | Reference |
|---|---|---|
| [Getting Started](./docs/getting-started.md) | [Guides](./docs/guides/index.md) — creating modules, schemas, ACL, middleware, testing, cookbooks | [Feature reference](./docs/features/index.md) — one page per subsystem |
| [Core Concepts](./docs/concepts.md) | [Troubleshooting](./docs/guides/troubleshooting.md) | [Specification](./docs/spec/index.md) — normative protocol, conformance, type mapping |
| [Architecture](./docs/architecture.md) | [Multi-language development](./docs/guides/multi-language.md) | [Decision register](./docs/spec/decision-register.md) — why the spec says what it says |
| [Glossary](./docs/glossary.md) | | [CHANGELOG](./CHANGELOG.md) |

Project documents: [Scope](./SCOPE.md) · [Roadmap](./ROADMAP.md) · [Adopters](./ADOPTERS.md) · [Governance](./GOVERNANCE.md) · [Maintainers](./MAINTAINERS.md) · [Security](./SECURITY.md) · [Code of Conduct](./CODE_OF_CONDUCT.md)

---

## Contributing

Questions and ideas go to [GitHub Discussions](https://github.com/aiperceivable/apcore/discussions). Specification feedback, new SDKs, adapters and documentation fixes are welcome — read [CONTRIBUTING.md](./CONTRIBUTING.md) first. Never report a vulnerability in a public issue; follow [SECURITY.md](./SECURITY.md).

## License

Apache 2.0
