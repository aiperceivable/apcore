---
description: "How to build an apcore adapter for a web framework (FastAPI, Express, Axum): scan routes, register them as modules or emit binding files, and build the request Context."
---

# Adapter Development Guide

> Build an apcore adapter for a third-party web framework.

An adapter turns a framework's routes into apcore modules. This guide walks through writing **your own** adapter for FastAPI (Python), Express (TypeScript) and Axum (Rust). The code is illustrative: the published adapter packages (for example `fastapi-apcore`, `express-apcore`, `axum-apcore`) have their own APIs — consult their documentation rather than this page when you use them.

## 1. Adapter Positioning

The apcore core stays framework-agnostic and contains no web-framework code. Each adapter lives in its own repository and maps the routes/endpoints of one framework onto apcore modules.

```text
┌─────────────────────────────────────────────────────────────┐
│                  apcore (Core Framework)                     │
│  module() / External Binding / Registry / Executor           │
└─────────────────────────────────────────────────────────────┘
                           ↑ Built on core mechanisms
      ┌──────────┬──────────┬──────────┬──────────┐
      │          │          │          │          │
   flask-     django-   fastapi-   express-     ...
   apcore     apcore    apcore     apcore
```

## 2. Adapter Responsibilities

An adapter:

1. **Scans** the framework's route/endpoint definitions.
2. **Extracts** route information (path, methods, parameters, return types).
3. **Registers** each route as a module at runtime, or **generates** binding YAML files.
4. **Maps** framework types to JSON Schema.
5. **Builds the request `Context`** — trace parent, correlation ID and caller identity (section 6).

An adapter does not:

- Change apcore core behaviour.
- Re-implement the Registry, Executor or schema validation.
- Tie itself to an AI protocol (MCP, A2A, …) — those are separate layers on top of apcore.

## 3. Naming Conventions

Repository and package name: `{framework}-apcore`.

```bash
pip install {framework}-apcore
npm install {framework}-apcore
cargo add {framework}-apcore
```

Module IDs generated from routes must be valid Canonical IDs: lowercase `snake_case` segments separated by dots (for example `api.get_user`). Normalise route or handler names before using them as ID segments.

## 4. Adapter Interface

The core workflow is scan → register (or generate bindings). A suggested interface:

=== "Python"

    ```python
    from typing import Any, Protocol, TypedDict

    from apcore import APCore


    class EndpointInfo(TypedDict):
        path: str
        methods: list[str]
        name: str
        summary: str | None


    class FrameworkAdapter(Protocol):
        def scan(self, app: Any) -> list[EndpointInfo]:
            """Extract endpoint information from a framework application."""
            ...

        def register(self, app: Any, client: APCore) -> None:
            """Register every endpoint as an apcore module."""
            ...

        def generate_bindings(self, app: Any) -> dict[str, Any]:
            """Return the content of a .binding.yaml file for the endpoints."""
            ...
    ```

=== "TypeScript"

    ```typescript
    import type { APCore } from 'apcore-js';

    export interface EndpointInfo {
      path: string;
      methods: string[];
      name: string;
      summary?: string;
    }

    export interface FrameworkAdapter<App> {
      /** Extract endpoint information from a framework application. */
      scan(app: App): EndpointInfo[];

      /** Register every endpoint as an apcore module. */
      register(app: App, client: APCore): void;
    }
    ```

=== "Rust"

    ```rust
    use apcore::{APCore, ModuleError};
    use serde_json::Value;

    #[derive(Debug, Clone)]
    pub struct EndpointInfo {
        pub path: String,
        pub methods: Vec<String>,
        pub name: String,
        pub summary: Option<String>,
    }

    /// `App` is the framework's application or router type.
    pub trait FrameworkAdapter<App> {
        /// Extract endpoint information from a framework application.
        fn scan(&self, app: &App) -> Vec<EndpointInfo>;

        /// Register every endpoint as an apcore module.
        fn register(&self, app: &App, client: &mut APCore) -> Result<(), ModuleError>;

        /// Return the content of a .binding.yaml file for the endpoints.
        fn generate_bindings(&self, app: &App) -> Value;
    }
    ```

## 5. Example: A Minimal Adapter

One minimal adapter per language: **FastAPI** (Python), **Express** (TypeScript) and **Axum** (Rust). Each registers routes at runtime; the Python and Rust ones also emit binding files.

A binding `target` has the form `module_path:callable` ([protocol-spec §5.12.3](../spec/protocol-spec.md#5123-target-resolution-algorithm)):

- **Python** — an importable dotted module path and a function or `Class.method` in it: `myapp.routes:get_user`.
- **TypeScript** — an ESM specifier the host can `import()` and an export name: `@my-org/billing:refund`. Express handlers take `(req, res, next)`, not module inputs, so they cannot be binding targets; the Express adapter below registers at runtime only.
- **Rust** — an opaque key into the handler map you pass to the binding loader: `routes:get_user`.

=== "Python"

    ```python
    # my_fastapi_adapter.py — your adapter
    import re
    from typing import Any

    from fastapi import FastAPI
    from fastapi.routing import APIRoute

    from apcore import APCore


    def to_segment(name: str) -> str:
        """Turn a route name into a Canonical ID segment (lowercase snake_case)."""
        name = re.sub(r"(?<=[a-z0-9])([A-Z])", r"_\1", name).lower()
        name = re.sub(r"[^a-z0-9_]+", "_", name).strip("_")
        return name if name[:1].isalpha() else f"r_{name}"


    def register_fastapi(app: FastAPI, client: APCore, prefix: str = "api") -> None:
        """Register every FastAPI route as an apcore module, e.g. "api.get_user"."""
        for route in app.routes:
            if not isinstance(route, APIRoute):
                continue
            # client.module derives the input/output schemas from the endpoint's type hints.
            client.module(
                id=f"{prefix}.{to_segment(route.name)}",
                description=route.summary or f"API endpoint: {route.path}",
                tags=[str(tag) for tag in route.tags] or None,
            )(route.endpoint)


    def generate_bindings(app: FastAPI, prefix: str = "api") -> dict[str, Any]:
        """Content for a .binding.yaml file. Endpoints must be module-level functions of an importable module."""
        bindings = []
        for route in app.routes:
            if not isinstance(route, APIRoute):
                continue
            endpoint = route.endpoint
            bindings.append(
                {
                    "module_id": f"{prefix}.{to_segment(route.name)}",
                    "target": f"{endpoint.__module__}:{endpoint.__qualname__}",
                    "description": route.summary or f"API endpoint: {route.path}",
                    "auto_schema": True,
                    "tags": [str(tag) for tag in route.tags],
                    "metadata": {"http_path": route.path, "http_methods": sorted(route.methods or [])},
                }
            )
        return {"bindings": bindings}
    ```

=== "TypeScript"

    ```typescript
    // express-adapter.ts — your adapter
    import { Type } from '@sinclair/typebox';
    import type { APCore } from 'apcore-js';
    import type { Express, Request, RequestHandler, Response } from 'express';

    interface ExpressEndpoint {
      path: string;
      methods: string[];
      name: string;
      handler: RequestHandler;
    }

    /** Turn a handler name or path into a Canonical ID segment (lowercase snake_case). */
    export function toSegment(name: string): string {
      const snake = name
        .replace(/([a-z0-9])([A-Z])/g, '$1_$2')
        .toLowerCase()
        .replace(/[^a-z0-9_]+/g, '_')
        .replace(/^_+|_+$/g, '');
      return /^[a-z]/.test(snake) ? snake : `r_${snake}`;
    }

    /** Read routes from the router stack (an Express internal: app.router in v5, app._router in v4). */
    export function scanExpress(app: Express): ExpressEndpoint[] {
      const internal = app as unknown as { _router?: { stack: unknown[] }; router?: { stack: unknown[] } };
      const stack = (internal._router ?? internal.router)?.stack ?? [];
      const endpoints: ExpressEndpoint[] = [];
      for (const layer of stack as Array<{ route?: { path: string; methods: Record<string, boolean>; stack: Array<{ handle: RequestHandler }> } }>) {
        if (!layer.route) continue;
        const handler = layer.route.stack[layer.route.stack.length - 1].handle;
        endpoints.push({
          path: layer.route.path,
          methods: Object.keys(layer.route.methods).map((m) => m.toUpperCase()),
          name: toSegment(handler.name || layer.route.path),
          handler,
        });
      }
      return endpoints;
    }

    /** Register every Express route as an apcore module, e.g. "api.get_user". */
    export function registerExpress(app: Express, client: APCore, prefix = 'api'): void {
      for (const ep of scanExpress(app)) {
        client.module({
          id: `${prefix}.${ep.name}`,
          description: `${ep.methods.join(',')} ${ep.path}`,
          tags: ['http', ...ep.methods.map((m) => m.toLowerCase())],
          inputSchema: Type.Object({
            params: Type.Optional(Type.Record(Type.String(), Type.String())),
            query: Type.Optional(Type.Record(Type.String(), Type.Unknown())),
            body: Type.Optional(Type.Unknown()),
          }),
          outputSchema: Type.Record(Type.String(), Type.Unknown()),
          execute: async (inputs) => {
            // Run the existing handler against a minimal req/res pair and capture what it sends.
            let captured: unknown = {};
            const req = {
              params: inputs.params ?? {},
              query: inputs.query ?? {},
              body: inputs.body,
              method: ep.methods[0],
              path: ep.path,
            } as unknown as Request;
            const res = {
              status() {
                return this;
              },
              json(payload: unknown) {
                captured = payload;
                return this;
              },
              send(payload: unknown) {
                captured = payload;
                return this;
              },
            } as unknown as Response;
            await Promise.resolve(ep.handler(req, res, () => {}));
            return typeof captured === 'object' && captured !== null
              ? (captured as Record<string, unknown>)
              : { result: captured };
          },
        });
      }
    }
    ```

=== "Rust"

    ```rust
    // src/adapter.rs — your adapter
    use apcore::{APCore, Context, ModuleError};
    use serde_json::{json, Value};

    /// One Axum route. Axum does not expose its route table, so the adapter
    /// takes the list you build alongside the router.
    #[derive(Debug, Clone)]
    pub struct AxumEndpoint {
        pub path: String,
        pub methods: Vec<String>,
        pub name: String, // already a Canonical ID segment, e.g. "get_user"
        pub summary: String,
    }

    fn input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "params": {"type": "object"},
                "query":  {"type": "object"},
                "body":   {}
            }
        })
    }

    fn tags(ep: &AxumEndpoint) -> Vec<String> {
        let mut tags = vec!["http".to_string()];
        tags.extend(ep.methods.iter().map(|m| m.to_lowercase()));
        tags
    }

    /// Register every endpoint as an apcore module, e.g. "api.get_user".
    pub fn register_axum(endpoints: &[AxumEndpoint], client: &mut APCore, prefix: &str) -> Result<(), ModuleError> {
        for ep in endpoints {
            let path = ep.path.clone();
            client.module(
                &format!("{prefix}.{}", ep.name),
                &ep.summary,
                input_schema(),
                json!({"type": "object"}),
                None,     // documentation
                tags(ep), // tags
                None,     // version
                None,     // metadata
                vec![],   // examples
                None,     // display
                move |inputs: Value, _ctx: &Context<Value>| {
                    let path = path.clone();
                    Box::pin(async move {
                        // A real adapter dispatches into the matched Axum handler here.
                        Ok(json!({"path": path, "received": inputs}))
                    })
                },
            )?;
        }
        Ok(())
    }

    /// Content for a .binding.yaml file. Each `target` is a handler-map key
    /// ("routes:<name>"); load the file with a handler map that uses the same keys.
    pub fn generate_bindings(endpoints: &[AxumEndpoint], prefix: &str) -> Value {
        let bindings: Vec<Value> = endpoints
            .iter()
            .map(|ep| {
                json!({
                    "module_id": format!("{prefix}.{}", ep.name),
                    "target": format!("routes:{}", ep.name),
                    "description": ep.summary,
                    "input_schema": input_schema(),
                    "output_schema": {"type": "object"},
                    "tags": tags(ep),
                    "metadata": {"http_path": ep.path, "http_methods": ep.methods}
                })
            })
            .collect();
        json!({ "bindings": bindings })
    }
    ```

**Usage:**

=== "Python"

    ```python
    from fastapi import FastAPI

    from apcore import APCore
    from my_fastapi_adapter import register_fastapi

    app = FastAPI()
    client = APCore()


    @app.get("/users/{user_id}", summary="Get user information")
    def get_user(user_id: int) -> dict:
        return {"id": user_id, "name": "Alice"}


    @app.post("/emails/send", summary="Send an email")
    def send_email(to: str, subject: str, body: str) -> dict:
        return {"success": True}


    register_fastapi(app, client)

    # Any apcore caller can now invoke "api.get_user" / "api.send_email".
    print(client.call("api.get_user", {"user_id": 42}))  # {'id': 42, 'name': 'Alice'}
    ```

=== "TypeScript"

    ```typescript
    import express from 'express';
    import type { Request, Response } from 'express';
    import { APCore } from 'apcore-js';
    import { registerExpress } from './express-adapter.js';

    const app = express();
    app.use(express.json());
    const client = new APCore();

    // Named handlers give readable module IDs: getUser -> api.get_user.
    function getUser(req: Request, res: Response): void {
      res.json({ id: Number(req.params.user_id), name: 'Alice' });
    }

    function sendEmail(_req: Request, res: Response): void {
      res.json({ success: true });
    }

    app.get('/users/:user_id', getUser);
    app.post('/emails/send', sendEmail);

    registerExpress(app, client);

    // Any apcore caller can now invoke "api.get_user" / "api.send_email".
    console.log(await client.call('api.get_user', { params: { user_id: '42' } })); // { id: 42, name: 'Alice' }
    ```

=== "Rust"

    ```rust
    // src/main.rs — Cargo.toml: apcore, axum, serde_json, tokio
    mod adapter; // the adapter code above, in src/adapter.rs

    use adapter::{register_axum, AxumEndpoint};
    use apcore::APCore;
    use axum::{routing::get, routing::post, Router};
    use serde_json::json;

    async fn get_user() -> &'static str {
        "Alice"
    }

    async fn send_email() -> &'static str {
        "ok"
    }

    #[tokio::main]
    async fn main() -> Result<(), Box<dyn std::error::Error>> {
        // 1. Build the Axum router as usual.
        let _router: Router = Router::new()
            .route("/users/{user_id}", get(get_user))
            .route("/emails/send", post(send_email));

        // 2. Describe the same endpoints for the adapter.
        let endpoints = vec![
            AxumEndpoint {
                path: "/users/{user_id}".into(),
                methods: vec!["GET".into()],
                name: "get_user".into(),
                summary: "Get user information".into(),
            },
            AxumEndpoint {
                path: "/emails/send".into(),
                methods: vec!["POST".into()],
                name: "send_email".into(),
                summary: "Send an email".into(),
            },
        ];

        // 3. Register every endpoint as an apcore module.
        let mut client = APCore::new();
        register_axum(&endpoints, &mut client, "api")?;

        // 4. Any apcore caller can now invoke "api.get_user" / "api.send_email".
        let out = client
            .call("api.get_user", json!({"params": {"user_id": 42}}), None, None)
            .await?;
        println!("{out}");
        Ok(())
    }
    ```

## 6. Request Context

When the adapter also serves HTTP requests through apcore, build one `Context` per request: the W3C `traceparent` header becomes the trace parent, the project's existing correlation header goes into `context.data["x-correlation-id"]`, and the authenticated user becomes the `Identity`. Pass that context to `client.call()`. [Integrating Existing Projects](./integrating-existing-projects.md) has the Django, Express and Axum versions; [features/identity-system.md](../features/identity-system.md) covers `Identity` and `ContextFactory`.

## 7. Interaction with apcore Core

Adapters use only these parts of apcore:

| Interaction | Use |
|---------|------|
| `module()` / `register()` | Register handlers as modules at runtime |
| Binding YAML | Generate binding files for the binding loader |
| `Registry` API | Query registered modules |
| `Context.create` | Build the per-request context |

Adapters do not reach into apcore internals such as the schema loader or the executor's pipeline.

## 8. Testing Recommendations

Test at least:

- Route scanning is complete (no endpoint missed).
- Framework types map to the expected JSON Schema.
- Generated module IDs are valid Canonical IDs.
- Generated binding files validate against `schemas/binding.schema.json` and load with the SDK's binding loader.
- An end-to-end call through apcore reaches the framework handler.

## Next Steps

- [Creating Modules Guide](./creating-modules.md) — module definition styles
- [Module Interface](../features/module-interface.md) — the module contract
- [features/decorator-bindings.md](../features/decorator-bindings.md) — `module()` and binding files
- [protocol-spec §5.12](../spec/protocol-spec.md#512-external-schema-binding-external-schema-binding) — binding YAML format and target resolution
