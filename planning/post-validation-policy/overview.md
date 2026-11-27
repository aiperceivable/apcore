# Post-validation Policy

## Overview

Gap 1 is the first milestone after the alpha.2 Core profile and fail-closed approval changes. This proposal evaluates policy on the arguments that the selected module will receive and uses one bound approval for those arguments when an evaluator is installed.

Core starts from `ea1c147` and Surfaces from `fc1a52c`. Both main-branch CI runs passed. The maintainer approved the opted-in gate and protocol draft revision 2.

Targeted approval baselines passed on the merged Core tree. Python passed 129 cases, TypeScript passed 86, and Rust passed 112 with two existing ignored cases. A Python public-entry probe confirmed the design conflict. Step 5 saw the submitted value `10` and rejected a test credential scoped to the value `20` before middleware could normalize the input.

## Scope

- Core evaluator registration, Step 7.5 execution boundary, and invocation-scoped approval.
- Python, TypeScript, and Rust parity across execution, preflight, and streaming.
- MCP scoped approval and installed-package compatibility. Check other Surfaces consumers where the approved contract affects their builds or behavior.

Gap 3 discovery, delegated identity, new budget and interaction primitives, media references, and SDK declaration ergonomics remain separate milestones. Release is paused.

## Technology Stack

Python with pytest, TypeScript with Vitest and pnpm, Rust with Cargo and Tokio, and the shared JSON fixture suite. Reuse the existing validators, execution strategies, scoped approval stores, and package CI.

## Task Execution Order

| Phase | Task | Scope | Status |
|---|---|---|---|
| 1 | [Shared contract](./tasks/shared-contract.md) | Maintainer decision, draft, and conformance expectations | Complete |
| 2 | [Python](./tasks/python.md) | Python SDK implementation and tests | Complete |
| 2 | [TypeScript](./tasks/typescript.md) | Portable and full SDK implementations and tests | Complete |
| 2 | [Rust](./tasks/rust.md) | Default and full SDK implementations and tests | Complete |
| 3 | [Surfaces](./tasks/surfaces.md) | Scoped approval storage and MCP transport verification | In progress |
| 4 | [Integration](./tasks/integration.md) | Candidate packages, cross-language tests, and compatibility review | In progress |

## Progress

All three SDKs pass the shared fixture. Cross-repository verification uses built Core distributions.

## Reference Documents

- [Implementation plan](./plan.md)
- Atlas `apcore_architecture_evolution_blueprint.md`, Gap 1
- [Protocol specification](../../docs/spec/protocol-spec.md)
