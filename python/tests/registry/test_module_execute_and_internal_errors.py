"""Tests for ModuleExecuteError and InternalError."""

from __future__ import annotations

from apcore.errors import InternalError, ModuleError, ModuleExecuteError


class TestModuleExecuteError:
    def test_code_is_module_execute_error(self) -> None:
        """ModuleExecuteError sets code='MODULE_EXECUTE_ERROR'."""
        err = ModuleExecuteError(module_id="mod.a")
        assert err.code == "MODULE_EXECUTE_ERROR"

    def test_default_message(self) -> None:
        """Default message is 'Module execution failed'."""
        err = ModuleExecuteError()
        assert err.message == "Module execution failed"

    def test_custom_message(self) -> None:
        """Custom message overrides the default."""
        err = ModuleExecuteError(module_id="mod.a", message="custom boom")
        assert err.message == "custom boom"

    def test_default_module_id(self) -> None:
        """Default module_id is empty string."""
        err = ModuleExecuteError()
        assert err.details["module_id"] == ""

    def test_stores_module_id_in_details(self) -> None:
        """module_id is stored in details."""
        err = ModuleExecuteError(module_id="my.mod")
        assert err.details["module_id"] == "my.mod"

    def test_str_includes_code_and_message(self) -> None:
        """str() includes code and message."""
        err = ModuleExecuteError(module_id="mod.x", message="something broke")
        text = str(err)
        assert "MODULE_EXECUTE_ERROR" in text
        assert "something broke" in text

    def test_is_subclass_of_module_error(self) -> None:
        """ModuleExecuteError extends ModuleError."""
        assert issubclass(ModuleExecuteError, ModuleError)

    def test_is_subclass_of_exception(self) -> None:
        """ModuleExecuteError extends Exception."""
        assert issubclass(ModuleExecuteError, Exception)

    def test_supports_cause_and_trace_id(self) -> None:
        """Supports cause and trace_id kwargs."""
        cause = ValueError("underlying")
        err = ModuleExecuteError(module_id="mod.a", cause=cause, trace_id="t-456")
        assert err.cause is cause
        assert err.trace_id == "t-456"

    def test_has_timestamp(self) -> None:
        """Timestamp is set on instantiation."""
        err = ModuleExecuteError()
        assert err.timestamp is not None
        assert isinstance(err.timestamp, str)


class TestInternalError:
    def test_code_is_general_internal_error(self) -> None:
        """InternalError sets code='GENERAL_INTERNAL_ERROR'."""
        err = InternalError()
        assert err.code == "GENERAL_INTERNAL_ERROR"

    def test_default_message(self) -> None:
        """Default message is 'Internal error'."""
        err = InternalError()
        assert err.message == "Internal error"

    def test_custom_message(self) -> None:
        """Custom message overrides the default."""
        err = InternalError(message="unexpected state")
        assert err.message == "unexpected state"

    def test_str_includes_code_and_message(self) -> None:
        """str() includes code and message."""
        err = InternalError(message="kaboom")
        text = str(err)
        assert "GENERAL_INTERNAL_ERROR" in text
        assert "kaboom" in text

    def test_is_subclass_of_module_error(self) -> None:
        """InternalError extends ModuleError."""
        assert issubclass(InternalError, ModuleError)

    def test_is_subclass_of_exception(self) -> None:
        """InternalError extends Exception."""
        assert issubclass(InternalError, Exception)

    def test_supports_cause_and_trace_id(self) -> None:
        """Supports cause and trace_id kwargs."""
        cause = RuntimeError("oops")
        err = InternalError(message="internal fail", cause=cause, trace_id="t-789")
        assert err.cause is cause
        assert err.trace_id == "t-789"

    def test_has_timestamp(self) -> None:
        """Timestamp is set on instantiation."""
        err = InternalError()
        assert err.timestamp is not None
        assert isinstance(err.timestamp, str)

    def test_details_default_empty(self) -> None:
        """Details default to empty dict when not provided."""
        err = InternalError()
        assert err.details == {}
