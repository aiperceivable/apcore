# ACL Conditions Redesign — Implementation Plan

## Task Order (dependency-aware)

| # | Task | Depends On | Languages |
|---|------|-----------|-----------|
| 1 | handler-protocol | — | Py, TS, Rust |
| 2 | handler-registry | 1 | Py, TS, Rust |
| 3 | builtin-handlers | 1 | Py, TS, Rust |
| 4 | compound-handlers | 1, 3 | Py, TS, Rust |
| 5 | handler-dispatch | 2, 3, 4 | Py, TS, Rust |
| 6 | async-check | 5 | Py, TS, Rust |
| 7 | cross-language-fixes | — | TS, Rust |

## TDD Approach
Each task:
1. Write failing tests
2. Implement minimum code to pass
3. Run full suite, verify no regressions

## File Changes

### Python
- **NEW** `apcore-python/src/apcore/acl_handlers.py` — Handler protocols, built-in handlers, compound handlers
- **MOD** `apcore-python/src/apcore/acl.py` — Add registry, dispatch, async_check
- **NEW** `apcore-python/tests/test_acl_conditions.py` — All condition handler tests

### TypeScript
- **NEW** `apcore-typescript/src/acl-handlers.ts` — Handler interface, built-in + compound handlers
- **MOD** `apcore-typescript/src/acl.ts` — Add registry, dispatch, asyncCheck, removeRule fix
- **NEW** `apcore-typescript/tests/test-acl-conditions.test.ts` — All condition handler tests

### Rust
- **NEW** `apcore-rust/src/acl_handlers.rs` — Handler trait, built-in + compound handler structs
- **MOD** `apcore-rust/src/acl.rs` — Add registry, dispatch, async_check, empty callers fix, audit_logger constructor
- **MOD** `apcore-rust/src/lib.rs` — Add `pub mod acl_handlers`
- **NEW** `apcore-rust/tests/test_acl_conditions.rs` — All condition handler tests
