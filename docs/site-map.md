---
description: "Navigable map of the apcore documentation tree, listing the index, concepts, architecture, features, guides, and spec directories with one-line descriptions of each file."
---

# apcore Documentation

> Complete technical documentation for the apcore (AI-Perceivable Core) standard.

This directory contains all technical documentation for apcore, covering core concepts, architecture design, feature specifications, usage guides, and standard specifications.
For a project overview and quick start, see the [main README](index.md).
For language SDK implementations, see [Implementations](index.md#implementations).

## Directory Overview

```
docs/
├── index.md                           ← Project Overview (former README)
├── concepts.md                        ← Design philosophy & core concepts
├── architecture.md                    ← Framework technical architecture
├── site-map.md                        ← This file (Documentation Map)
├── features/                          ← Feature specifications (for SDK implementors)
│   ├── index.md                       ← Feature overview
│   ├── module-interface.md            ← Module Protocol contract
│   ├── context-object.md              ← Per-invocation execution context
│   ├── apcore-client.md               ← Unified high-level client
│   ├── core-executor.md               ← Core Execution Engine
│   ├── registry-system.md             ← Module Registry and Discovery System
│   ├── schema-system.md               ← Schema System
│   ├── acl-system.md                  ← Access Control System
│   ├── approval-system.md             ← Approval System
│   ├── decorator-bindings.md          ← Decorator and YAML Bindings
│   ├── middleware-system.md           ← Middleware System
│   ├── observability.md               ← Observability System
│   ├── event-system.md                ← Event System
│   ├── system-modules.md              ← System Modules (AI Introspection)
│   └── …                              ← (full list in features/index.md)
├── guides/                            ← Usage guides (8 articles)
│   ├── index.md                       ← Guides overview
│   ├── creating-modules.md            ← Getting started with module creation
│   ├── schema-definition.md           ← Schema definition in detail
│   ├── middleware.md                  ← Middleware development
│   ├── acl-configuration.md           ← ACL permission configuration
│   ├── testing-modules.md             ← Module testing strategies
│   ├── adapter-development.md        ← Adapter development
│   ├── multi-language.md              ← Cross-language development
│   └── integrating-existing-projects.md ← Adoption guide
├── spec/                              ← Framework specifications (for SDK implementors)
    ├── index.md                       ← Specification overview
    ├── algorithms.md                  ← Core algorithm reference
    ├── type-mapping.md                ← Cross-language type mapping
    └── conformance.md                 ← Conformance level definitions
```

Also see the root directory: [protocol-spec.md](./spec/protocol-spec.md) — Complete protocol specification in RFC style (4,450+ lines)

## Documentation Structure

### Concepts & Architecture

| Document | Description |
|----------|-------------|
| [concepts.md](./concepts.md) | apcore's design philosophy, core concepts, and modular philosophy |
| [architecture.md](./architecture.md) | Framework technical architecture, core component interactions, and execution flow |

### [Feature Specifications](./features/index.md)

Implementation-ready feature specifications for SDK developers. Each document defines a specific subsystem's behavior, interfaces, contracts, and test scenarios.

| Feature Spec | Description |
|--------------|-------------|
| [Module Interface](./features/module-interface.md) | Module Protocol — required schema attributes, lifecycle hooks, optional methods, function-based form |
| [Context Object](./features/context-object.md) | Per-invocation state — trace, identity, call chain, executor, redaction, shared `data` map |
| [APCore Client](./features/apcore-client.md) | Unified high-level client managing Registry, Executor, and subsystems |
| [Core Executor](./features/core-executor.md) | Central execution engine with a secured execution lifecycle |
| [Registry System](./features/registry-system.md) | Module discovery, registration, and querying with 8-step pipeline |
| [Schema System](./features/schema-system.md) | Schema loading, validation, `$ref` resolution, and LLM export |
| [ACL System](./features/acl-system.md) | Pattern-based Access Control List with first-match-wins evaluation |
| [Approval System](./features/approval-system.md) | Runtime enforcement of `requires_approval` via pluggable ApprovalHandler |
| [Decorator & YAML Bindings](./features/decorator-bindings.md) | `@module` decorator and YAML-based declarative module creation |
| [Middleware System](./features/middleware-system.md) | Composable middleware pipeline with onion execution model |
| [Observability](./features/observability.md) | Distributed tracing, metrics collection, and structured logging |
| [Event System](./features/event-system.md) | Global event bus, subscribers, and threshold alerting |
| [System Modules](./features/system-modules.md) | Built-in `system.*` modules for AI bidirectional introspection |

For the full list of feature specs see [features/index.md](./features/index.md).

### [Usage Guides](./guides/index.md)

Practical tutorials covering everything from creating your first module to middleware development, ACL configuration, and cross-language development. 8 guides in total, covering the full path from beginner to advanced.

| Guide | Description |
|-------|-------------|
| [Creating Modules](./guides/creating-modules.md) | Create modules from scratch, introducing multiple module definition approaches |
| [Schema Definition](./guides/schema-definition.md) | Detailed Schema definition, mastering input/output structure declaration |
| [Middleware](./guides/middleware.md) | Middleware development, extending pre/post module execution logic |
| [ACL Configuration](./guides/acl-configuration.md) | ACL permission configuration, setting up access control rules between modules |
| [Testing Modules](./guides/testing-modules.md) | Module testing strategies, covering unit tests, Schema tests, and integration tests |
| [Adapter Development](./guides/adapter-development.md) | Developing apcore adapters for third-party web frameworks |
| [Multi-Language Development](./guides/multi-language.md) | Developing modules in multiple languages using YAML Schema |
| [Integrating Projects](./guides/integrating-existing-projects.md) | Adopt apcore in applications that already have their own request-ID system |

### [Framework Specifications](./spec/index.md) - Authoritative Specifications

Formal technical specifications for SDK implementors, defining core algorithms, cross-language type mapping, and conformance levels.

- [Protocol Specification](./spec/protocol-spec.md) - Complete protocol specification in RFC style
- [Type Mapping](./spec/type-mapping.md) - Cross-language type mapping
- [Conformance Definition](./spec/conformance.md) - Implementation conformance levels
- [Algorithm Reference](./spec/algorithms.md) - Core algorithm compendium

## Recommended Reading Order

If you are new to apcore, it is recommended to read in the following order:

1. [Core Concepts](./concepts.md) — Understand the design philosophy
2. [Creating Modules](./guides/creating-modules.md) — Hands-on: create your first module
3. [Schema Definition](./guides/schema-definition.md) — Master input/output Schema
4. [Module Interface](./features/module-interface.md) — Learn the complete interface definition
5. [Context Object](./features/context-object.md) — Understand the execution context
6. [Middleware](./guides/middleware.md) — Extend the execution flow
7. [ACL Configuration](./guides/acl-configuration.md) — Configure access control

After completing the above, you can read the advanced guides on testing, adapter development, multi-language development, etc. as needed.

## Concept Index

Quickly find authoritative definitions for concepts:

| Concept | Authoritative Definition | Quick Reference |
|---------|--------------------------|-----------------|
| APCore Client | [apcore-client.md](./features/apcore-client.md) | [Getting Started](./getting-started.md) |
| Module | [module-interface.md](./features/module-interface.md) | [README](index.md#module-development) |
| ModuleAnnotations | [module-interface.md#moduleannotations](./features/module-interface.md#moduleannotations) | [README](index.md#schema-system) |
| Context | [context-object.md](./features/context-object.md) | [README](index.md#context-object) |
| Canonical ID | [protocol-spec.md §2](./spec/protocol-spec.md#2-naming-specification) | [README](index.md#directory-as-id) |
| Registry | [registry-system.md](./features/registry-system.md) | [README](index.md#quick-start) |
| Executor | [core-executor.md](./features/core-executor.md) | [README](index.md#quick-start) |
| ACL | [protocol-spec.md §6](./spec/protocol-spec.md#6-acl-specification) | [README](index.md#acl-access-control) |
| ApprovalHandler | [approval-system.md](./features/approval-system.md) | [protocol-spec.md §7](./spec/protocol-spec.md#7-approval-system) |
| Middleware | [middleware.md](./guides/middleware.md) | [README](index.md#middleware) |

For a single-page reference of all terminology see the [Glossary](./glossary.md).
