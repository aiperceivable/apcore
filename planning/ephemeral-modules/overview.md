# ephemeral-modules — Reserved namespace for runtime-registered modules

## Goal

Reserve `ephemeral.*` as a sanctioned namespace for programmatically-registered modules synthesized at runtime by LLM agents (à la ToolMaker, ACL 2025), with minimum-viable lifecycle, audit, and ACL guardrails. Add a new `discoverable: false` annotation so ephemeral modules can be hidden from enumeration surfaces (`Registry.list()`, MCP `tools/list`).

## Source

- Spec RFC: [`docs/spec/rfc-ephemeral-modules.md`](../../docs/spec/rfc-ephemeral-modules.md)
- Frontier-research alignment plan (Stage 3)

## Scope

- **In scope** (this feature):
  - Reserve `ephemeral.*` in §2.5 reserved namespaces
  - Add `discoverable: false` annotation to §4.4 ModuleAnnotations
  - Audit-event shape mirroring D-35 (contextual auditing for control plane)
  - Pilot implementation in `apcore-python` first
- **Out of scope** (deferred):
  - Sandboxing (host concern; see RFC §"Sandboxing — out of scope")
  - TTL / GC sweeper (open question; deferred to v2 if leakage observed)
  - Codegen pipeline (ToolMaker territory, not apcore's)

## Dependencies

- No blocking dependencies in this repo.
- Cross-repo: pilot in `apcore-python` precedes TS+Rust replication.
- Recommendation in RFC §"Open questions" #5: file a 1-line spec patch to reserve `ephemeral.*` immediately (separate from full RFC acceptance) to prevent third-party collision.

## Status

**Pending** — RFC created; awaiting acceptance review. Pilot in `apcore-python` queued.

## Adjacent literature

- ToolMaker (ACL 2025, arXiv 2502.11705 / KatherLab) — verified real
- LATM (LLM-ToolMaker)
- Dynamic Tool Generation survey topic
