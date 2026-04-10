# apcore — Module Testing Guide

> Comprehensive coverage of apcore module testing strategies: unit testing, Schema testing, integration testing, ACL testing and Mock techniques.

!!! note "Cross-language applicability"
    Code examples in this guide use Python syntax with `pytest`. The testing strategies and patterns apply equally to TypeScript (with Jest/Vitest) and Rust (with `#[tokio::test]`). Each SDK provides the same core test utilities: `Context.create()`, schema validation, and `Executor` with configurable strategies.

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

```python
# tests/test_db_params_validator.py

import pytest
from extensions.executor.validator.db_params import DbParamsValidator

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
            "timeout": 30
        }
        context = create_mock_context()

        result = self.module.execute(inputs, context)

        assert result["valid"] is True
        assert result["message"] == "Validation passed"
        assert result["errors"] == []

    def test_dangerous_sql_drop(self):
        """Detect DROP statement"""
        inputs = {
            "table": "user_info",
            "sql": "DROP TABLE user_info"
        }
        context = create_mock_context()

        result = self.module.execute(inputs, context)

        assert result["valid"] is False
        assert any(e["code"] == "DANGEROUS_SQL" for e in result["errors"])

    def test_default_timeout(self):
        """Verify default value of timeout field"""
        inputs = {
            "table": "user_info",
            "sql": "SELECT 1"
        }
        context = create_mock_context()

        result = self.module.execute(inputs, context)

        assert result["valid"] is True
```

### 2.2 Mock Context Creation

Context is the runtime context for module execution; Mock objects need to be constructed for testing.

```python
# tests/conftest.py

import uuid
from dataclasses import dataclass, field
from typing import Any, Optional

@dataclass
class MockIdentity:
    """Mock identity information"""
    id: str = "test-user"
    type: str = "user"
    roles: list[str] = field(default_factory=lambda: ["admin"])
    attrs: dict[str, Any] = field(default_factory=dict)

@dataclass
class MockContext:
    """Mock execution context"""
    trace_id: str = field(default_factory=lambda: str(uuid.uuid4()))
    caller_id: Optional[str] = None
    call_chain: list[str] = field(default_factory=list)
    executor: Any = None
    identity: Optional[MockIdentity] = None
    data: dict[str, Any] = field(default_factory=dict)

def create_mock_context(
    caller_id: Optional[str] = None,
    trace_id: Optional[str] = None,
    call_chain: Optional[list[str]] = None,
    identity: Optional[MockIdentity] = None,
    executor: Any = None,
    data: Optional[dict] = None
) -> MockContext:
    """Factory function for creating Mock Context"""
    return MockContext(
        trace_id=trace_id or str(uuid.uuid4()),
        caller_id=caller_id,
        call_chain=call_chain or [],
        executor=executor,
        identity=identity or MockIdentity(),
        data=data or {}
    )
```

### 2.3 Mock Executor Pattern

When modules internally need to call other modules, Mock Executor is required.

```python
# tests/conftest.py

class MockExecutor:
    """Configurable Mock Executor with response values"""

    def __init__(self):
        self._responses: dict[str, Any] = {}
        self._errors: dict[str, Exception] = {}
        self._call_log: list[dict] = []

    def register_response(self, module_id: str, response: dict):
        """Register return value for module call"""
        self._responses[module_id] = response

    def register_error(self, module_id: str, error: Exception):
        """Register exception for module call"""
        self._errors[module_id] = error

    def call(self, module_id: str, inputs: dict, context: Any) -> dict:
        """Simulate module call"""
        self._call_log.append({
            "module_id": module_id,
            "inputs": inputs,
            "trace_id": getattr(context, "trace_id", None)
        })

        if module_id in self._errors:
            raise self._errors[module_id]

        if module_id in self._responses:
            return self._responses[module_id]

        raise ModuleNotFoundError(f"Mock: Unregistered module {module_id}")

    async def call_async(self, module_id: str, inputs: dict, context: Any) -> dict:
        """Simulate async module call"""
        return self.call(module_id, inputs, context)

    @property
    def call_log(self) -> list[dict]:
        """Get call log"""
        return self._call_log

    def assert_called(self, module_id: str, times: int = 1):
        """Assert a module was called a specific number of times"""
        actual = sum(1 for c in self._call_log if c["module_id"] == module_id)
        assert actual == times, f"Expected {module_id} to be called {times} times, actually {actual} times"

    def assert_not_called(self, module_id: str):
        """Assert a module was not called"""
        self.assert_called(module_id, times=0)


class ModuleNotFoundError(Exception):
    pass
```

**Test example using Mock Executor:**

```python
class TestTaskFlowOrchestrator:
    """Test task flow orchestrator (internally calls other modules)"""

    def setup_method(self):
        self.module = TaskFlowOrchestrator()
        self.executor = MockExecutor()

    def test_orchestrator_calls_validator_then_executor(self):
        """Verify orchestrator calls validator then executor"""
        # Register Mock return values
        self.executor.register_response(
            "executor.validator.db_params",
            {"valid": True, "message": "Validation passed", "errors": []}
        )
        self.executor.register_response(
            "executor.handler.db_query",
            {"rows": [{"id": 1, "name": "test"}], "count": 1}
        )

        context = create_mock_context(
            caller_id="api.handler.task_submit",
            executor=self.executor
        )

        inputs = {"table": "user_info", "sql": "SELECT * FROM user_info"}
        result = self.module.execute(inputs, context)

        # Verify call order and count
        self.executor.assert_called("executor.validator.db_params", times=1)
        self.executor.assert_called("executor.handler.db_query", times=1)
        assert result["count"] == 1

    def test_orchestrator_stops_on_validation_failure(self):
        """Verify executor is not called when validation fails"""
        self.executor.register_response(
            "executor.validator.db_params",
            {"valid": False, "message": "Validation failed", "errors": [{"code": "DANGEROUS_SQL"}]}
        )

        context = create_mock_context(executor=self.executor)

        inputs = {"table": "user_info", "sql": "DROP TABLE user_info"}
        result = self.module.execute(inputs, context)

        self.executor.assert_called("executor.validator.db_params", times=1)
        self.executor.assert_not_called("executor.handler.db_query")
        assert result["valid"] is False
```

### 2.4 Input/Output Validation Testing

Verify that the module's Schema definition matches actual behavior.

```python
from pydantic import ValidationError

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
        # Minimum value
        model = DBParamsInput(table="t", sql="SELECT 1", timeout=1)
        assert model.timeout == 1

        # Maximum value
        model = DBParamsInput(table="t", sql="SELECT 1", timeout=300)
        assert model.timeout == 300

        # Exceeding maximum value
        with pytest.raises(ValidationError):
            DBParamsInput(table="t", sql="SELECT 1", timeout=301)

        # Below minimum value
        with pytest.raises(ValidationError):
            DBParamsInput(table="t", sql="SELECT 1", timeout=0)
```

---

## 3. Schema Testing

### 3.1 Validating YAML Schema Definitions

Ensure the YAML Schema file structure itself is correct.

```python
# tests/test_schemas.py

import yaml
import jsonschema
from pathlib import Path

SCHEMAS_DIR = Path("schemas")

class TestSchemaDefinitions:
    """Test validity of Schema files"""

    def get_all_schema_files(self) -> list[Path]:
        """Get all Schema files"""
        return list(SCHEMAS_DIR.glob("*.schema.yaml"))

    @pytest.mark.parametrize("schema_file", SCHEMAS_DIR.glob("*.schema.yaml"))
    def test_schema_is_valid_yaml(self, schema_file: Path):
        """Schema file must be valid YAML"""
        with open(schema_file) as f:
            data = yaml.safe_load(f)
        assert data is not None, f"{schema_file.name} is empty"

    @pytest.mark.parametrize("schema_file", SCHEMAS_DIR.glob("*.schema.yaml"))
    def test_schema_has_required_fields(self, schema_file: Path):
        """Schema file must contain required fields"""
        with open(schema_file) as f:
            data = yaml.safe_load(f)

        assert "module_id" in data, f"{schema_file.name} missing module_id"
        assert "description" in data, f"{schema_file.name} missing description"
        assert "input_schema" in data, f"{schema_file.name} missing input_schema"
        assert "output_schema" in data, f"{schema_file.name} missing output_schema"

    @pytest.mark.parametrize("schema_file", SCHEMAS_DIR.glob("*.schema.yaml"))
    def test_input_schema_is_valid_json_schema(self, schema_file: Path):
        """input_schema must be valid JSON Schema"""
        with open(schema_file) as f:
            data = yaml.safe_load(f)

        input_schema = data.get("input_schema", {})
        # Verify it is a valid JSON Schema
        jsonschema.Draft202012Validator.check_schema(input_schema)

    @pytest.mark.parametrize("schema_file", SCHEMAS_DIR.glob("*.schema.yaml"))
    def test_output_schema_is_valid_json_schema(self, schema_file: Path):
        """output_schema must be valid JSON Schema"""
        with open(schema_file) as f:
            data = yaml.safe_load(f)

        output_schema = data.get("output_schema", {})
        jsonschema.Draft202012Validator.check_schema(output_schema)

    @pytest.mark.parametrize("schema_file", SCHEMAS_DIR.glob("*.schema.yaml"))
    def test_all_properties_have_description(self, schema_file: Path):
        """All properties should have description"""
        with open(schema_file) as f:
            data = yaml.safe_load(f)

        def check_descriptions(schema: dict, path: str = ""):
            properties = schema.get("properties", {})
            for name, prop in properties.items():
                current_path = f"{path}.{name}" if path else name
                assert "description" in prop, (
                    f"{schema_file.name}: {current_path} missing description"
                )
                # Recursively check nested objects
                if prop.get("type") == "object":
                    check_descriptions(prop, current_path)

        check_descriptions(data.get("input_schema", {}))
        check_descriptions(data.get("output_schema", {}))
```

### 3.2 Schema Boundary Value Testing

```python
class TestSchemaEdgeCases:
    """Test Schema boundary cases"""

    def setup_method(self):
        """Load Schema"""
        with open("schemas/executor/validator/db_params.schema.yaml") as f:
            self.schema = yaml.safe_load(f)
        self.input_schema = self.schema["input_schema"]

    def test_null_values(self):
        """Test handling of null values"""
        validator = jsonschema.Draft202012Validator(self.input_schema)

        # Required field as null should fail
        result = validator.is_valid({"table": None, "sql": "SELECT 1"})
        assert result is False

    def test_empty_string(self):
        """Test empty string"""
        validator = jsonschema.Draft202012Validator(self.input_schema)

        # Empty table name doesn't match pattern
        result = validator.is_valid({"table": "", "sql": "SELECT 1"})
        assert result is False

    def test_missing_optional_field(self):
        """Test missing optional field"""
        validator = jsonschema.Draft202012Validator(self.input_schema)

        # Only provide required fields
        result = validator.is_valid({"table": "user_info", "sql": "SELECT 1"})
        assert result is True

    def test_extra_fields_rejected(self):
        """Reject extra fields when additionalProperties: false"""
        validator = jsonschema.Draft202012Validator(self.input_schema)

        result = validator.is_valid({
            "table": "user_info",
            "sql": "SELECT 1",
            "unknown_field": "value"
        })
        # If Schema has additionalProperties: false
        if self.input_schema.get("additionalProperties") is False:
            assert result is False

    def test_boundary_integer_values(self):
        """Test integer boundaries"""
        validator = jsonschema.Draft202012Validator(self.input_schema)

        # Minimum value for timeout
        result = validator.is_valid({
            "table": "t",
            "sql": "SELECT 1",
            "timeout": 1
        })
        assert result is True

        # Maximum value for timeout
        result = validator.is_valid({
            "table": "t",
            "sql": "SELECT 1",
            "timeout": 300
        })
        assert result is True

        # Exceeding maximum value
        result = validator.is_valid({
            "table": "t",
            "sql": "SELECT 1",
            "timeout": 301
        })
        assert result is False
```

### 3.3 Schema Compatibility Testing

Ensure Schema changes don't break compatibility.

```python
class TestSchemaCompatibility:
    """Test Schema version compatibility"""

    def test_new_optional_field_is_backward_compatible(self):
        """Adding optional field should not break old data"""
        # Old version input (without new field)
        old_input = {"table": "user_info", "sql": "SELECT 1"}

        validator = jsonschema.Draft202012Validator(self.input_schema)
        assert validator.is_valid(old_input), "Old data should still be valid after adding optional field"

    def test_required_fields_not_removed(self):
        """Required fields should not be removed"""
        required = self.input_schema.get("required", [])
        # Compare with previous version's required fields list
        previous_required = ["table", "sql"]
        for field_name in previous_required:
            assert field_name in required, (
                f"Required field {field_name} was removed, this is a breaking change"
            )

    def test_enum_values_not_removed(self):
        """Enum values should not be removed"""
        properties = self.input_schema.get("properties", {})
        for name, prop in properties.items():
            if "enum" in prop:
                # Ensure current enum values are a superset of previous version
                pass  # Implement according to actual situation
```

---

## 4. Integration Testing

### 4.1 Testing Module Interactions

```python
# tests/integration/test_module_interactions.py

class TestModuleInteractions:
    """Test interactions between modules"""

    def setup_method(self):
        """Initialize real Registry and Executor"""
        from apcore import Registry, Executor

        self.registry = Registry(extensions_dir="./extensions")
        self.registry.discover()
        self.executor = Executor(registry=self.registry)

    def test_orchestrator_calls_executor(self):
        """Complete workflow of orchestrator calling executor"""
        context = create_mock_context(
            caller_id="api.handler.task_submit",
            executor=self.executor
        )

        result = self.executor.call(
            module_id="orchestrator.engine.task_flow",
            inputs={"table": "user_info", "sql": "SELECT * FROM user_info"},
            context=context
        )

        assert "rows" in result or "valid" in result

    def test_call_chain_propagation(self):
        """Verify call chain propagates correctly"""
        context = create_mock_context(
            caller_id="api.handler.task_submit"
        )

        # After call, call_chain should contain call path
        result = self.executor.call(
            module_id="executor.validator.db_params",
            inputs={"table": "user_info", "sql": "SELECT 1"},
            context=context
        )

        # Verify trace_id remains consistent throughout the call chain
        assert context.trace_id is not None
```

### 4.2 Testing Middleware Chain

```python
class TestMiddlewareChain:
    """Test middleware execution order"""

    def test_middleware_execution_order(self):
        """Verify middleware executes in onion model"""
        execution_log = []

        class LoggingMiddleware:
            def __init__(self, name):
                self.name = name

            def before(self, module_id, inputs, context):
                execution_log.append(f"{self.name}:before")
                return inputs

            def after(self, module_id, output, context):
                execution_log.append(f"{self.name}:after")
                return output

            def on_error(self, module_id, error, context):
                execution_log.append(f"{self.name}:error")

        from apcore import Registry, Executor

        registry = Registry(extensions_dir="./extensions")
        registry.discover()

        # Add middleware
        registry.add_middleware("first", LoggingMiddleware("first"), priority=100)
        registry.add_middleware("second", LoggingMiddleware("second"), priority=50)

        executor = Executor(registry=registry)
        context = create_mock_context(executor=executor)

        executor.call(
            module_id="executor.validator.db_params",
            inputs={"table": "user_info", "sql": "SELECT 1"},
            context=context
        )

        # Onion model: before in descending priority, after in ascending priority
        assert execution_log == [
            "first:before",
            "second:before",
            "second:after",
            "first:after"
        ]

    def test_middleware_abort(self):
        """Middleware can abort execution"""

        class AbortMiddleware:
            def before(self, module_id, inputs, context):
                raise PermissionError("Middleware abort: Insufficient permission")

            def after(self, module_id, output, context):
                return output

            def on_error(self, module_id, error, context):
                pass

        from apcore import Registry, Executor

        registry = Registry(extensions_dir="./extensions")
        registry.discover()
        registry.add_middleware("abort", AbortMiddleware(), priority=100)

        executor = Executor(registry=registry)
        context = create_mock_context(executor=executor)

        with pytest.raises(PermissionError):
            executor.call(
                module_id="executor.validator.db_params",
                inputs={"table": "user_info", "sql": "SELECT 1"},
                context=context
            )
```

### 4.3 Full Pipeline Testing

```python
class TestFullPipeline:
    """Test complete Registry -> Executor -> Module workflow"""

    def test_full_pipeline_with_acl(self):
        """Full Pipeline: Register -> Discover -> ACL Check -> Schema Validation -> Execute"""
        from apcore import Registry, Executor, ACL

        # 1. Registry
        registry = Registry(extensions_dir="./extensions")
        registry.discover()

        # 2. ACL
        acl = ACL.load("./acl/global_acl.yaml")

        # 3. Executor
        executor = Executor(registry=registry, acl=acl)

        # 4. Execute
        context = create_mock_context(
            caller_id="orchestrator.engine.task_flow",
            executor=executor
        )

        result = executor.call(
            module_id="executor.validator.db_params",
            inputs={"table": "user_info", "sql": "SELECT * FROM user_info"},
            context=context
        )

        assert result["valid"] is True

    def test_full_pipeline_acl_denied(self):
        """Full Pipeline: ACL denial should raise ACLDeniedError"""
        from apcore import Registry, Executor, ACL, ACLDeniedError

        registry = Registry(extensions_dir="./extensions")
        registry.discover()

        # Use strict ACL rules
        acl = ACL.load("./acl/strict_acl.yaml")
        executor = Executor(registry=registry, acl=acl)

        context = create_mock_context(
            caller_id="unauthorized.module",
            executor=executor
        )

        with pytest.raises(ACLDeniedError) as exc_info:
            executor.call(
                module_id="internal.secret.module",
                inputs={},
                context=context
            )

        assert exc_info.value.caller_id == "unauthorized.module"
        assert exc_info.value.target_id == "internal.secret.module"

    def test_full_pipeline_schema_validation_error(self):
        """Full Pipeline: Schema validation failure"""
        from apcore import Registry, Executor, SchemaValidationError

        registry = Registry(extensions_dir="./extensions")
        registry.discover()
        executor = Executor(registry=registry)

        context = create_mock_context(executor=executor)

        with pytest.raises(SchemaValidationError) as exc_info:
            executor.call(
                module_id="executor.validator.db_params",
                inputs={"table": "INVALID-TABLE!", "sql": "SELECT 1"},
                context=context
            )

        error = exc_info.value
        assert error.code == "SCHEMA_VALIDATION_ERROR"
        assert len(error.errors) > 0
```

---

## 5. ACL Testing

### 5.1 Testing Permission Rules

```python
# tests/test_acl.py

import yaml
from apcore import ACL

class TestACLRules:
    """Test ACL permission rules"""

    def setup_method(self):
        """Load test ACL configuration"""
        self.acl = ACL.load("./acl/global_acl.yaml")

    def test_allow_orchestrator_to_executor(self):
        """Orchestration layer can call execution layer"""
        result = self.acl.check(
            caller_id="orchestrator.engine.task_flow",
            target_id="executor.validator.db_params"
        )
        assert result is True

    def test_deny_executor_to_api(self):
        """Execution layer cannot call API layer"""
        result = self.acl.check(
            caller_id="executor.handler.db_query",
            target_id="api.handler.task_submit"
        )
        assert result is False

    def test_external_caller(self):
        """External call (caller_id is None)"""
        result = self.acl.check(
            caller_id=None,
            target_id="api.handler.task_submit"
        )
        # Depends on ACL configuration for @external
        assert isinstance(result, bool)
```

### 5.2 Testing Deny/Allow Scenarios

```python
class TestACLDenyAllow:
    """Test various deny and allow scenarios"""

    def test_default_deny_without_matching_rule(self):
        """With default_effect: deny, no matching rule should deny"""
        acl_config = {
            "default_effect": "deny",
            "rules": [
                {
                    "callers": ["orchestrator.*"],
                    "targets": ["executor.*"],
                    "effect": "allow"
                }
            ]
        }
        acl = ACL.from_dict(acl_config)

        # No matching rule -> deny
        result = acl.check("api.handler.test", "internal.secret")
        assert result is False

    def test_default_allow_without_matching_rule(self):
        """With default_effect: allow, no matching rule should allow"""
        acl_config = {
            "default_effect": "allow",
            "rules": [
                {
                    "callers": ["*"],
                    "targets": ["internal.*"],
                    "effect": "deny"
                }
            ]
        }
        acl = ACL.from_dict(acl_config)

        # No matching rule -> allow
        result = acl.check("api.handler.test", "common.util.format")
        assert result is True

    def test_deny_overrides_allow_same_priority(self):
        """Deny takes precedence over allow at same priority"""
        acl_config = {
            "default_effect": "allow",
            "rules": [
                {
                    "callers": ["*"],
                    "targets": ["internal.*"],
                    "effect": "allow",
                    "priority": 10
                },
                {
                    "callers": ["*"],
                    "targets": ["internal.*"],
                    "effect": "deny",
                    "priority": 10
                }
            ]
        }
        acl = ACL.from_dict(acl_config)

        result = acl.check("api.handler.test", "internal.secret")
        assert result is False

    def test_higher_priority_wins(self):
        """Higher priority rule takes precedence"""
        acl_config = {
            "default_effect": "deny",
            "rules": [
                {
                    "callers": ["*"],
                    "targets": ["internal.*"],
                    "effect": "deny",
                    "priority": 10
                },
                {
                    "callers": ["admin.*"],
                    "targets": ["internal.*"],
                    "effect": "allow",
                    "priority": 100
                }
            ]
        }
        acl = ACL.from_dict(acl_config)

        # Admin can access due to higher priority
        result = acl.check("admin.panel", "internal.secret")
        assert result is True

        # Regular user denied
        result = acl.check("api.handler.test", "internal.secret")
        assert result is False
```

### 5.3 Testing Pattern Matching

```python
class TestACLPatternMatching:
    """Test ACL pattern matching"""

    def test_exact_match(self):
        """Exact match"""
        acl_config = {
            "default_effect": "deny",
            "rules": [{
                "callers": ["orchestrator.user.register"],
                "targets": ["executor.email.send_email"],
                "effect": "allow"
            }]
        }
        acl = ACL.from_dict(acl_config)

        assert acl.check("orchestrator.user.register", "executor.email.send_email") is True
        assert acl.check("orchestrator.user.login", "executor.email.send_email") is False

    def test_wildcard_prefix(self):
        """Prefix wildcard"""
        acl_config = {
            "default_effect": "deny",
            "rules": [{
                "callers": ["orchestrator.*"],
                "targets": ["executor.*"],
                "effect": "allow"
            }]
        }
        acl = ACL.from_dict(acl_config)

        assert acl.check("orchestrator.engine.task_flow", "executor.validator.db_params") is True
        assert acl.check("orchestrator.a.b.c", "executor.x.y.z") is True
        assert acl.check("api.handler.test", "executor.validator.db_params") is False

    def test_global_wildcard(self):
        """Global wildcard"""
        acl_config = {
            "default_effect": "deny",
            "rules": [{
                "callers": ["*"],
                "targets": ["common.*"],
                "effect": "allow"
            }]
        }
        acl = ACL.from_dict(acl_config)

        assert acl.check("api.handler.test", "common.util.format") is True
        assert acl.check("executor.handler.db", "common.util.format") is True

    def test_special_callers(self):
        """Special caller identifiers"""
        acl_config = {
            "default_effect": "deny",
            "rules": [
                {
                    "callers": ["@external"],
                    "targets": ["api.*"],
                    "effect": "allow"
                },
                {
                    "callers": ["@system"],
                    "targets": ["internal.*"],
                    "effect": "allow"
                }
            ]
        }
        acl = ACL.from_dict(acl_config)

        # None caller_id maps to @external
        assert acl.check(None, "api.handler.task_submit") is True
        assert acl.check(None, "internal.secret") is False
```

### 5.4 ACL Batch Testing

Use parameterized testing to efficiently cover multiple scenarios.

```python
@pytest.mark.parametrize("caller,target,expected", [
    # Normal layered calls
    ("api.handler.user", "orchestrator.engine.task", True),
    ("orchestrator.engine.task", "executor.handler.db", True),
    ("api.handler.user", "common.util.format", True),

    # Cross-layer calls (forbidden)
    ("api.handler.user", "executor.handler.db", False),
    ("executor.handler.db", "api.handler.user", False),

    # External calls
    (None, "api.handler.user", True),
    (None, "internal.secret", False),

    # Self calls
    ("executor.handler.db", "executor.handler.db", True),
])
def test_acl_rules(caller, target, expected):
    """Parameterized testing of ACL rules"""
    acl = ACL.load("./acl/global_acl.yaml")
    result = acl.check(caller, target)
    assert result == expected, (
        f"ACL check({caller} -> {target}): "
        f"Expected {expected}, actual {result}"
    )
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

```python
# tests/factories.py

class ContextFactory:
    """Context factory supporting different test scenarios"""

    @staticmethod
    def external_call(**overrides) -> MockContext:
        """Simulate external call (no caller_id)"""
        defaults = {
            "caller_id": None,
            "call_chain": [],
            "identity": MockIdentity(type="api_key")
        }
        defaults.update(overrides)
        return create_mock_context(**defaults)

    @staticmethod
    def internal_call(
        caller_id: str,
        call_chain: list[str] = None,
        **overrides
    ) -> MockContext:
        """Simulate internal module call"""
        defaults = {
            "caller_id": caller_id,
            "call_chain": call_chain or [caller_id],
            "identity": MockIdentity(type="service")
        }
        defaults.update(overrides)
        return create_mock_context(**defaults)

    @staticmethod
    def admin_call(**overrides) -> MockContext:
        """Simulate admin call"""
        defaults = {
            "caller_id": "admin.panel",
            "identity": MockIdentity(
                id="admin-001",
                type="user",
                roles=["admin", "super_admin"]
            )
        }
        defaults.update(overrides)
        return create_mock_context(**defaults)

    @staticmethod
    def agent_call(agent_id: str = "ai-agent-001", **overrides) -> MockContext:
        """Simulate AI Agent call"""
        defaults = {
            "caller_id": None,
            "identity": MockIdentity(
                id=agent_id,
                type="agent",
                roles=["agent"]
            )
        }
        defaults.update(overrides)
        return create_mock_context(**defaults)
```

### 6.3 Advanced Mock Executor Usage

```python
class AdvancedMockExecutor(MockExecutor):
    """Mock Executor supporting conditional responses and delay simulation"""

    def __init__(self):
        super().__init__()
        self._conditional_responses: list[tuple] = []
        self._call_count: dict[str, int] = {}

    def register_conditional_response(
        self,
        module_id: str,
        condition: callable,
        response: dict
    ):
        """Return different results based on condition"""
        self._conditional_responses.append((module_id, condition, response))

    def call(self, module_id: str, inputs: dict, context: Any) -> dict:
        """Enhanced module call simulation"""
        self._call_count[module_id] = self._call_count.get(module_id, 0) + 1

        # Check conditional responses
        for mid, condition, response in self._conditional_responses:
            if mid == module_id and condition(inputs, context):
                self._call_log.append({
                    "module_id": module_id,
                    "inputs": inputs,
                    "matched": "conditional"
                })
                return response

        return super().call(module_id, inputs, context)

    def get_call_count(self, module_id: str) -> int:
        """Get number of times module was called"""
        return self._call_count.get(module_id, 0)


# Usage example
def test_conditional_executor_response():
    executor = AdvancedMockExecutor()

    # Return different results based on input condition
    executor.register_conditional_response(
        "executor.validator.db_params",
        condition=lambda inputs, ctx: "DROP" in inputs.get("sql", "").upper(),
        response={"valid": False, "errors": [{"code": "DANGEROUS_SQL"}]}
    )
    executor.register_response(
        "executor.validator.db_params",
        {"valid": True, "errors": []}
    )

    context = create_mock_context(executor=executor)

    # Safe SQL -> default response
    result = executor.call(
        "executor.validator.db_params",
        {"table": "t", "sql": "SELECT 1"},
        context
    )
    assert result["valid"] is True

    # Dangerous SQL -> conditional response
    result = executor.call(
        "executor.validator.db_params",
        {"table": "t", "sql": "DROP TABLE t"},
        context
    )
    assert result["valid"] is False
```

### 6.4 Mock Registry

```python
class MockRegistry:
    """Mock Registry for testing module isolation"""

    def __init__(self):
        self._modules: dict[str, Any] = {}
        self._schemas: dict[str, dict] = {}

    def register(self, module_id: str, module_instance: Any, schema: dict = None):
        """Register module"""
        self._modules[module_id] = module_instance
        if schema:
            self._schemas[module_id] = schema

    def get(self, module_id: str) -> Any:
        """Get module"""
        if module_id not in self._modules:
            raise ModuleNotFoundError(f"Module not found: {module_id}")
        return self._modules[module_id]

    def has(self, module_id: str) -> bool:
        """Check if module exists"""
        return module_id in self._modules

    def list_modules(self) -> list[str]:
        """List all modules"""
        return list(self._modules.keys())

    def get_schema(self, module_id: str) -> dict:
        """Get module Schema"""
        return self._schemas.get(module_id, {})


# Usage example
def test_with_mock_registry():
    registry = MockRegistry()

    # Only register modules needed for testing
    module = DbParamsValidator()
    registry.register(
        "executor.validator.db_params",
        module,
        schema={"input_schema": {...}, "output_schema": {...}}
    )

    assert registry.has("executor.validator.db_params")
    assert not registry.has("executor.handler.db_query")
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

```python
# ❌ BAD: Only checks no exception was raised
result = executor.call("export.render", {"format": "pdf"})
assert result is not None

# ✅ GOOD: Verifies semantic output
result = executor.call("export.render", {"format": "pdf"})
assert result["file_path"].endswith(".pdf")
assert os.path.getsize(result["file_path"]) > 0
```

---

## 7. Testing Best Practices

### 7.1 Test Naming Convention

```python
# Naming format: test_{function_being_tested}_{scenario}_{expected_result}

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

### 7.2 Fixtures and Test Data Management

```python
# tests/conftest.py

import pytest
from pathlib import Path

@pytest.fixture
def mock_context():
    """Provide default Mock Context"""
    return create_mock_context()

@pytest.fixture
def mock_executor():
    """Provide configurable Mock Executor"""
    return MockExecutor()

@pytest.fixture
def context_factory():
    """Provide Context factory"""
    return ContextFactory()

@pytest.fixture
def db_params_module():
    """Provide database parameters validator instance"""
    return DbParamsValidator()

@pytest.fixture
def acl_config():
    """Provide standard test ACL configuration"""
    return {
        "default_effect": "deny",
        "rules": [
            {"callers": ["orchestrator.*"], "targets": ["executor.*"], "effect": "allow"},
            {"callers": ["api.*"], "targets": ["orchestrator.*"], "effect": "allow"},
            {"callers": ["*"], "targets": ["common.*"], "effect": "allow"},
            {"callers": ["@external"], "targets": ["api.*"], "effect": "allow"},
        ]
    }

# tests/fixtures/schemas/
# Store test Schema files

# tests/fixtures/acl/
# Store test ACL configuration files
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

```python
# scripts/validate_schemas.py

import sys
import yaml
import jsonschema
from pathlib import Path

def validate_all_schemas():
    """Validate all Schema files"""
    schemas_dir = Path("schemas")
    errors = []

    for schema_file in schemas_dir.glob("*.schema.yaml"):
        try:
            with open(schema_file) as f:
                data = yaml.safe_load(f)

            # Check required fields
            for field in ["module_id", "description", "input_schema", "output_schema"]:
                if field not in data:
                    errors.append(f"{schema_file.name}: missing {field}")

            # Validate JSON Schema legitimacy
            for schema_key in ["input_schema", "output_schema"]:
                if schema_key in data:
                    jsonschema.Draft202012Validator.check_schema(data[schema_key])

            print(f"✓ {schema_file.name} ... OK")

        except Exception as e:
            errors.append(f"{schema_file.name}: {e}")
            print(f"✗ {schema_file.name} ... FAIL")

    if errors:
        print(f"\n{len(errors)} errors:")
        for err in errors:
            print(f"  - {err}")
        sys.exit(1)
    else:
        print(f"\nAll Schema files validated successfully")

if __name__ == "__main__":
    validate_all_schemas()
```

---

## Next Steps

- [Schema Definition Details](./schema-definition.md) - Deep dive into Schema
- [ACL Configuration Guide](./acl-configuration.md) - ACL configuration details
- [Executor API](../api/executor-api.md) - Executor interface reference
- [Multi-Language Development Guide](./multi-language.md) - Cross-language testing strategies
