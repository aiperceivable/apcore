# Task: ContextKey<T> Typed Accessor

## Goal

Implement `ContextKey<T>` class/struct in all 3 SDKs (Python, TypeScript, Rust) with `get()`, `set()`, `delete()`, `exists()`, and `scoped()` methods. This provides type-safe access to `context.data` entries, replacing raw string key usage throughout the framework.

## Files Involved

### Python SDK
- **Create:** `/Users/tercel/WorkSpace/aipartnerup/apcore-python/src/apcore/context_key.py`
- **Create:** `/Users/tercel/WorkSpace/aipartnerup/apcore-python/tests/test_context_key.py`

### TypeScript SDK
- **Create:** `/Users/tercel/WorkSpace/aipartnerup/apcore-typescript/src/context-key.ts`
- **Modify:** `/Users/tercel/WorkSpace/aipartnerup/apcore-typescript/src/index.ts` (export ContextKey)
- **Create:** `/Users/tercel/WorkSpace/aipartnerup/apcore-typescript/tests/context-key.test.ts`

### Rust SDK
- **Create:** `/Users/tercel/WorkSpace/aipartnerup/apcore-rust/src/context_key.rs`
- **Modify:** `/Users/tercel/WorkSpace/aipartnerup/apcore-rust/src/lib.rs` (add `pub mod context_key;`)
- **Create:** `/Users/tercel/WorkSpace/aipartnerup/apcore-rust/tests/context_key_test.rs`

## Steps

### Step 1: Write failing tests (Python)

Create `apcore-python/tests/test_context_key.py`:

```python
import pytest
from apcore.context_key import ContextKey
from apcore.context import Context


class TestContextKey:
    def _make_ctx(self) -> Context:
        """Create a minimal Context for testing."""
        return Context.create(executor=None)

    def test_get_returns_typed_value(self):
        """AC-001: get() returns typed value from context.data."""
        key = ContextKey[int]("test.counter")
        ctx = self._make_ctx()
        ctx.data["test.counter"] = 42
        assert key.get(ctx) == 42

    def test_get_absent_returns_none(self):
        """AC-016: get() with absent key returns None by default."""
        key = ContextKey[int]("test.absent")
        ctx = self._make_ctx()
        assert key.get(ctx) is None

    def test_get_absent_returns_default(self):
        """AC-016: get() with absent key returns provided default."""
        key = ContextKey[int]("test.absent")
        ctx = self._make_ctx()
        assert key.get(ctx, default=99) == 99

    def test_get_distinguishes_none_from_absent(self):
        """get() with value=None stored should return None, not default."""
        key = ContextKey[int]("test.nullable")
        ctx = self._make_ctx()
        ctx.data["test.nullable"] = None
        assert key.get(ctx, default=99) is None

    def test_set_writes_to_data(self):
        """AC-001: set() writes value to context.data."""
        key = ContextKey[str]("test.name")
        ctx = self._make_ctx()
        key.set(ctx, "hello")
        assert ctx.data["test.name"] == "hello"

    def test_delete_removes_key(self):
        """delete() removes key from context.data."""
        key = ContextKey[int]("test.temp")
        ctx = self._make_ctx()
        key.set(ctx, 10)
        key.delete(ctx)
        assert "test.temp" not in ctx.data

    def test_delete_absent_is_noop(self):
        """AC-017: delete() on absent key is no-op, no exception."""
        key = ContextKey[int]("test.absent")
        ctx = self._make_ctx()
        key.delete(ctx)  # Should not raise

    def test_exists_false_when_absent(self):
        """AC-018: exists() returns False for absent key."""
        key = ContextKey[int]("test.absent")
        ctx = self._make_ctx()
        assert key.exists(ctx) is False

    def test_exists_true_when_present(self):
        """AC-018: exists() returns True after set."""
        key = ContextKey[int]("test.present")
        ctx = self._make_ctx()
        key.set(ctx, 1)
        assert key.exists(ctx) is True

    def test_scoped_creates_subkey(self):
        """AC-002: scoped(suffix) creates sub-key with {name}.{suffix}."""
        base = ContextKey[int]("_apcore.mw.retry.count")
        scoped = base.scoped("mod1")
        assert scoped.name == "_apcore.mw.retry.count.mod1"

    def test_scoped_key_is_independent(self):
        """Scoped key operates on its own data slot."""
        base = ContextKey[int]("base")
        scoped = base.scoped("child")
        ctx = self._make_ctx()
        base.set(ctx, 1)
        scoped.set(ctx, 2)
        assert base.get(ctx) == 1
        assert scoped.get(ctx) == 2

    def test_frozen_dataclass(self):
        """ContextKey is immutable (frozen dataclass)."""
        key = ContextKey[int]("test")
        with pytest.raises(AttributeError):
            key.name = "changed"  # type: ignore
```

### Step 2: Implement Python ContextKey

Create `apcore-python/src/apcore/context_key.py`:

```python
"""Typed key for type-safe access to context.data."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Generic, TypeVar

from apcore.context import Context

T = TypeVar("T")

_MISSING = object()


@dataclass(frozen=True)
class ContextKey(Generic[T]):
    """Typed key for context.data with namespace isolation."""

    name: str

    def get(self, ctx: Context, default: T | None = None) -> T | None:
        value = ctx.data.get(self.name, _MISSING)
        return default if value is _MISSING else value  # type: ignore[return-value]

    def set(self, ctx: Context, value: T) -> None:
        ctx.data[self.name] = value

    def delete(self, ctx: Context) -> None:
        ctx.data.pop(self.name, None)

    def exists(self, ctx: Context) -> bool:
        return self.name in ctx.data

    def scoped(self, suffix: str) -> ContextKey[T]:
        return ContextKey(f"{self.name}.{suffix}")
```

### Step 3: Run Python tests and verify all pass

```bash
cd apcore-python && python -m pytest tests/test_context_key.py -v
```

### Step 4: Write failing tests (TypeScript)

Create `apcore-typescript/tests/context-key.test.ts`:

```typescript
import { describe, it, expect } from "vitest";
import { ContextKey } from "../src/context-key";
import { Context } from "../src/context";

function makeCtx(): Context {
  return Context.create({ executor: null as any });
}

describe("ContextKey", () => {
  it("AC-001: get() returns typed value from context.data", () => {
    const key = new ContextKey<number>("test.counter");
    const ctx = makeCtx();
    ctx.data["test.counter"] = 42;
    expect(key.get(ctx)).toBe(42);
  });

  it("AC-016: get() with absent key returns undefined", () => {
    const key = new ContextKey<number>("test.absent");
    const ctx = makeCtx();
    expect(key.get(ctx)).toBeUndefined();
  });

  it("AC-016: get() with absent key returns default", () => {
    const key = new ContextKey<number>("test.absent");
    const ctx = makeCtx();
    expect(key.get(ctx, 99)).toBe(99);
  });

  it("AC-001: set() writes value to context.data", () => {
    const key = new ContextKey<string>("test.name");
    const ctx = makeCtx();
    key.set(ctx, "hello");
    expect(ctx.data["test.name"]).toBe("hello");
  });

  it("delete() removes key from context.data", () => {
    const key = new ContextKey<number>("test.temp");
    const ctx = makeCtx();
    key.set(ctx, 10);
    key.delete(ctx);
    expect("test.temp" in ctx.data).toBe(false);
  });

  it("AC-017: delete() on absent key is no-op", () => {
    const key = new ContextKey<number>("test.absent");
    const ctx = makeCtx();
    expect(() => key.delete(ctx)).not.toThrow();
  });

  it("AC-018: exists() returns false for absent key", () => {
    const key = new ContextKey<number>("test.absent");
    const ctx = makeCtx();
    expect(key.exists(ctx)).toBe(false);
  });

  it("AC-018: exists() returns true after set", () => {
    const key = new ContextKey<number>("test.present");
    const ctx = makeCtx();
    key.set(ctx, 1);
    expect(key.exists(ctx)).toBe(true);
  });

  it("AC-002: scoped(suffix) creates sub-key", () => {
    const base = new ContextKey<number>("_apcore.mw.retry.count");
    const scoped = base.scoped("mod1");
    expect(scoped.name).toBe("_apcore.mw.retry.count.mod1");
  });
});
```

### Step 5: Implement TypeScript ContextKey

Create `apcore-typescript/src/context-key.ts`:

```typescript
import { Context } from "./context";

export class ContextKey<T> {
  constructor(readonly name: string) {}

  get(ctx: Context, defaultValue?: T): T | undefined {
    const val = ctx.data[this.name];
    return val !== undefined ? (val as T) : defaultValue;
  }

  set(ctx: Context, value: T): void {
    ctx.data[this.name] = value;
  }

  delete(ctx: Context): void {
    delete ctx.data[this.name];
  }

  exists(ctx: Context): boolean {
    return this.name in ctx.data;
  }

  scoped(suffix: string): ContextKey<T> {
    return new ContextKey(`${this.name}.${suffix}`);
  }
}
```

### Step 6: Run TypeScript tests and verify all pass

```bash
cd apcore-typescript && npx vitest run tests/context-key.test.ts
```

### Step 7: Write failing tests (Rust)

Create `apcore-rust/tests/context_key_test.rs`:

```rust
use apcore::context::Context;
use apcore::context_key::ContextKey;
use serde_json::json;

// AC-001: get() returns typed value from context.data
#[test]
fn test_get_returns_typed_value() {
    let key: ContextKey<i64> = ContextKey::new("test.counter");
    let ctx = Context::create_test();
    key.set(&ctx, 42);
    assert_eq!(key.get(&ctx), Some(42));
}

// AC-016: get() with absent key returns None
#[test]
fn test_get_absent_returns_none() {
    let key: ContextKey<i64> = ContextKey::new("test.absent");
    let ctx = Context::create_test();
    assert_eq!(key.get(&ctx), None);
}

// AC-001: set() writes value to context.data
#[test]
fn test_set_writes_to_data() {
    let key: ContextKey<String> = ContextKey::new("test.name");
    let ctx = Context::create_test();
    key.set(&ctx, "hello".to_string());
    let map = ctx.data.read().unwrap();
    assert_eq!(map.get("test.name"), Some(&json!("hello")));
}

// AC-017: delete() on absent key is no-op
#[test]
fn test_delete_absent_is_noop() {
    let key: ContextKey<i64> = ContextKey::new("test.absent");
    let ctx = Context::create_test();
    key.delete(&ctx); // Should not panic
}

// AC-018: exists() returns false for absent, true for present
#[test]
fn test_exists() {
    let key: ContextKey<i64> = ContextKey::new("test.flag");
    let ctx = Context::create_test();
    assert!(!key.exists(&ctx));
    key.set(&ctx, 1);
    assert!(key.exists(&ctx));
}

// AC-002: scoped(suffix) creates sub-key
#[test]
fn test_scoped_creates_subkey() {
    let base: ContextKey<i64> = ContextKey::new("_apcore.mw.retry.count");
    let scoped = base.scoped("mod1");
    assert_eq!(scoped.name.as_ref(), "_apcore.mw.retry.count.mod1");
}
```

### Step 8: Implement Rust ContextKey

Create `apcore-rust/src/context_key.rs`:

```rust
use std::borrow::Cow;
use std::marker::PhantomData;

pub struct ContextKey<T> {
    pub name: Cow<'static, str>,
    _marker: PhantomData<T>,
}

impl<T> ContextKey<T> {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name: Cow::Borrowed(name),
            _marker: PhantomData,
        }
    }

    pub fn scoped(&self, suffix: &str) -> ContextKey<T> {
        ContextKey {
            name: Cow::Owned(format!("{}.{}", self.name, suffix)),
            _marker: PhantomData,
        }
    }
}

impl<T: serde::de::DeserializeOwned> ContextKey<T> {
    pub fn get(&self, ctx: &Context<impl std::any::Any>) -> Option<T> {
        let map = ctx.data.read().ok()?;
        let val = map.get(self.name.as_ref())?;
        serde_json::from_value(val.clone()).ok()
    }

    pub fn exists(&self, ctx: &Context<impl std::any::Any>) -> bool {
        ctx.data
            .read()
            .map(|map| map.contains_key(self.name.as_ref()))
            .unwrap_or(false)
    }
}

impl<T: serde::Serialize> ContextKey<T> {
    pub fn set(&self, ctx: &Context<impl std::any::Any>, value: T) {
        if let Ok(mut map) = ctx.data.write() {
            if let Ok(v) = serde_json::to_value(value) {
                map.insert(self.name.to_string(), v);
            }
        }
    }

    pub fn delete(&self, ctx: &Context<impl std::any::Any>) {
        if let Ok(mut map) = ctx.data.write() {
            map.remove(self.name.as_ref());
        }
    }
}

use crate::context::Context;
```

### Step 9: Register module in lib.rs and run Rust tests

```bash
cd apcore-rust && cargo test context_key
```

## Acceptance Criteria

- [x] **AC-001**: `ContextKey.get()` returns typed value from `context.data` (all 3 SDKs)
- [x] **AC-002**: `ContextKey.scoped(suffix)` creates sub-key with `{name}.{suffix}` (all 3 SDKs)
- [x] **AC-016**: `ContextKey.get()` with absent key returns default (all 3 SDKs)
- [x] **AC-017**: `ContextKey.delete()` on absent key is no-op (all 3 SDKs)
- [x] **AC-018**: `ContextKey.exists()` returns False/false for absent, True/true for present (all 3 SDKs)

## Dependencies

- **Depends on:** none
- **Required by:** builtin-keys, data-key-migration, serialization

## Estimated Time

3 hours
