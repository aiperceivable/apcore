# Feature Specifications

> Implementation-ready feature specifications for apcore subsystems.

This directory contains detailed specifications for each subsystem of the apcore framework. These documents are intended for SDK implementers and contributors who need to understand the internal mechanics, requirements, and edge cases of specific features.

## Specification Categories

### Foundational Protocols
*Primary interfaces for defining and interacting with modules.*
- [Module Interface](./module-interface.md) — The fundamental contract of an apcore module.
- [Context Object](./context-object.md) — Per-invocation state and shared data.
- [APCore Client](./apcore-client.md) — The unified entry point for all SDK features.
- [Bindings (Decorator/YAML)](./decorator-bindings.md) — How code is mapped to the standard.

### Execution & Workflow
*The runtime behavior of the execution engine.*
- [Core Executor](./core-executor.md) — The 11-step pipeline mechanics.
- [Streaming Pipeline](./streaming.md) — Incremental output and chunk merging.
- [Async Task Management](./async-tasks.md) — Background execution and concurrency.
- [Cancellation Mechanism](./cancellation.md) — Cooperative and forced termination.
- [Config Bus](./config-bus.md) — Unified multi-package configuration.

### Security & Governance
*Guardrails, identity, and access control.*
- [ACL System](./acl-system.md) — Pattern-based permission rules.
- [Identity System](./identity-system.md) — Caller representation and roles.
- [Approval System](./approval-system.md) — Human-in-the-loop gates.
- [Call Chain Guard](./call-chain-guard.md) — Recursion and depth protection.

### Reliability & Ops
*Observability, errors, and system-level introspection.*
- [Observability](./observability.md) — Tracing, metrics, and logs.
- [Error & AI Guidance](./error-system.md) — Self-healing error protocols.
- [Event System](./event-system.md) — Framework-wide async event bus.
- [System Modules (sys.*)](./system-modules.md) — Built-in control plane modules.

### Framework Internals
*Deep-level infrastructure, primarily for SDK implementers.*
- [Registry System](./registry-system.md) — Discovery and module management.
- [Schema System](./schema-system.md) — JSON Schema processing and validation.
- [Extension Points](./extension-system.md) — The pluggable architecture.
- [Middleware System](./middleware-system.md) — The onion execution model.
- [Multi-Module Discovery](./multi-module-discovery.md) — Multi-class file scanning.


For a high-level overview of how these features fit together, see the [Architecture Design](../architecture.md) or the [Core Concepts](../concepts.md).
