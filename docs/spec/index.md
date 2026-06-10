---
description: "Landing page for the apcore specification set for SDK implementers — protocol spec, algorithms, type mapping, conformance, and declarative config — with a recommended reading order."
---

# Framework Specification

> The official technical specification of apcore, intended for SDK implementers.

This directory contains the official specification documents for the apcore standard. These documents define the algorithms, type mappings, and conformance requirements that all language SDK implementations must follow. The specification documents use RFC 2119 keywords (MUST, SHOULD, MAY, etc.) to distinguish the level of obligation for each requirement.

## Document List

| Document | Description |
|----------|-------------|
| [protocol-spec.md](./protocol-spec.md) | Canonical Protocol Specification (RFC 2119 conformant), the single source of truth for all SDKs |
| [algorithms.md](./algorithms.md) | Core algorithm reference, a unified index summarizing 17+ pseudocode algorithms |
| [type-mapping.md](./type-mapping.md) | Cross-language type mapping, defining the standard mapping from JSON Schema types to native types in each language |
| [conformance.md](./conformance.md) | Conformance definitions, specifying implementation conformance levels, test suite requirements, and declaration specifications |
| [DECLARATIVE_CONFIG_SPEC.md](./DECLARATIVE_CONFIG_SPEC.md) | Declarative configuration specification (bindings / pipeline / entry-point YAML). Defines cross-SDK YAML syntax and per-SDK resolution semantics. Introduced in 0.19.0. |

## Recommended Reading Order

1. **Canonical Protocol Spec** -- Read the full standard to understand the core concepts and requirements
2. **Conformance Definitions** -- Understand the conformance levels and clarify implementation goals
3. **Type Mapping** -- Master the type correspondences for the target language
4. **Algorithm Reference** -- Implement core algorithms one by one

If you are a module developer rather than an SDK implementer, these documents are for reference only. For day-to-day development, please refer to the [Usage Guides](../guides/index.md).
