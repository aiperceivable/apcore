# Dual-repository Verification

Run shared conformance inventory, minimal and full Core tests, language-specific checks, and strict documentation builds. Build the wheel, npm tarball, and Rust crate from one fixed Core candidate commit. Verify their digests before installing them into Surfaces.

Run the installed-candidate Toolkit, MCP, A2A, and CLI test matrix, including Toolkit with its declared stable minimum. Run the real MCP discovery and invocation tests, plus the existing direct and MCP Golden Path. Verify that the active candidate and expected Core implementation are actually installed in every matrix job.

Record Core and Surfaces revisions, per-language results, and artifact digests. This milestone ends at reviewed candidates and verification. Release and merge authorization are separate.
