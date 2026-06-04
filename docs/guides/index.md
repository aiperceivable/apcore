---
description: "Landing page for the eight apcore user guides, organized as a learning path across foundations, governance and extension, and professional integration topics."
---

# User Guides

> Practical guides for apcore, from beginner to advanced.

This directory contains 8 practical guides covering the complete module development workflow. Documents are arranged by learning path; it is recommended to read the first four in order, then selectively read the rest as needed.

## Document List

### Foundations
- [Creating Modules](./creating-modules.md) — Create modules from scratch, multiple definition approaches
- [Schema Definition](./schema-definition.md) — Mastering input/output structure declaration
- [Testing Modules](./testing-modules.md) — Unit tests, Schema tests, and Mock techniques

### Governance & Extension
- [ACL Configuration](./acl-configuration.md) — Access control rules and security
- [Middleware System](./middleware.md) — Extending execution logic with pre/post hooks

### Professional Integration
- [Multi-Language](./multi-language.md) — Cross-language development using YAML Schema
- [Adapter Development](./adapter-development.md) — Building adapters for Web frameworks
- [Legacy Integration](./integrating-existing-projects.md) — Adopting apcore in existing applications

### Cookbooks & Troubleshooting
- [Cookbooks Overview](#cookbooks) — Scenario-based recipes (Approval, Cancellation, Streaming, etc.)
- [Troubleshooting](./troubleshooting.md) — Common issues and solutions

## Learning Path

1. **Foundations**: Start with [Creating Modules](./creating-modules.md) and [Schema Definition](./schema-definition.md).
2. **Quality**: Read [Testing Modules](./testing-modules.md) before moving to production.
3. **Control**: Master [ACL Configuration](./acl-configuration.md) and [Middleware](./middleware.md) for governance.
4. **Integration**: Explore [Professional Integration](#professional-integration) topics based on your tech stack.
5. **Real-world**: Visit the [Cookbooks](#cookbooks) for specific recipes.

## Cookbooks

Practical "recipes" for common real-world scenarios:

| Recipe | Description |
|--------|-------------|
| [Approval Flow](./cookbook-approval-flow.md) | How to implement human-in-the-loop approval gates |
| [Cancellation](./cookbook-cancellation.md) | Handling long-running task cancellation gracefully |
| [Streaming](./cookbook-streaming.md) | Implementing and consuming incremental module outputs |
| [Observability](./cookbook-observability.md) | Custom tracing, metrics, and structured logging patterns |


Before reading these guides, it is recommended to first understand the [Core Concepts](../concepts.md); for interface details, see the [Feature Specifications](../features/).
