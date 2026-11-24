# Feature Manifest: apcore-python Protocol Compliance (v0.7.1 → v0.8.0)

**Generated:** 2026-03-04
**Project:** apcore-python SDK
**Goal:** Achieve Level 1 full conformance (100%) and Level 2 substantial conformance (≥90%)
**Current State:** Level 1 ~85%, Level 2 ~70%

---

## Dependency Graph

```
F01 Config (A12) ─────────┬──→ F02 Timeout (A22)
                          ├──→ F07 Guard Call Chain (A20)
                          ├──→ F08 Version Negotiation (A14)
                          └──→ F09 Safe Hot-Reload (A21)

F04 ID Normalization (A02) ──→ (standalone)

F05 Error Code Collision (A17) ──→ (standalone, enhances errors.py)

F06 ACL Specificity (A10) ──→ F10 ACL Audit Logging

F11 Retry Middleware ──→ (standalone, uses error.retryable)

F12 Streaming Deep Merge ──→ (standalone bug fix)

F03 Conformance Tests ──→ depends on F01, F02, F04, F05, F06, F07 (runs last)
```

---

## Phase 1: Foundation (No Dependencies)

### F01: Config System (A12) — MUST, P0
- **Scope:** Replace `config.py` stub with full YAML loading, env var overrides, validation
- **Files:** `src/apcore/config.py` (rewrite), `tests/test_config.py` (new)
- **Spec:** Algorithm A12 `validate_config()`
- **Estimated Size:** ~250 LOC source + ~400 LOC tests
- **Dependencies:** None (foundation for F02, F07, F08, F09)
- **Blocked by:** Nothing
- **Blocks:** F02, F07, F08, F09
- **Key Requirements:**
  - YAML file loading with `Config.load(path)`
  - Environment variable overrides: `APCORE_{SECTION}_{KEY}` prefix
  - Merge priority: env vars > config file > defaults
  - Schema validation: required fields, type checking, constraint checking
  - Dot-path access preserved (backward compatible)
  - Hot-reload via `Config.reload()`
  - Required fields: `version`, `extensions.root`, `schema.root`, `acl.root`, `acl.default_effect`, `project.name`
  - Constraint validation: sampling_rate ∈ [0.0, 1.0], max_depth ∈ [1, 16], etc.

### F04: Cross-Language ID Normalization (A02) — MUST, P1
- **Scope:** New utility function for cross-language module ID conversion
- **Files:** `src/apcore/utils/normalize.py` (new), `tests/test_normalize.py` (new)
- **Spec:** Algorithm A02 `normalize_to_canonical_id()`
- **Estimated Size:** ~100 LOC source + ~200 LOC tests
- **Dependencies:** None
- **Blocked by:** Nothing
- **Blocks:** F03
- **Key Requirements:**
  - Input: `(local_id: str, language: str)` where language ∈ {python, rust, go, java, typescript}
  - Language-specific separators: Python ".", Rust "::", Go ".", Java ".", TypeScript "."
  - Case normalization: PascalCase/camelCase → snake_case
  - Acronym handling: `HttpJsonParser` → `http_json_parser` (not `h_t_t_p_...`)
  - Output validated against Canonical ID EBNF grammar
  - Export from `apcore.__init__`

### F05: Error Code Collision Detection (A17) — MUST, P1
- **Scope:** Error code registry with collision detection
- **Files:** `src/apcore/errors.py` (extend), `tests/test_error_codes.py` (new)
- **Spec:** Algorithm A17 `detect_error_code_collisions()`
- **Estimated Size:** ~80 LOC source + ~150 LOC tests
- **Dependencies:** None
- **Blocked by:** Nothing
- **Blocks:** F03
- **Key Requirements:**
  - `ErrorCodeRegistry` class with `register(module_id, codes)` method
  - Framework reserved codes: prefixes `MODULE_`, `SCHEMA_`, `ACL_`, `GENERAL_`, `CONFIG_`, `CIRCULAR_`, `DEPENDENCY_`
  - Detect: module code collides with framework code → error
  - Detect: module code collides with another module's code → error
  - Return complete code registry set
  - Run at framework startup (integrate with Registry.discover())
  - Thread-safe

### F06: ACL Specificity Scoring (A10) — SHOULD, P1
- **Scope:** Pattern specificity calculation for ACL debugging
- **Files:** `src/apcore/utils/pattern.py` (extend), `tests/test_specificity.py` (new)
- **Spec:** Algorithm A10 `calculate_specificity()`
- **Estimated Size:** ~40 LOC source + ~100 LOC tests
- **Dependencies:** None
- **Blocked by:** Nothing
- **Blocks:** F10
- **Key Requirements:**
  - `calculate_specificity(pattern: str) -> int`
  - Scoring: `"*"` → 0, exact segment → +2, partial wildcard segment → +1
  - Examples: `"*"` → 0, `"api.*"` → 2, `"api.handler.*"` → 4, `"api.handler.task_submit"` → 6
  - Export from `apcore.__init__`

### F12: Streaming Deep Merge (Bug Fix) — P0
- **Scope:** Fix shallow merge in executor streaming accumulation
- **Files:** `src/apcore/executor.py` (fix ~5 lines), `tests/test_executor_stream.py` (extend)
- **Estimated Size:** ~30 LOC source + ~50 LOC tests
- **Dependencies:** None
- **Blocked by:** Nothing
- **Blocks:** Nothing
- **Key Requirements:**
  - Replace `{**accumulated, **chunk}` with recursive deep merge
  - Nested dicts merged recursively, lists replaced (not concatenated)
  - Existing flat streaming behavior unchanged
  - New test: overlapping nested keys across chunks

---

## Phase 2: Executor Enhancements (Depend on F01)

### F02: Timeout Enforcement (A22) — MUST, P0
- **Scope:** Cooperative cancellation with grace period for sync modules
- **Files:** `src/apcore/executor.py` (refactor timeout section), `tests/test_executor.py` (extend)
- **Spec:** Algorithm A22 `enforce_timeout()`
- **Estimated Size:** ~100 LOC source + ~150 LOC tests
- **Dependencies:** F01 (reads timeout config)
- **Blocked by:** F01
- **Blocks:** F03
- **Key Requirements:**
  - If timeout_ms == 0: skip timeout enforcement
  - Cooperative cancellation: set CancelToken before force-killing
  - Grace period: 5 seconds after cancel signal before giving up
  - Async path: `asyncio.wait_for()` + cancel token (already partially works)
  - Sync path: thread + cancel token signal + join(grace_period)
  - Log warning when thread cannot be killed (Python limitation)
  - Timing starts from first middleware before()

### F07: Guard Call Chain (A20) — MUST, P1
- **Scope:** Extract call chain safety into standalone algorithm
- **Files:** `src/apcore/utils/call_chain.py` (new), `src/apcore/executor.py` (refactor), `tests/test_call_chain.py` (new)
- **Spec:** Algorithm A20 `guard_call_chain()`
- **Estimated Size:** ~80 LOC source + ~120 LOC tests
- **Dependencies:** F01 (reads max_depth, max_repeat config)
- **Blocked by:** F01
- **Blocks:** F03
- **Key Requirements:**
  - `guard_call_chain(module_id, call_chain, config) -> None` (raises on violation)
  - Three checks: depth limit, circular detection, frequency throttling
  - Extract from `executor._check_safety()`, keep executor calling the utility
  - Configurable via Config: `executor.max_call_depth`, `executor.max_module_repeat`
  - Same error types: `CallDepthExceededError`, `CircularCallError`, `CallFrequencyExceededError`

### F08: Version Negotiation (A14) — MUST, P2
- **Scope:** Semver compatibility checking between declared and SDK versions
- **Files:** `src/apcore/version.py` (new), `tests/test_version.py` (new)
- **Spec:** Algorithm A14 `negotiate_version()`
- **Estimated Size:** ~80 LOC source + ~150 LOC tests
- **Dependencies:** F01 (reads declared version from config)
- **Blocked by:** F01
- **Blocks:** F03
- **Key Requirements:**
  - `negotiate_version(declared_version: str, sdk_version: str) -> str`
  - Parse semver (major.minor.patch, optional pre-release)
  - Major mismatch → `VERSION_INCOMPATIBLE` error
  - Declared minor > SDK minor → error (SDK too old)
  - Declared minor < SDK minor by >2 → deprecation warning
  - Same minor → effective = max(declared, sdk)
  - New error class: `VersionIncompatibleError`
  - Integrate into Config.load() or Executor initialization

---

## Phase 3: Advanced Features

### F09: Safe Hot-Reload (A21) — SHOULD, P2
- **Scope:** Reference-counted safe module unregistration
- **Files:** `src/apcore/registry/registry.py` (extend), `src/apcore/executor.py` (add ref counting), `tests/registry/test_hot_reload.py` (new)
- **Spec:** Algorithm A21 `safe_unregister()`
- **Estimated Size:** ~120 LOC source + ~200 LOC tests
- **Dependencies:** F01 (config for timeout), F07 (executor awareness)
- **Blocked by:** F01
- **Blocks:** F03
- **Key Requirements:**
  - `Registry.safe_unregister(module_id) -> bool`
  - Reference counting: executor increments on call start, decrements on call end
  - Mark module state as "UNLOADING" (new calls get MODULE_NOT_FOUND)
  - Wait for ref_count == 0 with configurable timeout (default 30s)
  - Call `on_unload()` hook after all executions finish
  - Idempotent: unregistering non-existent module returns True
  - Force-unload on timeout with logging
  - Thread-safe with atomic state transitions

### F10: ACL Audit Logging — P2
- **Scope:** Structured audit trail for ACL decisions
- **Files:** `src/apcore/acl.py` (extend), `tests/test_acl_audit.py` (new)
- **Estimated Size:** ~80 LOC source + ~100 LOC tests
- **Dependencies:** F06 (includes specificity in audit entries)
- **Blocked by:** F06
- **Blocks:** Nothing
- **Key Requirements:**
  - `AuditEntry` dataclass: timestamp, caller_id, target_id, decision, matched_rule, specificity, context_summary
  - `ACL.set_audit_handler(handler)` — pluggable handler protocol
  - Default: no-op (zero overhead when not configured)
  - Built-in: `InMemoryAuditHandler` for testing, `LoggingAuditHandler` for production
  - Every `check()` call produces an audit entry
  - Thread-safe

### F11: Retry Middleware — P2
- **Scope:** Configurable retry strategy middleware
- **Files:** `src/apcore/middleware/retry.py` (new), `tests/test_retry_middleware.py` (new)
- **Estimated Size:** ~120 LOC source + ~200 LOC tests
- **Dependencies:** None (uses existing middleware + error.retryable)
- **Blocked by:** Nothing
- **Blocks:** Nothing
- **Key Requirements:**
  - `RetryMiddleware(max_retries=3, strategy="exponential", base_delay_ms=100, max_delay_ms=10000)`
  - Strategies: "exponential" (base * 2^attempt), "fixed" (constant delay), "linear" (base * attempt)
  - Only retries if `error.retryable is True`
  - Jitter: optional random jitter to prevent thundering herd
  - Respects module timeout (total retry time < module timeout)
  - Implemented via `on_error()` hook — returns recovery by re-executing
  - Configurable per-module overrides via annotations

---

## Phase 4: Validation

### F03: Cross-Language Conformance Test Suite — P0
- **Scope:** JSON fixture-based tests validating protocol compliance
- **Files:** `tests/conformance/` (new directory), `tests/conformance/fixtures/` (JSON fixtures)
- **Estimated Size:** ~500 LOC tests + ~300 LOC fixtures
- **Dependencies:** F01, F02, F04, F05, F06, F07
- **Blocked by:** All Phase 1-2 features
- **Blocks:** Nothing
- **Key Requirements:**
  - JSON fixtures define input → expected output for each algorithm
  - Fixtures shareable with TypeScript SDK (same JSON, different test runner)
  - Coverage sections:
    - §2: ID normalization (A01, A02)
    - §6: ACL pattern matching (A08), evaluation (A09), specificity (A10)
    - §8: Error codes and propagation (A11, A17)
    - §9: Config validation (A12)
    - §10: Redaction (A13)
    - §12: Executor pipeline (10 steps), timeout (A22), call chain (A20)
    - §13: Version negotiation (A14)
  - Each fixture: `{"algorithm": "A02", "input": {...}, "expected": {...}, "description": "..."}`
  - pytest parametrize over fixture files

---

## Implementation Order (Recommended)

```
Week 1: Foundation
  F12 Streaming Deep Merge (bug fix, 1h)
  F06 ACL Specificity (small, 2h)
  F05 Error Code Collision (small, 3h)
  F04 ID Normalization (medium, 4h)
  F01 Config System (large, 8h)

Week 2: Executor & Protocol
  F02 Timeout Enforcement (medium, 6h)
  F07 Guard Call Chain (medium, 4h)
  F08 Version Negotiation (medium, 4h)
  F11 Retry Middleware (medium, 6h)

Week 3: Advanced + Validation
  F10 ACL Audit Logging (medium, 4h)
  F09 Safe Hot-Reload (large, 8h)
  F03 Conformance Tests (large, 8h)
```

---

## Version Bump Plan

After all features complete:
- Version: 0.7.1 → **0.8.0** (minor bump — new features, no breaking changes)
- `Config` API is additive (new class methods, existing `get()` preserved)
- `ACL` API is additive (new `set_audit_handler()`, specificity export)
- `Executor` behavior change: timeout now cooperative (may affect edge cases)
- `errors.py` additive (new `ErrorCodeRegistry`, `VersionIncompatibleError`)
- New public exports: `normalize_to_canonical_id`, `calculate_specificity`, `guard_call_chain`, `negotiate_version`, `ErrorCodeRegistry`, `RetryMiddleware`, `AuditEntry`

---

## Risk Assessment

| Risk | Impact | Mitigation |
|------|--------|------------|
| Config rewrite breaks existing users | High | Preserve `Config(data=dict)` constructor + `get()` method |
| Timeout change affects sync modules | Medium | Grace period + cooperative cancel before force |
| Ref counting in executor adds overhead | Low | Atomic counter, negligible cost per call |
| New dependencies needed | Low | All features use existing deps (pydantic, pyyaml) |
| Conformance fixtures diverge from TS | Medium | Generate fixtures from spec, not from implementation |
