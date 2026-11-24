"""End-to-end integration tests for the apcore pipeline."""

from __future__ import annotations

import re


from apcore.context import Context
from apcore.executor import Executor
from apcore.middleware import Middleware


class _ContextRecorder(Middleware):
    """Captures context objects from before() calls."""

    def __init__(self):
        self.contexts: list[Context] = []

    def before(self, module_id, inputs, context):
        self.contexts.append(context)
        return None


class TestEndToEndFlow:
    """Full pipeline: discover -> register -> call -> verify."""

    def test_discover_register_call_verify(self, int_executor):
        result = int_executor.call("greet", {"name": "Alice"})
        assert result == {"message": "Hello, Alice!"}

    def test_trace_id_is_32_hex_format(self, int_registry):
        recorder = _ContextRecorder()
        executor = Executor(registry=int_registry, middlewares=[recorder])
        executor.call("greet", {"name": "Alice"})
        assert len(recorder.contexts) == 1
        trace_id = recorder.contexts[0].trace_id
        pattern = r"^[0-9a-f]{32}$"
        assert re.match(pattern, trace_id)

    def test_trace_id_propagates_through_nested_calls(self):
        ctx = Context.create()
        child = ctx.child("module.a")
        assert child.trace_id == ctx.trace_id
        grandchild = child.child("module.b")
        assert grandchild.trace_id == ctx.trace_id

    def test_call_chain_grows_with_nested_calls(self):
        ctx = Context.create()
        child = ctx.child("module.a")
        assert child.call_chain == ["module.a"]
        grandchild = child.child("module.b")
        assert grandchild.call_chain == ["module.a", "module.b"]

    def test_fresh_context_no_state_leakage(self, int_registry):
        recorder = _ContextRecorder()
        executor = Executor(registry=int_registry, middlewares=[recorder])
        executor.call("greet", {"name": "Alice"})
        executor.call("greet", {"name": "Bob"})
        assert len(recorder.contexts) == 2
        assert recorder.contexts[0].trace_id != recorder.contexts[1].trace_id
