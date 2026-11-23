"""Conformance tests for call_with_trace cancellation semantics (A-D-001) fixture.

When the pipeline raises ExecutionCancelledError mid-execution, the trace
variant MUST propagate it directly (code EXECUTION_CANCELLED) and MUST NOT route
it through the on_error middleware chain — an on_error middleware that would
normally recover MUST NOT be able to suppress a cancellation. call() and
call_with_trace() must behave identically here.
"""

from __future__ import annotations

from typing import Any

import pytest

from apcore.cancel import ExecutionCancelledError
from apcore.context import Context
from apcore.executor import Executor
from apcore.middleware import Middleware
from apcore.registry import Registry
from conformance.canonical_fixtures import load_fixture


def _load_fixture() -> dict:
    """Load the canonical fixture from the apcore spec repo."""
    return load_fixture("executor_trace_cancellation.json")


_FIXTURE = _load_fixture()


class _CancellingModule:
    """Module whose execute() raises ExecutionCancelledError mid-pipeline."""

    input_schema = None
    output_schema = None

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        raise ExecutionCancelledError("cancelled inside module body")


class _RecoveringRecorderMiddleware(Middleware):
    """on_error middleware that records invocation and would otherwise recover."""

    def __init__(self) -> None:
        super().__init__()
        self.on_error_invoked = False

    def on_error(
        self,
        module_id: str,
        inputs: dict[str, Any],
        error: Exception,
        context: Context,
    ) -> dict[str, Any]:
        self.on_error_invoked = True
        # Would normally swallow the error and recover with a success result.
        return {"recovered": True}


def _build() -> tuple[Executor, _RecoveringRecorderMiddleware]:
    reg = Registry()
    reg.register("test.cancelling", _CancellingModule())
    mw = _RecoveringRecorderMiddleware()
    executor = Executor(registry=reg, middlewares=[mw])
    return executor, mw


@pytest.mark.parametrize("case", _FIXTURE["test_cases"], ids=[c["id"] for c in _FIXTURE["test_cases"]])
def test_trace_cancellation_bypasses_on_error(case: dict) -> None:
    # --- call_with_trace: cancellation propagates, on_error bypassed ---
    executor, mw = _build()
    with pytest.raises(ExecutionCancelledError) as exc_info:
        executor.call_with_trace("test.cancelling", {})
    assert exc_info.value.code == case["expected_error"]
    assert (
        mw.on_error_invoked is case["expected_on_error_invoked"]
    ), "on_error middleware must NOT be invoked for a cancellation in call_with_trace"

    # --- call(): MUST behave identically ---
    executor2, mw2 = _build()
    with pytest.raises(ExecutionCancelledError) as exc_info2:
        executor2.call("test.cancelling", {})
    assert exc_info2.value.code == case["expected_error"]
    assert (
        mw2.on_error_invoked is case["expected_on_error_invoked"]
    ), "on_error middleware must NOT be invoked for a cancellation in call()"
