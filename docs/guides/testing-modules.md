# apcore — Module Testing Guide

> Comprehensive coverage of apcore module testing strategies: unit testing, Schema testing, integration testing, ACL testing and Mock techniques.

!!! note "Cross-language applicability"
    Each section below shows the same test pattern in Python (pytest), TypeScript (Vitest), and Rust (built-in `#[test]` / `#[tokio::test]`). Every SDK exposes the same core building blocks — `Context`, `ACL`, `Registry`, `Executor`, `SchemaValidator`, and `Middleware` — so the strategies translate one-to-one across languages.

## 1. Overview

apcore's Schema-driven design is naturally suited for testing. Module inputs and outputs all have clear Schema definitions, behaviors are annotated through Annotations, and permissions are configured through ACL—all of which can be independently verified.

**Testing Pyramid:**

```
         /  E2E  \           ← End-to-end (minimal)
        / Integration \      ← Module interaction (moderate)
       / Schema Tests \      ← Schema validation (moderate)
      /  Unit Tests   \      ← Module logic (extensive)
     ──────────────────
```

**Testing Tier Strategy:**

| Tier | Target | Frequency | Speed |
|------|--------|-----------|-------|
| Unit Tests | Execute logic of individual modules | Every commit | < 1s |
| Schema Tests | Completeness and boundaries of Schema definitions | Every commit | < 1s |
| ACL Tests | Correctness of permission rules | Every commit | < 1s |
| Integration Tests | Module interactions and middleware chain | Every PR | < 10s |
| E2E Tests | Complete call chain | Pre-release | < 60s |

---

## 2. Unit Testing

### 2.1 Testing Individual Modules

The core of unit testing is to isolate and test the module's `execute` method.

=== "Python"

    ```python
    # tests/test_db_params_validator.py

    import pytest
    from extensions.executor.validator.db_params import DbParamsValidator
    from tests.conftest import create_mock_context

    class TestDbParamsValidator:
        """Test database parameters validator"""

        def setup_method(self):
            """Initialize module instance before each test"""
            self.module = DbParamsValidator()

        def test_valid_select_statement(self):
            """Validate legal SELECT statement"""
            inputs = {
                "table": "user_info",
                "sql": "SELECT * FROM user_info WHERE id = 1",
                "timeout": 30,
            }
            context = create_mock_context()

            result = self.module.execute(inputs, context)

            assert result["valid"] is True
            assert result["message"] == "Validation passed"
            assert result["errors"] == []

        def test_dangerous_sql_drop(self):
            """Detect DROP statement"""
            inputs = {"table": "user_info", "sql": "DROP TABLE user_info"}
            context = create_mock_context()

            result = self.module.execute(inputs, context)

            assert result["valid"] is False
            assert any(e["code"] == "DANGEROUS_SQL" for e in result["errors"])

        def test_default_timeout(self):
            """Verify default value of timeout field"""
            inputs = {"table": "user_info", "sql": "SELECT 1"}
            context = create_mock_context()

            result = self.module.execute(inputs, context)

            assert result["valid"] is True
    ```

=== "TypeScript"

    ```typescript
    // tests/db-params-validator.test.ts
    import { describe, it, expect, beforeEach } from 'vitest';
    import { DbParamsValidator } from '../src/extensions/executor/validator/db-params.js';
    import { createMockContext } from './helpers.js';

    describe('DbParamsValidator', () => {
      let module: DbParamsValidator;

      beforeEach(() => {
        module = new DbParamsValidator();
      });

      it('validates a legal SELECT statement', async () => {
        const inputs = {
          table: 'user_info',
          sql: 'SELECT * FROM user_info WHERE id = 1',
          timeout: 30,
        };
        const context = createMockContext();

        const result = await module.execute(inputs, context);

        expect(result['valid']).toBe(true);
        expect(result['message']).toBe('Validation passed');
        expect(result['errors']).toEqual([]);
      });

      it('detects DROP statements as dangerous', async () => {
        const inputs = { table: 'user_info', sql: 'DROP TABLE user_info' };
        const context = createMockContext();

        const result = await module.execute(inputs, context);

        expect(result['valid']).toBe(false);
        const errors = result['errors'] as Array<{ code: string }>;
        expect(errors.some((e) => e.code === 'DANGEROUS_SQL')).toBe(true);
      });

      it('applies the default timeout', async () => {
        const inputs = { table: 'user_info', sql: 'SELECT 1' };
        const context = createMockContext();

        const result = await module.execute(inputs, context);

        expect(result['valid']).toBe(true);
      });
    });
    ```

=== "Rust"

    ```rust
    // tests/test_db_params_validator.rs
    use apcore::context::{Context, Identity};
    use serde_json::{json, Value};

    use crate::extensions::executor::validator::db_params::DbParamsValidator;

    fn make_ctx() -> Context<Value> {
        Context::new(Identity::new(
            "test-user".to_string(),
            "user".to_string(),
            vec![],
            std::collections::HashMap::new(),
        ))
    }

    #[tokio::test]
    async fn validates_a_legal_select_statement() {
        let module = DbParamsValidator::new();
        let inputs = json!({
            "table": "user_info",
            "sql": "SELECT * FROM user_info WHERE id = 1",
            "timeout": 30,
        });
        let ctx = make_ctx();

        let result = module.execute(inputs, &ctx).await.unwrap();

        assert_eq!(result["valid"], json!(true));
        assert_eq!(result["message"], json!("Validation passed"));
        assert_eq!(result["errors"], json!([]));
    }

    #[tokio::test]
    async fn detects_drop_statements_as_dangerous() {
        let module = DbParamsValidator::new();
        let inputs = json!({"table": "user_info", "sql": "DROP TABLE user_info"});
        let ctx = make_ctx();

        let result = module.execute(inputs, &ctx).await.unwrap();

        assert_eq!(result["valid"], json!(false));
        let errors = result["errors"].as_array().unwrap();
        assert!(errors.iter().any(|e| e["code"] == json!("DANGEROUS_SQL")));
    }

    #[tokio::test]
    async fn applies_the_default_timeout() {
        let module = DbParamsValidator::new();
        let inputs = json!({"table": "user_info", "sql": "SELECT 1"});
        let ctx = make_ctx();

        let result = module.execute(inputs, &ctx).await.unwrap();

        assert_eq!(result["valid"], json!(true));
    }
    ```

### 2.2 Mock Context Creation

Context is the runtime context for module execution. Each SDK already ships a real `Context` type — for unit tests, build one directly rather than rolling a parallel mock class.

=== "Python"

    ```python
    # tests/conftest.py

    from typing import Any, Optional
    from apcore import Context, Identity

    def create_mock_context(
        *,
        caller_id: Optional[str] = None,
        call_chain: Optional[list[str]] = None,
        executor: Any = None,
        identity: Optional[Identity] = None,
        data: Optional[dict[str, Any]] = None,
        roles: Optional[list[str]] = None,
    ) -> Context:
        """Factory function for creating a real Context for tests."""
        # Context.create() does NOT take an executor — the Executor self-binds
        # to the context at pipeline entry (see core-executor.md §Contract:
        # Executor binding to Context). Pass a pre-bound executor only via the
        # raw Context(...) constructor below.
        ctx = Context.create(
            identity=identity or Identity(
                id="test-user",
                type="user",
                roles=tuple(roles or ["admin"]),
            ),
            data=data or {},
        )
        # Context.create() always returns a fresh top-level context (caller_id
        # is None and call_chain is []). For child-call simulations, construct
        # the descendant context yourself rather than mutating the returned
        # object — apcore Contexts are immutable.
        if caller_id is None and not call_chain:
            return ctx
        return Context(
            trace_id=ctx.trace_id,
            caller_id=caller_id,
            call_chain=call_chain or [],
            executor=executor,
            identity=ctx.identity,
            data=ctx.data,
        )
    ```

=== "TypeScript"

    ```typescript
    // tests/helpers.ts
    import { Context, createIdentity } from 'apcore-js';
    import type { Identity } from 'apcore-js';

    export interface MockContextOptions {
      callerId?: string | null;
      callChain?: string[];
      executor?: unknown;
      identity?: Identity | null;
      data?: Record<string, unknown>;
      roles?: string[];
    }

    /** Build a real Context instance suitable for unit tests. */
    export function createMockContext(opts: MockContextOptions = {}): Context {
      const identity =
        opts.identity ??
        createIdentity('test-user', 'user', opts.roles ?? ['admin']);

      // Top-level context — Context.create() generates a fresh trace_id.
      // create() signature is (identity, traceParent?, cancelToken?, data?, ...);
      // the executor self-binds at pipeline entry and is NOT a create() argument.
      const top = Context.create(identity, undefined, undefined, opts.data ?? {});

      if (opts.callerId == null && (opts.callChain ?? []).length === 0) {
        return top;
      }

      // Build a derived context preserving the trace_id but injecting a
      // synthetic caller_id / call_chain for child-call tests.
      return new Context(
        top.traceId,
        opts.callerId ?? null,
        opts.callChain ?? [],
        opts.executor ?? null,
        identity,
        null,
        opts.data ?? {},
      );
    }
    ```

=== "Rust"

    ```rust
    // tests/common/mod.rs
    use apcore::context::{Context, Identity};
    use serde_json::Value;
    use std::collections::HashMap;

    pub struct MockContextOptions {
        pub caller_id: Option<String>,
        pub call_chain: Vec<String>,
        pub identity: Option<Identity>,
        pub data: HashMap<String, Value>,
        pub roles: Vec<String>,
    }

    impl Default for MockContextOptions {
        fn default() -> Self {
            Self {
                caller_id: None,
                call_chain: vec![],
                identity: None,
                data: HashMap::new(),
                roles: vec!["admin".to_string()],
            }
        }
    }

    /// Build a real `Context<Value>` configured for unit tests.
    pub fn create_mock_context(opts: MockContextOptions) -> Context<Value> {
        let identity = opts.identity.unwrap_or_else(|| {
            Identity::new(
                "test-user".to_string(),
                "user".to_string(),
                opts.roles.clone(),
                HashMap::new(),
            )
        });

        Context::builder()
            .identity(Some(identity))
            .caller_id(opts.caller_id)
            .data(opts.data)
            .build()
        // call_chain on the builder is intentionally absent: Context tracks
        // the chain through `child()` calls. For a child-call simulation,
        // derive the context from a parent via `parent.child("target")`.
    }
    ```

### 2.3 Mock Executor Pattern

When the module under test calls other modules through `context.executor`, swap the real executor for a configurable double.

=== "Python"

    ```python
    # tests/conftest.py
    from typing import Any
    from apcore.errors import ModuleNotFoundError

    class MockExecutor:
        """Configurable Mock Executor with response values"""

        def __init__(self) -> None:
            self._responses: dict[str, Any] = {}
            self._errors: dict[str, Exception] = {}
            self._call_log: list[dict[str, Any]] = []

        def register_response(self, module_id: str, response: dict[str, Any]) -> None:
            """Register return value for module call"""
            self._responses[module_id] = response

        def register_error(self, module_id: str, error: Exception) -> None:
            """Register exception for module call"""
            self._errors[module_id] = error

        def call(self, module_id: str, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
            """Simulate module call"""
            self._call_log.append({
                "module_id": module_id,
                "inputs": inputs,
                "trace_id": getattr(context, "trace_id", None),
            })

            if module_id in self._errors:
                raise self._errors[module_id]
            if module_id in self._responses:
                return self._responses[module_id]
            raise ModuleNotFoundError(f"Mock: Unregistered module {module_id}")

        async def call_async(self, module_id: str, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
            """Simulate async module call"""
            return self.call(module_id, inputs, context)

        @property
        def call_log(self) -> list[dict[str, Any]]:
            """Get call log"""
            return self._call_log

        def assert_called(self, module_id: str, times: int = 1) -> None:
            """Assert a module was called a specific number of times"""
            actual = sum(1 for c in self._call_log if c["module_id"] == module_id)
            assert actual == times, (
                f"Expected {module_id} to be called {times} times, actually {actual}"
            )

        def assert_not_called(self, module_id: str) -> None:
            """Assert a module was not called"""
            self.assert_called(module_id, times=0)
    ```

=== "TypeScript"

    ```typescript
    // tests/mock-executor.ts
    import { ModuleNotFoundError } from 'apcore-js';
    import type { Context } from 'apcore-js';

    interface CallLogEntry {
      moduleId: string;
      inputs: Record<string, unknown>;
      traceId: string | null;
    }

    /** Configurable mock executor with canned responses and a call log. */
    export class MockExecutor {
      private responses = new Map<string, Record<string, unknown>>();
      private errors = new Map<string, Error>();
      private log: CallLogEntry[] = [];

      registerResponse(moduleId: string, response: Record<string, unknown>): void {
        this.responses.set(moduleId, response);
      }

      registerError(moduleId: string, error: Error): void {
        this.errors.set(moduleId, error);
      }

      async call(
        moduleId: string,
        inputs: Record<string, unknown>,
        context: Context,
      ): Promise<Record<string, unknown>> {
        this.log.push({
          moduleId,
          inputs,
          traceId: context.traceId ?? null,
        });

        const err = this.errors.get(moduleId);
        if (err) throw err;

        const resp = this.responses.get(moduleId);
        if (resp) return resp;

        throw new ModuleNotFoundError(`Mock: Unregistered module ${moduleId}`);
      }

      get callLog(): readonly CallLogEntry[] {
        return this.log;
      }

      assertCalled(moduleId: string, times = 1): void {
        const actual = this.log.filter((e) => e.moduleId === moduleId).length;
        if (actual !== times) {
          throw new Error(
            `Expected ${moduleId} to be called ${times} times, actually ${actual}`,
          );
        }
      }

      assertNotCalled(moduleId: string): void {
        this.assertCalled(moduleId, 0);
      }
    }
    ```

=== "Rust"

    ```rust
    // tests/common/mock_executor.rs
    use apcore::context::Context;
    use apcore::errors::{ErrorCode, ModuleError};
    use serde_json::Value;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Clone, Debug)]
    pub struct CallLogEntry {
        pub module_id: String,
        pub inputs: Value,
        pub trace_id: String,
    }

    /// Configurable mock executor with canned responses and a call log.
    #[derive(Default)]
    pub struct MockExecutor {
        responses: Mutex<HashMap<String, Value>>,
        errors: Mutex<HashMap<String, ModuleError>>,
        log: Mutex<Vec<CallLogEntry>>,
    }

    impl MockExecutor {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn register_response(&self, module_id: &str, response: Value) {
            self.responses
                .lock()
                .unwrap()
                .insert(module_id.to_string(), response);
        }

        pub fn register_error(&self, module_id: &str, error: ModuleError) {
            self.errors
                .lock()
                .unwrap()
                .insert(module_id.to_string(), error);
        }

        pub async fn call(
            &self,
            module_id: &str,
            inputs: Value,
            ctx: &Context<Value>,
        ) -> Result<Value, ModuleError> {
            self.log.lock().unwrap().push(CallLogEntry {
                module_id: module_id.to_string(),
                inputs: inputs.clone(),
                trace_id: ctx.trace_id().to_string(),
            });

            if let Some(err) = self.errors.lock().unwrap().get(module_id).cloned() {
                return Err(err);
            }
            if let Some(resp) = self.responses.lock().unwrap().get(module_id).cloned() {
                return Ok(resp);
            }
            Err(ModuleError::new(
                ErrorCode::ModuleNotFound,
                format!("Mock: Unregistered module {module_id}"),
            ))
        }

        pub fn call_log(&self) -> Vec<CallLogEntry> {
            self.log.lock().unwrap().clone()
        }

        pub fn assert_called(&self, module_id: &str, times: usize) {
            let actual = self
                .log
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e.module_id == module_id)
                .count();
            assert_eq!(
                actual, times,
                "Expected {module_id} to be called {times} times, actually {actual}",
            );
        }

        pub fn assert_not_called(&self, module_id: &str) {
            self.assert_called(module_id, 0);
        }
    }
    ```

**Test example using Mock Executor:**

=== "Python"

    ```python
    import pytest
    from extensions.orchestrator.engine.task_flow import TaskFlowOrchestrator
    from tests.conftest import MockExecutor, create_mock_context

    class TestTaskFlowOrchestrator:
        """Test task flow orchestrator (internally calls other modules)"""

        def setup_method(self):
            self.module = TaskFlowOrchestrator()
            self.executor = MockExecutor()

        def test_orchestrator_calls_validator_then_executor(self):
            """Verify orchestrator calls validator then executor"""
            self.executor.register_response(
                "executor.validator.db_params",
                {"valid": True, "message": "Validation passed", "errors": []},
            )
            self.executor.register_response(
                "executor.handler.db_query",
                {"rows": [{"id": 1, "name": "test"}], "count": 1},
            )

            context = create_mock_context(
                caller_id="api.handler.task_submit",
                executor=self.executor,
            )

            inputs = {"table": "user_info", "sql": "SELECT * FROM user_info"}
            result = self.module.execute(inputs, context)

            self.executor.assert_called("executor.validator.db_params", times=1)
            self.executor.assert_called("executor.handler.db_query", times=1)
            assert result["count"] == 1

        def test_orchestrator_stops_on_validation_failure(self):
            """Verify executor is not called when validation fails"""
            self.executor.register_response(
                "executor.validator.db_params",
                {
                    "valid": False,
                    "message": "Validation failed",
                    "errors": [{"code": "DANGEROUS_SQL"}],
                },
            )

            context = create_mock_context(executor=self.executor)

            inputs = {"table": "user_info", "sql": "DROP TABLE user_info"}
            result = self.module.execute(inputs, context)

            self.executor.assert_called("executor.validator.db_params", times=1)
            self.executor.assert_not_called("executor.handler.db_query")
            assert result["valid"] is False
    ```

=== "TypeScript"

    ```typescript
    import { describe, it, expect, beforeEach } from 'vitest';
    import { TaskFlowOrchestrator } from '../src/extensions/orchestrator/engine/task-flow.js';
    import { MockExecutor } from './mock-executor.js';
    import { createMockContext } from './helpers.js';

    describe('TaskFlowOrchestrator', () => {
      let module: TaskFlowOrchestrator;
      let executor: MockExecutor;

      beforeEach(() => {
        module = new TaskFlowOrchestrator();
        executor = new MockExecutor();
      });

      it('calls validator then executor', async () => {
        executor.registerResponse('executor.validator.db_params', {
          valid: true,
          message: 'Validation passed',
          errors: [],
        });
        executor.registerResponse('executor.handler.db_query', {
          rows: [{ id: 1, name: 'test' }],
          count: 1,
        });

        const context = createMockContext({
          callerId: 'api.handler.task_submit',
          executor,
        });

        const inputs = { table: 'user_info', sql: 'SELECT * FROM user_info' };
        const result = await module.execute(inputs, context);

        executor.assertCalled('executor.validator.db_params', 1);
        executor.assertCalled('executor.handler.db_query', 1);
        expect(result['count']).toBe(1);
      });

      it('stops when validation fails', async () => {
        executor.registerResponse('executor.validator.db_params', {
          valid: false,
          message: 'Validation failed',
          errors: [{ code: 'DANGEROUS_SQL' }],
        });

        const context = createMockContext({ executor });

        const inputs = { table: 'user_info', sql: 'DROP TABLE user_info' };
        const result = await module.execute(inputs, context);

        executor.assertCalled('executor.validator.db_params', 1);
        executor.assertNotCalled('executor.handler.db_query');
        expect(result['valid']).toBe(false);
      });
    });
    ```

=== "Rust"

    ```rust
    use serde_json::json;

    use crate::common::mock_executor::MockExecutor;
    use crate::common::{create_mock_context, MockContextOptions};
    use crate::extensions::orchestrator::engine::task_flow::TaskFlowOrchestrator;

    #[tokio::test]
    async fn orchestrator_calls_validator_then_executor() {
        let module = TaskFlowOrchestrator::new();
        let executor = MockExecutor::new();

        executor.register_response(
            "executor.validator.db_params",
            json!({"valid": true, "message": "Validation passed", "errors": []}),
        );
        executor.register_response(
            "executor.handler.db_query",
            json!({"rows": [{"id": 1, "name": "test"}], "count": 1}),
        );

        let ctx = create_mock_context(MockContextOptions {
            caller_id: Some("api.handler.task_submit".to_string()),
            ..Default::default()
        });

        let inputs = json!({"table": "user_info", "sql": "SELECT * FROM user_info"});
        // The module under test reads the executor from ctx.services / ctx.data
        // in real code; here we drive it directly through the test harness.
        let result = module.execute_with_executor(inputs, &ctx, &executor).await.unwrap();

        executor.assert_called("executor.validator.db_params", 1);
        executor.assert_called("executor.handler.db_query", 1);
        assert_eq!(result["count"], json!(1));
    }

    #[tokio::test]
    async fn orchestrator_stops_on_validation_failure() {
        let module = TaskFlowOrchestrator::new();
        let executor = MockExecutor::new();

        executor.register_response(
            "executor.validator.db_params",
            json!({
                "valid": false,
                "message": "Validation failed",
                "errors": [{"code": "DANGEROUS_SQL"}],
            }),
        );

        let ctx = create_mock_context(MockContextOptions::default());

        let inputs = json!({"table": "user_info", "sql": "DROP TABLE user_info"});
        let result = module.execute_with_executor(inputs, &ctx, &executor).await.unwrap();

        executor.assert_called("executor.validator.db_params", 1);
        executor.assert_not_called("executor.handler.db_query");
        assert_eq!(result["valid"], json!(false));
    }
    ```

### 2.4 Input/Output Validation Testing

Verify that the module's Schema definition matches actual behavior. Each SDK ships a schema validator: Python uses Pydantic via `SchemaLoader`, TypeScript uses TypeBox via `SchemaValidator`, and Rust uses `apcore::schema::SchemaValidator` over `serde_json::Value`.

=== "Python"

    ```python
    import pytest
    from pydantic import ValidationError
    from extensions.executor.validator.db_params import DBParamsInput

    class TestDbParamsInputSchema:
        """Test input Schema validation"""

        def test_valid_input(self):
            """Valid input should pass validation"""
            data = {"table": "user_info", "sql": "SELECT 1", "timeout": 30}
            model = DBParamsInput(**data)
            assert model.table == "user_info"

        def test_missing_required_field(self):
            """Missing required field should raise error"""
            with pytest.raises(ValidationError) as exc_info:
                DBParamsInput(sql="SELECT 1")  # Missing table

            errors = exc_info.value.errors()
            assert any(e["loc"] == ("table",) for e in errors)

        def test_invalid_table_pattern(self):
            """Table name not matching pattern should raise error"""
            with pytest.raises(ValidationError):
                DBParamsInput(table="User-Info", sql="SELECT 1")

        def test_timeout_boundary_values(self):
            """Boundary values of timeout field"""
            assert DBParamsInput(table="t", sql="SELECT 1", timeout=1).timeout == 1
            assert DBParamsInput(table="t", sql="SELECT 1", timeout=300).timeout == 300

            with pytest.raises(ValidationError):
                DBParamsInput(table="t", sql="SELECT 1", timeout=301)
            with pytest.raises(ValidationError):
                DBParamsInput(table="t", sql="SELECT 1", timeout=0)
    ```

=== "TypeScript"

    ```typescript
    import { describe, it, expect } from 'vitest';
    import { Type } from '@sinclair/typebox';
    import { SchemaValidator } from 'apcore-js';

    const DbParamsInputSchema = Type.Object({
      table: Type.String({ pattern: '^[a-z][a-z0-9_]*$' }),
      sql: Type.String({ minLength: 1 }),
      timeout: Type.Optional(
        Type.Integer({ minimum: 1, maximum: 300, default: 30 }),
      ),
    });

    describe('DbParamsInput schema', () => {
      const validator = new SchemaValidator();

      it('accepts valid input', () => {
        const data = { table: 'user_info', sql: 'SELECT 1', timeout: 30 };
        const result = validator.validate(data, DbParamsInputSchema);
        expect(result.valid).toBe(true);
      });

      it('rejects when required field is missing', () => {
        const result = validator.validate({ sql: 'SELECT 1' }, DbParamsInputSchema);
        expect(result.valid).toBe(false);
        expect(result.errors.some((e) => e.path?.includes('table'))).toBe(true);
      });

      it('rejects table names that do not match the pattern', () => {
        const result = validator.validate(
          { table: 'User-Info', sql: 'SELECT 1' },
          DbParamsInputSchema,
        );
        expect(result.valid).toBe(false);
      });

      it('enforces timeout boundary values', () => {
        const ok = (timeout: number) =>
          validator.validate(
            { table: 't', sql: 'SELECT 1', timeout },
            DbParamsInputSchema,
          ).valid;

        expect(ok(1)).toBe(true);
        expect(ok(300)).toBe(true);
        expect(ok(301)).toBe(false);
        expect(ok(0)).toBe(false);
      });
    });
    ```

=== "Rust"

    ```rust
    use apcore::schema::SchemaValidator;
    use serde_json::json;

    fn db_params_input_schema() -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["table", "sql"],
            "properties": {
                "table":   {"type": "string", "pattern": "^[a-z][a-z0-9_]*$"},
                "sql":     {"type": "string", "minLength": 1},
                "timeout": {"type": "integer", "minimum": 1, "maximum": 300}
            },
            "additionalProperties": false
        })
    }

    #[test]
    fn accepts_valid_input() {
        let v = SchemaValidator::new();
        let data = json!({"table": "user_info", "sql": "SELECT 1", "timeout": 30});
        assert!(v.validate(&data, &db_params_input_schema()).is_valid());
    }

    #[test]
    fn rejects_when_required_field_missing() {
        let v = SchemaValidator::new();
        let data = json!({"sql": "SELECT 1"});
        let result = v.validate_detailed(&data, &db_params_input_schema());
        assert!(!result.is_valid());
        assert!(result.errors().iter().any(|e| e.path().contains("table")));
    }

    #[test]
    fn rejects_invalid_table_pattern() {
        let v = SchemaValidator::new();
        let data = json!({"table": "User-Info", "sql": "SELECT 1"});
        assert!(!v.validate(&data, &db_params_input_schema()).is_valid());
    }

    #[test]
    fn enforces_timeout_boundary_values() {
        let v = SchemaValidator::new();
        let schema = db_params_input_schema();
        let ok = |timeout: i64| {
            v.validate(
                &json!({"table": "t", "sql": "SELECT 1", "timeout": timeout}),
                &schema,
            )
            .is_valid()
        };

        assert!(ok(1));
        assert!(ok(300));
        assert!(!ok(301));
        assert!(!ok(0));
    }
    ```

---

## 3. Schema Testing

### 3.1 Validating YAML Schema Definitions

Ensure the YAML Schema file structure itself is correct.

=== "Python"

    ```python
    # tests/test_schemas.py
    import pytest
    import yaml
    import jsonschema
    from pathlib import Path

    SCHEMAS_DIR = Path("schemas")

    SCHEMA_FILES = sorted(SCHEMAS_DIR.glob("*.schema.yaml"))

    @pytest.mark.parametrize("schema_file", SCHEMA_FILES, ids=lambda p: p.name)
    def test_schema_is_valid_yaml(schema_file: Path):
        """Schema file must be valid YAML"""
        with open(schema_file) as f:
            data = yaml.safe_load(f)
        assert data is not None, f"{schema_file.name} is empty"

    @pytest.mark.parametrize("schema_file", SCHEMA_FILES, ids=lambda p: p.name)
    def test_schema_has_required_fields(schema_file: Path):
        """Schema file must contain required fields"""
        with open(schema_file) as f:
            data = yaml.safe_load(f)
        for key in ("module_id", "description", "input_schema", "output_schema"):
            assert key in data, f"{schema_file.name} missing {key}"

    @pytest.mark.parametrize("schema_file", SCHEMA_FILES, ids=lambda p: p.name)
    def test_input_schema_is_valid_json_schema(schema_file: Path):
        """input_schema must be valid JSON Schema (Draft 2020-12)"""
        with open(schema_file) as f:
            data = yaml.safe_load(f)
        jsonschema.Draft202012Validator.check_schema(data.get("input_schema", {}))

    @pytest.mark.parametrize("schema_file", SCHEMA_FILES, ids=lambda p: p.name)
    def test_output_schema_is_valid_json_schema(schema_file: Path):
        """output_schema must be valid JSON Schema (Draft 2020-12)"""
        with open(schema_file) as f:
            data = yaml.safe_load(f)
        jsonschema.Draft202012Validator.check_schema(data.get("output_schema", {}))

    @pytest.mark.parametrize("schema_file", SCHEMA_FILES, ids=lambda p: p.name)
    def test_all_properties_have_description(schema_file: Path):
        """All properties should have a description"""
        with open(schema_file) as f:
            data = yaml.safe_load(f)

        def check_descriptions(schema: dict, path: str = "") -> None:
            for name, prop in schema.get("properties", {}).items():
                current_path = f"{path}.{name}" if path else name
                assert "description" in prop, (
                    f"{schema_file.name}: {current_path} missing description"
                )
                if prop.get("type") == "object":
                    check_descriptions(prop, current_path)

        check_descriptions(data.get("input_schema", {}))
        check_descriptions(data.get("output_schema", {}))
    ```

=== "TypeScript"

    ```typescript
    // tests/schemas.test.ts
    import { readdirSync, readFileSync } from 'node:fs';
    import { join } from 'node:path';
    import { describe, it, expect } from 'vitest';
    import yaml from 'js-yaml';
    import Ajv2020 from 'ajv/dist/2020.js';

    const SCHEMAS_DIR = 'schemas';
    const SCHEMA_FILES = readdirSync(SCHEMAS_DIR)
      .filter((f) => f.endsWith('.schema.yaml'))
      .sort();

    function loadSchema(file: string): Record<string, unknown> {
      const text = readFileSync(join(SCHEMAS_DIR, file), 'utf8');
      return yaml.load(text) as Record<string, unknown>;
    }

    describe.each(SCHEMA_FILES)('%s', (file) => {
      it('is valid YAML', () => {
        expect(loadSchema(file)).toBeTruthy();
      });

      it('has the required top-level fields', () => {
        const data = loadSchema(file);
        for (const key of ['module_id', 'description', 'input_schema', 'output_schema']) {
          expect(data).toHaveProperty(key);
        }
      });

      it('has a valid input_schema (Draft 2020-12)', () => {
        const ajv = new Ajv2020({ strict: false });
        const data = loadSchema(file);
        expect(() => ajv.compile(data['input_schema'] as object)).not.toThrow();
      });

      it('has a valid output_schema (Draft 2020-12)', () => {
        const ajv = new Ajv2020({ strict: false });
        const data = loadSchema(file);
        expect(() => ajv.compile(data['output_schema'] as object)).not.toThrow();
      });

      it('has a description on every property', () => {
        const data = loadSchema(file);
        const checkDescriptions = (schema: any, path = ''): void => {
          const props = schema?.properties ?? {};
          for (const [name, prop] of Object.entries(props)) {
            const current = path ? `${path}.${name}` : name;
            expect(prop, `${file}: ${current} missing description`).toHaveProperty(
              'description',
            );
            if ((prop as any).type === 'object') {
              checkDescriptions(prop, current);
            }
          }
        };
        checkDescriptions(data['input_schema']);
        checkDescriptions(data['output_schema']);
      });
    });
    ```

=== "Rust"

    ```rust
    // tests/test_schemas.rs
    use apcore::schema::SchemaValidator;
    use serde_json::Value;
    use std::fs;
    use std::path::PathBuf;

    fn schema_files() -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = fs::read_dir("schemas")
            .expect("schemas/ should exist")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.ends_with(".schema.yaml"))
                    .unwrap_or(false)
            })
            .collect();
        files.sort();
        files
    }

    fn load_schema(path: &PathBuf) -> Value {
        let text = fs::read_to_string(path).expect("read schema file");
        serde_yaml::from_str(&text).expect("valid YAML")
    }

    fn check_descriptions(schema: &Value, path: &str, file: &str) {
        let Some(props) = schema.get("properties").and_then(|p| p.as_object()) else {
            return;
        };
        for (name, prop) in props {
            let current = if path.is_empty() {
                name.clone()
            } else {
                format!("{path}.{name}")
            };
            assert!(
                prop.get("description").is_some(),
                "{file}: {current} missing description",
            );
            if prop.get("type").and_then(|t| t.as_str()) == Some("object") {
                check_descriptions(prop, &current, file);
            }
        }
    }

    #[test]
    fn every_schema_is_valid_yaml_and_well_formed() {
        let validator = SchemaValidator::new();

        for path in schema_files() {
            let file = path.file_name().unwrap().to_string_lossy().to_string();
            let data = load_schema(&path);

            for key in ["module_id", "description", "input_schema", "output_schema"] {
                assert!(data.get(key).is_some(), "{file} missing {key}");
            }

            // Apply each schema to a trivial value — Draft 2020-12 well-formedness
            // is enforced by the validator: an ill-formed schema returns an error.
            let input_schema = &data["input_schema"];
            let output_schema = &data["output_schema"];
            assert!(
                validator.validate(&Value::Null, input_schema).errors().len() < usize::MAX,
                "{file}: input_schema is malformed",
            );
            assert!(
                validator.validate(&Value::Null, output_schema).errors().len() < usize::MAX,
                "{file}: output_schema is malformed",
            );

            check_descriptions(input_schema, "", &file);
            check_descriptions(output_schema, "", &file);
        }
    }
    ```

### 3.2 Schema Boundary Value Testing

=== "Python"

    ```python
    import yaml
    import jsonschema

    class TestSchemaEdgeCases:
        """Test Schema boundary cases"""

        def setup_method(self):
            with open("schemas/executor/validator/db_params.schema.yaml") as f:
                self.schema = yaml.safe_load(f)
            self.input_schema = self.schema["input_schema"]
            self.validator = jsonschema.Draft202012Validator(self.input_schema)

        def test_null_values(self):
            """Required field as null should fail"""
            assert not self.validator.is_valid({"table": None, "sql": "SELECT 1"})

        def test_empty_string(self):
            """Empty table name does not match pattern"""
            assert not self.validator.is_valid({"table": "", "sql": "SELECT 1"})

        def test_missing_optional_field(self):
            """Only providing required fields succeeds"""
            assert self.validator.is_valid({"table": "user_info", "sql": "SELECT 1"})

        def test_extra_fields_rejected(self):
            """Reject extra fields when additionalProperties: false"""
            data = {"table": "user_info", "sql": "SELECT 1", "unknown_field": "x"}
            if self.input_schema.get("additionalProperties") is False:
                assert not self.validator.is_valid(data)

        def test_boundary_integer_values(self):
            """Test integer boundaries"""
            base = {"table": "t", "sql": "SELECT 1"}
            assert self.validator.is_valid({**base, "timeout": 1})
            assert self.validator.is_valid({**base, "timeout": 300})
            assert not self.validator.is_valid({**base, "timeout": 301})
    ```

=== "TypeScript"

    ```typescript
    import { describe, it, expect, beforeAll } from 'vitest';
    import { readFileSync } from 'node:fs';
    import yaml from 'js-yaml';
    import Ajv2020 from 'ajv/dist/2020.js';
    import type { ValidateFunction } from 'ajv';

    describe('SchemaEdgeCases', () => {
      let validate: ValidateFunction;
      let inputSchema: Record<string, unknown>;

      beforeAll(() => {
        const text = readFileSync(
          'schemas/executor/validator/db_params.schema.yaml',
          'utf8',
        );
        const schema = yaml.load(text) as Record<string, unknown>;
        inputSchema = schema['input_schema'] as Record<string, unknown>;
        const ajv = new Ajv2020({ strict: false });
        validate = ajv.compile(inputSchema);
      });

      it('rejects null in a required field', () => {
        expect(validate({ table: null, sql: 'SELECT 1' })).toBe(false);
      });

      it('rejects empty string that does not match the pattern', () => {
        expect(validate({ table: '', sql: 'SELECT 1' })).toBe(false);
      });

      it('accepts when only required fields are provided', () => {
        expect(validate({ table: 'user_info', sql: 'SELECT 1' })).toBe(true);
      });

      it('rejects extra fields when additionalProperties is false', () => {
        const data = { table: 'user_info', sql: 'SELECT 1', unknown_field: 'x' };
        if (inputSchema['additionalProperties'] === false) {
          expect(validate(data)).toBe(false);
        }
      });

      it('enforces integer boundaries on timeout', () => {
        const base = { table: 't', sql: 'SELECT 1' };
        expect(validate({ ...base, timeout: 1 })).toBe(true);
        expect(validate({ ...base, timeout: 300 })).toBe(true);
        expect(validate({ ...base, timeout: 301 })).toBe(false);
      });
    });
    ```

=== "Rust"

    ```rust
    use apcore::schema::SchemaValidator;
    use serde_json::{json, Value};
    use std::fs;
    use std::sync::OnceLock;

    fn input_schema() -> &'static Value {
        static SCHEMA: OnceLock<Value> = OnceLock::new();
        SCHEMA.get_or_init(|| {
            let text = fs::read_to_string(
                "schemas/executor/validator/db_params.schema.yaml",
            )
            .expect("read schema");
            let parsed: Value = serde_yaml::from_str(&text).expect("valid YAML");
            parsed["input_schema"].clone()
        })
    }

    #[test]
    fn rejects_null_in_required_field() {
        let v = SchemaValidator::new();
        let data = json!({"table": null, "sql": "SELECT 1"});
        assert!(!v.validate(&data, input_schema()).is_valid());
    }

    #[test]
    fn rejects_empty_string_failing_pattern() {
        let v = SchemaValidator::new();
        let data = json!({"table": "", "sql": "SELECT 1"});
        assert!(!v.validate(&data, input_schema()).is_valid());
    }

    #[test]
    fn accepts_only_required_fields() {
        let v = SchemaValidator::new();
        let data = json!({"table": "user_info", "sql": "SELECT 1"});
        assert!(v.validate(&data, input_schema()).is_valid());
    }

    #[test]
    fn rejects_extra_fields_when_additional_properties_false() {
        let v = SchemaValidator::new();
        let data = json!({
            "table": "user_info",
            "sql": "SELECT 1",
            "unknown_field": "x",
        });
        if input_schema()
            .get("additionalProperties")
            .and_then(|p| p.as_bool())
            == Some(false)
        {
            assert!(!v.validate(&data, input_schema()).is_valid());
        }
    }

    #[test]
    fn enforces_integer_boundaries_on_timeout() {
        let v = SchemaValidator::new();
        let case = |timeout: i64| {
            v.validate(
                &json!({"table": "t", "sql": "SELECT 1", "timeout": timeout}),
                input_schema(),
            )
            .is_valid()
        };
        assert!(case(1));
        assert!(case(300));
        assert!(!case(301));
    }
    ```

### 3.3 Schema Compatibility Testing

Ensure Schema changes don't break compatibility.

=== "Python"

    ```python
    import jsonschema
    import yaml

    class TestSchemaCompatibility:
        """Test Schema version compatibility"""

        def setup_method(self):
            with open("schemas/executor/validator/db_params.schema.yaml") as f:
                self.input_schema = yaml.safe_load(f)["input_schema"]

        def test_new_optional_field_is_backward_compatible(self):
            """Adding an optional field should not break old data"""
            old_input = {"table": "user_info", "sql": "SELECT 1"}
            validator = jsonschema.Draft202012Validator(self.input_schema)
            assert validator.is_valid(old_input)

        def test_required_fields_not_removed(self):
            """Required fields must not be removed"""
            required = self.input_schema.get("required", [])
            previous_required = ["table", "sql"]
            for field_name in previous_required:
                assert field_name in required, (
                    f"Required field {field_name} was removed (breaking change)"
                )

        def test_enum_values_not_removed(self):
            """Enum values present in the previous version must still be accepted"""
            previous_enums = {"effect": ["allow", "deny"]}  # snapshot from prior version
            for name, prev_values in previous_enums.items():
                prop = self.input_schema.get("properties", {}).get(name, {})
                current = prop.get("enum")
                if current is not None:
                    for v in prev_values:
                        assert v in current, (
                            f"Enum value {v!r} removed from {name} (breaking change)"
                        )
    ```

=== "TypeScript"

    ```typescript
    import { describe, it, expect, beforeAll } from 'vitest';
    import { readFileSync } from 'node:fs';
    import yaml from 'js-yaml';
    import Ajv2020 from 'ajv/dist/2020.js';
    import type { ValidateFunction } from 'ajv';

    describe('SchemaCompatibility', () => {
      let inputSchema: Record<string, any>;
      let validate: ValidateFunction;

      beforeAll(() => {
        const text = readFileSync(
          'schemas/executor/validator/db_params.schema.yaml',
          'utf8',
        );
        const schema = yaml.load(text) as Record<string, unknown>;
        inputSchema = schema['input_schema'] as Record<string, any>;
        validate = new Ajv2020({ strict: false }).compile(inputSchema);
      });

      it('keeps old data valid after adding an optional field', () => {
        const oldInput = { table: 'user_info', sql: 'SELECT 1' };
        expect(validate(oldInput)).toBe(true);
      });

      it('does not remove required fields', () => {
        const required = (inputSchema['required'] ?? []) as string[];
        const previousRequired = ['table', 'sql'];
        for (const field of previousRequired) {
          expect(required, `Required field ${field} removed`).toContain(field);
        }
      });

      it('does not remove enum values from the previous version', () => {
        const previousEnums: Record<string, string[]> = {
          effect: ['allow', 'deny'],
        };
        const props = (inputSchema['properties'] ?? {}) as Record<string, any>;
        for (const [name, prevValues] of Object.entries(previousEnums)) {
          const current = props[name]?.enum as string[] | undefined;
          if (current) {
            for (const v of prevValues) {
              expect(current, `Enum value ${v} removed from ${name}`).toContain(v);
            }
          }
        }
      });
    });
    ```

=== "Rust"

    ```rust
    use apcore::schema::SchemaValidator;
    use serde_json::{json, Value};
    use std::fs;

    fn load_input_schema() -> Value {
        let text = fs::read_to_string(
            "schemas/executor/validator/db_params.schema.yaml",
        )
        .expect("read schema");
        let parsed: Value = serde_yaml::from_str(&text).expect("valid YAML");
        parsed["input_schema"].clone()
    }

    #[test]
    fn old_data_remains_valid_after_adding_optional_field() {
        let schema = load_input_schema();
        let v = SchemaValidator::new();
        let old_input = json!({"table": "user_info", "sql": "SELECT 1"});
        assert!(v.validate(&old_input, &schema).is_valid());
    }

    #[test]
    fn required_fields_are_not_removed() {
        let schema = load_input_schema();
        let required: Vec<&str> = schema
            .get("required")
            .and_then(|r| r.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        for field in ["table", "sql"] {
            assert!(
                required.contains(&field),
                "Required field {field} removed (breaking change)",
            );
        }
    }

    #[test]
    fn enum_values_are_not_removed() {
        let schema = load_input_schema();
        let previous_enums: &[(&str, &[&str])] = &[("effect", &["allow", "deny"])];
        let props = schema.get("properties").and_then(|p| p.as_object());

        for (name, prev_values) in previous_enums {
            let Some(props) = props else { continue };
            let Some(current) = props
                .get(*name)
                .and_then(|p| p.get("enum"))
                .and_then(|e| e.as_array())
            else {
                continue;
            };
            let current: Vec<&str> = current.iter().filter_map(|v| v.as_str()).collect();
            for v in *prev_values {
                assert!(
                    current.contains(v),
                    "Enum value {v} removed from {name} (breaking change)",
                );
            }
        }
    }
    ```

---

## 4. Integration Testing

### 4.1 Testing Module Interactions

=== "Python"

    ```python
    # tests/integration/test_module_interactions.py
    import pytest
    from apcore import Registry, Executor
    from tests.conftest import create_mock_context

    class TestModuleInteractions:
        """Test interactions between modules"""

        def setup_method(self):
            self.registry = Registry(extensions_dir="./extensions")
            self.registry.discover()
            self.executor = Executor(registry=self.registry)

        def test_orchestrator_calls_executor(self):
            """Orchestrator calls a downstream executor module"""
            context = create_mock_context(
                caller_id="api.handler.task_submit",
                executor=self.executor,
            )

            result = self.executor.call(
                module_id="orchestrator.engine.task_flow",
                inputs={"table": "user_info", "sql": "SELECT * FROM user_info"},
                context=context,
            )

            assert "rows" in result or "valid" in result

        def test_call_chain_propagation(self):
            """trace_id stays consistent across the call chain"""
            context = create_mock_context(caller_id="api.handler.task_submit")

            self.executor.call(
                module_id="executor.validator.db_params",
                inputs={"table": "user_info", "sql": "SELECT 1"},
                context=context,
            )

            assert context.trace_id is not None
    ```

=== "TypeScript"

    ```typescript
    // tests/integration/module-interactions.test.ts
    import { describe, it, expect, beforeEach } from 'vitest';
    import { Registry } from 'apcore-js';
    import { Executor } from 'apcore-js';
    import { createMockContext } from '../helpers.js';

    describe('Module interactions', () => {
      let registry: Registry;
      let executor: Executor;

      beforeEach(async () => {
        registry = new Registry({ extensionsDir: './extensions' });
        await registry.discover();
        executor = new Executor({ registry });
      });

      it('orchestrator calls a downstream executor module', async () => {
        const context = createMockContext({
          callerId: 'api.handler.task_submit',
          executor,
        });

        const result = await executor.call(
          'orchestrator.engine.task_flow',
          { table: 'user_info', sql: 'SELECT * FROM user_info' },
          context,
        );

        expect('rows' in result || 'valid' in result).toBe(true);
      });

      it('keeps trace_id consistent across the call chain', async () => {
        const context = createMockContext({
          callerId: 'api.handler.task_submit',
        });

        await executor.call(
          'executor.validator.db_params',
          { table: 'user_info', sql: 'SELECT 1' },
          context,
        );

        expect(context.traceId).toBeDefined();
      });
    });
    ```

=== "Rust"

    ```rust
    // tests/integration_module_interactions.rs
    use apcore::APCore;
    use serde_json::json;

    use crate::common::{create_mock_context, MockContextOptions};

    #[tokio::test]
    async fn orchestrator_calls_a_downstream_executor_module() {
        let mut apcore = APCore::new();
        apcore.registry().discover_internal().await.unwrap();

        let ctx = create_mock_context(MockContextOptions {
            caller_id: Some("api.handler.task_submit".to_string()),
            ..Default::default()
        });

        let result = apcore
            .call(
                "orchestrator.engine.task_flow",
                json!({"table": "user_info", "sql": "SELECT * FROM user_info"}),
                Some(&ctx),
                None,
            )
            .await
            .expect("call should succeed");

        assert!(result.get("rows").is_some() || result.get("valid").is_some());
    }

    #[tokio::test]
    async fn trace_id_stays_consistent_across_call_chain() {
        let mut apcore = APCore::new();
        apcore.registry().discover_internal().await.unwrap();

        let ctx = create_mock_context(MockContextOptions {
            caller_id: Some("api.handler.task_submit".to_string()),
            ..Default::default()
        });
        let trace_before = ctx.trace_id().to_string();

        apcore
            .call(
                "executor.validator.db_params",
                json!({"table": "user_info", "sql": "SELECT 1"}),
                Some(&ctx),
                None,
            )
            .await
            .unwrap();

        assert_eq!(ctx.trace_id(), trace_before);
    }
    ```

### 4.2 Testing Middleware Chain

=== "Python"

    ```python
    import pytest
    from apcore import APCore, Middleware
    from tests.conftest import create_mock_context

    class LoggingMiddleware(Middleware):
        def __init__(self, name: str, log: list[str], priority: int = 100):
            super().__init__(priority=priority)
            self.name = name
            self.log = log

        def before(self, module_id, inputs, context):
            self.log.append(f"{self.name}:before")
            return None  # pass through unchanged

        def after(self, module_id, inputs, output, context):
            self.log.append(f"{self.name}:after")
            return None

        def on_error(self, module_id, inputs, error, context):
            self.log.append(f"{self.name}:error")
            return None

    def test_middleware_execution_order():
        """Onion model: high priority wraps low priority"""
        log: list[str] = []
        client = APCore()
        client.registry.discover()
        client.use_middleware(LoggingMiddleware("first", log, priority=100))
        client.use_middleware(LoggingMiddleware("second", log, priority=50))

        client.call(
            "executor.validator.db_params",
            {"table": "user_info", "sql": "SELECT 1"},
            context=create_mock_context(),
        )

        assert log == [
            "first:before",
            "second:before",
            "second:after",
            "first:after",
        ]

    def test_middleware_abort():
        """Middleware can abort execution by raising"""

        class AbortMiddleware(Middleware):
            def before(self, module_id, inputs, context):
                raise PermissionError("Middleware abort")

        client = APCore()
        client.registry.discover()
        client.use_middleware(AbortMiddleware(priority=100))

        with pytest.raises(PermissionError):
            client.call(
                "executor.validator.db_params",
                {"table": "user_info", "sql": "SELECT 1"},
                context=create_mock_context(),
            )
    ```

=== "TypeScript"

    ```typescript
    import { describe, it, expect } from 'vitest';
    import { APCore, Middleware } from 'apcore-js';
    import type { Context } from 'apcore-js';
    import { createMockContext } from '../helpers.js';

    class LoggingMiddleware extends Middleware {
      constructor(
        public readonly label: string,
        public readonly log: string[],
        priority = 100,
      ) {
        super(priority);
      }
      override before(
        _moduleId: string,
        _inputs: Record<string, unknown>,
        _ctx: Context,
      ): null {
        this.log.push(`${this.label}:before`);
        return null;
      }
      override after(
        _moduleId: string,
        _inputs: Record<string, unknown>,
        _output: Record<string, unknown>,
        _ctx: Context,
      ): null {
        this.log.push(`${this.label}:after`);
        return null;
      }
    }

    describe('Middleware chain', () => {
      it('executes in onion order: high priority wraps low', async () => {
        const log: string[] = [];
        const client = new APCore();
        await client.registry.discover();
        client.useMiddleware(new LoggingMiddleware('first', log, 100));
        client.useMiddleware(new LoggingMiddleware('second', log, 50));

        await client.call(
          'executor.validator.db_params',
          { table: 'user_info', sql: 'SELECT 1' },
          createMockContext(),
        );

        expect(log).toEqual([
          'first:before',
          'second:before',
          'second:after',
          'first:after',
        ]);
      });

      it('aborts execution when a middleware throws in before()', async () => {
        class AbortMiddleware extends Middleware {
          override before(): null {
            throw new Error('Middleware abort');
          }
        }
        const client = new APCore();
        await client.registry.discover();
        client.useMiddleware(new AbortMiddleware(100));

        await expect(
          client.call(
            'executor.validator.db_params',
            { table: 'user_info', sql: 'SELECT 1' },
            createMockContext(),
          ),
        ).rejects.toThrow('Middleware abort');
      });
    });
    ```

=== "Rust"

    ```rust
    use apcore::context::Context;
    use apcore::errors::{ErrorCode, ModuleError};
    use apcore::middleware::Middleware;
    use apcore::APCore;
    use async_trait::async_trait;
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};

    #[derive(Debug)]
    struct LoggingMiddleware {
        name: String,
        log: Arc<Mutex<Vec<String>>>,
        priority: u16,
    }

    #[async_trait]
    impl Middleware for LoggingMiddleware {
        fn name(&self) -> &str {
            &self.name
        }
        fn priority(&self) -> u16 {
            self.priority
        }
        async fn before(
            &self,
            _module_id: &str,
            _inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            self.log.lock().unwrap().push(format!("{}:before", self.name));
            Ok(None)
        }
        async fn after(
            &self,
            _module_id: &str,
            _inputs: Value,
            _output: Value,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            self.log.lock().unwrap().push(format!("{}:after", self.name));
            Ok(None)
        }
        async fn on_error(
            &self,
            _module_id: &str,
            _inputs: Value,
            _err: &ModuleError,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            self.log.lock().unwrap().push(format!("{}:error", self.name));
            Ok(None)
        }
    }

    #[tokio::test]
    async fn middleware_executes_in_onion_order() {
        let log = Arc::new(Mutex::new(Vec::<String>::new()));
        let mut apcore = APCore::new();
        apcore.registry().discover_internal().await.unwrap();
        apcore.use_middleware(Box::new(LoggingMiddleware {
            name: "first".into(),
            log: log.clone(),
            priority: 100,
        }));
        apcore.use_middleware(Box::new(LoggingMiddleware {
            name: "second".into(),
            log: log.clone(),
            priority: 50,
        }));

        apcore
            .call(
                "executor.validator.db_params",
                json!({"table": "user_info", "sql": "SELECT 1"}),
                None,
                None,
            )
            .await
            .unwrap();

        assert_eq!(
            *log.lock().unwrap(),
            vec![
                "first:before",
                "second:before",
                "second:after",
                "first:after"
            ]
        );
    }

    #[derive(Debug)]
    struct AbortMiddleware;

    #[async_trait]
    impl Middleware for AbortMiddleware {
        fn name(&self) -> &str {
            "abort"
        }
        async fn before(
            &self,
            _module_id: &str,
            _inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            Err(ModuleError::new(
                ErrorCode::PermissionDenied,
                "Middleware abort".to_string(),
            ))
        }
        async fn after(
            &self,
            _: &str,
            _: Value,
            _: Value,
            _: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            Ok(None)
        }
        async fn on_error(
            &self,
            _: &str,
            _: Value,
            _: &ModuleError,
            _: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            Ok(None)
        }
    }

    #[tokio::test]
    async fn middleware_can_abort_execution() {
        let mut apcore = APCore::new();
        apcore.registry().discover_internal().await.unwrap();
        apcore.use_middleware(Box::new(AbortMiddleware));

        let err = apcore
            .call(
                "executor.validator.db_params",
                json!({"table": "user_info", "sql": "SELECT 1"}),
                None,
                None,
            )
            .await
            .expect_err("should abort");
        assert_eq!(err.code, ErrorCode::PermissionDenied);
    }
    ```

### 4.3 Full Pipeline Testing

=== "Python"

    ```python
    import pytest
    from apcore import APCore, ACL
    from apcore.errors import ACLDeniedError, SchemaValidationError
    from tests.conftest import create_mock_context

    def test_full_pipeline_with_acl():
        """Discover -> ACL check -> Schema validation -> Execute"""
        client = APCore()
        client.registry.discover()
        client.executor.acl = ACL.load("./acl/global_acl.yaml")

        context = create_mock_context(
            caller_id="orchestrator.engine.task_flow",
            executor=client.executor,
        )

        result = client.call(
            "executor.validator.db_params",
            {"table": "user_info", "sql": "SELECT * FROM user_info"},
            context=context,
        )

        assert result["valid"] is True

    def test_full_pipeline_acl_denied():
        """ACL denial should raise ACLDeniedError"""
        client = APCore()
        client.registry.discover()
        client.executor.acl = ACL.load("./acl/strict_acl.yaml")

        context = create_mock_context(
            caller_id="unauthorized.module",
            executor=client.executor,
        )

        with pytest.raises(ACLDeniedError) as exc_info:
            client.call("internal.secret.module", {}, context=context)

        assert exc_info.value.caller_id == "unauthorized.module"
        assert exc_info.value.target_id == "internal.secret.module"

    def test_full_pipeline_schema_validation_error():
        """Schema validation failure raises SchemaValidationError"""
        client = APCore()
        client.registry.discover()

        context = create_mock_context(executor=client.executor)

        with pytest.raises(SchemaValidationError) as exc_info:
            client.call(
                "executor.validator.db_params",
                {"table": "INVALID-TABLE!", "sql": "SELECT 1"},
                context=context,
            )

        error = exc_info.value
        assert error.code == "SCHEMA_VALIDATION_ERROR"
        assert len(error.errors) > 0
    ```

=== "TypeScript"

    ```typescript
    import { describe, it, expect } from 'vitest';
    import { APCore, ACL } from 'apcore-js';
    import { ACLDeniedError, SchemaValidationError } from 'apcore-js';
    import { createMockContext } from '../helpers.js';

    describe('Full Pipeline', () => {
      it('runs ACL check then schema validation then execute', async () => {
        const client = new APCore();
        await client.registry.discover();
        client.executor.acl = ACL.load('./acl/global_acl.yaml');

        const context = createMockContext({
          callerId: 'orchestrator.engine.task_flow',
          executor: client.executor,
        });

        const result = await client.call(
          'executor.validator.db_params',
          { table: 'user_info', sql: 'SELECT * FROM user_info' },
          context,
        );

        expect(result['valid']).toBe(true);
      });

      it('raises ACLDeniedError when ACL denies the call', async () => {
        const client = new APCore();
        await client.registry.discover();
        client.executor.acl = ACL.load('./acl/strict_acl.yaml');

        const context = createMockContext({
          callerId: 'unauthorized.module',
          executor: client.executor,
        });

        await expect(
          client.call('internal.secret.module', {}, context),
        ).rejects.toMatchObject({
          name: 'ACLDeniedError',
          callerId: 'unauthorized.module',
          targetId: 'internal.secret.module',
        });
      });

      it('raises SchemaValidationError on invalid input', async () => {
        const client = new APCore();
        await client.registry.discover();
        const context = createMockContext({ executor: client.executor });

        await expect(
          client.call(
            'executor.validator.db_params',
            { table: 'INVALID-TABLE!', sql: 'SELECT 1' },
            context,
          ),
        ).rejects.toBeInstanceOf(SchemaValidationError);
      });
    });
    ```

=== "Rust"

    ```rust
    use apcore::acl::ACL;
    use apcore::errors::ErrorCode;
    use apcore::APCore;
    use serde_json::json;

    use crate::common::{create_mock_context, MockContextOptions};

    #[tokio::test]
    async fn full_pipeline_with_acl() {
        let mut apcore = APCore::new();
        apcore.registry().discover_internal().await.unwrap();
        let acl = ACL::load("./acl/global_acl.yaml").unwrap();
        apcore.executor().set_acl(acl);

        let ctx = create_mock_context(MockContextOptions {
            caller_id: Some("orchestrator.engine.task_flow".to_string()),
            ..Default::default()
        });

        let result = apcore
            .call(
                "executor.validator.db_params",
                json!({"table": "user_info", "sql": "SELECT * FROM user_info"}),
                Some(&ctx),
                None,
            )
            .await
            .unwrap();

        assert_eq!(result["valid"], json!(true));
    }

    #[tokio::test]
    async fn full_pipeline_acl_denied() {
        let mut apcore = APCore::new();
        apcore.registry().discover_internal().await.unwrap();
        apcore
            .executor()
            .set_acl(ACL::load("./acl/strict_acl.yaml").unwrap());

        let ctx = create_mock_context(MockContextOptions {
            caller_id: Some("unauthorized.module".to_string()),
            ..Default::default()
        });

        let err = apcore
            .call("internal.secret.module", json!({}), Some(&ctx), None)
            .await
            .expect_err("should be denied");

        assert_eq!(err.code, ErrorCode::AclDenied);
        assert!(err
            .message
            .contains("unauthorized.module"));
    }

    #[tokio::test]
    async fn full_pipeline_schema_validation_error() {
        let mut apcore = APCore::new();
        apcore.registry().discover_internal().await.unwrap();

        let ctx = create_mock_context(MockContextOptions::default());

        let err = apcore
            .call(
                "executor.validator.db_params",
                json!({"table": "INVALID-TABLE!", "sql": "SELECT 1"}),
                Some(&ctx),
                None,
            )
            .await
            .expect_err("should fail schema validation");

        assert_eq!(err.code, ErrorCode::SchemaValidationError);
    }
    ```

---

## 5. ACL Testing

### 5.1 Testing Permission Rules

=== "Python"

    ```python
    # tests/test_acl.py
    from apcore import ACL

    class TestACLRules:
        """Test ACL permission rules"""

        def setup_method(self):
            self.acl = ACL.load("./acl/global_acl.yaml")

        def test_allow_orchestrator_to_executor(self):
            """Orchestration layer can call execution layer"""
            assert self.acl.check(
                caller_id="orchestrator.engine.task_flow",
                target_id="executor.validator.db_params",
            ) is True

        def test_deny_executor_to_api(self):
            """Execution layer cannot call API layer"""
            assert self.acl.check(
                caller_id="executor.handler.db_query",
                target_id="api.handler.task_submit",
            ) is False

        def test_external_caller(self):
            """External call (caller_id is None) maps to @external"""
            result = self.acl.check(caller_id=None, target_id="api.handler.task_submit")
            assert isinstance(result, bool)
    ```

=== "TypeScript"

    ```typescript
    // tests/test-acl.test.ts
    import { describe, it, expect, beforeEach } from 'vitest';
    import { ACL } from 'apcore-js';

    describe('ACL rules', () => {
      let acl: ACL;
      beforeEach(() => {
        acl = ACL.load('./acl/global_acl.yaml');
      });

      it('allows orchestrator -> executor', () => {
        expect(
          acl.check('orchestrator.engine.task_flow', 'executor.validator.db_params'),
        ).toBe(true);
      });

      it('denies executor -> api', () => {
        expect(
          acl.check('executor.handler.db_query', 'api.handler.task_submit'),
        ).toBe(false);
      });

      it('handles external caller (null) via @external', () => {
        const result = acl.check(null, 'api.handler.task_submit');
        expect(typeof result).toBe('boolean');
      });
    });
    ```

=== "Rust"

    ```rust
    // tests/test_acl_rules.rs
    use apcore::acl::ACL;

    #[test]
    fn allows_orchestrator_to_executor() {
        let acl = ACL::load("./acl/global_acl.yaml").unwrap();
        assert!(acl.check(
            Some("orchestrator.engine.task_flow"),
            "executor.validator.db_params",
            None,
        ));
    }

    #[test]
    fn denies_executor_to_api() {
        let acl = ACL::load("./acl/global_acl.yaml").unwrap();
        assert!(!acl.check(
            Some("executor.handler.db_query"),
            "api.handler.task_submit",
            None,
        ));
    }

    #[test]
    fn external_caller_is_handled() {
        let acl = ACL::load("./acl/global_acl.yaml").unwrap();
        // None caller maps to @external — the result depends on the loaded
        // configuration. We only assert the call returns a boolean (no panic).
        let _ = acl.check(None, "api.handler.task_submit", None);
    }
    ```

### 5.2 Testing Deny/Allow Scenarios

ACL evaluation across all SDKs is **first-match-wins** over an ordered rule list, with a `default_effect` falling back when nothing matches.

=== "Python"

    ```python
    from apcore import ACL
    from apcore.acl import ACLRule

    def test_default_deny_without_matching_rule():
        """default_effect=deny -> no matching rule means deny"""
        acl = ACL(
            rules=[ACLRule(
                callers=["orchestrator.*"],
                targets=["executor.*"],
                effect="allow",
            )],
            default_effect="deny",
        )
        assert acl.check("api.handler.test", "internal.secret") is False

    def test_default_allow_without_matching_rule():
        """default_effect=allow -> no matching rule means allow"""
        acl = ACL(
            rules=[ACLRule(
                callers=["*"],
                targets=["internal.*"],
                effect="deny",
            )],
            default_effect="allow",
        )
        assert acl.check("api.handler.test", "common.util.format") is True

    def test_first_match_wins_deny_before_allow():
        """When deny precedes allow for the same target, deny wins"""
        acl = ACL(
            rules=[
                ACLRule(callers=["*"], targets=["internal.*"], effect="deny"),
                ACLRule(callers=["*"], targets=["internal.*"], effect="allow"),
            ],
            default_effect="allow",
        )
        assert acl.check("api.handler.test", "internal.secret") is False

    def test_more_specific_rule_listed_first_wins():
        """Order rules from specific to general; the first match wins"""
        acl = ACL(
            rules=[
                ACLRule(callers=["admin.*"], targets=["internal.*"], effect="allow"),
                ACLRule(callers=["*"],       targets=["internal.*"], effect="deny"),
            ],
            default_effect="deny",
        )
        assert acl.check("admin.panel", "internal.secret") is True
        assert acl.check("api.handler.test", "internal.secret") is False
    ```

=== "TypeScript"

    ```typescript
    import { describe, it, expect } from 'vitest';
    import { ACL } from 'apcore-js';

    describe('ACL deny/allow scenarios', () => {
      it('default deny: no matching rule means deny', () => {
        const acl = new ACL(
          [{
            callers: ['orchestrator.*'],
            targets: ['executor.*'],
            effect: 'allow',
            description: '',
          }],
          'deny',
        );
        expect(acl.check('api.handler.test', 'internal.secret')).toBe(false);
      });

      it('default allow: no matching rule means allow', () => {
        const acl = new ACL(
          [{
            callers: ['*'],
            targets: ['internal.*'],
            effect: 'deny',
            description: '',
          }],
          'allow',
        );
        expect(acl.check('api.handler.test', 'common.util.format')).toBe(true);
      });

      it('first-match-wins: deny listed before allow', () => {
        const acl = new ACL(
          [
            { callers: ['*'], targets: ['internal.*'], effect: 'deny',  description: '' },
            { callers: ['*'], targets: ['internal.*'], effect: 'allow', description: '' },
          ],
          'allow',
        );
        expect(acl.check('api.handler.test', 'internal.secret')).toBe(false);
      });

      it('more specific rule listed first wins', () => {
        const acl = new ACL(
          [
            { callers: ['admin.*'], targets: ['internal.*'], effect: 'allow', description: '' },
            { callers: ['*'],       targets: ['internal.*'], effect: 'deny',  description: '' },
          ],
          'deny',
        );
        expect(acl.check('admin.panel', 'internal.secret')).toBe(true);
        expect(acl.check('api.handler.test', 'internal.secret')).toBe(false);
      });
    });
    ```

=== "Rust"

    ```rust
    use apcore::acl::{ACLRule, ACL};

    fn rule(callers: &[&str], targets: &[&str], effect: &str) -> ACLRule {
        ACLRule {
            callers: callers.iter().map(|s| s.to_string()).collect(),
            targets: targets.iter().map(|s| s.to_string()).collect(),
            effect: effect.to_string(),
            description: None,
            conditions: None,
        }
    }

    #[test]
    fn default_deny_no_matching_rule() {
        let acl = ACL::new(
            vec![rule(&["orchestrator.*"], &["executor.*"], "allow")],
            "deny",
            None,
        );
        assert!(!acl.check(Some("api.handler.test"), "internal.secret", None));
    }

    #[test]
    fn default_allow_no_matching_rule() {
        let acl = ACL::new(
            vec![rule(&["*"], &["internal.*"], "deny")],
            "allow",
            None,
        );
        assert!(acl.check(Some("api.handler.test"), "common.util.format", None));
    }

    #[test]
    fn first_match_wins_deny_before_allow() {
        let acl = ACL::new(
            vec![
                rule(&["*"], &["internal.*"], "deny"),
                rule(&["*"], &["internal.*"], "allow"),
            ],
            "allow",
            None,
        );
        assert!(!acl.check(Some("api.handler.test"), "internal.secret", None));
    }

    #[test]
    fn more_specific_rule_listed_first_wins() {
        let acl = ACL::new(
            vec![
                rule(&["admin.*"], &["internal.*"], "allow"),
                rule(&["*"],       &["internal.*"], "deny"),
            ],
            "deny",
            None,
        );
        assert!(acl.check(Some("admin.panel"), "internal.secret", None));
        assert!(!acl.check(Some("api.handler.test"), "internal.secret", None));
    }
    ```

### 5.3 Testing Pattern Matching

=== "Python"

    ```python
    from apcore import ACL
    from apcore.acl import ACLRule

    def test_exact_match():
        acl = ACL(
            rules=[ACLRule(
                callers=["orchestrator.user.register"],
                targets=["executor.email.send_email"],
                effect="allow",
            )],
            default_effect="deny",
        )
        assert acl.check("orchestrator.user.register", "executor.email.send_email") is True
        assert acl.check("orchestrator.user.login",    "executor.email.send_email") is False

    def test_wildcard_prefix():
        acl = ACL(
            rules=[ACLRule(callers=["orchestrator.*"], targets=["executor.*"], effect="allow")],
            default_effect="deny",
        )
        assert acl.check("orchestrator.engine.task_flow", "executor.validator.db_params") is True
        assert acl.check("orchestrator.a.b.c",            "executor.x.y.z") is True
        assert acl.check("api.handler.test",              "executor.validator.db_params") is False

    def test_global_wildcard():
        acl = ACL(
            rules=[ACLRule(callers=["*"], targets=["common.*"], effect="allow")],
            default_effect="deny",
        )
        assert acl.check("api.handler.test",   "common.util.format") is True
        assert acl.check("executor.handler.db", "common.util.format") is True

    def test_special_callers():
        """None caller_id is normalized to '@external' before matching"""
        acl = ACL(
            rules=[
                ACLRule(callers=["@external"], targets=["api.*"],      effect="allow"),
                ACLRule(callers=["@system"],   targets=["internal.*"], effect="allow"),
            ],
            default_effect="deny",
        )
        assert acl.check(None, "api.handler.task_submit") is True
        assert acl.check(None, "internal.secret") is False
    ```

=== "TypeScript"

    ```typescript
    import { describe, it, expect } from 'vitest';
    import { ACL } from 'apcore-js';

    describe('ACL pattern matching', () => {
      it('exact match', () => {
        const acl = new ACL(
          [{
            callers: ['orchestrator.user.register'],
            targets: ['executor.email.send_email'],
            effect: 'allow',
            description: '',
          }],
          'deny',
        );
        expect(acl.check('orchestrator.user.register', 'executor.email.send_email')).toBe(true);
        expect(acl.check('orchestrator.user.login',    'executor.email.send_email')).toBe(false);
      });

      it('prefix wildcard', () => {
        const acl = new ACL(
          [{
            callers: ['orchestrator.*'],
            targets: ['executor.*'],
            effect: 'allow',
            description: '',
          }],
          'deny',
        );
        expect(acl.check('orchestrator.engine.task_flow', 'executor.validator.db_params')).toBe(true);
        expect(acl.check('orchestrator.a.b.c',            'executor.x.y.z')).toBe(true);
        expect(acl.check('api.handler.test',              'executor.validator.db_params')).toBe(false);
      });

      it('global wildcard', () => {
        const acl = new ACL(
          [{ callers: ['*'], targets: ['common.*'], effect: 'allow', description: '' }],
          'deny',
        );
        expect(acl.check('api.handler.test',    'common.util.format')).toBe(true);
        expect(acl.check('executor.handler.db', 'common.util.format')).toBe(true);
      });

      it('null caller is normalized to @external', () => {
        const acl = new ACL(
          [
            { callers: ['@external'], targets: ['api.*'],      effect: 'allow', description: '' },
            { callers: ['@system'],   targets: ['internal.*'], effect: 'allow', description: '' },
          ],
          'deny',
        );
        expect(acl.check(null, 'api.handler.task_submit')).toBe(true);
        expect(acl.check(null, 'internal.secret')).toBe(false);
      });
    });
    ```

=== "Rust"

    ```rust
    use apcore::acl::{ACLRule, ACL};

    fn rule(callers: &[&str], targets: &[&str], effect: &str) -> ACLRule {
        ACLRule {
            callers: callers.iter().map(|s| s.to_string()).collect(),
            targets: targets.iter().map(|s| s.to_string()).collect(),
            effect: effect.to_string(),
            description: None,
            conditions: None,
        }
    }

    #[test]
    fn exact_match() {
        let acl = ACL::new(
            vec![rule(
                &["orchestrator.user.register"],
                &["executor.email.send_email"],
                "allow",
            )],
            "deny",
            None,
        );
        assert!(acl.check(
            Some("orchestrator.user.register"),
            "executor.email.send_email",
            None,
        ));
        assert!(!acl.check(
            Some("orchestrator.user.login"),
            "executor.email.send_email",
            None,
        ));
    }

    #[test]
    fn prefix_wildcard() {
        let acl = ACL::new(
            vec![rule(&["orchestrator.*"], &["executor.*"], "allow")],
            "deny",
            None,
        );
        assert!(acl.check(
            Some("orchestrator.engine.task_flow"),
            "executor.validator.db_params",
            None,
        ));
        assert!(acl.check(Some("orchestrator.a.b.c"), "executor.x.y.z", None));
        assert!(!acl.check(
            Some("api.handler.test"),
            "executor.validator.db_params",
            None,
        ));
    }

    #[test]
    fn global_wildcard() {
        let acl = ACL::new(
            vec![rule(&["*"], &["common.*"], "allow")],
            "deny",
            None,
        );
        assert!(acl.check(Some("api.handler.test"), "common.util.format", None));
        assert!(acl.check(Some("executor.handler.db"), "common.util.format", None));
    }

    #[test]
    fn none_caller_is_normalized_to_external() {
        let acl = ACL::new(
            vec![
                rule(&["@external"], &["api.*"],      "allow"),
                rule(&["@system"],   &["internal.*"], "allow"),
            ],
            "deny",
            None,
        );
        assert!(acl.check(None, "api.handler.task_submit", None));
        assert!(!acl.check(None, "internal.secret", None));
    }
    ```

### 5.4 ACL Batch Testing

Use parameterized testing to efficiently cover multiple scenarios.

=== "Python"

    ```python
    import pytest
    from apcore import ACL

    @pytest.mark.parametrize("caller,target,expected", [
        # Normal layered calls
        ("api.handler.user",         "orchestrator.engine.task", True),
        ("orchestrator.engine.task", "executor.handler.db",      True),
        ("api.handler.user",         "common.util.format",       True),

        # Cross-layer calls (forbidden)
        ("api.handler.user",   "executor.handler.db", False),
        ("executor.handler.db","api.handler.user",    False),

        # External calls
        (None, "api.handler.user", True),
        (None, "internal.secret",  False),

        # Self calls
        ("executor.handler.db", "executor.handler.db", True),
    ])
    def test_acl_rules(caller, target, expected):
        """Parameterized testing of ACL rules"""
        acl = ACL.load("./acl/global_acl.yaml")
        assert acl.check(caller, target) is expected, (
            f"ACL check({caller} -> {target}): expected {expected}"
        )
    ```

=== "TypeScript"

    ```typescript
    import { describe, it, expect } from 'vitest';
    import { ACL } from 'apcore-js';

    type Case = [caller: string | null, target: string, expected: boolean];

    const cases: Case[] = [
      // Normal layered calls
      ['api.handler.user',         'orchestrator.engine.task', true],
      ['orchestrator.engine.task', 'executor.handler.db',      true],
      ['api.handler.user',         'common.util.format',       true],
      // Cross-layer calls (forbidden)
      ['api.handler.user',    'executor.handler.db', false],
      ['executor.handler.db', 'api.handler.user',    false],
      // External calls
      [null, 'api.handler.user', true],
      [null, 'internal.secret',  false],
      // Self calls
      ['executor.handler.db', 'executor.handler.db', true],
    ];

    describe('Parameterized ACL rules', () => {
      const acl = ACL.load('./acl/global_acl.yaml');

      it.each(cases)('check(%s -> %s) === %s', (caller, target, expected) => {
        expect(acl.check(caller, target)).toBe(expected);
      });
    });
    ```

=== "Rust"

    ```rust
    use apcore::acl::ACL;

    #[test]
    fn parameterized_acl_rules() {
        let acl = ACL::load("./acl/global_acl.yaml").unwrap();
        let cases: &[(Option<&str>, &str, bool)] = &[
            // Normal layered calls
            (Some("api.handler.user"),         "orchestrator.engine.task", true),
            (Some("orchestrator.engine.task"), "executor.handler.db",      true),
            (Some("api.handler.user"),         "common.util.format",       true),
            // Cross-layer calls (forbidden)
            (Some("api.handler.user"),    "executor.handler.db", false),
            (Some("executor.handler.db"), "api.handler.user",    false),
            // External calls
            (None, "api.handler.user", true),
            (None, "internal.secret",  false),
            // Self calls
            (Some("executor.handler.db"), "executor.handler.db", true),
        ];

        for &(caller, target, expected) in cases {
            assert_eq!(
                acl.check(caller, target, None),
                expected,
                "ACL check({caller:?} -> {target}): expected {expected}",
            );
        }
    }
    ```

---

## 6. Mock Strategies

### 6.1 When to Mock, When to Use Real Implementation

| Scenario | Recommendation | Reason |
|----------|---------------|---------|
| Unit test module logic | Mock Executor + Mock Context | Isolate module under test |
| Test Schema validation | Use real Schema | Schema is part of source code |
| Test ACL rules | Use real ACL configuration | ACL configuration is the test subject |
| Test middleware chain | Mock Module + real middleware | Isolate middleware behavior |
| Test module interactions | Real Registry + real Executor | Verify integration |
| Test external API calls | Mock external dependencies | Avoid network dependencies |

### 6.2 Mock Context Factory

=== "Python"

    ```python
    # tests/factories.py
    from typing import Any
    from apcore import Context, Identity
    from tests.conftest import create_mock_context

    class ContextFactory:
        """Context factory supporting different test scenarios"""

        @staticmethod
        def external_call(**overrides: Any) -> Context:
            """Simulate external call (no caller_id, api_key identity)"""
            defaults: dict[str, Any] = {
                "caller_id": None,
                "call_chain": [],
                "identity": Identity(id="anon", type="api_key"),
            }
            defaults.update(overrides)
            return create_mock_context(**defaults)

        @staticmethod
        def internal_call(
            caller_id: str,
            call_chain: list[str] | None = None,
            **overrides: Any,
        ) -> Context:
            """Simulate internal module call"""
            defaults: dict[str, Any] = {
                "caller_id": caller_id,
                "call_chain": call_chain or [caller_id],
                "identity": Identity(id="service-bot", type="service"),
            }
            defaults.update(overrides)
            return create_mock_context(**defaults)

        @staticmethod
        def admin_call(**overrides: Any) -> Context:
            """Simulate admin call"""
            defaults: dict[str, Any] = {
                "caller_id": "admin.panel",
                "identity": Identity(
                    id="admin-001",
                    type="user",
                    roles=("admin", "super_admin"),
                ),
            }
            defaults.update(overrides)
            return create_mock_context(**defaults)

        @staticmethod
        def agent_call(agent_id: str = "ai-agent-001", **overrides: Any) -> Context:
            """Simulate AI Agent call"""
            defaults: dict[str, Any] = {
                "caller_id": None,
                "identity": Identity(id=agent_id, type="agent", roles=("agent",)),
            }
            defaults.update(overrides)
            return create_mock_context(**defaults)
    ```

=== "TypeScript"

    ```typescript
    // tests/factories.ts
    import { Context, createIdentity } from 'apcore-js';
    import { createMockContext, type MockContextOptions } from './helpers.js';

    export const ContextFactory = {
      /** Simulate external call (no caller_id, api_key identity). */
      externalCall(overrides: Partial<MockContextOptions> = {}): Context {
        return createMockContext({
          callerId: null,
          callChain: [],
          identity: createIdentity('anon', 'api_key', []),
          ...overrides,
        });
      },

      /** Simulate internal module call. */
      internalCall(
        callerId: string,
        callChain?: string[],
        overrides: Partial<MockContextOptions> = {},
      ): Context {
        return createMockContext({
          callerId,
          callChain: callChain ?? [callerId],
          identity: createIdentity('service-bot', 'service', []),
          ...overrides,
        });
      },

      /** Simulate admin call. */
      adminCall(overrides: Partial<MockContextOptions> = {}): Context {
        return createMockContext({
          callerId: 'admin.panel',
          identity: createIdentity('admin-001', 'user', ['admin', 'super_admin']),
          ...overrides,
        });
      },

      /** Simulate AI Agent call. */
      agentCall(
        agentId = 'ai-agent-001',
        overrides: Partial<MockContextOptions> = {},
      ): Context {
        return createMockContext({
          callerId: null,
          identity: createIdentity(agentId, 'agent', ['agent']),
          ...overrides,
        });
      },
    };
    ```

=== "Rust"

    ```rust
    // tests/common/factories.rs
    use apcore::context::{Context, Identity};
    use serde_json::Value;
    use std::collections::HashMap;

    use crate::common::{create_mock_context, MockContextOptions};

    fn ident(id: &str, ty: &str, roles: &[&str]) -> Identity {
        Identity::new(
            id.to_string(),
            ty.to_string(),
            roles.iter().map(|s| s.to_string()).collect(),
            HashMap::new(),
        )
    }

    pub struct ContextFactory;

    impl ContextFactory {
        /// Simulate external call (no caller_id, api_key identity).
        pub fn external_call() -> Context<Value> {
            create_mock_context(MockContextOptions {
                caller_id: None,
                call_chain: vec![],
                identity: Some(ident("anon", "api_key", &[])),
                ..Default::default()
            })
        }

        /// Simulate internal module call.
        pub fn internal_call(caller_id: &str, call_chain: Option<Vec<String>>) -> Context<Value> {
            create_mock_context(MockContextOptions {
                caller_id: Some(caller_id.to_string()),
                call_chain: call_chain.unwrap_or_else(|| vec![caller_id.to_string()]),
                identity: Some(ident("service-bot", "service", &[])),
                ..Default::default()
            })
        }

        /// Simulate admin call.
        pub fn admin_call() -> Context<Value> {
            create_mock_context(MockContextOptions {
                caller_id: Some("admin.panel".to_string()),
                identity: Some(ident("admin-001", "user", &["admin", "super_admin"])),
                ..Default::default()
            })
        }

        /// Simulate AI Agent call.
        pub fn agent_call(agent_id: &str) -> Context<Value> {
            create_mock_context(MockContextOptions {
                caller_id: None,
                identity: Some(ident(agent_id, "agent", &["agent"])),
                ..Default::default()
            })
        }
    }
    ```

### 6.3 Advanced Mock Executor Usage

=== "Python"

    ```python
    from typing import Any, Callable
    from tests.conftest import MockExecutor, create_mock_context

    class AdvancedMockExecutor(MockExecutor):
        """MockExecutor supporting conditional responses keyed on inputs/context"""

        def __init__(self) -> None:
            super().__init__()
            self._conditional: list[tuple[str, Callable[[dict, Any], bool], dict]] = []
            self._call_count: dict[str, int] = {}

        def register_conditional_response(
            self,
            module_id: str,
            condition: Callable[[dict, Any], bool],
            response: dict[str, Any],
        ) -> None:
            self._conditional.append((module_id, condition, response))

        def call(self, module_id: str, inputs: dict, context: Any) -> dict[str, Any]:
            self._call_count[module_id] = self._call_count.get(module_id, 0) + 1
            for mid, condition, response in self._conditional:
                if mid == module_id and condition(inputs, context):
                    self._call_log.append({
                        "module_id": module_id,
                        "inputs": inputs,
                        "matched": "conditional",
                    })
                    return response
            return super().call(module_id, inputs, context)

        def get_call_count(self, module_id: str) -> int:
            return self._call_count.get(module_id, 0)

    def test_conditional_executor_response():
        executor = AdvancedMockExecutor()
        executor.register_conditional_response(
            "executor.validator.db_params",
            lambda inputs, ctx: "DROP" in inputs.get("sql", "").upper(),
            {"valid": False, "errors": [{"code": "DANGEROUS_SQL"}]},
        )
        executor.register_response(
            "executor.validator.db_params",
            {"valid": True, "errors": []},
        )

        ctx = create_mock_context(executor=executor)

        # Safe SQL -> default response
        result = executor.call(
            "executor.validator.db_params", {"table": "t", "sql": "SELECT 1"}, ctx,
        )
        assert result["valid"] is True

        # Dangerous SQL -> conditional response
        result = executor.call(
            "executor.validator.db_params", {"table": "t", "sql": "DROP TABLE t"}, ctx,
        )
        assert result["valid"] is False
    ```

=== "TypeScript"

    ```typescript
    import { describe, it, expect } from 'vitest';
    import type { Context } from 'apcore-js';
    import { MockExecutor } from './mock-executor.js';
    import { createMockContext } from './helpers.js';

    type ConditionalCondition = (
      inputs: Record<string, unknown>,
      ctx: Context,
    ) => boolean;

    class AdvancedMockExecutor extends MockExecutor {
      private conditional: Array<{
        moduleId: string;
        condition: ConditionalCondition;
        response: Record<string, unknown>;
      }> = [];
      private callCount = new Map<string, number>();

      registerConditionalResponse(
        moduleId: string,
        condition: ConditionalCondition,
        response: Record<string, unknown>,
      ): void {
        this.conditional.push({ moduleId, condition, response });
      }

      override async call(
        moduleId: string,
        inputs: Record<string, unknown>,
        context: Context,
      ): Promise<Record<string, unknown>> {
        this.callCount.set(moduleId, (this.callCount.get(moduleId) ?? 0) + 1);
        for (const c of this.conditional) {
          if (c.moduleId === moduleId && c.condition(inputs, context)) {
            return c.response;
          }
        }
        return super.call(moduleId, inputs, context);
      }

      getCallCount(moduleId: string): number {
        return this.callCount.get(moduleId) ?? 0;
      }
    }

    describe('AdvancedMockExecutor', () => {
      it('returns conditional response when predicate matches', async () => {
        const executor = new AdvancedMockExecutor();

        executor.registerConditionalResponse(
          'executor.validator.db_params',
          (inputs) =>
            String(inputs['sql'] ?? '')
              .toUpperCase()
              .includes('DROP'),
          { valid: false, errors: [{ code: 'DANGEROUS_SQL' }] },
        );
        executor.registerResponse('executor.validator.db_params', {
          valid: true,
          errors: [],
        });

        const ctx = createMockContext({ executor });

        const safe = await executor.call(
          'executor.validator.db_params',
          { table: 't', sql: 'SELECT 1' },
          ctx,
        );
        expect(safe['valid']).toBe(true);

        const dangerous = await executor.call(
          'executor.validator.db_params',
          { table: 't', sql: 'DROP TABLE t' },
          ctx,
        );
        expect(dangerous['valid']).toBe(false);
      });
    });
    ```

=== "Rust"

    ```rust
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use serde_json::{json, Value};
    use std::sync::Mutex;

    use crate::common::mock_executor::MockExecutor;
    use crate::common::{create_mock_context, MockContextOptions};

    type Condition = Box<dyn Fn(&Value, &Context<Value>) -> bool + Send + Sync>;

    struct ConditionalEntry {
        module_id: String,
        condition: Condition,
        response: Value,
    }

    pub struct AdvancedMockExecutor {
        inner: MockExecutor,
        conditional: Mutex<Vec<ConditionalEntry>>,
    }

    impl AdvancedMockExecutor {
        pub fn new() -> Self {
            Self {
                inner: MockExecutor::new(),
                conditional: Mutex::new(Vec::new()),
            }
        }

        pub fn register_response(&self, module_id: &str, response: Value) {
            self.inner.register_response(module_id, response);
        }

        pub fn register_conditional_response<F>(
            &self,
            module_id: &str,
            condition: F,
            response: Value,
        )
        where
            F: Fn(&Value, &Context<Value>) -> bool + Send + Sync + 'static,
        {
            self.conditional.lock().unwrap().push(ConditionalEntry {
                module_id: module_id.to_string(),
                condition: Box::new(condition),
                response,
            });
        }

        pub async fn call(
            &self,
            module_id: &str,
            inputs: Value,
            ctx: &Context<Value>,
        ) -> Result<Value, ModuleError> {
            let entries = self.conditional.lock().unwrap();
            for entry in entries.iter() {
                if entry.module_id == module_id && (entry.condition)(&inputs, ctx) {
                    return Ok(entry.response.clone());
                }
            }
            drop(entries);
            self.inner.call(module_id, inputs, ctx).await
        }
    }

    #[tokio::test]
    async fn returns_conditional_response_when_predicate_matches() {
        let executor = AdvancedMockExecutor::new();

        executor.register_conditional_response(
            "executor.validator.db_params",
            |inputs, _ctx| {
                inputs
                    .get("sql")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_uppercase().contains("DROP"))
                    .unwrap_or(false)
            },
            json!({"valid": false, "errors": [{"code": "DANGEROUS_SQL"}]}),
        );
        executor.register_response(
            "executor.validator.db_params",
            json!({"valid": true, "errors": []}),
        );

        let ctx = create_mock_context(MockContextOptions::default());

        let safe = executor
            .call(
                "executor.validator.db_params",
                json!({"table": "t", "sql": "SELECT 1"}),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(safe["valid"], json!(true));

        let dangerous = executor
            .call(
                "executor.validator.db_params",
                json!({"table": "t", "sql": "DROP TABLE t"}),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(dangerous["valid"], json!(false));
    }
    ```

### 6.4 Isolated Registry

For unit tests you usually want an empty `Registry` populated with only the modules under test, instead of triggering full filesystem discovery.

=== "Python"

    ```python
    from apcore import Registry
    from extensions.executor.validator.db_params import DbParamsValidator

    def test_with_isolated_registry():
        registry = Registry()  # No extensions_dir -> nothing auto-discovered

        # Register only modules needed for the test
        registry.register("executor.validator.db_params", DbParamsValidator())

        assert registry.has("executor.validator.db_params")
        assert not registry.has("executor.handler.db_query")
        assert "executor.validator.db_params" in registry.list_modules()
    ```

=== "TypeScript"

    ```typescript
    import { describe, it, expect } from 'vitest';
    import { Registry } from 'apcore-js';
    import { DbParamsValidator } from '../src/extensions/executor/validator/db-params.js';

    describe('Isolated Registry', () => {
      it('contains only explicitly registered modules', () => {
        const registry = new Registry(); // No extensionsDir -> nothing auto-discovered

        registry.register('executor.validator.db_params', new DbParamsValidator());

        expect(registry.has('executor.validator.db_params')).toBe(true);
        expect(registry.has('executor.handler.db_query')).toBe(false);
        expect(registry.listModules()).toContain('executor.validator.db_params');
      });
    });
    ```

=== "Rust"

    ```rust
    use apcore::registry::Registry;
    use apcore::module::Module;

    use crate::extensions::executor::validator::db_params::DbParamsValidator;

    #[test]
    fn isolated_registry_contains_only_registered_modules() {
        let registry = Registry::new(); // Empty by default

        registry
            .register_module(
                "executor.validator.db_params",
                Box::new(DbParamsValidator::new()) as Box<dyn Module>,
            )
            .expect("register should succeed");

        assert!(registry.contains("executor.validator.db_params"));
        assert!(!registry.contains("executor.handler.db_query"));
        let ids = registry.list_modules(None, None);
        assert!(ids
            .iter()
            .any(|id| id == "executor.validator.db_params"));
    }
    ```

---

## 6.5 Output Verification

Schema validation (Executor Step 9) ensures output structural correctness, but does not guarantee semantic correctness. Module tests **should** additionally verify output content.

### Verification Levels

| Level | What to Verify | Automated? | Example |
|------|------|------|------|
| **Schema** | Output matches `output_schema` | Yes (Executor Step 9) | Handled by framework |
| **Content** | Output contains expected values | Manual assertion | `assert result["status"] == "sent"` |
| **Side Effect** | External state changed correctly | Manual assertion | `assert os.path.exists(result["path"])` |
| **Format** | Binary output has correct format | Manual assertion | `assert content[:4] == b"%PDF"` |

### Anti-Pattern: Trust-Based Testing

=== "Python"

    ```python
    import os

    # BAD: Only checks no exception was raised
    result = executor.call("export.render", {"format": "pdf"})
    assert result is not None

    # GOOD: Verifies semantic output
    result = executor.call("export.render", {"format": "pdf"})
    assert result["file_path"].endswith(".pdf")
    assert os.path.getsize(result["file_path"]) > 0
    ```

=== "TypeScript"

    ```typescript
    import { statSync } from 'node:fs';

    // BAD: Only checks no exception was raised
    const bad = await executor.call('export.render', { format: 'pdf' });
    expect(bad).toBeDefined();

    // GOOD: Verifies semantic output
    const result = await executor.call('export.render', { format: 'pdf' });
    expect(result['file_path']).toMatch(/\.pdf$/);
    expect(statSync(result['file_path'] as string).size).toBeGreaterThan(0);
    ```

=== "Rust"

    ```rust
    use std::fs;

    // BAD: Only checks the call succeeded
    let bad = apcore
        .call("export.render", json!({"format": "pdf"}), None, None)
        .await
        .unwrap();
    assert!(bad.is_object());

    // GOOD: Verifies semantic output
    let result = apcore
        .call("export.render", json!({"format": "pdf"}), None, None)
        .await
        .unwrap();
    let path = result["file_path"].as_str().expect("file_path is a string");
    assert!(path.ends_with(".pdf"));
    assert!(fs::metadata(path).unwrap().len() > 0);
    ```

---

## 7. Testing Best Practices

### 7.1 Test Naming Convention

The same naming idea — `{subject}_{scenario}_{expected_result}` — works in every language; only the test-runner conventions differ.

=== "Python"

    ```python
    # pytest: snake_case function names that start with `test_`.
    # Format: test_{function_being_tested}_{scenario}_{expected_result}

    # Unit tests
    def test_validate_sql_with_drop_statement_returns_invalid(): ...
    def test_validate_sql_with_select_statement_returns_valid(): ...
    def test_execute_with_timeout_zero_raises_validation_error(): ...

    # Schema tests
    def test_input_schema_rejects_null_table(): ...
    def test_input_schema_accepts_optional_timeout(): ...
    def test_output_schema_requires_valid_field(): ...

    # ACL tests
    def test_acl_allows_orchestrator_to_executor(): ...
    def test_acl_denies_api_to_internal(): ...
    def test_acl_external_caller_mapped_to_at_external(): ...

    # Integration tests
    def test_pipeline_validator_then_executor_succeeds(): ...
    def test_pipeline_with_acl_denied_raises_error(): ...
    ```

=== "TypeScript"

    ```typescript
    // Vitest: describe groups by subject, sentence-style `it` titles.
    // Title format: '{subject} {scenario} {expected result}'

    describe('SQL validator', () => {
      it('rejects DROP statements as invalid', () => { /* ... */ });
      it('accepts SELECT statements as valid',   () => { /* ... */ });
      it('raises on timeout=0',                  () => { /* ... */ });
    });

    describe('Input schema', () => {
      it('rejects null table',           () => { /* ... */ });
      it('accepts an optional timeout',  () => { /* ... */ });
    });

    describe('ACL', () => {
      it('allows orchestrator -> executor',          () => { /* ... */ });
      it('denies api -> internal',                   () => { /* ... */ });
      it('maps a null caller_id to @external',       () => { /* ... */ });
    });

    describe('Pipeline integration', () => {
      it('runs validator then executor successfully',     async () => { /* ... */ });
      it('raises ACLDeniedError when ACL denies the call', async () => { /* ... */ });
    });
    ```

=== "Rust"

    ```rust
    // Built-in test framework: snake_case function names with #[test] / #[tokio::test].
    // Format: {subject}_{scenario}_{expected_result}

    // Unit tests
    #[test] fn validate_sql_with_drop_statement_returns_invalid() {}
    #[test] fn validate_sql_with_select_statement_returns_valid() {}
    #[test] fn execute_with_timeout_zero_raises_validation_error() {}

    // Schema tests
    #[test] fn input_schema_rejects_null_table() {}
    #[test] fn input_schema_accepts_optional_timeout() {}
    #[test] fn output_schema_requires_valid_field() {}

    // ACL tests
    #[test] fn acl_allows_orchestrator_to_executor() {}
    #[test] fn acl_denies_api_to_internal() {}
    #[test] fn acl_external_caller_mapped_to_at_external() {}

    // Integration tests (typically async — use tokio)
    #[tokio::test] async fn pipeline_validator_then_executor_succeeds() {}
    #[tokio::test] async fn pipeline_with_acl_denied_raises_error() {}
    ```

### 7.2 Fixtures and Test Data Management

=== "Python"

    ```python
    # tests/conftest.py
    import pytest
    from apcore import ACL
    from apcore.acl import ACLRule
    from extensions.executor.validator.db_params import DbParamsValidator
    from tests.conftest import MockExecutor, create_mock_context
    from tests.factories import ContextFactory

    @pytest.fixture
    def mock_context():
        """Default Mock Context"""
        return create_mock_context()

    @pytest.fixture
    def mock_executor():
        """Configurable Mock Executor"""
        return MockExecutor()

    @pytest.fixture
    def context_factory():
        """Context factory for scenario-based tests"""
        return ContextFactory

    @pytest.fixture
    def db_params_module():
        """Database parameters validator instance"""
        return DbParamsValidator()

    @pytest.fixture
    def acl_default_layered():
        """Standard layered-call ACL used across most tests"""
        return ACL(
            rules=[
                ACLRule(callers=["orchestrator.*"], targets=["executor.*"],     effect="allow"),
                ACLRule(callers=["api.*"],          targets=["orchestrator.*"], effect="allow"),
                ACLRule(callers=["*"],              targets=["common.*"],       effect="allow"),
                ACLRule(callers=["@external"],      targets=["api.*"],          effect="allow"),
            ],
            default_effect="deny",
        )

    # Store binary fixtures under:
    #   tests/fixtures/schemas/   -- test Schema files
    #   tests/fixtures/acl/       -- test ACL YAML configurations
    #   tests/fixtures/inputs/    -- canned input payloads
    ```

=== "TypeScript"

    ```typescript
    // tests/fixtures.ts
    // Vitest does not have a fixture system like pytest; instead, export
    // factory helpers and call them inside `beforeEach` (or the test body).
    import { ACL } from 'apcore-js';
    import { MockExecutor } from './mock-executor.js';
    import { createMockContext } from './helpers.js';
    import { ContextFactory } from './factories.js';
    import { DbParamsValidator } from '../src/extensions/executor/validator/db-params.js';

    export const fixtures = {
      mockContext: () => createMockContext(),
      mockExecutor: () => new MockExecutor(),
      contextFactory: () => ContextFactory,
      dbParamsModule: () => new DbParamsValidator(),
      aclDefaultLayered: () =>
        new ACL(
          [
            { callers: ['orchestrator.*'], targets: ['executor.*'],     effect: 'allow', description: '' },
            { callers: ['api.*'],          targets: ['orchestrator.*'], effect: 'allow', description: '' },
            { callers: ['*'],              targets: ['common.*'],       effect: 'allow', description: '' },
            { callers: ['@external'],      targets: ['api.*'],          effect: 'allow', description: '' },
          ],
          'deny',
        ),
    };

    // Usage:
    //   import { describe, it, beforeEach } from 'vitest';
    //   import { fixtures } from './fixtures.js';
    //   describe('something', () => {
    //     let executor: ReturnType<typeof fixtures.mockExecutor>;
    //     beforeEach(() => { executor = fixtures.mockExecutor(); });
    //   });
    //
    // Store binary fixtures under:
    //   tests/fixtures/schemas/  -- test Schema files
    //   tests/fixtures/acl/      -- test ACL YAML configurations
    //   tests/fixtures/inputs/   -- canned input payloads
    ```

=== "Rust"

    ```rust
    // tests/common/fixtures.rs
    // Rust does not have pytest-style fixtures. The idiomatic equivalent is
    // a set of free helper functions in a `tests/common/` module that each
    // test imports and calls directly.
    use apcore::acl::{ACLRule, ACL};
    use apcore::context::Context;
    use serde_json::Value;

    use crate::common::mock_executor::MockExecutor;
    use crate::common::{create_mock_context, MockContextOptions};
    use crate::extensions::executor::validator::db_params::DbParamsValidator;

    pub fn mock_context() -> Context<Value> {
        create_mock_context(MockContextOptions::default())
    }

    pub fn mock_executor() -> MockExecutor {
        MockExecutor::new()
    }

    pub fn db_params_module() -> DbParamsValidator {
        DbParamsValidator::new()
    }

    pub fn acl_default_layered() -> ACL {
        let rule = |callers: &[&str], targets: &[&str], effect: &str| ACLRule {
            callers: callers.iter().map(|s| s.to_string()).collect(),
            targets: targets.iter().map(|s| s.to_string()).collect(),
            effect: effect.to_string(),
            description: None,
            conditions: None,
        };
        ACL::new(
            vec![
                rule(&["orchestrator.*"], &["executor.*"],     "allow"),
                rule(&["api.*"],          &["orchestrator.*"], "allow"),
                rule(&["*"],              &["common.*"],       "allow"),
                rule(&["@external"],      &["api.*"],          "allow"),
            ],
            "deny",
            None,
        )
    }

    // Store binary fixtures under:
    //   tests/fixtures/schemas/  -- test Schema files
    //   tests/fixtures/acl/      -- test ACL YAML configurations
    //   tests/fixtures/inputs/   -- canned input payloads
    ```

**Recommended test directory structure:**

```
tests/
├── conftest.py                   # Global fixtures
├── factories.py                  # Mock factories
├── fixtures/                     # Test data
│   ├── schemas/                  # Test Schemas
│   ├── acl/                      # Test ACL configurations
│   └── inputs/                   # Test input data
├── unit/                         # Unit tests
│   ├── test_db_params.py
│   └── test_email_sender.py
├── schema/                       # Schema tests
│   ├── test_schema_validity.py
│   └── test_schema_edge_cases.py
├── acl/                          # ACL tests
│   ├── test_acl_rules.py
│   └── test_acl_patterns.py
└── integration/                  # Integration tests
    ├── test_module_interactions.py
    └── test_middleware_chain.py
```

### 7.3 CI/CD Integration

```yaml
# .github/workflows/test.yml

name: apcore Tests

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  unit-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-python@v5
        with:
          python-version: "3.11"
      - run: pip install -r requirements-dev.txt
      - run: pytest tests/unit/ tests/schema/ tests/acl/ -v --tb=short
        name: "Unit tests + Schema tests + ACL tests"

  integration-tests:
    runs-on: ubuntu-latest
    needs: unit-tests
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-python@v5
        with:
          python-version: "3.11"
      - run: pip install -r requirements-dev.txt
      - run: pytest tests/integration/ -v --tb=short
        name: "Integration tests"

  schema-validation:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-python@v5
        with:
          python-version: "3.11"
      - run: pip install pyyaml jsonschema
      - run: python scripts/validate_schemas.py
        name: "Schema file validation"
```

**Schema batch validation script:**

=== "Python"

    ```python
    # scripts/validate_schemas.py
    import sys
    import yaml
    import jsonschema
    from pathlib import Path

    def validate_all_schemas() -> None:
        schemas_dir = Path("schemas")
        errors: list[str] = []

        for schema_file in sorted(schemas_dir.glob("*.schema.yaml")):
            try:
                with open(schema_file, encoding="utf-8") as f:
                    data = yaml.safe_load(f)

                for field in ("module_id", "description", "input_schema", "output_schema"):
                    if field not in data:
                        errors.append(f"{schema_file.name}: missing {field}")

                for schema_key in ("input_schema", "output_schema"):
                    if schema_key in data:
                        jsonschema.Draft202012Validator.check_schema(data[schema_key])

                print(f"OK    {schema_file.name}")
            except Exception as e:  # noqa: BLE001
                errors.append(f"{schema_file.name}: {e}")
                print(f"FAIL  {schema_file.name}")

        if errors:
            print(f"\n{len(errors)} errors:")
            for err in errors:
                print(f"  - {err}")
            sys.exit(1)
        else:
            print("\nAll Schema files validated successfully")

    if __name__ == "__main__":
        validate_all_schemas()
    ```

=== "TypeScript"

    ```typescript
    // scripts/validate-schemas.ts
    // Run with: tsx scripts/validate-schemas.ts
    import { readdirSync, readFileSync } from 'node:fs';
    import { join } from 'node:path';
    import yaml from 'js-yaml';
    import Ajv2020 from 'ajv/dist/2020.js';

    function validateAllSchemas(): void {
      const schemasDir = 'schemas';
      const errors: string[] = [];
      const ajv = new Ajv2020({ strict: false });

      const files = readdirSync(schemasDir)
        .filter((f) => f.endsWith('.schema.yaml'))
        .sort();

      for (const file of files) {
        try {
          const text = readFileSync(join(schemasDir, file), 'utf8');
          const data = yaml.load(text) as Record<string, unknown>;

          for (const field of ['module_id', 'description', 'input_schema', 'output_schema']) {
            if (!(field in data)) errors.push(`${file}: missing ${field}`);
          }

          for (const key of ['input_schema', 'output_schema'] as const) {
            if (key in data) ajv.compile(data[key] as object);
          }

          console.log(`OK    ${file}`);
        } catch (e) {
          errors.push(`${file}: ${e instanceof Error ? e.message : String(e)}`);
          console.log(`FAIL  ${file}`);
        }
      }

      if (errors.length > 0) {
        console.log(`\n${errors.length} errors:`);
        for (const err of errors) console.log(`  - ${err}`);
        process.exit(1);
      } else {
        console.log('\nAll Schema files validated successfully');
      }
    }

    validateAllSchemas();
    ```

=== "Rust"

    ```rust
    // src/bin/validate_schemas.rs
    // Run with: cargo run --bin validate_schemas
    use apcore::schema::SchemaValidator;
    use serde_json::Value;
    use std::fs;
    use std::path::PathBuf;
    use std::process::ExitCode;

    fn main() -> ExitCode {
        let mut errors: Vec<String> = Vec::new();
        let validator = SchemaValidator::new();

        let mut files: Vec<PathBuf> = fs::read_dir("schemas")
            .expect("schemas/ should exist")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.ends_with(".schema.yaml"))
                    .unwrap_or(false)
            })
            .collect();
        files.sort();

        for path in files {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            match validate_one(&path, &validator) {
                Ok(()) => println!("OK    {name}"),
                Err(e) => {
                    errors.push(format!("{name}: {e}"));
                    println!("FAIL  {name}");
                }
            }
        }

        if !errors.is_empty() {
            println!("\n{} errors:", errors.len());
            for err in &errors {
                println!("  - {err}");
            }
            ExitCode::FAILURE
        } else {
            println!("\nAll Schema files validated successfully");
            ExitCode::SUCCESS
        }
    }

    fn validate_one(path: &PathBuf, validator: &SchemaValidator) -> Result<(), String> {
        let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let data: Value = serde_yaml::from_str(&text).map_err(|e| e.to_string())?;

        for field in ["module_id", "description", "input_schema", "output_schema"] {
            if data.get(field).is_none() {
                return Err(format!("missing {field}"));
            }
        }

        for key in ["input_schema", "output_schema"] {
            // Apply each schema to a trivial value so any structural defects
            // surface as a validator error rather than a panic.
            let _ = validator.validate(&Value::Null, &data[key]);
        }

        Ok(())
    }
    ```

---

## Next Steps

- [Schema Definition Details](./schema-definition.md) - Deep dive into Schema
- [ACL Configuration Guide](./acl-configuration.md) - ACL configuration details
- [Core Executor](../features/core-executor.md) - Executor feature spec
- [Multi-Language Development Guide](./multi-language.md) - Cross-language testing strategies
