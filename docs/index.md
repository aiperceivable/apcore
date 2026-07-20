---
description: "Documentation home for apcore, the governed protocol-neutral runtime and module standard for agent-callable application capabilities."
---

# apcore

apcore is a **governed, protocol-neutral runtime and module standard** for application capabilities that can be called by agents or code.

Define a capability with required input and output schemas plus behavioral metadata. The runtime applies identity, ACL, approval, validation, middleware, execution, structured errors, and trace context. Independently versioned adapters can then project the same capability to MCP, A2A, CLI, HTTP, or direct code.

## Start Here

- [Getting Started](getting-started.md) — install a maintained SDK and define the first module
- [Positioning](POSITIONING.md) — understand the problem boundary and relationship to MCP/A2A
- [Core Concepts](concepts.md) — modules, registry, executor, context, and schemas
- [Architecture](architecture.md) — how the runtime components fit together
- [Integration Paths](guides/creating-modules.md) — choose a native module, decorator, or external binding

## Source of Truth

- [Protocol Specification](spec/protocol-spec.md) — normative behavior (`1.9.0-draft`)
- [Scope](https://github.com/aiperceivable/apcore/blob/main/SCOPE.md) — what apcore owns and does not own
- [Roadmap](https://github.com/aiperceivable/apcore/blob/main/ROADMAP.md) — current priorities and 1.0 gates
- [Adopters](https://github.com/aiperceivable/apcore/blob/main/ADOPTERS.md) — public adoption evidence
- [Conformance](spec/conformance.md) — cross-language conformance expectations

## Current Implementations

| Language | Package | Maintained release line |
|---|---|---|
| Python | `apcore` | `0.26.0` |
| TypeScript | `apcore-js` | `0.26.0` |
| Rust | `apcore` | `0.26.0` |

Package metadata in each SDK repository is authoritative for a released version.

## Governed Execution

The primary adoption path is:

1. choose one existing application operation;
2. define schemas and behavioral annotations;
3. verify allowed, denied, approval-required, invalid-input, failed, and successful paths;
4. add the one surface adapter required by the caller; and
5. inspect the structured execution evidence.

A schema makes a capability contract machine-readable and validatable. It does not guarantee model comprehension, tool selection, or autonomous recovery.

## Documentation

- [Feature Specifications](features/index.md)
- [Guides](guides/index.md)
- [Schema Definition](guides/schema-definition.md)
- [ACL Configuration](guides/acl-configuration.md)
- [Approval Flow](guides/cookbook-approval-flow.md)
- [Observability](guides/cookbook-observability.md)
- [Troubleshooting](guides/troubleshooting.md)
- [Documentation Map](site-map.md)

## Protocol Adapters

- `apcore-mcp` projects modules as MCP tools
- `apcore-a2a` projects modules as A2A skills and Agent Card metadata
- `apcore-cli` maps modules to commands and arguments
- framework integrations bind HTTP endpoints and apcore modules

Use a protocol SDK directly when a protocol server is sufficient. Use apcore when validation, access, approval, and audit semantics must remain consistent across callers or surfaces.
