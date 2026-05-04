# `validate()` Step Count — PROTOCOL_SPEC Patch Draft

## Status
DRAFT — needs linked issue + 2 maintainer reviews per CLAUDE.md "Specification Integrity" before merging into PROTOCOL_SPEC.md.

## Problem
PROTOCOL_SPEC §5.1 (line ~6462) describes `Executor.validate()` as running "**Steps 1–6** of the execution pipeline" with the enumerated steps being:

> "context creation, safety checks, module lookup, ACL enforcement, approval detection (report only, MUST NOT invoke ApprovalHandler), and input schema validation."

This was correct **before** the v0.18 step swap (CHANGELOG.md line 259, 0.20.0 release):

> **PROTOCOL_SPEC.md pipeline order — Steps 6/7 swapped to match all SDK implementations (`middleware_before` → `input_validation`).**

After the swap, the canonical 11-step pipeline is:

1. Context Creation
2. Call Chain Guard
3. Module Lookup
4. ACL Check
5. Approval Gate
6. **Middleware Before Chain** ← skipped by `validate()`
7. **Input Validation** ← the 6th check `validate()` performs
8. Module Execute
9. Output Validation
10. Middleware After Chain
11. Return Result

PROTOCOL_SPEC §5.1 still says "Steps 1–6" which under the post-swap numbering would include "Middleware Before Chain" — wrong (validate() must NOT run middleware).

PROTOCOL_SPEC §12.8 line 7065 already uses the corrected wording:

> "runs Steps 1–5 and Step 7 of the Executor pipeline (plus optional module-level preflight Check 7) without executing module code or middleware"

So the spec is internally inconsistent: §5.1 contradicts §12.8.

## Patch summary

Surgical 5-line edit to PROTOCOL_SPEC §5.1 around line 6462.

### Patch — replace the multi-line `validate()` description

**Before** (current, stale):

```markdown
  /**
   * [SHOULD] Non-destructive preflight check through Steps 1–6 of the
   * execution pipeline without invoking module code or middleware.
   *
   * Runs: context creation, safety checks, module lookup, ACL enforcement,
   * approval detection (report only, MUST NOT invoke ApprovalHandler),
   * and input schema validation.
   *
   * MUST NOT: execute module code, run middleware, or modify external state.
   * ...
   */
  validate(module_id: String, inputs: Map, context: Context?) → PreflightResult
```

**After** (proposed):

```markdown
  /**
   * [SHOULD] Non-destructive preflight check that runs Steps 1–5 and Step 7
   * of the execution pipeline (skipping Step 6 Middleware Before Chain),
   * plus an optional module-level preflight (Check 7), without invoking
   * module code or middleware. See §12.8 for the language-specific guide.
   *
   * Runs: context creation (Step 1), call chain guard (Step 2), module
   * lookup (Step 3), ACL enforcement (Step 4), approval detection (Step 5,
   * report only — MUST NOT invoke ApprovalHandler), input schema validation
   * (Step 7), and optionally module.preflight() for advisory warnings.
   *
   * MUST NOT: execute module code, run middleware, or modify external state.
   * ...
   */
  validate(module_id: String, inputs: Map, context: Context?) → PreflightResult
```

The trailing "..." block (`@param`, `@return`, etc.) is unchanged.

## Cross-references that must be updated together

- `PROTOCOL_SPEC.md` version history table at the bottom — bump minor version (e.g., 1.6.0-draft → 1.7.0-draft) and add a row noting this clarification.
- No `CHANGELOG.md` entry needed — this is a clarification of already-correct behavior, not a behavior change.
- Already-aligned docs (no further action required after the patch):
  - `docs/api/executor-api.md` — already says "6 pipeline checks plus optional module-level preflight" (lines 140-152).
  - `docs/api/executor-api.md` — already says "6 validation checks plus optional module-level preflight" (line 454).
  - `docs/api/client-api.md:98` — already says "6 validation checks plus optional module-level preflight".
  - `docs/features/core-executor.md:99` — already says "6 pipeline checks plus an optional module-level preflight".

## Verification checklist for reviewer

- [ ] No SDK behavior change is implied — all three SDKs already implement the post-swap order (Step 6 = Middleware Before, Step 7 = Input Validation). Verified in `apcore-python/src/apcore/builtin_steps.py` (`BuiltinMiddlewareBefore` then `BuiltinInputValidation`); same ordering in TS and Rust.
- [ ] PROTOCOL_SPEC §12.8 already uses the corrected "Steps 1–5 and Step 7" phrasing — this patch only aligns §5.1 to match §12.8.
- [ ] No anchor IDs are renamed.
- [ ] `PreflightResult` type definition (lines 6481+) is unchanged.

## Rationale for not auto-fixing

CLAUDE.md "Specification Integrity":

> Do NOT modify `PROTOCOL_SPEC.md` without a linked issue and dual maintainer approval

This patch is staged here for the maintainers to review and apply through the proper governance channel.
