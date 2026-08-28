# Unevaluable ACL Conditions, ACL Introspection, Policy Call Site — SDK Rollout

## Status

**Complete.** Specification merged on `apcore` main; apcore-python, apcore-typescript and
apcore-rust all merged and pushed; the corrected conformance fixture has moved into
`conformance/fixtures/` and all three drivers pass against it — Python 15/15, TypeScript 15/15
plus a presence guard, Rust 13 executed with 2 skipped as unrepresentable (`ACLRule.callers` is
`Vec<String>`, so the two malformed-pattern-field cases cannot be built).

## Why the fixture was staged here rather than committed to `conformance/` (resolved)

`conformance/fixtures/acl_handler_error.json` currently pins the **old** behaviour, under a case named
`throwing_handler_does_not_flip_default_allow_to_deny_unsafely` that expects a `deny` rule with a
crashing condition handler to let the call through. §6.1.1 reverses that.

Committing the corrected fixture before the SDK drivers land turns CI red in all three SDK
repositories simultaneously, for as long as the rollout takes. The corrected file therefore lived at `staged-fixtures/acl_handler_error.json` until all three
drivers had landed. It has since moved into `conformance/fixtures/`, which was the last step of
the rollout; `staged-fixtures/` no longer exists.

Do not "fix" the red by reverting the spec: the fixture is wrong relative to v1.22.0, not the other
way round.

## Landing order

1. **Spec** — merged as `f201ec7`.
2. **apcore-python** — implement §6.1.1 three-outcome evaluation, §6.1.2 warn + validator, §6.8
   accessors, §7.9.6 call site. Keep the old fixture passing by leaving
   `conformance/fixtures/` untouched; add SDK-local tests for the new behaviour.
3. **apcore-typescript** — same.
4. **apcore-rust** — same, plus the #103 doc comments and semantic constructors.
5. **Fixture** — move `staged-fixtures/acl_handler_error.json` into `conformance/fixtures/`, update
   the `conformance/README.md` row, and point all three drivers at the four new case IDs in one
   coordinated pass.

## Per-SDK task notes

### #100 — unevaluable conditions

The evaluator returns a boolean in all three SDKs today. It has to carry three outcomes.

- **apcore-python** `acl.py::_evaluate_conditions` / `_evaluate_conditions_async` — the three
  `return False` sites for unknown key, handler exception, and unresolvable coroutine become the
  UNEVALUABLE outcome. `_matches_rule` propagates it; the rule loop applies §6.1.1's effect rule.
- **apcore-typescript** `acl.ts` — same three sites (`Unknown ACL condition`, the `catch`, and the
  `result instanceof Promise` branch on the sync path).
- **apcore-rust** `acl.rs::evaluate_conditions` — the unknown-key `return false`, the
  `catch_unwind` `Err` arm, and the `Poll::Pending` arm. Note `matches_rule` and
  `matches_rule_async` are separate code paths and both need it.

Compound operators compose by the three-valued table now in §6.1.1. The two that bite:
`$not` of an unevaluable condition is **unevaluable**, never satisfied; and short-circuiting is
allowed on a decisive child (UNSATISFIED in an AND, SATISFIED in an `$or`) but **not** on an
unevaluable one, because a later sibling may still decide it.

`add_rule()` performs no validation in any SDK today — §6.1.2 rule 4 covers it.

### #101 — accessors

Additive. apcore-rust already has `rules()`; it needs `default_effect()`. apcore-python and
apcore-typescript need both, and TypeScript's `_defaultEffect` / `_rules` are `private`, so getters
are the only way to satisfy §6.8 rule 1.

### #102 — policy call site

Signature change in all three. apcore-rust's `resolve(&self, module_id, annotations)` is public, so
§7.9.6 rule 5's overload allowance applies — prefer an added method over breaking the existing one.
Call site is `builtin_steps.rs` (Rust) and the equivalent approval-gate step elsewhere; the
arguments and context are already in scope there.

### #103 — apcore-rust doc comments

Twelve occurrences of "Construct via `..Default::default()` or a builder pattern" across
`approval.rs` (2), `module.rs` (4), `async_task.rs` (3), `config.rs` (2), `middleware/retry.rs` (1).
`Change` and `PreviewResult` carry no construction guidance at all and want semantic constructors
rather than a corrected sentence.

## Out of scope, recorded deliberately

- **§6.5 "conditions present but no context provided" stays a non-match.** Calling with no context is
  a legitimate shape for external entry points, not a misconfiguration; treating it as unevaluable
  would flip the decision for every `@external` call meeting a conditional `deny` rule. It gained a
  warning and an explicit note on the consequence instead.
- **A declarative argument predicate on `PolicyRule`.** Deferred to a joint design with ACL
  conditions — see the note under §7.9.6.
