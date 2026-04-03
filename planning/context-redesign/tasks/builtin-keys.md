# Task: Built-in Context Key Constants

## Goal

Define framework-internal key constants (`TRACING_SPANS`, `METRICS_STARTS`, `LOGGING_START`, `REDACTED_OUTPUT`, `RETRY_COUNT_BASE`, etc.) in dedicated modules per SDK, using the `ContextKey<T>` type. These constants replace raw string literals throughout middleware code.

## Files Involved

### Python SDK
- **Create:** `/Users/tercel/WorkSpace/aipartnerup/apcore-python/src/apcore/context_keys.py`

### TypeScript SDK
- **Create:** `/Users/tercel/WorkSpace/aipartnerup/apcore-typescript/src/context-keys.ts`
- **Modify:** `/Users/tercel/WorkSpace/aipartnerup/apcore-typescript/src/index.ts` (export constants)

### Rust SDK
- **Create:** `/Users/tercel/WorkSpace/aipartnerup/apcore-rust/src/context_keys.rs`
- **Modify:** `/Users/tercel/WorkSpace/aipartnerup/apcore-rust/src/lib.rs` (add `pub mod context_keys;`)

## Steps

### Step 1: Write tests verifying key names and types (Python)

Add to `apcore-python/tests/test_context_key.py` or create `apcore-python/tests/test_context_keys.py`:

```python
from apcore.context_keys import (
    TRACING_SPANS,
    TRACING_SAMPLED,
    METRICS_STARTS,
    LOGGING_START,
    REDACTED_OUTPUT,
    RETRY_COUNT_BASE,
)
from apcore.context_key import ContextKey


class TestBuiltinKeys:
    def test_tracing_spans_name(self):
        assert TRACING_SPANS.name == "_apcore.mw.tracing.spans"

    def test_tracing_sampled_name(self):
        assert TRACING_SAMPLED.name == "_apcore.mw.tracing.sampled"

    def test_metrics_starts_name(self):
        assert METRICS_STARTS.name == "_apcore.mw.metrics.starts"

    def test_logging_start_name(self):
        assert LOGGING_START.name == "_apcore.mw.logging.start_time"

    def test_redacted_output_name(self):
        assert REDACTED_OUTPUT.name == "_apcore.executor.redacted_output"

    def test_retry_count_base_name(self):
        assert RETRY_COUNT_BASE.name == "_apcore.mw.retry.count"

    def test_retry_count_base_scoped(self):
        scoped = RETRY_COUNT_BASE.scoped("my_module")
        assert scoped.name == "_apcore.mw.retry.count.my_module"

    def test_all_keys_are_context_key_instances(self):
        for key in [TRACING_SPANS, TRACING_SAMPLED, METRICS_STARTS,
                     LOGGING_START, REDACTED_OUTPUT, RETRY_COUNT_BASE]:
            assert isinstance(key, ContextKey)
```

### Step 2: Implement Python context_keys module

Create `apcore-python/src/apcore/context_keys.py`:

```python
"""Built-in context key constants for apcore framework middleware."""

from apcore.context_key import ContextKey

# Direct keys -- used as-is by middleware
TRACING_SPANS: ContextKey[list] = ContextKey("_apcore.mw.tracing.spans")
TRACING_SAMPLED: ContextKey[bool] = ContextKey("_apcore.mw.tracing.sampled")
METRICS_STARTS: ContextKey[list] = ContextKey("_apcore.mw.metrics.starts")
LOGGING_START: ContextKey[float] = ContextKey("_apcore.mw.logging.start_time")
REDACTED_OUTPUT: ContextKey[dict] = ContextKey("_apcore.executor.redacted_output")

# Base keys -- always use .scoped(module_id) for per-module sub-keys
RETRY_COUNT_BASE: ContextKey[int] = ContextKey("_apcore.mw.retry.count")
```

### Step 3: Run Python tests

```bash
cd apcore-python && python -m pytest tests/test_context_keys.py -v
```

### Step 4: Write tests and implement TypeScript context-keys

Create `apcore-typescript/src/context-keys.ts`:

```typescript
import { ContextKey } from "./context-key";

// Direct keys
export const TRACING_SPANS = new ContextKey<unknown[]>("_apcore.mw.tracing.spans");
export const TRACING_SAMPLED = new ContextKey<boolean>("_apcore.mw.tracing.sampled");
export const METRICS_STARTS = new ContextKey<unknown[]>("_apcore.mw.metrics.starts");
export const LOGGING_START = new ContextKey<number>("_apcore.mw.logging.start_time");
export const REDACTED_OUTPUT = new ContextKey<Record<string, unknown>>("_apcore.executor.redacted_output");

// Base keys (use .scoped(moduleId) for per-module sub-keys)
export const RETRY_COUNT_BASE = new ContextKey<number>("_apcore.mw.retry.count");
```

Add tests in `apcore-typescript/tests/context-keys.test.ts`:

```typescript
import { describe, it, expect } from "vitest";
import {
  TRACING_SPANS, TRACING_SAMPLED, METRICS_STARTS,
  LOGGING_START, REDACTED_OUTPUT, RETRY_COUNT_BASE,
} from "../src/context-keys";
import { ContextKey } from "../src/context-key";

describe("Built-in context keys", () => {
  it("TRACING_SPANS has correct name", () => {
    expect(TRACING_SPANS.name).toBe("_apcore.mw.tracing.spans");
  });
  it("METRICS_STARTS has correct name", () => {
    expect(METRICS_STARTS.name).toBe("_apcore.mw.metrics.starts");
  });
  it("RETRY_COUNT_BASE scoped produces correct name", () => {
    expect(RETRY_COUNT_BASE.scoped("my_module").name).toBe("_apcore.mw.retry.count.my_module");
  });
  it("all keys are ContextKey instances", () => {
    for (const key of [TRACING_SPANS, TRACING_SAMPLED, METRICS_STARTS,
                        LOGGING_START, REDACTED_OUTPUT, RETRY_COUNT_BASE]) {
      expect(key).toBeInstanceOf(ContextKey);
    }
  });
});
```

### Step 5: Run TypeScript tests

```bash
cd apcore-typescript && npx vitest run tests/context-keys.test.ts
```

### Step 6: Implement Rust context_keys module

Create `apcore-rust/src/context_keys.rs`:

```rust
//! Built-in context key constants for apcore framework middleware.

use crate::context_key::ContextKey;
use serde_json::Value;

// Direct keys
pub const TRACING_SPANS: ContextKey<Vec<Value>> = ContextKey::new("_apcore.mw.tracing.spans");
pub const TRACING_SAMPLED: ContextKey<bool> = ContextKey::new("_apcore.mw.tracing.sampled");
pub const METRICS_STARTS: ContextKey<Vec<Value>> = ContextKey::new("_apcore.mw.metrics.starts");
pub const LOGGING_START: ContextKey<f64> = ContextKey::new("_apcore.mw.logging.start_time");
pub const REDACTED_OUTPUT: ContextKey<Value> = ContextKey::new("_apcore.executor.redacted_output");

// Base keys (use .scoped(module_id) for per-module sub-keys)
pub const RETRY_COUNT_BASE: ContextKey<i64> = ContextKey::new("_apcore.mw.retry.count");
```

Register in `lib.rs`:

```rust
pub mod context_keys;
```

### Step 7: Run Rust tests

```bash
cd apcore-rust && cargo test context_key
```

## Acceptance Criteria

- [ ] All 6 built-in keys defined in each SDK with correct `_apcore.*` names
- [ ] Keys are `ContextKey` instances with appropriate type parameters
- [ ] `RETRY_COUNT_BASE.scoped("x")` produces `"_apcore.mw.retry.count.x"`
- [ ] Modules are exported from each SDK's public API

## Dependencies

- **Depends on:** context-key
- **Required by:** data-key-migration

## Estimated Time

1 hour
