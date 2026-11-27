# ACL-aware Capability Discovery

Gap 3 follows the merged post-validation policy milestone. The Core base is `0daf50a3`; the Surfaces base is `04e1c698`. The local candidate branches are `core-alpha4` and `surfaces-alpha4`.

The milestone lets a trusted caller discover public modules that it is authorized to know about. Core owns the visibility decision, summaries, expansion, and pagination. MCP applies the same visibility decision to the modules it already exports and preserves complete `inputSchema` values on standard `tools/list`.

The [implementation plan](./plan.md) records the accepted design. The protocol specification records draft revision 3 on the current protocol version line.

The merged baseline passed targeted ACL and system manifest checks in all three SDKs. Python passed 349 tests with 3 skips, TypeScript passed 71 tests, and Rust passed 326 tests with 1 ignored test. These runs verify the existing invocation and manifest behavior that discovery must preserve.

## Acceptance

- Shared cases and SDK-specific regressions run through Python, TypeScript, and Rust. They cover hidden and inactive modules, caller-sensitive ACL, deferred decisions, approval hints, and cursor isolation.
- MCP client tests compare listing and execution for different authenticated identities and verify that listed tools retain the existing full input schema.
- Core Profile and full SDK checks pass. Surfaces verify against the candidate wheel, npm tarball, and Rust crate and preserve declared stable minimum compatibility where applicable.

## Tasks

1. [Shared contract and conformance](./tasks/shared-contract.md)
2. [Core SDKs](./tasks/core-sdks.md)
3. [MCP surfaces](./tasks/surfaces.md)
4. [Dual-repository verification](./tasks/verification.md)

## Verification results

The candidate Core distributions were built from `de0c797760796daeefc1ba70d1e44263ea6519f1` and tested against Surfaces commit `a6ac98c61f5ff44e9ae92e5056d218161f9d9e61`. The local candidate branches retain the corresponding source and lockfiles.

| Gate | Python | TypeScript | Rust |
| --- | ---: | ---: | ---: |
| Core full suite | 6,706 passed | 5,907 passed | 3,254 passed |
| Toolkit with installed Core | 1,289 passed | 1,079 passed | 714 passed |
| MCP with installed Core | 1,281 passed | 997 passed | 1,155 passed |
| A2A with installed Core | 522 passed | 492 passed | 242 passed |
| CLI with installed Core | 1,098 passed | 950 passed | 1,052 passed |
| Toolkit with stable minimum Core | 1,289 passed | 1,079 passed | 714 passed |

All three SDKs drive the 14 shared discovery cases. The repository inventory validates 100 fixtures and 1,252 cases. Rust 1.86 passed the 18-case minimal Core Profile. The isolated Python wheel and TypeScript browser-entry checks confirm discovery is available from the minimal package. Core and MCP strict documentation builds pass.

Real MCP client tests cover authenticated role isolation, complete input schemas, document listing and direct reads, actual invocation enforcement, and redaction after a same-version replacement. Python's direct and MCP Golden Path passed with both wheels installed.

| Core candidate artifact | SHA-256 |
| --- | --- |
| Python wheel | `a016417e8260d4d256a362415361a6afb7cfad8871a1f71abecda48e008814e1` |
| Python source distribution | `d6aad0217860162de84451f2f6ad369c4cb6f3804a8e49929839e868717af64e` |
| npm tarball | `1acc4775d2ec6c8006f5aca264d16d4c2fa9327f5f3ee7af28984109c41d7ed1` |
| Rust crate | `412082f3f58f5a59165f20e1df00f7191b5e382ea59f2369eb486f963cb7d604` |
