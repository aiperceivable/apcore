"""Tests for the Middleware base class, function adapters, and priority ordering."""

from __future__ import annotations

import abc
from typing import Any
from unittest.mock import MagicMock, Mock

from apcore.context import Context
from apcore.middleware import AfterMiddleware, BeforeMiddleware, Middleware
from apcore.middleware.manager import MiddlewareManager


# === Middleware Base Class ===


class TestMiddlewareBase:
    """Tests for the Middleware base class."""

    def test_is_not_abc(self) -> None:
        """Middleware is a plain class, not an ABC."""
        assert not issubclass(Middleware, abc.ABC)

    def test_can_be_instantiated_directly(self) -> None:
        """Middleware can be instantiated without raising TypeError."""
        mw = Middleware()
        assert isinstance(mw, Middleware)

    def test_before_returns_none_by_default(self) -> None:
        """before() returns None by default."""
        mw = Middleware()
        ctx = MagicMock(spec=Context)
        result = mw.before("some.module", {"key": "val"}, ctx)
        assert result is None

    def test_after_returns_none_by_default(self) -> None:
        """after() returns None by default."""
        mw = Middleware()
        ctx = MagicMock(spec=Context)
        result = mw.after("some.module", {"key": "val"}, {"out": 1}, ctx)
        assert result is None

    def test_on_error_returns_none_by_default(self) -> None:
        """on_error() returns None by default."""
        mw = Middleware()
        ctx = MagicMock(spec=Context)
        result = mw.on_error("some.module", {"key": "val"}, RuntimeError("boom"), ctx)
        assert result is None

    def test_subclass_can_override_before_only(self) -> None:
        """Subclass overriding only before() leaves other methods as no-ops."""

        class MyMiddleware(Middleware):
            def before(self, module_id: str, inputs: dict[str, Any], context: Context) -> dict[str, Any] | None:
                return {"modified": True}

        mw = MyMiddleware()
        ctx = MagicMock(spec=Context)
        assert mw.before("mod", {}, ctx) == {"modified": True}
        assert mw.after("mod", {}, {}, ctx) is None
        assert mw.on_error("mod", {}, RuntimeError(), ctx) is None

    def test_subclass_can_override_all_methods(self) -> None:
        """Subclass can override all three methods with custom behavior."""

        class FullMiddleware(Middleware):
            def before(self, module_id: str, inputs: dict[str, Any], context: Context) -> dict[str, Any] | None:
                return {"before": True}

            def after(
                self,
                module_id: str,
                inputs: dict[str, Any],
                output: dict[str, Any],
                context: Context,
            ) -> dict[str, Any] | None:
                return {"after": True}

            def on_error(
                self,
                module_id: str,
                inputs: dict[str, Any],
                error: Exception,
                context: Context,
            ) -> dict[str, Any] | None:
                return {"error_handled": True}

        mw = FullMiddleware()
        ctx = MagicMock(spec=Context)
        assert mw.before("mod", {}, ctx) == {"before": True}
        assert mw.after("mod", {}, {}, ctx) == {"after": True}
        assert mw.on_error("mod", {}, RuntimeError(), ctx) == {"error_handled": True}


# === BeforeMiddleware Adapter ===


class TestBeforeMiddleware:
    """Tests for the BeforeMiddleware function adapter."""

    def test_is_middleware_subclass(self) -> None:
        """BeforeMiddleware wraps a callback as a Middleware subclass."""
        bm = BeforeMiddleware(lambda mid, inp, ctx: None)
        assert isinstance(bm, Middleware)

    def test_before_delegates_to_callback(self) -> None:
        """before() delegates to the wrapped callback."""
        callback = Mock(return_value={"modified": True})
        bm = BeforeMiddleware(callback)
        ctx = MagicMock(spec=Context)
        result = bm.before("mod.id", {"k": "v"}, ctx)
        assert result == {"modified": True}

    def test_after_returns_none(self) -> None:
        """after() returns None regardless of callback."""
        bm = BeforeMiddleware(lambda mid, inp, ctx: {"modified": True})
        ctx = MagicMock(spec=Context)
        assert bm.after("mod.id", {"k": "v"}, {"out": 1}, ctx) is None

    def test_on_error_returns_none(self) -> None:
        """on_error() returns None regardless of callback."""
        bm = BeforeMiddleware(lambda mid, inp, ctx: {"modified": True})
        ctx = MagicMock(spec=Context)
        assert bm.on_error("mod.id", {"k": "v"}, RuntimeError(), ctx) is None

    def test_callback_receives_correct_args(self) -> None:
        """Callback receives (module_id, inputs, context)."""
        spy = Mock(return_value=None)
        bm = BeforeMiddleware(spy)
        ctx = MagicMock(spec=Context)
        bm.before("mod.id", {"k": "v"}, ctx)
        spy.assert_called_once_with("mod.id", {"k": "v"}, ctx)


# === AfterMiddleware Adapter ===


class TestAfterMiddleware:
    """Tests for the AfterMiddleware function adapter."""

    def test_is_middleware_subclass(self) -> None:
        """AfterMiddleware wraps a callback as a Middleware subclass."""
        am = AfterMiddleware(lambda mid, inp, out, ctx: None)
        assert isinstance(am, Middleware)

    def test_after_delegates_to_callback(self) -> None:
        """after() delegates to the wrapped callback."""
        callback = Mock(return_value={"enriched": True})
        am = AfterMiddleware(callback)
        ctx = MagicMock(spec=Context)
        result = am.after("mod.id", {"k": "v"}, {"out": 1}, ctx)
        assert result == {"enriched": True}

    def test_before_returns_none(self) -> None:
        """before() returns None regardless of callback."""
        am = AfterMiddleware(lambda mid, inp, out, ctx: {"enriched": True})
        ctx = MagicMock(spec=Context)
        assert am.before("mod.id", {"k": "v"}, ctx) is None

    def test_on_error_returns_none(self) -> None:
        """on_error() returns None regardless of callback."""
        am = AfterMiddleware(lambda mid, inp, out, ctx: {"enriched": True})
        ctx = MagicMock(spec=Context)
        assert am.on_error("mod.id", {"k": "v"}, RuntimeError(), ctx) is None

    def test_callback_receives_correct_args(self) -> None:
        """Callback receives (module_id, inputs, output, context)."""
        spy = Mock(return_value=None)
        am = AfterMiddleware(spy)
        ctx = MagicMock(spec=Context)
        am.after("mod.id", {"k": "v"}, {"out": 1}, ctx)
        spy.assert_called_once_with("mod.id", {"k": "v"}, {"out": 1}, ctx)


# === Middleware Priority Ordering ===


class TestMiddlewarePriority:
    """Tests for middleware priority ordering in MiddlewareManager."""

    def test_default_priority_is_100(self) -> None:
        """Middleware instances default to priority 100 per PROTOCOL_SPEC."""
        mw = Middleware()
        assert mw.priority == 100

    def test_custom_priority(self) -> None:
        """Middleware accepts a custom priority via constructor."""
        mw = Middleware(priority=500)
        assert mw.priority == 500

    def test_higher_priority_executes_first(self) -> None:
        """Middlewares with higher priority appear earlier in the list."""
        manager = MiddlewareManager()
        low = Middleware(priority=100)
        high = Middleware(priority=900)
        mid = Middleware(priority=500)

        manager.add(low)
        manager.add(high)
        manager.add(mid)

        snapshot = manager.snapshot()
        assert snapshot == [high, mid, low]

    def test_equal_priority_preserves_registration_order(self) -> None:
        """Middlewares with the same priority are ordered by registration time."""
        manager = MiddlewareManager()
        first = Middleware(priority=100)
        second = Middleware(priority=100)
        third = Middleware(priority=100)

        manager.add(first)
        manager.add(second)
        manager.add(third)

        snapshot = manager.snapshot()
        assert snapshot == [first, second, third]

    def test_mixed_priorities_with_ties(self) -> None:
        """Mixed priorities sort correctly with registration-order tiebreaking."""
        manager = MiddlewareManager()
        a = Middleware(priority=500)
        b = Middleware(priority=100)
        c = Middleware(priority=500)
        d = Middleware(priority=1000)
        e = Middleware(priority=0)

        manager.add(a)
        manager.add(b)
        manager.add(c)
        manager.add(d)
        manager.add(e)

        snapshot = manager.snapshot()
        assert snapshot == [d, a, c, b, e]

    def test_default_priority_backward_compatible(self) -> None:
        """Middlewares without explicit priority still work (default 100)."""
        manager = MiddlewareManager()
        mw1 = Middleware()
        mw2 = Middleware()
        mw3 = Middleware()

        manager.add(mw1)
        manager.add(mw2)
        manager.add(mw3)

        snapshot = manager.snapshot()
        assert snapshot == [mw1, mw2, mw3]

    def test_subclass_without_super_init_defaults_to_100(self) -> None:
        """Subclasses that don't call super().__init__() still have class default priority 100."""

        class CustomMiddleware(Middleware):
            def __init__(self) -> None:
                self.custom_field = "hello"

        mw = CustomMiddleware()
        assert mw.priority == 100

    def test_remove_preserves_priority_order(self) -> None:
        """Removing a middleware preserves the priority-sorted order."""
        manager = MiddlewareManager()
        low = Middleware(priority=100)
        high = Middleware(priority=900)
        mid = Middleware(priority=500)

        manager.add(low)
        manager.add(high)
        manager.add(mid)
        manager.remove(mid)

        snapshot = manager.snapshot()
        assert snapshot == [high, low]

    def test_priority_below_zero_raises_value_error(self) -> None:
        """Priority below 0 raises ValueError."""
        import pytest

        with pytest.raises(ValueError, match="priority must be between 0 and 1000"):
            Middleware(priority=-1)

    def test_priority_above_1000_raises_value_error(self) -> None:
        """Priority above 1000 raises ValueError."""
        import pytest

        with pytest.raises(ValueError, match="priority must be between 0 and 1000"):
            Middleware(priority=1001)

    def test_before_middleware_accepts_priority(self) -> None:
        """BeforeMiddleware forwards priority to the base class."""
        bm = BeforeMiddleware(lambda mid, inp, ctx: None, priority=42)
        assert bm.priority == 42

    def test_after_middleware_accepts_priority(self) -> None:
        """AfterMiddleware forwards priority to the base class."""
        am = AfterMiddleware(lambda mid, inp, out, ctx: None, priority=99)
        assert am.priority == 99
