---
description: "Maintainer decision log for three cross-language inconsistencies found during a 2026-09-05 consistency audit (independent of the 2026-05 apcore-skills:sync batch), covering start_reaper's async signature, APCore.discover's sync/async split, and the ACL construction door's rule-mutation gap."
title: Cross-language alignment decision log (2026-09)
date: 2026-09-05
status: resolved — 3/3 items closed the same day they were opened
audience: maintainers + spec reviewers
source: cross-repo consistency audit (2026-09-05), triggered while dispatching parallel SDK fixes for the 2026-05 batch's remaining findings
---

# Cross-language alignment — 2026-09 decisions

This log is separate from `2026-05-decision-log.md` on purpose: these three items were not part of the 20-module 2026-05 sync batch that log tracks. They surfaced while investigating a fourth, unrelated finding (`ApprovalRequest.caller_id`/`action`, decision D-03) and were resolved the same session, so this file records the reasoning rather than an open recommendation awaiting a separate approval round.

Decision template per item: **Status quo** (what each SDK does today, with evidence) / **Options considered** / **Decision** / **Action taken**.

---

## E-01 — `AsyncTaskManager.start_reaper`: does the call itself need to be awaited?

**Status quo**

- `docs/features/async-tasks.md`'s canonical signature table showed all three SDKs as `await manager.start_reaper(...)`, citing decision **D-11**.
- Read against D-11 itself (`2026-05-decision-log.md`): D-11 fixed the argument names (`ttl_seconds`, `sweep_interval_ms`), their units, and the `ReaperHandle` return type. It never decided that `start_reaper` itself must be awaitable — that requirement was written into the async-tasks Contract at some point after D-11, without its own tracking decision.
- None of the three SDKs has any `await`/`.await` inside `start_reaper` before the handle is returned:
  - Python: plain `def start_reaper(...)`. Its own test suite documents this as deliberate — `test_start_reaper_property_async` (`tests/test_async_tasks_spec.py`): "Python `start_reaper` itself returns the handle synchronously; the spawned loop is the async/background effect."
  - Rust: plain `fn start_reaper(...)`. Confirmed during this session's SDK dispatch: the function body has no internal `.await`; making it `async fn` would be a pure signature change, not a restructuring.
  - TypeScript: this session initially made `startReaper` return `Promise<ReaperHandle>` (via `Promise.resolve(handle)`, deliberately not marking the function `async`, to avoid turning its synchronous "already running" guard into a rejected Promise).

**Options considered**

- **A.** Force Python and Rust to become genuinely `async`/`.await`-able, matching the literal contract text. Rejected: breaking change with no functional justification in either language — scheduling a background task is a synchronous action in all three, and neither SDK's internals need to wait on anything before returning the handle.
- **B (as first proposed).** Add a second, purely-async entry point (`start_reaper_async` / `startReaperAsync`) alongside the existing synchronous one. Rejected on review: this doubles the public API and test matrix in Python and Rust without removing the inconsistency it exists to fix — a caller would still need to know which of two canonical names to prefer, in every language, forever.
- **C.** Correct the contract to state what is actually true and structurally necessary: `start_reaper` starts synchronously and returns the handle immediately in every SDK; only `ReaperHandle.stop()` is genuinely awaitable, because stopping has to wait for an in-flight sweep to drain.

**Decision: C.** TypeScript's `Promise<ReaperHandle>` return is documented as a compatibility surface (harmless to `await`, not required to), not elevated to a cross-language MUST.

**Action taken**

- `docs/features/async-tasks.md`: canonical signature table, Properties block, and both Python/Rust code examples corrected to drop `await`/`.await` on `start_reaper` itself; the rationale paragraph above is now in the doc, with the D-11-scope correction stated explicitly.
- No SDK code changes beyond what this session's TypeScript dispatch had already done (`Promise<ReaperHandle>`, not `async fn`) — Python and Rust are conforming as-is.
- Governance: maintainer-approved in this session; no separate GitHub issue — this log is the record.

---

## E-02 — `APCore.discover`: "synchronous in all languages" contradicted two of three implementations

**Status quo**

- `docs/features/apcore-client.md`'s Properties line read: "async: false (synchronous in all languages; file-system I/O and module imports run on the calling thread)."
- Python: genuinely synchronous.
- TypeScript: `async discover(): Promise<number>`, and not merely by convention — `_discoverDefault()` awaits `_ensureIdMap()`, `_scanRoots()`, and `resolveEntryPoint()` (`src/registry/registry.ts`), the last of which resolves a module's entry point via ESM dynamic `import()`. Node's ESM loader has no synchronous dynamic import; a discovery root containing an ESM module file cannot be scanned without an `await` somewhere in the call chain.
- Rust: `pub async fn discover(&self, discoverer: &dyn Discoverer)` awaits `discoverer.discover(...)` (`src/registry/registry.rs`). The `Discoverer` trait is `async` in its own definition, not a synchronous trait merely called from an async wrapper — the same "pluggable, possibly-async extension point" pattern as TypeScript's `CustomDiscoverer` (which may itself return a `Promise`) and Python's `ApprovalHandler`.

**Options considered**

- **A.** Force TypeScript and Rust's `discover()` to become synchronous. Rejected: not achievable in TypeScript at all (ESM dynamic `import()` has no sync form); achievable in Rust only by abandoning the `Discoverer` trait's async extensibility, which would be removing a real capability to satisfy a contract line that was never validated against either implementation.
- **B.** Correct the contract to state the real, language-necessitated split, and make the actual cross-language guarantee the **outcome** (return value, error types, registry state after the call) rather than the calling convention.

**Decision: B**, unambiguously — unlike E-01, at least one SDK (TypeScript) has a hard language constraint, not merely a design preference, so "align the SDKs to the doc" was never actually available as an option here.

**Action taken**

- `docs/features/apcore-client.md`: Properties line rewritten to state Python sync / TypeScript+Rust async, with the specific structural reason for each (ESM dynamic import; an async `Discoverer` trait), and to name the return-value/error/registry-state contract as what actually needs to hold across languages.
- No SDK code changes — this was a documentation-only correction; no implementation was non-conforming with anything but a contract line that had never been checked against them.
- Governance: maintainer-approved in this session; no separate GitHub issue.

---

## E-03 — `ACL(rules=[...])` did not re-validate a rule mutated before its first construction

**Status quo (as this was first, incorrectly, assessed)**

An initial pass flagged `ACL.__init__` (Python) as silently accepting a rule mutated after `ACLRule`'s own construction but before ever being passed to `ACL(rules=[...])` for the first time — while apcore-typescript and apcore-rust both reject the identical sequence. The initial assessment concluded this was **not** a defect: it read `ACL.__init__`'s existing test coverage (`tests/test_acl.py::TestPatternArrayArityBackstop`, whose `_mutated()` helper mutates a rule *before* the one `ACL(rules=[...])` call that ever sees it) as evidence the silent-accept behavior was deliberately spec-sanctioned, and recommended leaving all three SDKs' differing behavior as a permitted, language-optional choice (Option "C" in that pass).

**Where that assessment was wrong**

- PROTOCOL_SPEC §6.1.4.1 / §6.2.1's phrase "assigned onto an already-constructed rule, which no constructor intercepts" is genuinely ambiguous between two readings: (1) a rule already installed inside a *live, previously-constructed* `ACL` and mutated afterward through a reference the caller holds — a route no door runs again to intercept — or (2) any rule object currently holding a bad value, regardless of whether it had ever been offered to a door. apcore-typescript and apcore-rust both implement reading (1) (their constructors validate every rule they are handed, unconditionally); the initial assessment read the spec as (2) and concluded Python's behavior was the sanctioned one.
- The conformance fixture (`conformance/fixtures/acl_pattern_arity.json`) settles which reading is correct: all 9 existing `kind: "backstop"` cases carry `"mutation_route": "installed_rule"`, and the Python conformance driver (`tests/conformance/test_acl_pattern_arity.py::_run_backstop`) implements this literally — it constructs `ACL(rules=[ACLRule(**valid_kwargs)])` from a **well-formed** rule, retrieves the live rule via the public `acl.rules[0]` accessor, and mutates it **afterward**. At no point does this driver hand a pre-mutated rule to a fresh `ACL(...)` call. `ACL.__init__`'s own unit test class (`TestPatternArrayArityBackstop`, distinct from the conformance driver) tested a different sequence — mutate first, construct fresh — that the actual conformance contract never required and that a validating `ACL.__init__` does not touch, because the mutation there happens *after* construction has already succeeded.
- `ACL(rules=[...])` is explicitly one of the three entry points PROTOCOL_SPEC §6.1.6 rule 3 names ("direct construction"). A rule handed to it for the first time is being offered to a door for the first time, regardless of what happened to the rule object beforehand — construction history is not legible to the door receiving it and cannot be what decides whether the door's own check runs.

**Decision:** Fix Python (apcore-typescript and apcore-rust were already conforming). Disambiguate the spec text so this cannot recur as an interpretation question. Add a conformance case exercising exactly this sequence, expecting `reject` uniformly. Do **not** record this as SDK-optional behavior.

**Action taken**

- `docs/spec/protocol-spec.md` §6.1.4.1 and §6.2.1: both instances of the ambiguous phrase rewritten to state the backstop covers only a rule *already installed inside a live ACL and mutated afterward*; a rule mutated before its first offering to any door — `ACL`'s own constructor included — MUST be rejected there. Table row at §6.1's field-table summary corrected to match. Spec bumped to **v1.33.0**; changelog entry added.
- `apcore-python/src/apcore/acl.py`: `ACL.__init__` now calls `_validate_rule(rule, where="ACLRule")` on every rule in `rules`, mirroring `add_rule`'s existing call, raising `ACLRuleError` on the first (lowest-index) invalid rule.
- `apcore-python/tests/test_acl.py`: `TestPatternArrayArityBackstop._mutated()` corrected to construct the `ACL` from a well-formed rule *first* and mutate the installed rule through the public `rules` accessor afterward — matching the conformance driver exactly, and continuing to test the genuine, uninterceptable backstop case. New class `TestConstructionRejectsAPreviouslyMutatedRule` added for the newly-rejected sequence (13 new test cases total; full suite: 5298 passed, 37 skipped, 5 xfailed, no regressions).
- `apcore-python/tests/conformance/test_acl_pattern_arity.py`: new driver `_door_construct_mutated`, registered as a second driver for the `construct` door (mirroring `add_rule`'s existing three-driver pattern), so every existing `kind: "closure"` reject case listing `construct` is now also exercised via mutate-then-construct, not only build-with-a-bad-value-directly.
- apcore-typescript and apcore-rust: **no code changes** — both already reject this sequence (verified empirically this session: `new ACL([mutated])` throws `ACLRuleError` in TypeScript; `ACL::try_new` calls `validate_rule` unconditionally on every rule in Rust). Their existing `construct`-door test coverage already satisfies the disambiguated spec without modification.
- Governance: maintainer-approved in this session, correcting this log's own initial (incorrect) recommendation; no separate GitHub issue.
