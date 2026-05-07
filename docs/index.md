# apcore Documentation

> Complete technical documentation for the apcore (AI-Perceivable Core) standard.

This directory contains all technical documentation for apcore, covering core concepts, architecture design, feature specifications, API references, usage guides, and standard specifications.
For a project overview and quick start, see the [main README](overview.md).
For language SDK implementations, see [Implementations](overview.md#implementations).

## Directory Overview

```
docs/
├── index.md                           ← This file (navigation index)
├── concepts.md                        ← Design philosophy & core concepts
├── architecture.md                    ← Framework technical architecture
├── api/                               ← API reference (authoritative definitions)
│   ├── index.md                       ← API overview
│   ├── client-api.md                  ← APCore unified client
│   ├── module-interface.md            ← Module base class interface
│   ├── context-object.md              ← Context execution context
│   ├── registry-api.md                ← Registry center
│   └── executor-api.md                ← Executor
├── features/                          ← Feature specifications (for SDK implementors)
│   ├── index.md                       ← Feature overview
│   ├── acl-system.md                  ← Access Control System
│   ├── approval-system.md             ← Approval System
│   ├── core-executor.md               ← Core Execution Engine
│   ├── decorator-bindings.md          ← Decorator and YAML Bindings
│   ├── middleware-system.md           ← Middleware System
│   ├── observability.md               ← Observability System
│   ├── event-system.md                ← Event System
│   ├── system-modules.md              ← System Modules (AI Introspection)
│   ├── registry-system.md             ← Module Registry and Discovery System
│   └── schema-system.md              ← Schema System
├── guides/                            ← Usage guides (8 articles)
│   ├── index.md                       ← Guides overview
│   ├── creating-modules.md            ← Getting started with module creation
│   ├── schema-definition.md           ← Schema definition in detail
│   ├── middleware.md                   ← Middleware development
│   ├── acl-configuration.md           ← ACL permission configuration
│   ├── testing-modules.md             ← Module testing strategies
│   ├── adapter-development.md         ← Adapter development
│   ├── multi-language.md              ← Cross-language development
│   └── integrating-existing-projects.md ← Adoption guide
├── spec/                              ← Framework specifications (for SDK implementors)
    ├── index.md                       ← Specification overview
    ├── algorithms.md                  ← Core algorithm reference (24 algorithms)
    ├── type-mapping.md                ← Cross-language type mapping
    └── conformance.md                 ← Conformance level definitions
```

Also see the root directory: [PROTOCOL_SPEC.md](../PROTOCOL_SPEC.md) — Complete protocol specification in RFC style (4,450+ lines)

## Documentation Structure

### Concepts & Architecture

| Document | Description |
|----------|-------------|
| [concepts.md](./concepts.md) | apcore's design philosophy, core concepts, and modular philosophy |
| [architecture.md](./architecture.md) | Framework technical architecture, core component interactions, and execution flow |

### [API Reference](./api/) - Authoritative Definitions

Core interface definitions for the module system, including complete API documentation for the Module base class, Context object, Registry center, and Executor.

| API Document | Description |
|--------------|-------------|
| [APCore Client](./api/client-api.md) | Unified client API (recommended entry point) |
| [Module Interface](./api/module-interface.md) | Complete definition of the Module base class |
| [Context Object](./api/context-object.md) | Complete definition of the execution context |
| [Registry API](./api/registry-api.md) | Module registry center API |
| [Executor API](./api/executor-api.md) | Module executor API |

### [Feature Specifications](./features/)

Implementation-ready feature specifications for SDK developers. Each document defines a specific subsystem's behavior, interfaces, acceptance criteria, and test scenarios.

| Feature Spec | Description |
|--------------|-------------|
| [ACL System](./features/acl-system.md) | Pattern-based Access Control List with first-match-wins evaluation |
| [Approval System](./features/approval-system.md) | Runtime enforcement of `requires_approval` via pluggable ApprovalHandler |
| [Core Executor](./features/core-executor.md) | Central execution engine with a secured execution lifecycle |
| [Decorator & YAML Bindings](./features/decorator-bindings.md) | `@module` decorator and YAML-based declarative module creation |
| [Middleware System](./features/middleware-system.md) | Composable middleware pipeline with onion execution model |
| [Observability](./features/observability.md) | Distributed tracing, metrics collection, and structured logging |
| [Event System](./features/event-system.md) | Global event bus, subscribers, and threshold alerting |
| [System Modules](./features/system-modules.md) | Built-in `system.*` modules for AI bidirectional introspection |
| [Registry System](./features/registry-system.md) | Module discovery, registration, and querying with 8-step pipeline |
| [Schema System](./features/schema-system.md) | Schema loading, validation, `$ref` resolution, and LLM export |

### [Usage Guides](./guides/)

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

### [Framework Specifications](./spec/) - Authoritative Specifications

Formal technical specifications for SDK implementors, defining core algorithms, cross-language type mapping, and conformance levels.

- [Protocol Specification](../PROTOCOL_SPEC.md) - Complete protocol specification in RFC style
- [Type Mapping](./spec/type-mapping.md) - Cross-language type mapping
- [Conformance Definition](./spec/conformance.md) - Implementation conformance levels
- [Algorithm Reference](./spec/algorithms.md) - Core algorithm compendium

## Recommended Reading Order

If you are new to apcore, it is recommended to read in the following order:

1. [Core Concepts](./concepts.md) — Understand the design philosophy
2. [Creating Modules](./guides/creating-modules.md) — Hands-on: create your first module
3. [Schema Definition](./guides/schema-definition.md) — Master input/output Schema
4. [Module Interface](./api/module-interface.md) — Learn the complete interface definition
5. [Context Object](./api/context-object.md) — Understand the execution context
6. [Middleware](./guides/middleware.md) — Extend the execution flow
7. [ACL Configuration](./guides/acl-configuration.md) — Configure access control

After completing the above, you can read the advanced guides on testing, adapter development, multi-language development, etc. as needed.

## Concept Index

Quickly find authoritative definitions for concepts:

| Concept | Authoritative Definition | Quick Reference |
|---------|--------------------------|-----------------|
| APCore Client | [client-api.md](./api/client-api.md) | [Getting Started](./getting-started.md) |
| Module | [module-interface.md](./api/module-interface.md) | [README](overview.md#module-development) |
| ModuleAnnotations | [module-interface.md#annotations](./api/module-interface.md#34-annotations) | [README](overview.md#schema-system) |
| Context | [context-object.md](./api/context-object.md) | [README](overview.md#context-object) |
| Canonical ID | [PROTOCOL_SPEC.md §2](../PROTOCOL_SPEC.md#2-naming-specification) | [README](overview.md#directory-as-id) |
| Registry | [registry-api.md](./api/registry-api.md) | [README](overview.md#quick-start) |
| Executor | [executor-api.md](./api/executor-api.md) | [README](overview.md#quick-start) |
| ACL | [PROTOCOL_SPEC.md §6](../PROTOCOL_SPEC.md#6-acl-specification) | [README](overview.md#acl-access-control) |
| ApprovalHandler | [approval-system.md](./features/approval-system.md) | [PROTOCOL_SPEC.md §7](../PROTOCOL_SPEC.md#7-approval-system) |
| Middleware | [middleware.md](./guides/middleware.md) | [README](overview.md#middleware) |

For a single-page reference of all terminology see the [Glossary](./glossary.md).
