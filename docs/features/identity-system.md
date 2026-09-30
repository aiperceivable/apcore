---
description: "Immutable Identity (id, type, roles, attrs) for the caller, attached to Context and propagated to children; consumed by ACL for type/role decisions; ContextFactory extracts it from requests."
---

# Identity System

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §5.7 Context Parameter Specification (`identity` sub-schema).


## Overview

The Identity System gives the caller a structured representation that flows through the execution pipeline. A module call can carry an `Identity` describing who — or what — initiated it: a human user, a service account, an AI agent. The identity is immutable, attached to the `Context`, inherited by child calls, and consumed by the ACL System for access-control decisions.

## Requirements

- Provide an immutable `Identity` with `id`, `type`, `roles`, and `attrs` fields.
- The `type` field **MUST** default to `"user"` and accept any string.
- The `roles` field **MUST** be an immutable sequence (tuple / frozen array / owned `Vec` behind a getter) of role names.
- The `attrs` field **MUST** be an immutable map of arbitrary key-value metadata.
- Identity **MUST** be attachable to a `Context` and propagated to child contexts.
- Identity **MUST** integrate with the ACL System for identity-type and role conditions.
- Provide a `ContextFactory` interface for integrations that build a `Context` from a runtime request.

## Technical Design

### Identity

=== "Python"
    ```python
    from dataclasses import dataclass, field
    from typing import Any

    @dataclass(frozen=True)
    class Identity:
        id: str                                             # Unique identifier
        type: str = "user"                                  # Identity type
        roles: tuple[str, ...] = ()                         # Stored as a tuple
        attrs: dict[str, Any] = field(default_factory=dict) # Copied on construction
    ```

    `roles` passed as a list is converted to a tuple, and `attrs` is copied, so mutating the caller's objects afterwards does not change the identity.
=== "TypeScript"
    ```typescript
    // Shape of the frozen Identity class. The package root exports it as a
    // type; construct instances with createIdentity().
    interface Identity {
        readonly id: string;
        readonly type: string;                              // Default: "user"
        readonly roles: readonly string[];                  // Frozen copy
        readonly attrs: Readonly<Record<string, unknown>>;  // Frozen copy
        getAttr<T = unknown>(key: string, defaultValue?: T): T | undefined;
    }

    declare function createIdentity(
        id: string,
        type?: string,                  // Default: "user"
        roles?: string[],
        attrs?: Record<string, unknown>,
    ): Identity;
    ```
=== "Rust"
    ```rust
    use apcore::Identity;
    use std::collections::HashMap;

    fn main() {
        // Fields are private; read them through getters.
        let identity = Identity::new(
            "user-123".to_string(),
            "user".to_string(),
            vec!["admin".to_string()],
            HashMap::new(),
        );

        let _id: &str = identity.id();
        let _kind: &str = identity.identity_type();
        let _roles: &[String] = identity.roles();
        let _attrs = identity.attrs(); // &HashMap<String, serde_json::Value>
    }
    ```

All three expose `get_attr(key)` (`getAttr` in TypeScript) for reading a single attribute.

### Well-Known Identity Types

| Type | Description | Typical Use |
|------|-------------|-------------|
| `user` | Human user (default) | Web app users, CLI operators |
| `service` | Service account | Microservices, background jobs |
| `agent` / `ai` | AI agent or LLM | Autonomous agents, chatbots |
| `api_key` | Caller authenticated by an API key | Programmatic clients |
| `system` | Framework-internal | System modules, health checks; matched by the `@system` ACL pattern |

The `type` field is a free-form string; the values above are the conventions listed in the §5.7 schema `examples`. Applications **MAY** define their own types — implementations do not validate the value. An unauthenticated caller is represented by **no identity** (`identity` is null), not by a special type.

### Equality and Hashability

`Identity` is a value type. Equality is **structural** — two identities are equal when `id`, `type`, `roles` and `attrs` are equal. Hashability differs per language (D-26):

- **Rust**: derives `PartialEq`/`Eq` and implements `Hash` (attributes are hashed in key order), so it can be used as a `HashMap` key.
- **Python**: a frozen dataclass whose `attrs` field is a `dict`, so `hash(identity)` raises `TypeError`; cross-language code SHOULD NOT rely on hashing identities.
- **TypeScript**: a frozen class instance; `===` compares references, so structural equality and hashing are the caller's responsibility (for example via a stable serialization).

### Usage with Context

=== "Python"
    ```python
    from apcore import Context, Identity

    admin = Identity(
        id="admin@example.com",
        type="user",
        roles=("admin", "operator"),
        attrs={"department": "engineering"},
    )

    # Attach to a context
    ctx = Context.create(identity=admin)
    print(ctx.identity.id)      # "admin@example.com"
    print(ctx.identity.roles)   # ("admin", "operator")

    # The identity propagates to child contexts
    child = ctx.child("target.module")
    assert child.identity is ctx.identity
    ```
=== "TypeScript"
    ```typescript
    import { Context, createIdentity } from "apcore-js";

    const admin = createIdentity(
        "admin@example.com",
        "user",
        ["admin", "operator"],
        { department: "engineering" },
    );

    // Attach to a context
    const ctx = Context.create(admin);
    console.log(ctx.identity?.id);    // "admin@example.com"
    console.log(ctx.identity?.roles); // ["admin", "operator"]

    // The identity propagates to child contexts
    const child = ctx.child("target.module");
    console.log(child.identity === ctx.identity); // true
    ```
=== "Rust"
    ```rust
    use apcore::{Context, Identity};
    use serde_json::{json, Value};
    use std::collections::HashMap;

    fn main() {
        let admin = Identity::new(
            "admin@example.com".to_string(),
            "user".to_string(),
            vec!["admin".to_string(), "operator".to_string()],
            HashMap::from([("department".to_string(), json!("engineering"))]),
        );

        // Attach to a context
        let ctx: Context<Value> = Context::create(Some(admin), None, None, None, Value::Null, None);
        println!("{}", ctx.identity.as_ref().map(|i| i.id()).unwrap_or_default()); // "admin@example.com"

        // The identity propagates to child contexts
        let child = ctx.child("target.module");
        assert_eq!(child.identity, ctx.identity);
    }
    ```

### Integration with ACL

The ACL evaluates rules against the caller and, through conditions, against the identity:

**Identity type conditions:**
```yaml
rules:
  - callers: ["*"]
    targets: ["admin.*"]
    effect: allow
    conditions:
      identity_types: ["user"]   # Only human users can call admin modules
```

**Role conditions:**
```yaml
rules:
  - callers: ["*"]
    targets: ["billing.*"]
    effect: allow
    conditions:
      roles: ["finance", "admin"]   # The identity must hold at least one of these roles
```

**Special caller patterns:**

| Pattern | Matches |
|---------|---------|
| `@external` | Calls with no `caller_id` — top-level calls from outside any module. It is the sentinel the ACL substitutes for a null `caller_id`, whether or not the call carries an identity. |
| `@system` | Calls whose `identity.type` is `"system"` |

See [ACL System](./acl-system.md) for the full condition syntax.

### ContextFactory

A `ContextFactory` turns a runtime-specific request (a Django `HttpRequest`, an Express `Request`, an Axum extractor) into a `Context`, extracting the identity on the way.

=== "Python"
    ```python
    from typing import Any

    from apcore import Context, Identity
    from apcore.context import ContextFactory


    class DjangoContextFactory:
        """Satisfies the ContextFactory protocol."""

        def create_context(self, request: Any) -> Context:
            user = request.user
            if not user.is_authenticated:
                return Context.create()  # no identity; the ACL decides
            identity = Identity(
                id=str(user.id),
                type="user",
                roles=tuple(user.groups.values_list("name", flat=True)),
                attrs={"email": user.email},
            )
            return Context.create(identity=identity)


    factory: ContextFactory = DjangoContextFactory()
    ```
=== "TypeScript"
    ```typescript
    import { Context, createIdentity } from "apcore-js";
    import type { ContextFactory } from "apcore-js";

    interface ExpressLikeRequest {
        user?: { id: string; roles: string[]; email: string };
    }

    export class ExpressContextFactory implements ContextFactory {
        createContext(request: unknown): Context {
            const user = (request as ExpressLikeRequest).user;
            if (user === undefined) {
                return Context.create(); // no identity; the ACL decides
            }
            const identity = createIdentity(user.id, "user", user.roles, { email: user.email });
            return Context.create(identity);
        }
    }
    ```
=== "Rust"
    ```rust
    use apcore::errors::ModuleError;
    use apcore::{Context, ContextFactory, Identity};
    use async_trait::async_trait;
    use serde_json::Value;
    use std::collections::HashMap;

    struct AxumRequest {
        user_id: Option<String>,
    }

    struct AxumContextFactory;

    #[async_trait]
    impl ContextFactory for AxumContextFactory {
        type Request = AxumRequest;

        async fn create_context(&self, request: AxumRequest) -> Result<Context<Value>, ModuleError> {
            // An unauthenticated request still yields a usable Context with no identity.
            let identity = request
                .user_id
                .map(|id| Identity::new(id, "user".to_string(), vec![], HashMap::new()));
            Ok(Context::create(identity, None, None, None, Value::Null, None))
        }
    }

    fn main() {}
    ```

### Serialization

The identity is included when a Context is serialized for cross-process transfer:

```json
{
  "_context_version": 1,
  "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736",
  "caller_id": null,
  "call_chain": [],
  "identity": {
    "id": "admin@example.com",
    "type": "user",
    "roles": ["admin", "operator"],
    "attrs": {"department": "engineering"}
  },
  "data": {}
}
```

Deserialization rebuilds the `Identity` from this object. See [Context Object §Serialization](./context-object.md#serialization).

## Dependencies

- **Context** — Identity is a field on the Context object.
- **ACL System** — Consumes the identity type and roles in rule conditions.

??? info "Python SDK reference"
    Not a protocol requirement — the Python SDK's source layout for users of `apcore-python`.

    | File | Purpose |
    |------|---------|
    | `src/apcore/context.py` | `Identity`, `Context`, `ContextFactory` |

## Testing Strategy

- **Immutability tests** verify that Identity fields cannot be modified after creation, including through the caller's original `roles` / `attrs` objects.
- **Default tests** verify that `type` defaults to `"user"` and `roles` / `attrs` default to empty.
- **Context propagation tests** verify that Identity propagates through `Context.child()`.
- **Serialization tests** verify round-trip serialization of Identity within Context.
- **ACL integration tests** verify identity-type and role conditions and the `@external` / `@system` patterns.
- **ContextFactory tests** verify that factories produce valid Contexts, including for unauthenticated requests.

## Contract: ContextFactory.create_context

### Inputs
- `request` (Any / unknown / associated type, required) — the runtime-specific request object. It is **opaque to apcore**: the implementation, not the protocol, knows how to read it. Rust expresses it as the trait's associated type `Request` (D-76).

### Errors
- No errors under normal operation. A request that cannot be authenticated is **not** an error: the factory returns a usable Context with no identity, and the ACL's `@external` rules and default-deny govern what it may reach. An implementation MUST NOT synthesize an `Identity` for it (D-103).
- An implementation whose extraction can genuinely fail (I/O during extraction, for example) MAY declare a fallible return — Rust returns `Result<Context<Value>, ModuleError>` — but MUST NOT use that channel to report "not authenticated".

### Returns
- A `Context` carrying a fresh `trace_id` and the extracted identity (or none).

### Properties
- async: false in Python and TypeScript; `async` in Rust (`#[async_trait]`), where request handling is inherently asynchronous. The parameter is the request in every SDK.
- thread_safe: true
- pure: false (generates a new trace ID on each call)
- idempotent: false

### Additional members

An implementation MAY expose further factory members — apcore-rust provides `create(identity, services)` and `create_child(parent, module_name)` with default implementations. They are additive and unconstrained by this contract; `create_context(request)` **MUST** be present so a factory written against the spec ports across SDKs.
