---
description: "Maintainer decisions left open by the 2026-09 configuration-surface audit (apcore#118): eight declared config surfaces that no SDK reaches, each needing a keep-or-withdraw call before v2.0."
title: Configuration surface — open decisions (2026-09)
date: 2026-09-10
status: open — 8 items, 0 decided
audience: maintainers + spec reviewers
source: apcore#118 configuration-key audit (2026-09-09 – 2026-09-10, all 65 declared keys examined)
---

# Configuration surface — open decisions

`conformance/config_key_consumers.json` now records a status for **every one of the 65
declared configuration keys**, and `unaudited` is zero. The audit that produced it
(apcore#118) closed the question *"is this key read?"* for all of them. It did not close a
second question, which only became answerable once the first was: **for each surface that
nothing reads, is it wired up or withdrawn?**

This file holds the eight that need a maintainer's call. They are collected here rather
than left as comments on #118 because a decision needs a shape a comment thread does not
give it — the status quo per SDK, the options, a recommendation, an owner, and the concrete
action once decided — and because there are eight of them, which is too many for eight
issues and too many to keep in one comment.

Tracking issue for all eight: [#118](https://github.com/aiperceivable/apcore/issues/118).

**Nothing here is decided.** Each entry carries a first-pass recommendation, which is an
argument to be accepted or rejected, not a fait accompli.

## Why these are one family

Six of the eight are the same defect wearing different names, and the audit made the shape
explicit enough to state once:

> **The mechanism is implemented; the configuration key does not reach it.**

`acl.default_effect`, `id_map.overrides` and the three `pipeline.*` keys are all like this.
The behaviour exists, it is tested, and it is reachable — through a **constructor argument
or a function parameter**, never from a loaded `Config`. This matters for how they get
audited as much as for how they get fixed: **a probe that exercises the behaviour by that
other door reports the key as working.** That is precisely how `acl.default_effect` came to
be recorded `live` in the guard written to find keys nothing reads, and the checker now
rejects a `live` probe that never puts its own key into a `Config`.

The remaining two (`_config.allow_unknown`, `extensions.roots`) are ordinary gaps: one that
no SDK implements, one that only apcore-rust does.

Deadline for every item: **v2.0**, since each ends in either wiring (any 1.x, additive) or
removal (§13.2's two-minor floor, so v2.0 at the earliest).

---

## D-66 — `acl.audit`: two declared homes, one has to go

**Tracking issue**: [#118](https://github.com/aiperceivable/apcore/issues/118). Opened by spec v1.39.0 (§9.2.4, §9.2.4.1), which started the deprecation window for **both** homes deliberately, so that choosing between them would not also be a scheduling problem.

**Status quo**

ACL audit configuration is declared in two places and read in neither:

| Home | Declared in | Read by |
|---|---|---|
| An ACL file's `audit:` block | `schemas/acl-config.schema.json` | nothing — no SDK validates ACL files against that schema, and all three loaders parse into an open container, so the block is dropped in silence |
| `acl.audit.enabled` / `.include_denied` / `.log_level` | `schemas/apcore-config.schema.json` | nothing |

Both now warn at load (§9.2.4.1 puts the ACL-file notice in the **loader**, the only place that can see the block at all). Both are scheduled for removal no earlier than v2.0.

**And there is nothing to wire either of them TO.** Raised in review and verified: ACL auditing has no delivery contract at all. `ACL(audit_logger=…)` takes a programmatic callback defaulting to `None`, and that is the whole surface — there is **no default sink**, no statement of what happens when delivery fails, and no defined semantics for either `include_denied` (does it filter what is *delivered*, or what is *recorded*?) or `log_level` (a level against which logger, applied where?). So "keep the ACL file's block" currently means keeping a key that configures an unspecified mechanism.

**Decision needed**

Two things, in order. **First**, what ACL audit delivery *is*: sink, failure behaviour, and the meaning of the two modifiers. **Second**, which of the two homes configures it, so that the deprecation notice can name a migration target — a notice that says "this is going away" without saying "use that instead" is half a notice, and it is what an operator writing ACL configuration gets today.

The order matters and is not cosmetic: choosing a home for a contract that does not exist decides where to put a key whose meaning is still open, which is how both homes came to be declared in the first place.

**Options**

- **A — the ACL file's `audit:` block survives; `acl.audit.*` is removed at v2.0.** ACL configuration lives in the ACL file; §9.2.4.1's notice already sits in the ACL loader, which is the only code that can read the block.
- **B — `acl.audit.*` survives; the ACL file's block is removed at v2.0.** One configuration file for the whole framework; ACL files stay purely about rules.
- **C — both are removed and auditing becomes API-only** (`ACL(audit_logger=…)`), with no configuration surface at all.

**Recommendation: A.**

The asymmetry is not aesthetic. Under B the surviving key still has to be *wired*, and wiring it means the `apcore.yaml` loader reaching into ACL construction — while under A the wiring is local to the ACL loader, which already parses the document the block lives in and already emits the notice about it. C is defensible but withdraws a capability rather than fixing it, and the audit block is one of the few ACL surfaces an operator can reasonably want to change per environment.

**Owner**: maintainer (GOVERNANCE.md § Decision Making — `MAINTAINERS.md` lists one, so one approval suffices).

**Action once decided**

1. **Define the delivery contract first** — a normative section covering the default sink (or the explicit statement that there is none and a callback is required), what an implementation does when delivery fails, what `include_denied` filters, and what `log_level` is a level *of*. Without this, steps 2–4 wire a key to nothing.
2. Record the surviving home in §9.2.4's table and in §9.2.4.1.
3. Amend both deprecation notices to name the survivor as the migration target.
4. Wire the survivor in all three SDKs (a 1.x change; additive).
5. Remove the other at v2.0.

---

## D-67 — logging: what replaces `logging.level` / `logging.format` at v2.0

**Tracking issue**: [#118](https://github.com/aiperceivable/apcore/issues/118). Window opened by spec v1.39.0.

**Status quo**

`logging.level` and `logging.format` are declared, documented with defaults, environment-overridable — and read by nothing. Both are deprecated for removal at v2.0.

What has not been recorded anywhere: **apcore has no logging configuration surface at all.** `ContextLogger`'s level and format are constructor arguments, and **no construction site in any SDK accepts a `Config`** — verified by search across all three. So v2.0 does not take two key spellings away from an operator; it takes away the ability to control logging from configuration, which they do not have today either.

**Decision needed**

Whether apcore offers configuration-driven logging control at all.

**Options**

- **A — wire it.** Before v2.0, give `ContextLogger` a `from_config` path and have the client construct its logger through it, keeping the two keys.
- **B — withdraw and say so.** Remove both at v2.0 and state in the specification that logging configuration belongs to the host application, which owns the logging framework anyway.

**Recommendation: B, with the second half actually written.**

apcore emits structured records; the host owns the sink, the level policy and the format. A framework-level level key competes with whatever the host already has, and the divergence risk is the one §10.6.1 has been paying for all week. But B is only finished when the specification *says* it: withdrawing the keys and leaving the question unanswered is how an operator concludes the capability was lost rather than never offered.

**Owner**: maintainer.

**Action once decided**

- If A: `ContextLogger.from_config` in three SDKs, wired at client construction, plus a probe in `check_config_key_consumers.py` and a conformance case.
- If B: a short normative note (§10.x) stating that logging configuration is host-owned; remove both keys at v2.0; keep the deprecation notice pointing at that note.

---

## D-68 — the tracing configuration surface is declared in two places that disagree

**Tracking issue**: [#118](https://github.com/aiperceivable/apcore/issues/118). Window opened by spec v1.39.0.
**Revised 2026-09-11** — the original entry's status quo was wrong in a way that changed the decision. It is kept below, struck through in prose rather than deleted, because the error is the interesting part.

**What the original entry said, and why it was wrong**

The original recommendation was **C**: *"complete tracing (`strategy` + `sampling_rate`), withdraw `metrics.exporter` and `jaeger`"*, resting on this claim:

> There is **no `sampling_strategy` configuration key** — verified: the identifier does not appear in `schemas/`.

The verification was correct and the conclusion was not. The identifier is absent from `schemas/`; **the key is not**. §9.15.2's `observability` namespace registration declares it — as `strategy`, with the default `"full"` and the comment `"full" | "proportional" | "error_first" | "off"` — and all three SDKs carry that registration verbatim. `strategy` is not a new key. It is an **old key declared by one canonical source and absent from the other**.

Two more of the same shape came out with it: `observability.tracing.otlp_endpoint` (declared in §9.15.2, default `null`, present in all three SDKs) and the exporter enums, which the two surfaces spell differently.

**Status quo, measured**

apcore-python, by execution:

```
Config.from_defaults() → namespace("observability")["tracing"]
  = {"enabled": false, "sampling_rate": 1.0}      # the registration declares FIVE leaves

get("observability.tracing.strategy")      = None
get("observability.tracing.exporter")      = None
get("observability.tracing.otlp_endpoint") = None

_config.strict: true + observability.tracing.strategy       → REJECTED, "Unknown key"
_config.strict: true + observability.tracing.otlp_endpoint  → REJECTED, "Unknown key"
```

So the same key is **declared with a default by §9.15.2 and rejected outright by `_config.strict`**. This is the third variant of the shape this audit keeps finding, and the sharpest: the key is declared twice, the two declarations disagree, and the fuller one is the one that gets discarded.

Three further facts, all verified:

| | |
|---|---|
| `schemas/observability.schema.json` | **does not exist.** §9.15.2 line 7727 names it as the namespace's schema; that line is the only reference to it anywhere in the repo or the three SDKs. |
| tracing exporter enum | `apcore-config.schema.json` says `stdout`/`otlp`/`jaeger`; §9.15.2 says `stdout`/`otlp`/`in_memory`. |
| metrics exporter enum | `apcore-config.schema.json` says `stdout`/`prometheus`/`otlp`; §9.15.2 says `stdout`/`prometheus`/`in_memory`. |

And the keys cannot be fixed one at a time in any order. Wiring `sampling_rate` alone yields a key that reads configuration, sets a field and still samples every span, because the strategy short-circuits ahead of the rate. Adding the strategy yields two keys configuring a middleware **nothing installs** — no SDK has ever built a `TracingMiddleware` from configuration. Installing one needs an exporter, and an exporter is an object rather than a name. Each key is unreachable until the one before it exists.

**Decision: C′ — unify the declaration surface, and wire the five tracing keys in one change.**

Ratified 2026-09-11. Not the original C, and **not** split into a follow-up decision: the surface disagreement and the wiring are the same defect seen from two sides, and fixing either alone leaves a key that is declared in one place and unreachable from the other.

Scope:

1. **One canonical declaration.** `schemas/apcore-config.schema.json` gains `observability.tracing.strategy` and `.otlp_endpoint`. §9.15.2 stops listing its own enums and **references the canonical schema** instead; the reference to the nonexistent `schemas/observability.schema.json` is removed rather than satisfied — creating that file would make a third surface, and `apcore-config.schema.json` is already the canonical home for these keys.
2. **`schemas/defaults.schema.json` gains the tracing defaults it lacks** — `strategy`, `exporter` and `otlp_endpoint` — so all three SDKs are held to the same default table.
3. **The exporter enum is `stdout | otlp | jaeger` in 1.x.** `jaeger` stays accepted, warns once per configuration load, installs nothing, and is removed at v2.0 under §13.4's window, because narrowing an enum rejects a configuration accepted today.
4. **`in_memory` is NOT a configuration value.** All three `InMemoryExporter`s are test buffers, and a caller who constructs one from configuration has **no standardised way to reach the spans it holds**. It has a class; it is not an exporter an operator can use or observe. Admitting it to the enum would manufacture a fresh instance of the exact failure this audit exists to remove — a setting that takes effect and produces nothing visible. It stays an API and testing facility.
5. **`otlp_endpoint` is wired**: `string | null`, default `null`, passed to the exporter **only when the effective exporter is `otlp`**. An endpoint set alongside a non-OTLP exporter is a **configuration error at load**, not a silent no-op — "written but ignored" is what this whole audit is about.
6. **Precedence** is unchanged from D-73: an explicitly supplied middleware or exporter beats `Config`, and a caller-supplied `TracingMiddleware` is never joined by a second.
7. `observability.metrics.enabled` and `.exporter` stay deprecated and proceed to removal. There is no metrics-exporter abstraction in any SDK and `MetricsCollector` arrives only as a constructor argument.

**Nothing is un-deprecated until the SDKs read the keys.** The §9.2.4 rows, the `deprecated: true` schema flags and the `inert` records in `config_key_consumers.json` stay exactly as they are until the three implementations land, and the spec change ships in the same push. A specification that says a key has a consumer while no implementation reads it is the defect, not the fix.

**Owner**: maintainer.

**Acceptance** — the wiring is not accepted on a factory test. Every item is end-to-end, Config → client → middleware → observable effect:

- all five keys readable, writable and validating alike from defaults, from a file, from `APCORE_*`, through `Config.set`, and under `_config.strict`;
- `strategy: proportional` with `sampling_rate: 0.1` produces approximately 10% sampling, measured over a run, not asserted on a field;
- `otlp_endpoint` reaches the OTLP exporter;
- `enabled: false` installs no middleware;
- an explicitly supplied middleware or exporter wins, and no second middleware is installed;
- `jaeger` warns, installs nothing, and substitutes nothing;
- `otlp_endpoint` set with a non-OTLP exporter is rejected at load.

## D-69 — `_config.allow_unknown`: implement §9.6.3's row, or withdraw the key

**Tracking issue**: [#118](https://github.com/aiperceivable/apcore/issues/118). Found 2026-09-10 by the audit.

**Status quo**

§9.6.3 publishes a four-row behaviour matrix for an unknown top-level namespace (`billing:`). Measured against apcore-python, stated **per configuration mode**, because an earlier draft of this entry collapsed them into one row set and contradicted itself:

| Mode / `_config` position | `strict: true` | `strict: false`, `allow_unknown: true` | `strict: false`, `allow_unknown: false` |
|---|---|---|---|
| legacy (document root *is* the apcore namespace) | stored | stored | stored |
| namespace, `_config` nested inside `apcore:` | stored | stored | stored |
| namespace, `_config` at the document root | **`ConfigError`** | stored | stored |

Two separate readings come out of that, and only the first is this decision's subject:

1. **`allow_unknown` changes nothing, anywhere.** Every `allow_unknown: true` cell and its `false` neighbour are identical, in all three positions. §9.6.3's third row — "silently ignored, **not stored**" — is never what happens.
2. **`strict`'s namespace clause (§9.6.3 clause a) is position-dependent**: it fires only in namespace mode with `_config` at the document root, and the specification does not say that it should be. That is a separate finding about `_config.strict`, which is recorded `live` on the strength of clause (b) — the §9.14 framework-key walk, which does work in legacy mode. Noted here rather than folded in, because fixing `allow_unknown` does not touch it.

apcore-python's `config.py:843` and apcore-typescript's `config.ts:1537` each carry a comment explaining why the field plays no part in the §9.14 strict walk — correct, since §9.6.3 scopes it to *namespaces* and §9.14 to *framework keys* — but no other path reads it either, in any of the three.

**Decision needed**

Whether "drop unknown namespaces" is a capability apcore offers.

**Options**

- **A — implement it.** At load, when `strict` is false and `allow_unknown` is false, drop unregistered top-level namespaces instead of retaining them.
- **B — withdraw the key** and delete the matrix row, leaving unknown namespaces always retained.

**Recommendation: A.**

The change is small and its blast radius is bounded in a way most withdrawals are not. The default is `true`, so **only a deployment that explicitly writes `allow_unknown: false` changes behaviour at all** — and what changes is that it finally gets the published contract instead of a no-op. Stated that way rather than as "nobody can be relying on it", which is too strong: a deployment *can* be relying on the flag's current inertness, by reading a namespace it also declared `allow_unknown: false` against. That combination is self-contradictory on the face of the configuration, but it is reachable and it would break.

It also has a real use: a host that embeds apcore beside its own configuration may want the framework to ignore sections it does not own rather than hold references to them.

Against A: it is the one option that makes a `get()` start returning `null` where it returned a value.

**Owner**: maintainer.

**Action once decided**

- If A: implement in three SDKs, add a conformance fixture covering all four matrix rows, promote the key to `live` with a probe.
- If B: remove the row from §9.6.3, open a deprecation window for the key per §13.4.

---

## D-70 — `extensions.roots`: converge the other two SDKs, or withdraw

**Tracking issue**: [#118](https://github.com/aiperceivable/apcore/issues/118). Found 2026-09-10 by the audit.

**Status quo**

| SDK | Reads `extensions.roots` |
|---|---|
| apcore-rust | **yes** — `Registry::set_extension_roots_from_config`, accepting both the `["./a"]` and `[{root: "./a"}]` shapes |
| apcore-python | no — measured: a document setting it still scans `extensions.root`'s default `./extensions` and fails when that directory is absent |
| apcore-typescript | no — the identifier appears only in `config.ts`'s path-typed marker declarations, never as a reader |

A multi-root project therefore works on one SDK of three, silently. This is the #116/#117 shape one layer down: not three dialects of one value, but one value honoured by one implementation.

**Decision needed**

Whether multi-root extension discovery is part of the protocol.

**Options**

- **A — converge on Rust.** Implement `extensions.roots` in apcore-python and apcore-typescript, both shapes, with a conformance fixture.
- **B — withdraw the key**, remove Rust's reader, and keep `extensions.root` as the single root.
- **C — keep it Rust-only** and mark it explicitly as an implementation extension rather than a protocol key.

**Recommendation: A.**

The scanner in every SDK already supports multiple roots internally — apcore-python has `scan_multi_root` and `_extension_roots` is a list — so this is wiring, not design. C is the worst option: a declared key in the canonical schema that only one implementation honours is exactly the state this whole cycle has been removing, and it is undetectable to a project that only tests against one SDK.

**Owner**: maintainer.

**Action once decided**

- If A: implement in two SDKs, add a `multi_root_discovery` fixture, promote to `live`.
- If B: deprecation window per §13.4, remove Rust's reader at v2.0.

---

## D-71 — `id_map.overrides`: wire the config key, or withdraw it

**Tracking issue**: [#118](https://github.com/aiperceivable/apcore/issues/118). Found 2026-09-10 by the audit.

**Status quo**

The ID-map mechanism is implemented in **all three** SDKs, and the three agree closely:

| SDK | Mechanism |
|---|---|
| apcore-python | `Registry._apply_id_map_overrides`, stage 2 of discovery, rewrites a discovered `canonical_id`; map loaded by `load_id_map` |
| apcore-typescript | `loadIdMap` / `lazyLoadIdMap` (`registry/metadata.ts`), the same override stage, with the load deferred until `discover`, `watch`, or a constructor carrying `idMapPath` |
| apcore-rust | the same discovery stage (`default_discoverer.rs`) |

Corrected in review: an earlier draft of this entry named only Python and Rust, which understated the cross-language consistency and made A look like new implementation work in TypeScript. It is not — all three already have the mechanism and the loader.

What none of them has is the **config path**: the map arrives only through the `Registry(id_map_path=…)` / `idMapPath` constructor argument. Measured: with `id_map.overrides` pointing at a valid map that renames `executor/orig/mod.py`, discovery still registers `executor.orig.mod`.

**Decision needed**

Whether the config key is wired to the mechanism it names, or removed.

**Options**

- **A — wire it.** Resolve `id_map.overrides` at registry construction, exactly as `extensions.root` already is.
- **B — withdraw the key** and leave ID mapping an API-only facility.

**Recommendation: A.**

It is wiring, not implementation, in all three: the resolution site for `extensions.root` sits beside it in the same constructor, and each SDK's loader already exists. The capability is one a configuration file is the natural home for — which discovered file gets which module ID is a per-project fact, not a per-call-site one.

**Owner**: maintainer.

**Action once decided**

- If A: read the key at registry construction in all three SDKs, add a fixture, promote to `live`.
- If B: deprecation window per §13.4.

---

## D-72 — `pipeline.steps` / `.configure` / `.remove`: the largest inert surface, and the one with teeth

**Tracking issue**: [#118](https://github.com/aiperceivable/apcore/issues/118). Found 2026-09-10 by the audit.

**Status quo**

The declarative pipeline surface is **fully implemented and carefully specified**. All three SDKs expose `build_strategy_from_config(pipeline_config, …)`, which applies `remove`, then `configure` against a **closed** field set, then `steps`, with its own error code (`PIPELINE_CONFIGURATION_ERROR`) and its own contract in `DECLARATIVE_CONFIG_SPEC` §4. `schemas/apcore-config.schema.json` declares all three keys.

Its first parameter is a dict the **caller** supplies. Nothing extracts the `pipeline:` section from a loaded `Config` and passes it — `APCore` never calls the builder. Measured:

```yaml
pipeline:
  remove: [acl_check]
```

...leaves all eleven steps in place, `acl_check` included.

**Decision needed**

Whether a `pipeline:` block in `apcore.yaml` does anything.

**Options**

- **A — wire it.** Have the client read the `pipeline` section and build its strategy through `build_strategy_from_config` when the section is present.
- **B — withdraw the three keys** from `apcore-config.schema.json` and document the builder as an API-only facility that takes a dict the application supplies from wherever it likes.

**Recommendation: A, and this one should be ranked first of the eight.**

The failure is not symmetric. `pipeline.remove: [acl_check]` failing to remove ACL is fail-safe. But `pipeline.steps` failing to *insert* is not: an operator who declares a custom step for audit logging, rate limiting or an authorization gate gets a client that **silently never runs it**, with no error and no warning, and the pipeline they inspect in configuration is not the pipeline that executes. It is the **most direct** route from an inert key to a security or audit control that does not run.

Not the only one, though — an earlier draft claimed it was, and that is wrong. **`extensions.ignore_patterns`** is inert in all three SDKs while §3.6's `scan_extensions` (A04) step 3a is a **MUST** written about it — *"if entry name matches ignore_patterns → Skip"* — so a project that excludes a directory from discovery has it scanned and its modules registered anyway. Same class, opposite direction: a rule that fails **open** rather than a step that fails to run. It is absent from this file only because it has no keep-or-withdraw question outstanding — the key is simply unimplemented and needs implementing, tracked on #118.

Against A: wiring it means a `pipeline:` block starts changing execution in a project that has one today and is currently ignored. That population is not empty in principle — the section is schema-valid and documented — though every such project is already running the default pipeline, so what changes is that they get what they asked for.

**Owner**: maintainer.

**Action once decided**

- If A: wire at client construction in three SDKs, add a conformance fixture (a removed step, a configured field, an inserted step), promote all three keys to `live`.
- If B: deprecation window per §13.4 for all three, and a note in `DECLARATIVE_CONFIG_SPEC` §4 that the section is not read from `apcore.yaml`.

**Resolved 2026-09-11 — A, implemented.** Spec v1.43.0 adds §5.16 requirements 6 and 7. Wired at
executor construction in all three SDKs, with an explicit strategy still winning per D-73's
precedence (API argument > `Config` > declared default). `pipeline_section_wiring.json` (6 cases)
drives every case through client construction from a full configuration document, because the three
fixtures that already covered this surface hand the section straight to the builder — which was
never the broken part — and stayed green throughout. All three keys are now `live` with behavioural
probes in `config_key_consumers.json`; the `inert` ceiling falls 25 → 21 (`extensions.ignore_patterns`
was wired in v1.42.0 and is promoted in the same change).

The migration guard is a **warning**, not a confirmation key. Three reasons: a confirmation key is
itself a newly declared key, which is exactly what D-73 exists to constrain, and 2.0 would have to
deprecate it again; §9.2.2 already establishes the "warn once per configuration load" cadence for
*your configuration is about to mean something different*, so reusing it is consistent; and turning
a currently ignored `pipeline:` into an **error** would break projects whose block is harmless,
which is worse than the status quo.

---

## D-73 — the general rule: may a declared key be reachable only through an API?

**Tracking issue**: [#118](https://github.com/aiperceivable/apcore/issues/118).

**Status quo**

Four separate entries above (D-66's `acl.audit`, D-71's `id_map.overrides`, D-72's `pipeline.*`, and the already-recorded `acl.default_effect`) are the same shape: a mechanism reachable through a constructor argument or function parameter, and a configuration key naming it that reaches nothing. Nothing in the specification forbids that arrangement, which is why it arose four times independently.

It also has an auditing consequence, learned the hard way: **a probe that exercises the mechanism through the API door reports the key as working.** `acl.default_effect` was recorded `live` in the guard built to find keys nothing reads, for exactly that reason.

**Decision needed**

Whether the specification states a rule.

**Options**

- **A — a declared configuration key MUST reach its mechanism from a `Config`.** A facility reachable only through an API argument MUST NOT have a key in the canonical schemas.
- **B — no rule**; judge case by case, and rely on `check_config_key_consumers.py` to keep the count honest.

**Recommendation: A — but the rule needs two definitions, or it leaves open the gap it closes.**

Raised in review, and correct: "from a `Config`" is not self-evident, and a rule stated loosely would let the same shape back in through a narrower door.

1. **What counts as a configuration entry point.** All three of these, equally: a `Config` produced by a **file load** (`Config.load`), one carrying an **environment override** (`APCORE_*`), and one **constructed programmatically** (`Config({…})` / `set()`). A value reaching the mechanism from any of them satisfies the rule. What does **not** satisfy it is the shape this decision exists to forbid — a **direct constructor or function argument that bypasses `Config` entirely**: `Registry(id_map_path=…)`, `build_strategy_from_config(pipeline_config, …)`, `ACL(default_effect=…)`.

2. **Precedence, stated rather than left to each implementation.** An explicit **API argument wins over `Config`, which wins over the declared default.** Without this, the rule creates a fresh ambiguity for every key that then has both doors — and "both are wired, which one applies" is the question that produced §9.2.2's project-root divergence one layer up. The API argument winning is also what keeps A from being a breaking change: every current caller passing the argument keeps its behaviour, and the key becomes the fallback it always looked like.

With those two clauses the rule is testable, and `check_config_key_consumers.py`'s `live` criterion is already its enforcement: a probe must put the key into a `Config` and observe the effect, which is clause 1 restated.

The guard catches this class today, but only *after* a key ships and only for keys someone thought to probe. A rule in §9.1 makes the class unrepresentable — the same move §9.2.3 made for pattern dialects: name the requirement once rather than fix its instances one at a time. Four independent instances are the argument for it.

**Owner**: maintainer.

**Action once decided**

- If A: add the requirement to §9.1 with a version bump, including both clauses above; cite it from `config_key_consumers.json`'s status definitions; and audit the four known instances against it as the rule's first application.

---

## Resolution status

| # | Subject | Recommendation | Decided |
|---|---|---|---|
| D-66 | `acl.audit` — two homes | A (ACL file's block survives) — **define the delivery contract first** | — |
| D-67 | logging keys at v2.0 | B (withdraw, and say so) | — |
| D-68 | observability model | C (complete tracing, withdraw metrics) | — |
| D-69 | `_config.allow_unknown` | A (implement §9.6.3's row) | — |
| D-70 | `extensions.roots` | A (converge on Rust) | — |
| D-71 | `id_map.overrides` | A (wire it) | — |
| D-72 | `pipeline.*` | A (wire it) — **rank first** | — |
| D-73 | API-only keys, in general | A (state the rule, **with both clauses**) | — |

**Suggested sequence.** D-72 first: it is the most direct route from an inert key to a security or audit control that does not run. Then D-68's additive half (`observability.tracing.strategy`), because adding must precede removing and the v2.0 removals are already scheduled. Then D-66, so the deprecation notices can name a migration target while the window is still open. D-69, D-70 and D-71 are independent and small. D-73 last, since it is best written once the four instances have been resolved and their shape is settled.
