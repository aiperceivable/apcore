---
description: "Incrementally adopt apcore in apps with existing request/correlation IDs using the dual-ID model: W3C trace_id plus preserved x-correlation-id, wired via ContextFactory at the HTTP boundary."
---

# Integrating apcore into Existing Projects

> **Audience:** Teams adopting apcore incrementally in an application that already has its own request-ID, correlation-ID, or tracing system.

Existing web applications almost always already emit some form of request identifier — an `X-Request-ID` header, a Sentry transaction ID, an AWS X-Ray trace segment, or a custom `req_*` string logged on every line. When you adopt apcore, you do **not** need to throw those away. apcore uses a dual-ID model:

| Field | Owner | Format | Role |
|---|---|---|---|
| `context.trace_id` | apcore framework | 32-char lowercase hex (W3C Trace Context) | Distributed tracing, span correlation, observability backends |
| `context.data["x-correlation-id"]` | Your project | Any string, preserved verbatim | Response headers, business logs, legacy correlation |

Both travel together down the call chain. Your existing dashboards, log queries, and Sentry tags continue to work; apcore's middleware, tracing, and ACL engine use the canonical `trace_id` they require.

## The integration pattern

Every integration does the same three things at the HTTP boundary:

1. Parse the W3C `traceparent` header if present — this becomes `trace_id`.
2. Read the project's existing correlation header (`X-Request-ID`, `X-Correlation-ID`, etc.) — this goes into `context.data["x-correlation-id"]` unchanged.
3. Attach the caller's identity from whatever auth system is already in place.

apcore provides the `ContextFactory` Protocol/interface for exactly this purpose.

## Framework examples

=== "Python"

    **Django** (`django-apcore` integration):

    ```python
    from apcore import Context, Identity, ContextFactory
    from apcore.trace_context import parse_traceparent

    class DjangoContextFactory:
        def create_context(self, request) -> Context:
            # 1. W3C trace parent (may be None, or invalid — Context.create handles both)
            tp = parse_traceparent(request.META.get("HTTP_TRACEPARENT"))

            # 2. Project's existing correlation ID — accept any format verbatim
            correlation_id = (
                request.META.get("HTTP_X_REQUEST_ID")
                or request.META.get("HTTP_X_CORRELATION_ID")
                or ""
            )

            # 3. Identity from Django auth
            identity = None
            if request.user.is_authenticated:
                identity = Identity(
                    id=str(request.user.id),
                    type="user",
                    roles=tuple(request.user.groups.values_list("name", flat=True)),
                )

            ctx = Context.create(trace_parent=tp, identity=identity)
            if correlation_id:
                ctx.data["x-correlation-id"] = correlation_id
            return ctx
    ```

    **Flask / FastAPI** follow the same pattern — read `request.headers["traceparent"]`, `request.headers.get("X-Request-ID")`, then call `Context.create()`.

=== "TypeScript"

    **Express** (`express-apcore` or raw middleware):

    ```typescript
    import { Context, Identity, ContextFactory } from "apcore";
    import { parseTraceparent } from "apcore/trace-context";
    import type { Request } from "express";

    export class ExpressContextFactory implements ContextFactory {
      createContext(request: Request): Context {
        // 1. W3C trace parent
        const tp = parseTraceparent(request.get("traceparent") ?? null);

        // 2. Project's existing correlation ID
        const correlationId =
          request.get("X-Request-ID") ?? request.get("X-Correlation-ID") ?? "";

        // 3. Identity from your auth middleware
        const identity = request.user
          ? new Identity({
              id: String(request.user.id),
              type: "user",
              roles: request.user.roles,
            })
          : null;

        const ctx = Context.create({ traceParent: tp, identity });
        if (correlationId) {
          ctx.data["x-correlation-id"] = correlationId;
        }
        return ctx;
      }
    }
    ```

    **NestJS** integrations provide the same factory via the `@Req()` decorator — the body is identical.

=== "Rust"

    **Actix-web** (`actix-apcore` integration):

    ```rust
    use apcore::{Context, Identity, ContextFactory, TraceParent};
    use actix_web::HttpRequest;

    pub struct ActixContextFactory;

    impl ContextFactory<HttpRequest> for ActixContextFactory {
        fn create_context(&self, req: &HttpRequest) -> Context {
            // 1. W3C trace parent
            let trace_parent = req
                .headers()
                .get("traceparent")
                .and_then(|v| v.to_str().ok())
                .and_then(TraceParent::parse);

            // 2. Project's existing correlation ID
            let correlation_id = req
                .headers()
                .get("x-request-id")
                .or_else(|| req.headers().get("x-correlation-id"))
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();

            // 3. Identity from auth extractor (example)
            let identity = req.extensions().get::<AuthUser>().map(|u| {
                Identity::new(u.id.to_string(), "user", u.roles.clone())
            });

            let mut ctx = Context::builder()
                .trace_parent(trace_parent)
                .identity(identity)
                .build();
            if !correlation_id.is_empty() {
                ctx.data_mut().insert(
                    "x-correlation-id".into(),
                    correlation_id.into(),
                );
            }
            ctx
        }
    }
    ```

## What `trace_parent` accepts

apcore's `Context.create(trace_parent=...)` is strict on format and safe on failure:

| Input | Outcome |
|---|---|
| `4bf92f3577b34da6a3ce929d0e0e4736` (32-char lowercase hex, not all-zero, not all-f) | **accepted as-is** |
| Dashed UUID, uppercase hex, wrong length, non-hex characters, all-zero, all-f, empty string | **regenerated** + WARN logged |

The SDK **never** raises on bad input — it logs a warning and generates a fresh `trace_id` so the request still proceeds. This is intentional: a malformed inbound header must not take down production traffic.

### Why strict, not lenient?

`Context.create` deliberately does **not** auto-strip dashes from UUIDs or auto-lowercase hex. If the inbound `traceparent` header is malformed, that is a bug in either the upstream emitter or your header parser — silently normalizing would hide it. The [W3C Trace Context spec](https://www.w3.org/TR/trace-context/#trace-id) mandates 32-char lowercase hex on the wire; well-behaved upstream services already produce this format.

### If your legacy ID is a dashed UUID and you want to reuse it as `trace_id`

Do the normalization yourself in your `ContextFactory` — it's one line:

=== "Python"

    ```python
    legacy_id = request.headers.get("X-Legacy-Trace-ID", "")
    normalized = legacy_id.replace("-", "").lower()
    tp = TraceParent(trace_id=normalized, ...) if len(normalized) == 32 else None
    ctx = Context.create(trace_parent=tp, identity=identity)
    ```

=== "TypeScript"

    ```typescript
    const legacyId = request.get("X-Legacy-Trace-ID") ?? "";
    const normalized = legacyId.replaceAll("-", "").toLowerCase();
    const tp = normalized.length === 32 ? { traceId: normalized, ... } : null;
    const ctx = Context.create({ traceParent: tp, identity });
    ```

=== "Rust"

    ```rust
    let legacy_id = req.headers().get("x-legacy-trace-id")
        .and_then(|v| v.to_str().ok()).unwrap_or("");
    let normalized: String = legacy_id.chars()
        .filter(|c| *c != '-').map(|c| c.to_ascii_lowercase()).collect();
    let tp = (normalized.len() == 32).then(|| TraceParent::new(&normalized, ...));
    let ctx = Context::builder().trace_parent(tp).identity(identity).build();
    ```

This keeps Context.create's contract strict and puts the format decision where it belongs — at the HTTP boundary where you already know whether your legacy ID system is trustworthy.

## What goes where in logs

Once the factory is wired up, both IDs are available everywhere:

=== "Python"

    ```python
    @module(id="order.create")
    def create_order(ctx: Context, payload: dict) -> dict:
        ctx.logger.info(
            "creating order",
            extra={
                "trace_id": ctx.trace_id,                              # 32-hex, for tracing
                "correlation_id": ctx.data.get("x-correlation-id"),   # original, for business logs
            },
        )
        # ...
    ```

=== "TypeScript"

    ```typescript
    export const createOrder = module({ id: "order.create" }, (ctx, payload) => {
      ctx.logger.info("creating order", {
        trace_id: ctx.traceId,
        correlation_id: ctx.data["x-correlation-id"],
      });
      // ...
    });
    ```

=== "Rust"

    ```rust
    use apcore::{APCore, Context};
    use apcore::errors::ModuleError;
    use serde_json::{json, Value};

    let mut client = APCore::new();
    client.module(
        "order.create",
        "Create a new order",
        json!({ "type": "object" }),
        json!({ "type": "object" }),
        None,                // documentation
        vec![],              // tags
        None,                // version
        None,                // metadata
        vec![],              // examples
        None,                // display
        |inputs: Value, ctx: &Context<Value>| {
            Box::pin(async move {
                let correlation_id = ctx
                    .data
                    .read()
                    .get("x-correlation-id")
                    .cloned()
                    .unwrap_or(Value::Null);
                tracing::info!(
                    trace_id = %ctx.trace_id,
                    ?correlation_id,
                    "creating order"
                );
                let _ = inputs;
                Ok::<Value, ModuleError>(json!({ "ok": true }))
            })
        },
    )?;
    ```

Response headers should echo both:

```
traceparent: 00-<trace_id>-<span_id>-01
X-Request-ID: <whatever the caller sent, preserved verbatim>
```

## Migration checklist

Adopting apcore in an existing project typically takes one PR:

- [ ] Implement `ContextFactory` for your framework (10–30 lines — see examples above).
- [ ] Wire the factory into your request middleware so every inbound request produces a `Context`.
- [ ] Keep your existing log format — just add the `trace_id` field alongside your existing `correlation_id`.
- [ ] Verify your observability backend (Jaeger / Tempo / Honeycomb / Datadog) accepts the 32-hex `trace_id` — all W3C-compliant backends do by default.
- [ ] Leave your existing `X-Request-ID` propagation, Sentry transaction IDs, and legacy log fields **unchanged**.

## FAQ

**Q: Can't we just use our existing request ID as `trace_id`?**

You could, but it breaks distributed tracing backends (Jaeger/Tempo/Honeycomb/Datadog) that expect W3C-compliant 32-char hex. You also lose interop with any service in your fleet that already emits W3C `traceparent`. The dual-ID model gives you both without compromise.

**Q: Our existing IDs are UUIDs with dashes. Do we have to change them?**

No. Keep them as your correlation ID. apcore will generate a separate W3C-compliant `trace_id` for tracing. If you want to *reuse* the dashed UUID as `trace_id` instead of generating a new one, strip the dashes yourself in your `ContextFactory` before passing it as `trace_parent` — see the [normalization example above](#if-your-legacy-id-is-a-dashed-uuid-and-you-want-to-reuse-it-as-trace_id). `Context.create` itself does **not** auto-normalize; this is intentional so upstream parser bugs stay visible.

**Q: What about AWS X-Ray trace IDs (`1-<hex>-<hex>`)?**

These are not W3C-compatible. Store the X-Ray ID in `context.data["x-correlation-id"]` (or `context.data["x-amzn-trace-id"]` if you prefer the original header name) and let apcore generate a fresh `trace_id`. If you need X-Ray and apcore to share the same trace, use an OpenTelemetry bridge — see the [observability guide](../features/observability.md).

**Q: What if our SDK version still emits 36-char UUID trace IDs?**

Rust SDK versions before the trace-context-unification release emit 36-char UUIDs with dashes. Update to the aligned version before relying on W3C header interop. See the project CHANGELOG in the repository root for the exact version.
