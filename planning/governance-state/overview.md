# Governance state accessor — issue #97

**Internal planning. Not published to the doc site.**

Spec side landed: PROTOCOL_SPEC §6.6.3 (rewritten), §6.6.3.1, §6.6.3.2, §6.6.5,
plus `docs/features/core-executor.md` "Governance State API" and cross-links from
`system-modules.md`, `acl-system.md`, `approval-system.md`.

Held here: `fixtures/governance_state.json` (10 cases). Same CI reason as
`planning/usage-contract-parity/overview.md` — see that file for the landing-order rule.

## The finding that shaped the spec text

Issue #97 frames "an ACL attached but the gate not wired" as something a *custom*
strategy can cause. It is stronger than that: **three of the four presets this
specification itself defines remove `acl_check`** — `internal`, `testing` and
`minimal` (`docs/spec/design-execution-pipeline.md` §5.2). So
`executor.set_acl(acl)` followed by selecting `internal` is a documented, supported
configuration in which `acl != null` and no ACL evaluation happens. That is now §6.6.3.2,
and it is why the accessor reports `acl_configured` and `builtin_acl_gate_wired`
separately rather than collapsing them.

## Landing order

1. **Spec** — §6.6.3 rewrite + §6.6.5. *Done.*
2. **SDK implementation**, three PRs. Purely additive; no default changes.
   - Each SDK adds `governance_state()` / `governanceState()` on `Executor`,
     returning the eight fields of §6.6.5.1 with the field names spelled in that
     language's casing.
   - `builtin_*_gate_wired` reuses the type test the executor **already performs**
     when wiring: `set_acl()` locates the step by name *and* built-in type before
     injecting. Reuse that predicate; do not add a name-only one.
   - apcore-rust's existing public `acl` / `approval_handler` / `policy` fields stay.
     Privatising them is a separate decision and is not part of this.
3. **Drivers** for `governance_state.json` in all three SDKs, merged.
4. **Fixture** — move to `conformance/fixtures/`, add rows to `conformance/README.md`
   and `docs/spec/conformance.md` §8.1, bump the Total.
5. **Consumers**, only after step 3: `apcore-mcp#15` and `apcore-a2a#5` call the
   accessor instead of re-deriving the condition from `describe_pipeline()` output,
   which carries step names only.

## Do not

- Make the accessor warn, throw, enforce or mutate. The reaction belongs to the
  caller; putting it inside makes it unavoidable and untestable (§6.6.5.3).
- Return the ACL object, the handler, or rule content. Booleans only.
- Add an `is_secure` field or present `unprotected_control_surface` as a security
  verdict. It reports the absence of a recognised gate, never the presence of
  protection — a wired ACL that permits everything still yields `false`.
- Detect gates by step name. `custom_step_named_acl_check_is_not_the_builtin` is the
  case that catches it, and a name test fails it in the one direction that matters:
  reporting a gate that is not there.
- Change the ACL discovery default or the approval-skip default. Both are stated as
  invariants in §6.6.3.1 and neither is in scope here.
