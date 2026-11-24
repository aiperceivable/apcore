"""Tests for CircularDependencyError."""

from __future__ import annotations

from apcore.errors import CircularDependencyError, ModuleError


class TestCircularDependencyError:
    def test_code_is_circular_dependency(self) -> None:
        """CircularDependencyError sets code='CIRCULAR_DEPENDENCY'."""
        err = CircularDependencyError(cycle_path=["A", "B", "A"])
        assert err.code == "CIRCULAR_DEPENDENCY"

    def test_stores_cycle_path_in_details(self) -> None:
        """cycle_path is stored in details."""
        err = CircularDependencyError(cycle_path=["A", "B", "C", "A"])
        assert err.details["cycle_path"] == ["A", "B", "C", "A"]

    def test_message_includes_cycle_info(self) -> None:
        """Message includes cycle path members."""
        err = CircularDependencyError(cycle_path=["A", "B", "A"])
        msg = str(err)
        assert "A" in msg
        assert "B" in msg

    def test_is_subclass_of_module_error(self) -> None:
        """CircularDependencyError extends ModuleError."""
        assert issubclass(CircularDependencyError, ModuleError)

    def test_is_subclass_of_exception(self) -> None:
        """CircularDependencyError extends Exception."""
        assert issubclass(CircularDependencyError, Exception)

    def test_supports_cause_and_trace_id(self) -> None:
        """Supports cause and trace_id kwargs."""
        cause = ValueError("test")
        err = CircularDependencyError(cycle_path=["X", "Y", "X"], cause=cause, trace_id="t-123")
        assert err.cause is cause
        assert err.trace_id == "t-123"
