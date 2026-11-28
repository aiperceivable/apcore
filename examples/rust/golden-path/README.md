# Rust Golden Path

`verify.rs` registers a compiled `ModulePack` and shares its executor with the
Rust MCP adapter. The integration test checks tool discovery, direct and
protocol execution, input validation, and the canonical arguments reviewed
by final approval.

Use the Surface repository's candidate verifier with both Core crate archives.
It stages the test in the MCP crate, enables the macro feature for that run,
and checks that Cargo resolved the unpacked artifacts.

```sh
cd "$SURFACES_CHECKOUT"
python3 scripts/verify-rust-core-candidate.py mcp "$APCORE_VERSION" \
  --archive "$APCORE_CRATE" \
  --macro-archive "$APCORE_MACRO_CRATE" \
  --typed-example "$APCORE_CHECKOUT/examples/rust/golden-path/verify.rs"
```
