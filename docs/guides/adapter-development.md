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

```python
# Recommended adapter interface
class FrameworkAdapter:
    """Framework adapter base interface"""

    def scan(self, app) -> list[EndpointInfo]:
        """
        Scan framework application and extract endpoint information

        Args:
            app: Framework application instance (e.g., FastAPI app, Flask app)

        Returns:
            List of endpoint information
        """
        ...

    def generate_bindings(self, endpoints: list[EndpointInfo]) -> dict:
        """
        Generate Binding YAML content from endpoint information

        Args:
            endpoints: List of endpoint information

        Returns:
            dict that can be written to .binding.yaml file
        """
        ...

    def register(self, app, registry=None):
        """
        Scan application and register directly via module()

        Args:
            app: Framework application instance
            registry: apcore Registry instance (optional)
        """
        ...
```

## 5. Example: Minimal FastAPI Adapter Implementation

```python
# apcore_fastapi/adapter.py

from apcore import module
from fastapi import FastAPI
from fastapi.routing import APIRoute
import inspect


def scan_fastapi(app: FastAPI) -> list[dict]:
    """Scan FastAPI application and extract route information"""
    endpoints = []
    for route in app.routes:
        if isinstance(route, APIRoute):
            endpoints.append({
                "path": route.path,
                "methods": route.methods,
                "name": route.name,
                "endpoint": route.endpoint,
                "summary": route.summary,
            })
    return endpoints


def register_fastapi(app: FastAPI, prefix: str = "api"):
    """
    Register FastAPI routes as apcore modules

    Args:
        app: FastAPI application instance
        prefix: Module ID prefix
    """
    for route in app.routes:
        if not isinstance(route, APIRoute):
            continue

        endpoint = route.endpoint
        route_name = route.name or endpoint.__name__

        # Generate module ID: prefix.route_name
        module_id = f"{prefix}.{route_name}"

        # Register via module(), leveraging FastAPI endpoint's type annotations
        module(endpoint, id=module_id, description=route.summary)


def generate_bindings(app: FastAPI, prefix: str = "api") -> dict:
    """
    Generate Binding YAML content from FastAPI application

    Args:
        app: FastAPI application instance
        prefix: Module ID prefix

    Returns:
        dict that can be written to .binding.yaml
    """
    bindings = []
    for route in app.routes:
        if not isinstance(route, APIRoute):
            continue

        endpoint = route.endpoint
        route_name = route.name or endpoint.__name__
        module_path = f"{endpoint.__module__}:{endpoint.__qualname__}"

        bindings.append({
            "module_id": f"{prefix}.{route_name}",
            "target": module_path,
            "description": route.summary or f"API endpoint: {route.path}",
            "auto_schema": True,
            "tags": list(route.tags) if route.tags else [],
            "metadata": {
                "http_path": route.path,
                "http_methods": list(route.methods or []),
            }
        })

    return {"bindings": bindings}
```

**Usage:**

```python
from fastapi import FastAPI
from apcore_fastapi import register_fastapi

app = FastAPI()

@app.get("/users/{user_id}")
def get_user(user_id: int) -> dict:
    """Get user information"""
    return {"id": user_id, "name": "Alice"}

@app.post("/emails/send")
def send_email(to: str, subject: str, body: str) -> dict:
    """Send email"""
    return {"success": True}

# One line to register all routes as apcore modules
register_fastapi(app)
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
