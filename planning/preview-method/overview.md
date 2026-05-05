# preview-method — Module-level structured-diff preview

## Goal

Add an optional `Module.preview(inputs, context) -> PreviewResult | None` method and extend `PreflightResult` with `predicted_changes: list[Change]`, so destructive modules can self-report what would change if executed.

## Source

- Spec RFC: [`docs/spec/rfc-preview-method.md`](../../docs/spec/rfc-preview-method.md)
- Decision log: D-57 (companion `x-supports-dry-run` convention) — see `docs/spec/2026-05-decision-log.md`
- Frontier-research alignment plan (Stage 2)

## Scope

- **In scope** (this feature):
  - Optional `Module.preview()` method specification
  - New `PreviewResult` and `Change` types
  - `Executor.validate()` integration to fold preview output into `PreflightResult.predicted_changes`
  - Cross-language conformance fixture
- **Out of scope** (deferred):
  - Per-module-class typed `Change` schemas (potential §4.6 follow-up)
  - Sandboxing or runtime enforcement (host concern)
  - Replacing or deprecating existing `preflight()` warnings

## Dependencies

- **Blocking pre-req (cross-repo)**: `apcore-rust` must mark `PreflightResult` and `PreflightCheckResult` `#[non_exhaustive]` before adding the new field. Open as tracking issue against `apcore-rust`.
- No dependencies on other planning features in this repo.

## Status

**Pending** — RFC created; awaiting acceptance review. No spec normative text added until RFC is accepted.

## Adjacent literature

- AgentSpec (ICSE 2026, arXiv 2503.18666)
- AgentHarm safety benchmark
- Superego agent architecture
