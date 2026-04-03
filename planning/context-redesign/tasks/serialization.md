# Task: Context Serialization Protocol

## Goal

Implement `serialize()` and `deserialize()` methods on Context in all 3 SDKs. Serialization includes `_context_version: 1` at top level, excludes non-serializable fields (`executor`, `services`, `cancel_token`, `global_deadline`), and filters `_`-prefixed keys from `data`. Deserialization handles forward compatibility (unknown versions warn, unknown fields preserved).

## Files Involved

### Python SDK
- **Modify:** `/Users/tercel/WorkSpace/aipartnerup/apcore-python/src/apcore/context.py` (add `serialize()` and `deserialize()`)
- **Create:** `/Users/tercel/WorkSpace/aipartnerup/apcore-python/tests/test_context_serialization.py`

### TypeScript SDK
- **Modify:** `/Users/tercel/WorkSpace/aipartnerup/apcore-typescript/src/context.ts` (add `serialize()` and static `deserialize()`)
- **Create:** `/Users/tercel/WorkSpace/aipartnerup/apcore-typescript/tests/context-serialization.test.ts`

### Rust SDK
- **Modify:** `/Users/tercel/WorkSpace/aipartnerup/apcore-rust/src/context.rs` (add `serialize()` and `deserialize()`)
- **Create or extend:** `/Users/tercel/WorkSpace/aipartnerup/apcore-rust/tests/context_serialization_test.rs`

## Steps

### Step 1: Write failing tests (Python)

Create `apcore-python/tests/test_context_serialization.py`:

```python
import pytest
from apcore.context import Context, Identity


class TestContextSerialization:
    def _make_ctx(self) -> Context:
        ctx = Context.create(executor=None)
        ctx._identity = Identity(
            id="user-1", type="user",
            roles=["admin"], attrs={"org": "acme"},
        )
        return ctx

    def test_serialize_includes_context_version(self):
        """AC-003: serialization includes _context_version: 1."""
        ctx = self._make_ctx()
        result = ctx.serialize()
        assert result["_context_version"] == 1

    def test_serialize_includes_required_fields(self):
        """Serialized output includes trace_id, caller_id, call_chain, identity."""
        ctx = self._make_ctx()
        result = ctx.serialize()
        assert "trace_id" in result
        assert "caller_id" in result
        assert "call_chain" in result
        assert "identity" in result

    def test_serialize_identity_structure(self):
        """Identity serializes with id, type, roles, attrs."""
        ctx = self._make_ctx()
        result = ctx.serialize()
        identity = result["identity"]
        assert identity["id"] == "user-1"
        assert identity["type"] == "user"
        assert identity["roles"] == ["admin"]
        assert identity["attrs"] == {"org": "acme"}

    def test_serialize_excludes_executor(self):
        """AC-004: executor is not in serialized output."""
        ctx = self._make_ctx()
        result = ctx.serialize()
        assert "executor" not in result

    def test_serialize_excludes_services(self):
        """AC-004: services is not in serialized output."""
        ctx = self._make_ctx()
        result = ctx.serialize()
        assert "services" not in result

    def test_serialize_excludes_cancel_token(self):
        """AC-004: cancel_token is not in serialized output."""
        ctx = self._make_ctx()
        result = ctx.serialize()
        assert "cancel_token" not in result

    def test_serialize_excludes_global_deadline(self):
        """AC-004: global_deadline is not in serialized output."""
        ctx = self._make_ctx()
        result = ctx.serialize()
        assert "global_deadline" not in result

    def test_serialize_filters_underscore_data_keys(self):
        """AC-005: _-prefixed keys excluded from serialized data."""
        ctx = self._make_ctx()
        ctx.data["_apcore.mw.metrics.starts"] = [1, 2, 3]
        ctx.data["_internal"] = "hidden"
        ctx.data["public.counter"] = 42
        ctx.data["app.name"] = "test"
        result = ctx.serialize()
        assert "_apcore.mw.metrics.starts" not in result["data"]
        assert "_internal" not in result["data"]
        assert result["data"]["public.counter"] == 42
        assert result["data"]["app.name"] == "test"

    def test_serialize_empty_data(self):
        """Serialization with no public data keys produces empty data dict."""
        ctx = self._make_ctx()
        ctx.data["_private"] = "hidden"
        result = ctx.serialize()
        assert result["data"] == {}

    def test_deserialize_roundtrip(self):
        """Serialize then deserialize preserves fields."""
        ctx = self._make_ctx()
        ctx.data["app.counter"] = 42
        serialized = ctx.serialize()
        restored = Context.deserialize(serialized)
        assert restored.trace_id == ctx.trace_id
        assert restored.caller_id == ctx.caller_id
        assert restored.data.get("app.counter") == 42

    def test_deserialize_executor_is_none(self):
        """After deserialization, executor is None."""
        ctx = self._make_ctx()
        serialized = ctx.serialize()
        restored = Context.deserialize(serialized)
        assert restored.executor is None

    def test_deserialize_future_version_warns(self, caplog):
        """Deserializing _context_version > 1 logs warning but succeeds."""
        data = {
            "_context_version": 99,
            "trace_id": "abc-123",
            "caller_id": "test",
            "call_chain": [],
            "data": {},
        }
        with caplog.at_level("WARNING"):
            restored = Context.deserialize(data)
        assert restored.trace_id == "abc-123"
        # Should have logged a warning about unknown version
```

### Step 2: Implement Python serialize/deserialize

Add to `apcore-python/src/apcore/context.py`:

```python
def serialize(self) -> dict:
    """Serialize Context to a dict suitable for JSON encoding.

    Includes _context_version: 1 at top level.
    Excludes: executor, services, cancel_token, global_deadline.
    Filters _-prefixed keys from data.
    """
    result = {
        "_context_version": 1,
        "trace_id": self.trace_id,
        "caller_id": self.caller_id,
        "call_chain": list(self.call_chain),
    }
    if self.identity is not None:
        result["identity"] = {
            "id": self.identity.id,
            "type": self.identity.type,
            "roles": list(self.identity.roles),
            "attrs": dict(self.identity.attrs),
        }
    if self.redacted_inputs is not None:
        result["redacted_inputs"] = self.redacted_inputs
    # Filter _-prefixed keys from data
    result["data"] = {
        k: v for k, v in self.data.items() if not k.startswith("_")
    }
    return result

@classmethod
def deserialize(cls, data: dict) -> "Context":
    """Deserialize a dict (from JSON) into a Context.

    Non-serializable fields (executor, services, cancel_token,
    global_deadline) are set to None after deserialization.
    """
    import logging
    logger = logging.getLogger(__name__)

    version = data.get("_context_version", 1)
    if version > 1:
        logger.warning(
            "Unknown _context_version %d (expected 1). "
            "Proceeding with best-effort deserialization.",
            version,
        )

    identity = None
    if "identity" in data:
        id_data = data["identity"]
        identity = Identity(
            id=id_data["id"],
            type=id_data["type"],
            roles=id_data.get("roles", []),
            attrs=id_data.get("attrs", {}),
        )

    ctx = cls.__new__(cls)
    ctx.trace_id = data.get("trace_id", "")
    ctx.caller_id = data.get("caller_id", "")
    ctx.call_chain = list(data.get("call_chain", []))
    ctx._identity = identity
    ctx.redacted_inputs = data.get("redacted_inputs")
    ctx.data = dict(data.get("data", {}))
    ctx.executor = None
    ctx.services = None
    ctx.cancel_token = None
    ctx.global_deadline = None
    return ctx
```

### Step 3: Run Python tests

```bash
cd apcore-python && python -m pytest tests/test_context_serialization.py -v
```

### Step 4: Write failing tests (TypeScript)

Create `apcore-typescript/tests/context-serialization.test.ts`:

```typescript
import { describe, it, expect } from "vitest";
import { Context } from "../src/context";

function makeCtx(): Context {
  return Context.create({
    executor: null as any,
    identity: {
      id: "user-1",
      type: "user",
      roles: ["admin"],
      attrs: { org: "acme" },
    },
  });
}

describe("Context serialization", () => {
  it("AC-003: includes _context_version: 1", () => {
    const result = makeCtx().serialize();
    expect(result._context_version).toBe(1);
  });

  it("AC-004: excludes executor, services, cancelToken, globalDeadline", () => {
    const result = makeCtx().serialize();
    expect(result).not.toHaveProperty("executor");
    expect(result).not.toHaveProperty("services");
    expect(result).not.toHaveProperty("cancelToken");
    expect(result).not.toHaveProperty("globalDeadline");
  });

  it("AC-005: filters _-prefixed keys from data", () => {
    const ctx = makeCtx();
    ctx.data["_apcore.internal"] = "hidden";
    ctx.data["public.counter"] = 42;
    const result = ctx.serialize();
    expect(result.data).not.toHaveProperty("_apcore.internal");
    expect(result.data["public.counter"]).toBe(42);
  });

  it("includes identity with correct structure", () => {
    const result = makeCtx().serialize();
    expect(result.identity.id).toBe("user-1");
    expect(result.identity.type).toBe("user");
    expect(result.identity.roles).toEqual(["admin"]);
  });

  it("deserialize roundtrip preserves fields", () => {
    const ctx = makeCtx();
    ctx.data["app.counter"] = 42;
    const serialized = ctx.serialize();
    const restored = Context.deserialize(serialized);
    expect(restored.traceId).toBe(ctx.traceId);
    expect(restored.data["app.counter"]).toBe(42);
  });

  it("deserialized context has null executor", () => {
    const serialized = makeCtx().serialize();
    const restored = Context.deserialize(serialized);
    expect(restored.executor).toBeNull();
  });
});
```

### Step 5: Implement TypeScript serialize/deserialize

Add to `apcore-typescript/src/context.ts`:

```typescript
serialize(): Record<string, unknown> {
    const result: Record<string, unknown> = {
        _context_version: 1,
        trace_id: this.traceId,
        caller_id: this.callerId,
        call_chain: [...this.callChain],
    };
    if (this.identity) {
        result.identity = {
            id: this.identity.id,
            type: this.identity.type,
            roles: [...this.identity.roles],
            attrs: { ...this.identity.attrs },
        };
    }
    if (this.redactedInputs !== undefined) {
        result.redacted_inputs = this.redactedInputs;
    }
    // Filter _-prefixed keys from data
    const filteredData: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(this.data)) {
        if (!k.startsWith("_")) {
            filteredData[k] = v;
        }
    }
    result.data = filteredData;
    return result;
}

static deserialize(data: Record<string, unknown>): Context {
    const version = (data._context_version as number) ?? 1;
    if (version > 1) {
        console.warn(
            `Unknown _context_version ${version} (expected 1). Proceeding with best-effort deserialization.`
        );
    }
    // Reconstruct with null for non-serializable fields
    return new Context({
        traceId: data.trace_id as string,
        callerId: data.caller_id as string,
        callChain: data.call_chain as string[] ?? [],
        identity: data.identity as any ?? null,
        redactedInputs: data.redacted_inputs as any,
        data: (data.data as Record<string, unknown>) ?? {},
        executor: null as any,
        services: null as any,
        cancelToken: null as any,
        globalDeadline: null,
    });
}
```

### Step 6: Run TypeScript tests

```bash
cd apcore-typescript && npx vitest run tests/context-serialization.test.ts
```

### Step 7: Write failing tests (Rust)

Create `apcore-rust/tests/context_serialization_test.rs`:

```rust
use apcore::context::{Context, Identity};
use std::collections::HashMap;

#[test]
fn test_serialize_includes_context_version() {
    // AC-003
    let ctx = Context::<()>::create_test();
    let serialized = ctx.serialize();
    assert_eq!(serialized["_context_version"], 1);
}

#[test]
fn test_serialize_excludes_non_serializable_fields() {
    // AC-004
    let ctx = Context::<()>::create_test();
    let serialized = ctx.serialize();
    assert!(!serialized.as_object().unwrap().contains_key("executor"));
    assert!(!serialized.as_object().unwrap().contains_key("services"));
    assert!(!serialized.as_object().unwrap().contains_key("cancel_token"));
    assert!(!serialized.as_object().unwrap().contains_key("global_deadline"));
}

#[test]
fn test_serialize_filters_underscore_data_keys() {
    // AC-005
    let ctx = Context::<()>::create_test();
    {
        let mut data = ctx.data.write().unwrap();
        data.insert("_apcore.internal".to_string(), serde_json::json!("hidden"));
        data.insert("public.counter".to_string(), serde_json::json!(42));
    }
    let serialized = ctx.serialize();
    let data = serialized["data"].as_object().unwrap();
    assert!(!data.contains_key("_apcore.internal"));
    assert_eq!(data["public.counter"], 42);
}

#[test]
fn test_deserialize_roundtrip() {
    let ctx = Context::<()>::create_test();
    {
        let mut data = ctx.data.write().unwrap();
        data.insert("app.counter".to_string(), serde_json::json!(42));
    }
    let serialized = ctx.serialize();
    let restored = Context::<()>::deserialize(serialized).unwrap();
    assert_eq!(restored.trace_id, ctx.trace_id);
    let data = restored.data.read().unwrap();
    assert_eq!(data.get("app.counter"), Some(&serde_json::json!(42)));
}
```

### Step 8: Implement Rust serialize/deserialize

Add to `apcore-rust/src/context.rs`:

```rust
impl<T> Context<T> {
    pub fn serialize(&self) -> serde_json::Value {
        let mut result = serde_json::json!({
            "_context_version": 1,
            "trace_id": self.trace_id,
            "caller_id": self.caller_id,
            "call_chain": self.call_chain,
        });

        if let Some(ref identity) = self.identity {
            result["identity"] = serde_json::to_value(identity).unwrap_or_default();
        }

        if let Some(ref redacted_inputs) = self.redacted_inputs {
            result["redacted_inputs"] = serde_json::to_value(redacted_inputs)
                .unwrap_or_default();
        }

        // Filter _-prefixed keys from data
        let filtered: HashMap<String, serde_json::Value> = self
            .data
            .read()
            .map(|map| {
                map.iter()
                    .filter(|(k, _)| !k.starts_with('_'))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            })
            .unwrap_or_default();

        result["data"] = serde_json::to_value(filtered).unwrap_or_default();
        result
    }
}

impl<T: Default> Context<T> {
    pub fn deserialize(value: serde_json::Value) -> Result<Self, serde_json::Error> {
        use log::warn;

        let obj = value.as_object().ok_or_else(|| {
            serde::de::Error::custom("expected object")
        })?;

        let version = obj
            .get("_context_version")
            .and_then(|v| v.as_i64())
            .unwrap_or(1);

        if version > 1 {
            warn!(
                "Unknown _context_version {} (expected 1). \
                 Proceeding with best-effort deserialization.",
                version
            );
        }

        let identity: Option<Identity> = obj
            .get("identity")
            .and_then(|v| serde_json::from_value(v.clone()).ok());

        let data_map: HashMap<String, serde_json::Value> = obj
            .get("data")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        // Reconstruct context with None for non-serializable fields
        Ok(Context {
            trace_id: obj.get("trace_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            caller_id: obj.get("caller_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            call_chain: obj.get("call_chain")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default(),
            identity,
            redacted_inputs: obj.get("redacted_inputs")
                .and_then(|v| serde_json::from_value(v.clone()).ok()),
            data: std::sync::Arc::new(std::sync::RwLock::new(data_map)),
            executor: None,
            services: None,
            cancel_token: None,
            global_deadline: None,
        })
    }
}
```

### Step 9: Run Rust tests

```bash
cd apcore-rust && cargo test context_serialization
```

### Step 10: Run full test suites across all SDKs

```bash
cd apcore-python && python -m pytest
cd apcore-typescript && npx vitest run
cd apcore-rust && cargo test
```

## Acceptance Criteria

- [x] **AC-003**: Context serialization includes `_context_version: 1` at top level (all 3 SDKs)
- [x] **AC-004**: Serialization excludes `executor`, `services`, `cancel_token`, `global_deadline` (all 3 SDKs)
- [x] **AC-005**: Serialization filters `_`-prefixed keys from `data` (all 3 SDKs)
- [ ] Deserialization roundtrip preserves `trace_id`, `caller_id`, `call_chain`, `identity`, `data`
- [ ] Deserialized context has `None`/`null` for `executor`, `services`, `cancel_token`, `global_deadline`
- [ ] Future `_context_version` > 1 logs warning but does not fail
- [ ] All existing tests pass

## Dependencies

- **Depends on:** context-key, rust-field-alignment, rust-identity-immutable, ts-global-deadline
- **Required by:** none (final task)

## Estimated Time

3 hours
