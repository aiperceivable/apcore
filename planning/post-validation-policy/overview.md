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
| 3 | [Surfaces](./tasks/surfaces.md) | Scoped approval storage and MCP transport verification | Complete |
| 4 | [Integration](./tasks/integration.md) | Candidate packages, cross-language tests, and compatibility review | Complete |

## Progress

Core implementation commit `83f627cb300a42b5c46550a04b48d18745147f21` and Surfaces commit `86a6f22bfa82a54a3f105ec434ab880687155b4f` passed the local verification recorded below. The Core artifact builds and repository-level integration are separate checks.

## Verification results

| Gate | Python | TypeScript | Rust |
|---|---:|---:|---:|
| Core full test suite | 6,674 passed | 5,883 passed | 3,245 passed |
| Toolkit with built Core | 1,289 passed | 1,079 passed | 714 passed |
| MCP with built Core | 1,278 passed | 996 passed | 1,153 passed |
| A2A with built Core | 522 passed | 492 passed | 242 passed |
| CLI with built Core | 1,098 passed | 950 passed | 1,052 passed |
| Toolkit with the stable minimum Core | 1,289 passed | 1,079 passed | 714 passed |

All three SDKs run the nine shared post-validation policy cases. The conformance guards find 99 fixtures and 1,238 cases, with every fixture driven by each SDK. The packaged Python minimal and full profiles passed isolated installation checks. TypeScript passed its portable entry and browser import checks. Rust passed the minimum-toolchain profile and verified the packaged crate on that toolchain.

MCP verified final-scope pending approval, polling, and one-shot resumption through real call and streaming entry points in each language. The direct and MCP Golden Path also passed with installed Core and MCP wheels. Core and MCP MkDocs strict builds passed. Artifact digests agree with the Surfaces locks and compatibility workflow pins.

| Core distribution used for integration | SHA-256 |
|---|---|
| Python wheel | `02610c5a0d3a5519ba27914edd9b0e63da30c9b4985b31efaa43d5e8609092b9` |
| TypeScript npm tarball | `abce533e664afec207c4ff93d3af7475789e7144ac48db154af6efd08bbe3e07` |
| Rust crate | `7514b549b91d7beb83a0fefef9d4f365e4cb19b38f7a0664c4a814bcac4009bc` |

## Reference Documents

- [Implementation plan](./plan.md)
- Atlas `apcore_architecture_evolution_blueprint.md`, Gap 1
- [Protocol specification](../../docs/spec/protocol-spec.md)
