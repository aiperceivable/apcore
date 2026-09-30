---
description: "Incrementally adopt apcore in apps with existing request/correlation IDs using the dual-ID model: W3C trace_id plus preserved x-correlation-id, built into the Context at the HTTP boundary."
---

# Integrating apcore into Existing Projects

> **Audience:** Teams adopting apcore incrementally in an application that already has its own request-ID, correlation-ID, or tracing system.

Existing web applications almost always emit some request identifier already — an `X-Request-ID` header, a Sentry transaction ID, an AWS X-Ray trace segment, or a custom `req_*` string logged on every line. Adopting apcore does **not** mean throwing those away. apcore uses a dual-ID model:

| Field | Owner | Format | Role |
|---|---|---|---|
| `context.trace_id` | apcore | 32-char lowercase hex (W3C Trace Context) | Distributed tracing, span correlation, observability backends |
| `context.data["x-correlation-id"]` | Your project | Any string, preserved verbatim | Response headers, business logs, legacy correlation |

Both travel down the call chain together. Your dashboards, log queries and Sentry tags keep working; apcore's tracing and audit records use the canonical `trace_id`.

## The integration pattern

Every integration does the same three things at the HTTP boundary:

1. Parse the W3C `traceparent` header if present — it becomes `trace_id`.
2. Read the project's existing correlation header (`X-Request-ID`, `X-Correlation-ID`, …) and store it unchanged in `context.data["x-correlation-id"]`.
3. Attach the caller's identity from the auth system already in place.

apcore's `ContextFactory` interface (Python, TypeScript) names this step; in Rust the equivalent is a plain function in your web layer.

## Framework examples

=== "Python"

    **Django** — `request.headers` is case-insensitive, which is what `TraceContext.extract()` needs (`request.META` spells the header `HTTP_TRACEPARENT` and would not match):

    ```python
    from apcore import Context, Identity, TraceContext


    class DjangoContextFactory:
        def create_context(self, request) -> Context:
            # 1. W3C trace parent — None when the header is missing or malformed.
            trace_parent = TraceContext.extract(request.headers)

            # 2. The project's existing correlation ID, kept verbatim.
            correlation_id = request.headers.get("X-Request-ID") or request.headers.get("X-Correlation-ID")

            # 3. Identity from Django auth.
            identity = None
            if request.user.is_authenticated:
                identity = Identity(
                    id=str(request.user.id),
                    type="user",
                    roles=tuple(request.user.groups.values_list("name", flat=True)),
                )

            return Context.create(
                identity=identity,
                trace_parent=trace_parent,
                data={"x-correlation-id": correlation_id} if correlation_id else None,
            )
    ```

    Flask and FastAPI follow the same pattern with their own `request.headers`.

=== "TypeScript"

    **Express:**

    ```typescript
    import { Context, createIdentity, TraceContext } from 'apcore-js';
    import type { ContextFactory } from 'apcore-js';
    import type { Request } from 'express';

    // The shape your auth middleware attaches to the request.
    type AuthenticatedRequest = Request & { user?: { id: string; roles: string[] } };

    export class ExpressContextFactory implements ContextFactory {
      createContext(request: AuthenticatedRequest): Context {
        // 1. W3C trace parent — null when the header is missing or malformed.
        const traceParent = TraceContext.extract(request.headers);

        // 2. The project's existing correlation ID, kept verbatim.
        const correlationId = request.get('X-Request-ID') ?? request.get('X-Correlation-ID');

        // 3. Identity from your auth middleware.
        const identity = request.user ? createIdentity(request.user.id, 'user', request.user.roles) : null;

        return Context.create(
          identity,
          traceParent,
          null,
          correlationId ? { 'x-correlation-id': correlationId } : undefined,
        );
      }
    }
    ```

    NestJS and Fastify integrations build the same `Context` from their request objects.

=== "Rust"

    **Axum:**

    ```rust
    use apcore::{Context, Identity, TraceParent};
    use axum::http::HeaderMap;
    use serde_json::Value;
    use std::collections::HashMap;

    /// What your auth layer stores for an authenticated request.
    #[derive(Clone)]
    pub struct AuthUser {
        pub id: String,
        pub roles: Vec<String>,
    }

    pub fn context_from_request(headers: &HeaderMap, user: Option<&AuthUser>) -> Context<Value> {
        // 1. W3C trace parent — parse() returns Err for a malformed header; treat that as absent.
        let trace_parent = headers
            .get("traceparent")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| TraceParent::parse(s).ok());

        // 2. The project's existing correlation ID, kept verbatim.
        let mut data = HashMap::new();
        if let Some(correlation_id) = headers
            .get("x-request-id")
            .or_else(|| headers.get("x-correlation-id"))
            .and_then(|v| v.to_str().ok())
        {
            data.insert("x-correlation-id".to_string(), Value::String(correlation_id.to_string()));
        }

        // 3. Identity from your auth layer.
        let identity = user.map(|u| Identity::new(u.id.clone(), "user".to_string(), u.roles.clone(), HashMap::new()));

        // Context::create(identity, trace_parent, cancel_token, data, services, global_deadline)
        Context::create(identity, trace_parent, None, Some(data), Value::Null, None)
    }
    ```

    Call it from a handler that takes `HeaderMap` and pass the result to `client.call(module_id, inputs, Some(&ctx), None)`.

## What `trace_parent` accepts

`Context.create(trace_parent=...)` is strict on format and safe on failure:

| Input `trace_id` | Outcome |
|---|---|
| `4bf92f3577b34da6a3ce929d0e0e4736` (32-char lowercase hex, not all-zero, not all-f) | **accepted as-is** |
| Dashed UUID, uppercase hex, wrong length, non-hex characters, all-zero, all-f, empty string | **regenerated** and a warning logged |

The SDK never raises on bad input: it generates a fresh `trace_id` so the request still proceeds. A malformed inbound header must not take down production traffic.

### Why strict, not lenient?

`Context.create` deliberately does **not** strip dashes or lowercase hex. A malformed inbound `traceparent` is a bug in the upstream emitter or in your header parsing, and silently normalising it would hide that. The [W3C Trace Context spec](https://www.w3.org/TR/trace-context/#trace-id) requires 32-char lowercase hex on the wire, which well-behaved services already send.

### If your legacy ID is a dashed UUID and you want to reuse it as `trace_id`

Normalise it yourself at the boundary and build the trace parent from it:

=== "Python"

    ```python
    import secrets

    from apcore import TraceParent


    def trace_parent_from_legacy_id(legacy_id: str) -> TraceParent | None:
        normalized = legacy_id.replace("-", "").lower()
        if len(normalized) != 32:
            return None
        return TraceParent(version="00", trace_id=normalized, parent_id=secrets.token_hex(8), trace_flags="01")
    ```

=== "TypeScript"

    ```typescript
    import { randomBytes } from 'node:crypto';
    import type { TraceParent } from 'apcore-js';

    export function traceParentFromLegacyId(legacyId: string): TraceParent | null {
      const normalized = legacyId.replaceAll('-', '').toLowerCase();
      if (normalized.length !== 32) return null;
      return {
        version: '00',
        traceId: normalized,
        parentId: randomBytes(8).toString('hex'),
        traceFlags: '01',
        tracestate: [],
      };
    }
    ```

=== "Rust"

    ```rust
    use apcore::{TraceContext, TraceParent};

    pub fn trace_parent_from_legacy_id(legacy_id: &str) -> Option<TraceParent> {
        let normalized: String = legacy_id
            .chars()
            .filter(|c| *c != '-')
            .map(|c| c.to_ascii_lowercase())
            .collect();
        if normalized.len() != 32 {
            return None;
        }
        // Start from a fresh root (random parent_id), then substitute the trace id.
        let mut trace_parent = TraceContext::new_root().traceparent;
        trace_parent.trace_id = normalized;
        Some(trace_parent)
    }
    ```

Pass the result as `trace_parent` to `Context.create`. The format decision stays at the HTTP boundary, where you know whether the legacy ID system is trustworthy.

## What goes where in logs

Once the context is built this way, both IDs are available in every module. Log them through your application's own logger:

=== "Python"

    ```python
    import logging

    from apcore import APCore
    from apcore.context import Context

    logger = logging.getLogger("orders")
    client = APCore()


    @client.module(id="order.create", description="Create an order")
    def create_order(item: str, quantity: int, context: Context) -> dict:
        logger.info(
            "creating order",
            extra={
                "trace_id": context.trace_id,  # 32-hex, for tracing
                "correlation_id": context.data.get("x-correlation-id"),  # original, for business logs
            },
        )
        return {"order_id": "ord_1"}
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';
    import { APCore } from 'apcore-js';

    const client = new APCore();

    client.module({
      id: 'order.create',
      description: 'Create an order',
      inputSchema: Type.Object({ item: Type.String(), quantity: Type.Integer() }),
      outputSchema: Type.Object({ order_id: Type.String() }),
      execute: async (_inputs, context) => {
        console.info('creating order', {
          trace_id: context.traceId, // 32-hex, for tracing
          correlation_id: context.data['x-correlation-id'], // original, for business logs
        });
        return { order_id: 'ord_1' };
      },
    });
    ```

=== "Rust"

    ```rust
    // Cargo.toml: apcore, serde_json, tracing
    use apcore::{APCore, Context, ModuleError};
    use serde_json::{json, Value};

    fn build_client() -> Result<APCore, ModuleError> {
        let mut client = APCore::new();
        client.module(
            "order.create",
            "Create an order",
            json!({
                "type": "object",
                "properties": {"item": {"type": "string"}, "quantity": {"type": "integer"}},
                "required": ["item", "quantity"]
            }),
            json!({"type": "object", "properties": {"order_id": {"type": "string"}}}),
            None,   // documentation
            vec![], // tags
            None,   // version
            None,   // metadata
            vec![], // examples
            None,   // display
            |_inputs: Value, ctx: &Context<Value>| {
                Box::pin(async move {
                    let correlation_id = ctx.data.read().get("x-correlation-id").cloned();
                    tracing::info!(trace_id = %ctx.trace_id, ?correlation_id, "creating order");
                    Ok(json!({"order_id": "ord_1"}))
                })
            },
        )?;
        Ok(client)
    }
    ```

Response headers should echo both:

```text
traceparent: 00-<trace_id>-<span_id>-01
X-Request-ID: <whatever the caller sent, preserved verbatim>
```

## Migration checklist

Adopting apcore in an existing project typically takes one PR:

- [ ] Build the `Context` from the request at your HTTP boundary (10–30 lines — see the examples above).
- [ ] Call modules with that context so every inbound request carries its IDs.
- [ ] Keep your existing log format — add the `trace_id` field next to your existing correlation ID.
- [ ] Check that your observability backend (Jaeger, Tempo, Honeycomb, Datadog) accepts the 32-hex `trace_id` — W3C-compliant backends do.
- [ ] Leave your existing `X-Request-ID` propagation, Sentry transaction IDs and legacy log fields **unchanged**.

## FAQ

**Q: Can't we just use our existing request ID as `trace_id`?**

Only if it is already 32-char lowercase hex. Anything else breaks tracing backends that expect W3C IDs, and loses interop with services that already send `traceparent`. The dual-ID model gives you both.

**Q: Our existing IDs are UUIDs with dashes. Do we have to change them?**

No. Keep them as the correlation ID and let apcore generate a separate `trace_id`. To reuse the UUID as the `trace_id`, strip the dashes yourself — see the [normalization example above](#if-your-legacy-id-is-a-dashed-uuid-and-you-want-to-reuse-it-as-trace_id). `Context.create` does not normalise on its own, so upstream parser bugs stay visible.

**Q: What about AWS X-Ray trace IDs (`1-<hex>-<hex>`)?**

They are not W3C-compatible. Store the X-Ray ID in `context.data["x-correlation-id"]` (or under `x-amzn-trace-id`) and let apcore generate a fresh `trace_id`. To make X-Ray and apcore share one trace, bridge them with OpenTelemetry — see [features/observability.md](../features/observability.md).
