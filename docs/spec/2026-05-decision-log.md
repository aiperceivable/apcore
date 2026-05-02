---
title: Cross-language alignment decision log (2026-05)
date: 2026-05-02
status: draft
audience: maintainers + spec reviewers
source: /apcore-skills:sync findings (2026-05-02 run, 20/20 modules covered)
---

# Cross-language alignment — open decisions

This document lists every cross-language inconsistency that the May 2026 sync run surfaced as **spec ambiguity** rather than implementation bug. Each item is something that needs a maintainer decision before code can be aligned across Python, TypeScript, and Rust SDKs.

Items already fixed (28 commits across 3 SDKs) are not in this log — see `CHANGELOG.md` of each SDK and the sync report.

Decision template per item:
- **Status quo**: what each SDK does today
- **Options**: A / B (sometimes C)
- **Recommendation**: maintainer's first-pass call (subject to review)
- **Owner**: who needs to sign off
- **Action**: concrete change once decided

---

## D-01 — IdentityType: closed enum vs free-form

**Status quo**
- `PROTOCOL_SPEC.md §5.7` declares `enum: [user, service, agent, api_key, system]`
- `docs/features/identity-system.md:80` says "type field is free-form"
- `docs/features/identity-system.md:10` and `core-executor.md` / `approval-system.md` examples reference `"ai"` as a value
- All 3 SDKs accept any string at construction (no validation)

**Options**
- **A** Closed enum: pin §5.7 list, add `ai` to it explicitly, validate at Identity construction in all 3 SDKs (rejects unknown types with `InvalidInputError`)
- **B** Free-form: drop §5.7 enum, document the 5–6 well-known values as a non-binding convention, no SDK validation

**Recommendation**: **B**. Identity type is observably free-form today; closing the enum is breaking for any consumer that uses non-standard values (custom service types, cluster role names). Document the 5–6 well-known values as `examples` rather than `enum` in the JSON schema.

**Action**: edit PROTOCOL_SPEC §5.7 to remove `enum:` and add `examples: [user, service, agent, api_key, system, ai]`. Update identity-system.md to remove the contradictory "free-form" sentence and reference the examples list.

---

## D-02 — ApprovalStatus enum value: `granted` vs `approved`

**Status quo**
- Every SDK uses `"approved"` literal (Python `Literal['approved', ...]`, TS string union, Rust comment)
- `conformance/fixtures/approval_gate.json` uses `"approved"`
- `docs/features/approval-system.md:90` uses `"approved"`
- The agent-prompt-level spec text used `"granted"` — this was the wording mismatch flagged in AP-002

**Options**
- **A** Keep `"approved"` (matches all implementations + fixtures + feature spec doc)
- **B** Change to `"granted"` (matches the agent-prompt spec text)

**Recommendation**: **A**. Implementation, fixtures, and the canonical feature spec doc all agree on `"approved"`. The `"granted"` variant only appeared in derivative prompt text. No code change needed; close the finding and update any derivative prompt/audit material that says `granted`.

**Action**: grep for `granted` across spec docs and replace with `approved`. No SDK change.

---

## D-03 — ApprovalRequest fields: add `caller_id` and `action`?

**Status quo**
- `docs/features/approval-system.md` Contract block: "request MUST contain `module_id`, `caller_id`, and `action`"
- Every SDK only has `module_id, arguments, context, annotations, description, tags` — no `caller_id`, no `action`
- Handlers can derive `caller_id` from `context.identity` and `action` from `module_id` today

**Options**
- **A** Add both fields to ApprovalRequest in all 3 SDKs; populate from context+module_id in BuiltinApprovalGate
- **B** Drop the requirement from the spec — document that handlers should derive these from `context.identity` and `module_id` themselves

**Recommendation**: **A**. The fields are cheap to add and they let handlers (Slack approver, audit log) inspect a single flat record without traversing context. Spec is authoritative; align implementations.

**Action**: 
- Add `caller_id: Optional[str]` and `action: str` to ApprovalRequest in all 3 SDKs
- BuiltinApprovalGate populates: `caller_id = context.caller_id`, `action = module_id`
- Add conformance fixture asserting both are present in the request payload
- Bump SDK version to 0.21.0; CHANGELOG ### Added

---

## D-04 — Audit-entry & error wire format: snake_case vs camelCase (TS outlier)

**Status quo**
- Python: `to_dict()` emits `trace_id`, `ai_guidance`, `user_fixable`; details keys are snake_case
- TS: `toJSON()` emits `traceId`, `aiGuidance`, `userFixable`; details keys are camelCase; AuditEntry interface uses camelCase keys
- Rust: serde defaults (snake_case)
- `CLAUDE.md` writing-rules section states: "use `caller_id` / `target_id` / `module_id` (not `caller` / `target` alone)"
- `propagate_error` in TS even produces duplicate keys (`module_id` + `moduleId`) on the same payload because the constructor wrote camelCase but the helper appends snake_case

**Options**
- **A** Make snake_case the canonical wire form: fix TS to emit snake_case in `toJSON`, details, AuditEntry. Add deprecation aliases (camelCase getters that warn) for one minor version. Internal TS code can keep camelCase — only wire/serialization changes.
- **B** Make camelCase the canonical wire form: change Python `to_dict` and Rust serde renames. Largest disruption (Python users + Rust JSON consumers both affected).
- **C** Allow both forms simultaneously: emit canonical (snake) but accept both on parse. Reduces breakage but complicates consumers.

**Recommendation**: **A**. snake_case is already the protocol convention (per CLAUDE.md, PROTOCOL_SPEC, type-mapping). TS is the only outlier. Fixing TS aligns with 2 of 3 SDKs and the spec text. Add deprecation period: `errorOnDeprecated: false` mode logs warning when old key is used.

**Action** (deferred — needs dedicated PR):
A first attempt was made on 2026-05-02 (Step C of this decision-log batch) but stalled mid-rename. The rename touches:
- `src/errors.ts` — ~50 error subclass constructors writing detail keys (`moduleId`, `callerId`, `targetId`, `modulePath`, `callableName`, `approvalId`, `errorCode`, ...)
- `src/sys-modules/audit.ts` — AuditEntry interface fields (`targetModuleId`, `actorId`, `actorType`, `traceId`)
- `src/utils/error-propagation.ts` — duplicate-key writes
- ~30 test files that reference the old field names directly (`err.traceId`, `err.details.moduleId`, `entry.actorId`)

Plus deprecation getters on ModuleError instances for `traceId` / `aiGuidance` / `userFixable` with one-shot deprecation warnings.

**Why deferred**: the mechanical scope is too brittle for an unattended sub-agent. Recommend a dedicated PR with a codemod (jscodeshift or simple sed/AST rename) that:

1. Scans every test file for `\.(traceId|aiGuidance|userFixable)\b` and `details\.(moduleId|callerId|...)` — produces a rename plan
2. Applies the source-side rename atomically (errors.ts + audit.ts + error-propagation.ts in one commit)
3. Applies the test-side rename atomically (one commit per test directory)
4. Adds the deprecated camelCase getters with one-shot console.warn (separate commit)
5. Updates CHANGELOG.md `### Changed` with a v0.21.0 deprecation notice + removal target v0.22.0

Expected commit count: 4–6.
Expected duration: 1–2 hours of focused work.

Blocking factor: needs a dev session with full test-suite re-validation, not an unattended agent.

---

## D-05 — `Module.stream()` fallback: implement everywhere?

**Status quo**
- Spec MUST: `Module.stream()` defaults to `execute()` wrapped as a single chunk
- Python+TS implement the fallback; Rust returns `streaming_not_supported_error()` (CRITICAL — fixed in round-3)

**Status**: **resolved** in round-3 commit `b89b7a5`. Mention here for completeness; no further action.

---

## D-06 — `multi_class_enabled` config plumbing

**Status quo**
- Spec: there's a global config key `extensions.multi_class_discovery` (boolean)
- Python: no enabled flag at all; per-class `@multi_class` decorator opt-in
- TS: function arg `multiClassEnabled: boolean = false`
- Rust: struct field `DiscoveryConfig.multi_class`
- **None of the 3 SDKs reads `extensions.multi_class_discovery` from Config** — the global toggle is unimplemented everywhere

**Options**
- **A** Implement the global toggle: each SDK reads `Config.get("extensions.multi_class_discovery", false)` at registry init; per-class decorator/marker overrides
- **B** Drop the global toggle: spec says per-class decorator is the only opt-in (matches Python today)
- **C** Make it explicit per-call: keep TS+Rust function-param style; remove the global config key entirely

**Recommendation**: **B**. Per-class decorator (Python's design) is more composable and easier to reason about (each class declares its own intent). The global toggle is dead documentation today.

**Action**: 
- Remove the global `extensions.multi_class_discovery` config key from PROTOCOL_SPEC + multi-module-discovery.md
- TS `discoverMultiClass(filePath, classes, extensionsRoot)` — drop the `multiClassEnabled` arg (TS still needs the per-class marker; document how to pass it)
- Rust `DiscoveryConfig` — keep `multi_class` for explicit-config use, but document as Rust-only convenience for callers that bypass the per-class marker

---

## D-07 — `FrequencyExceededError` vs `CallFrequencyExceededError`

**Status quo**
- `docs/features/call-chain-guard.md:206` (Contract block) lists `FrequencyExceededError(code=FREQUENCY_EXCEEDED)`
- The same file's Error Types table at line 92-96 lists `CallFrequencyExceededError(code=CALL_FREQUENCY_EXCEEDED)`
- All 3 SDKs implement `CallFrequencyExceededError` with code `CALL_FREQUENCY_EXCEEDED`

**Options**
- **A** Fix the doc Contract block (canonical name = `CallFrequencyExceededError`)
- **B** Rename in all 3 SDKs to match the Contract block (`FrequencyExceededError`)

**Recommendation**: **A**. Implementation + Error Types table + ACL/spec usage all agree on `CallFrequencyExceededError`. Only the Contract block heading is stale. Single-line doc fix.

**Action**: edit `docs/features/call-chain-guard.md:206` Contract block heading to `CallFrequencyExceededError(code=CALL_FREQUENCY_EXCEEDED)`.

---

## D-08 — `RetryConfig.compute_delay_ms` method name canonicalization

**Status quo**
- Python: `compute_delay_ms(attempt)` returns `float` ms
- TS: `computeDelay(attempt)` returns `number` ms
- Rust: `delay_for_attempt(attempt)` returns `u64` ms

All three produce identical numerical output for integer attempts. Only the method name differs.

**Options**
- **A** Pin `compute_delay_ms` as canonical (Python current). Rename TS+Rust.
- **B** Pin `compute_delay` (TS-style, drop unit suffix). Rename Python+Rust.
- **C** Pin `delay_for_attempt` (Rust-style). Rename Python+TS.

**Recommendation**: **A** (`compute_delay_ms`). The unit suffix `_ms` is helpful (TS and Python both use ms units already; the suffix avoids ambiguity).

**Action**:
- TS: rename `computeDelay` → `computeDelayMs`. Add `computeDelay` as deprecated alias for one minor version.
- Rust: rename `delay_for_attempt` → `compute_delay_ms`. Same pattern.

---

## D-09 — APCore lifecycle: `close()` vs `start()` vs `stop()`

**Status quo**
- Python: only `close()` (releases the cached event loop). No `start()`, no `stop()`.
- TS: no `close()`, no `start()`, no `stop()`.
- Rust: no `close()`, no `start()`, no `stop()`. Has `reload()` (Rust-only).
- Spec text mentions `start()` and `stop()` but no SDK implements either.

**Options**
- **A** Drop `start()`/`stop()` from spec, document `close()` as Python-only convenience
- **B** Add `start()`/`stop()` to all 3 SDKs (no-op default, override-able for async setup)
- **C** Standardize on async `shutdown()` (TS+Rust gain it; Python `close()` remains)

**Recommendation**: **A**. `start()` and `stop()` are aspirational and have no clients. Python's `close()` is mainly for tests cleaning up. No real lifecycle need exists in the 3 SDKs as currently designed.

**Action**: remove `start()` and `stop()` mentions from PROTOCOL_SPEC §12 and feature specs. Document Python `close()` as a Python-only convenience for releasing the cached event loop.

---

## D-10 — `TaskStore` protocol method names: `put` vs `save`, `list_expired` presence

**Status quo**
- Python: `get / put / delete / list` — uses `put`, lacks `list_expired`
- TS: `save / get / list / delete / listExpired` — full spec surface
- Rust: `save / get / list / delete / list_expired` — full spec surface
- Spec contract requires `save / get / list / delete / list_expired`

**Options**
- **A** Align Python with spec: rename `put` → `save`, add `list_expired`. Add deprecated alias `put` calling `save` for one minor version.
- **B** Change spec + TS+Rust to use `put` (Python-style) and drop `list_expired` from spec

**Recommendation**: **A**. TS and Rust already match the spec; Python is the outlier. The rename is a 1-day fix with deprecation alias.

**Action**:
- Rename Python `TaskStore.put` → `save`, add deprecation shim
- Add `TaskStore.list_expired(before_timestamp)` to Python (mirrors TS+Rust)
- Update `InMemoryTaskStore` and any tests
- CHANGELOG ### Changed for v0.21.0

---

## D-11 — `start_reaper` signature alignment

**Status quo**
- Python: `start_reaper(interval_seconds=3600.0, max_age_seconds=3600.0)` — sync, returns None, args swapped, units in **seconds**
- TS: `startReaper({ttlSeconds, sweepIntervalMs})` — async, returns ReaperHandle, sweepIntervalMs in **ms**
- Rust: `start_reaper(config: ReaperConfig{ttl_seconds, sweep_interval_ms})` — sync (returns ReaperHandle), ttl in seconds, sweep in ms

**Options**
- **A** Standardize on `(ttl_seconds: u64, sweep_interval_ms: u64) -> ReaperHandle`. Rename Python's args; switch sweep unit to ms; return `ReaperHandle`.
- **B** Take a `ReaperConfig` struct (Rust-style) everywhere; Python `dataclass`, TS `object`.

**Recommendation**: **A**. Two named arguments are simpler than a config struct for a 2-field signature. Python is the outlier on units (seconds vs ms) and on return type (None vs ReaperHandle).

**Action**:
- Python: rename `interval_seconds` → `sweep_interval_ms` (multiply previous values by 1000); rename `max_age_seconds` → `ttl_seconds`; return ReaperHandle. Add deprecation alias for old args.
- Cross-SDK: ensure `ReaperHandle.stop()` is async in TS+Rust+Python.

---

## D-12 — `TaskStatus` enum: drop Python's `RETRYING`?

**Status quo**
- Python: PENDING, RUNNING, COMPLETED, FAILED, CANCELLED, **RETRYING** (extra)
- TS: PENDING, RUNNING, COMPLETED, FAILED, CANCELLED
- Rust: Pending, Running, Completed, Failed, Cancelled

Python sets RETRYING during backoff; TS+Rust set it back to PENDING.

**Options**
- **A** Remove Python's RETRYING; use PENDING during backoff (matches TS+Rust)
- **B** Add RETRYING to TS+Rust spec
- **C** Keep RETRYING but document as a Python-only intermediate state that observers should treat as PENDING

**Recommendation**: **A**. Spec lists 5 states; Python's RETRYING is an internal observability shim that leaks into the public API. Cleaner to use PENDING for the backoff window and expose retry count as a separate `retry_count` field (already present).

**Action**: Python `task_info.status = TaskStatus.PENDING` during backoff; remove `RETRYING` from the enum (or keep deprecated). Update tests + docs.

---

## D-13 — `TaskInfo.retry_count` field name in Python

**Status quo**
- Python: field is named `attempt_number`
- TS: `retryCount`
- Rust: `retry_count`
- Spec contract: `retry_count`

**Options**
- **A** Rename Python field to `retry_count` (with deprecated `attempt_number` property)

**Recommendation**: **A**. Single-field rename; trivial.

**Action**: edit Python `TaskInfo` dataclass; add `@property attempt_number` returning `retry_count` for one minor version.

---

## D-14 — `RetryConfig::default()` in Rust: `max_retries=3` vs spec `max_retries=0`

**Status quo**
- Python: `max_retries=0`
- TS: `maxRetries=0`
- Rust: `max_retries=3` (defaults to 3 retries automatically — surprises callers using `..Default::default()`)
- Spec: 0

**Recommendation**: change Rust default to 0. Single-line fix.

**Action**: edit `apcore-rust/src/async_task.rs:193-202` Default impl. Tests update. CHANGELOG ### Changed.

---

## D-15 — `Registry.discover_multi_class`: free function vs Registry method?

**Status quo**
- Python: free function `discover_multi_class(file_path, extensions_root, pre_approval_hook)`
- TS: free function `discoverMultiClass(filePath, classes, extensionsRoot, multiClassEnabled)`
- Rust: split as `Registry::register_multi_class(...)` + `derive_module_ids(...)` free function
- Spec contract: names it `Registry.discover_multi_class` (a method)

**Options**
- **A** Add a `Registry.discover_multi_class` method on the Registry class in Python+TS, keep the free function as `_discover_multi_class` internal helper. Aligns surface with spec.
- **B** Update spec to describe the function as a free helper (matches Python+TS today). Document Rust's split as a Rust-only convenience.

**Recommendation**: **A**. Spec authority dictates. The method form is also more discoverable (`registry.discover_multi_class(file)` reads better than `from apcore.registry.multi_class import discover_multi_class`).

**Action**: add `Registry.discover_multi_class(file_path)` method to Python+TS that wraps the free function. Mark the free function as internal (rename to `_discover_multi_class` in TS, leave Python module-private).

---

## D-16 — `Module.stream` Rust signature in docs is wrong

**Status quo**
- `docs/features/streaming.md:81-109` shows Rust: `async fn stream(...) -> Result<Vec<Value>, ModuleError>` (returning a buffered Vec)
- Actual Rust trait: `fn stream(&self, ...) -> Option<ChunkStream>` where `ChunkStream` is a `Pin<Box<dyn Stream<...> + Send>>`

**Options**
- **A** Fix the doc to match the implementation

**Recommendation**: **A**. Documentation bug.

**Action**: rewrite the Rust example in `streaming.md:81-146` to use `StreamExt::next()` polling.

---

## D-17 — `caller_id_for_unknown` configurable?

**Status quo**
- All 3 SDKs hardcode `"@external"` for missing caller_id
- Spec implies it should be configurable

**Options**
- **A** Add constructor param `caller_id_for_unknown: str = "@external"` to ACL in all 3 SDKs
- **B** Pin `"@external"` as a constant in the spec; document as immutable

**Recommendation**: **B**. Configurability adds complexity for a value that's only used internally for ACL evaluation labeling. Compliance-driven users can wrap calls with their own caller_id resolver.

**Action**: edit `acl-system.md` to remove the configurability suggestion; add a constant `EXTERNAL_CALLER_ID = "@external"` to each SDK so it's discoverable.

---

## D-18 — ACL priority field: implement deny-wins-at-equal-priority?

**Status quo**
- Spec Algorithm A09: sort rules by priority, deny-before-allow at equal priority
- ACLRule in all 3 SDKs has NO priority field
- All 3 iterate in insertion order; `add_rule` inserts at index 0 (newer wins as a de-facto priority surrogate)

**Options**
- **A** Add `priority: int = 0` to ACLRule, implement sort + deny-tiebreak in all 3 SDKs
- **B** Drop the priority+tiebreak language from Algorithm A09; document insertion-order-with-add-at-front as canonical

**Recommendation**: **B**. Insertion-order is simpler and matches all 3 implementations today. The deny-tiebreak is rarely a real-world ergonomic issue (users can always reorder rules). If priority becomes needed in v1.0, add it then.

**Action**: edit Algorithm A09 to drop sort+tiebreak steps; document insertion-order-first-match-wins.

---

## D-19 — Stream chunk shape: dict-only vs any JSON?

**Status quo**
- Python `_deep_merge` requires dict (TypeError on non-dict)
- TS `deepMergeChunk` requires Record<string, unknown> (silent replace at top level)
- Rust `deep_merge_value` accepts any Value variant
- Spec docs/streaming.md:228 says chunks are "partial output dicts"

**Options**
- **A** Standardize: chunks must be objects (dicts/Records). Rust adds shape check; Python+TS already enforce.
- **B** Allow any JSON value: relax Python+TS to accept arrays/scalars; standardize the merge as "replace if non-object on either side, deep-merge if both objects".

**Recommendation**: **A**. The merge semantics for non-object chunks are unclear (replace? append?), and most modules emit objects naturally.

**Action**: Rust adds a `is_object()` check before `deep_merge_value`; raise InvalidInputError on non-object chunk. Spec text already implies this.

---

## D-20 — `compute_delay` rounding for fractional values

**Status quo**
- Python returns `float` ms
- TS returns `number` ms
- Rust returns `u64` ms (truncates fractional component)

For backoff_multiplier=2.5 and retry_delay_ms=1000, attempt=2: Python/TS return 6250.0, Rust returns 6250 (lossless). For backoff_multiplier=1.7 and attempt=3: Python/TS return 4913.0, Rust returns 4913. No realistic divergence at default values, but precision differences exist for non-integer multipliers at high attempts.

**Recommendation**: keep current behavior. Document Rust's `u64` return as "ms truncated to integer" — a no-op divergence in practice.

**Action**: no code change. Add a one-line note in `RetryConfig::compute_delay_ms` doc comment.

---

## D-21 — `apply()` idempotency for ExtensionManager

**Status quo**
- Python+TS: `apply()` preserves registered extensions; second call re-stacks
- Rust: `apply()` drains extensions; second call is a no-op (CRITICAL EXT-001)

**Options**
- **A** Migrate Rust ExtensionKind from `Box<dyn Trait>` to `Arc<dyn Trait>`; rewrite apply() to use `clone()` semantics. Cascading API changes to Executor::set_acl/set_approval_handler/use_middleware and Registry::set_discoverer/set_validator.

**Recommendation**: **A**. Required for cross-language parity, but it's a multi-API breaking change. Track as RFC + epic for v0.21.0 or v0.22.0. (Already deferred from round-3 fix cycle.)

**Action**: open an RFC issue. Schedule agent in 2 weeks to draft the migration plan.

---

## D-22 — `ExtensionManager.get/get_all/unregister` API

**Status quo**
- Python+TS: present
- Rust: missing (replaced with `count`/`has`/`clear`/`clear_all`)

**Options**
- **A** Bundle with D-21 Arc migration. Add the spec API alongside.

**Recommendation**: **A** (paired with D-21).

---

## D-23 — `RefResolver` max_depth integration

**Status quo (post-fix)**
- Round-3 added `RefResolver::with_max_depth` and depth tracking in `resolve_inner`. The default constructor uses 32. Resolved.

**Status**: **resolved** in commit `5914dd5`. Listed for trace.

---

## D-24 — `update_config` constraint registry & rollback

**Status quo**
- Python: full _CONSTRAINTS table with rollback on validation failure
- TS+Rust: no constraint registry, no rollback path; invalid values silently persist
- Spec MUST: rollback on ConfigError postcondition

**Options**
- **A** Port Python's _CONSTRAINTS table + post-set validation + rollback to TS+Rust
- **B** Drop the rollback requirement from spec (treat as Python-only safety net)

**Recommendation**: **A**. Spec contract is clear; rollback is a real protection against bad config writes. Implement in TS+Rust.

**Action**: 
- TS: add `_constraints` Map and `_validate_post_set` to `update_config` module; rollback via `Config.set(key, old_value)` on validation failure
- Rust: same pattern using a static constraints registry

---

## D-25 — `update_config` restricted-key error code

**Status quo**
- Spec: `CONFIG_KEY_RESTRICTED`
- Python: matches spec
- TS: raises `ConfigError` (no specific code)
- Rust: returns `ModuleError(GeneralInvalidInput)` (no specific code)

**Recommendation**: align TS+Rust to use `CONFIG_KEY_RESTRICTED`.

**Action**: TS adds `ErrorCodes.CONFIG_KEY_RESTRICTED`; Rust adds `ErrorCode::ConfigKeyRestricted` variant. Update `update_config` to use it.

---

## D-26 — Identity hashability

**Status quo**
- Python: claims frozen but unhashable (dict field). `hash(identity)` raises TypeError.
- TS: interface, no equality helper
- Rust: full Hash impl

**Options**
- **A** Make Identity hashable everywhere: Python uses tuple-of-tuples or FrozenDict for attrs; TS adds `identityEquals/identityHash` helpers
- **B** Document Identity as not-required-to-be-hashable; Rust's Hash impl is a Rust-only convenience

**Recommendation**: **B**. Cross-language hashability requires changing Python's attrs to a frozen-dict type that downstream Python code might find unfamiliar. Not worth the complexity for an unclear use case.

**Action**: edit identity-system.md to clarify Identity equality semantics: "Identity is a value type; equality is structural (id+type+roles+attrs); hashability is implementation-defined per language."

---

## D-27 — UsageCollector trend / period filtering / record timestamp

**Status quo**
- Python+TS: full implementation (trend computed, record accepts timestamp, summary accepts period)
- Rust: trend hardcoded "stable", record ignores timestamp, summary has no period filter

**Options**
- **A** Port Python/TS trend computation + period filter + record timestamp param to Rust

**Recommendation**: **A**. Core spec functionality. Should be done in Rust v0.21.0.

**Action**: implement in Rust UsageCollector. Track as 3 separate fix issues (OBS-001, OBS-002, OBS-003 already exist).

---

## D-28 — ContextLogger output schema (Rust uppercase + flattened extra)

**Status quo**
- Python+TS: lowercase level, nested `extra` key, `module_id` field
- Rust: UPPERCASE level, flattened extra (no `extra` key wrapper), `module` field (not `module_id`)

**Options**
- **A** Align Rust to Python+TS output shape

**Recommendation**: **A**. Log aggregators need consistent field names and casing.

**Action**: edit `apcore-rust/src/observability/logging.rs` to lowercase level_name, nest extra under `extra` key, rename `module → module_id` and `input → inputs` in middleware extras.

---

## D-29 — `EventEmitter.shutdown` parity (already partially fixed)

**Status quo (post-round-2)**
- Round-2 added `shutdown()` to Rust events. Python had it. TS gained it.
- Spec parity now achieved.

**Status**: **resolved** in round-2 commits.

---

## D-30 — `discover_multi_class` `pre_approval_hook` Python-only param

**Status quo**
- Python's `discover_multi_class` accepts a third `pre_approval_hook` parameter for file-import safety; TS+Rust don't have it (they don't import files at runtime — caller pre-resolves).

**Options**
- **A** Document as Python-only convenience: Python imports, the hook is its safety check; TS+Rust callers handle file safety themselves
- **B** Add equivalent hook to TS+Rust (e.g., a path-allowlist check before reading the file system)

**Recommendation**: **A**. Python's hook protects against importing arbitrary code; TS+Rust never import code from disk for this path. The asymmetry reflects real architectural difference.

**Action**: document in multi-module-discovery.md that `pre_approval_hook` is a Python-only construct; TS+Rust callers should sandbox file reads.

---

## D-31 — File-extension scope for discovery

**Status quo**
- Python: `.py` only
- TS: `.ts` and `.js` (skip `.d.ts`, `.test.*`, `.spec.*`)
- Rust: `[".rs"]` (configurable via `with_extensions`)
- Spec: silent

**Options**
- **A** Document the per-language file-extension defaults explicitly in `multi-module-discovery.md`
- **B** Make extensions a config knob in all 3 SDKs (already in Rust; add to Python+TS)

**Recommendation**: **A**. Sensible defaults per language are unsurprising. Document the differences and the test-file skip patterns.

**Action**: add a "File extensions and skip patterns" section to multi-module-discovery.md.

---

## D-32 — Discovery pipeline stage count alignment

**Status quo**
- Spec doc references "8 stages" (matches Rust `default_discoverer.rs`)
- Python `_discover_default` doc claims 7 stages, actually has 8 helpers
- TS `_discoverDefault` has only 6 helpers (conflict batching folded into `_registerInOrder`)

**Options**
- **A** Refactor TS to expose the 8-stage shape explicitly (extract `_filterIdConflicts` as a separate stage)
- **B** Update spec doc + Python docstring to "6–8 stages, language-dependent"; document that conflict batching MAY be inlined

**Recommendation**: **A**. The 8-stage breakdown matches the spec algorithm A04 better; TS just inlined for code golf and lost the conformance signal.

**Action**: refactor TS `_discoverDefault` to expose `_filterIdConflicts` and `_registerInOrder` as separate stages.

---

## Resolution status

- **Resolved** (no further action): D-02, D-05, D-23, D-29
- **Doc-only fixes** (1-line each): D-01, D-07, D-09, D-16, D-17, D-18, D-26, D-30, D-31
- **Code fix needed in 1 SDK**: D-04 (TS), D-08 (Python), D-10 (Python), D-11 (Python), D-12 (Python), D-13 (Python), D-14 (Rust), D-19 (Rust), D-25 (TS+Rust), D-27 (Rust), D-28 (Rust), D-32 (TS)
- **Multi-SDK code fix**: D-03 (all 3), D-15 (Python+TS), D-24 (TS+Rust)
- **Epic / RFC needed**: D-21, D-22 (extension Arc migration)

## Recommended sequence

1. Apply doc-only fixes as a single PR in `apcore` (D-01, D-07, D-09, D-16, D-17, D-18, D-26, D-30, D-31). Low risk.
2. Apply single-SDK code fixes per repo as separate PRs:
   - TS D-04 (snake_case wire alignment) — needs codemod tooling, see action note
   - Python D-10..D-13 (TaskStore + start_reaper alignment)
   - Rust D-14, D-19, D-27, D-28 (RetryConfig default, chunk shape, UsageCollector parity, ContextLogger schema)
3. Multi-SDK fixes (D-03 ApprovalRequest fields, D-15 discover_multi_class method, D-24 update_config rollback) batched into a v0.21.0 release.
4. Epic D-21/D-22 (Extension Arc migration) — RFC scheduled for 2026-05-18 (routine `trig_01DCE8sCcBxm9qRxZiMvUgQo`), target v0.22.0.
