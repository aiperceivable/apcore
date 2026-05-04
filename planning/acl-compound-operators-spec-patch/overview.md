# ACL Compound Operators — PROTOCOL_SPEC Patch Draft

## Status
DRAFT — needs linked issue + 2 maintainer reviews per CLAUDE.md "Specification Integrity" rule before merging.

## Background
The `$or` and `$not` compound operators are:
- **Implemented** in all three SDKs (apcore-python, apcore-typescript, apcore-rust) since the acl-conditions-redesign release.
- **Tested** in `conformance/fixtures/acl_evaluation.json` (4 dedicated cases: `$or`/`$not` in conditions and as caller-array prefix).
- **Designed** in `docs/spec/design-context-annotations-acl.md` (handler protocol, sync/async semantics, fail-closed rules).
- **User-documented** in `docs/features/acl-system.md` (now expanded — see commit alongside this draft).

But **not specified** in `PROTOCOL_SPEC.md §6`. SDK authors (and audit tooling) currently rely on the design doc + reference implementation, which is fragile and contradicts CLAUDE.md "Spec Integrity" expectations.

## Patch summary
Two surgical additions to `PROTOCOL_SPEC.md §6`:

### Patch 1 — extend §6.1 "Conditions sub-fields" table

Add two rows after `max_call_depth`:

```markdown
| `$or`           | `list[object]` | Compound: passes if **any** sub-condition object passes (each sub-object's keys are AND-ed internally). Sub-objects MAY contain further compound operators. |
| `$not`          | `object`       | Compound: passes if the wrapped condition object **fails**. Empty object MUST evaluate to false (fail-closed).                                              |
```

Add a paragraph beneath the table:

> **Compound + async:** Implementations MUST evaluate `$or`/`$not` sub-conditions using the same evaluator mode (sync vs async) as the enclosing call. An async-only sub-condition under a sync evaluator MUST fail closed and SHOULD emit a warning. Handlers SHOULD therefore be registered for both sync and async paths.

### Patch 2 — new §6.2.1 "Compound Operators in Pattern Arrays"

Insert **after** §6.2 "Rule Matching" and **before** §6.3 "Rule Evaluation Algorithm":

```markdown
### 6.2.1 Compound Operators in Pattern Arrays

The `callers` and `targets` pattern arrays MAY use the compound operators `$or` and
`$not` as the **first element** to alter the default OR-of-patterns semantics.

| Form                          | Semantics                                                                                                  |
|-------------------------------|------------------------------------------------------------------------------------------------------------|
| `["$or", p1, p2, ...]`        | **MUST** match the module ID if any of `p1, p2, …` matches. Observably equivalent to a flat list (which is also OR-ed) but documents intent explicitly. |
| `["$not", p]`                 | **MUST** match the module ID if `p` does **not** match.                                                    |
| `["$not"]` (no pattern)       | **MUST** evaluate to false (fail-closed).                                                                  |
| `["$not", p1, p2, ...]`       | Implementation-defined: SDKs MUST consult `p1` and MAY ignore subsequent patterns. Authors **SHOULD NOT** rely on this form. |

When `$or` or `$not` appear at any position other than index 0 of a pattern array,
implementations **MUST** treat them as literal pattern strings (no special semantics).
Implementations **MUST NOT** match a literal module ID equal to `"$or"` or `"$not"`
under default-deny semantics — these tokens are reserved for compound-operator use.
```

## Cross-references that must be updated together

- `PROTOCOL_SPEC.md` version history table — bump minor version (e.g., 1.6.0-draft → 1.7.0-draft)
- `CHANGELOG.md` — add to next release's "Added" section: "PROTOCOL_SPEC §6.1 / §6.2.1 — formalise `$or`/`$not` compound operators (already implemented in all 3 SDKs since 0.18; spec was lagging)."
- `schemas/apcore-config.schema.json` (if it constrains `conditions`) — extend to allow `$or` / `$not` keys. **Verify before patching** — current schema may already allow free-form objects.
- `docs/spec/conformance.md` — link the 4 fixture cases to the new spec sections so conformance traceability is bidirectional.

## Verification checklist for reviewer

- [ ] All 3 SDKs already pass `acl_evaluation.json` fixture cases for `$or`/`$not` (verified 2026-05-04 — see `acl.py:449-456`, `acl.ts:304-315`, `acl.rs:467-476`).
- [ ] `docs/features/acl-system.md` "YAML Configuration Format" matches the patch (updated in this commit batch).
- [ ] `docs/spec/design-context-annotations-acl.md` doesn't contradict the patch (it doesn't — design doc is the authoritative source for handler protocol; this patch only formalises the user-facing surface).
- [ ] `apcore-config.schema.json` validates a `conditions` block containing `$or` / `$not` (re-run `mkdocs build` + schema validation tests).

## Open question
Should `["$not", p1, p2, ...]` be made **normative-error** rather than implementation-defined? Current SDKs silently ignore extras; tightening this is a behaviour change requiring a major version bump. Recommendation: leave implementation-defined now, revisit in 2.0.
