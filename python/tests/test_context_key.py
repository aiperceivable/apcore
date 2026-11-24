"""Tests for ContextKey typed accessor."""

import pytest

from apcore.context import Context
from apcore.context_key import ContextKey


class TestContextKey:
    """Test suite for ContextKey<T> typed accessor."""

    def _make_ctx(self) -> Context:
        """Create a minimal Context for testing."""
        return Context.create()

    def test_get_returns_typed_value(self) -> None:
        """AC-001: get() returns typed value from context.data."""
        key: ContextKey[int] = ContextKey("test.counter")
        ctx = self._make_ctx()
        ctx.data["test.counter"] = 42
        assert key.get(ctx) == 42

    def test_get_absent_returns_none(self) -> None:
        """AC-016: get() with absent key returns None by default."""
        key: ContextKey[int] = ContextKey("test.absent")
        ctx = self._make_ctx()
        assert key.get(ctx) is None

    def test_get_absent_returns_default(self) -> None:
        """AC-016: get() with absent key returns provided default."""
        key: ContextKey[int] = ContextKey("test.absent")
        ctx = self._make_ctx()
        assert key.get(ctx, default=99) == 99

    def test_get_distinguishes_none_from_absent(self) -> None:
        """get() with value=None stored should return None, not default."""
        key: ContextKey[int | None] = ContextKey("test.nullable")
        ctx = self._make_ctx()
        ctx.data["test.nullable"] = None
        assert key.get(ctx, default=99) is None

    def test_set_writes_to_data(self) -> None:
        """AC-001: set() writes value to context.data."""
        key: ContextKey[str] = ContextKey("test.name")
        ctx = self._make_ctx()
        key.set(ctx, "hello")
        assert ctx.data["test.name"] == "hello"

    def test_delete_removes_key(self) -> None:
        """delete() removes key from context.data."""
        key: ContextKey[int] = ContextKey("test.temp")
        ctx = self._make_ctx()
        key.set(ctx, 10)
        key.delete(ctx)
        assert "test.temp" not in ctx.data

    def test_delete_absent_is_noop(self) -> None:
        """AC-017: delete() on absent key is no-op, no exception."""
        key: ContextKey[int] = ContextKey("test.absent")
        ctx = self._make_ctx()
        key.delete(ctx)  # Should not raise

    def test_exists_false_when_absent(self) -> None:
        """AC-018: exists() returns False for absent key."""
        key: ContextKey[int] = ContextKey("test.absent")
        ctx = self._make_ctx()
        assert key.exists(ctx) is False

    def test_exists_true_when_present(self) -> None:
        """AC-018: exists() returns True after set."""
        key: ContextKey[int] = ContextKey("test.present")
        ctx = self._make_ctx()
        key.set(ctx, 1)
        assert key.exists(ctx) is True

    def test_scoped_creates_subkey(self) -> None:
        """AC-002: scoped(suffix) creates sub-key with {name}.{suffix}."""
        base: ContextKey[int] = ContextKey("_apcore.mw.retry.count")
        scoped = base.scoped("mod1")
        assert scoped.name == "_apcore.mw.retry.count.mod1"

    def test_scoped_key_is_independent(self) -> None:
        """Scoped key operates on its own data slot."""
        base: ContextKey[int] = ContextKey("base")
        scoped = base.scoped("child")
        ctx = self._make_ctx()
        base.set(ctx, 1)
        scoped.set(ctx, 2)
        assert base.get(ctx) == 1
        assert scoped.get(ctx) == 2

    def test_frozen_dataclass(self) -> None:
        """ContextKey is immutable (frozen dataclass)."""
        key: ContextKey[int] = ContextKey("test")
        with pytest.raises(AttributeError):
            key.name = "changed"  # type: ignore[misc]
