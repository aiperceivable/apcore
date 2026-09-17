---
description: "Maintainer decision log for the forty-eight cross-language divergences (D-74 – D-121) settled in spec v1.49.0, v1.50.0 and v1.51.0, found by the 2026-09-14 deep-chain audit of apcore-python, apcore-typescript and apcore-rust. Five of the second wave are security defects."
title: Deep-chain alignment decisions (D-74 – D-121)
date: 2026-09-15
status: resolved — 18/18 in v1.49.0 (wave 1), 16/16 in v1.50.0 (wave 2), 14/14 in v1.51.0 (wave 3, implementation deferred)
audience: maintainers + spec reviewers
source: apcore-skills:audit --scope core (2026-09-14), dimensions D10 (contract parity) and D11 (deep-chain parity)
---

# Deep-chain alignment — D-74 – D-91

These eighteen items came out of a call-graph diff across the three core SDKs, not
a signature diff. That distinction is the whole story of this batch: every one of
them passed shape-level parity — same method, same arity, same declared types —
and diverged only in what the code actually did. A contract the spec never stated
is a contract three implementations each answer separately, and the striking
result is that **none of the three was the outlier every time**. Python was alone
on four, TypeScript alone on five, Rust alone on seven, and on two the three
agreed with each other while the spec text was wrong.

Decision template per item: **Status quo** / **Decision** / **Authority**.

---

## D-74 — `Config.get("")` is not an error

**Status quo.** The `Config.get` Inputs row said an empty key "is rejected with
`ValueError`/`ConfigInvalidError`". The same block's `### Errors` row said "No
errors raised under normal operation". Verified at runtime: `Config().get('')`
returns `None` in Python, `defaultValue` in TypeScript, `None` in Rust — no SDK
has ever rejected it, and Rust's `get` has no error channel at all.

**Decision.** Delete the clause. An empty key resolves no value and returns the
default, like any other absent key.

**Authority.** The three implementations. The spec text was self-contradictory and
described behaviour that never existed; a conformance case written from it would
have failed on all three.

---

## D-75 — `Executor.call` must enforce the module-ID length bound at entry

**Status quo.** `core-executor.md` "Contract: Executor.call" says: "Empty /
over-length / malformed IDs MUST be rejected before the pipeline context is
constructed." All three call a pattern-and-empty guard only; `MAX_MODULE_ID_LENGTH
= 192` exists in all three but is enforced only in the registry, one step later.
A 300-character well-formed ID therefore builds a `PipelineContext` and returns
`MODULE_NOT_FOUND` instead of the entry guard's `INVALID_MODULE_ID`.

**Decision.** No spec change — the requirement is already unambiguous. All three
SDKs add the length bound to the entry guard.

**Authority.** The existing MUST. Honouring it costs one comparison; weakening it
would remove a stated bound for no gain.

---

## D-76 — `ContextFactory.create_context(request)`

**Status quo.** The Contract declared `(identity, caller_id, data)`. apcore-python
(`context.py:406`) and apcore-typescript (`context.ts:447`) take a single opaque
`request`; apcore-rust takes `(identity, services)`, is `#[async_trait]` and
fallible, and carries two extra required trait members. Rust's own doc comment
states that "`create_context` is the canonical method name defined in the apcore
protocol spec (`ContextFactory.create_context(request)`)" while its signature
takes no request.

**Decision.** Rewrite the Contract to `create_context(request) -> Context`.
Rust's async/fallible declaration is permitted as a language-ecosystem allowance;
the parameter is not.

**Authority.** Python and TypeScript, and the interface's own purpose. Both
docstrings state it: the factory exists so a web-framework integration can extract
an identity *from a request*. A signature that already receives an `Identity` has
had that work done for it and has nothing left to do.

---

## D-77 — `Registry.describe` returns a string; a structured override falls through

**Status quo.** §12.2 declared `describe(module_id) → ModuleDescription`. All
three SDKs return a string (`-> str`, `: string`, `-> Result<String, _>`), so the
declared type matched none of them. For a module supplying its own `describe()` —
whose declared return is an introspection mapping — the three diverged: Python
returned `str(dict)` (a Python repr), TypeScript returned the raw object through a
method it types as `string`, and Rust returned the override only when it was a
string and otherwise generated the envelope.

**Decision.** §12.2 corrected to `→ String`. The module override is returned only
when it is itself a string; otherwise the generated envelope. No stringifying a
mapping, and no returning a mapping through a string-typed interface.

**Authority.** Rust, and the type system. `get_definition` is already the
structured accessor, so `describe` returning a structure would duplicate it; and
`str(dict)` is a language-specific repr, not a description.

---

## D-78 — `ExtensionManager.apply` must not drain the store

**Status quo.** No Postconditions section existed. Rust consumed its registrations
(`take_single`, `std::mem::take`); Python and TypeScript read through
non-consuming accessors. Applying one manager to two executors wires both on
Py/TS and only the first on Rust, silently.

**Decision.** Add Postconditions: `apply` retains the store.

**Authority.** Python and TypeScript — and, decisively, the block's own existing
`idempotent: false` row, which promises that calling `apply` twice *stacks*
middleware. Only a non-consuming implementation can produce that observable, so
the spec had already chosen; it just had not said so where an implementer would
look.

---

## D-79 — a stalled topological sort is not a cycle

**Status quo.** When Kahn's algorithm terminates with nodes remaining, there are
two causes. Python searched for a back edge and raised `MODULE_LOAD_ERROR` naming
the blocked modules when it found none. TypeScript and Rust raised
`CIRCULAR_DEPENDENCY` unconditionally, synthesising `cycle_path` from the leftover
set — for the non-cycle case, a one-element "cycle".

**Decision.** New row in §5.15.2: a stall with no cycle raises `MODULE_LOAD_ERROR`
and MUST NOT report `CIRCULAR_DEPENDENCY` or a fabricated `cycle_path`.

**Authority.** Python. The two causes need opposite fixes — break an edge, versus
add a missing module to the batch — and reporting a loop that does not exist sends
the author looking for something that is not there.

---

## D-80 — the registry event set is closed, and `file_changed` is in it

**Status quo.** All three emit exactly `register` and `unregister`. Python and
TypeScript validate `on()` against exactly those two; Rust accepts any name and
returns a valid handle, so a typo is a permanently silent subscription.
Meanwhile apcore-typescript's `watch()` emits `file_changed`
(`registry.ts:1697`) — a name its own `on()` rejects (`registry.ts:1548-1553`) —
so every hot-reload notification fired into an empty callback list. And
`registry-system.md`'s own example told readers to call `on("change")`,
`on("add")`, `on("remove")`: three names every SDK rejects.

**Decision.** The set is `register`, `unregister`, and a conditional
`file_changed` (emitted only by implementations whose `watch()` is notify-only).
`on`/`off` MUST reject anything outside it, and MUST accept everything the same
implementation can emit. Doc example corrected.

**Authority.** Observed emissions, plus the rule that an implementation may not
emit what it refuses to let you hear.

---

## D-81 — `TaskStoreError` must reach the caller

**Status quo.** The spec declares `TaskStoreError(code=TASK_STORE_UNAVAILABLE)` on
every `TaskStore` method. apcore-rust's manager discards all of them
(`.ok().flatten()`, `unwrap_or_default()`, `let _ = save(...)`), so a store outage
reports "task not found" / "no tasks" — and `cancel()` returns `true` after a save
that never landed. Python raises and TypeScript rejects.

**Decision.** Every manager method that touches the store propagates the error. An
implementation whose signature has no error channel MUST be given one (breaking
change for apcore-rust).

**Authority.** The spec's own declaration. Declaring an error on the store and
dropping it in the manager means no caller can ever catch it.

---

## D-82 — `list_tasks` insertion order is normative

**Status quo.** The spec said "insertion order (Python dict / JavaScript Map)",
which read as an implementation note. Rust's `InMemoryTaskStore` sorts by
`task_id` — a UUID, hence random with respect to submission.

**Decision.** Insertion order is the requirement. An implementation whose backing
map lacks it MUST carry a monotonic counter; sorting on `task_id` does not satisfy it.

**Authority.** The spec, de-parenthesised. `list_tasks()[0]` meaning
"first submitted" is the only reading a caller can port.

---

## D-83 — the published `guard_call_chain` signature is normative

**Status quo.** The spec's Rust tab publishes
`guard_call_chain(module_id, call_chain, max_call_depth, max_module_repeat)`. The
shipped crate had `guard_call_chain(ctx: &Context<serde_json::Value>, module_name,
max_depth)` — so the published code did not compile. Because `Context<T>` is
generic, a host whose services type was anything else could not call the guard at
all, and ran nested calls with no depth, cycle or frequency enforcement.
`DEFAULT_MAX_CALL_DEPTH` was a `pub const` the guard never consulted.

**Decision.** A chain-taking, generic-free entry point is required. A
`Context`-taking wrapper over it is permitted and is what the pipeline step uses.

**Authority.** The spec, and reachability. The A20 algorithm itself was verified
byte-for-byte identical across all three — only the door was missing.

---

## D-84 — a non-positive call-chain limit raises a typed error

**Status quo.** Python raised builtin `ValueError`, TypeScript a bare `Error`,
Rust `ModuleError(GENERAL_INVALID_INPUT)`. Neither of the first two is catchable
as an apcore error; a misconfigured `executor.max_call_depth: 0` surfaced through
Python's preflight as `error.code == "ValueError"`, which is not a code in the
registry.

**Decision.** All three raise the typed error with `GENERAL_INVALID_INPUT`. New
`### Errors` row.

**Authority.** Rust. It is the only one a cross-language caller can catch, and the
only one carrying a registry code.

---

## D-85 — malformed version constraints must be reportable

**Status quo.** Rust's `check_single_constraint` had no operand validation:
`"latest"` matched no operator prefix, fell to `("=", "latest")`, parsed to
`(0,0,0)` via a leading-numeric-prefix reader, and compared majors only — so it
returned **true** for every `0.x.y` module and the constraint was never enforced.
The mirror case, `"v1.0.0"` against an actual `1.0.0`, reported a mismatch. Python
and TypeScript both raise `VersionConstraintError`; Python's `_CONSTRAINT_RE`
comment names this exact failure mode as the reason it exists.

**Decision.** Operands MUST start with a digit. A fallible form reporting
`VERSION_CONSTRAINT_INVALID` is required; a non-fallible convenience form MAY
exist but MUST fail closed and warn.

**Authority.** Python and TypeScript.

---

## D-86 — `Registry.register` validation order

**Status quo.** The Side Effects list named neither the structure check nor the
custom validator. Python ran validator → streaming; TypeScript ran duplicate →
validator; Rust ran streaming → validator → duplicate. A module that was
malformed, validator-rejected and duplicate at once reported three different
errors.

**Decision.** `module_id` → structure/streaming → custom validator → duplicate.

**Authority.** Rust's order, as the intrinsic-then-extrinsic one: what is wrong
with the module is reported before what is wrong about where it is being put,
because the author must fix the module either way.

---

## D-87 — the ACL audit-block surface is a language idiom

**Status quo.** Python and TypeScript accept the `audit:` block as an optional
constructor parameter. Rust's constructors are fixed-arity, so the field was
reachable only through `load()`/`reload()`; a programmatically-built Rust ACL
could not be given one.

**Decision.** The capability must be reachable; its shape is unconstrained.
A constructor parameter and a builder/setter are equally conforming. The audit
sink must be rebuilt when it is supplied.

**Authority.** Neither — this is the one item where the divergence is genuinely
idiomatic and forcing uniformity would make one language worse.

---

## D-88 — index-keyed warning dedupe must be cleared on index shift

**Status quo.** The §6.5 "conditions present but no context" warning is deduped
per rule index. `add_rule` inserts at index 0, shifting every rule. TypeScript
cleared the dedupe set in both `addRule` and `reload`; Python cleared it in
`reload` only; Rust has no dedupe state and warns unconditionally.

**Decision.** Any operation that inserts, removes or reorders rules MUST clear the
set. Not deduping at all remains conforming; deduping by index without clearing
does not.

**Authority.** TypeScript. A stale entry suppresses the warning for a different
rule than the one it was recorded for — and specifically for the rule the operator
just added.

---

## D-89 — deprecation warning cadence

**Status quo.** Python logged on every `get_definition`; TypeScript once per
`(module_id, version)` per registry; Rust did not derive `sunset_date` at all.

**Decision.** At most once per `(module_id, version)` per registry instance.

**Authority.** TypeScript. `get_definition` is a read that hosts call in loops, so
per-read warning is spam proportional to traffic — which is how operators learn to
filter it out.

---

## D-90 — `reset()` must not substitute the cancellation handle

**Status quo.** Python and Rust reset their flag in place. TypeScript installed a
fresh `AbortController`, so a consumer holding the previous `signal` was
permanently detached and never observed a later `cancel()` — breaking exactly the
real-abort channel D-18 makes normative for TypeScript. Invisible to cooperative
checkers, since `isCancelled` and `check()` read the current controller.

**Decision.** Keep one handle for the token's lifetime and treat the cooperative
flag as authoritative. Where the platform cannot un-signal a one-shot handle, say
so once rather than faking a reset.

**Authority.** Python and Rust. Failing closed can only cancel work early;
substituting the handle fails open and cannot cancel it at all.

---

## D-91 — a removal method a host cannot call does not satisfy the contract

**Status quo.** Rust's `ExtensionManager::unregister(&mut self, point_name,
extension: &ExtensionKind)` identifies its target by identity — but the manager
owns its extensions, so a caller cannot borrow one out and pass it back across
`&mut self`. The method compiles; a positive removal is not expressible from
outside without a pre-registration handle.

**Decision.** At least one reachable removal path is required — the identity form
where the language allows it, or an equivalent keyed on something the caller holds
independently.

**Authority.** Reachability. "Provided but uncallable" is invisible to
signature-level parity checking and only shows up when someone tries to write the
call.


---

# Wave 2 — D-92 – D-107

Wave 1 came from a partial deep-chain pass: the orchestrator hit a rate limit
after three modules. Re-running it across all twenty-two logical modules roughly
tripled the defect count and, unlike wave 1, turned up **five security defects**.

The shared cause is worth stating once, because it explains why three green test
suites and seventy-nine passing conformance fixtures saw none of this: **a
fixture pins an OUTCOME, and these implementations reached the same outcome by
different paths.** `approval_gate.json` passes in all three SDKs off two
different sources of truth — apcore-rust satisfies it by putting
`requires_approval` in the DESCRIPTOR while its test module leaves
`annotations()` at the default. A case that cannot discriminate between two
implementations is not testing the thing that differs.

## Security defects

**D-93 — the contextual-audit redaction list is a superset, and bare substrings
are the point of it.** apcore-typescript enumerated compounds (`api_key`,
`apikey`, `access_key`, `private_key`, `authorization`) where apcore-python and
apcore-rust carry the bare `key`, `auth` and `session`. Enumerating compounds
catches only the spellings someone thought of: `signing_key`, `auth_header` and
`session_id` matched none of them, so two SDKs redacted those three and the
third published them verbatim to every subscriber, log sink and audit store on
the event bus. The list is deliberately a superset of
`obs.redaction.sensitive_keys` so credentials cannot leak here even when global
redaction is off — a guarantee one SDK had opted out of.
**Authority:** apcore-python + apcore-rust (byte-identical). *Fixed.*

**D-94 — symlink confinement runs before the dir/file split.** apcore-python's
check lived inside the `is_dir` branch, so a symlinked `.py` whose target
escaped the extensions root reached `elif is_file` unchecked, was discovered, and
was imported and executed. The check's own comment describes exactly this
failure — it was simply on the branch that does not yield importable files.
**Authority:** apcore-typescript + apcore-rust, which both check before the
split. *Fixed, with a companion test proving in-root aliasing still works so the
fix cannot be over-corrected into a blanket refusal.*

**D-95 — a configuration dot-path addresses data, never the host object graph.**
apcore-typescript's `setNested` guarded its descent with `part in current`.
`'__proto__' in {}` is true (an inherited accessor) and
`typeof Object.prototype === 'object'`, so the guard passed, the walk left the
config object entirely, and the assignment landed on `Object.prototype` —
polluting every object in the process while the config's own keys stayed
unchanged. Reachable with **no module call, no ACL decision and no approval**:
the `APCORE_` env loader maps `APCORE_____PROTO_____POLLUTED` to the dot-path
`__proto__.polluted`. Second-order, the pollution then corrupts every `'x' in obj`
check in the SDK, including `getNested`'s own guard.
**Authority:** apcore-python (dicts have no prototype chain) and apcore-rust (no
ambient object graph) — both store `__proto__` as an ordinary nested key.
*Fixed: forbidden segments refused as a whole path rather than sanitised out, and
`Object.hasOwn` replacing `in` on both the read and write paths.*

**D-96 / D-97 — two independent approval bypasses, which compose.**
§7.4 step 2 binds `annotations = module.annotations` and step 3 tests that
binding; apcore-rust decided gate firing from the registry **descriptor**, so a
module declaring `requires_approval: true` ran ungated whenever the descriptor
omitted it — while the `ApprovalRequest` handed to the handler read the live
module, so one call could be gated by one source and described by the other.
Separately, its `FunctionModule` stores `annotations`/`tags`/`documentation`/
`metadata` as public fields but implements **none** of the corresponding `Module`
accessors, so the trait defaults zeroed everything a `*.binding.yaml` declared —
including the approval requirement. The registry-side half of that had already
been fixed under A-D-017: the fix landed on the reader, and the type being read
was never updated.

The correction to D-96 is a **union, not a swap**, and this was the one place a
sub-agent's refusal to follow its brief was right. Implemented as first
specified — read the module, not the descriptor — the fix simply inverts the
bypass: an operator's requirement declared in a caller-supplied descriptor stops
gating. Both single-source readings are fail-OPEN, and on an approval gate the
failure direction is the whole argument: requiring an approval that was not
strictly needed costs a prompt; skipping one that was needed is a bypass. The
gate now fires on the union, mirroring §6.9, where the requirement is already the
union of the annotation, the ACL rule and `gate_destructive`. It costs
apcore-python and apcore-typescript nothing, since their descriptors are derived
from the module. **Authority:** §7.4 for D-96's module half, apcore-python +
apcore-typescript for D-97, fail-closed reasoning for the union. *Both fixed.*

**D-98 — `$ref` sibling keys are preserved.** apcore-rust's `$ref` branch read
only `map.get("$ref")` and returned the resolved target; every other key of the
node was dropped. Because §10.6 redaction is driven by `x-sensitive` on the
RESOLVED schema, a field marked sensitive beside a `$ref` was redacted by two
SDKs and written to logs in plaintext by the third. **Authority:** apcore-python
+ apcore-typescript, and §4.11 step 1b, which already requires siblings preserved
on the self-reference path. *Fixed; the end-to-end test printed the leak verbatim
before the fix.*

## The `global_deadline` group (D-99 – D-102)

Three SDKs kept one value on three clocks, in two places, with two lifetimes —
and every one of the four rules was already implied by existing text without
being stated where an implementer would look.

- **D-99, the clock is epoch seconds.** apcore-python used `time.monotonic()`,
  apcore-typescript `Date.now()` milliseconds. The field is a public
  `Context.create` parameter, so a caller writes `time.time() + budget`; against
  a monotonic basis that is ~1.8e9 compared against ~1e5, so the deadline never
  fires and the call runs with **no budget at all** — silent, and fail-open.
  **Authority:** the written definition, and apcore-rust, the only conforming
  implementation. The design doc's own note records the Rust change as made "to
  align with Python/TS"; that alignment had never happened.
- **D-100, the field is the storage.** apcore-typescript wrote and read
  `context.data['_apcore.executor.global_deadline']` and never consulted its own
  `globalDeadline` field, so a caller-supplied deadline was silently replaced by
  the config default. **Authority:** apcore-python + apcore-rust.
- **D-101, the deadline belongs to the call tree.** apcore-python mutated the
  caller's Context in place and apcore-typescript mutated the shared `data`
  object, so a Context reused across top-level calls kept the first call's
  budget. Reuse is explicitly blessed by the same contract. **Authority:**
  apcore-rust, which clones per call.
- **D-102, a deserialized Context recomputes unconditionally.** apcore-python
  gated recomputation on an empty `call_chain`, and a Context arriving from
  another process carries a non-empty one **by definition** — so the guard
  inverted the MUST exactly where it applies, and every cross-process sub-tree
  ran unbounded. **Authority:** apcore-rust + apcore-typescript.

## The remaining adjudications

**D-92 — `TaskStoreError` must exist before it can be raised.** Declared on eight
surfaces in `async-tasks.md`; defined in **no** SDK. A declared error type no
implementation can raise is one no caller can catch — the "declared surface
reaches no mechanism" shape §9.1.3 forbids for configuration keys, here applied
to an error contract. **Authority:** the spec. All three must define, register and
export it.

**D-103 — a null `identity` stays null.** apcore-rust manufactured an
`Identity{id:"@external", type:"external"}` for any call without one, and
`child()` cloned it through to the module. The spec contradicted itself: this
contract's parameter row promised the synthesis while its own Returns clause, the
`Context` field table and every example in `context-object.md` said nullable.
`@external` is the caller-side ACL sentinel for a null `caller_id`, not a
principal — the spec's own module example (`if not context.identity: raise
"Authentication required"`) rejects on two SDKs and **admits** on the third.
**Authority:** apcore-python + apcore-typescript; the contradictory row was the
outlier.

**D-104 — the base document for a local `#/…` reference.** A05 step 4a said only
"file_part = current_file", and the three read it two ways, so **no schema file
containing a local `$ref` loaded in all three**. apcore-rust resolved against the
file — which is what the step says, and what §4.11's own example requires, that
example being loadable on Rust alone. apcore-python and apcore-typescript
resolved against the schema node, so a nested `$defs` worked there and the spec's
example did not. Each rejected what the other accepted.
**Decision: both layouts, file root first with a fallback** — not the strict
reading. Layout B is what two of three accept today and is therefore presumably
in the wild, and the two lookups cannot collide. A side effect:
`SchemaDefinition.definitions`, collected by two SDKs and read by nothing,
becomes live; it was dead precisely because the refs that would use it could not
resolve.

**D-105 — the executor's ACL step MUST take the async path.** §6.1.3 defines what
each entry point resolves and never said which one the pipeline calls.
apcore-rust called the synchronous `check_access` from inside an already-`async`
step, which makes a condition registered through `register_async_condition`
"async only" on that path — resolving to UNEVALUABLE, so an `allow` rule carrying
it stops granting and a `deny` rule carrying it denies unconditionally. Both
directions wrong, and the entire async condition registry unreachable from the
only path that enforces. **Authority:** apcore-python + apcore-typescript, the
latter recording the reason in a source comment.

**D-106 — a p99 beyond the largest bucket is that bucket, not zero.** Returning
`0.0` reports the fastest possible latency for the slowest modules, so the
latency alert can never fire for a module slower than the top bucket.
apcore-rust pinned the wrong behaviour in a unit test, which is why the suite
stayed green. **Authority:** apcore-python + apcore-typescript.

**D-107 — per-class markers are the only multi-class opt-in.** Three SDKs shipped
three models and the fixture pinned a fourth: apcore-python honours a per-class
marker only; apcore-rust has no per-class marker at all and gates on a file-level
toggle whose doc comment still cites `extensions.multi_class_discovery`, the key
decision-log D-06 removed; apcore-typescript ships both, under two names with
opposite defaults, so the spec's own TypeScript example silently returns one
module where it documents two. And `multi_module_discovery.json` carries a
file-level `multi_class_enabled` on every non-conversion case, so all three pass
a fixture pinning the withdrawn model.
**Decision: per-class**, as the spec already said — against 2-of-3 on the
mechanism. A file-level toggle cannot express the case the feature exists for,
two participating classes beside a helper class that must not become a module,
and it is the same flag D-06 already removed once. The fixture is the thing that
has to change.


---

# Wave 3 — D-108 – D-121

Waves 1 and 2 corrected **defects**: behaviour that contradicted an existing
normative statement, or that leaked, bypassed or crashed. This wave is
different. These fourteen are the warning tail, and they share a property the
earlier ones did not — **the spec was silent, and each of the three SDKs had
answered reasonably**. There was no bug to find, only a question nobody had been
asked.

That makes them maintainer policy rather than audit findings, which is why they
were brought to the maintainer as a list rather than decided by the audit. Two
(D-112, D-121) were decided against the recommendation the audit offered, and
both are recorded below as the maintainer settled them.

Implementation is **deliberately deferred** until the v1.49.0 and v1.50.0
branches are reviewed: those already carry 12,747 changed lines across four
repositories, unreviewed, and stacking a third wave on an unreviewed base would
bury any defect in it.

## D-108 — an unknown extension point is an error; an empty one is not

**Status quo.** `get` / `get_all` / `unregister` raised `KeyError` on
apcore-python, threw `Error` on apcore-typescript, and returned
`None`/empty/`false` on apcore-rust. The contract's `### Errors` row said "No
errors raised".

**Decision.** Reject an unknown point name with `GENERAL_INVALID_INPUT`. Do NOT
raise for a registered point that currently holds nothing.

**Authority.** 2-of-3, and a re-reading of the row: it was written about the
EMPTY case. The distinction matters because the silent reading turns a typo into
a wiring bug that first appears at `apply()`, far from the call that caused it.

## D-109 — only the healthy/degraded boundary is configurable

**Status quo.** apcore-python and apcore-typescript computed the degraded/error
boundary as `healthy_threshold * 10`; apcore-rust fixed it at `0.10`, which is
what the classification table says.

**Decision.** The table. `error_rate_threshold` moves the first boundary only.

**Authority.** The spec, verified against `system-modules.md`'s own table. The
consequence is recorded in the contract because it surprises: with
`error_rate_threshold: 0.001`, `degraded` spans `0.1%–10%`. A caller wanting a
stricter ERROR boundary is asking for a second knob that deliberately does not
exist.

## D-110 — `project_name` defaults to `"apcore"`

**Status quo.** apcore-python and apcore-typescript returned `""` from
`manifest.full`; apcore-rust returned `"apcore"`. But `health.summary` already
returned `"apcore"` in all three.

**Decision.** `"apcore"`.

**Authority.** Neither SDK majority — internal consistency. Choosing `""` would
have required changing two system modules per SDK and left `health.summary`
disagreeing with `manifest.full`; choosing `"apcore"` changes one and removes an
existing contradiction.

## D-111 — a bulk reload audits per module, with a correlation id

**Status quo.** apcore-python and apcore-rust wrote one aggregate entry whose
`target_module_id` was the glob; apcore-typescript wrote one per module.

**Decision.** Per module — and every entry from one bulk reload carries the same
correlation id.

**Authority.** apcore-typescript, at 1-of-3, because `AuditStore.query(module_id?)`
filters by a concrete id and therefore cannot find an entry keyed on
`executor.*`. The correlation id is the maintainer's addition: per-module entries
alone lose the fact that they were one deploy.

## D-112 — a failed reload restores the previous module

**Status quo.** apcore-typescript re-registered the original on failure;
apcore-python and apcore-rust left it unregistered, and the contract endorsed
that ("callers must handle the partial state").

**Decision (maintainer).** Restore, treating replacement as an operation that
should not destroy a working module. Recorded with four constraints the
mechanism forces, because the audit flagged that "atomic" overstates what is
achievable: restoration is compensating and has a visible unregistered window; the
restore path MUST re-run `on_load`, since `on_unload` already ran and a module
republished without it is visible but torn down; if that load also fails the
module MAY stay unavailable, there being no good state left; and on the bulk path
restoration is per module, with no cross-module rollback, which the registry has
no primitive for.

**Authority.** Maintainer policy. This is a control-plane judgement — a failed
hot-fix making a working module *disappear* — not a consistency question.

## D-113 — storage-backend namespaces, and the omitted-argument default

**Status quo.** §1.1 made the `StorageBackend` argument a MUST and never named
the namespaces. Of nine collector/SDK combinations, four wrote anything; the two
SDKs that wrote `ErrorHistory` records used different names (`errors` vs
`error_history`); and only apcore-typescript honoured "when omitted,
`InMemoryStorageBackend` MUST be used".

**Decision.** `metrics` / `usage` / `error_history`, and `InMemoryStorageBackend`
when omitted. A dual-read migration window for apcore-python's existing `errors`
data, plus a migration note.

**Authority.** apcore-rust on the write path (the only SDK where the argument did
anything), apcore-typescript on the default. The migration constraint is the
maintainer's: a namespace rename is invisible until someone queries old data and
finds nothing.

## D-114 — `remove()` clears the duplicate-identity entry

**Status quo.** apcore-typescript cleared it; apcore-python and apcore-rust did
not, so `use` / `remove` / `use` warned about a duplicate naming a registration
that no longer exists.

**Decision / Authority.** apcore-typescript. The registry records the FIRST
registration so a duplicate can be traced to it; a stale entry corrupts that
record in both directions.

## D-115 — a malformed annotation value is tolerated and dropped

**Status quo.** For `{"extra": "oops"}`: apcore-python coerced to `{}`,
apcore-typescript object-spread the string and fabricated `{"0":"o", …}`,
apcore-rust failed the whole `ModuleDescriptor`. For `{"cache_ttl": -5}`: two
clamped, one failed.

**Decision.** apcore-python's: coerce or drop, keep the rest, warn.

**Authority.** apcore-python. TypeScript's output is worse than lenient — it is
invented data indistinguishable from a real declaration. Rust's is worse than
strict — one bad integer removes an entire module. Recorded with the
implementation note that a `u64` field puts the failure BEFORE the clamp, making
the required clamp unreachable; deserialize signed first.

**Correction (2026-09-17).** The status quo above was recorded without being run,
and two thirds of it are wrong. Measured at implementation time:

| input | apcore-python | apcore-typescript | apcore-rust |
|---|---|---|---|
| `{"extra": "oops"}` | **rejected the registration** with a bare `ValueError` | kept the string as `extra` | rejected |
| `{"cache_ttl": -5}` | clamped to 0 | kept `-5` | rejected |

apcore-python did coerce to `{}` — in `from_dict`, which the registration path
does not use. `merge_annotations` builds the dataclass directly, where
`dict("oops")` raised, so a module with one malformed annotation was refused with
an untyped error that escapes every `except ModuleError` handler. That is the
same shape as D-85: the tolerance existed at one door of two, and the status quo
was read off the tolerant one.

**The decision is unchanged**, because its argument never rested on the count:
keeping the value is worse than lenient (it then reads as a declaration the
author wrote), and rejecting is worse than strict (one bad value removes an
entire module). What is withdrawn is the **Authority** line — no SDK implemented
this, so there was no majority and no authority to follow. The decision stands on
its own reasoning, which is where it always stood.

Recording this rather than quietly amending the table: a decision whose status
quo was never run is exactly what D-125 and D-126 corrected elsewhere, and the
correction is only useful if it is visible.

## D-116 — circuit-breaker events carry the declared subscriber type

**Status quo.** apcore-python and apcore-typescript reported the wrapped
subscriber's class name; apcore-rust split `subscriber_id` on the first hyphen,
so `health-alert` became `health`.

**Decision.** The declared kind (`webhook` / `a2a` / `file`) that the DLQ path
already reports in all three. An undeclared subscriber takes the DLQ path's
existing default; no second default is invented for this surface.

**Authority.** None of the three — the value the same SDK already puts in its own
DLQ payload, which is what a consumer routing on `subscriber_type` receives.

## D-117 — registered-namespace defaults do not answer for a legacy document

**Status quo.** apcore-rust consulted them in both modes; apcore-python and
apcore-typescript in namespace mode only.

**Decision / Authority.** apcore-python + apcore-typescript, on §9.6.3's reading
that a legacy document has no namespaces — so a declaration ABOUT a namespace has
nothing to say about one.

## D-118 — an empty `roles` list is omitted from the audit snapshot

**Status quo.** apcore-typescript always emitted `roles: []`; the peers omitted
the key.

**Decision / Authority.** 2-of-3, and the spec names only `id`, `type` and
optionally `display_name`.

## D-119 — `system.*` modules declare `open_world: false` explicitly

**Status quo.** apcore-typescript set it; apcore-python and apcore-rust left it
at the language default, which is `true`.

**Decision.** `false`, written out in all three.

**Authority.** apcore-typescript's semantics at 1-of-3 — no system module reaches
an external system — with the explicitness requirement added because relying on a
default that means the opposite of the intended value is how the divergence
arose.

## D-120 — error timestamps are `Z` with millisecond precision

**Status quo.** `+00:00` microseconds (Python), `Z` milliseconds (TypeScript),
`+00:00` nanoseconds (Rust).

**Decision.** `2026-09-16T10:30:00.123Z`, fixed in `ErrorHistory`.

**Authority.** The spec's own example uses the `Z` form. The precision is part of
the requirement rather than a detail: specifying the suffix alone would leave
three precisions behind one `Z` — the same divergence, now harder to see. The fix
location matters for the same reason: one SDK's `to_rfc3339()` sits in
`health.rs`, and correcting it there would leave the producer untouched.

## D-121 — `reload_dependents` is deprecated for removal

**Status quo.** Declared in all three SDKs' input schemas and read by none. A
spec MUST ("also reload modules that depend on matched modules") that no
implementation satisfies.

**Decision (maintainer).** Deprecate now, remove at v2.0. Do NOT implement.

**Authority.** Maintainer policy. Three independent implementations skipping it
is evidence it is not a capability the protocol needs; continuing to declare it
manufactures an interface no caller can rely on — the §9.1.3 shape applied to a
module input field. The replacement (an explicit `path_filter` covering the
dependents) is named in the deprecation notice so removal leaves no capability
gap, and the migration note must call out that a field silently ignored today
becomes a **validation error** after removal, since every input schema sets
`additionalProperties: false`.

---

# Wave 4 — D-122 – D-123

These two did not come from the audit. They came from **reviewing the audit's
own output** — reading the 12,747 changed lines the first three waves produced,
before handing them to a maintainer.

That origin is the interesting part. D-122 is a divergence three sub-agents
*created* on the same day, independently, while implementing D-81: the decision
said `TaskStoreError` must reach the caller and said nothing about what
`shutdown()` does with the tasks it had not reached yet, so each SDK filled the
silence differently. A decision that closes one gap can open another, and the
only thing that catches it is reading the result. D-123 is the mirror image — a
requirement that was already written, in a contract scoped to the one entry
point that honoured it.

Unlike Wave 3, **both are implemented here**, because both are narrow enough to
review alongside the change that exposed them.

## D-122 — `shutdown()` attempts every cancellation before it reports

**Status quo.** All three SDKs cancel active tasks during `shutdown()`. On a
cancellation failure apcore-python and apcore-rust stop at the first error;
apcore-typescript attempts every task and reports afterwards. The divergence was
introduced by the D-81 implementation itself: before it, the error could not
escape `cancel()` at all, so there was nothing to aggregate and no way to tell
the strategies apart.

**Decision. Attempt every cancellation, then raise the first failure.** Later
failures are logged, not swallowed silently; the first is raised so the caller
still learns that shutdown was not clean.

**Authority.** The asymmetry of the two failure modes decides it, not a 2-of-3
count (which would have chosen the losing option). When the *store* is
unreachable, every cancellation fails and the two strategies are equivalent —
nothing gets cancelled either way, and fail-fast merely arrives there sooner.
When *one task* fails — a handler that raises, a task in a state the backend
rejects — fail-fast leaves every remaining active task uncancelled. Those costs
are not symmetric: an uncancelled task in a shared store holds a `max_tasks`
slot for every manager pointed at that store and outlives the process that could
have cancelled it, while a slower shutdown is transient and bounded by the task
count.

The natural objection — that best-effort can hang — carries little weight here,
because `shutdown()` is **already an unbounded wait by contract**: it waits for
in-flight tasks to complete and takes no timeout parameter in any of the three
SDKs. A caller who needs a bound must already impose one from outside, and that
bound covers the cancellation loop exactly as it covers the wait.

## D-123 — a hot-reloaded module MUST NOT become visible before its `on_load()` has run

**Status quo.** [`Contract: Registry.register`](../features/registry-system.md#contract-registryregister)
Side Effects step 8 already requires a module to run `on_load()` before it
becomes observable — and it is scoped to `register`. A watch-driven reload is a
different entry point, so an implementation that writes the internal maps
directly violates no stated rule. apcore-python's `_handle_file_change` does
exactly that: it published the re-imported instance into `_modules`,
`_versioned_modules` and `_module_meta` without ever calling `on_load`.

**Decision. The rule binds the publish, not the entry point.** Whichever
mechanism an implementation picks, it MUST NOT publish a module whose load hook
has not run. Recovery when that load fails is governed by **D-112 rules 2–4**
and is deliberately NOT restated at the second entry point.

**Authority.** A module that is visible but never initialised is harder to
diagnose than one that is absent — the same reasoning D-112 rule 2 already
records. It fails at call time, inside whatever the hook was supposed to have
set up, with a stack trace that points nowhere near the reload that caused it.

Two boundaries matter as much as the rule. First, it constrains **publication,
not mechanism**: D11-005 leaves the reload mechanism language-defined —
re-registering in place, re-running discovery, or notifying only are all
permitted — and this does not narrow that. Second, the recovery rules are
referenced rather than repeated, because a duplicated rule drifting apart is the
defect class this entire audit is about: §10.6.1's key matcher was implemented
twice and one copy drifted into a leak, and A-D-017's fix landed on the reader
while the type being read was never updated.

Only apcore-python is affected. apcore-rust's `watch_loop` re-runs
`discover_internal()`, which invokes `on_load` before registration and skips the
module when it fails; apcore-typescript's `_handleFileChange` unregisters and
emits `file_changed` without re-registering at all, leaving re-registration to
the consumer's own `register()` call — so it has no publish path to constrain.

---

# Wave 5 — D-124

Wave 4 came from reading the audit's output. This one came from the **maintainer
reading it** — a review of the four branches that returned three concrete misses
in apcore-rust, two of them still affecting security semantics. All three were
real. Two were the D-96 union failing to reach its readers and are recorded with
that decision; the third is general enough to be its own.

The pattern in all three is one thing: **a fix that lands at the point of
decision and not at the points that describe it.** D-96 unioned the approval
gate and left the `ApprovalRequest`, the preflight and the governance-posture
accessor reading a single source. D-104 taught the resolver a second base and
left the scope of that base unstated, so it applied to documents it was never
about. The union now lives in one function
(`ModuleAnnotations::governance_union`) and the fallback now travels per hop
rather than on the resolver, because in both cases the defect was that the rule
had one home and several readers.

## D-124 — the node fallback is scoped to its own document

**Status quo.** All three SDKs. apcore-rust held `node_fallback` on the
`RefResolver`, apcore-python returned `self._file_cache[_INLINE_SENTINEL]` from
`_fallback_document` for any local reference, and apcore-typescript's
`_resolveLocalPointer` fell back to the same inline sentinel — none of them
asking which document resolution had reached. A local pointer inside an external
schema that the external schema does not define resolved against the *calling*
module's `input_schema` `$defs`.

Confirmed by reproduction in each SDK rather than by reading the code: the same
two-file case (`caller.schema.yaml` referencing `./ext.schema.yaml#/$defs/Thing`,
whose `Thing` contains `{"$ref": "#/$defs/Shared"}` that `ext` does not define)
resolved `Shared` to the caller's definition in all three.

**Decision.** The fallback is available only while resolution is inside the
document the schema node belongs to. Once a reference has been followed into
another document, a local pointer that does not resolve *there* throws
`SCHEMA_NOT_FOUND`.

**Authority.** JSON Schema 2020-12 §8.2 base-URI resolution — a `#` fragment is
resolved against the current base, and following an external reference changes
the base. Nothing in that model lets a referenced document inherit the referrer's
definitions, and there is no reading under which it should: the external author
did not write the definition being bound, and cannot see it.

The cost is not theoretical. §10.6 reads `x-sensitive` off the **resolved**
schema, so a sensitive field in the external document can be replaced by a
same-named local definition that carries no marking, and the value is logged in
plaintext. That is SCH-001's leak reached by a different route, which is why it
is treated as a defect rather than a policy question.

**Boundary.** This does not narrow D-104. A local pointer inside an external
document still resolves in that document, and Layout B is untouched: at the
origin, the schema node and the document it belongs to are the same document.
Every SDK's test for this carries a control case proving the fallback was scoped
and not disabled.

---

# Wave 6 — D-125

## D-125 — D-96 binds every SDK and every governance reader

**Status quo.** D-96 closed with this sentence:

> It is unobservable in implementations whose descriptors are derived from the
> module (apcore-python, apcore-typescript), which is why the union costs them
> nothing.

That sentence was written during this audit, was never checked against the two
SDKs it names, and is **false**. Neither descriptor is derived from the module
alone: `merge_module_metadata` folds a `*_meta.yaml` / `*.binding.yaml` /
`metadata=` declaration into it with §4.13's YAML > code precedence. That is a
second place an operator declares governance — and neither gate read it.

Reproduced in both, by running it rather than reading it. A module registered
with `metadata.annotations.requires_approval: true`:

| Surface | Reported |
|---|---|
| `get_definition().annotations.requires_approval` | `true` |
| `system.manifest.module` entry | `true` |
| `Executor.validate` preflight | **`false`** |
| The approval gate | **never fired** — the handler was configured and never consulted |

So the operator's declaration reached everything that *describes* the module and
nothing that *stops* it. That is exactly the bypass D-96 was written to close,
in the two SDKs D-96 named as unaffected, and it is fail-OPEN.

**Decision.** The rule is unchanged; its scope is corrected. The union binds
every reader of `requires_approval` / `destructive` for a registered module —
the gate, the §7.9.5 preflight, the §6.6.5 posture accessor, the
`system.manifest.*` projection, and the ephemeral-registration advisory. Each
SDK resolves it through one function (`governance_union` /
`governanceUnion` / `ModuleAnnotations::governance_union`).

**Authority.** Two arguments, and the second is the one that generalises.

First, the direction of failure, which is D-96's own: requiring an approval that
was not strictly needed costs a prompt; skipping one that was needed is a
bypass. A reader narrower than the gate reports a verdict the gate will not
honour, and on the manifest that is worse than silence — an agent reads it to
decide whether to call, so a false `requires_approval: false` is a specific
claim it acts on.

Second, the union **must not** be the metadata merge. §4.13's YAML > code
precedence is right for a descriptor — the operator's document is the more
specific statement about what a module *is* — and wrong for a gate, because it
lets the weaker declaration win in both directions: a metadata `false` cancels a
module that asks to be gated, and a metadata `true` reaches only the descriptor.
Only the governance flags are unioned; everything else stays instance-sourced,
because the instance is authoritative for behaviour.

**What this says about the audit itself.** Three of the last four findings have
the same shape — a fix landing at the point of decision and not at the points
that describe it (D-96's readers, D-104's scope, and now D-96's own claim about
where it applies). This one is the sharpest version, because the drift was in a
sentence of the specification: the code was corrected in one SDK and the spec
recorded, without checking, that the other two needed nothing. A claim about
another implementation is a claim to verify, not to reason out — and the
verification here took one script per SDK.
