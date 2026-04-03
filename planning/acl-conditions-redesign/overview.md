# ACL Conditions Redesign — Overview

## Goal
Replace hardcoded if/else condition evaluation in all 3 SDKs (Python, TypeScript, Rust) with a pluggable handler registry system. Add compound operators ($or, $not), async check paths, and fail-closed behavior for unknown conditions.

## Scope
- **Handler protocol** — `ACLConditionHandler` in all 3 languages
- **Handler registry** — Class-level `register_condition()` API
- **Built-in handlers** — identity_types, roles, max_call_depth, $or, $not
- **Dispatch replacement** — Replace if/else chain with handler lookup
- **Async check** — New `async_check()` method
- **Cross-language fixes** — TS removeRule, Rust empty callers, Rust audit_logger constructor

## Repos
- Python: `apcore-python/src/apcore/acl.py` + new `acl_handlers.py`
- TypeScript: `apcore-typescript/src/acl.ts` + new `acl-handlers.ts`
- Rust: `apcore-rust/src/acl.rs` + new `acl_handlers.rs`

## Key Design Decisions
1. Two separate Protocol classes in Python (sync + async) because Protocol cannot unify return types
2. TypeScript uses single interface with `boolean | Promise<boolean>` return
3. Rust uses `#[async_trait]` making all handlers async
4. Compound handlers receive `evaluate_fn` at construction to break circular dependency
5. Global (class-level) registry, not per-instance
6. Unknown conditions fail-closed (WARN + return False)
