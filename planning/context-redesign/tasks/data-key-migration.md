# Task: Data Key Naming Migration

## Goal

Rename all legacy internal data keys to the `_apcore.*` naming convention and update middleware code to use `ContextKey` constants instead of raw string literals. This ensures consistent naming across all 3 SDKs.

| Old Key | New Key |
|---------|---------|
| `_metrics_starts` | `_apcore.mw.metrics.starts` |
| `_usage_starts` | `_apcore.mw.usage.starts` |
| `_obs_logging_starts` | `_apcore.mw.logging.starts` |

## Files Involved

### Python SDK
- **Modify:** `/Users/tercel/WorkSpace/aipartnerup/apcore-python/src/apcore/observability/metrics.py`
- **Modify:** `/Users/tercel/WorkSpace/aipartnerup/apcore-python/src/apcore/observability/usage.py`
- **Modify:** `/Users/tercel/WorkSpace/aipartnerup/apcore-python/src/apcore/observability/context_logger.py`
- **Modify:** `/Users/tercel/WorkSpace/aipartnerup/apcore-python/src/apcore/middleware/logging.py`
- **Modify:** `/Users/tercel/WorkSpace/aipartnerup/apcore-python/src/apcore/middleware/retry.py`

### TypeScript SDK
- **Modify:** `/Users/tercel/WorkSpace/aipartnerup/apcore-typescript/src/observability/metrics.ts`
- **Modify:** `/Users/tercel/WorkSpace/aipartnerup/apcore-typescript/src/observability/usage.ts`
- **Modify:** `/Users/tercel/WorkSpace/aipartnerup/apcore-typescript/src/middleware/logging.ts`

### Rust SDK
- **Modify:** `/Users/tercel/WorkSpace/aipartnerup/apcore-rust/src/observability/metrics.rs`
- **Modify:** `/Users/tercel/WorkSpace/aipartnerup/apcore-rust/src/observability/usage.rs`
- **Modify:** `/Users/tercel/WorkSpace/aipartnerup/apcore-rust/src/observability/logging.rs`
- **Modify:** `/Users/tercel/WorkSpace/aipartnerup/apcore-rust/src/middleware/logging.rs`
- **Modify:** `/Users/tercel/WorkSpace/aipartnerup/apcore-rust/src/middleware/retry.rs`

## Steps

### Step 1: Grep for all occurrences of legacy key names

```bash
# Python
cd apcore-python && grep -rn "_metrics_starts\|_usage_starts\|_obs_logging_starts" src/

# TypeScript
cd apcore-typescript && grep -rn "_metrics_starts\|_usage_starts\|_obs_logging_starts" src/

# Rust
cd apcore-rust && grep -rn "_metrics_starts\|_usage_starts\|_obs_logging_starts" src/
```

Document every file and line that uses the legacy names.

### Step 2: Write verification test (Python)

Create or add to `apcore-python/tests/test_data_key_migration.py`:

```python
import subprocess


def test_no_legacy_key_names_in_source():
    """AC-021: No occurrences of legacy key names in source code."""
    legacy_keys = ["_metrics_starts", "_usage_starts", "_obs_logging_starts"]
    result = subprocess.run(
        ["grep", "-rn"] + legacy_keys + ["src/apcore/"],
        capture_output=True, text=True, cwd="/path/to/apcore-python"
    )
    assert result.stdout == "", (
        f"Legacy key names still found in source:\n{result.stdout}"
    )
```

### Step 3: Update Python middleware to use ContextKey constants

For each middleware file, replace raw string access with `ContextKey` usage:

**Before (metrics.py):**
```python
ctx.data["_metrics_starts"] = starts
# ...
starts = ctx.data.get("_metrics_starts", [])
```

**After (metrics.py):**
```python
from apcore.context_keys import METRICS_STARTS

METRICS_STARTS.set(ctx, starts)
# ...
starts = METRICS_STARTS.get(ctx, default=[])
```

**Before (usage.py):**
```python
ctx.data["_usage_starts"] = starts
```

**After (usage.py):**
```python
from apcore.context_keys import USAGE_STARTS  # Add to context_keys.py if not already defined

USAGE_STARTS.set(ctx, starts)
```

> Note: If `USAGE_STARTS` is not yet in `context_keys.py`, add it:
> ```python
> USAGE_STARTS: ContextKey[list] = ContextKey("_apcore.mw.usage.starts")
> ```

**Before (logging middleware / context_logger.py):**
```python
ctx.data["_obs_logging_starts"] = start_time
```

**After:**
```python
from apcore.context_keys import LOGGING_START

LOGGING_START.set(ctx, start_time)
```

### Step 4: Update TypeScript middleware

Apply the same pattern as Python. Replace raw `ctx.data["_metrics_starts"]` with `METRICS_STARTS.set(ctx, ...)` / `METRICS_STARTS.get(ctx)`.

**Before (metrics.ts):**
```typescript
ctx.data["_metrics_starts"] = starts;
```

**After (metrics.ts):**
```typescript
import { METRICS_STARTS } from "../context-keys";

METRICS_STARTS.set(ctx, starts);
```

### Step 5: Update Rust middleware

**Before (metrics.rs):**
```rust
let mut data = ctx.data.write().unwrap();
data.insert("_metrics_starts".to_string(), serde_json::to_value(&starts).unwrap());
```

**After (metrics.rs):**
```rust
use crate::context_keys::METRICS_STARTS;

METRICS_STARTS.set(&ctx, starts);
```

### Step 6: Run grep verification across all SDKs

```bash
# Verify no legacy names remain in any SDK
grep -rn "_metrics_starts\|_usage_starts\|_obs_logging_starts" \
    apcore-python/src/ apcore-typescript/src/ apcore-rust/src/
```

Expected output: empty (no matches).

### Step 7: Run full test suites

```bash
cd apcore-python && python -m pytest
cd apcore-typescript && npx vitest run
cd apcore-rust && cargo test
```

## Acceptance Criteria

- [x] **AC-021**: Data key naming migration complete -- no occurrences of `_metrics_starts`, `_usage_starts`, `_obs_logging_starts` in source (grep verification)
- [ ] All middleware uses `ContextKey` constants instead of raw string literals
- [ ] New key names follow `_apcore.mw.{middleware}.{purpose}` convention
- [ ] All existing tests pass after migration

## Dependencies

- **Depends on:** context-key, builtin-keys
- **Required by:** none (but should be done before serialization for clean state)

## Estimated Time

2 hours
