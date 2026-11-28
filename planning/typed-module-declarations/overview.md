# Typed Module Declarations

This milestone follows the merged identity-delegation candidate. The Core baseline is `22d69599089c9053fd9110c8aab1a2ddef3c76d6` and the Surfaces baseline is `46f659f69396402314d5d5a14af481522fa78bcd`.

The architecture blueprint's declaration and Golden Path work covers a Python `BoundModule`, TypeScript `client.define`, and an optional Rust declaration macro with `TypedHandle` and `ModulePack`. Each declaration registers an ordinary protocol module so existing Core calls and MCP calls enter the same executor. Protocol wire forms, the Context constructor, and existing registration and invocation behavior remain supported.

The SDK work can proceed in parallel. Shared documentation and artifact-based acceptance depend on the public APIs agreed in [the plan](plan.md).
