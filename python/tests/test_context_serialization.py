"""Tests for Context serialize() / deserialize() protocol."""

from __future__ import annotations

import logging

import pytest

from apcore.context import Context, Identity


class TestContextSerialize:
    """AC-003, AC-004, AC-005: Context.serialize() protocol compliance."""

    def _make_ctx(self) -> Context:
        ctx = Context.create()
        ctx.identity = Identity(
            id="user-1",
            type="user",
            roles=("admin",),
            attrs={"org": "acme"},
        )
        return ctx

    def test_serialize_includes_context_version(self) -> None:
        """AC-003: serialization includes _context_version: 1."""
        ctx = self._make_ctx()
        result = ctx.serialize()
        assert result["_context_version"] == 1

    def test_serialize_includes_required_fields(self) -> None:
        """Serialized output includes trace_id, caller_id, call_chain, identity."""
        ctx = self._make_ctx()
        result = ctx.serialize()
        assert "trace_id" in result
        assert "caller_id" in result
        assert "call_chain" in result
        assert "identity" in result

    def test_serialize_identity_structure(self) -> None:
        """Identity serializes with id, type, roles, attrs."""
        ctx = self._make_ctx()
        result = ctx.serialize()
        identity = result["identity"]
        assert identity["id"] == "user-1"
        assert identity["type"] == "user"
        assert identity["roles"] == ["admin"]
        assert identity["attrs"] == {"org": "acme"}

    def test_serialize_excludes_executor(self) -> None:
        """AC-004: executor is not in serialized output."""
        ctx = self._make_ctx()
        result = ctx.serialize()
        assert "executor" not in result

    def test_serialize_excludes_services(self) -> None:
        """AC-004: services is not in serialized output."""
        ctx = self._make_ctx()
        result = ctx.serialize()
        assert "services" not in result

    def test_serialize_excludes_cancel_token(self) -> None:
        """AC-004: cancel_token is not in serialized output."""
        ctx = self._make_ctx()
        result = ctx.serialize()
        assert "cancel_token" not in result

    def test_serialize_excludesglobal_deadline(self) -> None:
        """AC-004: global_deadline is not in serialized output."""
        ctx = self._make_ctx()
        result = ctx.serialize()
        assert "global_deadline" not in result

    def test_serialize_filters_underscore_data_keys(self) -> None:
        """AC-005: _-prefixed keys excluded from serialized data."""
        ctx = self._make_ctx()
        ctx.data["_apcore.mw.metrics.starts"] = [1, 2, 3]
        ctx.data["_internal"] = "hidden"
        ctx.data["public.counter"] = 42
        ctx.data["app.name"] = "test"
        result = ctx.serialize()
        assert "_apcore.mw.metrics.starts" not in result["data"]
        assert "_internal" not in result["data"]
        assert result["data"]["public.counter"] == 42
        assert result["data"]["app.name"] == "test"

    def test_serialize_empty_data(self) -> None:
        """Serialization with no public data keys produces empty data dict."""
        ctx = self._make_ctx()
        ctx.data["_private"] = "hidden"
        result = ctx.serialize()
        assert result["data"] == {}


class TestContextDeserialize:
    """Deserialization protocol compliance."""

    def test_deserialize_roundtrip(self) -> None:
        """Serialize then deserialize preserves fields."""
        ctx = Context.create()
        ctx.identity = Identity(id="user-1", type="user", roles=("admin",), attrs={"org": "acme"})
        ctx.data["app.counter"] = 42
        serialized = ctx.serialize()
        restored = Context.deserialize(serialized)
        assert restored.trace_id == ctx.trace_id
        assert restored.caller_id == ctx.caller_id
        assert restored.data.get("app.counter") == 42
        assert restored.identity is not None
        assert restored.identity.id == "user-1"

    def test_deserialize_executor_is_none(self) -> None:
        """After deserialization, executor is None."""
        ctx = Context.create()
        # Simulate a Context that an Executor has already bound itself to.
        # Under the unified Context.create() signature (Issue #66), executor
        # is no longer a constructor input; binding happens via the private
        # helper.
        ctx._bind_executor("some-executor")
        serialized = ctx.serialize()
        restored = Context.deserialize(serialized)
        assert restored.executor is None

    def test_deserialize_services_is_none(self) -> None:
        """After deserialization, services is None."""
        ctx = Context.create()
        serialized = ctx.serialize()
        restored = Context.deserialize(serialized)
        assert restored.services is None

    def test_deserialize_cancel_token_is_none(self) -> None:
        """After deserialization, cancel_token is None."""
        ctx = Context.create()
        serialized = ctx.serialize()
        restored = Context.deserialize(serialized)
        assert restored.cancel_token is None

    def test_deserializeglobal_deadline_is_none(self) -> None:
        """After deserialization, global_deadline is None."""
        ctx = Context.create()
        serialized = ctx.serialize()
        restored = Context.deserialize(serialized)
        assert restored.global_deadline is None

    def test_deserialize_future_version_warns(self, caplog: pytest.LogCaptureFixture) -> None:
        """Deserializing _context_version > 1 logs warning but succeeds."""
        data: dict = {
            "_context_version": 99,
            "trace_id": "abc-123",
            "caller_id": "test",
            "call_chain": [],
            "data": {},
        }
        with caplog.at_level(logging.WARNING):
            restored = Context.deserialize(data)
        assert restored.trace_id == "abc-123"
        assert any("_context_version" in msg for msg in caplog.messages)

    def test_deserialize_preserves_unknown_fields_in_data(self) -> None:
        """Unknown top-level fields are preserved for forward compatibility."""
        data: dict = {
            "_context_version": 1,
            "trace_id": "abc-123",
            "caller_id": "test",
            "call_chain": [],
            "data": {"custom_key": "preserved"},
            "future_field": "should_not_crash",
        }
        restored = Context.deserialize(data)
        assert restored.trace_id == "abc-123"
        assert restored.data.get("custom_key") == "preserved"
