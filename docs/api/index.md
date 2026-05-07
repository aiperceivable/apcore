# API Reference

> API reference for apcore core interfaces.

This directory contains the complete definitions of the four core interfaces of the apcore module system. Each interface has a clear responsibility boundary; together they form the definition, registration, discovery, and execution system for modules.

## Document List

| Document | Description |
|------|------|
| [module-interface.md](./module-interface.md) | Module base class and interface definitions—the core contract every module must implement |
| [context-object.md](./context-object.md) | Execution context `Context` object, carrying trace info, call chain, and shared state |
| [registry-api.md](./registry-api.md) | Registry service, responsible for module registration, discovery, and management |
| [executor-api.md](./executor-api.md) | Executor, responsible for module execution, middleware orchestration, and observability |

## Recommended Reading Order

1. **Module Interface** — Understand the basic module structure and required methods
2. **Context Object** — Learn what context information is available during execution
3. **Registry API** — Learn how modules are registered and discovered
4. **Executor API** — Understand execution flow and middleware orchestration

If you are not yet familiar with the core concepts of apcore, read [Core Concepts](../concepts.md) first.
