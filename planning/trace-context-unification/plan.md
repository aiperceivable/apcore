# Trace Context Unification — Implementation Plan

## Phase 1: Proposal & Spec (blocking)

### Task 1: Draft spec patch + open maintainer issue
- File: [spec-patch.md](./spec-patch.md)
- Owner: core maintainer
- Deliverable: GitHub issue with linked proposal; 2 maintainer approvals before merging patch into `PROTOCOL_SPEC.md`
- Acceptance:
  - Issue references §5.7 and §10.5 line numbers
  - Proposed diff posted verbatim from `spec-patch.md`
  - Version bump decided (PATCH recommended)
  - CHANGELOG.md entry drafted

### Task 2: Cross-language conformance fixture
- File: `conformance/fixtures/context_trace_parent.json`
- Structure:
  ```json
  {
    "description": "Context.create trace_parent input handling — strict 32-hex, no auto-normalization",
    "test_cases": [
      {
        "id": "valid_32_hex",
        "input": { "trace_parent": "4bf92f3577b34da6a3ce929d0e0e4736" },
        "expected": { "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736", "regenerated": false, "warn_logged": false }
      },
      {
        "id": "invalid_dashed_uuid",
        "input": { "trace_parent": "550e8400-e29b-41d4-a716-446655440000" },
        "expected": { "regenerated": true, "warn_logged": true }
      },
      {
        "id": "invalid_uppercase",
        "input": { "trace_parent": "4BF92F3577B34DA6A3CE929D0E0E4736" },
        "expected": { "regenerated": true, "warn_logged": true }
      },
      {
        "id": "invalid_all_zero",
        "input": { "trace_parent": "00000000000000000000000000000000" },
        "expected": { "regenerated": true, "warn_logged": true }
      },
      {
        "id": "invalid_all_f",
        "input": { "trace_parent": "ffffffffffffffffffffffffffffffff" },
        "expected": { "regenerated": true, "warn_logged": true }
      },
      {
        "id": "invalid_too_short",
        "input": { "trace_parent": "deadbeef" },
        "expected": { "regenerated": true, "warn_logged": true }
      },
      {
        "id": "invalid_non_hex",
        "input": { "trace_parent": "req_abc123xyz_not_hex_at_all_nope" },
        "expected": { "regenerated": true, "warn_logged": true }
      },
      {
        "id": "invalid_empty",
        "input": { "trace_parent": "" },
        "expected": { "regenerated": true, "warn_logged": true }
      }
    ]
  }
  ```
- Add row to `conformance/README.md` fixtures table.

## Phase 2: SDK alignment (parallel, after Phase 1 merges)

### Task 3: Python — reject W3C-invalid values
- File: `apcore-python/src/apcore/context.py` lines 63-72
- Change: add `hex_id not in ("0" * 32, "f" * 32)` to the validity check alongside the existing `len==32 && all hex` test
- Tests: `apcore-python/tests/test_context_trace_parent.py` driving the shared fixture

### Task 4: TypeScript — add WARN log + invalid-value rejection
- File: `apcore-typescript/src/context.ts` lines 76-103
- Changes:
  - Reject all-zero and all-f
  - Emit `console.warn` (or wired logger) on regeneration
- Tests: `apcore-typescript/test/context.test.ts`

### Task 5: Rust — add trace_parent via builder + 32-hex output
- Files:
  - `apcore-rust/src/context.rs` lines 180-431
  - `apcore-rust/src/trace_context.rs` (verify `TraceParent::trace_id` accessor)
- Changes:
  - Add `Context::builder()` returning a `ContextBuilder` with `.trace_parent(TraceParent)`, `.identity(Identity)`, etc., terminating in `.build()`
  - Keep `Context::create(executor, identity)` as-is for backward compatibility, but have it internally delegate to the builder
  - Switch generated trace_id to `Uuid::new_v4().simple().to_string()` (32-char hex)
  - Reject all-zero and all-f; `tracing::warn!` on regeneration
  - `TraceContext::inject()` no longer needs to strip dashes (input is already 32-hex)
- Tests: `apcore-rust/tests/context_trace_parent.rs`

## Phase 3: Integration surfaces (parallel, after Phase 2)

### Task 6: User guide (drafted)
- File: `docs/guides/integrating-existing-projects.md` — drafted 2026-04-17
- Remaining work after spec merges: cross-link to the merged §5.7 Note and §10.5 `external_trace_parent_handling` rules using their final anchor IDs.
- mkdocs.yml nav + `docs/guides/README.md` Document List + Learning Path entries already added.

### Task 7: Framework default factories
- Repos:
  - `django-apcore`: `DjangoContextFactory` reads `request.META["HTTP_TRACEPARENT"]` and `HTTP_X_REQUEST_ID`
  - `flask-apcore`: `FlaskContextFactory` reads `request.headers["traceparent"]` and `X-Request-ID`
  - `nestjs-apcore`: `NestContextFactory` via `@Req()` and `req.headers`
- Each ships with a unit test hitting the same fixture semantics
- Each factory's responsibility: if the caller's legacy ID is a dashed UUID they want to reuse as trace_id, **the factory** strips dashes before passing to `Context.create(trace_parent=...)` — not Context.create itself

## Verification

- `mkdocs build` produces zero warnings
- All 3 SDKs pass `context_trace_parent.json` fixture (checked by `apcore-skills:tester`)
- CHANGELOG entries in all affected repos
- No remaining 36-char UUID format for `trace_id` anywhere in spec or SDKs
- PROTOCOL_SPEC.md diff matches `spec-patch.md` exactly

## Rollout order

1. Merge spec patch (maintainer review gate)
2. Land Python + TypeScript changes together (small diffs, near-identical behavior already)
3. Land Rust change (builder addition + format switch — coordinate with downstream Rust users via pre-announcement)
4. Ship framework integrations + polish user guide
