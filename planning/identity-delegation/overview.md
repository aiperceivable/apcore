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
