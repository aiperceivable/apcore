# Getting Started

apcore defines the **Cognitive Interface** for your application. It ensures your logic is naturally perceivable by AI Agents through enforced schemas, behavioral guardrails, and self-healing error guidance.

Whether you are building from scratch or upgrading a legacy system, apcore provides a path for you. See [Choose Your Integration Path](./guides/creating-modules.md#choose-your-integration-path) for details.

## 1. Installation

=== "Python"

    ```bash
    pip install apcore
    ```

    Requires Python 3.11+.

=== "TypeScript"

    ```bash
    npm install apcore-js
    ```

    Requires Node.js 18+ and TypeScript 5.5+.

=== "Rust"

    ```bash
    cargo add apcore
    ```

    Requires Rust 1.75+.

## 2. Your First Module

The `APCore` client is the recommended entry point. It manages Registry and Executor for you.

=== "Python"

    ```python
    from apcore import APCore

    client = APCore()

    @client.module(id="math.add", description="Add two numbers", tags=["math"])
    def add(a: int, b: int) -> dict:
        return {"sum": a + b}

    # Call the module
    result = client.call("math.add", {"a": 10, "b": 5})
    print(result)  # {'sum': 15}
    ```

    For simple scripts, you can also use the global `apcore.*` functions directly:

    ```python
    import apcore

    @apcore.module(id="math.add", description="Add two numbers")
    def add(a: int, b: int) -> dict:
        return {"sum": a + b}

    result = apcore.call("math.add", {"a": 10, "b": 5})
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';
    import { APCore } from 'apcore-js';

    const client = new APCore();

    client.module({
      id: 'math.add',
      description: 'Add two numbers',
      tags: ['math'],
      inputSchema: Type.Object({ a: Type.Number(), b: Type.Number() }),
      outputSchema: Type.Object({ sum: Type.Number() }),
      execute: (inputs) => ({ sum: (inputs.a as number) + (inputs.b as number) }),
    });

    const result = await client.call('math.add', { a: 10, b: 5 });
    console.log(result); // { sum: 15 }
    ```

=== "Rust"

    ```rust
    use apcore::APCore;
    use serde_json::json;

    #[tokio::main]
    async fn main() {
        let client = APCore::new();

        #[apcore::module(id = "math.add", description = "Add two numbers")]
        fn add(a: i64, b: i64) -> i64 {
            a + b
        }

        let result = client.call("math.add", json!({"a": 10, "b": 5})).await;
        println!("{:?}", result); // {"result": 15}
    }
    ```

## 3. Preflight Validation

Before executing, you can validate inputs without running the module:

=== "Python"

    ```python
    preflight = client.validate("math.add", {"a": 10, "b": 5})
    if preflight.valid:
        print("All checks passed")
    else:
        print(f"Errors: {preflight.errors}")
    ```

=== "TypeScript"

    ```typescript
    const preflight = client.validate('math.add', { a: 10, b: 5 });
    if (preflight.valid) {
      console.log('All checks passed');
    } else {
      console.log('Errors:', preflight.errors);
    }
    ```

=== "Rust"

    ```rust
    let preflight = client.validate("math.add", json!({"a": 10, "b": 5}));
    if preflight.is_valid() {
        println!("All checks passed");
    } else {
        println!("Errors: {:?}", preflight.errors());
    }
    ```

## 4. Streaming

Modules that implement `stream()` can yield output chunks incrementally:

=== "Python"

    ```python
    async for chunk in client.stream("my.streaming_module", {"query": "hello"}):
        print(chunk)
    ```

=== "TypeScript"

    ```typescript
    for await (const chunk of client.stream('my.streaming_module', { query: 'hello' })) {
      console.log(chunk);
    }
    ```

=== "Rust"

    ```rust
    use futures::StreamExt;

    let mut stream = client.stream("my.streaming_module", json!({"query": "hello"})).await;
    while let Some(chunk) = stream.next().await {
        println!("{:?}", chunk);
    }
    ```

## 5. Auto-Discovery (Directory as ID)

Instead of manual registration, apcore can automatically discover modules in a directory.

=== "Python"

    Project structure:
    ```text
    my-project/
    └── extensions/
        └── math/
            └── add.py  # Contains the module
    ```

    ```python
    from apcore import APCore

    client = APCore()
    client.discover()
    # Module ID is automatically "math.add"
    result = client.call("math.add", {"a": 10, "b": 5})
    ```

=== "TypeScript"

    Project structure:
    ```text
    my-project/
    └── extensions/
        └── math/
            └── add.ts  # Export default or named module
    ```

    ```typescript
    import { APCore } from 'apcore-js';

    const client = new APCore();
    await client.discover();
    // Module ID is automatically "math.add"
    const result = await client.call('math.add', { a: 10, b: 5 });
    ```

=== "Rust"

    Project structure:
    ```text
    my-project/
    └── extensions/
        └── math/
            └── add.rs  # Rust module file
    ```

    ```rust
    let mut client = APCore::new();
    client.discover().await?;
    // Module ID is automatically "math.add"
    let result = client.call("math.add", json!({"a": 10, "b": 5})).await;
    ```

## 6. Adding Middleware

Middleware can intercept calls for logging, tracing, or security.

=== "Python"

    ```python
    from apcore import LoggingMiddleware, TracingMiddleware

    client.use(LoggingMiddleware())
    client.use(TracingMiddleware())
    ```

=== "TypeScript"

    ```typescript
    import { ObsLoggingMiddleware } from 'apcore-js';

    client.use(new ObsLoggingMiddleware());
    ```

=== "Rust"

    ```rust
    use apcore::middleware::{LoggingMiddleware, TracingMiddleware};

    client.use_middleware(LoggingMiddleware::new());
    client.use_middleware(TracingMiddleware::new());
    ```

## 7. Advanced: Manual Registry + Executor

For full control, you can manage Registry and Executor separately:

=== "Python"

    ```python
    from apcore import Registry, Executor, module

    registry = Registry(extensions_dir="./extensions")
    registry.discover()

    executor = Executor(registry=registry)
    result = executor.call("math.add", {"a": 10, "b": 5})
    ```

=== "TypeScript"

    ```typescript
    import { Registry, Executor } from 'apcore-js';

    const registry = new Registry({ extensionsDir: './extensions' });
    await registry.discover();

    const executor = new Executor({ registry });
    const result = await executor.call('math.add', { a: 10, b: 5 });
    ```

=== "Rust"

    ```rust
    use apcore::{Registry, Executor};

    let mut registry = Registry::new("./extensions");
    registry.discover().await?;

    let executor = Executor::new(registry);
    let result = executor.call("math.add", json!({"a": 10, "b": 5})).await;
    ```

## 8. Global Entry Points

All `APCore` client methods are also available as module-level functions via a default client instance:

=== "Python"

    ```python
    import apcore

    # Registration
    @apcore.module(id="math.add", description="Add two numbers")
    def add(a: int, b: int) -> dict:
        return {"sum": a + b}

    apcore.register("math.add", my_module)   # Direct registration
    apcore.discover()                         # Auto-discover from extensions dir

    # Execution
    result = apcore.call("math.add", {"a": 1, "b": 2})
    preflight = apcore.validate("math.add", {"a": 1})
    # apcore.stream(), apcore.call_async() also available

    # Discovery & Inspection
    modules = apcore.list_modules(prefix="math.")
    desc = apcore.describe("math.add")

    # Middleware
    apcore.use(LoggingMiddleware())
    apcore.use_before(lambda mid, inp, ctx: print(f"→ {mid}"))
    apcore.use_after(lambda mid, inp, out, ctx: print(f"← {mid}"))
    apcore.remove(mw)

    # Events & Control (requires config with sys_modules enabled)
    sub = apcore.on("module_health_changed", handler)
    apcore.off(sub)
    apcore.disable("some.module", reason="maintenance")
    apcore.enable("some.module", reason="done")
    ```

=== "Rust"

    ```rust
    use apcore;

    // Registration
    #[apcore::module(id = "math.add")]
    fn add(a: i64, b: i64) -> i64 { a + b }

    apcore::discover().await?;

    // Execution
    let result = apcore::call("math.add", json!({"a": 1, "b": 2})).await;
    let preflight = apcore::validate("math.add", json!({"a": 1}));

    // Discovery & Inspection
    let modules = apcore::list_modules("math.");
    let desc = apcore::describe("math.add").await;

    // Middleware
    apcore::use_middleware(LoggingMiddleware::new());
    ```

See [APCore Client API](api/client-api.md) for the full reference.

## 9. System Modules

When configured with `sys_modules.enabled: true`, APCore auto-registers built-in `system.*` modules for health monitoring, usage analytics, manifest inspection, and runtime control.

=== "Python"

    ```python
    from apcore import APCore
    from apcore.config import Config

    config = Config.load("apcore.yaml")
    client = APCore(config=config)

    # Health overview
    health = client.call("system.health.summary", {})
    print(health["summary"])  # {"total_modules": 12, "healthy": 10, ...}

    # Usage statistics
    usage = client.call("system.usage.summary", {"period": "24h"})

    # Module manifest
    manifest = client.call("system.manifest.full", {"prefix": "math."})

    # Runtime control (requires approval handler + events enabled)
    client.disable("risky.module", reason="investigating issue")
    client.enable("risky.module", reason="issue resolved")
    ```

=== "Rust"

    ```rust
    use apcore::{APCore, Config};

    let config = Config::load("apcore.yaml")?;
    let client = APCore::with_config(config);

    // Health overview
    let health = client.call("system.health.summary", json!({})).await;

    // Usage statistics
    let usage = client.call("system.usage.summary", json!({"period": "24h"})).await;

    // Module manifest
    let manifest = client.call("system.manifest.full", json!({"prefix": "math."})).await;
    ```

See [System Modules](features/system-modules.md) for the full module reference.

## Next Steps

- [APCore Client API](api/client-api.md) - Complete unified client reference.
- [Creating Modules Guide](guides/creating-modules.md) - Deep dive into module definition.
- [Schema Definition Guide](guides/schema-definition.md) - Learn about advanced validation.
- [ACL Configuration](guides/acl-configuration.md) - Secure your modules.
- [Event System](features/event-system.md) - Framework event bus and subscribers.
- [System Modules](features/system-modules.md) - Built-in health, usage, and control modules.
- [apcore-mcp](https://github.com/aipartnerup/apcore-mcp) - Expose these modules as an MCP Server.
