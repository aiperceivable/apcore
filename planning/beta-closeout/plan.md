# Beta Closeout Implementation Plan

## Core acceptance

- Keep the six-argument Context constructor. Budget and interaction attachments
  live in process, follow child contexts, and stay out of serialized context.
- Verify that the stock call-chain guard reserves a slot atomically. Denial,
  validation failure, cancellation before dispatch, and a pending approval
  release that slot. Business dispatch commits the slot and the estimated cost.
  Each dispatched retry is charged separately. Preflight leaves the ledger intact.
- Settle each active invocation's cost once with the observed amount. Charge
  amounts even when they exceed the limit and prevent later spending over cap.
- Verify live progress and elicitation with configured callbacks, cancellation,
  deadline, and absent handlers. Keep the existing approval lifecycle.
- Bound regular-expression input to 65,536 UTF-8 bytes by default. With an
  active regex rule, redact string values above the cap before matching.

## Surface pilot

- Start with opt-in artifact links on MCP tool results. Use a top-level output
  property marked `x-mcp-artifact` for an opaque artifact ID and retain the
  existing text and structured results. Read binary content through a guarded
  resource lookup.
- Verify owner and current authority, expiry, size, and an actual MCP client.
  The adapter accepts opaque IDs and in-process data from a trusted host.
- Assess the A2A mapping using the same access and expiry guarantees before
  proposing a common Core artifact contract.

## Delivery

Python, TypeScript, and Rust SDK changes run in parallel. Combine their evidence
with the Surface integration tests, installed package checks, and the existing
candidate workflow. Changes to normative protocol text require maintainer
approval under AGENTS.md.
