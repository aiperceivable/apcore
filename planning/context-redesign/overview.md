# Context Redesign — Feature Overview

## Overview

Align the Context struct/class across Python, TypeScript, and Rust SDKs to a single canonical field definition. Introduce `ContextKey<T>` for type-safe access to `context.data`, fix data key naming convention violations, and add `_context_version` serialization support.

## Scope

**Included:**
- `ContextKey<T>` class/struct in all 3 SDKs with `get()`, `set()`, `delete()`, `exists()`, `scoped()`
- Built-in context key constants (`_apcore.mw.*`, `_apcore.executor.*`)
- Data key naming migration (`_metrics_starts` → `_apcore.mw.metrics.starts`, etc.)
- Context serialization with `_context_version: 1`
- Rust: remove `created_at`, `parent_trace_id`, `trace_context` fields; change `global_deadline` from `Instant` to `f64`
- Rust: make `Identity` fields immutable (private fields + pub getters)
- TypeScript: add `globalDeadline: number | null` field

**Excluded:**
- Changes to `Context.create()` or `Context.child()` logic
- Distributed context synchronization
- Thread-safety for `context.data` in Python/TS
- ContextKey runtime enforcement of `_apcore.*` prefix restriction

## Technology Stack

- **Languages:** Python 3.11+, TypeScript (Node 18+), Rust 1.75+
- **Testing:** pytest, vitest, cargo test
- **Key deps:** serde/serde_json (Rust), std::borrow::Cow (Rust)

## Task Execution Order

| # | Task File | Description | Status |
|---|-----------|-------------|--------|
| 1 | [context-key](./tasks/context-key.md) | ContextKey\<T\> typed accessor in all 3 SDKs | pending |
| 2 | [rust-field-alignment](./tasks/rust-field-alignment.md) | Remove non-spec fields from Rust Context | pending |
| 3 | [rust-identity-immutable](./tasks/rust-identity-immutable.md) | Make Rust Identity fields immutable | pending |
| 4 | [ts-global-deadline](./tasks/ts-global-deadline.md) | Add globalDeadline field to TypeScript Context | pending |
| 5 | [builtin-keys](./tasks/builtin-keys.md) | Define built-in context key constants | pending |
| 6 | [data-key-migration](./tasks/data-key-migration.md) | Migrate middleware data keys to _apcore.* convention | pending |
| 7 | [serialization](./tasks/serialization.md) | Context serialization with _context_version | pending |

## Progress

- **Total:** 7 tasks
- **Completed:** 0
- **In Progress:** 0
- **Pending:** 7

## Reference Documents

- Source: [docs/features/context-redesign.md](../../docs/features/context-redesign.md)
- Design spec: [docs/spec/design-context-annotations-acl.md](../../docs/spec/design-context-annotations-acl.md) §1
