---
description: "Decision record for the specification/SDK divergences found by the 2026-09-30 documentation audit (D-129 onward): security fixes, spec corrections, and SDK alignment."
title: Divergence decisions (D-129 onward, 2026-09)
date: 2026-09-30
status: in progress — phase 1 (security) implemented in all three SDKs
audience: maintainers + spec reviewers
---

# Divergence decisions (D-129 onward)

> **Historical decision record.** Current behaviour is defined by [protocol-spec.md](./protocol-spec.md); decision status is tracked in [decision-register.md](./decision-register.md).

The 2026-09-30 rewrite of the documentation checked every page against the three SDKs and found places where the specification and the implementations disagree. Each was classified before anything changed:

- **Security** — a gate that can silently stop gating. Fixed first, in all three SDKs.
- **Spec error** — the three SDKs agree and are right; the specification is corrected.
- **SDK divergence** — the SDKs disagree; the correct behaviour is chosen and the minority SDKs change, pinned by a fixture.
- **Design question** — the SDKs agree on behaviour that is questionable; decided on its merits.

The order of precedence when deciding which side is right: security, then the specification's intent, then whether the rule can be implemented in all three languages, then consistency with existing implementations. Three SDKs agreeing is evidence, not proof.

## Phase 1 — security (spec v1.61.0)

### D-129 — Providers reach the gate they configure

**Problem.** An ACL, `ApprovalHandler` or `ExecutionPolicy` given to an executor was bound into the built-in gate steps only when the executor built the strategy itself. A pre-built strategy instance received none of them: with a deny-all ACL the call ran, while `governance_state()` reported `acl_configured` and `builtin_acl_gate_wired` as `true` — the combination §6.6.5.2 exists to make trustworthy. Reproduced in apcore-python and apcore-typescript. apcore-rust was unaffected: its gate steps hold no provider and read the executor's on every call.

**Decision.** A provider given to an executor, by constructor or setter, is bound into the corresponding built-in gate step of the running strategy however that strategy was supplied, and replaces the provider the step held; an executor given no provider leaves the step alone. Replacing the strategy rebinds. `governance_state()` reports what the running built-in gate holds. Spec §6.6.5.1, §6.6.5.5.

**Pinned by** `conformance/fixtures/gate_provider_binding.json`.

### D-130 — `configure` cannot weaken a governance gate

**Problem.** `pipeline.configure` accepted `ignore_errors: true` on `acl_check`, which turns an ACL denial into a warning and runs the call; `match_modules` on either gate exempted every module it did not match; `pure: true` on `approval_gate` would make `validate()` consult the `ApprovalHandler` during a dry run. Only `remove:` warned (§5.16 requirement 7), although removing a gate is visible in `governance_state()` and weakening one is not.

**Decision.** On `acl_check` and `approval_gate`, `ignore_errors: true` and any `match_modules` are rejected, and so is `pure: true` on `approval_gate` (`acl_check` is pure by default, since `validate()` runs it), with `PIPELINE_CONFIGURATION_ERROR`, from configuration and from programmatic step configuration alike. `timeout_ms` stays configurable; writing a default value is accepted. Spec §5.16.1.

**Pinned by** `conformance/fixtures/gate_step_configure.json`.

### D-131 — Built-in logging middleware logs the captured values

**Problem.** A built-in logging middleware that logs the raw `inputs` / `output` it is handed bypasses the `x-sensitive` rule: in apcore-rust, `ObsLoggingMiddleware` wrote a field marked `x-sensitive` in plain text unless a `RedactionConfig` happened to list it, apcore-python's `ObsLoggingMiddleware.after` logged the raw output, and in apcore-typescript the JSON-Schema-to-TypeBox conversion dropped every `x-` keyword, so `x-sensitive` was ignored at the capture point for any module declared with plain JSON Schema.

**Decision.** Built-in logging middleware logs `context.redacted_inputs` and `context.redacted_output`, never the raw values; an `x-sensitive` field never appears unredacted, with or without a redaction configuration. Spec §10.6.1 requirement 5.

**Pinned by** per-SDK tests (log formats differ by language).
