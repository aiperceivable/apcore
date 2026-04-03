# Task: Rust Context Field Removal

## Goal

Remove three non-spec fields (`created_at`, `parent_trace_id`, `trace_context`) from the Rust `Context<T>` struct. Change `global_deadline` from `Option<Instant>` to `Option<f64>` (epoch seconds). Remove unused imports (`chrono`, `TraceContext`). This aligns the Rust Context with the canonical cross-language definition.

## Files Involved

### Rust SDK
- **Modify:** `/Users/tercel/WorkSpace/aipartnerup/apcore-rust/src/context.rs` (remove fields, change type)
- **Modify:** `/Users/tercel/WorkSpace/aipartnerup/apcore-rust/src/lib.rs` (remove re-exports if any)
- **Modify:** Any files that reference `created_at`, `parent_trace_id`, `trace_context`, or `Instant` on context
- **Create:** `/Users/tercel/WorkSpace/aipartnerup/apcore-rust/tests/context_field_removal_test.rs` (compile-fail test)

### Files to check for usages
- `/Users/tercel/WorkSpace/aipartnerup/apcore-rust/src/executor.rs`
- `/Users/tercel/WorkSpace/aipartnerup/apcore-rust/src/middleware/*.rs`
- `/Users/tercel/WorkSpace/aipartnerup/apcore-rust/src/observability/*.rs`
- `/Users/tercel/WorkSpace/aipartnerup/apcore-rust/tests/*.rs`

## Steps

### Step 1: Grep for all usages of removed fields

```bash
cd apcore-rust && grep -rn "created_at\|parent_trace_id\|trace_context" src/ tests/
```

Document every usage that needs updating.

### Step 2: Write compile-fail test (AC-019)

Using `trybuild` crate, create a compile-fail test. Add `trybuild` to `[dev-dependencies]` in `Cargo.toml` if not present.

Create `apcore-rust/tests/compile_fail/context_removed_fields.rs`:

```rust
// This file MUST NOT compile -- verifies AC-019
use apcore::context::Context;

fn main() {
    let ctx: Context<()> = Context::create_test();
    let _ = ctx.created_at;       // ERROR: no field `created_at`
    let _ = ctx.parent_trace_id;  // ERROR: no field `parent_trace_id`
    let _ = ctx.trace_context;    // ERROR: no field `trace_context`
}
```

Create the trybuild test runner in `apcore-rust/tests/context_field_removal_test.rs`:

```rust
#[test]
fn context_removed_fields_must_not_compile() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/context_removed_fields.rs");
}
```

### Step 3: Remove fields from Context struct

In `apcore-rust/src/context.rs`, remove:

```rust
// REMOVE these fields from the Context<T> struct:
// pub created_at: DateTime<Utc>,
// pub parent_trace_id: Option<String>,
// pub trace_context: Option<TraceContext>,
```

Change `global_deadline` type:

```rust
// BEFORE:
// pub global_deadline: Option<Instant>,

// AFTER:
pub global_deadline: Option<f64>,  // epoch seconds
```

### Step 4: Remove unused imports

```rust
// Remove if no longer used:
// use chrono::{DateTime, Utc};
// use std::time::Instant;  (if only used for global_deadline)
// use crate::trace_context::TraceContext;
```

### Step 5: Update Context constructors

Update `Context::create()`, `Context::child()`, and any builder/factory methods to remove initialization of the deleted fields and use `f64` for `global_deadline`.

### Step 6: Fix all compilation errors

Grep and fix all references to the removed fields in:
- `executor.rs` -- may reference `created_at` for timing
- `middleware/*.rs` -- may reference `trace_context`
- `observability/*.rs` -- may reference `parent_trace_id`
- `tests/*.rs` -- may construct Context with removed fields

For each removed field, apply the migration path:
| Field | Migration |
|-------|-----------|
| `created_at` | Use `ContextKey` with `"_apcore.created_at"` if needed |
| `parent_trace_id` | Derive from `call_chain[0]` or use `data` |
| `trace_context` | Move to `data["_apcore.trace_context"]` via ContextKey |

### Step 7: Run full test suite

```bash
cd apcore-rust && cargo test
```

Verify:
- All existing tests pass
- Compile-fail test passes (removed fields are indeed inaccessible)

## Acceptance Criteria

- [x] **AC-019**: Rust Context no longer has `created_at`, `parent_trace_id`, `trace_context` fields (compile-fail test)
- [ ] `global_deadline` is `Option<f64>` (epoch seconds), not `Option<Instant>`
- [ ] No unused imports of `chrono`, `Instant`, or `TraceContext` remain
- [ ] All existing tests pass after migration

## Dependencies

- **Depends on:** none
- **Required by:** serialization

## Estimated Time

2 hours
