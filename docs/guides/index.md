---
description: "Landing page for the apcore user guides: foundations, governance, integration, task-focused cookbooks, and troubleshooting, arranged as a learning path."
---

# User Guides

> Practical, task-oriented guides for building with apcore in Python, TypeScript, and Rust.

The guides are arranged as a learning path. Read the Foundations in order, then pick the Governance and Integration topics that match your project. Cookbooks are short end-to-end recipes for one scenario each; Troubleshooting is where to go when something does not behave as expected.

## Foundations

- [Creating Modules](./creating-modules.md) — Class-based modules, `module()` registration, and YAML bindings for existing code
- [Schema Definition](./schema-definition.md) — Declaring input/output schemas, constraints, and LLM extension fields
- [Testing Modules](./testing-modules.md) — Unit, schema, and integration tests for modules

## Governance

- [ACL Configuration](./acl-configuration.md) — Access control rules, audit logging, and ACL testing
- [Writing Middleware](./middleware.md) — Adding before/after/on_error hooks around module calls

## Integration

- [Multi-Language](./multi-language.md) — Sharing schemas and module IDs across the Python, TypeScript, and Rust SDKs
- [Adapter Development](./adapter-development.md) — Exposing modules through a web framework
- [Integrating Existing Projects](./integrating-existing-projects.md) — Adopting apcore in an application that already has request and correlation IDs

## Cookbooks

End-to-end recipes for one scenario each. They link to the feature specifications rather than restating them.

| Recipe | What it covers |
|--------|----------------|
| [Approval Flow](./cookbook-approval-flow.md) | Gating a module behind a human decision with an `ApprovalHandler` |
| [Cancellation](./cookbook-cancellation.md) | Cooperative cancellation of long-running modules via the context's cancel token |
| [Streaming](./cookbook-streaming.md) | Producing and consuming incremental output with `stream()` |
| [Observability](./cookbook-observability.md) | Tracing, structured logging, and redaction of sensitive fields |

## Troubleshooting

- [Troubleshooting](./troubleshooting.md) — Frequently asked questions, an error-code → cause → fix table, and diagnostic commands

## Learning Path

1. **Foundations**: Start with [Creating Modules](./creating-modules.md) and [Schema Definition](./schema-definition.md).
2. **Quality**: Read [Testing Modules](./testing-modules.md) before moving to production.
3. **Governance**: Set up [ACL Configuration](./acl-configuration.md) and learn [Writing Middleware](./middleware.md).
4. **Integration**: Pick the [Integration](#integration) topics that match your stack.
5. **Real-world**: Use the [Cookbooks](#cookbooks) for specific scenarios.

Before reading these guides, read the [Core Concepts](../concepts.md); for interface details, see the [Feature Specifications](../features/index.md).
