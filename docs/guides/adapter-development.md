# Adapter Development Guide

> Develop apcore adapters for third-party web frameworks.

!!! note "Cross-language applicability"
    This guide uses Python examples (Flask, FastAPI, Django). The adapter pattern is the same for TypeScript (Express, Fastify) and Rust (Axum, Actix) — implement `ContextFactory` to extract `Identity` from framework-specific requests, then map routes to module calls.

## 1. Adapter Positioning

The apcore core remains pure and **does not include** any web framework-specific implementations. Adapters are independent repository projects responsible for automatically mapping routes/endpoints of specific frameworks (Flask, FastAPI, Django, etc.) to apcore modules.

```
┌─────────────────────────────────────────────────────────────┐
│                  apcore (Core Framework)                     │
│  module() / External Binding / Registry / Executor           │
└─────────────────────────────────────────────────────────────┘
                           ↑ Built on core mechanisms
      ┌──────────┬──────────┬──────────┬──────────┐
      │          │          │          │          │
   tiptap-      flask-    django-    express-    ...
   apcore       apcore    apcore     apcore
   (separate)   (separate)  (separate)  (separate)
```

## 2. Adapter Responsibilities

Adapters should do the following:

1. **Scan** framework route/endpoint definitions
2. **Extract** route information (path, methods, parameters, return types)
3. **Generate** Binding YAML files or call `module()` to register
4. **Map** framework-specific types to JSON Schema

Adapters **should not**:

- Modify apcore core behavior
- Re-implement Registry/Executor/Schema validation
- Bind to specific AI protocols (MCP, A2A, etc.)

## 3. Naming Conventions

### Repository Naming

```
{framework}-apcore
```

Examples: `flask-apcore`, `django-apcore`, `express-apcore`

### Package Naming

```
pip install {framework}-apcore
npm install {framework}-apcore
```

## 4. Adapter Interface Reference

The core workflow of an adapter: scan → generate bindings → register

=== "Python"

    ```python
    # Recommended adapter interface
    from typing import Protocol, Any

    class EndpointInfo(dict):
        """Loose dict describing one scanned endpoint."""


    class FrameworkAdapter(Protocol):
        """Framework adapter base interface"""

        def scan(self, app: Any) -> list[EndpointInfo]:
            """
            Scan framework application and extract endpoint information.

            Args:
                app: Framework application instance (e.g., FastAPI app, Flask app)

            Returns:
                List of endpoint information.
            """
            ...

        def generate_bindings(self, endpoints: list[EndpointInfo]) -> dict:
            """
            Generate Binding YAML content from endpoint information.

            Args:
                endpoints: List of endpoint information.

            Returns:
                dict that can be written to a .binding.yaml file.
            """
            ...

        def register(self, app: Any, registry: Any | None = None) -> None:
            """
            Scan application and register modules directly via module().

            Args:
                app: Framework application instance.
                registry: apcore Registry instance (optional).
            """
            ...
    ```

=== "TypeScript"

    ```typescript
    // Recommended adapter interface
    import type { APCore, Registry } from 'apcore-js';

    export interface EndpointInfo {
      path: string;
      methods: string[];
      name: string;
      summary?: string;
      handler: (...args: unknown[]) => unknown;
    }

    export interface BindingsFile {
      bindings: Array<Record<string, unknown>>;
    }

    export interface FrameworkAdapter<App> {
      /**
       * Scan a framework application and extract endpoint information.
       */
      scan(app: App): EndpointInfo[];

      /**
       * Generate Binding YAML content from endpoint information.
       * The returned object can be serialized to a .binding.yaml file.
       */
      generateBindings(endpoints: EndpointInfo[]): BindingsFile;

      /**
       * Scan an application and register modules directly via APCore.module().
       */
      register(app: App, client: APCore): void;
    }
    ```

=== "Rust"

    ```rust
    // Recommended adapter interface
    use apcore::APCore;
    use apcore::errors::ModuleError;
    use serde::Serialize;
    use serde_json::Value;

    #[derive(Debug, Clone, Serialize)]
    pub struct EndpointInfo {
        pub path: String,
        pub methods: Vec<String>,
        pub name: String,
        pub summary: Option<String>,
    }

    #[derive(Debug, Serialize)]
    pub struct BindingsFile {
        pub bindings: Vec<Value>,
    }

    /// Framework adapter base interface.
    ///
    /// `App` is the concrete framework type (e.g., `axum::Router`,
    /// `actix_web::App`).
    pub trait FrameworkAdapter<App> {
        /// Scan a framework application and extract endpoint information.
        fn scan(&self, app: &App) -> Vec<EndpointInfo>;

        /// Generate Binding YAML content from endpoint information.
        /// The returned struct can be serialized to a .binding.yaml file.
        fn generate_bindings(&self, endpoints: &[EndpointInfo]) -> BindingsFile;

        /// Scan an application and register modules directly via `APCore::module()`.
        fn register(&self, app: &App, client: &mut APCore) -> Result<(), ModuleError>;
    }
    ```

## 5. Example: Minimal Web-Framework Adapter Implementation

The example below shows a minimal adapter for a popular web framework in each language: **FastAPI** for Python, **Express** for TypeScript, and **Axum** for Rust. The structure (scan → register → generate_bindings) is identical across the three.

=== "Python"

    ```python
    # apcore_fastapi/adapter.py
    from apcore import APCore
    from fastapi import FastAPI
    from fastapi.routing import APIRoute


    def scan_fastapi(app: FastAPI) -> list[dict]:
        """Scan FastAPI application and extract route information."""
        endpoints = []
        for route in app.routes:
            if isinstance(route, APIRoute):
                endpoints.append({
                    "path": route.path,
                    "methods": list(route.methods or []),
                    "name": route.name,
                    "endpoint": route.endpoint,
                    "summary": route.summary,
                    "tags": list(route.tags) if route.tags else [],
                })
        return endpoints


    def register_fastapi(app: FastAPI, client: APCore, prefix: str = "api") -> None:
        """
        Register FastAPI routes as apcore modules.

        Args:
            app: FastAPI application instance.
            client: apcore client used for registration.
            prefix: Module ID prefix (e.g., "api" -> "api.get_user").
        """
        for route in app.routes:
            if not isinstance(route, APIRoute):
                continue

            endpoint = route.endpoint
            route_name = route.name or endpoint.__name__
            module_id = f"{prefix}.{route_name}"

            # Use the @client.module decorator: it leverages FastAPI's
            # type annotations to auto-derive input/output schemas.
            client.module(
                id=module_id,
                description=route.summary or f"API endpoint: {route.path}",
                tags=list(route.tags) if route.tags else None,
            )(endpoint)


    def generate_bindings(app: FastAPI, prefix: str = "api") -> dict:
        """
        Generate Binding YAML content from a FastAPI application.

        Returns a dict that can be written to a .binding.yaml file.
        """
        bindings = []
        for route in app.routes:
            if not isinstance(route, APIRoute):
                continue

            endpoint = route.endpoint
            route_name = route.name or endpoint.__name__
            target = f"{endpoint.__module__}:{endpoint.__qualname__}"

            bindings.append({
                "module_id": f"{prefix}.{route_name}",
                "target": target,
                "description": route.summary or f"API endpoint: {route.path}",
                "auto_schema": True,
                "tags": list(route.tags) if route.tags else [],
                "metadata": {
                    "http_path": route.path,
                    "http_methods": list(route.methods or []),
                },
            })

        return {"bindings": bindings}
    ```

=== "TypeScript"

    ```typescript
    // express-apcore/adapter.ts
    import { Type, type TSchema } from '@sinclair/typebox';
    import type { APCore } from 'apcore-js';
    import type { Express, Request, Response, RequestHandler } from 'express';

    interface ExpressEndpoint {
      path: string;
      methods: string[];
      name: string;
      summary: string;
      handler: RequestHandler;
    }

    /**
     * Scan an Express application and extract route information from its
     * internal router stack.
     */
    export function scanExpress(app: Express): ExpressEndpoint[] {
      const endpoints: ExpressEndpoint[] = [];
      // express stores layers on app._router.stack
      const stack = (app as unknown as { _router?: { stack: unknown[] } })._router?.stack ?? [];

      for (const layer of stack as Array<Record<string, any>>) {
        if (!layer.route) continue;
        const route = layer.route;
        const methods = Object.keys(route.methods ?? {}).map((m) => m.toUpperCase());
        const handler: RequestHandler = route.stack[route.stack.length - 1].handle;
        const name: string = handler.name || `route_${route.path.replace(/\W+/g, '_')}`;

        endpoints.push({
          path: route.path,
          methods,
          name,
          summary: `${methods.join(',')} ${route.path}`,
          handler,
        });
      }
      return endpoints;
    }

    /**
     * Register all Express routes as apcore modules.
     *
     * `prefix` becomes the module-ID namespace, e.g. "api" -> "api.get_user".
     */
    export function registerExpress(
      app: Express,
      client: APCore,
      prefix = 'api',
    ): void {
      for (const ep of scanExpress(app)) {
        const moduleId = `${prefix}.${ep.name}`;

        // Express handlers are (req, res, next) — wrap them so apcore can
        // pass JSON inputs and receive a JSON output.
        const inputSchema: TSchema = Type.Object({
          params: Type.Optional(Type.Record(Type.String(), Type.Unknown())),
          query: Type.Optional(Type.Record(Type.String(), Type.Unknown())),
          body: Type.Optional(Type.Unknown()),
        });
        const outputSchema: TSchema = Type.Record(Type.String(), Type.Unknown());

        client.module({
          id: moduleId,
          description: ep.summary,
          tags: ['http', ...ep.methods.map((m) => m.toLowerCase())],
          inputSchema,
          outputSchema,
          execute: async (inputs) => {
            // Build mock req/res objects so the existing Express handler runs unchanged.
            let captured: unknown = {};
            const req = {
              params: (inputs.params as object) ?? {},
              query: (inputs.query as object) ?? {},
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
            return (typeof captured === 'object' && captured !== null
              ? (captured as Record<string, unknown>)
              : { result: captured });
          },
        });
      }
    }

    /**
     * Generate Binding YAML content from an Express application.
     *
     * The returned object can be passed to `js-yaml`'s `dump()` and written
     * to a `.binding.yaml` file.
     */
    export function generateBindings(
      app: Express,
      prefix = 'api',
    ): { bindings: Array<Record<string, unknown>> } {
      const bindings = scanExpress(app).map((ep) => ({
        module_id: `${prefix}.${ep.name}`,
        target: `./routes/${ep.name}#default`,
        description: ep.summary,
        auto_schema: true,
        tags: ['http', ...ep.methods.map((m) => m.toLowerCase())],
        metadata: {
          http_path: ep.path,
          http_methods: ep.methods,
        },
      }));
      return { bindings };
    }
    ```

=== "Rust"

    ```rust
    // axum-apcore/src/adapter.rs
    use apcore::APCore;
    use apcore::errors::ModuleError;
    use serde::Serialize;
    use serde_json::{json, Value};

    /// One scanned Axum route.
    #[derive(Debug, Clone, Serialize)]
    pub struct AxumEndpoint {
        pub path: String,
        pub methods: Vec<String>,
        pub name: String,
        pub summary: String,
    }

    /// Scan an Axum router and extract route information.
    ///
    /// Axum's public API does not expose the internal route table, so adapters
    /// typically build the endpoint list at the same time the router is built —
    /// usually via a small helper macro or builder. The function below accepts
    /// the already-collected endpoint list so it stays runtime-agnostic.
    pub fn scan_axum(endpoints: Vec<AxumEndpoint>) -> Vec<AxumEndpoint> {
        endpoints
    }

    /// Register every Axum endpoint as an apcore module.
    ///
    /// Each endpoint is wrapped in a `FunctionModule` that accepts a JSON
    /// payload `{params, query, body}` and returns a JSON object.
    pub fn register_axum(
        endpoints: &[AxumEndpoint],
        client: &mut APCore,
        prefix: &str,
    ) -> Result<(), ModuleError> {
        for ep in endpoints {
            let module_id = format!("{prefix}.{}", ep.name);
            let summary = ep.summary.clone();
            let methods = ep.methods.clone();
            let path = ep.path.clone();

            client.module(
                &module_id,
                &summary,
                json!({
                    "type": "object",
                    "properties": {
                        "params": {"type": "object"},
                        "query":  {"type": "object"},
                        "body":   {}
                    }
                }),
                json!({"type": "object"}),
                None,
                {
                    let mut tags = vec!["http".to_string()];
                    tags.extend(methods.iter().map(|m| m.to_lowercase()));
                    tags
                },
                None,
                None,
                vec![],
                None,
                {
                    let path = path.clone();
                    let methods = methods.clone();
                    move |inputs: Value, _ctx| {
                        let path = path.clone();
                        let methods = methods.clone();
                        Box::pin(async move {
                            // In a real adapter this would dispatch into the
                            // matched Axum handler. Here we just echo the
                            // request shape for illustration.
                            Ok(json!({
                                "path": path,
                                "methods": methods,
                                "received": inputs,
                            }))
                        })
                    }
                },
            )?;
        }
        Ok(())
    }

    /// Generate Binding YAML content from a list of Axum endpoints.
    ///
    /// The returned `Value` can be serialized with `serde_yaml` and written
    /// to a `.binding.yaml` file.
    pub fn generate_bindings(endpoints: &[AxumEndpoint], prefix: &str) -> Value {
        let bindings: Vec<Value> = endpoints
            .iter()
            .map(|ep| {
                json!({
                    "module_id":   format!("{prefix}.{}", ep.name),
                    "target":      format!("crate::routes::{}", ep.name),
                    "description": ep.summary,
                    "auto_schema": true,
                    "tags": {
                        let mut t = vec!["http".to_string()];
                        t.extend(ep.methods.iter().map(|m| m.to_lowercase()));
                        t
                    },
                    "metadata": {
                        "http_path": ep.path,
                        "http_methods": ep.methods,
                    }
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
    from apcore_fastapi import register_fastapi

    app = FastAPI()
    client = APCore()


    @app.get("/users/{user_id}")
    def get_user(user_id: int) -> dict:
        """Get user information."""
        return {"id": user_id, "name": "Alice"}


    @app.post("/emails/send")
    def send_email(to: str, subject: str, body: str) -> dict:
        """Send an email."""
        return {"success": True}


    # One call to register every FastAPI route as an apcore module.
    register_fastapi(app, client)

    # Now any apcore caller can invoke "api.get_user" / "api.send_email".
    print(client.call("api.get_user", {"user_id": 42}))
    ```

=== "TypeScript"

    ```typescript
    import express from 'express';
    import { APCore } from 'apcore-js';
    import { registerExpress } from 'express-apcore';

    const app = express();
    app.use(express.json());
    const client = new APCore();

    app.get('/users/:user_id', (req, res) => {
      // Expose this handler under the function name "getUser" for the adapter.
      res.json({ id: Number(req.params.user_id), name: 'Alice' });
    });

    app.post('/emails/send', (req, res) => {
      const { to, subject, body } = req.body ?? {};
      void to;
      void subject;
      void body;
      res.json({ success: true });
    });

    // One call to register every Express route as an apcore module.
    registerExpress(app, client);

    // Now any apcore caller can invoke "api.<route_name>".
    const out = await client.call('api.getUser', { params: { user_id: 42 } });
    console.log(out);
    ```

=== "Rust"

    ```rust
    use apcore::APCore;
    use axum::{routing::get, routing::post, Router};
    use axum_apcore::adapter::{register_axum, AxumEndpoint};
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

        // 2. Describe the endpoints for the adapter (kept in sync with the router).
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

        // 3. Register every endpoint as an apcore module in one call.
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

## 6. Interaction with apcore Core

Adapters interact with apcore core only through the following methods:

| Interaction Method | Description |
|---------|------|
| `module()` | Register functions as modules at runtime |
| Binding YAML | Generate binding files for framework to load |
| `Registry` API | Query and manage registered modules |

Adapters **should not** directly operate on apcore internal components (such as SchemaLoader, internal implementation of Executor).

## 7. Testing Recommendations

Adapters should include the following tests:

- Completeness of route scanning (whether endpoints are missed)
- Correctness of type mapping (framework types → JSON Schema)
- Validity of generated module IDs
- Validity of Binding YAML (can be validated through `binding.schema.json`)
- Integration tests with apcore Registry

## Next Steps

- [Creating Modules Guide](./creating-modules.md) - Learn about apcore module definition methods
- [Module Interface](../features/module-interface.md) - Module Protocol contract
- [PROTOCOL_SPEC §5.11](../spec/protocol-spec.md) - Functional module definition specification
- [PROTOCOL_SPEC §5.12](../spec/protocol-spec.md) - External Schema binding specification
