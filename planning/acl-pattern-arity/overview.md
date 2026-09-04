# Pattern-Array Arity Is Closed — PROTOCOL_SPEC §6.2.1 Patch Draft (#112)

## Status

**APPLIED to the apcore repo.** Maintainer approval given on 2026-09-04 (`MAINTAINERS.md`
lists one maintainer, so one approval satisfies GOVERNANCE.md § Decision Making); linked
issue [#112](https://github.com/aiperceivable/apcore/issues/112). Landed as spec **v1.31.0**:

- `docs/spec/protocol-spec.md` — §6.2.1 replaced in full, §6.1 field table, §6.1.4.1,
  §6.1.5, §6.5's edge-case row, version-history row. Open questions 8 and 9 were resolved
  in favour of the recommendations and are normative text in §6.2.1.
- `schemas/acl-config.schema.json` — `$defs.PatternArray`, both fields rewired to it.
- `conformance/fixtures/acl_pattern_arity.json` — 44 cases, moved here from
  `staged-fixtures/` (that directory no longer exists).
- `conformance/fixtures/acl_evaluation.json` — 21 → 19 cases; the two superseded cases
  deleted (step 1b).
- `conformance/README.md`, `docs/spec/conformance.md` (68 fixtures / 804 cases),
  `docs/features/acl-system.md`, `CHANGELOG.md`.

`mkdocs build` clean; no heading/anchor removed from either edited doc; conformance counts
re-derived from the fixture files and matching in all 68 §8.1 rows.

**SDK state:** all three implemented on unpushed `fix/acl-pattern-arity-112` branches —
apcore-python `9dd6ed5` (5190 passed), apcore-typescript `f17092b` (4535 passed),
apcore-rust `9ede204` (2696 passed). Follow-ups outstanding from questions 8 and 9, plus
the conformance drivers for the new fixture and the removal of the transitional handling
each SDK carried for the two now-deleted `acl_evaluation.json` cases.


**Amended after the first SDK follow-up (same day).** apcore-typescript's follow-up found
three defects in the just-landed §6.2.1 ordering clause, all confirmed and all fixed:
the clause cited §6.1.6 for an `effect`-before-`approval` ordering that §6.1.6 **does not
state**; cross-rule order was left open, so `ACL.load` and direct construction reported
different faults for the same file within one SDK; and the position of §6.1.4.1's *type*
fault relative to the shape closure was unstated. Point 2 now pins axis order, rule-index
dominance, and the pattern fields as a single axis. Three fixture cases were added to pin
it (`effect_is_reported_before_a_pattern_fault`,
`approval_is_reported_before_a_pattern_fault`, `callers_is_reported_before_targets`), with
a new `expected_refused_axis` key, because `expected_load` cannot see which fault is named.
41 → 44 cases.


**Second amendment — three implementation-found defects in the landed text.** Each was
found by an SDK implementing it, and each was a real divergence rather than a wording nit:

1. **The sweep prohibition did not name its own scope** (found by apcore-rust). Point 2
   enumerated `effect` / `approval` / patterns and then forbade sweeping "one axis" across
   every rule. A loader has *other* per-rule axes — the rule key set (§6.1.5), the
   missing-field check, the value types — and `ACL::load` swept the key closure across the
   whole file before any rule's `effect` was read, refusing a file for rule 1 that should
   have been refused for rule 0. An implementer scoping "axis" to the three named would
   read the sentence just as fairly and leave the bug in. The text now says outright that
   axis means **every per-rule check the door performs**, and names a loader's extras.
2. **`default_effect` had no stated position** (found by apcore-rust, then bitten by
   apcore-typescript). It is not a rule and has no index, so the rule ordering never
   reached it — yet apcore-typescript's loader validated it *after* parsing every rule, so
   one file was refused for rule 0's patterns through the file door and for
   `default_effect` through direct construction. It is now judged first, at every door.
3. **"First" was ambiguous at one boundary** (found by apcore-typescript). It now means
   ahead of the file-level checks on the `rules` collection itself, so a document both
   missing `rules` and carrying a bad `default_effect` is refused for the `default_effect`.

**Deliberately not fixture-pinned:** point 3's combination. A doubly malformed document is
refused either way and only the message differs, so it is stated in §6.2.1 and guarded by an
SDK-local test in each implementation rather than by a 52nd case. It is the one place the
three can still drift, and it is recorded here so that is a choice rather than an oversight.

Fixture grew 41 → 44 → 49 → 51 across these amendments.

Nothing is pushed.

## Verdict

**Necessary: yes.** The defect is real, reproduced independently in two SDKs and confirmed
by source reading in the third, and it reaches production through a plain YAML file rather
than only through direct construction. A `deny` rule that an operator wrote, that loads
without error and that `validate_rules()` calls clean, permits the call it names — under
`default_effect: allow`. Three separate normative artefacts in this repository already say
the shape is illegal and no code path enforces any of them.

**Implementable precisely: yes.** The fault is per-rule, static, context-free, handler-free
and identical in all three languages. It needs no new vocabulary and no new machinery: the
rejection mechanism is §6.1.5's, the unevaluable-resolution mechanism is §6.1.1's, the
reporting shape is §6.1.3 rule 3's. The one genuinely new code path is in apcore-rust,
which today has no pattern-field precheck at all because `Vec<String>` made §6.1.4.1's
*type* fault unrepresentable — and `Vec<String>` places no constraint on *length*.

**But the issue's proposed mechanism is the wrong one**, and the difference matters in
production. See **Resolution** below.

### What is one decision and what is four

Only the first row below is the fix #112 asks for. The other three were found while
verifying it, each is argued on its own merits, and **each is separable** — a maintainer can
take any subset without unravelling the rest. They are listed here rather than only in
**Open questions** at the end because a reader should not have to reach line 900 to learn
that three quarters of this draft is optional.

| Part | Scope | Drop it by |
|---|---|---|
| **Arity closure** — `[]`, `["$or"]`, `["$not"]` rejected at every door; UNEVALUABLE backstop | #112 as filed | — this is the fix |
| **`$not` exactly one operand** — rejects `["$not", p1, p2, …]` | beyond arity: replaces an *implementation-defined* row that is a live privilege escalation | one row of Patch 1, 3 fixture cases |
| **Reserved token only at index 0** — plus the flatness statement and worked examples | beyond arity: retires two §6.2.1 clauses no implementation honours | one paragraph of Patch 1, the `items.not` in the schema, 3 fixture cases |
| **Tier 2** — well-formed arrays that still match nothing, reported not rejected | beyond structure entirely: reasons about the match relation | one paragraph of Patch 1, 6 fixture cases |

The flatness statement and the worked examples are the one part with no behavioural
component at all — §6.2.1 has never contained an example, and it never says the operators
do not nest while the same tokens nest freely in `conditions`. That part is safe to take
even if every other row is deferred.

## What was measured

Independently of the issue's own matrix, which was taken against the Rust crate at 0.28.

### The matrix reproduces exactly in apcore-python

Rule is always `callers: ["*"]`, probe is `check(None, "cli.rm", None)`, one rule in the ACL:

| `targets` | rule `effect` | `default_effect` | `check()` | `validate_rules()` |
|---|---|---|---|---|
| `[]` | `deny` | `allow` | **ALLOW** | 0 findings |
| `[]` | `deny` | `deny` | deny | 0 findings |
| `[]` | `allow` | `allow` | ALLOW | 0 findings |
| `[]` | `allow` | `deny` | deny | 0 findings |
| `["$not"]` | `deny` | `allow` | **ALLOW** | 0 findings |
| `["$not"]` | `deny` | `deny` | deny | 0 findings |
| `["$not"]` | `allow` | `allow` | ALLOW | 0 findings |
| `["$not"]` | `allow` | `deny` | deny | 0 findings |
| `["$or"]` | `deny` | `allow` | **ALLOW** | 0 findings |
| `["$or"]` | `deny` | `deny` | deny | 0 findings |
| `["$or"]` | `allow` | `allow` | ALLOW | 0 findings |
| `["$or"]` | `allow` | `deny` | deny | 0 findings |

Twelve for twelve against the issue's Rust table. The outcome tracks `default_effect`
exactly; the rule contributes nothing in any cell.

### The YAML path reaches it

```yaml
default_effect: allow
rules:
  - callers: ["*"]
    targets: []
    effect: deny
    description: "block everything dangerous"
```

`ACL.load` → OK. `validate_rules()` → 0 findings. `check(None, "cli.rm", None)` → `True`.

### All three matchers agree, at the source

| SDK | Site | Behaviour |
|---|---|---|
| apcore-rust | `src/acl.rs` `match_patterns` | `if patterns.is_empty() { return false }`; `$not` with `len() < 2` → `false`; `$or` slices `[1..]` and `any()` over nothing → `false` |
| apcore-python | `src/apcore/acl.py` `_match_patterns` | `if not patterns: return False`; `$not` with `len < 2` → `False`; `any(...)` over `patterns[1:]` → `False` |
| apcore-typescript | `src/acl.ts` `_matchPatterns*` | `if (patterns.length === 0) return false;`; `$not` with `length < 2` → `false`; `$or` loop over `slice(1)` → `false` |

The agreement is what makes this a specification gap rather than three SDK bugs.

## Five things the issue does not say, and all five change the answer

### 1. The JSON Schema has always forbidden it

`schemas/acl-config.schema.json` declares **`"minItems": 1`** on both `callers` and
`targets`. It has done so since the file existed. No implementation validates an ACL file
against the schema at load time — which is the *exact* sentence §6.1.5 already carries
about `additionalProperties: false` (#107) and about the `effect` enum (#111).

This is the third instance of one pattern: **the normative artefact declared the
constraint, and no door enforced it.** The two previous instances were both resolved by
closing the doors with `ACLRuleError`, not by inventing runtime semantics for the illegal
value. That precedent points directly at the resolution below.

`docs/features/acl-system.md` says it too, in `add_rule`'s Preconditions: *"`rule` is a
well-formed `ACLRule` (**callers + targets non-empty**, effect ∈ {"allow", "deny"})"*.

### 2. The current behaviour is already specified — in two places, both as **MUST**

§6.2.1's table, since v1.7.0-draft:

> | `["$not"]` (no pattern) | **MUST** evaluate to false (fail-closed). |

And §6.5's edge-case table, one row above the row that already routes the *type* fault to
§6.1.4.1:

> | `callers` or `targets` in rule is an empty **list** | Rule never matches | **MUST** |

The issue proposes making both a precheck fault without noting that it is replacing two
standing **MUST**s, and does not mention §6.5's row at all. Replacing them is permitted —
CLAUDE.md forbids removing or weakening a MUST *without* a deprecation notice and a version
bump, and this is a strengthening with both — but it has to be stated in the version-history
entry rather than slipped in, and §6.5's row has to be edited or the specification
contradicts itself in two sections.

The parenthetical **"(fail-closed)" is itself wrong**, and wrong in the direction §6.1.1
exists to name. A non-match is fail-closed on an `allow` rule and fail-**open** on a `deny`
rule: a `deny` rule that never matches refuses nothing, and under `default_effect: allow`
the call is permitted. The label was written before v1.22.0 established that the direction
"does not match" points is decided by the rule's `effect`. No single non-match can be
called fail-closed.

### 3. A second defect in the same four-row table, and this one is not inert

§6.2.1 row 4:

> | `["$not", p1, p2, ...]` | Implementation-defined: SDKs **MUST** consult `p1` and **MAY** ignore subsequent patterns. Authors **SHOULD NOT** rely on this form. |

All three SDKs consult `p1` and drop the rest, so the form is *consistent* across
implementations and consistently **wider than written**. Measured in apcore-python 0.28,
rule `{callers: ["*"], targets: ["$not", "secrets.a", "secrets.b"], effect: "allow"}` under
`default_effect: deny`:

| target | granted |
|---|---|
| `secrets.a` | no |
| `secrets.b` | **yes** |
| `other` | yes |

The operator excluded two targets from an `allow` rule and the second one is granted. That
is a silent privilege escalation — the §6.1.5 admonition's exact wording — from a form the
specification blesses. `SHOULD NOT rely on this form` is not a guard: nothing reports it,
nothing rejects it, and the rule loads clean.

It belongs in this change because it is the **same question stated in the other
direction**: `$or` takes *at least* one operand, `$not` takes *exactly* one. Fixing one row
of a four-row table and leaving another row carrying a documented fail-open is a half-fix.
It is written below as a separable part so a maintainer who wants the narrower change can
drop it without unravelling the rest.

### 4. #112's rule closes an enumeration, not the class it names

The issue's title scopes it to "a pattern list with no operands", but its body says *"This is
the same failure class as #100 and #107: a rule that is wider than written, with nothing
reporting it."* Those are different sets. Measured, same probe (`deny` rule,
`default_effect: allow`, target `cli.rm`), one rule in the ACL:

| `targets` | matches anything? | `deny` + `default: allow` | findings | covered by #112 rule 1 |
|---|---|---|---|---|
| `[]` / `["$or"]` / `["$not"]` | never | **ALLOW** | 0 | yes |
| `["$not", "*"]` | **never** | **ALLOW** | 0 | **no** |
| `["$not", "**"]` | **never** | **ALLOW** | 0 | **no** |
| `[""]` / `["$or", ""]` | only the empty module ID | **ALLOW** | 0 | **no** |
| `["$or", "$not", "a"]` | only `a` — the `$not` is a dead literal | **ALLOW** | 0 | **no** |
| `["$not", "a", "b"]` | matches `b` — item 3 above | deny | 0 | **no** |
| `["cli.*"]` *(control)* | yes | ALLOW *(correctly — the rule does not name `cli.rm`)* | 0 | n/a |

`["$not", "*"]` is the sharpest: **perfectly legal arity**, exactly one operand, and it
matches nothing at all, producing the identical fail-open. #112's rule lets it through.

This is the mistake v1.25.0 recorded about §6.1.1's own original text — *"the closed list of
three unevaluable situations was **wrong, and wrong in the direction the section exists to
prevent**"* — which was fixed by restating it as a **principle with non-exhaustive
examples**. Counting three shapes repeats it one level down.

The rule also never says where the boundary is. Arity is **structural**; "matches nothing"
is **semantic**; both are decidable with no context and no handler. An implementer has to
pick a line and #112 does not draw one, so three SDKs will draw three.

### 5. All three SDKs violate §6.2.1's reserved-token MUST NOT today

§6.2.1 already says:

> Implementations **MUST NOT** match a literal module ID equal to `"$or"` or `"$not"` under
> default-deny semantics — these tokens are reserved for compound-operator use.

Measured in apcore-python: `targets: ["a", "$or"]` matches the module ID `$or`;
`targets: ["$or", "$not", "a"]` matches `$not`. All three SDKs route a non-index-0 element
straight to the glob matcher with no reserved-token check (`match_acl_pattern` in Rust,
`_match_pattern` in Python, `_matchPattern` in TypeScript), so all three do it.

Marginal on its own — a module ID would have to be literally `$or`. It matters here because
the tier-1 rule below **makes the violation unreachable by construction**: once a reserved
token may appear only at index 0, where it is an operator, no pattern can ever be the
literal string `$or`. One structural rule retires an unenforced behavioural MUST, for free.

### And one thing that makes the current behaviour deliberate rather than accidental

apcore-python carries a passing test asserting exactly the dangerous cell:

```python
# tests/test_acl_unevaluable_conditions.py
def test_an_empty_list_is_well_formed_and_simply_never_matches(self) -> None:
    """§6.5 keeps this a plain non-match — it is a valid list of strings."""
    acl = ACL(
        rules=[ACLRule(callers=[], targets=["*"], effect="deny")],
        default_effect="allow",
        audit_logger=captured.append,
    )
    assert acl.check("attacker", "service.op", _ctx()) is True
    assert captured[0].handler_error is None
```

This was decided during #106's implementation and enshrined. It cannot be corrected as an
SDK bug; the spec has to move first. The test is a direct casualty and is listed under
**Per-SDK notes**.

## Resolution

**Close the doors. Keep the precheck as a backstop for the one route no door covers.**

This differs from the issue, which proposes precheck-fault → UNEVALUABLE as the *primary*
mechanism. Under that proposal a `deny` rule with `targets: []` **denies every call in the
deployment**, discovered at runtime, on the traffic path. Under this draft the same
configuration **fails to load**, naming the field, at boot.

Why rejection is primary:

1. **The fault has no runtime excuse.** §6.1.5's own reasoning for why an unknown *rule
   key* may fail the load while an unknown *condition key* may not: `register_condition`
   writes to a runtime registry that discovery may legitimately precede, and a rule key has
   no such excuse because its set is fixed by the specification. A pattern array's arity is
   fixed the same way — it is decidable at every entry point, in every language, with no
   registry consulted and no context available. The mechanism §6.1.5 chose for #107 and
   #111 is available here and is the strongest one.
2. **The schema already says `minItems: 1`.** Enforcing a declared constraint is not new
   semantics.
3. **A fail-stop is the right answer for a security-policy config error.** The affected
   population is, by construction, deployments carrying a rule that provably does nothing.
   A boot-time error naming `targets` is immediately actionable. "Your deployment silently
   began denying everything" is not.
4. **The cited downstream already chose it.** `apexe`'s `never_matches`
   (`src/governance/acl.rs`) covers precisely these three shapes and **refuses to start**.
   The issue names it as the missing detector; it is also the missing precedent.

Why the precheck backstop is still required:

`ACLRule` is mutable in all three SDKs — a non-frozen `@dataclass` in Python, a plain
`interface` in TypeScript, `pub` fields in Rust. Assigning `rule.targets = []` on an
already-constructed rule is the one route no constructor can intercept. §6.1.5's v1.30.0
text disposes of that route for `effect` by observing that the value is never read again
once the doors are closed. **That reasoning does not transfer**: a mutated pattern array
*is* read — the matcher will consult it on the next `check()`. So the runtime classification
has to be stated, and §6.1.4.1 is where it belongs, since it already classifies a malformed
value on this same field.

This is the same two-layer structure §6.1.4.1 and §6.1.5 already have for `callers` /
`targets`: reject at the door, classify whatever arrives around it.

### Two tiers, and the line between them

Finding 4 above is that the inert class is larger than three shapes. It does **not** follow
that all of it should be rejected, because the shapes split cleanly on *what has to be
reasoned about to detect them*:

| | Tier 1 — **structural** | Tier 2 — **semantic** |
|---|---|---|
| Predicate over | the array's shape: length, element types, token positions | the **match relation**: does any legal module ID satisfy this array |
| Finite and total? | yes — a fixed list of shape checks | no — depends on the pattern language |
| Already declared? | yes, by `schemas/acl-config.schema.json` and §6.2.1 | no |
| Mechanism | `ACLRuleError` at every entry point; UNEVALUABLE as the mutation backstop | `validate_rules()` finding only |
| Changes a decision? | yes | **no** |

**Why tier 2 is a finding and not a rejection.** Three reasons, and the third is the
decisive one.

1. `["$not", "*"]` is semantically *legible* — "not everything" is a sentence, and an
   operator may well have written it to park a rule without deleting it. `targets: []` is
   not a sentence.
2. Detecting it requires reasoning about the glob language, which the specification defines
   in §6.2's algorithm but does not close: a future pattern feature changes which arrays are
   satisfiable.
3. **An incomplete predicate is survivable in a validator and not at a door.** §6.1.4's
   determinism guarantee — findings are a pure function of the rule, identical across
   implementations — is achievable for tier 1's finite structural predicate and is *not*
   achievable for tier 2 without freezing the pattern language. A rejection whose predicate
   differs between SDKs means the same config loads in Python and fails in Rust, which is
   the cross-language split #111 was opened about. A *finding* whose set differs is only a
   missed diagnostic, and tier 2 touches no decision, so the divergence is bounded and
   harmless. §6.1.3's own sentence covers it: *"This is diagnostics, not enforcement."*

Tier 2 is therefore stated as a **criterion plus a MUST-detect minimum**, explicitly
non-exhaustive — the shape §6.1.1 took after v1.25.0, and for the same reason.

### Consequence table

| Shape | v1.30.0 | v1.31.0 |
|---|---|---|
| `[]`, `["$or"]`, `["$not"]` | matcher returns false; rule inert; 0 findings | **T1** `ACLRuleError` at every door; UNEVALUABLE by mutation |
| `["$not", p1, p2, …]` | consults `p1`, drops the rest | **T1** `ACLRuleError`; UNEVALUABLE by mutation |
| `[""]`, `["$or", ""]` | matches only the empty module ID | **T1** `ACLRuleError` (schema already says `minLength: 1`) |
| `["a", "$or"]`, `["$or", "$not", "a"]` | dead element, and it *matches* the literal `$or`/`$not`, violating §6.2.1 | **T1** `ACLRuleError`; the MUST NOT becomes unreachable |
| `["$not", "*"]`, `["$not", "**"]` | never matches; 0 findings | **T2** loads; `validate_rules()` reports; decision unchanged |
| `["@external"]` in `targets` | never matches a legal module ID; 0 findings | **T2** loads; `validate_rules()` reports; decision unchanged |
| `["$or", p1]`, `["api.*", "cli.*"]`, `["$not", "cli.*"]` | matches | accepted, no finding |

### Out of scope, and worth its own issue

`targets: ["@system"]` is the **opposite** fail-open and this change does not address it.
`@system` is evaluated against the *caller's identity*, not against the value being matched,
so as a **target** pattern it returns true for every target whenever the caller's identity
type is `system`. Measured in apcore-python: one `allow` rule with
`callers: ["*"], targets: ["@system"]` and a `system` identity granted `a`, `api.x`,
`executor.email.send` and `cli.rm` alike — an operator who wrote "system modules" got "every
module". It is well-formed at both tiers, it is a matches-**everything** defect rather than a
matches-nothing one, and folding it in would take #112 past what it is about. It should be
filed separately.

## The normative patch

### Patch 1 — replace §6.2.1 in full

Current text is at `docs/spec/protocol-spec.md` §6.2.1 (four-row table plus the
reserved-token paragraph). Replace with:

```markdown
### 6.2.1 Compound Operators in Pattern Arrays

The `callers` and `targets` pattern arrays **MAY** use the compound operators `$or` and
`$not` as the **first element** to alter the default OR-of-patterns semantics.

| Form | Operands | Semantics |
|---|---|---|
| `[p1, p2, ...]` | at least 1 | The default. **MUST** match the module ID if any of `p1, p2, …` matches. |
| `["$or", p1, p2, ...]` | at least 1 | **MUST** match the module ID if any of `p1, p2, …` matches. Observably equivalent to a flat list (which is also OR-ed) but documents intent explicitly. |
| `["$not", p]` | exactly 1 | **MUST** match the module ID if `p` does **not** match. |

**A pattern array is FLAT. The operators do not nest and there is no precedence
(v1.31.0, #112).** An operand is always a plain pattern string, never a nested array and
never another operator. There is exactly one operator position — index 0 — and everything
after it is an operand. `$or` and `$not` therefore have **two different grammars** in this
specification, and only one of them nests:

| | in `conditions` (§6.1.1) | in a pattern array (this section) |
|---|---|---|
| operand | a condition **object** | a pattern **string** |
| nesting | arbitrary — `$or[1].$not.k` is a defined path (§6.1.4) | **none** |
| how many operators per expression | any number, at any depth | exactly one, at index 0 |

A reserved token at any index other than 0 is therefore **not** a nested operator and
**not** a usable pattern: `["$or", "$not", "a"]` is not "or-of-not", and
`["a", "$not", "b"]` is not "a, but not b". Both are rejected — see the closure below.
Through v1.30.0 this section instead required such a token to be *"treated as a literal
pattern string"* while also requiring that a literal module ID equal to `"$or"` or `"$not"`
**MUST NOT** be matched, which is a pattern the specification guarantees can never match
anything: dead weight in a security policy, and the two clauses together were honoured by no
implementation. Rejecting the token outside index 0 replaces both with one structural rule
and makes the reserved-token guarantee hold by construction.

**Not every intent is expressible in one pattern array, and that is deliberate.**
`NOT (a OR b)` has no single-array form: `$not` takes exactly one operand and the array's
own combinator is OR, so there is no way to write "neither `a` nor `b`" as one field. Use a
glob when the excluded patterns share a prefix, and otherwise **first-match-wins with two
rules**, which is the idiom §6.3 already provides:

```yaml
rules:
  # "everything except executor.secrets.a and executor.secrets.b"
  - callers: ["*"]
    targets: ["$or", "executor.secrets.a", "executor.secrets.b"]
    effect: deny
    description: "Excluded targets, refused first"
  - callers: ["*"]
    targets: ["*"]
    effect: allow
    description: "Everything else"
default_effect: deny
```

!!! warning "The two-rule form is not a drop-in replacement inside an existing rule list"
    The two forms differ in what happens to the excluded calls. `["$not", p]` makes the
    rule **not match** `p`, so evaluation **continues** and a later rule may still decide
    it. A leading `deny` on `p` **ends** the scan for `p`. They agree only when nothing
    after the rule could have matched `p` and `default_effect` would have refused it
    anyway — which is true of the complete policy above and is **not** true in general.
    Inserting a leading `deny` into an existing rule list changes the decision for every
    call that a later rule was written to allow. Rewriting a rule into this form is a
    change to the policy's order, not a local substitution.

**Worked examples.** Every legal form, and every form that looks legal and is not:

```yaml
# ---- legal ----
targets: ["executor.*"]                       # one pattern
targets: ["api.*", "worker.*"]                # OR, implicitly
targets: ["$or", "api.*", "worker.*"]         # OR, explicitly — same meaning, states intent
targets: ["$or", "api.*"]                     # one operand under $or is legal, if pointless
targets: ["$not", "executor.secrets.*"]       # "anything that is not executor.secrets.*"

# ---- rejected: arity (§6.2.1 closure) ----
targets: []                                   # no operands — matches nothing, so the rule is no rule
targets: ["$or"]                              # OR over nothing
targets: ["$not"]                             # negation of nothing
targets: [""]                                 # the empty pattern matches no legal module ID
targets: ["$not", "a", "b"]                   # $not takes EXACTLY one operand.
                                              #   Before v1.31.0 this silently meant ["$not", "a"],
                                              #   so an `allow` rule GRANTED "b" — write two rules.

# ---- rejected: a reserved token outside index 0 (there is no nesting) ----
targets: ["$or", "$not", "a"]                 # NOT "or-of-not". Before v1.31.0 the "$not" was a
                                              #   literal pattern and the array matched "a" and a
                                              #   module literally named "$not".
targets: ["api.*", "$not", "cli.*"]           # NOT "api.* but not cli.*". No such form exists.

# ---- legal, but reported by validate_rules() as matching nothing (§6.2.1 tier 2) ----
targets: ["$not", "*"]                        # "not everything" is well-formed and matches nothing
```

**Arity is part of the form, and the set of arities is closed (v1.31.0, #112).** A pattern
array **MUST** carry at least one operand and every element **MUST** be a non-empty string:
`callers` and `targets` **MUST NOT** be empty, no element **MUST** be the empty string,
`$or` at index 0 **MUST** be followed by at least one pattern, `$not` at index 0 **MUST**
be followed by exactly one, and `$or` / `$not` **MUST NOT** appear at any index other
than 0. A rule violating any of these **MUST** be rejected
with `ACLRuleError`, naming the field (`callers` / `targets`) and the rule index wherever
the entry point has one — a rule under construction has no position yet, and an
implementation **MUST NOT** invent one (§6.1.5). It **MUST** be rejected at **every** entry
point that accepts a rule — file loading, direct construction, and runtime insertion — on
§6.1.6 rule 3's reasoning, which applies here unchanged. **Closing the doors is the
mechanism**, exactly as it is for the `effect` value set: `schemas/acl-config.schema.json`
has always declared `minItems: 1` on both fields and §6.1's field table has always required
a list of patterns, and nothing enforced either, because no implementation validates an ACL
file against the schema at load time.

!!! danger "A pattern array with no operands is not a narrow rule; it is no rule"
    Through v1.30.0 all three implementations returned `false` from the matcher for `[]`,
    `["$or"]` and `["$not"]`, reading an arity fault as a scope decision. The rule was
    then inert: with one rule in the ACL, the decision tracked `default_effect` exactly
    across all twelve combinations of the three shapes, both effects and both defaults,
    and `validate_rules()` reported nothing in any of them. On an `allow` rule that is
    merely useless. On a `deny` rule under `default_effect: allow` it is a **fail-open**:
    the call the operator wrote the rule to block is permitted, by a rule that loaded
    without error and a validator that called it clean.

    Reached from a YAML file, not only from direct construction. `ACL.load` rejects an
    **omitted** `callers` / `targets` and permits an **empty** one; combined with the
    matcher, `targets: []` under `effect: deny` produced a rule that loads clean and does
    nothing.

    This section's previous form called `["$not"]` "fail-closed". That is true of an
    `allow` rule and false of a `deny` one, and the label predates §6.1.1 (v1.22.0)
    naming the asymmetry: which direction "does not match" points is decided by the
    rule's `effect`, so no single non-match can be called fail-closed. The **MUST** it
    carried — "MUST evaluate to false" — is replaced rather than reinterpreted.

**`$not` takes exactly one operand.** Through v1.30.0 this section made
`["$not", p1, p2, …]` *implementation-defined*: SDKs **MUST** consult `p1` and **MAY**
ignore subsequent patterns, with authors told they **SHOULD NOT** rely on the form. All
three implementations consult `p1` and drop the rest, so the form is consistent across
implementations and consistently **wider than written**. `targets: ["$not", "secrets.a",
"secrets.b"]` on an `allow` rule reads as "anything but `secrets.a`", and `secrets.b` — the
second target the operator excluded — is **granted**; measured in apcore-python at 0.28.
An under-specified form whose only observable behaviour is a silent privilege escalation is
not a form, and `SHOULD NOT rely on this` is not a guard: nothing reported it and nothing
rejected it. A future version **MAY** define the multi-operand form as `NOT (p1 OR p2 …)`,
which is the reading an operator writing it already has; rejecting it now is what keeps
that option open, because nothing can come to depend on the present reading in the
meantime.

**A well-formed array that can still match nothing is reported, not rejected (v1.31.0,
#112).** Closing the arities above does not exhaust the shapes that make a rule inert, and
an implementation **MUST NOT** read the closure as though it did. A pattern array that is
well-formed under every rule above and that **matches no legal module ID for any input** is
a rule that protects nothing, and `validate_rules()` (§6.1.2) **MUST** report it — with the
same finding shape as a structural fault: path `callers` / `targets`, a **null** key, and
both resolvability flags `false`. It **MUST NOT** be rejected and **MUST NOT** change any
access decision.

The criterion is normative and the list is a **minimum**, not a closed set — the mistake
§6.1.1 corrected in v1.25.0 was enumerating where it should have stated a principle. Every
implementation **MUST** detect at least:

- `["$not", p]` where `p` matches every module ID — `*`, `**`, or any pattern consisting
  only of wildcards. `!true` is false for every input, so the rule fires for nothing.
- `["@external"]` as a **`targets`** pattern. `@external` is the caller-side sentinel §6.5
  substitutes for a null `caller_id`; no module ID is `@external`, so as a target pattern it
  matches nothing. It remains entirely legal in `callers`, which is what it is for.

The array is judged **as a whole**, never element by element. The criterion is that *the
array* matches no legal module ID, so a flat or `$or` array is reported only when **every**
operand is unmatchable: `targets: ["@external"]` is reported and
`targets: ["api.*", "@external"]` is **not**, because `api.*` still matches. The MUST-detect
list above names whole-array shapes and would otherwise be readable as "report any
occurrence of the token", which would report a rule that works.

An implementation **MAY** report further shapes it can prove match nothing. Divergence in
this finding set between implementations is **acceptable and expected**, and is the reason
this is a validator finding rather than a rejection: §6.1.4's determinism guarantee binds
precheck-origin diagnostics because they feed `handler_error` and the decision, and tier-2
findings feed neither. A rejection whose predicate differed between SDKs would mean the same
ACL file loads in one language and fails in another, which is the cross-language split
§6.1.5 exists to prevent. §6.1.3's sentence governs: *this is diagnostics, not enforcement.*

**The backstop, for the route no door covers.** A value that arrives outside the entry
points — assigning `callers` or `targets` on an already-constructed rule, which every
implementation's rule type permits and no constructor can intercept — is outside what a
rejection reaches. Unlike an unrecognised `effect`, which once the doors are closed is
never read again, a mutated pattern array **is** read: the matcher consults it on the next
`check()`. A pattern array that reaches evaluation violating any clause of the closure above
is therefore a **precheck fault** under §6.1.4.1, on the same terms as a malformed type:
the rule's scope is unreadable, the rule is UNEVALUABLE, and §6.1.1's effect table decides
— a `deny` rule takes effect and denies, an `allow` rule does not match and **MUST NOT**
grant. `validate_rules()` (§6.1.2) **MUST** report it as §6.1.3 rule 3's keyless structural
fault: path `callers` / `targets`, a **null** key, and both resolvability flags `false`.

An arity fault is a malformed pattern field like any other, with no partially-readable
tier. In particular §6.1.1 rule 5's "unknowable scope counts as scope" applies unchanged:
a rule carrying `approval: required` whose pattern field is malformed **MUST** raise the
pending requirement. `targets: []` is legible as an empty scope in a way `targets: 3` is
not, and an implementation **MUST NOT** act on that difference — deciding per fault kind
whether a field is "readable enough" is the per-implementation judgement call that
produced three different answers in #100, and the direction it would resolve is toward
asking a human less often.
```

### Patch 2 — §6.1 field table

`callers` and `targets` rows currently read `list[string]`. Change the Description column
to name the arity, so the table that a reader reaches first is not the one that omits it:

```markdown
| `callers` | **MUST** | `list[string]` | Caller patterns, at least one (OR logic: any match is sufficient). Arity is closed — §6.2.1. |
| `targets` | **MUST** | `list[string]` | Target patterns, at least one (OR logic: any match is sufficient). Arity is closed — §6.2.1. |
```

### Patch 3 — §6.1.4.1 heading and scope sentence

The section is titled "Malformed `callers` / `targets`" and its first sentence scopes the
precheck to the field's *type*:

> `callers` and `targets` are **lists of patterns**. A value that is not a list of strings
> is a malformed rule, and the precheck **MUST** classify it as unevaluable […]

Extend to arity, and record why the type fault needs no door-closing while the arity fault
does:

```markdown
`callers` and `targets` are **lists of patterns**, and both the element type and the
**arity** are constrained (§6.2.1). A value that is not a list of strings, or a list whose
arity is outside §6.2.1's table, is a malformed rule, and the precheck **MUST** classify it
as unevaluable — resolving per §6.1.1's effect table, so an `allow` rule does not grant and
a `deny` rule takes effect. It **MUST NOT** raise out of `check()`, and it **MUST NOT** be
treated as a pattern set.
```

and, after the existing paragraph ending "…satisfies this clause by construction and needs
no runtime check":

```markdown
The **arity** half is not disposed of the same way. `ACL.load` deliberately permits an
empty `callers` / `targets` — only omission is rejected — so a YAML file reaches it, and a
`Vec<String>` constrains the element type while placing no constraint on length, so no
implementation is exempt by construction. §6.2.1 therefore closes every entry point against
it, and what remains for this precheck is the value assigned onto an already-constructed
rule, which no constructor intercepts and the matcher still reads.
```

### Patch 4 — §6.5 edge-case table

The row at `docs/spec/protocol-spec.md` §6.5 currently states the removed behaviour as a
**MUST**, immediately above the row that already routes the type fault to §6.1.4.1. Leaving
it would make the specification contradict itself in two sections. Replace:

```markdown
| `callers` or `targets` in rule is an empty **list** | Rule never matches | **MUST** |
```

with:

```markdown
| `callers` or `targets` arity is outside §6.2.1 — empty, `$or` with no operands, `$not` with none or more than one | Rejected with `ACLRuleError` at every entry point (§6.2.1); unevaluable → §6.1.4.1 if assigned onto a constructed rule | **MUST** |
```

The row stays adjacent to the type row it now mirrors, and both point at §6.1.4.1 for the
same reason. The `rules` is empty → use `default_effect` row above is **unchanged**: an ACL
with no rules is a legitimate configuration, and only a *rule* with no patterns is not.

### Patch 5 — §6.1.5, one cross-reference sentence

§6.1.5 is where "the mechanism is closing every door" is argued. Append to the paragraph
that begins "**The `effect` value set is closed too, at every entry point (v1.30.0,
#111).**":

```markdown
§6.2.1 closes a third door on the same reasoning in v1.31.0: a pattern array's **arity**.
The three instances are one pattern — an unknown rule **key** dropped in silence (#107), a
legal key's **value** dropped in silence (#111), and a legal value's **arity** read as a
scope decision (#112) — and in all three the constraint was already declared in
`schemas/acl-config.schema.json` and enforced by no entry point, because no implementation
validates an ACL file against the schema at load time.
```

### Patch 6 — version history row

Append to the table at the end of `docs/spec/protocol-spec.md`:

```markdown
| 1.31.0 | <release date> | **§6.2.1 — a pattern array with no operands was read as a scope decision, and made the rule inert (#112).** `callers` / `targets` of `[]`, `["$or"]` or `["$not"]` can never match, and all three SDKs agreed on returning `false` from the matcher, so the rule contributed nothing: with one rule in the ACL the decision tracked `default_effect` exactly across all twelve combinations of the three shapes, both effects and both defaults, and `validate_rules()` reported nothing in any of them. On a `deny` rule under `default_effect: allow` that is a **fail-open** — the call the operator wrote the rule to block is permitted, by a rule that loaded without error and a validator that called it clean — and it is reachable from a plain YAML file, because `ACL.load` rejects an omitted `callers` / `targets` and permits an empty one. Arity is now **closed at every entry point** on §6.1.5's mechanism: at least one operand, `$or` at least one pattern, `$not` exactly one, rejected with `ACLRuleError` at file loading, direct construction and runtime insertion alike. `schemas/acl-config.schema.json` has declared `minItems: 1` on both fields since the file existed and nothing enforced it — the third instance of #107's and #111's shape, in which the constraint was declared in the schema and no door enforced it, because no implementation validates an ACL file against the schema at load time. **Three MUSTs are replaced, not reinterpreted.** §6.5's edge-case table required an empty list to make the rule "never match", which is the behaviour this entry describes as a fail-open, stated one row above the row that already routes the *type* fault to §6.1.4.1. §6.2.1 required `["$not"]` (no pattern) to "evaluate to false (fail-closed)"; the parenthetical predates §6.1.1 (v1.22.0) and is wrong — a non-match is fail-closed on an `allow` rule and fail-**open** on a `deny` one. And `["$not", p1, p2, …]` was *implementation-defined*, consult `p1` and ignore the rest: all three SDKs do exactly that, so the form was uniform across implementations and uniformly **wider than written**, granting `secrets.b` from `targets: ["$not", "secrets.a", "secrets.b"]` on an `allow` rule — a silent privilege escalation from a form the specification blessed with `SHOULD NOT rely on this`. A future version MAY define the multi-operand form as `NOT (p1 OR p2 …)`; rejecting it now is what keeps that option open. **A pattern array is also stated to be FLAT** — the operators do not nest and there is no precedence — which this section never said, while `$or` / `$not` nest arbitrarily in `conditions`: an operator who learned the condition grammar and wrote `["$or", "$not", "a"]` got an OR of two literals that matched `a` and also matched a module literally named `$not`, violating this section's own reserved-token **MUST NOT**, which no implementation honoured. A reserved token outside index 0 is now rejected, which makes that clause hold by construction, and the section gains the worked examples it never had. **A second, validator-only tier is added**, because closing the arities does not exhaust the inert class: `["$not", "*"]` has legal arity, exactly one operand, and matches nothing, producing the identical fail-open. Such an array loads, is reported by `validate_rules()`, and changes no decision — stated as a criterion with a MUST-detect minimum rather than an enumeration, because its predicate cannot be closed without freezing the pattern language, and an incomplete predicate at a door would mean the same ACL file loads in one language and fails in another. §6.1.4.1 is extended from the field's type to its type **and** shape, as the backstop for the one route no door covers — assigning the field on an already-constructed rule, which no constructor can intercept and which, unlike an unrecognised `effect`, the matcher still reads. **This IS an SDK change** in all three, and a **breaking** one for any deployment currently carrying one of these shapes — which is exactly the population that believes it has a rule and does not. Governance: maintainer approval per GOVERNANCE.md § Decision Making; tracking issue #112. |
```

## The schema patch

`schemas/acl-config.schema.json`. Add a `$defs.PatternArray` and point both fields at it.
`$ref` alongside sibling keywords is legal in Draft 2020-12, so each field keeps its own
`description` and `examples`.

```json
"PatternArray": {
  "type": "array",
  "description": "A `callers` or `targets` pattern array. The array is FLAT — the operators do not nest, there is no precedence, and an operand is always a plain pattern string. Shape is CLOSED (PROTOCOL_SPEC §6.2.1): at least one non-empty element; `$or` at index 0 followed by at least one pattern; `$not` at index 0 followed by exactly one; and `$or` / `$not` nowhere but index 0, because a reserved token elsewhere is a pattern the specification guarantees can never match. A shape that can never match makes the rule inert — and under `default_effect: allow` an inert `deny` rule permits the call it was written to block (#112).",
  "minItems": 1,
  "prefixItems": [{ "type": "string", "minLength": 1, "maxLength": 192 }],
  "items": {
    "type": "string",
    "minLength": 1,
    "maxLength": 192,
    "not": { "enum": ["$or", "$not"] }
  },
  "allOf": [
    {
      "if": { "type": "array", "minItems": 1, "prefixItems": [{ "const": "$or" }] },
      "then": { "minItems": 2 }
    },
    {
      "if": { "type": "array", "minItems": 1, "prefixItems": [{ "const": "$not" }] },
      "then": { "minItems": 2, "maxItems": 2 }
    }
  ]
}
```

`prefixItems` constrains index 0 only; in Draft 2020-12 `items` then applies to every
*remaining* index, which is exactly the asymmetry the reserved-token rule needs — a token at
index 0 is an operator and legal, the same token anywhere else is rejected. Verified against
a Draft 2020-12 validator on 17 shapes, including that `["$not", "*"]` stays **valid**: it
is a tier-2 semantic finding and the schema must not reject it.

Then:

```json
"callers": {
  "$ref": "#/$defs/PatternArray",
  "description": "<unchanged>",
  "examples": [ ... ]
},
"targets": {
  "$ref": "#/$defs/PatternArray",
  "description": "<unchanged>",
  "examples": [ ... ]
}
```

`minItems: 1` inside each `if` is load-bearing: `prefixItems` alone is vacuously satisfied
by an empty array, which would make the `$or` branch fire on `[]` and produce a confusing
second error alongside the outer `minItems` failure.

The `maxLength: 192` and `minLength: 1` bounds move from the old inline `items` onto both
`prefixItems[0]` and `items` so index 0 keeps them. Dropping them from `prefixItems` would
silently stop bounding the first pattern of every rule in the repository.

Per CLAUDE.md § Modifying a JSON Schema: `$schema` stays Draft 2020-12, every `property`
keeps a `description`, and both new `$ref`s resolve within the file.

## Conformance

**Fixture — `conformance/fixtures/acl_pattern_arity.json`, 41 cases.** Staged at
`staged-fixtures/acl_pattern_arity.json` in this directory; do not move it until all three
drivers have landed (see **Landing order**).

Two case shapes, discriminated by an explicit `kind`, because the change has two layers and
they are asserted differently:

- `kind: "closure"` (32 cases: 19 rejections, 9 controls, 4 tier-2) — `rule`, `entry_points`, `expected_load: ok | reject`.
  Modelled on `acl_effect_value_closure.json`, including its deliberate absence of a
  per-door expectation: a shape legal through one entry point and illegal through another
  *is* the defect, so the fixture must not be able to express it. Six controls guard
  against over-rejection, one of them (`token_lookalike_pattern_loads`) against the
  specific over-reach of matching reserved tokens by `$` prefix rather than by equality.
- `kind: "backstop"` (9 cases) — `rule` plus `mutate`, then a decision assertion. Modelled
  on `acl_handler_error.json`, reusing `expected_audit_handler_error_present`,
  `expected_handler_error_paths` and `expected_validation_finding_paths`.

Notable cases:

| Case | Pins |
|---|---|
| `empty_targets_on_deny_rule_under_default_allow_is_rejected` | #112's headline cell, now unreachable |
| `not_with_two_operands_is_rejected` | the second defect, at the door |
| `mutated_not_with_two_operands_on_allow_rule_does_not_grant` | the second defect at runtime — a driver answering `true` here has the pre-v1.31.0 matcher |
| `mutated_both_fields_report_both_paths` | §6.1.4 rule 3, no short-circuit, lexicographic path order |
| `mutated_empty_targets_on_approval_rule_raises_pending_requirement` | §6.1.1 rule 5's "unknowable scope counts as scope" |
| `well_formed_rule_raises_no_finding` | the control without which an implementation that flags every rule passes everything else |
| `token_lookalike_pattern_loads` | reserved-token detection is **equality**, not a `$` prefix — `$orders.*` must load |
| `*_in_callers_is_rejected` (6 cases) | field parity — §6.2.1 constrains both fields identically, and without these an implementation validating only `targets` passes nearly every reject case |
| `not_of_wildcard_does_not_change_the_decision` | tier 2 is a finding, never a denial |

**Both fields are exercised deliberately, and the mirror is complete.** All **eight**
structural shapes that are rejected on `targets` — empty array, `$or` with no operand,
`$not` with no operand, multi-operand `$not`, empty element, empty element under `$or`,
reserved token after an operator, reserved token in a flat list — are also rejected on
`callers`. Of the 19 rejection cases, 9 carry the fault on `callers` and 11 on `targets`
(one carries it on both). The `*_in_callers_is_rejected` mirrors exist only so that an
implementation which checks one field cannot pass; they carry their rationale by reference
to the `targets` twin rather than repeating it.

Two of the mirrors are not redundant with the plainer ones and are worth keeping distinct:
`empty_pattern_string_under_or_in_callers_is_rejected` fails only if the implementation
scans a `$or`'s **operands** on the `callers` side, and
`reserved_token_after_operator_in_callers_is_rejected` fails only if it keeps checking
positions **after** consuming a leading operator. An implementation can pass every other
mirror while failing each of those.

**No `skip_if_unrepresentable` on a `closure` case, and a driver MUST NOT add one.**
apcore-rust's `Vec<String>` makes §6.1.4.1's *type* fault unbuildable, which is why two
cases in `acl_handler_error.json` carry the flag; it places no constraint on *length* or
element content, so every closure shape is constructible in all three SDKs.

**The `backstop` cases are the exception, and the answer is still not a skip.** They need to
mutate a rule that is **already installed** in an ACL. apcore-python and apcore-typescript
have that route; **apcore-rust does not** — `ACL.rules` is a private field, `ACL::rules()`
returns `&[ACLRule]`, there is no `rules_mut`, and `ACL` derives nothing, so the rule's
fields are public and its shape is constructible but the mutated rule cannot be *installed*.
Verified in the source. The draft's premise that `pub` fields make the route reachable was
half right and the fixture inherited the error.

Such an SDK satisfies the backstop cases **by construction** and MUST assert that closure
rather than skip: the rule accessor hands back an immutable view with no mutable
counterpart, and offering the mutated rule to `add_rule` is rejected at the door by tier 1.
Reported as passing, never as skipped — "N skipped" and "N satisfied by construction" are
different claims and only the second is evidence. The backstop *behaviour* is still required
and is covered by that SDK's own in-crate tests; this constrains what a driver can reach,
not what the implementation must do. The cases carry `mutation_route: "installed_rule"` so a
driver can find them.

**Two existing cases in `acl_evaluation.json` must be amended in the same pass.**

```json
{"id": "empty_callers_matches_none", "rules": [{"callers": [], "targets": ["*"], "effect": "allow"}],
 "default_effect": "deny", "expected": false}
{"id": "empty_targets_matches_none", "rules": [{"callers": ["*"], "targets": [], "effect": "allow"}],
 "default_effect": "deny", "expected": false}
```

Both use `effect: allow` under `default_effect: deny`, so `expected: false` survives the
change — an unevaluable `allow` rule does not grant either. But the **construction** no
longer succeeds through the door the driver uses, and the descriptions ("Empty callers
array never matches any caller") state the reading this change removes. Options, in order
of preference:

1. **Delete both**, since `acl_pattern_arity.json` covers the shape at both layers and
   covers it better. `acl_evaluation.json` drops 21 → 19. **Do this with the spec (step 1b),
   not with the new fixture** — see **Landing order**.
2. Rewrite both as `expected_load: reject` cases — but that changes `acl_evaluation.json`'s
   case shape, which is otherwise uniform. Not preferred.

Option 1 is assumed by the counts below.

**Counts to update** (`conformance-integrity` checks all of these against the fixtures, and
the totals are an inline CI check outside `conformance/check_*.py`):

| Location | From | To |
|---|---|---|
| `docs/spec/conformance.md` §8.1 — `acl_evaluation` row | 21 | 19 |
| `docs/spec/conformance.md` §8.1 — new `acl_pattern_arity` row | — | 41 |
| `docs/spec/conformance.md` §8.1 — Total row | 762 / 67 fixtures | 801 / 68 fixtures |
| `docs/spec/conformance.md` prose ("67 cross-language fixture files … 762 test cases") | 67 / 762 | 68 / 801 |
| `conformance/README.md` fixtures table | — | one new row |

## Per-SDK implementation notes

The shared shape in all three: one arity predicate, called from the rule-validation path
that every entry point already funnels through, plus the same predicate wired into the
pattern precheck that feeds `validate_rules()` and `handler_error`.

### apcore-python

- `src/apcore/acl.py` — `_precheck_patterns` already loops both fields and already
  produces the keyless `_Fault` with the right path. Extend its per-field test from
  "is a list of strings" to "is a list of strings **and** the shape is legal" — arity,
  non-empty elements, and no reserved token outside index 0. Both fields are still examined
  without stopping at the first fault; the existing determinism comment covers it.
- Tier 2 is a **separate** predicate and must not be wired into `_precheck_patterns`: it
  feeds `validate_rules()` only and must never reach `handler_error` or the decision. The
  cleanest shape is a sibling `_never_matches(field, patterns)` consulted by
  `validate_rules` after the precheck reports clean.
- Rejection: the loader already raises `ACLRuleError` per field at `acl.py:1005` / `:1009`.
  Add the arity check beside each. `ACLRule.__post_init__` and `add_rule` need the same,
  following exactly how `_validate_effect` is threaded through all three doors for #111 —
  that threading is the template, and reusing it is what makes the closure per entry point
  rather than per code path.
- **Casualty:** `tests/test_acl_unevaluable_conditions.py::
  test_an_empty_list_is_well_formed_and_simply_never_matches` asserts the removed
  behaviour, including `handler_error is None`. It is replaced by two tests — the
  construction is rejected, and the mutated rule denies — not deleted silently. Its
  docstring ("§6.5 keeps this a plain non-match") is the reading this change reverses and
  should be quoted in the replacement's docstring so the reversal is legible to whoever
  reads it next.
- `test_both_fields_are_reported_without_short_circuiting` builds
  `ACLRule(callers="a", targets=5)` and is unaffected — that is the type fault, still
  reached only by mutation-equivalent construction with `# type: ignore`.

### apcore-typescript

- `src/acl.ts` — `precheckPatternField` already has the exact shape: it computes a
  `detail` string, and on `null` returns without a fault. Add the shape branches to the
  `else` arm after the element-type scan, so a single field yields at most one fault and
  the type fault keeps precedence (an array whose element 0 is not a string has no
  meaningful arity reading). Order the shape checks empty-array → empty-element →
  reserved-token-position → operator arity, so the message names the most basic fault.
- Tier 2 goes beside it as its own function, reachable only from `validateRules`.
- Rejection: `_parseAclRule` raises `ACLRuleError` per field at `acl.ts:611` / `:616`;
  the constructor and `addRule` follow the `effect`-closure threading from #111.
- `precheckPatternField` is called from two sites (`acl.ts:467` and `:1513`); both pick up
  the change without further edits.

### apcore-rust — the only genuinely new code

- There is **no pattern-field precheck in apcore-rust at all**. §6.1.4.1 is satisfied by
  construction there (`callers: Vec<String>`), and the source says so in four places
  (`src/acl.rs:1067`, `:1095`, `:1102`, `:1591`). Arity is not covered by that argument:
  `Vec<String>` constrains the element type and says nothing about length. A
  `precheck_patterns` equivalent has to be written, producing `RuleFault`s at paths
  `callers` / `targets` with a null key and both flags false, and wired into both
  `validate_rules` and the per-call handler-error scope
  (`crate::acl_handlers::report_condition_unevaluable_at`). Tier 2 is a second function
  wired into `validate_rules` **only** — apcore-rust is the SDK where conflating them is
  easiest, because there is no existing pattern-precheck seam to keep them apart.
- `match_patterns` (`src/acl.rs:1699`) keeps its `false` returns as defence in depth; the
  precheck runs first and the rule never reaches the matcher.
- Rejection: `ACL::validate_rule` is the single funnel `try_new`, `try_add_rule` and the
  loader already share for the `effect` and `approval` checks — add the arity check there
  and all three doors close at once. Note that `add_rule` **panics** by contract and
  `try_add_rule` is the fallible pairing, which the new rejection inherits; that pairing is
  already documented in `add_rule`'s `# Panics` section and needs one more sentence.
- Both `matches_rule` and `matches_rule_async` are separate code paths, as they were in
  #100. Check both.

### Downstream

- **`apexe`** — `never_matches` in `src/governance/acl.rs` becomes redundant for the three
  zero-operand shapes and can be deleted once apcore ships, which is what the issue
  anticipates. Its `$not`-polarity handling is *not* redundant and must stay: a `$not`
  operand that matches nothing registered makes the rule fire for **every** module, which
  is a different defect this change does not touch. `apexe`'s own tests construct the inert
  shapes (`src/governance/acl.rs:947`, `src/module/registry.rs:892`,
  `tests/mcp_integration.rs:630`) and will start receiving `ACLRuleError` from `ACL::load`
  instead of a clean load — they need updating in the same release train, not before.
- `apexe` is **not blocked** on this: its generated ACLs put every `deny` rule under
  `default_effect: deny`, so it never lands in one of the four dangerous cells.

## Breaking change and migration

**This breaks deployments, deliberately, and the CHANGELOG must say so under `### Changed`
rather than `### Fixed`.**

- A config carrying `callers: []`, `targets: []`, `["$or"]`, `["$not"]` or
  `["$not", p1, p2, …]` **stops loading**. Previously it loaded and the rule did nothing
  (or, in the last case, did less than written).
- The affected population is exactly the deployments that believe they have a rule and do
  not. There is no deployment for which the old behaviour was the intended one: a rule that
  can never match is not a policy decision anyone writes on purpose, and the multi-operand
  `$not` form is one the specification already told authors not to rely on.
- **Migration is mechanical for the empty shapes and is not for the multi-operand `$not`.**
  `targets: []` intending "everything" becomes `targets: ["*"]`; intending "nothing" means
  the rule should be deleted; `["$or"]` and `["$not"]` are the same two readings. Each is a
  local substitution and the error names the field.
- **`["$not", p1, p2]` needs a human, and the draft previously said otherwise.** If only
  `p1` was ever meant — which is what the rule has actually been doing since it was
  written — it becomes `["$not", p1]` and nothing else changes. If `NOT (p1 OR p2)` was
  meant, there is **no general equivalence transform.** `["$not", p]` makes the rule *not
  match* `p`, so evaluation **continues** to later rules; a leading `deny` on `p1`/`p2`
  **ends** the scan and refuses calls that a later rule may have been written to allow.
  The two agree only when the rule is effectively terminal for those calls — nothing after
  it could match `p1`/`p2`, and `default_effect` would have refused them anyway. The
  migration note **MUST** therefore say: rewrite by hand, against the rule's position and
  everything below it, and use the two-rule form only for a terminal fallback. Migration
  tooling **MUST NOT** apply it automatically.
- The error message **SHOULD** say which of the two the operator likely meant, because both
  readings of an empty list are plausible and the failure is at boot, where a good message
  is the whole remedy.

Semantic versioning: the specification takes a **minor** bump (1.30.0 → 1.31.0) on the same
reasoning v1.22.0 used when a `deny` rule with an unevaluable condition began denying, and
v1.30.0 used when an out-of-enum `effect` began being rejected at every door. The SDKs take
a minor bump each. `CHANGELOG.md` entry, under the next release's `### Changed`:

> **A pattern list with no operands made an ACL rule inert, and under `default_effect:
> allow` that permitted the call the rule named (spec v1.31.0,
> [#112](https://github.com/aiperceivable/apcore/issues/112)).** `callers` / `targets` of
> `[]`, `["$or"]` or `["$not"]` can never match; all three SDKs returned `false` from the
> matcher and `validate_rules()` reported nothing, so a `deny` rule an operator wrote,
> loaded and validated contributed nothing to the decision. Reachable from a plain YAML
> file — `ACL.load` rejects an *omitted* `callers` / `targets` and permits an *empty* one.
> Arity is now closed at every entry point (§6.2.1): at least one operand, `$or` at least
> one pattern, `$not` exactly one, rejected with `ACLRuleError` at file loading, direct
> construction and runtime insertion alike. `schemas/acl-config.schema.json` had declared
> `minItems: 1` on both fields since the file existed, enforced by nothing — the same shape
> as [#107](https://github.com/aiperceivable/apcore/issues/107) and
> [#111](https://github.com/aiperceivable/apcore/issues/111). Three normative statements are
> replaced: §6.5's edge-case table required an empty list to make the rule never match;
> `["$not"]` was required to evaluate to false and called "fail-closed", which it
> is only on an `allow` rule; and `["$not", p1, p2, …]` was implementation-defined —
> consult `p1`, drop the rest — which every SDK did, granting `secrets.b` from
> `targets: ["$not", "secrets.a", "secrets.b"]` on an `allow` rule. **A configuration
> carrying any of these shapes will stop loading**; the error names the field, and the
> migration is `["*"]` for "everything" and deletion for "nothing". The multi-operand
> `$not` is the one shape with **no mechanical migration**: `["$not", p1]` preserves what
> the rule has actually been doing, but if `NOT (p1 OR p2)` was intended, a leading `deny`
> is not equivalent — a non-matching rule lets evaluation continue to later rules and a
> `deny` ends it, so the rewrite has to be done by hand against the rule's position. §6.2.1 also states for the first time that a pattern
> array is **flat** — the operators do not nest and there is no precedence, unlike the same
> tokens in `conditions` — gains worked examples, and rejects `$or` / `$not` outside index 0,
> which makes its own long-unenforced "MUST NOT match a literal module ID equal to `$or`"
> hold by construction. A separate validator-only tier reports arrays that are well-formed and
> still match nothing (`["$not", "*"]`); those keep loading and change no decision.

## Landing order

The fixture reds CI in all three SDK repositories from the moment it enters
`conformance/fixtures/` until the last driver lands, so it stays staged until the end —
the sequence `acl-unevaluable-conditions` used, for the same reason.

1. **Spec** — Patches 1–6 plus the schema patch, on a branch, with #112 linked. Maintainer
   approval per GOVERNANCE.md before merge.
1b. **Delete the two superseded `acl_evaluation.json` cases, in the same pass as the spec** —
   `empty_callers_matches_none` and `empty_targets_matches_none`. **This step was originally
   placed at step 5 and that was wrong.** Those two cases assert a behaviour each SDK removes,
   so they red that SDK's conformance run the moment *its own* fix lands — not when the new
   fixture lands. Leaving them until step 5 forces all three SDKs to carry a transitional
   driver branch that step 5 then has to unwind; apcore-typescript hit exactly this and had to
   rewrite its driver to assert the rejection instead. A **deletion** is safe to land first,
   which is the constant-fixtures-before half of the usual rule: an SDK that has not yet
   landed the fix simply stops running two cases and still passes, and an SDK that has landed
   it passes too. Only the *new* fixture has to wait for the drivers.
2. **apcore-python** — arity check at all three doors, `_precheck_patterns` extension,
   the two replacement tests for the deleted one. SDK-local tests only;
   `conformance/fixtures/` untouched.
3. **apcore-typescript** — same.
4. **apcore-rust** — same, plus the new `precheck_patterns` and its wiring into
   `validate_rules` and the handler-error scope.
5. **Fixture** — move `staged-fixtures/acl_pattern_arity.json` into
   `conformance/fixtures/`, update `conformance/README.md` and every count in
   `docs/spec/conformance.md` §8.1, and point all three drivers at the 41 case IDs in one
   coordinated pass. The two superseded `acl_evaluation.json` cases are already gone at
   step 1b; if any SDK carried a transitional driver branch for them, unwind it here.
6. **`apexe`** — delete `never_matches`' three zero-operand branches, keep the `$not`
   polarity handling, update the three tests that construct the inert shapes.

Per the issue's own priority comment, this sequences **after apcore-rust #36 → #37 → #38**:
#38 asks for a recorded construction/extension policy on `ACLRule` after the source break
its `approval` field caused in 0.28, and landing another `ACLRule` semantics change before
that policy exists repeats what #38 is for. The fixture is the exception the comment itself
names — it pins the regression class independently of when the fix lands, and can be
authored now, which is what this draft does.

## Cross-references to update together

- `docs/spec/protocol-spec.md` — §6.1 field table (line 3627–3628), §6.1.4.1 (3823),
  §6.1.5 (3853), §6.2.1 (4004–4015), §6.5 edge-case table (4123), version history
- `schemas/acl-config.schema.json` — `$defs.PatternArray`, both field `$ref`s
- `docs/features/acl-system.md` — the §6.2.1 restatement at lines ~178–187 currently
  reproduces both replaced MUSTs verbatim, including "authors SHOULD therefore supply
  exactly one pattern after `$not` and treat additional patterns as undefined behavior".
  Line 178 already calls `$or` / `$not` "compound operators with **two distinct surface
  forms**" and stops short of the sentence that matters — that only one of the two forms
  nests — which is the sentence Patch 1 adds. This is a **feature doc**, so per CLAUDE.md
  § Cross-Language Examples the worked examples there must be Python / TypeScript / Rust
  tabbed sections, not the YAML block Patch 1 puts in the spec;
  `Contract: ACL.load` Errors (line ~276) and `Contract: ACL.add_rule` Errors (~426) both
  need the new `ACLRuleError` cause; `add_rule`'s Preconditions already say "callers +
  targets non-empty" and become enforced rather than aspirational
- `docs/spec/conformance.md` — §8.1 row, Total row, and the prose fixture/case counts
- `conformance/README.md` — fixtures table row
- `CHANGELOG.md` — `### Changed`, per the entry above
- `docs/guides/troubleshooting.md` — carries `$or` / `$not` guidance; check whether it
  shows any now-rejected form
- `docs/spec/design-context-annotations-acl.md` and `docs/glossary.md` — both mention the
  compound operators; verify neither restates a replaced MUST

## Open questions for the maintainer

1. **Mechanism.** This draft rejects at every entry point and keeps UNEVALUABLE as a
   backstop; #112 proposes UNEVALUABLE as the primary mechanism. The difference is a
   boot-time failure versus a `deny` rule that begins denying every call at runtime. If the
   issue's version is preferred, Patch 1's closure paragraph and the whole `kind:
   "closure"` half of the fixture come out, and the backstop paragraph becomes the rule.
2. **Whether tier 2 belongs in this change at all.** It is what makes the fix cover the
   *class* #112 names rather than three shapes, and `["$not", "*"]` is a legal-arity array
   that produces the identical fail-open. But it is the only part that needs reasoning about
   the match relation rather than the array's shape, and it is fully separable: dropping it
   removes one paragraph from Patch 1, one row from the schema `description`, and six
   fixture cases (`not_of_wildcard_*`, `external_sentinel_*`, `not_of_narrow_pattern_*`,
   `not_of_wildcard_does_not_change_the_decision`). Nothing else depends on it.
3. **Scope of the `$not` multi-operand fix.** A second defect found while verifying the
   first, separable the same way: dropping it means deleting one row from Patch 1, two
   `closure` cases and one `backstop` case. Keeping it means the form table is consistent;
   dropping it means shipping a table where one row still carries a documented fail-open.
4. **Rejecting a reserved token outside index 0.** This replaces two §6.2.1 clauses ("treat
   as a literal pattern string" plus "MUST NOT match a literal module ID equal to `$or`")
   with one structural rule, and it is the only way the second clause is actually honoured —
   no implementation honours it today. The cost is that a module legitimately named `$or`
   becomes unaddressable, which it already was by the clause being replaced.
5. **The two superseded `acl_evaluation.json` cases** — delete (assumed here) or rewrite.
6. **A fault message MUST NOT contain the sequence `"; "` — and this is not #112's.**
   §6.1.1 rule 2 makes `"; "` the separator between multiple `handler_error` entries, and
   drivers recover the reported paths by splitting on it, so a message containing `"; "`
   splits into two bogus paths and the failure reads as a short-circuit bug rather than a
   message-format one. Observed while implementing this in apcore-typescript, where two
   natural fault messages did exactly that. The specification leaves message wording to the
   SDK and has never stated the constraint. It belongs in §6.1.1 rule 2 beside the separator
   that creates it, and applies to every fault message, not only the ones this change adds.
7. **Error-message shape.** Whether the rejection is required to distinguish "empty array"
   from "operator with wrong arity". This draft requires the field and the rule index and
   otherwise leaves the wording to the SDK, matching how §6.1.4.1 treats the type fault —
   **except** for question 6's `"; "` constraint, which is not a matter of taste: wording is
   free, that one sequence is not. A stricter requirement would be pinnable in the fixture
   but is not today.
8. **Does `add_rule` re-validate a rule handed in pre-built?** The draft says reject at
   "runtime insertion" and then gives two templates that disagree. apcore-python was told to
   thread like `_validate_effect`, which reaches `add_rule` only *through construction*, so
   `r = ACLRule(...); r.targets = []; acl.add_rule(r)` inserts without raising and is caught
   later by the backstop. apcore-rust was told to use `ACL::validate_rule`, the funnel
   `try_new`, `try_add_rule` and the loader share — which *does* re-validate a pre-built
   rule. apcore-typescript wired `addRule` explicitly and re-validates. So two of three
   raise and one does not, and the fixture **cannot** express the difference: `entry_points`
   deliberately carries no per-door expectation.

    §6.1.5's v1.30.0 text says mutation-then-use is outside what the section can require and
    an implementation **MAY** guard it — which permits exactly this divergence. That was
    tolerable for `effect`, which is never read again once the doors are closed. It is not
    tolerable here, because the mutated array **is** read. **Recommendation: `add_rule`
    MUST re-validate the rule it is handed.** A rule arriving at an entry point is that
    entry point's to check, whatever its history; re-validating is total and cheap; and it
    is what two of the three SDKs already do. Taking it means apcore-python needs a
    follow-up — the check moves from `ACLRule.__post_init__` alone to `add_rule` as well.
9. **Validation order inside a rule type — and this one already diverged.** Not
   hypothetical: apcore-python put the pattern check **after** `effect` and `approval`, so
   `ACLRule(callers=[], targets=[], effect="Allow")` reports the *effect*; apcore-rust
   checks **shape first** and reports the patterns. Both are conformant because nothing
   pins it. **Recommendation: `effect` → `approval` → patterns**, extending #111's existing
   "effect before approval" sentence rather than inventing a new order, which means only
   apcore-rust moves. Pinnable in the fixture as a case that is bad on two axes at once;
   not currently pinned.
10. **`validate_rules()` finding order when a rule has both pattern and condition faults.**
    §6.1.2 rule 3 says "ordered by rule index, then lexicographically by path", so pattern
    faults **interleave** with condition faults — `$or[0].x` < `callers` < `roles` <
    `targets` — rather than being grouped first. apcore-rust read it that way and is right;
    it is worth stating explicitly, because grouping all pattern faults ahead of condition
    faults is the reading a sibling SDK would reach for and produces a different order for
    exactly the rules that have both.
11. **Can tier 1 and tier 2 both fire on one field?** No — **at most one finding per field,
    tier 1 winning**, because tier 2's predicate presumes a well-formed array. apcore-rust
    implemented it that way. Unstated, three SDKs can report different finding *counts* for
    one rule. Not pinnable through a door (a tier-1 rule never loads), so it has to be
    stated rather than tested.
12. Minor, apcore-rust only: the §6.1.1 rule 3 warning's tracing field is still named
    `condition_paths` while now carrying `callers` / `targets`. Nothing asserts on it and
    renaming a log field is cross-cutting, so it was left alone — but it is now misnamed.
