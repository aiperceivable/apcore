# Project Review: apcore-js

**Date:** 2026-04-18
**Reviewer:** code-forge
**Overall Rating:** pass_with_notes
**Merge Readiness:** fix_required
**Scope:** full (75 source files under `src/`)
**Reference:** bare (no planning/ or docs/ directory — internal-consistency mode)

## Summary

The codebase is solid overall — architecture is clean, TypeScript usage is disciplined (ESM `.js` import convention observed throughout, minimal `any`), and the dependency footprint is lean. The sub-agent analyzed 96 public symbols via call-graph pre-analysis and found 24 issues, concentrated in three areas: (1) **error-handling divergence** between streaming and non-streaming executor paths; (2) **observability gaps** around fs.watch and onUnload that silently swallow real failures; (3) **dead-ergonomics features** — `RetryMiddleware` advertises behavior the pipeline doesn't actually implement, and `AsyncTaskManager.cancel` does not interrupt in-flight work.

No production-risk blockers were found, but three `critical` correctness issues should be fixed before the next release: a silently-bypassed version constraint in `Registry._resolveLoadOrder`, the `MiddlewareChainError` wrapper losing original error classes in `Executor.call`, and the streaming path swallowing post-stream validation errors. Several D15 findings point to duplicative work across sys-modules, schema-export, and logger/ContextLogger — tactical cleanups, not rewrites.

**Issue Breakdown:** 0 blockers · 3 critical · 11 warnings · 10 suggestions

**Call-Graph Coverage:** 96 public symbols analyzed · 15 partial chains · 0 suspicious chains · ⚠ A tail of ~35 private/internal helpers deferred with reason `scope-too-large` (version.ts sort helpers, metrics.ts labelsKey formatters, config.ts env-resolution helpers, etc.). Deferred symbols are mostly leaf helpers, so primary-path coverage is complete; consider a targeted follow-up review on `config.ts` (958 LOC) if that file sees changes.

---

## Tier 1 — Must-Fix Before Merge

### Functional Correctness (D1)

**Rating:** warning

| Severity | File | Line | Title |
|---|---|---|---|
| :warning: critical | src/registry/dependencies.ts | 42-53 | Version constraint silently ignored when dep target's version is missing from moduleVersions |
| :warning: critical | src/executor.ts | 333-335 | MiddlewareChainError remap drops original error code |
| :warning: critical | src/executor.ts | 481-491 | Streaming Phase 3 silently swallows validation + middleware_after errors |
| :large_orange_diamond: warning | src/middleware/manager.ts | 78-95 | executeAfter does not catch exceptions from after() |
| :large_orange_diamond: warning | src/registry/registry.ts | 672-698 | Registry.watch silently swallows fs.watch failures |
| :large_orange_diamond: warning | src/registry/registry.ts | 711-725 | _handleFileChange emits 'register' event with null module (consumer crash risk) |
| :large_orange_diamond: warning | src/sys-modules/control.ts | 143-167 | ReloadModule bypasses Registry.safeUnregister, breaking in-flight executions |
| :large_orange_diamond: warning | src/middleware/retry.ts | 46-101 | RetryMiddleware does not actually retry — delay hint has no consumer |
| :large_orange_diamond: warning | src/middleware/platform-notify.ts | 109-123 | Platform notify recovery event never fires for chronically failing modules |
| :blue_book: suggestion | src/schema/loader.ts | 266-269 | jsonSchemaToTypeBox 'const' branch loses array/object consts |
| :blue_book: suggestion | src/async-task.ts | 132-144 | AsyncTaskManager.cancel does not actually interrupt in-flight tasks |

**Critical detail — src/registry/dependencies.ts:42-53 (`resolveDependencies`):** When a dep declares a `version` and `versionLookup !== null` but the target is absent from the lookup (exactly the case `Registry._resolveLoadOrder` creates for pre-registered modules whose class lacks a `version` field), enforcement is silently skipped. Fix: raise `DependencyVersionUnknownError` for non-optional deps, or at minimum `console.warn('[apcore:registry] ...')`.

**Critical detail — src/executor.ts:333-335 (`Executor.call`):** Before-chain middleware exceptions are wrapped in `MiddlewareChainError` and re-thrown as `new ModuleError('MODULE_EXECUTE_ERROR', ...)`, losing the original class (`ACLDeniedError`, `InvalidInputError`). The streaming path (436-447) uses `propagateError` correctly — mirror that.

**Critical detail — src/executor.ts:481-491 (`Executor.stream`):** Post-stream output validation + after-middleware run inside a bare `try { ... } catch { /* non-fatal */ }` with no log. Schema-violating output reaches callers and telemetry never sees the failure. Fix: route through `propagateError` and emit to the event bus, matching `call()` behavior.

### Security (D2)

**Rating:** warning

| Severity | File | Line | Title |
|---|---|---|---|
| :warning: critical | src/schema/ref-resolver.ts | 159-164 | _assertWithinSchemasDir path-traversal check fails on Windows |
| :large_orange_diamond: warning | src/observability/metrics.ts | 152-162 | Prometheus label values are not escaped |
| :large_orange_diamond: warning | src/observability/tracing.ts | 121-131 | OTLPExporter has no timeout / abort signal |
| :large_orange_diamond: warning | src/observability/context-logger.ts | 63-72 | ContextLogger redaction does not honor schema x-sensitive |
| :large_orange_diamond: warning | src/middleware/logging.ts | 61-83 | LoggingMiddleware.after logs raw output (not redacted) |
| :blue_book: suggestion | src/sys-modules/control.ts | 13-20 | isSensitiveKey heuristic misses common patterns |

**Critical detail — src/schema/ref-resolver.ts:159-164:** `resolvedPath.startsWith(this._schemasDir + '/')` uses a hard-coded `'/'` separator; on Windows (supported per `engines.node >=18.0.0` with no OS pin), `path.resolve()` returns backslash paths so the prefix never matches, effectively disabling the path-traversal guard. Fix: use `path.relative()` + assert result doesn't start with `..`.

### Resource Management (D3)

**Rating:** warning

| Severity | File | Line | Title |
|---|---|---|---|
| :large_orange_diamond: warning | src/registry/registry.ts | 656-709 | fs.watch promises from _handleFileChange are unawaited (unhandled rejection risk) |
| :large_orange_diamond: warning | src/registry/registry.ts | 870-882 | endDrain wipes drainResolvers even when a waiter is pending |
| :blue_book: suggestion | src/events/subscribers.ts | 47-84 | WebhookSubscriber retries with no backoff |

---

## Tier 2 — Should-Fix

### Code Quality (D4)

**Rating:** warning

| Severity | File | Line | Title |
|---|---|---|---|
| :large_orange_diamond: warning | src/registry/registry.ts | 1-940 | registry.ts is 940 LOC — exceeds team simplicity standard |
| :large_orange_diamond: warning | src/config.ts | 1-958 | config.ts is 958 LOC with two unrelated modes in one class |
| :large_orange_diamond: warning | src/acl-handlers.ts | 48-50 | MaxCallDepthHandler uses `as any` casts |
| :blue_book: suggestion | src/errors.ts | 1-1123 | errors.ts is 1123 LOC with ~35 subclasses of near-identical shape |
| :blue_book: suggestion | src/builtin-steps.ts | 1-733 | builtin-steps.ts is 733 LOC — consider per-step files |

### Architecture & Design (D5)

**Rating:** pass

| Severity | File | Line | Title |
|---|---|---|---|
| :blue_book: suggestion | src/registry/schema-export.ts | 14 | Circular-import risk between registry/registry.ts and registry/schema-export.ts (type-only today; mark explicitly) |
| :blue_book: suggestion | src/executor.ts | 21-24 | Move `MODULE_ID_PATTERN` and friends into `registry/validation-constants.ts` to flatten dep graph |

### Performance (D6)

**Rating:** pass

| Severity | File | Line | Title |
|---|---|---|---|
| :blue_book: suggestion | src/observability/error-history.ts | 94-119 | `_evictTotal` is O(N × modules) per over-cap insertion |
| :blue_book: suggestion | src/middleware/manager.ts | 26-38 | `MiddlewareManager.add` is O(N) per insertion |

### Test Coverage (D7)

**Rating:** pass

Gaps flagged:
- `src/pipeline-config.ts` — verify dynamic-import edge cases (file: URLs, '..' segments, class-vs-factory fallback) are covered in `tests/test-pipeline-config.test.ts`.
- `src/middleware/retry.ts` — add a test that pins down `RetryMiddleware`'s actual semantics (it does not retry).
- `src/schema/extractor.ts` — no dedicated test file; adapter chain (priority sorting, partial-extraction) appears uncovered.
- `src/async-task.ts` — cancellation-continues-running behavior not asserted.
- `src/registry/version.ts` — no dedicated test; caret/tilde logic only indirectly covered via `test-dependencies.test.ts`. Add direct tests for `parseSemver`, `compareSemver`, `~1`, `^0.0.3` bounds.
- `src/sys-modules/manifest.ts` — verify prefix+tags filter interplay.
- `src/registry/schema-export.ts` — verify all profile-based export paths.

### Simplification & Anti-Bloat (D15)

**Rating:** warning

| Severity | File | Line | Title |
|---|---|---|---|
| :large_orange_diamond: warning | src/middleware/retry.ts | 10-101 | RetryMiddleware is dead-code-as-ergonomics — feature not implemented |
| :large_orange_diamond: warning | src/errors.ts | 539-544 | Deprecated alias `BindingSchemaMissingError` kept indefinitely |
| :large_orange_diamond: warning | src/context.ts + src/observability/context-logger.ts | N/A | Two parallel 'logger' concepts with partial overlap |
| :large_orange_diamond: warning | src/context.ts | 202-210 | `toJSON` / `fromJSON` vs `serialize` / `deserialize` are redundant aliases |
| :blue_book: suggestion | src/schema/loader.ts | 160-166 | `_loadAndResolve` duplicates `_modelCache` read its only caller already did |
| :blue_book: suggestion | src/executor.ts | 253-272 | `setAcl` / `setApprovalHandler` duplicate identical match-then-update pattern |
| :blue_book: suggestion | src/sys-modules/* | N/A | Every sys module class inlines the same annotations literal (~60 LOC duplication) |
| :blue_book: suggestion | src/registry/schema-export.ts | 46-108 | `exportSchema` / `exportAllSchemas` duplicate strict/compact paths |
| :blue_book: suggestion | src/async-task.ts | 66 | Non-ModuleError thrown from `submit` (use `TaskLimitExceededError`) |
| :blue_book: suggestion | src/registry/registry.ts + src/sys-modules/toggle.ts | N/A | `DEFAULT_TOGGLE_STATE` is a module-level singleton with hidden coupling |

---

## Tier 3 — Recommended

### Error Handling & Observability (D8/D9)

**Rating:** warning

| Severity | File | Line | Title |
|---|---|---|---|
| :large_orange_diamond: warning | src/registry/registry.ts | 719, 733 | Silent `catch { /* ignore */ }` on `onUnload` during file-watch reload |
| :large_orange_diamond: warning | src/pipeline-config.ts | 188-194 | `_importStep` swallows constructor error and falls back to function call |
| :large_orange_diamond: warning | src/context.ts | 217-230 | `child()` shares data by reference — mutations leak between parent/children (document or add `isolateData`) |
| :blue_book: suggestion | src/observability/tracing.ts | 46-51 | `StdoutExporter` uses `console.info` (CLAUDE.md mandates `console.warn` + `[apcore:*]` prefix) |
| :blue_book: suggestion | src/middleware/logging.ts | 13-20 | `defaultLogger` uses `console.info` / `console.error` without `[apcore:*]` prefix |

---

## Tier 4 — Nice-to-Have

### Maintainability & Compatibility (D10–D13)

**Rating:** pass

| Severity | File | Line | Title |
|---|---|---|---|
| :blue_book: suggestion | src/registry/registry.ts | 739-749 | `_pathToModuleId` uses `endsWith` heuristic — shadow-match risk on similar basenames |
| :blue_book: suggestion | src/registry/dependencies.ts | 5-11 | `ModuleLoadError` import dropped — surface BREAKING in 0.20.0 release notes |
| :blue_book: suggestion | src/index.ts | 82-83 | Two new error exports (`DependencyNotFoundError`, `DependencyVersionMismatchError`) — document in release notes |
| :blue_book: suggestion | package.json | 38-42 | Dependencies minimal and appropriate (@sinclair/typebox ^0.34, js-yaml ^4.1, uuid ^11) — no flags |

### Accessibility / i18n (D14)

**Rating:** skipped (library — no UI surface)

---

*No reference documents found — consistency check skipped.*

---

## Recommendations

**Must fix before merge:**
1. `src/registry/dependencies.ts:42-53` — raise `DependencyVersionUnknownError` (or warn) when `versionLookup.get(dep.moduleId) === undefined` for a non-optional versioned dep.
2. `src/executor.ts:333-335` — use `propagateError((exc as MiddlewareChainError).original, moduleId, ctxObj)` instead of the generic `ModuleError('MODULE_EXECUTE_ERROR', ...)` wrap.
3. `src/executor.ts:481-491` — stop swallowing post-stream validation + after-middleware errors; log, record via `executedMiddlewares.onError`, emit to the event bus.
4. `src/schema/ref-resolver.ts:159-164` — replace `startsWith(schemasDir + '/')` with `path.relative()`-based containment check; this is cross-platform-reachable.

**Should fix:**
1. `src/middleware/manager.ts:78-95` — wrap `after()` calls in try/catch to match `executeBefore` / `executeOnError` behavior.
2. `src/registry/registry.ts:672-698` — log `fs.watch` failures instead of swallowing (EMFILE, EACCES, kernel-unsupported recursive).
3. `src/registry/registry.ts:711-725` — do not emit `'register'` with `null` module on file change; introduce `'file_changed'` or drop the event.
4. `src/sys-modules/control.ts:143-167` — `ReloadModule.execute` should use `registry.safeUnregister` to avoid interrupting in-flight executions.
5. `src/middleware/retry.ts` — decide: rename to `RetryHintMiddleware` + drop unused config, or implement real retries. Current state is misleading ergonomics.
6. `src/middleware/platform-notify.ts:109-123` — also check recovery in `onError()` so chronically-failing modules can clear the alert state.
7. `src/observability/metrics.ts:152-162` — escape Prometheus label values (`"`, `\`, `\n`).
8. `src/observability/tracing.ts:121-131` — add `AbortController` + timeout to `OTLPExporter.export`, mirroring `WebhookSubscriber`.
9. `src/observability/context-logger.ts:63-72` — honor schema `x-sensitive` fields or always use `context.redactedInputs` / `context.redactedOutput`.
10. `src/middleware/logging.ts:61-83` — use `context.redactedOutput` in the after-log (0.19.0 already redacts it on the Context).
11. `src/registry/registry.ts:656-709` — attach `.catch()` to the unawaited async `_handleFileChange` / `_handleFileDeletion` calls.
12. `src/registry/registry.ts` — split into registry-core / registry-discover / registry-watch / registry-export (940 LOC exceeds team simplicity rule).
13. `src/config.ts` — extract env-resolution helpers to `src/config-env.ts` (958 LOC).
14. `src/acl-handlers.ts:48-50` — replace `as any` with narrowed `unknown` type guards per CLAUDE.md.
15. `src/middleware/retry.ts` / `src/context.ts` — resolve D15 redundancies: dead retry-delay logic, `toJSON`/`serialize` alias split, parallel logger concepts.

**Consider for later:**
1. `src/errors.ts` — factory helper `makeModuleError(code, messageFn, detailsFn)` to amortize ~35 subclasses of identical shape.
2. `src/builtin-steps.ts` — per-step files under `src/steps/`.
3. `src/observability/error-history.ts:94-119` — track global oldest entry to avoid O(N × modules) eviction scan.
4. `src/registry/schema-export.ts:46-108` — extract shared `applyStrictOrCompact(schema, strict, compact)` helper.
5. `src/sys-modules/*` — `SYS_READONLY_ANNOTATIONS` / `SYS_WRITE_ANNOTATIONS` constants to dedupe annotation literals.
6. `src/executor.ts:253-272` — extract `_updateStepByName<T>(name, klass, update)` helper.
7. Test coverage: add a dedicated `tests/registry/test-version.test.ts` covering caret/tilde + parseSemver edge cases.
8. `src/events/subscribers.ts` — exponential backoff for `WebhookSubscriber` retries.
9. `src/context.ts:217-230` — add `child({ isolateData: true })` option and document the shared-by-reference invariant.
10. Config & toggle singletons — scope `DEFAULT_TOGGLE_STATE` per-Registry to avoid cross-instance bleed in tests.

## Verdict

**Fix the three `critical` items (plus the Windows path-traversal security critical) before cutting the next release**, then merge. The `warning` tier can be addressed incrementally — several items (RetryMiddleware semantics, duplicative logger concepts, large-file splits, logger-prefix convention) are worth a dedicated cleanup pass but do not block correctness. No rework required on the architecture or dependency layers. Dimensions D5 (Architecture), D6 (Performance), D7 (Test Coverage), and D10–D13 (Maintainability/Compat/Dependencies) all pass.
