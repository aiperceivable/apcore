# ACL-aware Capability Discovery

Gap 3 follows the merged post-validation policy milestone. The Core base is `0daf50a3`; the Surfaces base is `04e1c698`. The local candidate branches are `core-alpha4` and `surfaces-alpha4`.

The milestone lets a trusted caller discover public modules that it is authorized to know about. Core owns the visibility decision, summaries, expansion, and pagination. MCP applies the same visibility decision to the modules it already exports and preserves complete `inputSchema` values on standard `tools/list`.

The [implementation plan](./plan.md) records the proposed contract. The maintainers review and approve changes to the normative protocol specification before it is edited. The target is draft revision 3 on the current protocol version line.

The merged baseline passed targeted ACL and system manifest checks in all three SDKs. Python passed 349 tests with 3 skips, TypeScript passed 71 tests, and Rust passed 326 tests with 1 ignored test. These runs verify the existing invocation and manifest behavior that discovery must preserve.

## Acceptance

- The same shared cases run through Python, TypeScript, and Rust. They cover hidden and inactive modules, caller-sensitive ACL, deferred decisions, approval hints, and cursor isolation.
- MCP client tests compare listing and execution for different authenticated identities and verify that listed tools retain the existing full input schema.
- Core Profile and full SDK checks pass. Surfaces verify against the candidate wheel, npm tarball, and Rust crate and preserve declared stable minimum compatibility where applicable.

## Tasks

1. [Shared contract and conformance](./tasks/shared-contract.md)
2. [Core SDKs](./tasks/core-sdks.md)
3. [MCP surfaces](./tasks/surfaces.md)
4. [Dual-repository verification](./tasks/verification.md)
