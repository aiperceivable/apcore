---
description: "Historical design document (superseded by PROTOCOL_SPEC 4.4.1) for the v0.17 Context, Annotations, and ACL redesign, retained for design rationale on the ModuleAnnotations.extra wire format."
---

# Design: Context, Annotations & ACL Interface Redesign

> **⚠️ Status: Historical Design Document — Superseded by PROTOCOL_SPEC §4.4.1 (v0.18.0)**
>
> This document captures the original problem statement and design proposals
> for the Annotations / Context / ACL redesign that shipped in apcore v0.17.
> The normative wire format for `ModuleAnnotations.extra` is now defined in
> [PROTOCOL_SPEC §4.4.1](./protocol-spec.md#441-annotations-extension-field-extra-wire-format).
>
> Specifically, the following parts of this document are no longer accurate
> as of v0.18.0:
>
> - **§2.1** says "ModuleAnnotations is frozen with 11 fields" — there are now
>   12 canonical fields, with `extra` formally defined per §4.4.1.
> - **§2 implementation hints** show `#[serde(flatten)]` as the recommended
>   Rust pattern. This approach is now FORBIDDEN by §4.4.1 producer rule 2;
>   the canonical wire format is a nested `"extra"` object. apcore-rust
>   v0.18.0 fixed this and removed the `flatten` attribute.
> - **§3.6** shows condition evaluation returning `False` — commented
>   `# fail-closed` — for an unknown condition key and for an async handler on
>   the sync path. Both are now **unevaluable** outcomes, not `False`, and
>   §6.1.1 requires them to resolve toward refusing access: a `deny` rule takes
>   effect, an `allow` rule does not grant. Returning `False` was fail-closed
>   only on `allow` rules; on `deny` rules it failed **open**, which is the
>   defect [PROTOCOL_SPEC §6.1.1](./protocol-spec.md#611-unevaluable-conditions-v1220-100)
>   (v1.22.0) exists to close. The pseudocode below is retained as the original
>   design, not as current behaviour.
>
> Read this document for design rationale and historical context only. For
> current normative behavior, always consult PROTOCOL_SPEC.

> Original status: **Draft** | Author: apcore team | Date: 2026-04-02

This document specifies interface-level changes to three core apcore modules to improve cross-language consistency, type safety, and extensibility. All changes apply equally to Python, TypeScript, and Rust SDKs.

---

## 1. Context

### 1.1 Problem Statement

- Rust SDK has 3 fields (`created_at`, `parent_trace_id`, `trace_context`) not in spec or other SDKs
- TypeScript SDK missing `global_deadline` field that Python and Rust have
- `context.data` key naming inconsistent — `_metrics_starts` violates `_apcore.` prefix convention
- `context.data` access is untyped — callers must know key strings and expected types from documentation
- Rust `Identity` is mutable, violating spec's immutability requirement
- No version field for forward-compatible serialization

### 1.2 Canonical Context Definition

All three SDKs MUST implement exactly these fields:

```
Context<T> {
  // ─── Core (MUST, execution engine contract) ───
  trace_id:        string              // 32-char hex (W3C Trace Context), immutable after creation
  caller_id:       string | nil        // module that initiated this call
  call_chain:      list[string]        // ordered call stack, max depth 32
  executor:        Executor | nil      // for nested calls, MUST NOT serialize
  identity:        Identity | nil      // caller identity, immutable

  // ─── Execution control (SHOULD) ───
  cancel_token:    CancelToken | nil   // cancellation propagation, MUST NOT serialize
  global_deadline: float | nil         // absolute deadline as epoch seconds.
                                       // All languages: f64/float/number (epoch seconds).
                                       // MUST NOT serialize.

  // ─── Observability (SHOULD) ───
  redacted_inputs: map | nil           // sanitized inputs for logging

  // ─── Injection (MAY) ───
  services:        T | nil             // DI container, MUST NOT serialize

  // ─── Extension storage (MUST) ───
  data:            map[string, any]    // middleware/module state bag
}
```

```
Identity {
  id:    string                        // required, immutable
  type:  string = "user"               // immutable
  roles: list[string]                  // immutable (tuple in Python, frozen array in TS)
  attrs: map[string, any]              // immutable (frozen in Python/TS)
}
```

> **Rust immutability:** Rust has no built-in "frozen" concept. Identity MUST be
> constructed via `Identity::new(id, type, roles, attrs)` and expose fields via
> `pub fn roles(&self) -> &[String]` (shared reference only). Fields MUST NOT be
> `pub` directly. This prevents mutation after construction.

Fields NOT in this list MUST NOT exist as struct/class fields in any SDK. If an SDK needs additional internal state, it MUST use `data` with `_apcore.*` key prefix.

### 1.3 Cross-Language Alignment Changes

| SDK | Change | Reason |
|-----|--------|--------|
| **Rust** | Remove `created_at` field | Not in spec, not in Python/TS. If needed, use `data["_apcore.created_at"]` |
| **Rust** | Remove `parent_trace_id` field | Not in spec. Derive from `call_chain[0]` or use `data` |
| **Rust** | Remove `trace_context` field | Not in spec. Move to `data["_apcore.trace_context"]` |
| **Rust** | `global_deadline`: change `Option<Instant>` → `Option<f64>` (epoch seconds) | Align with Python/TS. `Instant` is not serializable and not cross-process. Rust SDK has no external users yet — safe to change. |
| **Rust** | Make `Identity` fields immutable | Spec requires immutability. Use pub getters with private fields, or make struct fields non-pub with a constructor |
| **TypeScript** | Add `globalDeadline: number \| null` field | Python and Rust already have it |
| **All** | Fix `_metrics_starts` → `_apcore.mw.metrics.starts` | Naming convention violation |
| **All** | Fix `_usage_starts` → `_apcore.mw.usage.starts` | Same |
| **All** | Fix `_obs_logging_starts` → `_apcore.mw.logging.starts` | Same |

### 1.4 ContextKey\<T\> — Typed Data Accessor

A helper type for type-safe access to `context.data`. Does NOT change the Context class itself.

**Python:**
```python
@dataclass(frozen=True)
class ContextKey(Generic[T]):
    """Typed key for context.data with namespace isolation."""
    name: str

    def get(self, ctx: Context, default: T | None = None) -> T | None:
        # Use sentinel to distinguish "key absent" from "value is None".
        _MISSING = object()
        value = ctx.data.get(self.name, _MISSING)
        return default if value is _MISSING else value  # type: ignore[return-value]

    def set(self, ctx: Context, value: T) -> None:
        ctx.data[self.name] = value

    def delete(self, ctx: Context) -> None:
        ctx.data.pop(self.name, None)

    def exists(self, ctx: Context) -> bool:
        return self.name in ctx.data

    def scoped(self, suffix: str) -> "ContextKey[T]":
        """Create a sub-key for per-module or per-instance scoping."""
        return ContextKey(f"{self.name}.{suffix}")
```

**TypeScript:**
```typescript
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

**Rust:**
```rust
use std::borrow::Cow;
use std::marker::PhantomData;

/// Typed key for context.data access.
///
/// Uses `Cow<'static, str>` so static keys (defined at module level) are
/// zero-allocation, while `scoped()` keys allocate only when needed.
pub struct ContextKey<T> {
    pub name: Cow<'static, str>,
    _marker: PhantomData<T>,
}

impl<T> ContextKey<T> {
    /// Create a static key (no allocation). Use for module-level constants.
    /// Requires Rust >= 1.48 (Cow::Borrowed is const since 1.48).
    pub const fn new(name: &'static str) -> Self {
        Self { name: Cow::Borrowed(name), _marker: PhantomData }
    }

    /// Create a scoped sub-key (allocates). Use for per-module tracking.
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
}

impl<T: serde::Serialize> ContextKey<T> {
    pub fn set(&self, ctx: &Context<impl std::any::Any>, value: T) {
        if let Ok(mut map) = ctx.data.write() {
            if let Ok(v) = serde_json::to_value(value) {
                map.insert(self.name.to_string(), v);
            }
        }
    }
}
```

> **Note on Rust trait bounds:** `get` requires `DeserializeOwned`, `set` requires `Serialize`.
> They are in separate `impl` blocks because a key may only be read or only be written.
> For keys that do both, `T` must satisfy both bounds — this is always the case for
> standard types (`bool`, `i64`, `String`, `Vec<T>`, `HashMap<K,V>`, etc.).

### 1.5 Built-in Context Keys

Framework-internal keys, exported from the SDK for middleware authors:

```python
# Python — in apcore/context_keys.py

# Direct keys (get/set directly)
TRACING_SPANS    = ContextKey[list]("_apcore.mw.tracing.spans")
TRACING_SAMPLED  = ContextKey[bool]("_apcore.mw.tracing.sampled")
METRICS_STARTS   = ContextKey[list]("_apcore.mw.metrics.starts")
LOGGING_START    = ContextKey[float]("_apcore.mw.logging.start_time")
REDACTED_OUTPUT  = ContextKey[dict]("_apcore.executor.redacted_output")

# Base keys (use .scoped(module_id) to create per-module sub-keys)
RETRY_COUNT_BASE = ContextKey[int]("_apcore.mw.retry.count")
```

The `scoped()` method creates per-module keys from a base key:

```python
# Retry middleware: track count per module
retry_key = RETRY_COUNT_BASE.scoped(module_id)  # → "_apcore.mw.retry.count.email.send"
count = retry_key.get(ctx, 0)
retry_key.set(ctx, count + 1)
```

> **Convention:** Keys that are always scoped per-module are suffixed with `_BASE`
> to signal that they should not be used directly without `.scoped()`.

### 1.6 Data Key Naming Convention (spec-level)

```
Prefix                              Owner                  Example
─────────────────────────────────── ────────────────────── ──────────────────────────────────
_apcore.mw.{middleware}.{key}       apcore built-in MW     _apcore.mw.tracing.spans
_apcore.executor.{key}              apcore executor        _apcore.executor.redacted_output
{package}.{key}                     ecosystem package      apcore-mcp.session_id
(any key without _ prefix)          user application       request_id, tenant_id
_secret_{key}                       sensitive data filter  _secret_api_token
```

Rules:
- `_apcore.*` prefix is RESERVED — user code MUST NOT write to these keys
- `_` prefixed keys (except `_context_version`) are NOT serialized when crossing process boundaries
- Keys without `_` prefix are serialized
- `_secret_*` keys are filtered by logging middleware for redaction

> **Relationship to Pipeline redesign** (see `design-execution-pipeline.md` §2.2):
> `context.data` with ContextKey is **Tier 2** of the two-tier data model. Pipeline-
> essential data (module lookup result, validated inputs/outputs) is stored in
> PipelineContext fields (Tier 1), NOT in context.data. ContextKey is used by
> middleware and custom pipeline steps for extension state only.

### 1.7 Serialization Convention

```json
{
  "_context_version": 1,
  "trace_id": "550e8400-e29b-41d4-a716-446655440000",
  "caller_id": "api.users.get",
  "call_chain": ["api.users.get", "db.users.find"],
  "identity": {
    "id": "user-123",
    "type": "user",
    "roles": ["admin"],
    "attrs": {}
  },
  "redacted_inputs": {"password": "[REDACTED]"},
  "data": {
    "request_id": "req-abc",
    "tenant_id": "tenant-456"
  }
}
```

Rules:
- **Top-level fields:** `_context_version` is a RESERVED top-level serialization field, always included (currently `1`). It is NOT inside `data` — it is a peer of `trace_id`, `caller_id`, etc.
- **Fields NOT serialized:** `executor`, `services`, `cancel_token`, `global_deadline`
- **data key filtering:** Within the `data` dict, keys starting with `_` are filtered out during serialization. This does NOT affect top-level fields like `_context_version`.
- Old SDK receiving `_context_version: 2` SHOULD warn but MUST NOT crash
- Unknown top-level fields in serialized context MUST be preserved (forward compat)

### 1.8 New Core Field Upgrade Path

When a new field needs to become a core Context field:

```
Phase 1: Use as data key          → data["_apcore.tenant_id"]
Phase 2: Ecosystem validates need  → multiple packages depend on it
Phase 3: Promote to core field     → add tenant_id: string | nil
Phase 4: Backward compat           → read from core field, fallback to data key
```

This ensures no breaking change: old SDKs still work via data, new SDKs use the core field.

---

## 2. Annotations

### 2.1 Problem Statement

- No extension mechanism — `ModuleAnnotations` is frozen with 11 fields
- apcore-cli reads `approval_message` which doesn't exist in the definition (workaround via duck-typing)
- TypeScript has 5 optional fields while Python/Rust treat all as required (with defaults)
- `pagination_style` is `Literal["cursor", "offset", "page"]` in Python — can't add new styles without breaking
- `cache_ttl` type inconsistent: Python `int`, TypeScript `number`, Rust `u64`
- No canonical wire format (snake_case vs camelCase)
- 3 fields (`cache_ttl`, `cache_key_fields`, `pagination_style`) never consumed by any ecosystem package

### 2.2 Canonical ModuleAnnotations Definition

```
ModuleAnnotations {
  // ─── Behavioral semantics (consumed by execution engine + surface adapters) ───
  readonly:           bool = false
  destructive:        bool = false
  idempotent:         bool = false
  requires_approval:  bool = false
  open_world:         bool = true
  streaming:          bool = false

  // ─── Caching (consumed by cache middleware) ───
  cacheable:          bool = false
  cache_ttl:          int  = 0           // seconds, MUST be non-negative
  cache_key_fields:   list[string] | nil = nil

  // ─── Pagination (consumed by surface adapters) ───
  paginated:          bool = false
  pagination_style:   string = "cursor"  // recommended: "cursor", "offset", "page"; open for custom values

  // ─── Extension (consumed by ecosystem packages and user code) ───
  extra:              map[string, any] = {}
}
```

### 2.3 Cross-Language Alignment Changes

| SDK | Change | Reason |
|-----|--------|--------|
| **Python** | `pagination_style`: `Literal[...]` → `str` | Allow custom pagination strategies |
| **Python** | Add `extra: dict[str, Any] = field(default_factory=dict)` | Extension point |
| **Python** | `cache_key_fields`: accept `list` or `tuple` input, store as `tuple[str, ...] \| None` internally. `extra`: shallow-copy via `dict(extra)` in `__post_init__`. | Enforce immutability (frozen dataclass). Use `object.__setattr__` in `__post_init__` to bypass frozen restriction: convert `cache_key_fields` list→tuple, and copy `extra` to detach from caller's mutable dict. |
| **TypeScript** | All 5 optional fields (`cacheable?` etc.) → required with defaults. Add `createAnnotations(overrides?)` factory function to ease migration — callers only pass non-default values. | Align with Python/Rust — all fields always present |
| **TypeScript** | Add `readonly extra: Readonly<Record<string, unknown>>` | Extension point |
| **TypeScript** | `paginationStyle`: union type → `string` | Allow custom values |
| **Rust** | Add `pub extra: HashMap<String, serde_json::Value>` with `#[serde(default)]` | Extension point |
| **All** | `cache_ttl` type: non-negative integer | Python `int` (validate ≥0), TS `number` (validate ≥0), Rust `u64` (inherently ≥0) |

### 2.4 Extra Key Naming Convention

```
Pattern                   Owner                  Example
───────────────────────── ────────────────────── ──────────────────────────
{package}.{key}           ecosystem package      mcp.category, a2a.guidance, cli.approval_message
{app}.{key}               user application       myapp.cost_center, myapp.team
```

Rules:
- Extra keys SHOULD use `{namespace}.{key}` format to avoid collisions
- Unprefixed keys are allowed but SHOULD NOT be used by ecosystem packages
- No schema validation of extra values — consuming code validates as needed
- Commonly used extra keys MAY be promoted to core fields in future versions (read from core first, fallback to extra)

### 2.5 Canonical Wire Format (JSON serialization)

All three SDKs MUST serialize ModuleAnnotations to **snake_case** as the canonical wire format:

```json
{
  "readonly": false,
  "destructive": true,
  "idempotent": false,
  "requires_approval": true,
  "open_world": true,
  "streaming": false,
  "cacheable": false,
  "cache_ttl": 0,
  "cache_key_fields": null,
  "paginated": false,
  "pagination_style": "cursor",
  "extra": {
    "cli.approval_message": "This will send real emails",
    "mcp.category": "communication"
  }
}
```

TypeScript uses camelCase internally but MUST convert at serialization boundary:

```typescript
toJSON(): Record<string, unknown> {
  return {
    readonly: this.readonly,
    destructive: this.destructive,
    idempotent: this.idempotent,
    requires_approval: this.requiresApproval,
    open_world: this.openWorld,
    streaming: this.streaming,
    cacheable: this.cacheable,
    cache_ttl: this.cacheTtl,
    cache_key_fields: this.cacheKeyFields,
    paginated: this.paginated,
    pagination_style: this.paginationStyle,
    extra: this.extra,
  };
}

static fromJSON(data: Record<string, unknown>): ModuleAnnotations {
  // Collect unknown top-level keys into extra (forward compat).
  const KNOWN = new Set([
    'readonly', 'destructive', 'idempotent', 'requires_approval',
    'open_world', 'streaming', 'cacheable', 'cache_ttl',
    'cache_key_fields', 'paginated', 'pagination_style', 'extra',
  ]);
  const explicitExtra = (data['extra'] as Record<string, unknown>) ?? {};
  const overflow: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(data)) {
    if (!KNOWN.has(k)) overflow[k] = v;
  }

  return Object.freeze({
    readonly: (data['readonly'] as boolean) ?? false,
    destructive: (data['destructive'] as boolean) ?? false,
    idempotent: (data['idempotent'] as boolean) ?? false,
    requiresApproval: (data['requires_approval'] as boolean) ?? false,
    openWorld: (data['open_world'] as boolean) ?? true,
    streaming: (data['streaming'] as boolean) ?? false,
    cacheable: (data['cacheable'] as boolean) ?? false,
    cacheTtl: (data['cache_ttl'] as number) ?? 0,
    cacheKeyFields: (data['cache_key_fields'] as string[] | null) ?? null,
    paginated: (data['paginated'] as boolean) ?? false,
    paginationStyle: (data['pagination_style'] as string) ?? 'cursor',
    extra: { ...explicitExtra, ...overflow },
  });
}
```

### 2.6 DEFAULT_ANNOTATIONS Constant and Factory

Each SDK MUST export a frozen/immutable default annotations constant:

```python
# Python
DEFAULT_ANNOTATIONS = ModuleAnnotations()  # all defaults, frozen

# TypeScript
export const DEFAULT_ANNOTATIONS: ModuleAnnotations = Object.freeze({ ... });

/** Convenience factory — callers only pass non-default values. */
export function createAnnotations(
  overrides?: Partial<ModuleAnnotations>
): ModuleAnnotations {
  return Object.freeze({ ...DEFAULT_ANNOTATIONS, ...overrides });
}

// Usage:
const annot = createAnnotations({ destructive: true, extra: { 'mcp.icon': '🔥' } });
// Note: createAnnotations REPLACES extra entirely (spread semantics).
// To MERGE with existing extra, spread it yourself:
//   createAnnotations({ extra: { ...existing.extra, 'new.key': 'val' } })

# Rust
impl Default for ModuleAnnotations { fn default() -> Self { ... } }
```

### 2.7 Extra Deserialization Rule

When deserializing `ModuleAnnotations` from JSON (e.g., via `fromJSON()` or serde):
- All keys within `extra` MUST be preserved — do NOT filter or validate
- Unknown top-level keys (not in the 12 canonical fields) SHOULD be placed into `extra` automatically, so forward-compatible fields from newer SDKs are not lost

> **Python implementation note:** `@dataclass(frozen=True)` does not accept unknown
> kwargs in `__init__`. Deserialization MUST use a `@classmethod` factory:
>
> ```python
> _CANONICAL_FIELDS = {
>     "readonly", "destructive", "idempotent", "requires_approval",
>     "open_world", "streaming", "cacheable", "cache_ttl",
>     "cache_key_fields", "paginated", "pagination_style", "extra",
> }
>
> @classmethod
> def from_dict(cls, data: dict[str, Any]) -> "ModuleAnnotations":
>     known = {k: v for k, v in data.items() if k in cls._CANONICAL_FIELDS}
>     unknown = {k: v for k, v in data.items() if k not in cls._CANONICAL_FIELDS}
>     # Pop "extra" from known so it isn't passed twice to __init__.
>     # Then merge the explicit extra dict with any unknown overflow keys.
>     explicit_extra = known.pop("extra", {})
>     extra = {**explicit_extra, **unknown}
>     return cls(**known, extra=extra)
> ```
>
> **Rust:** Use `#[serde(flatten)]` on the `extra` field to automatically capture
> unknown keys during deserialization:
> ```rust
> #[serde(flatten)]
> pub extra: HashMap<String, serde_json::Value>,
> ```

---

## 3. ACL Conditions

### 3.1 Problem Statement

- Only 3 hardcoded condition keys: `identity_types`, `roles`, `max_call_depth`
- Unknown conditions silently ignored — user writes `ip_range` thinking it works, it doesn't
- No extension mechanism for custom conditions
- No compound conditions (OR/NOT) — can only express AND logic
- No async condition support — can't check external auth services
- TypeScript `removeRule` uses `JSON.stringify` comparison (order-sensitive)
- Rust treats empty `callers` list as "match all", Python/TS don't
- `audit_logger` injection inconsistent (constructor in Python/TS, setter in Rust)

### 3.2 ACL Condition Handler Protocol

The handler protocol supports both sync and async evaluation. Implementations choose one.

**Python** — two separate Protocols, union type for the registry:
```python
@runtime_checkable
class SyncACLConditionHandler(Protocol):
    """Sync condition handler."""
    def evaluate(self, value: Any, context: Context) -> bool: ...

@runtime_checkable
class AsyncACLConditionHandler(Protocol):
    """Async condition handler."""
    async def evaluate(self, value: Any, context: Context) -> bool: ...

# Registry accepts either
ACLConditionHandler = SyncACLConditionHandler | AsyncACLConditionHandler
```

**TypeScript** — single interface, return type is union:
```typescript
export interface ACLConditionHandler {
  evaluate(value: unknown, context: Context): boolean | Promise<boolean>;
}
```

**Rust** — async trait (sync handlers simply don't `.await` internally):
```rust
#[async_trait]
pub trait ACLConditionHandler: Send + Sync {
    async fn evaluate(
        &self,
        value: &serde_json::Value,
        ctx: &Context<serde_json::Value>,
    ) -> bool;
}
```

Design notes:
- **Python**: Two distinct Protocols avoid the "Protocol method can't return union of sync+async" problem. Dispatch uses `inspect.iscoroutinefunction(handler.evaluate)` to detect which variant.
- **TypeScript**: Single interface works because TS/JS naturally unifies sync and `Promise` return. Dispatch always uses `await` (no-op for plain boolean via `Promise.resolve()`).
- **Rust**: `async_trait` makes all handlers async. Sync handlers just return immediately without `.await`. No runtime overhead — the compiler optimizes trivial async.

### 3.3 Registration API

```python
# Class-level (global), callable before or after ACL.load()
ACL.register_condition(key: str, handler: ACLConditionHandler) -> None
```

```typescript
static registerCondition(key: string, handler: ACLConditionHandler): void
```

```rust
pub fn register_condition(key: impl Into<String>, handler: Box<dyn ACLConditionHandler>)
```

Rules:
- Registration is global (class-level), not per-instance
- Registering the same key twice replaces the previous handler (no error — allows override)
- Thread-safe: uses global lock/registry

### 3.4 Built-in Handlers

Five handlers auto-registered at module load time — three basic + two compound operators:

**Basic handlers:**

```python
class _IdentityTypesHandler:
    """Check context.identity.type is in the allowed list."""
    def evaluate(self, value: Any, context: Context) -> bool:
        if not isinstance(value, list) or context.identity is None:
            return False
        return context.identity.type in value

class _RolesHandler:
    """Check at least one role overlaps."""
    def evaluate(self, value: Any, context: Context) -> bool:
        if not isinstance(value, list) or context.identity is None:
            return False
        return bool(set(context.identity.roles) & set(value))

class _MaxCallDepthHandler:
    """Check call chain length does not exceed threshold."""
    def evaluate(self, value: Any, context: Context) -> bool:
        if not isinstance(value, int):
            return False
        return len(context.call_chain) <= value
```

**Compound operators:**

Compound handlers need to recursively evaluate sub-conditions. To avoid circular
dependency (`_OrHandler` → `ACL` → `_OrHandler`), compound handlers receive a
reference to the evaluation function at construction time:

```python
_EvalFn = Callable[[dict[str, Any], "Context"], bool]
# Note: ACL._evaluate_conditions is a classmethod. When accessed via
# ACL._evaluate_conditions (without calling), Python returns a bound method
# with cls already bound. The resulting callable signature matches _EvalFn.

class _OrHandler:
    """Evaluate sub-condition sets with OR logic.
    
    Value: list of condition dicts. Returns True if ANY sub-set passes.
    """
    def __init__(self, evaluate_fn: _EvalFn) -> None:
        self._evaluate = evaluate_fn

    def evaluate(self, value: Any, context: Context) -> bool:
        if not isinstance(value, list):
            return False
        for sub_conditions in value:
            if not isinstance(sub_conditions, dict):
                continue
            if self._evaluate(sub_conditions, context):
                return True
        return False

class _NotHandler:
    """Negate a condition set.
    
    Value: a single condition dict. Returns True if the sub-set FAILS.
    """
    def __init__(self, evaluate_fn: _EvalFn) -> None:
        self._evaluate = evaluate_fn

    def evaluate(self, value: Any, context: Context) -> bool:
        if not isinstance(value, dict):
            return False
        return not self._evaluate(value, context)
```

**Auto-registration:**

```python
# Compound handlers receive the evaluate function to break the circular reference.
ACL.register_condition("identity_types", _IdentityTypesHandler())
ACL.register_condition("roles", _RolesHandler())
ACL.register_condition("max_call_depth", _MaxCallDepthHandler())
ACL.register_condition("$or", _OrHandler(ACL._evaluate_conditions))
ACL.register_condition("$not", _NotHandler(ACL._evaluate_conditions))
```

> **Compound + async limitation:** `$or` and `$not` call the **sync**
> `_evaluate_conditions`, which means async sub-conditions inside `$or`/`$not`
> will fail-closed (with a warning). To support async sub-conditions in compound
> operators, use `async_check()` — its `_evaluate_conditions_async` variant is
> passed to async-aware compound handlers automatically. Implementations SHOULD
> register both sync and async versions of compound handlers:
>
> ```python
> ACL.register_condition("$or", _OrHandler(ACL._evaluate_conditions))
> # For async_check(), the dispatch will call _evaluate_conditions_async internally.
> ```

### 3.5 Compound Condition Examples

**Simple AND** (all conditions must pass — default top-level behavior):
```yaml
rules:
  - callers: ["*"]
    targets: ["admin.*"]
    effect: allow
    conditions:
      roles: ["admin"]
      identity_types: ["user"]
```

**OR** (admin role OR internal IP):
```yaml
  - callers: ["*"]
    targets: ["admin.*"]
    effect: allow
    conditions:
      $or:
        - roles: ["admin"]
        - ip_range: "10.0.0.0/8"   # requires custom handler registration
```

**NOT** (deny service accounts):
```yaml
  - callers: ["*"]
    targets: ["sensitive.*"]
    effect: deny
    conditions:
      $not:
        identity_types: ["service"]
```

**Nested compound** — (admin role) OR (internal IP AND service account):
```yaml
  - callers: ["*"]
    targets: ["internal.*"]
    effect: allow
    conditions:
      $or:
        - roles: ["admin"]          # sub-condition 1: just admin role
        - identity_types: ["service"]   # sub-condition 2: service AND ip_range
          ip_range: "10.0.0.0/8"        # (same dict = AND)
```
> In the nested example, `identity_types` and `ip_range` are keys in the **same dict**
> (the second element of the `$or` list), so they are evaluated with AND logic.

### 3.6 Condition Evaluation Logic

The evaluation function is extracted as a class method so compound handlers can recurse:

```python
class ACL:
    _condition_handlers: ClassVar[dict[str, ACLConditionHandler]] = {}

    @classmethod
    def _evaluate_conditions(
        cls, conditions: dict[str, Any], context: Context,
    ) -> bool:
        """Evaluate all conditions. ALL must pass (AND logic). Fail-closed on unknown.
        
        This is a classmethod so compound handlers ($or, $not) can recurse.
        """
        for key, value in conditions.items():
            handler = cls._condition_handlers.get(key)
            if handler is None:
                _logger.warning("Unknown ACL condition %r — treated as unsatisfied", key)
                return False  # fail-closed
            result = handler.evaluate(value, context)
            if inspect.isawaitable(result):
                # In sync context, cannot await. Close the coroutine to prevent
                # "RuntimeWarning: coroutine was never awaited", then fail-closed.
                result.close()
                _logger.warning("Async condition %r in sync context — treated as unsatisfied. Use async_check().", key)
                return False
            if not result:
                return False
        return True

    @classmethod
    async def _evaluate_conditions_async(
        cls, conditions: dict[str, Any], context: Context,
    ) -> bool:
        """Async variant. Awaits async handlers, calls sync handlers directly.

        Shares the same handler dispatch logic as _evaluate_conditions but
        can await coroutine results instead of failing closed.
        """
        for key, value in conditions.items():
            handler = cls._condition_handlers.get(key)
            if handler is None:
                _logger.warning("Unknown ACL condition %r — treated as unsatisfied", key)
                return False
            result = handler.evaluate(value, context)
            if inspect.isawaitable(result):
                result = await result
            if not result:
                return False
        return True

    # Implementation note: _evaluate_conditions and _evaluate_conditions_async
    # have similar structure. This is intentional — the sync variant MUST NOT
    # import or reference asyncio at the top level (for sync-only deployments).
    # Do NOT merge them into a single function with a "sync_or_async" flag.
```

Public API provides both sync and async check:

```python
class ACL:
    def check(self, caller_id: str | None, target_id: str, 
              context: Context | None = None) -> bool:
        """Sync ACL check. Async condition handlers are treated as unsatisfied.
        Returns True (allow) or False (deny). Raises only on internal error."""
        ...

    async def async_check(self, caller_id: str | None, target_id: str,
                          context: Context | None = None) -> bool:
        """Async ACL check. Supports both sync and async condition handlers.
        Returns True (allow) or False (deny). Raises only on internal error."""
        ...
```

```typescript
class ACL {
  check(callerId: string | null, targetId: string, context?: Context | null): boolean { ... }
  async asyncCheck(callerId: string | null, targetId: string, context?: Context | null): Promise<boolean> { ... }
  // Both return boolean. Throws only on internal error (not for deny decisions).
}
```

```rust
impl ACL {
    /// Sync check. Returns Result because Rust convention for fallible operations.
    /// The bool inside Ok indicates allow (true) or deny (false).
    pub fn check(&self, caller_id: Option<&str>, target_id: &str,
                 ctx: Option<&Context<serde_json::Value>>) -> Result<bool, ModuleError> { ... }

    pub async fn async_check(&self, caller_id: Option<&str>, target_id: &str,
                             ctx: Option<&Context<serde_json::Value>>) -> Result<bool, ModuleError> { ... }
}
```

> **Rust returns `Result<bool, ModuleError>`** while Python/TS return plain `bool`.
> This aligns with Rust's error handling conventions — `ModuleError` covers internal
> failures (e.g., poisoned lock), not "deny" decisions. A deny decision is `Ok(false)`,
> not `Err(...)`. Python/TS raise exceptions only for internal errors (not for deny).

Key behaviors:
- `check()` (sync): works with all sync handlers. Async handlers → warn + fail-closed
- `async_check()`: awaits async handlers, calls sync handlers directly. Recommended for production use
- **Evaluation at check() time, not load() time** — handlers may not be registered when YAML is loaded
- **Fail-closed** — unknown condition key → warn log + return False (rule doesn't match)
- **AND logic** at top level — all conditions must pass for the rule to match
- **$or / $not** — compound operators providing OR and NOT logic as registered handlers
- **Type validation in handler** — each handler validates its own value type

### 3.7 Cross-Language Alignment Changes

| SDK | Change | Reason |
|-----|--------|--------|
| **All** | Replace `_check_conditions` if/else chain with handler dispatch | Extensibility |
| **All** | Add `ACL.register_condition()` class method | Extension API |
| **All** | Add `_evaluate_conditions` as classmethod for recursion | Compound operator support |
| **All** | Add 5 built-in handlers (3 basic + $or + $not) | Full condition logic |
| **All** | Add `async_check()` method | Async condition support |
| **All** | Unknown conditions: silent ignore → warn + fail-closed | Safety |
| **Rust** | Empty callers/targets → NOT match all (align with Python/TS) | Consistency |
| **Rust** | `audit_logger`: setter → constructor parameter | Align with Python/TS |
| **TypeScript** | `removeRule`: JSON.stringify → element-wise array comparison | Consistency with Python/Rust |

### 3.8 Custom Handler Example

```python
import ipaddress

class IpRangeHandler:
    """Check client IP is within allowed network range."""
    def evaluate(self, value: Any, context: Context) -> bool:
        if not isinstance(value, str):
            return False
        client_ip = context.data.get("client_ip")
        if client_ip is None:
            return False
        try:
            return ipaddress.ip_address(client_ip) in ipaddress.ip_network(value)
        except ValueError:
            return False

class ExternalAuthHandler:
    """Check permission via external auth service (async)."""
    async def evaluate(self, value: Any, context: Context) -> bool:
        if context.identity is None:
            return False
        # Async call to external service
        return await auth_client.check_permission(
            user_id=context.identity.id,
            permission=value,
        )

# Register
ACL.register_condition("ip_range", IpRangeHandler())
ACL.register_condition("external_permission", ExternalAuthHandler())

# Use in YAML
# conditions:
#   ip_range: "10.0.0.0/8"
#   external_permission: "admin:write"
```

---

## 4. Implementation Plan

### Phase 1: Protocol Spec Update (apcore)

Update protocol-spec.md:
- §5 Context: canonical field list, ContextKey, data naming convention, serialization rules, `_context_version`
- §4 Module: ModuleAnnotations with `extra`, `pagination_style` as string, wire format
- §6 ACL: condition handler protocol (sync+async), registration API, compound operators ($or, $not), fail-closed behavior

### Phase 2: Context (all 3 SDKs)

| Step | Python | TypeScript | Rust |
|------|--------|-----------|------|
| 2.1 | Add `ContextKey` class | Add `ContextKey` class | Add `ContextKey` struct |
| 2.2 | Add `scoped()` method for per-module keys | Same | Same |
| 2.3 | Define built-in keys in `context_keys.py` | Define in `context-keys.ts` | Define in `context_keys.rs` |
| 2.4 | Fix data key naming (`_metrics_starts` etc.) — do BEFORE key migration | Same | Same |
| 2.5 | Migrate middleware to use typed keys (depends on 2.4) | Same | Same |
| 2.6 | Add `_context_version` to serialization | Same | Same |
| 2.7 | — | Add `globalDeadline` field | Remove `created_at`, `parent_trace_id`, `trace_context` |
| 2.8 | — | — | Make Identity fields immutable |
| 2.9 | Export `ContextKey` + built-in keys | Same | Same |
| 2.10 | Tests | Tests | Tests |

### Phase 3: Annotations (all 3 SDKs)

| Step | Python | TypeScript | Rust |
|------|--------|-----------|------|
| 3.1 | Add `extra: dict` to ModuleAnnotations | Add `extra: Record` | Add `extra: HashMap` |
| 3.2 | `pagination_style` → `str` | → `string` (remove union) | Already `String` |
| 3.3 | `cache_key_fields` → `tuple[str, ...] \| None` via `__post_init__` + `object.__setattr__`. Also copy `extra` dict in `__post_init__`. | Already `readonly string[]` | Already `Vec<String>` |
| 3.4 | — | 5 optional fields → required with defaults | — |
| 3.5 | — | Add `toJSON()` / `fromJSON()` with snake_case | — |
| 3.6 | Update apcore-cli to use `extra["cli.approval_message"]` | Same | Same |
| 3.7 | Update apcore-mcp to read from `extra` where applicable | Same | Same |
| 3.8 | Tests | Tests | Tests |

### Phase 4: ACL Conditions (all 3 SDKs)

| Step | Python | TypeScript | Rust |
|------|--------|-----------|------|
| 4.1 | Add `ACLConditionHandler` protocol (sync+async return) | Add interface (boolean \| Promise) | Add async trait |
| 4.2 | Add `_condition_handlers` registry + `register_condition()` | Same | Same (with RwLock) |
| 4.3 | Implement 3 basic handlers | Same | Same |
| 4.4 | Implement `$or` and `$not` compound handlers | Same | Same |
| 4.5 | Extract `_evaluate_conditions` as classmethod | Same | Same |
| 4.6 | Replace if/else in `_check_conditions` with handler dispatch | Same | Same |
| 4.7 | Add `async_check()` method | Same | Same |
| 4.8 | Add warn + fail-closed for unknown conditions | Same | Same |
| 4.9 | — | Fix `removeRule` comparison | Fix empty callers matching |
| 4.10 | — | — | Move `audit_logger` to constructor |
| 4.11 | Tests (basic + compound + async + fail-closed) | Same | Same |

### Phase 5: Full Test Suites

Run all tests across all repos to verify zero regressions.

---

## 5. Breaking Change Assessment

| Change | Breaking? | Migration |
|--------|-----------|-----------|
| **Context** | | |
| ContextKey (new type) | No | Pure addition |
| Fix data key naming | **Yes** (internal) | Only affects apcore's own middleware, not user code |
| Add `_context_version` to serialization | No | Old code ignores unknown fields |
| TS add `globalDeadline` | No | New field with null default |
| Rust remove 3 fields | **Yes** (Rust only) | Code using these fields must migrate to `data` |
| Rust `global_deadline` Instant→f64 | **Yes** (Rust only) | No external users — safe to change |
| Rust Identity immutable | **Yes** (Rust only) | Replace field writes with constructor |
| **Annotations** | | |
| Add `extra` field | No | New field with empty dict default |
| `pagination_style` string | **Micro** | Removes compile-time check in Python; runtime unchanged |
| TS optional → required | **Yes** (TS only) | Code that constructs `ModuleAnnotations` without all fields will fail. Mitigated by `createAnnotations()` factory that accepts `Partial<ModuleAnnotations>`. |
| **ACL** | | |
| register_condition API | No | Pure addition, built-in behavior unchanged |
| $or / $not handlers | No | Pure addition, new YAML syntax |
| async_check() method | No | New method, existing check() unchanged |
| Fail-closed for unknown conditions | **Behavioral** | Rules with unregistered conditions now fail instead of pass |
| Rust empty callers fix | **Behavioral** (Rust only) | Empty callers no longer matches everything |
| TS removeRule fix | **Behavioral** (TS only) | Order-independent comparison |
| Rust audit_logger to constructor | **Yes** (Rust only) | Constructor signature change |

### Risk Assessment

- **Python**: No breaking changes for user code
- **TypeScript**: Breaking on Annotations (optional → required, mitigated by `createAnnotations()` factory), behavioral on removeRule
- **Rust**: Most breaking changes (3 Context fields removed, Identity immutable, audit_logger move)
- **All**: ACL fail-closed is behavioral — unknown conditions now block instead of pass. This is **safer** but could affect rules with unregistered custom conditions. Mitigation: clearly document in migration guide.

---

## 6. What This Design Does NOT Do (by decision)

| Not doing | Why | Future path |
|-----------|-----|-------------|
| Annotations schema validation for `extra` | Consumer's responsibility — framework stays unopinionated about extension content | Each package validates what it reads |
| Distributed Context synchronization | Out of apcore scope — Context is per-process | Belongs to apflow |
| Context.data thread-safety in Python/TS | GIL / single-threaded event loop mitigates | Document as known limitation |
| ACL policy language (Rego/CEL) | Too complex for the 95% use case; handler registration covers power users | External OPA integration as ecosystem package |
