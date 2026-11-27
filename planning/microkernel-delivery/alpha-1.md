# APCore alpha.1 delivery record

The last stable release on the old line is `0.33.0`. The next public Core candidate is `1.0.0-alpha.1`. The Python distribution uses the PEP 440 spelling `1.0.0a1`. Each affected Surfaces package uses its own prerelease version.

## Scope

Core includes the C1 execution and governance fixes, C2 streaming completion, and the managed lifecycle already merged at `316cb05ce1d30931586a12e9296751690782bc7a`. The release changes package metadata and adds the Python Golden Path. The import surface, default governance, and configuration behavior follow the current SDKs until the microkernel work in the next alpha.

Surfaces starts from `a2b4a3af7d58cf1095569fbea9434f48270668e7` and the candidate work for MCP cancellation and completion, A2A deadlines and cleanup, CLI strict streaming and structured arguments, registration integrity, and the M1 and M2 migration paths.

## Core rehearsal

| Local artifact | SHA-256 |
|---|---|
| `apcore-1.0.0a1-py3-none-any.whl` | `a674d4bfd25790a928e67a8960604205dc676f4f1f70522012b2a5ebdfe96e9c` |
| `apcore-1.0.0a1.tar.gz` | `e875f3c03e6aad8c2d6fdd99dcd818731b38968ac2ccc6e2e4c4a4e0817447fc` |
| `apcore-js-1.0.0-alpha.1.tgz` | `fe86feb59e4f8ce9686217c1d9bbd80b1c12360b99ca58107000473a0c0890a5` |
| `apcore-1.0.0-alpha.1.crate` | `a98f01a31ebdd737c9cadb46b8a4e2a44d0635a0a3a305063ef6fa7eba2c87e9` |

Full local suites passed 6,703 Python, 5,910 TypeScript, and 3,286 Rust tests. The fixture integrity checks covered 98 shared fixtures and 643 expected fields. The installed wheel and npm tarball and unpacked crate passed the Quick Start and governed-stream examples and lifecycle checks. The wheel was exercised on Python 3.11.18 and the npm archive on Node 20.0.0. The local Rust rehearsal used Rust 1.100.0. The distribution workflow covers the declared Rust floor.

The [Golden Path](../../examples/python/golden-path/README.md) passed after installing `apcore==1.0.0a1` and `apcore-mcp==0.23.5a1` from their local wheels in an isolated environment. It uses a real MCP session and verifies schema rejection, ACL denial, approval suspension and resumption, one execution per authorized call, and ACL audit evidence.

## Public release gates

- Record the final source commits, workflow runs, and SHA-256 digests of the exact build artifacts. Verify package contents and the declared runtime versions from clean checkouts.
- Against the selected Core artifacts, run the Surfaces matrix for each component's declared minimum and the alpha candidate. Verify Rust MCP and CLI installations using registry resolution and confirm the resolved Core identity.
- After release approval, publish Core first and then the affected Surfaces packages in dependency order. Verify the same behavior from public package installs and record the resolved versions and checksums. Keep the prior stable line available for consumers that have not adopted an alpha.
- Use the Golden Path with the first Python pilot and record setup, governance outcomes, and adoption friction separately from build acceptance.
