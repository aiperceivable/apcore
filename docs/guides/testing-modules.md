---
description: "Test apcore modules in Python, TypeScript and Rust: unit tests with a real Context, calls through the client, schema checks, middleware, error paths and approval-gated modules."
---

# Testing Modules

apcore modules are easy to test because everything around them is a real, cheap object: a `Context` is a plain value, a client with an in-memory registry costs almost nothing to create, and schemas are data you can validate directly. This guide builds a small module and tests it at each level, in pytest, Vitest and Rust's built-in test harness.

## 1. What to test where

| What you want to know | How to test it | Section |
|---|---|---|
| The module's own logic is right | Call `execute()` directly with a real `Context` | [§3](#3-unit-testing-a-module) |
| The module behaves correctly behind the pipeline (validation, errors, output) | Call it through a fresh `APCore` client | [§4](#4-testing-through-the-client) |
| Your middleware does what it should, in the right order | Call its hooks directly, then register it with `client.use()` | [§5](#5-testing-middleware) |
| The schema accepts and rejects the right values | `SchemaValidator` against the module's schema | [§6](#6-testing-schemas) |
| Failures reach the caller as the right error | Assert on the error type and `code` | [§7](#7-testing-error-paths) |
| A human-approval gate blocks and releases the call | Built-in approval handlers | [§8](#8-testing-approval-gated-modules) |
| An external service is called correctly | Inject a fake through the module constructor | [§9](#9-mocking-dependencies) |
| ACL rules allow and deny the right callers | See [ACL Configuration: Testing ACL](./acl-configuration.md#102-testing-acl) | [§10](#10-testing-acl-rules) |

Prefer real apcore objects over mocks. `Context`, `Registry` and `APCore` need no network or files, and a test that goes through them exercises the same code your callers do.

## 2. The module under test

A greeting module with a bounded input: `name` is 1–50 characters, `times` is an optional integer from 1 to 3.

=== "Python"

    ```python
    # my_app/greet.py
    from typing import Any

    from pydantic import BaseModel, Field

    from apcore import Context


    class GreetInput(BaseModel):
        name: str = Field(min_length=1, max_length=50)
        times: int = Field(default=1, ge=1, le=3)


    class GreetOutput(BaseModel):
        message: str


    class GreetModule:
        input_schema = GreetInput
        output_schema = GreetOutput
        description = "Greet someone by name"

        def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
            times = inputs.get("times", 1)
            return {"message": " ".join([f"Hello, {inputs['name']}!"] * times)}
    ```

    The executor validates inputs but passes them on as given, so read optional fields with a default (`inputs.get("times", 1)`).

=== "TypeScript"

    ```typescript
    // src/greet.ts
    import { Type } from '@sinclair/typebox';
    import type { Context, Module } from 'apcore-js';

    export const GreetInput = Type.Object({
      name: Type.String({ minLength: 1, maxLength: 50 }),
      times: Type.Optional(Type.Integer({ minimum: 1, maximum: 3 })),
    });

    export const GreetOutput = Type.Object({ message: Type.String() });

    export class GreetModule implements Module {
      inputSchema = GreetInput;
      outputSchema = GreetOutput;
      description = 'Greet someone by name';

      execute(inputs: Record<string, unknown>, _context: Context): Record<string, unknown> {
        const times = (inputs['times'] as number | undefined) ?? 1;
        const greeting = `Hello, ${String(inputs['name'])}!`;
        return { message: Array<string>(times).fill(greeting).join(' ') };
      }
    }
    ```

=== "Rust"

    ```rust
    // src/lib.rs of the `my_app` crate
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::module::Module;
    use async_trait::async_trait;
    use serde_json::{json, Value};

    pub struct GreetModule;

    #[async_trait]
    impl Module for GreetModule {
        fn input_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "minLength": 1, "maxLength": 50 },
                    "times": { "type": "integer", "minimum": 1, "maximum": 3 }
                },
                "required": ["name"]
            })
        }

        fn output_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": { "message": { "type": "string" } },
                "required": ["message"]
            })
        }

        fn description(&self) -> &str {
            "Greet someone by name"
        }

        async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
            let name = inputs["name"]
                .as_str()
                .ok_or_else(|| ModuleError::invalid_input("name must be a string"))?;
            let times = inputs["times"].as_u64().unwrap_or(1) as usize;
            Ok(json!({ "message": vec![format!("Hello, {name}!"); times].join(" ") }))
        }
    }
    ```

    Integration tests under `tests/` are separate crates: they import your code by crate name (`use my_app::GreetModule;`), never through `crate::`. Add `tokio` (features `macros`, `rt-multi-thread`) as a dev-dependency for `#[tokio::test]`.

## 3. Unit-testing a module

Call `execute()` directly with a real `Context`. This is the fastest test and isolates the module's logic — but it skips everything the pipeline does: no schema validation, no ACL, no middleware. Out-of-range inputs are the job of §4 and §6.

=== "Python"

    ```python
    # tests/test_greet.py
    from apcore import Context
    from my_app.greet import GreetModule


    def test_greets_by_name() -> None:
        assert GreetModule().execute({"name": "Ada"}, Context.create()) == {"message": "Hello, Ada!"}


    def test_repeats_the_greeting() -> None:
        result = GreetModule().execute({"name": "Ada", "times": 2}, Context.create())
        assert result == {"message": "Hello, Ada! Hello, Ada!"}
    ```

=== "TypeScript"

    ```typescript
    // tests/greet.test.ts
    import { describe, expect, it } from 'vitest';
    import { Context } from 'apcore-js';
    import { GreetModule } from '../src/greet.js';

    describe('GreetModule.execute', () => {
      it('greets by name', () => {
        expect(new GreetModule().execute({ name: 'Ada' }, Context.create())).toEqual({
          message: 'Hello, Ada!',
        });
      });

      it('repeats the greeting', () => {
        expect(new GreetModule().execute({ name: 'Ada', times: 2 }, Context.create())).toEqual({
          message: 'Hello, Ada! Hello, Ada!',
        });
      });
    });
    ```

=== "Rust"

    ```rust
    // tests/greet.rs
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::module::Module;
    use my_app::GreetModule;
    use serde_json::{json, Value};

    #[tokio::test]
    async fn greets_by_name() -> Result<(), ModuleError> {
        let out = GreetModule.execute(json!({ "name": "Ada" }), &Context::<Value>::anonymous()).await?;
        assert_eq!(out, json!({ "message": "Hello, Ada!" }));
        Ok(())
    }

    #[tokio::test]
    async fn repeats_the_greeting() -> Result<(), ModuleError> {
        let ctx = Context::<Value>::anonymous();
        let out = GreetModule.execute(json!({ "name": "Ada", "times": 2 }), &ctx).await?;
        assert_eq!(out, json!({ "message": "Hello, Ada! Hello, Ada!" }));
        Ok(())
    }
    ```

### 3.1 Building a context the module depends on

If a module reads `identity`, `caller_id` or `data`, build its context the way the executor does: create a top-level context, then derive with `child()`. The executor calls `child(module_id)` at pipeline entry, which sets `call_chain` and makes the previous entry the `caller_id`. Deriving is better than setting fields by hand — the Python `Context` is a mutable dataclass, so hand edits are possible, but they skip the bookkeeping `child()` does.

=== "Python"

    ```python
    from apcore import Context, Identity


    def make_context(*, caller: str | None = None, roles: tuple[str, ...] = ()) -> Context:
        """A context shaped like the one the executor hands to greet.hello."""
        ctx = Context.create(identity=Identity(id="test-user", roles=roles), data={"ext.acme.tenant": "t-1"})
        if caller is not None:
            ctx = ctx.child(caller)  # the calling module
        return ctx.child("greet.hello")  # the module under test; caller_id == caller
    ```

=== "TypeScript"

    ```typescript
    import { Context, createIdentity } from 'apcore-js';

    /** A context shaped like the one the executor hands to greet.hello. */
    export function makeContext(caller?: string, roles: string[] = []): Context {
      let ctx = Context.create(createIdentity('test-user', 'user', roles), null, null, {
        'ext.acme.tenant': 't-1',
      });
      if (caller !== undefined) {
        ctx = ctx.child(caller); // the calling module
      }
      return ctx.child('greet.hello'); // the module under test; callerId === caller
    }
    ```

=== "Rust"

    ```rust
    use apcore::context::{Context, Identity};
    use serde_json::{json, Value};
    use std::collections::HashMap;

    /// A context shaped like the one the executor hands to greet.hello.
    pub fn make_context(caller: Option<&str>, roles: Vec<String>) -> Context<Value> {
        let ctx = Context::<Value>::new(Identity::new(
            "test-user".to_string(),
            "user".to_string(),
            roles,
            HashMap::new(),
        ));
        ctx.data.write().insert("ext.acme.tenant".to_string(), json!("t-1"));
        let ctx = match caller {
            Some(caller) => ctx.child(caller), // the calling module
            None => ctx,
        };
        ctx.child("greet.hello") // the module under test; caller_id == caller
    }
    ```

## 4. Testing through the client

A call through `APCore` runs the full pipeline: input validation, the module, output validation, and any middleware you add. Build a fresh client in each test so registrations and middleware never leak between tests.

=== "Python"

    ```python
    # tests/test_greet_client.py
    import pytest

    from apcore import APCore, SchemaValidationError
    from my_app.greet import GreetModule


    @pytest.fixture
    def client() -> APCore:
        client = APCore()
        client.register("greet.hello", GreetModule())
        return client


    def test_call_returns_the_module_output(client: APCore) -> None:
        assert client.call("greet.hello", {"name": "Ada", "times": 2}) == {"message": "Hello, Ada! Hello, Ada!"}


    def test_invalid_input_is_rejected_before_execute(client: APCore) -> None:
        with pytest.raises(SchemaValidationError) as exc_info:
            client.call("greet.hello", {"name": ""})
        assert exc_info.value.code == "SCHEMA_VALIDATION_ERROR"
        assert exc_info.value.details["errors"]  # one entry per violation


    def test_validate_checks_without_executing(client: APCore) -> None:
        assert client.validate("greet.hello", {"name": "Ada", "times": 9}).valid is False
    ```

    `client.call()` is synchronous. In an `async def` test, use `await client.call_async(...)`.

=== "TypeScript"

    ```typescript
    // tests/greet.client.test.ts
    import { beforeEach, describe, expect, it } from 'vitest';
    import { APCore, Registry, SchemaValidationError } from 'apcore-js';
    import { GreetModule } from '../src/greet.js';

    describe('greet.hello through the client', () => {
      let client: APCore;

      beforeEach(async () => {
        const registry = new Registry();
        await registry.register('greet.hello', new GreetModule());
        client = new APCore({ registry });
      });

      it('returns the module output', async () => {
        await expect(client.call('greet.hello', { name: 'Ada', times: 2 })).resolves.toEqual({
          message: 'Hello, Ada! Hello, Ada!',
        });
      });

      it('rejects invalid input before execute', async () => {
        const error = await client.call('greet.hello', { name: '' }).catch((e: unknown) => e);
        expect(error).toBeInstanceOf(SchemaValidationError);
        expect((error as SchemaValidationError).code).toBe('SCHEMA_VALIDATION_ERROR');
        expect((error as SchemaValidationError).details['errors']).not.toHaveLength(0);
      });

      it('validate() checks without executing', async () => {
        const preflight = await client.validate('greet.hello', { name: 'Ada', times: 9 });
        expect(preflight.valid).toBe(false);
      });
    });
    ```

    `Registry.register()` returns a promise; await it before the first call.

=== "Rust"

    ```rust
    // tests/greet_client.rs
    use apcore::errors::{ErrorCode, ModuleError};
    use apcore::APCore;
    use my_app::GreetModule;
    use serde_json::json;

    fn client() -> APCore {
        let client = APCore::new();
        client.register("greet.hello", Box::new(GreetModule)).expect("register greet.hello");
        client
    }

    #[tokio::test]
    async fn call_returns_the_module_output() -> Result<(), ModuleError> {
        let out = client().call("greet.hello", json!({ "name": "Ada", "times": 2 }), None, None).await?;
        assert_eq!(out, json!({ "message": "Hello, Ada! Hello, Ada!" }));
        Ok(())
    }

    #[tokio::test]
    async fn invalid_input_is_rejected_before_execute() {
        let err = client()
            .call("greet.hello", json!({ "name": "" }), None, None)
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SchemaValidationError);
    }

    #[tokio::test]
    async fn validate_checks_without_executing() -> Result<(), ModuleError> {
        let preflight = client()
            .validate("greet.hello", &json!({ "name": "Ada", "times": 9 }), None)
            .await?;
        assert!(!preflight.valid);
        Ok(())
    }
    ```

To check what discovery registered, assert on `client.registry.list()` and `client.registry.has(module_id)` (Rust: `client.registry().list(None, None, None)` and `has(module_id)`).

## 5. Testing middleware

### 5.1 Hooks in isolation

A hook is an ordinary method: call it with a real `Context` and assert on what it returns. These tests use the `ShapingMiddleware` from the [Middleware Guide](./middleware.md#5-modifying-inputs-and-outputs), which adds a default `locale` to `notify.*` inputs.

=== "Python"

    ```python
    from apcore import Context
    from my_app.middleware import ShapingMiddleware


    def test_before_adds_a_default_locale() -> None:
        mw, ctx = ShapingMiddleware(), Context.create()
        assert mw.before("notify.email", {"to": "a@example.com"}, ctx) == {"to": "a@example.com", "locale": "en-US"}


    def test_before_leaves_other_inputs_alone() -> None:
        mw, ctx = ShapingMiddleware(), Context.create()
        assert mw.before("notify.email", {"locale": "fr-FR"}, ctx) is None
        assert mw.before("greet.hello", {"name": "Ada"}, ctx) is None
    ```

=== "TypeScript"

    ```typescript
    import { describe, expect, it } from 'vitest';
    import { Context } from 'apcore-js';
    import { ShapingMiddleware } from '../src/middleware.js';

    describe('ShapingMiddleware.before', () => {
      const mw = new ShapingMiddleware();

      it('adds a default locale', () => {
        expect(mw.before('notify.email', { to: 'a@example.com' }, Context.create())).toEqual({
          to: 'a@example.com',
          locale: 'en-US',
        });
      });

      it('leaves other inputs alone', () => {
        expect(mw.before('notify.email', { locale: 'fr-FR' }, Context.create())).toBeNull();
        expect(mw.before('greet.hello', { name: 'Ada' }, Context.create())).toBeNull();
      });
    });
    ```

=== "Rust"

    ```rust
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::middleware::Middleware;
    use my_app::ShapingMiddleware;
    use serde_json::{json, Value};

    #[tokio::test]
    async fn before_adds_a_default_locale() -> Result<(), ModuleError> {
        let ctx = Context::<Value>::anonymous();
        let out = ShapingMiddleware.before("notify.email", json!({ "to": "a@example.com" }), &ctx).await?;
        assert_eq!(out, Some(json!({ "to": "a@example.com", "locale": "en-US" })));
        Ok(())
    }

    #[tokio::test]
    async fn before_leaves_other_inputs_alone() -> Result<(), ModuleError> {
        let ctx = Context::<Value>::anonymous();
        assert_eq!(ShapingMiddleware.before("notify.email", json!({ "locale": "fr-FR" }), &ctx).await?, None);
        assert_eq!(ShapingMiddleware.before("greet.hello", json!({ "name": "Ada" }), &ctx).await?, None);
        Ok(())
    }
    ```

### 5.2 Order through the client

Register middleware with `client.use()` (Rust: `use_middleware`) and record what runs. A higher priority wraps a lower one.

=== "Python"

    ```python
    from typing import Any

    from apcore import APCore, Context, Middleware
    from my_app.greet import GreetModule


    class Recorder(Middleware):
        def __init__(self, label: str, log: list[str], priority: int) -> None:
            super().__init__(priority=priority)
            self._label, self._log = label, log

        def before(self, module_id: str, inputs: dict[str, Any], context: Context) -> None:
            self._log.append(f"{self._label}:before")

        def after(self, module_id: str, inputs: dict[str, Any], output: dict[str, Any], context: Context) -> None:
            self._log.append(f"{self._label}:after")


    def test_higher_priority_wraps_lower_priority() -> None:
        log: list[str] = []
        client = APCore()
        client.register("greet.hello", GreetModule())
        # Two instances of one class: expect a duplicate-registration warning in the log.
        client.use(Recorder("inner", log, priority=100))
        client.use(Recorder("outer", log, priority=500))

        client.call("greet.hello", {"name": "Ada"})

        assert log == ["outer:before", "inner:before", "inner:after", "outer:after"]
    ```

=== "TypeScript"

    ```typescript
    import { expect, it } from 'vitest';
    import { APCore, Middleware, Registry } from 'apcore-js';
    import type { Context } from 'apcore-js';
    import { GreetModule } from '../src/greet.js';

    class Recorder extends Middleware {
      constructor(
        private readonly label: string,
        private readonly log: string[],
        priority: number,
      ) {
        super(priority);
      }

      override before(_m: string, _i: Record<string, unknown>, _c: Context): null {
        this.log.push(`${this.label}:before`);
        return null;
      }

      override after(_m: string, _i: Record<string, unknown>, _o: Record<string, unknown>, _c: Context): null {
        this.log.push(`${this.label}:after`);
        return null;
      }
    }

    it('higher priority wraps lower priority', async () => {
      const log: string[] = [];
      const registry = new Registry();
      await registry.register('greet.hello', new GreetModule());
      const client = new APCore({ registry });
      // Two instances of one class: expect a duplicate-registration warning.
      client.use(new Recorder('inner', log, 100)).use(new Recorder('outer', log, 500));

      await client.call('greet.hello', { name: 'Ada' });

      expect(log).toEqual(['outer:before', 'inner:before', 'inner:after', 'outer:after']);
    });
    ```

=== "Rust"

    ```rust
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::middleware::Middleware;
    use apcore::APCore;
    use async_trait::async_trait;
    use my_app::GreetModule;
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};

    #[derive(Debug)]
    struct Recorder {
        label: &'static str,
        priority: u16,
        log: Arc<Mutex<Vec<String>>>,
    }

    impl Recorder {
        fn record(&self, hook: &str) {
            self.log.lock().unwrap().push(format!("{}:{hook}", self.label));
        }
    }

    #[async_trait]
    impl Middleware for Recorder {
        fn name(&self) -> &str {
            self.label
        }
        fn priority(&self) -> u16 {
            self.priority
        }
        async fn before(&self, _: &str, _: Value, _: &Context<Value>) -> Result<Option<Value>, ModuleError> {
            self.record("before");
            Ok(None)
        }
        async fn after(&self, _: &str, _: Value, _: Value, _: &Context<Value>) -> Result<Option<Value>, ModuleError> {
            self.record("after");
            Ok(None)
        }
        async fn on_error(&self, _: &str, _: Value, _: &ModuleError, _: &Context<Value>) -> Result<Option<Value>, ModuleError> {
            self.record("on_error");
            Ok(None)
        }
    }

    #[tokio::test]
    async fn higher_priority_wraps_lower_priority() -> Result<(), ModuleError> {
        let log = Arc::new(Mutex::new(Vec::new()));
        let client = APCore::new();
        client.register("greet.hello", Box::new(GreetModule))?;
        client.use_middleware(Box::new(Recorder { label: "inner", priority: 100, log: log.clone() }))?;
        client.use_middleware(Box::new(Recorder { label: "outer", priority: 500, log: log.clone() }))?;

        client.call("greet.hello", json!({ "name": "Ada" }), None, None).await?;

        assert_eq!(*log.lock().unwrap(), ["outer:before", "inner:before", "inner:after", "outer:after"]);
        Ok(())
    }
    ```

## 6. Testing schemas

Test a schema's boundaries with the SDK's own `SchemaValidator` — the same validation rules the executor applies — rather than a general-purpose JSON Schema library. Validation is strict: `"2"` is not an integer.

=== "Python"

    ```python
    import pytest

    from apcore import SchemaValidator
    from my_app.greet import GreetInput

    validator = SchemaValidator()


    @pytest.mark.parametrize("inputs", [{"name": "A"}, {"name": "x" * 50}, {"name": "Ada", "times": 3}])
    def test_accepts_boundary_values(inputs: dict) -> None:
        assert validator.validate(inputs, GreetInput).valid


    @pytest.mark.parametrize(
        "inputs",
        [
            {},
            {"name": ""},
            {"name": "x" * 51},
            {"name": "Ada", "times": 0},
            {"name": "Ada", "times": 4},
            {"name": "Ada", "times": "2"},
        ],
    )
    def test_rejects_out_of_range_values(inputs: dict) -> None:
        result = validator.validate(inputs, GreetInput)
        assert not result.valid
        assert result.errors  # each has .path, .message, .constraint
    ```

=== "TypeScript"

    ```typescript
    import { describe, expect, it } from 'vitest';
    import { SchemaValidator } from 'apcore-js';
    import { GreetInput } from '../src/greet.js';

    const validator = new SchemaValidator();

    describe('greet.hello input schema', () => {
      it.each([{ name: 'A' }, { name: 'x'.repeat(50) }, { name: 'Ada', times: 3 }])(
        'accepts %j',
        (inputs) => {
          expect(validator.validate(inputs, GreetInput).valid).toBe(true);
        },
      );

      it.each([
        {},
        { name: '' },
        { name: 'x'.repeat(51) },
        { name: 'Ada', times: 0 },
        { name: 'Ada', times: 4 },
        { name: 'Ada', times: '2' },
      ])('rejects %j', (inputs: Record<string, unknown>) => {
        const result = validator.validate(inputs, GreetInput);
        expect(result.valid).toBe(false);
        expect(result.errors.length).toBeGreaterThan(0); // each has path, message, constraint
      });
    });
    ```

=== "Rust"

    ```rust
    use apcore::module::Module;
    use apcore::SchemaValidator;
    use my_app::GreetModule;
    use serde_json::json;

    #[test]
    fn input_schema_boundaries() {
        let validator = SchemaValidator::new();
        let schema = GreetModule.input_schema();

        for ok in [
            json!({ "name": "A" }),
            json!({ "name": "x".repeat(50) }),
            json!({ "name": "Ada", "times": 3 }),
        ] {
            assert!(validator.validate(&ok, &schema).valid, "should accept {ok}");
        }

        for bad in [
            json!({}),
            json!({ "name": "" }),
            json!({ "name": "x".repeat(51) }),
            json!({ "name": "Ada", "times": 0 }),
            json!({ "name": "Ada", "times": 4 }),
            json!({ "name": "Ada", "times": "2" }),
        ] {
            let result = validator.validate(&bad, &schema);
            assert!(!result.valid, "should reject {bad}");
            assert!(!result.errors.is_empty()); // each has path, message, constraint
        }
    }
    ```

The error detail format is specified in [Schema System](../features/schema-system.md).

## 7. Testing error paths

Assert on the error the **caller** receives, and on its `code` — that is what bridges and clients dispatch on. What arrives depends on what was raised:

| Raised by | What the caller receives |
|---|---|
| Module or `before()` raises a `ModuleError` (any subclass) | That error, unchanged, with `trace_id` and call details added |
| Module or `before()` raises anything else (Python, TypeScript) | `ModuleExecuteError`, code `MODULE_EXECUTE_ERROR`, with the original as `cause` |
| Rust module or hook returns `Err(ModuleError)` | That error, unchanged apart from the same added details |

A middleware `on_error()` that returns a value turns the failure into a result instead — test that separately if you use one.

=== "Python"

    ```python
    import pytest
    from pydantic import BaseModel

    from apcore import APCore, Context, InvalidInputError, Middleware, ModuleExecuteError
    from my_app.greet import GreetModule


    class Empty(BaseModel):
        pass


    class BrokenModule:
        input_schema = Empty
        output_schema = Empty
        description = "Always fails"

        def execute(self, inputs: dict, context: Context) -> dict:
            raise RuntimeError("database unavailable")


    def test_unexpected_exception_is_wrapped() -> None:
        client = APCore()
        client.register("demo.broken", BrokenModule())
        with pytest.raises(ModuleExecuteError) as exc_info:
            client.call("demo.broken", {})
        assert exc_info.value.code == "MODULE_EXECUTE_ERROR"
        assert isinstance(exc_info.value.cause, RuntimeError)


    def test_module_error_from_middleware_reaches_the_caller_unchanged() -> None:
        class Reject(Middleware):
            def before(self, module_id: str, inputs: dict, context: Context) -> None:
                raise InvalidInputError(message="rejected by policy")

        client = APCore()
        client.register("greet.hello", GreetModule())
        client.use(Reject())
        with pytest.raises(InvalidInputError) as exc_info:
            client.call("greet.hello", {"name": "Ada"})
        assert exc_info.value.code == "GENERAL_INVALID_INPUT"
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';
    import { expect, it } from 'vitest';
    import { APCore, InvalidInputError, Middleware, ModuleExecuteError, Registry } from 'apcore-js';
    import type { Module } from 'apcore-js';
    import { GreetModule } from '../src/greet.js';

    class BrokenModule implements Module {
      inputSchema = Type.Object({});
      outputSchema = Type.Object({});
      description = 'Always fails';

      execute(): Record<string, unknown> {
        throw new Error('database unavailable');
      }
    }

    it('wraps an unexpected exception as ModuleExecuteError', async () => {
      const registry = new Registry();
      await registry.register('demo.broken', new BrokenModule());
      const client = new APCore({ registry });

      const error = await client.call('demo.broken', {}).catch((e: unknown) => e);
      expect(error).toBeInstanceOf(ModuleExecuteError);
      expect((error as ModuleExecuteError).code).toBe('MODULE_EXECUTE_ERROR');
      expect((error as ModuleExecuteError).cause).toBeInstanceOf(Error);
    });

    it('passes a ModuleError from middleware through unchanged', async () => {
      class Reject extends Middleware {
        override before(): null {
          throw new InvalidInputError('rejected by policy');
        }
      }
      const registry = new Registry();
      await registry.register('greet.hello', new GreetModule());
      const client = new APCore({ registry }).use(new Reject());

      const error = await client.call('greet.hello', { name: 'Ada' }).catch((e: unknown) => e);
      expect(error).toBeInstanceOf(InvalidInputError);
      expect((error as InvalidInputError).code).toBe('GENERAL_INVALID_INPUT');
    });
    ```

=== "Rust"

    ```rust
    use apcore::context::Context;
    use apcore::errors::{ErrorCode, ModuleError};
    use apcore::middleware::adapters::BeforeAdapter;
    use apcore::module::Module;
    use apcore::APCore;
    use async_trait::async_trait;
    use my_app::GreetModule;
    use serde_json::{json, Value};

    struct BrokenModule;

    #[async_trait]
    impl Module for BrokenModule {
        fn input_schema(&self) -> Value {
            json!({ "type": "object" })
        }
        fn output_schema(&self) -> Value {
            json!({ "type": "object" })
        }
        fn description(&self) -> &str {
            "Always fails"
        }
        async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
            Err(ModuleError::new(ErrorCode::ModuleExecuteError, "database unavailable"))
        }
    }

    #[tokio::test]
    async fn module_error_reaches_the_caller() {
        let client = APCore::new();
        client.register("demo.broken", Box::new(BrokenModule)).unwrap();

        let err = client.call("demo.broken", json!({}), None, None).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::ModuleExecuteError);
        assert_eq!(err.message, "database unavailable");
    }

    #[tokio::test]
    async fn module_error_from_middleware_reaches_the_caller_unchanged() {
        let client = APCore::new();
        client.register("greet.hello", Box::new(GreetModule)).unwrap();
        client
            .use_middleware(Box::new(BeforeAdapter::new(
                "reject",
                |_module_id: String, _inputs: Value, _ctx: Context<Value>| async move {
                    Err::<Option<Value>, ModuleError>(ModuleError::invalid_input("rejected by policy"))
                },
            )))
            .unwrap();

        let err = client.call("greet.hello", json!({ "name": "Ada" }), None, None).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::GeneralInvalidInput);
    }
    ```

## 8. Testing approval-gated modules

A module that declares `requires_approval` stops at the approval gate (pipeline step 5) until an `ApprovalHandler` approves it. Test both outcomes with the built-in handlers: `AlwaysDenyHandler` and `AutoApproveHandler`. Set a handler in every such test — with none configured, the gate is skipped with a warning and the module runs.

The gate also fires when an ACL rule says `approval: required` or an `ExecutionPolicy` gates the module; see [Approval System](../features/approval-system.md).

=== "Python"

    ```python
    from typing import Any

    import pytest
    from pydantic import BaseModel

    from apcore import (
        AlwaysDenyHandler,
        APCore,
        ApprovalDeniedError,
        AutoApproveHandler,
        Context,
        ModuleAnnotations,
    )


    class RefundInput(BaseModel):
        order_id: str


    class RefundOutput(BaseModel):
        refunded: bool


    class RefundModule:
        input_schema = RefundInput
        output_schema = RefundOutput
        description = "Refund an order"
        annotations = ModuleAnnotations(requires_approval=True)

        def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
            return {"refunded": True}


    def make_client(handler: Any) -> APCore:
        client = APCore()
        client.register("billing.refund", RefundModule())
        client.executor.set_approval_handler(handler)
        return client


    def test_denied_approval_blocks_the_call() -> None:
        with pytest.raises(ApprovalDeniedError) as exc_info:
            make_client(AlwaysDenyHandler()).call("billing.refund", {"order_id": "o-1"})
        assert exc_info.value.code == "APPROVAL_DENIED"


    def test_granted_approval_lets_the_call_run() -> None:
        assert make_client(AutoApproveHandler()).call("billing.refund", {"order_id": "o-1"}) == {"refunded": True}


    def test_preflight_reports_the_requirement() -> None:
        assert make_client(AutoApproveHandler()).validate("billing.refund", {"order_id": "o-1"}).requires_approval
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';
    import { expect, it } from 'vitest';
    import {
      AlwaysDenyHandler,
      APCore,
      ApprovalDeniedError,
      AutoApproveHandler,
      createAnnotations,
      Registry,
    } from 'apcore-js';
    import type { ApprovalHandler, Module } from 'apcore-js';

    class RefundModule implements Module {
      inputSchema = Type.Object({ order_id: Type.String() });
      outputSchema = Type.Object({ refunded: Type.Boolean() });
      description = 'Refund an order';
      annotations = createAnnotations({ requiresApproval: true });

      execute(): Record<string, unknown> {
        return { refunded: true };
      }
    }

    async function makeClient(handler: ApprovalHandler): Promise<APCore> {
      const registry = new Registry();
      await registry.register('billing.refund', new RefundModule());
      const client = new APCore({ registry });
      client.executor.setApprovalHandler(handler);
      return client;
    }

    it('blocks the call when approval is denied', async () => {
      const client = await makeClient(new AlwaysDenyHandler());
      const error = await client.call('billing.refund', { order_id: 'o-1' }).catch((e: unknown) => e);
      expect(error).toBeInstanceOf(ApprovalDeniedError);
      expect((error as ApprovalDeniedError).code).toBe('APPROVAL_DENIED');
    });

    it('runs the call when approval is granted', async () => {
      const client = await makeClient(new AutoApproveHandler());
      await expect(client.call('billing.refund', { order_id: 'o-1' })).resolves.toEqual({ refunded: true });
    });

    it('reports the requirement in preflight', async () => {
      const client = await makeClient(new AutoApproveHandler());
      expect((await client.validate('billing.refund', { order_id: 'o-1' })).requiresApproval).toBe(true);
    });
    ```

=== "Rust"

    ```rust
    use apcore::context::Context;
    use apcore::errors::{ErrorCode, ModuleError};
    use apcore::module::Module;
    use apcore::{AlwaysDenyHandler, ApprovalHandler, AutoApproveHandler, Config, Executor, ModuleAnnotations, Registry};
    use async_trait::async_trait;
    use serde_json::{json, Value};

    struct RefundModule;

    #[async_trait]
    impl Module for RefundModule {
        fn input_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": { "order_id": { "type": "string" } },
                "required": ["order_id"]
            })
        }
        fn output_schema(&self) -> Value {
            json!({ "type": "object", "properties": { "refunded": { "type": "boolean" } } })
        }
        fn description(&self) -> &str {
            "Refund an order"
        }
        fn annotations(&self) -> ModuleAnnotations {
            ModuleAnnotations { requires_approval: true, ..Default::default() }
        }
        async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
            Ok(json!({ "refunded": true }))
        }
    }

    // The approval handler is an Executor constructor argument, so build the Executor directly.
    fn executor_with(handler: Box<dyn ApprovalHandler>) -> Executor {
        let registry = Registry::new();
        registry.register_module("billing.refund", Box::new(RefundModule)).expect("register");
        Executor::with_options(registry, Config::default(), None, None, Some(handler))
    }

    #[tokio::test]
    async fn denied_approval_blocks_the_call() {
        let executor = executor_with(Box::new(AlwaysDenyHandler));
        let err = executor
            .call("billing.refund", json!({ "order_id": "o-1" }), None, None)
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::ApprovalDenied);
    }

    #[tokio::test]
    async fn granted_approval_lets_the_call_run() -> Result<(), ModuleError> {
        let executor = executor_with(Box::new(AutoApproveHandler));
        let out = executor.call("billing.refund", json!({ "order_id": "o-1" }), None, None).await?;
        assert_eq!(out, json!({ "refunded": true }));

        let preflight = executor.validate("billing.refund", &json!({ "order_id": "o-1" }), None).await?;
        assert!(preflight.requires_approval);
        Ok(())
    }
    ```

## 9. Mocking dependencies

Mock at the edge of your code, not inside apcore. Two patterns cover most cases:

- **External services** (mail, payments, HTTP APIs): pass them to the module's constructor, and pass a fake in tests. The module still runs behind the real pipeline.
- **Other modules your module calls**: register a stub module under the dependency's ID in the test client. The real executor, validation and middleware still run on the nested call.

The example below uses the first pattern.

=== "Python"

    ```python
    # my_app/notify.py
    from typing import Any, Protocol

    from pydantic import BaseModel

    from apcore import Context


    class Mailer(Protocol):
        def send(self, to: str, subject: str, body: str) -> str: ...


    class WelcomeInput(BaseModel):
        email: str


    class WelcomeOutput(BaseModel):
        message_id: str


    class SendWelcomeModule:
        input_schema = WelcomeInput
        output_schema = WelcomeOutput
        description = "Send the welcome email"

        def __init__(self, mailer: Mailer) -> None:
            self._mailer = mailer

        def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
            return {"message_id": self._mailer.send(inputs["email"], "Welcome", "Thanks for signing up.")}
    ```

    ```python
    # tests/test_notify.py
    from unittest.mock import Mock

    from apcore import APCore
    from my_app.notify import SendWelcomeModule


    def test_sends_one_welcome_email() -> None:
        mailer = Mock()
        mailer.send.return_value = "msg-1"
        client = APCore()
        client.register("notify.send_welcome", SendWelcomeModule(mailer))

        assert client.call("notify.send_welcome", {"email": "ada@example.com"}) == {"message_id": "msg-1"}
        mailer.send.assert_called_once_with("ada@example.com", "Welcome", "Thanks for signing up.")
    ```

=== "TypeScript"

    ```typescript
    // src/notify.ts
    import { Type } from '@sinclair/typebox';
    import type { Context, Module } from 'apcore-js';

    export interface Mailer {
      send(to: string, subject: string, body: string): Promise<string>;
    }

    export class SendWelcomeModule implements Module {
      inputSchema = Type.Object({ email: Type.String() });
      outputSchema = Type.Object({ message_id: Type.String() });
      description = 'Send the welcome email';

      constructor(private readonly mailer: Mailer) {}

      async execute(inputs: Record<string, unknown>, _context: Context): Promise<Record<string, unknown>> {
        const messageId = await this.mailer.send(String(inputs['email']), 'Welcome', 'Thanks for signing up.');
        return { message_id: messageId };
      }
    }
    ```

    ```typescript
    // tests/notify.test.ts
    import { expect, it, vi } from 'vitest';
    import { APCore, Registry } from 'apcore-js';
    import { SendWelcomeModule } from '../src/notify.js';

    it('sends one welcome email', async () => {
      const mailer = { send: vi.fn().mockResolvedValue('msg-1') };
      const registry = new Registry();
      await registry.register('notify.send_welcome', new SendWelcomeModule(mailer));
      const client = new APCore({ registry });

      await expect(client.call('notify.send_welcome', { email: 'ada@example.com' })).resolves.toEqual({
        message_id: 'msg-1',
      });
      expect(mailer.send).toHaveBeenCalledOnce();
      expect(mailer.send).toHaveBeenCalledWith('ada@example.com', 'Welcome', 'Thanks for signing up.');
    });
    ```

=== "Rust"

    ```rust
    // src/lib.rs of the `my_app` crate (continued)
    use std::sync::Arc;

    pub trait Mailer: Send + Sync {
        fn send(&self, to: &str, subject: &str, body: &str) -> Result<String, ModuleError>;
    }

    pub struct SendWelcomeModule {
        mailer: Arc<dyn Mailer>,
    }

    impl SendWelcomeModule {
        pub fn new(mailer: Arc<dyn Mailer>) -> Self {
            Self { mailer }
        }
    }

    #[async_trait]
    impl Module for SendWelcomeModule {
        fn input_schema(&self) -> Value {
            json!({ "type": "object", "properties": { "email": { "type": "string" } }, "required": ["email"] })
        }
        fn output_schema(&self) -> Value {
            json!({ "type": "object", "properties": { "message_id": { "type": "string" } }, "required": ["message_id"] })
        }
        fn description(&self) -> &str {
            "Send the welcome email"
        }
        async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
            let to = inputs["email"].as_str().unwrap_or_default();
            let message_id = self.mailer.send(to, "Welcome", "Thanks for signing up.")?;
            Ok(json!({ "message_id": message_id }))
        }
    }
    ```

    ```rust
    // tests/notify.rs
    use apcore::errors::ModuleError;
    use apcore::APCore;
    use my_app::{Mailer, SendWelcomeModule};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct FakeMailer {
        sent: Mutex<Vec<(String, String)>>,
    }

    impl Mailer for FakeMailer {
        fn send(&self, to: &str, subject: &str, _body: &str) -> Result<String, ModuleError> {
            let mut sent = self.sent.lock().unwrap();
            sent.push((to.to_string(), subject.to_string()));
            Ok(format!("msg-{}", sent.len()))
        }
    }

    #[tokio::test]
    async fn sends_one_welcome_email() -> Result<(), ModuleError> {
        let mailer = Arc::new(FakeMailer::default());
        let client = APCore::new();
        client.register("notify.send_welcome", Box::new(SendWelcomeModule::new(mailer.clone())))?;

        let out = client
            .call("notify.send_welcome", json!({ "email": "ada@example.com" }), None, None)
            .await?;

        assert_eq!(out, json!({ "message_id": "msg-1" }));
        assert_eq!(
            *mailer.sent.lock().unwrap(),
            vec![("ada@example.com".to_string(), "Welcome".to_string())]
        );
        Ok(())
    }
    ```

## 10. Testing ACL rules

Load the ACL file you ship and assert on its decision for each `caller_id` / `target_id` pair you care about. The pattern, in all three languages, is in [ACL Configuration: Testing ACL](./acl-configuration.md#102-testing-acl).

## Next steps

- [Middleware Guide](./middleware.md) — writing the middleware you test in §5
- [ACL Configuration](./acl-configuration.md) — access rules and how to test them
- [Approval System](../features/approval-system.md) — every source that triggers the approval gate
- [Error System](../features/error-system.md) — error codes and their fields
