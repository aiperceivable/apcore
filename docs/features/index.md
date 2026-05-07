# Feature Specifications

> Implementation-ready feature specifications for apcore subsystems.

This directory contains detailed specifications for each subsystem of the apcore framework. These documents are intended for SDK implementers and contributors who need to understand the internal mechanics, requirements, and edge cases of specific features.

## Specification List

| Feature | Description |
|---------|-------------|
| [Module Interface](./module-interface.md) | Module Protocol contract: required schema attributes, lifecycle hooks, optional methods, and the function-based form |
| [Context Object](./context-object.md) | Per-invocation state object: trace, identity, call chain, executor, redaction, shared `data` map |
| [APCore Client](./apcore-client.md) | Unified high-level client managing Registry, Executor, and subsystems |
| [ACL System](./acl-system.md) | Pattern-based Access Control List with first-match-wins evaluation |
| [Approval System](./approval-system.md) | Runtime enforcement of `requires_approval` via pluggable ApprovalHandler |
| [Async Task Management](./async-tasks.md) | Background module execution with concurrency limiting and task lifecycle |
| [Call Chain Guard](./call-chain-guard.md) | Depth limiting, circular detection, and frequency throttling |
| [Cancellation](./cancellation.md) | Cooperative cancellation via CancelToken with executor timeout integration |
| [Config Bus](./config-bus.md) | Unified multi-package configuration with per-namespace env overrides |
| [Core Executor](./core-executor.md) | Central execution engine with a secured execution lifecycle |
| [Decorator & YAML Bindings](./decorator-bindings.md) | `@module` decorator and YAML-based declarative module creation |
| [Error System](./error-system.md) | Structured error hierarchy with AI guidance fields and error code registry |
| [Event System](./event-system.md) | Global event bus, subscribers, and threshold alerting |
| [Extension System](./extension-system.md) | Pluggable extension points for discoverers, middleware, ACL, exporters |
| [Identity System](./identity-system.md) | Caller identity with types, roles, and ContextFactory protocol |
| [Middleware System](./middleware-system.md) | Composable middleware pipeline with onion execution model |
| [Multi-Module Discovery](./multi-module-discovery.md) | Opt-in multi-class discovery: multiple Module classes per file |
| [Observability](./observability.md) | Distributed tracing, metrics collection, and structured logging |
| [Registry System](./registry-system.md) | Module discovery, registration, and querying |
| [Schema System](./schema-system.md) | Schema loading, validation, `$ref` resolution, and LLM export |
| [Streaming](./streaming.md) | Three-phase streaming pipeline with deep merge accumulation |
| [System Modules](./system-modules.md) | Built-in `system.*` modules for AI bidirectional introspection |

For a high-level overview of how these features fit together, see the [Architecture Design](../architecture.md) or the [Core Concepts](../concepts.md).
