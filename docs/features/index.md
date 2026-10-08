---
description: "Index of apcore feature specs by category: foundational protocols, execution and workflow, security and governance, reliability and ops, and framework internals; links each subsystem."
---

# Feature Specifications

> Implementation-ready feature specifications for apcore subsystems.

One page per apcore subsystem: what it does, the contract the SDKs implement, and Python / TypeScript / Rust usage. These pages are the reference for SDK users and implementers; the normative text is the [Protocol Specification](../spec/protocol-spec.md).

## Specification Categories

### Foundational Protocols
*Primary interfaces for defining and interacting with modules.*
- [Module Interface](./module-interface.md) — The fundamental contract of an apcore module.
- [Context Object](./context-object.md) — Per-invocation state and shared data.
- [APCore Client](./apcore-client.md) — The unified entry point for all SDK features.
- [Bindings (Decorator/YAML)](./decorator-bindings.md) — How code is mapped to the standard.
- [ID Normalization](./id-normalization.md) — Bare-name canonicalization and non-repairing module-ID conversion.

### Execution & Workflow
*The runtime behavior of the execution engine.*
- [Core Executor](./core-executor.md) — The 11-step pipeline mechanics.
- [Execution Pipeline](./execution-pipeline.md) — The Step protocol, strategy presets, and the pipeline engine.
- [Streaming Pipeline](./streaming.md) — Incremental output and chunk merging.
- [Async Task Management](./async-tasks.md) — Background execution and concurrency.
- [Cancellation Mechanism](./cancellation.md) — Cooperative cancellation via cancel tokens (no forced termination).

### Security & Governance
*Guardrails, identity, and access control.*
- [ACL System](./acl-system.md) — Pattern-based permission rules.
- [Identity System](./identity-system.md) — Caller representation and roles.
- [Approval System](./approval-system.md) — Human-in-the-loop gates.
- [Call Chain Guard](./call-chain-guard.md) — Recursion and depth protection.

### Reliability & Ops
*Observability, errors, and system-level introspection.*
- [Observability](./observability.md) — Tracing, spans, and exporters.
- [Metrics & Usage](./metrics-and-usage.md) — Call counters, latency histograms, and usage tracking.
- [Error History](./error-history.md) — Recent-error aggregation for introspection.
- [Redaction](./redaction.md) — Sensitive-key redaction in logs, traces, and events.
- [Error & AI Guidance](./error-system.md) — Self-healing error protocols.
- [Event System](./event-system.md) — Framework-wide async event bus.
- [System Modules (system.*)](./system-modules.md) — Built-in control plane modules.

### Framework Internals
*Deep-level infrastructure, primarily for SDK implementers.*
- [Registry System](./registry-system.md) — Discovery and module management.
- [Schema System](./schema-system.md) — JSON Schema processing and validation.
- [Extension Points](./extension-system.md) — The pluggable architecture.
- [Middleware System](./middleware-system.md) — The onion execution model.
- [Multi-Module Discovery](./multi-module-discovery.md) — Multi-class file scanning.
- [Config Bus](./config-bus.md) — Unified multi-package configuration.

For a high-level overview of how these features fit together, see the [Architecture Design](../architecture.md) or the [Core Concepts](../concepts.md).
