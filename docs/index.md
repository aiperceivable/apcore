---
description: "Documentation home for apcore, the governed protocol-neutral runtime and module standard for agent-callable application capabilities."
---

# apcore

apcore is a **governed, protocol-neutral runtime and module standard** for application capabilities that can be called by agents or code.

Define a capability with a description, input and output schemas, and behavioral annotations. The runtime applies identity, ACL, approval, validation, middleware, execution, structured errors, and trace context on every call. Independently versioned adapters then project the same capability to MCP, A2A, CLI, HTTP, or direct code.

A schema makes a capability contract machine-readable and validatable. It does not guarantee that a model will choose the right tool or recover from an error on its own.

## Start Here

- [Getting Started](getting-started.md) — install an SDK and define your first module
- [Core Concepts](concepts.md) — modules, registry, executor, context, schemas, ACL
- [Architecture](architecture.md) — how the runtime components fit together
- [Positioning](POSITIONING.md) — the problem boundary and the relationship to MCP and A2A
- [Glossary](glossary.md) — terminology in one page

## Current Versions

| Component | Package | Version |
|---|---|---|
| Protocol specification | [protocol-spec.md](spec/protocol-spec.md) | `1.62.0` |
| Python SDK | `apcore` | `0.31.0` |
| TypeScript SDK | `apcore-js` | `0.31.0` |
| Rust SDK | `apcore` | `0.31.0` |

The three core SDKs share a release line. Surface adapters (`apcore-mcp`, `apcore-a2a`, `apcore-cli`, `apcore-toolkit`) are versioned independently and declare the core range they support in their package metadata.

## Documentation

- [Guides](guides/index.md) — task-oriented tutorials and cookbooks
- [Features](features/index.md) — one reference page per runtime subsystem
- [Specification](spec/index.md) — the normative protocol, conformance, type mapping, and decision records
- [Troubleshooting](guides/troubleshooting.md) — common failures and their fixes

## Adoption Path

1. Choose one existing application operation.
2. Define its schemas and behavioral annotations.
3. Verify the allowed, denied, approval-required, invalid-input, failed, and successful paths.
4. Add the one surface adapter your caller needs.
5. Inspect the structured execution evidence (traces, errors, events).

## Surface Adapters

- `apcore-mcp` projects modules as MCP tools
- `apcore-a2a` projects modules as A2A skills and Agent Card metadata
- `apcore-cli` maps modules to commands and arguments
- framework integrations (`fastapi-apcore`, `django-apcore`, `flask-apcore`, `nestjs-apcore`, `express-apcore`, `hono-apcore`, `axum-apcore`) bind HTTP endpoints to modules

Use a protocol SDK directly when a protocol server is all you need. Use apcore when validation, access, approval, and audit semantics must stay consistent across callers or surfaces.

## Project

[Scope](https://github.com/aiperceivable/apcore/blob/main/SCOPE.md) · [Roadmap](https://github.com/aiperceivable/apcore/blob/main/ROADMAP.md) · [Adopters](https://github.com/aiperceivable/apcore/blob/main/ADOPTERS.md) · [Changelog](https://github.com/aiperceivable/apcore/blob/main/CHANGELOG.md) · [Governance](https://github.com/aiperceivable/apcore/blob/main/GOVERNANCE.md)
