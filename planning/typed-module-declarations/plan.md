# Typed Module Declarations Implementation Plan

## Shared acceptance

Every typed handle resolves its registered ID through the public client. Verify normal execution, denial before business execution, required approval, and validation through normal Core calls and an MCP client. Any SDK-owned conversion of arguments that changes the values sent to a module must happen before post-validation policy and final-scope approval.

## SDK declarations

- Python returns a `ParamSpec`-preserving `BoundModule` from `APCore.module`, with governed sync and async invocation, validation and preview, and a raw testing accessor. Existing direct decorated calls retain their callable behavior. Declaration shortcuts use the existing annotation representation. Preserve native Pydantic values through the existing typed function adapter and decode the governed result.
- TypeScript adds `APCore.define` for Standard Schema v1. Keep schema library adapters structural. Obtain the protocol's Draft 2020-12 JSON Schema through Standard JSON Schema where available, with explicit schema input for validators that supply validation only. Validate normalized values through the governed runtime and preserve the input/output type distinction.
- Rust adds an optional `macros` feature for a sibling `apcore-macros` crate. Generate existing `Module` implementations from typed functions, Doc comments and Draft 2020-12 schemas. Typed handles call the executor and resolve through the registry. A static pack assembles declarations explicitly without global discovery and can be exercised by MCP and CLI consumers.

## Integration

Validate no extra runtime dependency in the minimal Python and TypeScript package profiles. In Rust, verify the default feature set and test the optional macro installation from local candidate archives. Use Core and Surfaces source to exercise declaration-driven tool calls and Rust empty-discovery startup behavior. Publish examples through existing documentation and the established Golden Path workflow.
