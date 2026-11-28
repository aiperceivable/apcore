# Verification

Run the three SDK regression suites, documentation checks and installed-package examples. For each supported declaration, exercise the same module by typed call and MCP. Check a compiled pack through MCP and empty directory backends through both Rust binaries. Record tested candidates and any compatibility boundaries.

Local validation on 2026-11-28 used the Core alpha.6 wheel, npm tarball, and
two Rust crates. Python passed 6,748 SDK tests. TypeScript passed 5,955 SDK
tests with 90.53% branch coverage. Rust 1.86 passed 3,271 full workspace tests
and 522 default-profile tests. Strict MkDocs and the installed typed examples
passed.

The installed candidate passed the Toolkit, MCP, A2A, and CLI test suites in
all three languages. The Python Golden Path exercised approval resumption and
an ACL denial over a real HTTP MCP session. TypeScript and Rust verified
direct and MCP results, invalid input, policy denial, and final approval
arguments. The Rust binary tests checked stderr diagnostics for an empty
directory backend. The Python Agents SDK and TypeScript AI SDK bridges ran
local tool calls using the governed handles.

The candidate workflow stages dependency overrides for compatibility tests.
GitHub CI and review follow when the candidate branches are opened.

PR review added regression coverage for native Python inputs and replaced
TypeScript registrations. Python passed 6,750 tests, including rejection of
ambiguous native argument projections before hosted policy evaluates.
TypeScript passed 5,959 tests, including replacement before lookup, before
dispatch, and during preflight. Ordinary ID-based calls still resolve the
current registration.
