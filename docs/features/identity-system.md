# Identity System

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../../PROTOCOL_SPEC.md) §5.7 Context Object (`identity` sub-schema).


## Overview

The Identity System provides a structured representation of the caller's identity that flows through the execution pipeline. Every module call can carry an `Identity` describing who (or what) initiated the request — whether a human user, a service account, or an AI agent. The identity is immutable, attached to the `Context`, and consumed by the ACL System for access control decisions.

## Requirements

- Provide an immutable `Identity` data structure with `id`, `type`, `roles`, and `attrs` fields.
- The `type` field **MUST** default to `"user"` and accept any string. Well-known types include `user`, `service`, `ai`, `system`, and `anonymous`.
- The `roles` field **MUST** be an immutable sequence (tuple/readonly array) of role name strings.
- The `attrs` field **MUST** be an immutable dictionary for arbitrary key-value metadata.
- Identity **MUST** be attachable to a `Context` and propagated to child contexts.
- Identity **MUST** integrate with the ACL System for identity-type-based and role-based access control decisions.
- Provide a `ContextFactory` protocol for web framework integrations to extract `Identity` from HTTP requests.

## Technical Design

### Identity

=== "Python"
    ```python
    from dataclasses import dataclass
    from typing import Any

    @dataclass(frozen=True)
    class Identity:
        id: str                          # Unique identifier
        type: str = "user"               # Identity type
        roles: tuple[str, ...] = ()      # Immutable role list
        attrs: dict[str, Any] = {}       # Additional attributes (frozen via dataclass)
    ```
=== "TypeScript"
    ```typescript
    interface Identity {
        readonly id: string;
        readonly type: string;               // Default: "user"
        readonly roles: readonly string[];
        readonly attrs: Readonly<Record<string, unknown>>;
    }

    function createIdentity(
        id: string,
        type?: string,      // Default: "user"
        roles?: string[],
        attrs?: Record<string, unknown>,
    ): Identity;
    // Returns a frozen Identity object
    ```
=== "Rust"
    ```rust
    use std::collections::HashMap;
    use apcore::Identity;

    // Fields are private; use getters to access
    let identity = Identity::new(
        "user-123".to_string(),
        "user".to_string(),
        vec!["admin".to_string()],
        HashMap::new(),
    );

    identity.id()             // -> &str
    identity.identity_type()  // -> &str  (default: "user")
    identity.roles()          // -> &[String]
    identity.attrs()          // -> &HashMap<String, Value>
    ```

### Well-Known Identity Types

| Type | Description | Typical Use |
|------|-------------|-------------|
| `user` | Human user (default) | Web app users, CLI operators |
| `service` | Service account | Microservices, background jobs |
| `ai` | AI agent or LLM | Autonomous agents, chatbots |
| `system` | Framework-internal | System modules, health checks |
| `anonymous` | No authenticated identity | Public endpoints, unauthenticated callers |

!!! note
    The `type` field is a free-form string. The values above are the well-known conventions surfaced in the JSON Schema `examples` (PROTOCOL_SPEC.md §5.7). Applications **MAY** define custom types — implementations do not validate the value against the conventions list.

### Equality and hashability

`Identity` is a value type. Equality is **structural** — two identities are equal iff `id`, `type`, `roles`, and `attrs` are equal as deep value comparisons. Hashability is **implementation-defined per language**:

- **Rust**: `Identity` derives `Hash` and `Eq`; safe to use as a `HashMap` key.
- **Python**: `Identity` is a frozen dataclass, but the `attrs: dict` field makes it not hashable by default; cross-language code SHOULD NOT rely on `hash(identity)` portability.
- **TypeScript**: object literal; equality and hashing are caller's responsibility (use a stable serialization or a structural-equality helper).

Resolved per `docs/spec/2026-05-decision-log.md` D-26.

### Usage with Context

=== "Python"
    ```python
    from apcore.context import Context, Identity

    # Create identity
    admin = Identity(
        id="admin@example.com",
        type="user",
        roles=("admin", "operator"),
        attrs={"department": "engineering"},
    )

    # Attach to context
    ctx = Context.create(identity=admin)
    print(ctx.identity.id)      # "admin@example.com"
    print(ctx.identity.roles)   # ("admin", "operator")

    # Identity propagates to child contexts
    child = ctx.child("target.module")
    assert child.identity is ctx.identity
    ```
=== "TypeScript"
    ```typescript
    import { Context, createIdentity } from "apcore-js";

    // Create identity
    const admin = createIdentity(
        "admin@example.com",
        "user",
        ["admin", "operator"],
        { department: "engineering" },
    );

    // Attach to context
    const ctx = Context.create(undefined, admin);
    console.log(ctx.identity?.id);    // "admin@example.com"
    console.log(ctx.identity?.roles); // ["admin", "operator"]

    // Identity propagates to child contexts
    const child = ctx.child("target.module");
    // child.identity === ctx.identity
    ```
=== "Rust"
    ```rust
    use apcore::context::{Context, Identity};
    use std::collections::HashMap;

    // Create identity
    let admin = Identity::new(
        "admin@example.com".to_string(),
        "user".to_string(),
        vec!["admin".to_string(), "operator".to_string()],
        HashMap::from([("department".to_string(), serde_json::json!("engineering"))]),
    );

    // Attach to context
    let ctx = Context::create(Some(admin));
    println!("{}", ctx.identity.as_ref().unwrap().id()); // "admin@example.com"

    // Identity propagates to child contexts
    let child = ctx.child("target.module");
    ```

### Integration with ACL

The ACL System uses the Identity for access control decisions. ACL rules can match against identity properties:

**Identity type conditions:**
```yaml
rules:
  - callers: ["*"]
    targets: ["admin.*"]
    effect: allow
    conditions:
      identity_types: ["user"]   # Only human users can call admin modules
```

**Role-based conditions:**
```yaml
rules:
  - callers: ["*"]
    targets: ["billing.*"]
    effect: allow
    conditions:
      roles: ["finance", "admin"]   # Requires one of these roles
```

**Special patterns:**
| Pattern | Matches |
|---------|---------|
| `@external` | Calls with no identity (`identity is None`) |
| `@system` | Calls where `identity.type == "system"` |

See [ACL System](./acl-system.md) for full condition syntax.

### ContextFactory Protocol

The `ContextFactory` protocol enables web framework integrations to extract `Identity` from incoming HTTP requests:

=== "Python"
    ```python
    from apcore.context import ContextFactory, Context, Identity
    from typing import Protocol, runtime_checkable

    @runtime_checkable
    class ContextFactory(Protocol):
        def create_context(self, request: Any) -> Context: ...

    # Example: Django integration
    class DjangoContextFactory:
        def create_context(self, request) -> Context:
            user = request.user
            identity = Identity(
                id=str(user.id),
                type="user",
                roles=tuple(user.groups.values_list("name", flat=True)),
                attrs={"email": user.email},
            )
            return Context.create(identity=identity)
    ```
=== "TypeScript"
    ```typescript
    import { Context, createIdentity } from "apcore-js";
    import type { ContextFactory } from "apcore-js";

    // Example: Express integration
    class ExpressContextFactory implements ContextFactory {
        createContext(request: any): Context {
            const user = request.user;
            const identity = createIdentity(
                user.id,
                "user",
                user.roles,
                { email: user.email },
            );
            return Context.create(undefined, identity);
        }
    }
    ```
=== "Rust"
    ```rust
    use apcore::context::{Context, ContextFactory, Identity};
    use std::collections::HashMap;

    struct AxumContextFactory;

    impl ContextFactory for AxumContextFactory {
        fn create_context(&self, request: &dyn std::any::Any) -> Context<serde_json::Value> {
            // Extract identity from framework-specific request type
            let identity = Identity::new(
                "extracted-user-id".to_string(),
                "user".to_string(),
                vec!["viewer".to_string()],
                HashMap::new(),
            );
            Context::create(Some(identity))
        }
    }
    ```

### Serialization

Identity is included when a Context is serialized (e.g., for distributed tracing across service boundaries):

```json
{
  "trace_id": "a1b2c3d4-...",
  "identity": {
    "id": "admin@example.com",
    "type": "user",
    "roles": ["admin", "operator"],
    "attrs": {"department": "engineering"}
  },
  "_context_version": 1
}
```

On deserialization, the Identity is reconstructed from the serialized form.

## Dependencies

- **Context** — Identity is a field on the Context object.
- **ACL System** — Consumes identity type and roles for access control decisions.
- **Observability** — Identity information (id, type) is included in trace spans and structured logs.

??? info "Python SDK reference"
    The following table is **not a protocol requirement** — it documents the Python SDK's source layout for implementers/users of `apcore-python`.

    **Source files:**

    | File | Purpose |
    |------|---------|
    | `src/apcore/context.py` | `Identity`, `Context`, `ContextFactory` |

## Testing Strategy

- **Immutability tests** verify that Identity fields cannot be modified after creation.
- **Default tests** verify that `type` defaults to `"user"` and `roles`/`attrs` default to empty.
- **Context propagation tests** verify that Identity propagates through `Context.child()`.
- **Serialization tests** verify round-trip serialization/deserialization of Identity within Context.
- **ACL integration tests** verify that identity type conditions and role conditions produce correct allow/deny decisions.
- **ContextFactory tests** verify that custom factories produce valid Context objects with correctly extracted Identity.

## Contract: ContextFactory.create_context

### Inputs
- `identity` (Identity, optional) — caller identity; defaults to `@external` when absent
- `caller_id` (str/string/&str, optional) — caller module ID for call-chain tracking
- `data` (dict/object/Value, optional) — initial context data payload

### Errors
- No errors raised (invalid identity fields are sanitized, not rejected)

### Returns
- On success: `Context` — initialized execution context with assigned trace ID and caller identity

### Properties
- async: false
- thread_safe: true
- pure: false (generates a new trace ID on each call; not idempotent)
- idempotent: false
