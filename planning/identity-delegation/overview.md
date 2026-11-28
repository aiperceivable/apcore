# Identity Delegation

Gap 2 follows the merged capability discovery milestone. The Core base is `173db455`, and the Surfaces base is `d2bac9be`. The local candidate branches are `core-alpha5` and `surfaces-alpha5`.

The proposed contract covers trusted delegation in Core, scoped authorization and audit, and opt-in mapping of verified JWT claims in MCP and A2A. Implementation of the new protocol behavior follows maintainer approval of the proposed semantics. The proposal advances the draft revision on the current protocol version line.

## Baseline verification

| Existing behavior | Python | TypeScript | Rust |
| --- | ---: | ---: | ---: |
| Core identity and context | 272 passed | 108 passed | 44 passed |
| MCP Python authentication | 72 passed | | |
| A2A Python authentication | 35 passed | | |

The Python and TypeScript Core runs include ACL tests. The Rust aggregate run selects existing identity cases.

## Implementation order

1. Approve the [proposed contract](./plan.md) and define the [shared conformance cases](./tasks/shared-contract.md).
2. Implement the [Core SDK changes](./tasks/core-sdks.md) independently in Python, TypeScript, and Rust.
3. Apply the same authority contract to the [MCP and A2A adapters](./tasks/surfaces.md).
4. Run the [dual-repository verification](./tasks/verification.md) against fixed source and package candidates.

## Local candidate verification

The Core distribution source is `6a4aa25b93526f09ca698aa9ffb982f94e613158` on `core-alpha5`. The verified Surfaces source is `417116190fe2815c0204ffacd70744cc55f43f3c` on `surfaces-alpha5`. The Core distribution files used for the Surfaces runs have the following SHA-256 digests.

| Distribution | SHA-256 |
| --- | --- |
| Python wheel | `4e14ff5854f3024ebbc63d4fd7d7e73f6e6a5255291f3b3f0bd9372c5112c7d5` |
| npm archive, produced with the CI pnpm version | `a3c57f10ead7cae35c6d6b22ee2013cc87e7e1281fa820234827e6c2e83679cc` |
| Rust crate, produced with Rust 1.86 | `9471bdd15aa13ec971ba940414f486b5e40b62426d73827b63918bd6b88d5cb9` |

The table counts passing tests, including Rust doctests.

| Verification | Python | TypeScript | Rust |
| --- | ---: | ---: | ---: |
| Core full suite | 6,732 | 5,945 | 3,265 |
| Toolkit with the candidate Core | 1,289 | 1,079 | 714 |
| MCP with the candidate Core | 1,303 | 1,002 | 1,164 |
| A2A with the candidate Core | 539 | 495 | 247 |
| CLI with the candidate Core | 1,098 | 950 | 1,052 |
| Toolkit with its minimum stable Core | 1,289 | 1,079 | 714 |

All three SDKs exercise the 17 shared delegation cases. The fixture drivers and expected-key guards cover 101 JSON fixtures and 1,269 cases. Python Ruff and Pyright, TypeScript type checks with 90.46% Core branch coverage, and strict Rust Clippy passed. The installed minimal Python wheel ran Quick Start, the installed npm archive ran the browser entry on Node 20, and Rust passed the 23-test Core Profile gate on Rust 1.86. Core and the four Surface documentation sites passed their configured strict builds.

Signed MCP requests exercise current actor and root subject isolation, authority-bound session reuse, durable task disclosure, and atomic approval consumption. The Python SSE smoke test rejected another subject and narrower scopes, then accepted equivalent refreshed credentials. A2A task and push operations enforce current scope, including against Rust task records after restarting the server. The Python Golden Path passed through direct calls and an MCP client with both wheels installed.

The A2A authentication guide records the owner-key migration for existing IDs beginning with `apcore:`. The candidate is at the local verification stage.
