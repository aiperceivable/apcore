"""Integration tests for the middleware chain through the Executor."""

from __future__ import annotations


from apcore.executor import Executor
from apcore.middleware import Middleware


class _RecordingMiddleware(Middleware):
    """Middleware that records the order of before/after/on_error calls."""

    def __init__(self, name: str, log: list):
        self.name = name
        self.log = log

    def before(self, module_id, inputs, context):
        self.log.append(f"{self.name}.before")
        return None

    def after(self, module_id, inputs, output, context):
        self.log.append(f"{self.name}.after")
        return None

    def on_error(self, module_id, inputs, error, context):
        self.log.append(f"{self.name}.on_error")
        return None


class TestMiddlewareChain:
    """Middleware chain ordering and behavior integration tests."""

    def test_before_hooks_execute_in_registration_order(self, int_registry):
        log = []
        mw_a = _RecordingMiddleware("A", log)
        mw_b = _RecordingMiddleware("B", log)
        executor = Executor(registry=int_registry, middlewares=[mw_a, mw_b])
        executor.call("greet", {"name": "Alice"})
        before_entries = [e for e in log if "before" in e]
        assert before_entries == ["A.before", "B.before"]

    def test_after_hooks_execute_in_reverse_registration_order(self, int_registry):
        log = []
        mw_a = _RecordingMiddleware("A", log)
        mw_b = _RecordingMiddleware("B", log)
        executor = Executor(registry=int_registry, middlewares=[mw_a, mw_b])
        executor.call("greet", {"name": "Alice"})
        after_entries = [e for e in log if "after" in e]
        assert after_entries == ["B.after", "A.after"]

    def test_before_middleware_can_modify_inputs(self, int_registry):
        class InputModifier(Middleware):
            def before(self, module_id, inputs, context):
                return {"name": "Modified"}

        executor = Executor(registry=int_registry, middlewares=[InputModifier()])
        result = executor.call("greet", {"name": "Original"})
        assert result == {"message": "Hello, Modified!"}

    def test_after_middleware_can_modify_output(self, int_registry):
        class OutputModifier(Middleware):
            def after(self, module_id, inputs, output, context):
                return {"message": output["message"] + " [modified]"}

        executor = Executor(registry=int_registry, middlewares=[OutputModifier()])
        result = executor.call("greet", {"name": "Alice"})
        assert result == {"message": "Hello, Alice! [modified]"}

    def test_on_error_recovery_suppresses_error(self, int_registry):
        class RecoveryMiddleware(Middleware):
            def on_error(self, module_id, inputs, error, context):
                return {"recovered": True}

        executor = Executor(registry=int_registry, middlewares=[RecoveryMiddleware()])
        result = executor.call("failing", {})
        assert result == {"recovered": True}

    def test_on_error_cascade_stops_at_first_recovery(self, int_registry):
        log = []

        class FirstRecovery(Middleware):
            def on_error(self, module_id, inputs, error, context):
                log.append("first.on_error")
                return {"recovered": "first"}

        class SecondRecovery(Middleware):
            def on_error(self, module_id, inputs, error, context):
                log.append("second.on_error")
                return {"recovered": "second"}

        executor = Executor(registry=int_registry, middlewares=[FirstRecovery(), SecondRecovery()])
        result = executor.call("failing", {})
        # SecondRecovery fires first (reverse order), returns recovery.
        assert result == {"recovered": "second"}
        assert log == ["second.on_error"]
