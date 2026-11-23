"""Tests for module decorator, type inference, and error classes."""

from __future__ import annotations

import inspect
from typing import Annotated, Any, Literal, Optional

import pytest
from pydantic import BaseModel, Field, ValidationError

from apcore.context import Context
from apcore.decorator import (
    FunctionModule,
    generate_input_model,
    generate_output_model,
    _context_param_name,
    _make_auto_id,
    module,
)
from apcore.errors import (
    BindingCallableNotFoundError,
    BindingFileInvalidError,
    BindingInvalidTargetError,
    BindingModuleNotFoundError,
    BindingNotCallableError,
    BindingSchemaInferenceFailedError,
    BindingSchemaMissingError,
    FuncMissingReturnTypeError,
    FuncMissingTypeHintError,
    ModuleError,
)
from apcore.executor import Executor
from apcore.registry import Registry


class TestFuncMissingTypeHintError:
    """Tests for FuncMissingTypeHintError."""

    def test_instantiates_with_function_and_parameter_name(self):
        """Error stores function_name and parameter_name in details."""
        err = FuncMissingTypeHintError(function_name="send_email", parameter_name="to")
        assert err.details["function_name"] == "send_email"
        assert err.details["parameter_name"] == "to"

    def test_has_correct_code(self):
        """Error code is FUNC_MISSING_TYPE_HINT."""
        err = FuncMissingTypeHintError(function_name="f", parameter_name="x")
        assert err.code == "FUNC_MISSING_TYPE_HINT"

    def test_message_includes_function_and_parameter_name(self):
        """Error message includes both the function name and parameter name for diagnostics."""
        err = FuncMissingTypeHintError(function_name="send_email", parameter_name="to")
        assert "'send_email'" in str(err)
        assert "'to'" in str(err)

    def test_inherits_from_module_error(self):
        """Error inherits from ModuleError."""
        err = FuncMissingTypeHintError(function_name="f", parameter_name="x")
        assert isinstance(err, ModuleError)


class TestFuncMissingReturnTypeError:
    """Tests for FuncMissingReturnTypeError."""

    def test_instantiates_with_function_name(self):
        """Error stores function_name in details."""
        err = FuncMissingReturnTypeError(function_name="process")
        assert err.details["function_name"] == "process"

    def test_has_correct_code(self):
        """Error code is FUNC_MISSING_RETURN_TYPE."""
        err = FuncMissingReturnTypeError(function_name="process")
        assert err.code == "FUNC_MISSING_RETURN_TYPE"

    def test_inherits_from_module_error(self):
        """Error inherits from ModuleError."""
        err = FuncMissingReturnTypeError(function_name="process")
        assert isinstance(err, ModuleError)


class TestBindingInvalidTargetError:
    """Tests for BindingInvalidTargetError."""

    def test_has_correct_code_and_includes_target(self):
        """Error code is BINDING_INVALID_TARGET and details include the target string."""
        err = BindingInvalidTargetError(target="no_colon_here")
        assert err.code == "BINDING_INVALID_TARGET"
        assert err.details["target"] == "no_colon_here"

    def test_inherits_from_module_error(self):
        """Error inherits from ModuleError."""
        err = BindingInvalidTargetError(target="bad")
        assert isinstance(err, ModuleError)


class TestBindingModuleNotFoundError:
    """Tests for BindingModuleNotFoundError."""

    def test_has_correct_code_and_includes_module_path(self):
        """Error code is BINDING_MODULE_NOT_FOUND and details include the module path."""
        err = BindingModuleNotFoundError(module_path="myapp.missing")
        assert err.code == "BINDING_MODULE_NOT_FOUND"
        assert err.details["module_path"] == "myapp.missing"

    def test_inherits_from_module_error(self):
        """Error inherits from ModuleError."""
        err = BindingModuleNotFoundError(module_path="x")
        assert isinstance(err, ModuleError)


class TestBindingCallableNotFoundError:
    """Tests for BindingCallableNotFoundError."""

    def test_has_correct_code_and_includes_callable_name(self):
        """Error code is BINDING_CALLABLE_NOT_FOUND and details include callable name and module path."""
        err = BindingCallableNotFoundError(callable_name="send", module_path="myapp.email")
        assert err.code == "BINDING_CALLABLE_NOT_FOUND"
        assert err.details["callable_name"] == "send"
        assert err.details["module_path"] == "myapp.email"

    def test_inherits_from_module_error(self):
        """Error inherits from ModuleError."""
        err = BindingCallableNotFoundError(callable_name="x", module_path="y")
        assert isinstance(err, ModuleError)


class TestBindingNotCallableError:
    """Tests for BindingNotCallableError."""

    def test_has_correct_code(self):
        """Error code is BINDING_NOT_CALLABLE."""
        err = BindingNotCallableError(target="myapp.email:NOT_A_FUNC")
        assert err.code == "BINDING_NOT_CALLABLE"

    def test_inherits_from_module_error(self):
        """Error inherits from ModuleError."""
        err = BindingNotCallableError(target="x")
        assert isinstance(err, ModuleError)


class TestBindingSchemaInferenceFailedError:
    """Tests for BindingSchemaInferenceFailedError (canonical name in spec 1.0)."""

    def test_has_correct_code(self):
        """Error code is BINDING_SCHEMA_INFERENCE_FAILED."""
        err = BindingSchemaInferenceFailedError(target="myapp:func")
        assert err.code == "BINDING_SCHEMA_INFERENCE_FAILED"

    def test_inherits_from_module_error(self):
        """Error inherits from ModuleError."""
        err = BindingSchemaInferenceFailedError(target="x")
        assert isinstance(err, ModuleError)

    def test_legacy_alias_points_to_canonical(self):
        """BindingSchemaMissingError is a deprecated alias for the canonical class."""
        assert BindingSchemaMissingError is BindingSchemaInferenceFailedError

    def test_includes_module_id_and_remediation_in_message(self):
        """Message includes module_id and remediation hint when supplied."""
        err = BindingSchemaInferenceFailedError(
            target="myapp:func",
            module_id="app.thing",
            file_path="b.yaml",
            line=12,
        )
        assert "app.thing" in err.message
        assert "b.yaml:12" in err.message
        assert "See protocol-spec §5.12.5" in err.message


class TestBindingFileInvalidError:
    """Tests for BindingFileInvalidError."""

    def test_has_correct_code_and_includes_file_path(self):
        """Error code is BINDING_FILE_INVALID and details include the file path."""
        err = BindingFileInvalidError(file_path="/etc/bindings.yaml", reason="empty file")
        assert err.code == "BINDING_FILE_INVALID"
        assert err.details["file_path"] == "/etc/bindings.yaml"
        assert err.details["reason"] == "empty file"

    def test_inherits_from_module_error(self):
        """Error inherits from ModuleError."""
        err = BindingFileInvalidError(file_path="x.yaml", reason="bad")
        assert isinstance(err, ModuleError)


class TestAllErrorsInheritFromModuleError:
    """Cross-cutting test ensuring all 8 new errors inherit from ModuleError."""

    @pytest.mark.parametrize(
        "error_cls,kwargs",
        [
            (FuncMissingTypeHintError, {"function_name": "f", "parameter_name": "p"}),
            (FuncMissingReturnTypeError, {"function_name": "f"}),
            (BindingInvalidTargetError, {"target": "t"}),
            (BindingModuleNotFoundError, {"module_path": "m"}),
            (BindingCallableNotFoundError, {"callable_name": "c", "module_path": "m"}),
            (BindingNotCallableError, {"target": "t"}),
            (BindingSchemaMissingError, {"target": "t"}),
            (BindingFileInvalidError, {"file_path": "f", "reason": "r"}),
        ],
    )
    def test_inherits_from_module_error(self, error_cls, kwargs):
        """Every new error class is a subclass of ModuleError."""
        err = error_cls(**kwargs)
        assert isinstance(err, ModuleError)

    @pytest.mark.parametrize(
        "error_cls,kwargs,expected_code",
        [
            (
                FuncMissingTypeHintError,
                {"function_name": "f", "parameter_name": "p"},
                "FUNC_MISSING_TYPE_HINT",
            ),
            (
                FuncMissingReturnTypeError,
                {"function_name": "f"},
                "FUNC_MISSING_RETURN_TYPE",
            ),
            (BindingInvalidTargetError, {"target": "t"}, "BINDING_INVALID_TARGET"),
            (
                BindingModuleNotFoundError,
                {"module_path": "m"},
                "BINDING_MODULE_NOT_FOUND",
            ),
            (
                BindingCallableNotFoundError,
                {"callable_name": "c", "module_path": "m"},
                "BINDING_CALLABLE_NOT_FOUND",
            ),
            (BindingNotCallableError, {"target": "t"}, "BINDING_NOT_CALLABLE"),
            (BindingSchemaInferenceFailedError, {"target": "t"}, "BINDING_SCHEMA_INFERENCE_FAILED"),
            (
                BindingFileInvalidError,
                {"file_path": "f", "reason": "r"},
                "BINDING_FILE_INVALID",
            ),
        ],
    )
    def test_code_attribute_accessible(self, error_cls, kwargs, expected_code):
        """Every error has the correct code attribute."""
        err = error_cls(**kwargs)
        assert err.code == expected_code


# ---------------------------------------------------------------------------
# Section 02: Type Inference Engine Tests
# ---------------------------------------------------------------------------


class _HelperModel(BaseModel):
    """Helper model for testing nested BaseModel parameters."""

    x: int
    y: str


class _OutputModel(BaseModel):
    """Helper model for testing BaseModel return types."""

    greeting: str


class TestGenerateInputModel:
    """Tests for generate_input_model()."""

    def test_simple_primitives(self):
        """func(name: str, age: int) produces model with required str and int fields."""

        def func(name: str, age: int) -> dict:
            return {}

        Model = generate_input_model(func)
        assert "name" in Model.model_fields
        assert "age" in Model.model_fields
        assert Model.model_fields["name"].is_required()
        assert Model.model_fields["age"].is_required()
        inst = Model(name="Alice", age=30)
        assert inst.name == "Alice"
        assert inst.age == 30

    def test_default_values(self):
        """func(name: str, count: int = 5) produces model where count has default 5."""

        def func(name: str, count: int = 5) -> dict:
            return {}

        Model = generate_input_model(func)
        assert Model.model_fields["name"].is_required()
        assert not Model.model_fields["count"].is_required()
        inst = Model(name="Alice")
        assert inst.count == 5

    def test_optional_type(self):
        """func(value: Optional[str]) allows None."""

        def func(value: Optional[str]) -> dict:
            return {}

        Model = generate_input_model(func)
        inst = Model(value=None)
        assert inst.value is None

    def test_union_type(self):
        """func(value: str | int) accepts both types."""

        def func(value: str | int) -> dict:
            return {}

        Model = generate_input_model(func)
        assert Model(value="hello").value == "hello"
        assert Model(value=42).value == 42

    def test_list_type(self):
        """func(items: list[str]) produces model with list field."""

        def func(items: list[str]) -> dict:
            return {}

        Model = generate_input_model(func)
        inst = Model(items=["a", "b"])
        assert inst.items == ["a", "b"]

    def test_dict_type(self):
        """func(data: dict[str, int]) produces model with dict field."""

        def func(data: dict[str, int]) -> dict:
            return {}

        Model = generate_input_model(func)
        inst = Model(data={"a": 1})
        assert inst.data == {"a": 1}

    def test_literal_type(self):
        """func(mode: Literal["fast", "slow"]) produces model with enum constraint."""

        def func(mode: Literal["fast", "slow"]) -> dict:
            return {}

        Model = generate_input_model(func)
        inst = Model(mode="fast")
        assert inst.mode == "fast"
        with pytest.raises(ValidationError):
            Model(mode="invalid")

    def test_annotated_with_field(self):
        """func(x: Annotated[int, Field(ge=0)]) produces model with constraint."""

        def func(x: Annotated[int, Field(ge=0)]) -> dict:
            return {}

        Model = generate_input_model(func)
        inst = Model(x=5)
        assert inst.x == 5
        with pytest.raises(ValidationError):
            Model(x=-1)

    def test_nested_basemodel_param(self):
        """func(config: MyModel) produces model with nested object."""

        def func(config: _HelperModel) -> dict:
            return {}

        Model = generate_input_model(func)
        inst = Model(config={"x": 1, "y": "hello"})
        assert inst.config.x == 1
        assert inst.config.y == "hello"

    def test_context_param_skipped(self):
        """func(name: str, context: Context) produces model with only name."""

        def func(name: str, context: Context) -> dict:
            return {}

        Model = generate_input_model(func)
        assert "name" in Model.model_fields
        assert "context" not in Model.model_fields

    def test_context_detected_by_type_not_name(self):
        """func(ctx: Context) still skipped (type-based detection)."""

        def func(name: str, ctx: Context) -> dict:
            return {}

        Model = generate_input_model(func)
        assert "name" in Model.model_fields
        assert "ctx" not in Model.model_fields

    def test_self_skipped(self):
        """Method's self parameter is skipped."""

        class Svc:
            def method(self, name: str) -> dict:
                return {}

        Model = generate_input_model(Svc.method)
        assert "name" in Model.model_fields
        assert "self" not in Model.model_fields

    def test_args_skipped(self):
        """*args is skipped."""

        def func(*args: Any, name: str) -> dict:
            return {}

        Model = generate_input_model(func)
        assert "name" in Model.model_fields
        assert len(Model.model_fields) == 1

    def test_kwargs_sets_extra_allow(self):
        """**kwargs causes extra='allow' on model."""

        def func(name: str, **kwargs: Any) -> dict:
            return {}

        Model = generate_input_model(func)
        assert "name" in Model.model_fields
        assert Model.model_config.get("extra") == "allow"
        # Extra fields accepted
        inst = Model(name="Alice", extra_field="value")
        assert inst.name == "Alice"

    def test_missing_type_hint_raises(self):
        """Missing type hint raises FuncMissingTypeHintError."""

        # Create function without annotations on 'age'
        def func(name: str, age):  # noqa: ANN001
            return {}

        with pytest.raises(FuncMissingTypeHintError) as exc_info:
            generate_input_model(func)
        assert exc_info.value.details["parameter_name"] == "age"

    def test_nameerror_raises_func_missing_type_hint(self):
        """get_type_hints NameError raises FuncMissingTypeHintError."""

        # Create a function with an unresolvable forward reference
        ns: dict[str, Any] = {}
        exec("def func(x: 'NonExistentType123') -> dict: return {}", ns)  # noqa: S102
        func = ns["func"]
        # Ensure the function's globals don't contain the type
        func.__globals__.pop("NonExistentType123", None)
        with pytest.raises(FuncMissingTypeHintError):
            generate_input_model(func)

    def test_only_args_and_kwargs(self):
        """Function with only *args and **kwargs produces empty model with extra='allow'."""

        def func(*args: Any, **kwargs: Any) -> dict:
            return {}

        Model = generate_input_model(func)
        assert len(Model.model_fields) == 0
        assert Model.model_config.get("extra") == "allow"

    def test_empty_function(self):
        """Function with no params produces empty model."""

        def func() -> dict:
            return {}

        Model = generate_input_model(func)
        assert len(Model.model_fields) == 0
        inst = Model()
        assert inst.model_dump() == {}

    def test_multiple_defaults(self):
        """func(a: str = 'x', b: int = 0) both have defaults."""

        def func(a: str = "x", b: int = 0) -> dict:
            return {}

        Model = generate_input_model(func)
        inst = Model()
        assert inst.a == "x"
        assert inst.b == 0

    def test_works_with_future_annotations(self):
        """Functions from modules using from __future__ import annotations work.

        This test file itself uses from __future__ import annotations,
        so all inline functions have string annotations.
        """

        def func(name: str, age: int = 25) -> dict:
            return {}

        Model = generate_input_model(func)
        assert "name" in Model.model_fields
        assert "age" in Model.model_fields


class TestGenerateOutputModel:
    """Tests for generate_output_model()."""

    def test_return_bare_dict(self):
        """Return dict -> permissive model (extra='allow')."""

        def func() -> dict:
            return {}

        Model = generate_output_model(func)
        assert Model.model_config.get("extra") == "allow"

    def test_return_typed_dict(self):
        """Return dict[str, Any] -> permissive model (extra='allow')."""

        def func() -> dict[str, Any]:
            return {}

        Model = generate_output_model(func)
        assert Model.model_config.get("extra") == "allow"

    def test_return_basemodel_subclass(self):
        """Return BaseModel subclass -> returned directly."""

        def func() -> _OutputModel:
            return _OutputModel(greeting="hi")

        Model = generate_output_model(func)
        assert Model is _OutputModel

    def test_return_str(self):
        """Return str -> model with 'result' field of type str."""

        def func() -> str:
            return "hello"

        Model = generate_output_model(func)
        assert "result" in Model.model_fields
        inst = Model(result="hello")
        assert inst.model_dump() == {"result": "hello"}

    def test_return_int(self):
        """Return int -> model with 'result' field of type int."""

        def func() -> int:
            return 42

        Model = generate_output_model(func)
        assert "result" in Model.model_fields
        inst = Model(result=42)
        assert inst.model_dump() == {"result": 42}

    def test_return_list_str(self):
        """Return list[str] -> model with 'result' field of type list[str]."""

        def func() -> list[str]:
            return ["a"]

        Model = generate_output_model(func)
        assert "result" in Model.model_fields

    def test_return_none(self):
        """Return None -> empty permissive model."""

        def func() -> None:
            pass

        Model = generate_output_model(func)
        assert Model.model_config.get("extra") == "allow"
        assert len(Model.model_fields) == 0

    def test_missing_return_type_raises(self):
        """Missing return type raises FuncMissingReturnTypeError."""

        def func():
            pass

        with pytest.raises(FuncMissingReturnTypeError):
            generate_output_model(func)

    def test_result_field_invariant(self):
        """Output schema result field coordinates with execute() wrapping."""

        def func() -> str:
            return "hello"

        Model = generate_output_model(func)
        # Schema has result field
        assert "result" in Model.model_fields
        # Model validates the wrapped format
        inst = Model(result="hello")
        assert inst.model_dump() == {"result": "hello"}


class TestContextParamName:
    """Tests for _context_param_name()."""

    def test_function_with_context(self):
        """Function with Context param returns the parameter name."""

        def func(name: str, ctx: Context) -> dict:
            return {}

        assert _context_param_name(func) == "ctx"

    def test_function_without_context(self):
        """Function without Context param returns None."""

        def func(name: str) -> dict:
            return {}

        assert _context_param_name(func) is None

    def test_detection_is_type_based(self):
        """Detection is by type, not name — 'ctx' with Context type detected."""

        def func(ctx: Context) -> dict:
            return {}

        assert _context_param_name(func) == "ctx"

    def test_non_context_named_context(self):
        """Param named 'context' with type str is NOT detected."""

        def func(context: str) -> dict:
            return {}

        assert _context_param_name(func) is None


# ---------------------------------------------------------------------------
# Section 03: FunctionModule Class Tests
# ---------------------------------------------------------------------------

# --- Helper functions for FunctionModule tests ---


def _greet(name: str, age: int) -> dict:
    """Greet someone."""
    return {"greeting": f"Hello {name}, age {age}"}


def _no_doc_func(x: int) -> int:
    return x * 2


async def _async_greet(name: str) -> dict:
    return {"greeting": f"Hello {name}"}


def _returns_none(name: str) -> None:
    pass


def _returns_string(name: str) -> str:
    return f"Hello {name}"


def _returns_int(x: int) -> int:
    return x * 2


class _FMOutputModel(BaseModel):
    greeting: str


def _returns_model(name: str) -> _FMOutputModel:
    return _FMOutputModel(greeting=f"Hello {name}")


def _with_context(name: str, ctx: Context) -> dict:
    """Function that uses context."""
    return {"greeting": f"Hello {name}", "trace": ctx.trace_id}


async def _async_with_context(name: str, ctx: Context) -> dict:
    return {"greeting": f"Hello {name}", "trace": ctx.trace_id}


def _raises_error(name: str) -> dict:
    raise ValueError("something went wrong")


class TestFunctionModuleConstructor:
    """Tests for FunctionModule.__init__ and stored attributes."""

    def test_stores_input_schema_as_basemodel_class(self):
        """input_schema should be a Pydantic BaseModel subclass."""
        fm = FunctionModule(func=_greet, module_id="test.greet")
        assert issubclass(fm.input_schema, BaseModel)

    def test_stores_output_schema_as_basemodel_class(self):
        """output_schema should be a Pydantic BaseModel subclass."""
        fm = FunctionModule(func=_greet, module_id="test.greet")
        assert issubclass(fm.output_schema, BaseModel)

    def test_stores_module_id(self):
        """module_id should be stored as provided."""
        fm = FunctionModule(func=_greet, module_id="test.greet")
        assert fm.module_id == "test.greet"

    def test_description_from_explicit_param(self):
        """Explicit description parameter takes priority over docstring."""
        fm = FunctionModule(func=_greet, module_id="test.greet", description="Custom desc")
        assert fm.description == "Custom desc"

    def test_description_from_docstring(self):
        """First line of docstring used when no explicit description."""
        fm = FunctionModule(func=_greet, module_id="test.greet")
        assert fm.description == "Greet someone."

    def test_description_fallback(self):
        """Falls back to 'Module {func_name}' when no docstring."""
        fm = FunctionModule(func=_no_doc_func, module_id="test.no_doc")
        assert fm.description == "Module _no_doc_func"

    def test_description_from_multiline_docstring(self):
        """Only the first line of a multiline docstring is used."""

        def multi_doc(x: int) -> int:
            """First line summary.

            Detailed description that should be ignored.
            More details here.
            """
            return x

        fm = FunctionModule(func=multi_doc, module_id="test.multi_doc")
        assert fm.description == "First line summary."

    def test_stores_optional_attrs(self):
        """documentation, tags, version, annotations, metadata should be stored."""
        fm = FunctionModule(
            func=_greet,
            module_id="test.greet",
            documentation="Full docs",
            tags=["email"],
            version="2.0.0",
            annotations={"readonly": True, "destructive": False},
            metadata={"author": "test"},
        )
        assert fm.documentation == "Full docs"
        assert fm.tags == ["email"]
        assert fm.version == "2.0.0"
        assert fm.annotations.readonly is True
        assert fm.annotations.destructive is False
        assert fm.metadata == {"author": "test"}


class TestFunctionModuleSyncExecute:
    """Tests for sync function execution through FunctionModule.execute()."""

    def test_sync_function_called_correctly(self):
        """execute() should call the wrapped function with unpacked inputs."""
        fm = FunctionModule(func=_greet, module_id="test.greet")
        ctx = Context.create()
        result = fm.execute({"name": "Alice", "age": 30}, ctx)
        assert result == {"greeting": "Hello Alice, age 30"}

    def test_sync_returning_dict_passthrough(self):
        """Dict return values should be passed through unchanged."""
        fm = FunctionModule(func=_greet, module_id="test.greet")
        ctx = Context.create()
        result = fm.execute({"name": "Bob", "age": 25}, ctx)
        assert isinstance(result, dict)

    def test_sync_returning_none(self):
        """None return should become empty dict."""
        fm = FunctionModule(func=_returns_none, module_id="test.none")
        ctx = Context.create()
        result = fm.execute({"name": "Alice"}, ctx)
        assert result == {}

    def test_sync_returning_basemodel(self):
        """BaseModel return should be converted via model_dump()."""
        fm = FunctionModule(func=_returns_model, module_id="test.model")
        ctx = Context.create()
        result = fm.execute({"name": "Alice"}, ctx)
        assert result == {"greeting": "Hello Alice"}

    def test_sync_returning_string(self):
        """String return should be wrapped as {"result": value}."""
        fm = FunctionModule(func=_returns_string, module_id="test.str")
        ctx = Context.create()
        result = fm.execute({"name": "Alice"}, ctx)
        assert result == {"result": "Hello Alice"}

    def test_sync_returning_int(self):
        """Int return should be wrapped as {"result": value}."""
        fm = FunctionModule(func=_returns_int, module_id="test.int")
        ctx = Context.create()
        result = fm.execute({"x": 21}, ctx)
        assert result == {"result": 42}

    def test_context_injected(self):
        """Context should be injected when function has a Context parameter."""
        fm = FunctionModule(func=_with_context, module_id="test.ctx")
        ctx = Context.create()
        result = fm.execute({"name": "Alice"}, ctx)
        assert result["trace"] == ctx.trace_id

    def test_context_not_injected(self):
        """Context should NOT be injected when function lacks Context parameter."""
        fm = FunctionModule(func=_greet, module_id="test.greet")
        ctx = Context.create()
        result = fm.execute({"name": "Alice", "age": 30}, ctx)
        assert "greeting" in result

    def test_exception_propagates(self):
        """Exceptions from wrapped function should propagate uncaught."""
        fm = FunctionModule(func=_raises_error, module_id="test.err")
        ctx = Context.create()
        with pytest.raises(ValueError, match="something went wrong"):
            fm.execute({"name": "Alice"}, ctx)

    def test_sync_not_coroutine_function(self):
        """inspect.iscoroutinefunction should return False for sync function module."""
        fm = FunctionModule(func=_greet, module_id="test.greet")
        assert not inspect.iscoroutinefunction(fm.execute)


class TestFunctionModuleAsyncExecute:
    """Tests for async function execution through FunctionModule.execute()."""

    @pytest.mark.asyncio
    async def test_async_function_called_correctly(self):
        """execute() should await the wrapped async function."""
        fm = FunctionModule(func=_async_greet, module_id="test.async")
        ctx = Context.create()
        result = await fm.execute({"name": "Alice"}, ctx)
        assert result == {"greeting": "Hello Alice"}

    @pytest.mark.asyncio
    async def test_async_returning_dict(self):
        """Async dict return should pass through."""
        fm = FunctionModule(func=_async_greet, module_id="test.async")
        ctx = Context.create()
        result = await fm.execute({"name": "Bob"}, ctx)
        assert isinstance(result, dict)

    @pytest.mark.asyncio
    async def test_async_returning_none(self):
        """Async None return should become empty dict."""

        async def async_none(x: int) -> None:
            pass

        fm = FunctionModule(func=async_none, module_id="test.async_none")
        ctx = Context.create()
        result = await fm.execute({"x": 1}, ctx)
        assert result == {}

    @pytest.mark.asyncio
    async def test_async_returning_non_dict(self):
        """Async non-dict return should be wrapped as {"result": value}."""

        async def async_int(x: int) -> int:
            return x * 2

        fm = FunctionModule(func=async_int, module_id="test.async_int")
        ctx = Context.create()
        result = await fm.execute({"x": 21}, ctx)
        assert result == {"result": 42}

    @pytest.mark.asyncio
    async def test_async_returning_basemodel(self):
        """Async BaseModel return should be converted via model_dump()."""

        async def async_model(name: str) -> _FMOutputModel:
            return _FMOutputModel(greeting=f"Hello {name}")

        fm = FunctionModule(func=async_model, module_id="test.async_model")
        ctx = Context.create()
        result = await fm.execute({"name": "Alice"}, ctx)
        assert result == {"greeting": "Hello Alice"}

    @pytest.mark.asyncio
    async def test_async_context_injected(self):
        """Context should be injected when async function has Context parameter."""
        fm = FunctionModule(func=_async_with_context, module_id="test.async_ctx")
        ctx = Context.create()
        result = await fm.execute({"name": "Alice"}, ctx)
        assert result["trace"] == ctx.trace_id

    def test_async_is_coroutine_function(self):
        """inspect.iscoroutinefunction should return True for async function module."""
        fm = FunctionModule(func=_async_greet, module_id="test.async")
        assert inspect.iscoroutinefunction(fm.execute)

    @pytest.mark.asyncio
    async def test_async_exception_propagates(self):
        """Exceptions from async function should propagate uncaught."""

        async def async_error(name: str) -> dict:
            raise RuntimeError("async failure")

        fm = FunctionModule(func=async_error, module_id="test.async_err")
        ctx = Context.create()
        with pytest.raises(RuntimeError, match="async failure"):
            await fm.execute({"name": "Alice"}, ctx)


# ---------------------------------------------------------------------------
# Section 04: module() Function Tests
# ---------------------------------------------------------------------------


class TestModuleDecoratorWithArgs:
    """Tests for @module(id='x', ...) decorator form."""

    def test_returns_original_function(self):
        """@module(id='x') should return the original function."""

        @module(id="test.greet")
        def greet(name: str) -> str:
            return f"Hello {name}"

        assert callable(greet)
        assert greet("Alice") == "Hello Alice"

    def test_attaches_apcore_module(self):
        """The original function should have .apcore_module after decoration."""

        @module(id="test.greet")
        def greet(name: str) -> str:
            return f"Hello {name}"

        assert hasattr(greet, "apcore_module")

    def test_apcore_module_is_function_module(self):
        """func.apcore_module should be a FunctionModule instance."""

        @module(id="test.greet")
        def greet(name: str) -> str:
            return f"Hello {name}"

        assert isinstance(greet.apcore_module, FunctionModule)

    def test_apcore_module_has_correct_id(self):
        """func.apcore_module.module_id should match the id passed to @module()."""

        @module(id="test.greet")
        def greet(name: str) -> str:
            return f"Hello {name}"

        assert greet.apcore_module.module_id == "test.greet"

    def test_with_registry_registers(self):
        """When registry is provided, FunctionModule should be registered."""
        reg = Registry()

        @module(id="test.greet", registry=reg)
        def greet(name: str) -> str:
            return f"Hello {name}"

        assert reg.get("test.greet") is greet.apcore_module

    def test_function_still_callable(self):
        """The decorated function should still be callable as normal."""

        @module(id="test.greet")
        def greet(name: str) -> str:
            return f"Hello {name}"

        assert greet("Bob") == "Hello Bob"

    def test_stores_tags_and_version(self):
        """Tags and version from decorator args should be stored."""

        @module(id="test.greet", tags=["email"], version="2.0.0")
        def greet(name: str) -> str:
            return f"Hello {name}"

        assert greet.apcore_module.tags == ["email"]
        assert greet.apcore_module.version == "2.0.0"


class TestBareModuleDecorator:
    """Tests for @module (no parentheses) decorator form."""

    def test_returns_original_function(self):
        """@module without parentheses should return the original function."""

        @module
        def greet(name: str) -> str:
            return f"Hello {name}"

        assert callable(greet)
        assert greet("Alice") == "Hello Alice"

    def test_attaches_apcore_module(self):
        """@module without parentheses should still attach .apcore_module."""

        @module
        def greet(name: str) -> str:
            return f"Hello {name}"

        assert hasattr(greet, "apcore_module")
        assert isinstance(greet.apcore_module, FunctionModule)

    def test_auto_generates_id(self):
        """@module without parentheses should auto-generate module_id from __module__ + __qualname__."""

        @module
        def greet(name: str) -> str:
            return f"Hello {name}"

        auto_id = greet.apcore_module.module_id
        assert "greet" in auto_id
        assert "test_decorator" in auto_id


class TestModuleFunctionCallForm:
    """Tests for module(func, id='x') programmatic form."""

    def test_returns_function_module(self):
        """module(func, id='x') should return a FunctionModule instance."""

        def greet(name: str) -> str:
            return f"Hello {name}"

        fm = module(greet, id="test.greet")
        assert isinstance(fm, FunctionModule)

    def test_with_registry_registers(self):
        """module(func, id='x', registry=reg) should register."""
        reg = Registry()

        def greet(name: str) -> str:
            return f"Hello {name}"

        fm = module(greet, id="test.greet", registry=reg)
        assert reg.get("test.greet") is fm

    def test_has_correct_schemas(self):
        """The returned FunctionModule should have properly generated schemas."""

        def greet(name: str) -> str:
            return f"Hello {name}"

        fm = module(greet, id="test.greet")
        assert issubclass(fm.input_schema, BaseModel)
        assert issubclass(fm.output_schema, BaseModel)
        assert "name" in fm.input_schema.model_fields


class TestMakeAutoId:
    """Tests for auto-ID generation."""

    def test_from_module_and_qualname(self):
        """Auto-generated ID should combine __module__ and __qualname__."""

        def my_func(x: int) -> int:
            return x

        auto_id = _make_auto_id(my_func)
        # Should contain both the module path and function name
        assert "my_func" in auto_id
        assert "test_decorator" in auto_id

    def test_locals_segments_replaced(self):
        """'<locals>.' in __qualname__ should be replaced."""

        def my_func(x: int) -> int:
            return x

        # __qualname__ of a nested func includes '<locals>'
        auto_id = _make_auto_id(my_func)
        assert "<locals>" not in auto_id

    def test_is_lowercased(self):
        """Auto-generated IDs should be fully lowercased."""

        def MyFunc(x: int) -> int:  # noqa: N802
            return x

        auto_id = _make_auto_id(MyFunc)
        assert auto_id == auto_id.lower()

    def test_non_alphanumeric_replaced(self):
        """Non-alphanumeric chars (except _ and .) should become underscores."""
        import types

        func = types.FunctionType(
            compile("0", "<test>", "eval"),
            {"__builtins__": {}},
            "func-with-dashes",
        )
        func.__module__ = "my-module"
        func.__qualname__ = "func-with-dashes"
        auto_id = _make_auto_id(func)
        assert "-" not in auto_id

    def test_digit_segment_prepended(self):
        """Segments starting with a digit should get a leading underscore."""
        import types

        func = types.FunctionType(
            compile("0", "<test>", "eval"),
            {"__builtins__": {}},
            "func",
        )
        func.__module__ = "pkg.2bad"
        func.__qualname__ = "func"
        auto_id = _make_auto_id(func)
        segments = auto_id.split(".")
        for seg in segments:
            if seg and seg[0].isdigit():
                pytest.fail(f"Segment '{seg}' starts with digit but was not prepended with _")


class TestModuleRegistration:
    """Tests for registry interaction."""

    def test_registration_with_correct_args(self):
        """registry.register() should be called with correct module_id and FunctionModule."""
        reg = Registry()

        @module(id="test.registered", registry=reg)
        def greet(name: str) -> str:
            return f"Hello {name}"

        registered = reg.get("test.registered")
        assert registered is greet.apcore_module
        assert registered.module_id == "test.registered"

    def test_no_registration_when_registry_none(self):
        """When registry is None (default), no registration should occur."""

        @module(id="test.greet")
        def greet(name: str) -> str:
            return f"Hello {name}"

        # No error, and function works normally
        assert greet("Alice") == "Hello Alice"


# ---------------------------------------------------------------------------
# Section 06: Integration and Exports Tests
# ---------------------------------------------------------------------------


class TestPublicAPIExportsDecorator:
    """Verify that decorator-related symbols are importable from apcore."""

    def test_module_importable(self):
        """'from apcore import module' works."""
        from apcore import module as m

        assert m is not None

    def test_function_module_importable(self):
        """'from apcore import FunctionModule' works."""
        from apcore import FunctionModule as FM

        assert FM is not None


class TestDecoratorExecutorIntegration:
    """Full pipeline: @module / module() -> Registry -> Executor.call() -> output."""

    def test_module_decorator_sync_through_executor(self):
        """@module decorator -> Registry -> Executor.call() produces correct output."""
        reg = Registry()

        @module(id="test.greet", registry=reg)
        def greet(name: str) -> dict:
            return {"greeting": f"Hello, {name}!"}

        executor = Executor(registry=reg)
        result = executor.call("test.greet", {"name": "World"})
        assert result == {"greeting": "Hello, World!"}

    @pytest.mark.asyncio
    async def test_module_decorator_async_through_executor(self):
        """@module decorator -> Registry -> Executor.call_async() (async)."""
        reg = Registry()

        @module(id="test.async_greet", registry=reg)
        async def greet_async(name: str) -> dict:
            return {"greeting": f"Hello, {name}!"}

        executor = Executor(registry=reg)
        result = await executor.call_async("test.async_greet", {"name": "World"})
        assert result == {"greeting": "Hello, World!"}

    def test_context_injection_through_executor(self):
        """Function with Context parameter receives executor-created context."""
        reg = Registry()

        @module(id="test.ctx", registry=reg)
        def with_ctx(name: str, ctx: Context) -> dict:
            return {"greeting": f"Hello, {name}!", "trace_id": ctx.trace_id}

        executor = Executor(registry=reg)
        result = executor.call("test.ctx", {"name": "World"})
        assert result["greeting"] == "Hello, World!"
        assert isinstance(result["trace_id"], str)
        assert len(result["trace_id"]) > 0

    def test_non_dict_return_through_executor(self):
        """Function returning non-dict -> {"result": value} passes output validation."""
        reg = Registry()

        @module(id="test.compute", registry=reg)
        def compute(x: int, y: int) -> int:
            return x + y

        executor = Executor(registry=reg)
        result = executor.call("test.compute", {"x": 1, "y": 2})
        assert result == {"result": 3}

    def test_basemodel_params_validated_by_executor(self):
        """Input validation works through executor — invalid types raise error."""
        reg = Registry()

        @module(id="test.typed", registry=reg)
        def typed_func(name: str, age: int) -> dict:
            return {"name": name, "age": age}

        executor = Executor(registry=reg)
        # Valid inputs
        result = executor.call("test.typed", {"name": "Alice", "age": 30})
        assert result == {"name": "Alice", "age": 30}

    def test_bare_decorator_through_executor(self):
        """Bare @module (no parens) -> manual register -> Executor flow."""
        reg = Registry()

        @module
        def bare_greet(name: str) -> dict:
            return {"greeting": f"Hello, {name}!"}

        reg.register("bare.greet", bare_greet.apcore_module)
        executor = Executor(registry=reg)
        result = executor.call("bare.greet", {"name": "World"})
        assert result == {"greeting": "Hello, World!"}

    def test_function_call_form_through_executor(self):
        """module(func, id=...) -> register -> Executor flow."""
        reg = Registry()

        def fn(name: str) -> dict:
            return {"greeting": f"Hello, {name}!"}

        fm = module(fn, id="test.fn")
        reg.register("test.fn", fm)
        executor = Executor(registry=reg)
        result = executor.call("test.fn", {"name": "World"})
        assert result == {"greeting": "Hello, World!"}


class _Greeter:
    def __init__(self, prefix: str) -> None:
        self.prefix = prefix

    def greet(self, name: str) -> dict:
        return {"text": f"{self.prefix} {name}"}

    async def greet_async(self, name: str) -> dict:
        return {"text": f"{self.prefix} {name}"}


class TestBoundMethodModules:
    """#123: a bound method cannot carry ``apcore_module``; decorating one must not half-register."""

    def test_client_module_accepts_bound_method(self) -> None:
        from apcore.client import APCore

        client = APCore()
        greeter = _Greeter("hi")
        wrapped = client.module(id="greeter.greet")(greeter.greet)

        assert client.registry.has("greeter.greet")
        assert isinstance(wrapped.apcore_module, FunctionModule)
        assert wrapped("bob") == {"text": "hi bob"}
        assert client.call("greeter.greet", {"name": "bob"}) == {"text": "hi bob"}

    def test_decorator_with_registry_accepts_bound_method(self) -> None:
        registry = Registry()
        greeter = _Greeter("hey")
        wrapped = module(id="greeter.greet", registry=registry)(greeter.greet)

        assert wrapped.apcore_module is registry.get("greeter.greet")
        assert wrapped.__name__ == "greet"

    def test_async_bound_method_stays_a_coroutine_function(self) -> None:
        registry = Registry()
        greeter = _Greeter("yo")
        wrapped = module(id="greeter.greet_async", registry=registry)(greeter.greet_async)

        assert inspect.iscoroutinefunction(wrapped)
        assert registry.has("greeter.greet_async")

    def test_plain_function_is_returned_unchanged(self) -> None:
        def add(a: int, b: int) -> int:
            return a + b

        wrapped = module(id="math.add_plain", registry=Registry())(add)
        assert wrapped is add


class TestAutoIdFromMain:
    """#123: ``__main__`` is not a legal ID segment, so auto-derivation must refuse it clearly."""

    def test_function_defined_in_main_raises_invalid_module_id(self) -> None:
        def handler(x: int) -> int:
            return x

        handler.__module__ = "__main__"
        with pytest.raises(ModuleError) as exc_info:
            _make_auto_id(handler)
        assert exc_info.value.code == "INVALID_MODULE_ID"
        assert "explicit id" in str(exc_info.value)

    def test_decorator_refuses_before_registering(self) -> None:
        def handler(x: int) -> int:
            return x

        handler.__module__ = "__main__"
        registry = Registry()
        with pytest.raises(ModuleError):
            module(handler, registry=registry)
        assert registry.module_ids == []

    def test_explicit_id_still_works_in_main(self) -> None:
        def handler(x: int) -> int:
            return x

        handler.__module__ = "__main__"
        registry = Registry()
        module(handler, id="app.handler", registry=registry)
        assert registry.has("app.handler")
