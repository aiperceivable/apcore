# Dual-repository Verification

Verify all three SDKs against the shared cases, the Core Profile, and full checks. Confirm that syntax validation, authority proof, denial behavior, cursor isolation, approval identity and recursive audit redaction have passing runtime assertions. Build the wheel, npm tarball and Rust crate from one Core candidate and record checksums.

Install the Core candidates into Toolkit, MCP, A2A and CLI in all three SDK ecosystems. Run real protocol client tests for both MCP and A2A with verified credentials and a before-and-after scope reduction on the same actor. Verify the existing minimum Toolkit compatibility and the direct and MCP Golden Path. Record both repository revisions and the installed package identities.
