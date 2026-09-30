---
description: "Index of the apcore specification set for SDK implementers: normative spec, reference material, security considerations, open RFCs and decision records, each with its status."
---

# Framework Specification

> The technical specification of apcore, written for SDK implementers.

[protocol-spec.md](./protocol-spec.md) is the single source of truth. It uses RFC 2119 keywords (MUST, SHOULD, MAY) to mark the level of obligation for each requirement; every other page here either supports it or records how it came to say what it says.

**Status legend:** **Normative** — binding on every SDK · **Reference** — derived from or explaining the spec; the spec wins on conflict · **Informative** — guidance, no requirements · **Proposed** — not adopted · **Historical** — a record of past decisions.

## Normative

| Document | Status | Description |
|---|---|---|
| [Protocol Specification](./protocol-spec.md) | Normative | The canonical protocol: module IDs, schemas, the execution pipeline, ACL, approval, errors, configuration, observability. |
| [Conformance](./conformance.md) | Normative | Conformance levels, the fixture-based conformance suite, and how an implementation declares conformance. |
| [Type Mapping](./type-mapping.md) | Normative | JSON Schema types mapped to Python, TypeScript and Rust native types. |
| [API Surface Conventions](./api-surface-conventions.md) | Normative | When a symbol is public API, and how each SDK names and scopes what it exports. |

## Reference

| Document | Status | Description |
|---|---|---|
| [Algorithm Reference](./algorithms.md) | Reference | The 25 pseudocode algorithms (A01–A25) collected in one place, derived from the protocol spec. |
| [Execution Pipeline](../features/execution-pipeline.md) | Reference | The step protocol, execution strategies, built-in steps and the engine loop behind `Executor.call()`. |
| [Durability Boundary](./design-durability-boundary.md) | Reference | Which hooks apcore guarantees to retry, replay and workflow layers, and which durability concerns it leaves to them. |

## Security

| Document | Status | Description |
|---|---|---|
| [Security Considerations](./security-considerations.md) | Informative | Threat model, mitigations, residual risks and production guidance. |

## RFC

| Document | Status | Description |
|---|---|---|
| [`include:` Configuration Composition](./rfc-config-include.md) | Proposed | Composing `apcore.yaml` from YAML fragments. Awaiting a maintainer decision; not implemented. |

## Decision Records

Decision records are history, not guidance; protocol-spec wins on any conflict.

| Document | Status | Description |
|---|---|---|
| [Decision Register](./decision-register.md) | Reference | Every decision ID with its status, spec version and record. Start here. |
| [Cross-language alignment decisions (D-01 – D-65)](./2026-05-decision-log.md) | Historical | The 2026-05 sync run. |
| [Consistency audit decisions (E-01 – E-03)](./2026-09-decision-log.md) | Historical | The 2026-09-05 consistency audit. |
| [Configuration surface decisions (D-66 – D-73)](./2026-09-config-surface-decisions.md) | Historical | The apcore#118 configuration-key audit. |
| [Deep-chain alignment decisions (D-74 – D-128)](./2026-09-deep-chain-decisions.md) | Historical | The 2026-09 deep-chain audit, spec v1.49.0–v1.59.0. |

## Recommended reading order

1. **Protocol Specification** — the core concepts and requirements.
2. **Conformance** — the level you are implementing and how it is tested.
3. **Type Mapping** — the type correspondences for your language.
4. **Algorithm Reference** and **Execution Pipeline** — implement the core algorithms and the pipeline.

If you build modules rather than an SDK, start with the [Usage Guides](../guides/index.md) instead.
