"""Tests for the MiddlewareManager class."""

from __future__ import annotations

import logging
import threading
from typing import Any
from unittest.mock import MagicMock

from apcore.context import Context
from apcore.middleware import Middleware
from apcore.middleware.manager import MiddlewareChainError, MiddlewareManager


# === Helper Middleware Subclasses ===


class TrackingMiddleware(Middleware):
    """Middleware that records calls for verification."""

    def __init__(
        self,
        name: str,
        before_return: dict[str, Any] | None = None,
        after_return: dict[str, Any] | None = None,
        on_error_return: dict[str, Any] | None = None,
    ) -> None:
        self.name = name
        self.before_calls: list[tuple[str, dict[str, Any]]] = []
        self.after_calls: list[tuple[str, dict[str, Any], dict[str, Any]]] = []
        self.on_error_calls: list[tuple[str, dict[str, Any], Exception]] = []
        self._before_return = before_return
        self._after_return = after_return
        self._on_error_return = on_error_return

    def before(self, module_id: str, inputs: dict[str, Any], context: Context) -> dict[str, Any] | None:
        self.before_calls.append((module_id, inputs))
        return self._before_return

    def after(
        self,
        module_id: str,
        inputs: dict[str, Any],
        output: dict[str, Any],
        context: Context,
    ) -> dict[str, Any] | None:
        self.after_calls.append((module_id, inputs, output))
        return self._after_return

    def on_error(
        self, module_id: str, inputs: dict[str, Any], error: Exception, context: Context
    ) -> dict[str, Any] | None:
        self.on_error_calls.append((module_id, inputs, error))
        return self._on_error_return


class FailingMiddleware(Middleware):
    """Middleware that raises in before()."""

    def before(self, module_id: str, inputs: dict[str, Any], context: Context) -> dict[str, Any] | None:
        raise RuntimeError("before failed")


class FailingAfterMiddleware(Middleware):
    """Middleware that raises in after()."""

    def after(
        self,
        module_id: str,
        inputs: dict[str, Any],
        output: dict[str, Any],
        context: Context,
    ) -> dict[str, Any] | None:
        raise RuntimeError("after failed")


class FailingOnErrorMiddleware(Middleware):
    """Middleware that raises in on_error()."""

    def on_error(
        self, module_id: str, inputs: dict[str, Any], error: Exception, context: Context
    ) -> dict[str, Any] | None:
        raise RuntimeError("on_error failed")


# === add / remove Tests ===


class TestAddRemove:
    """Tests for add() and remove() methods."""

    def test_add_appends_middleware(self) -> None:
        """Adding a middleware should append it to the internal list."""
        mgr = MiddlewareManager()
        mw = Middleware()
        mgr.add(mw)
        assert len(mgr._middlewares) == 1
        assert mgr._middlewares[0] is mw

    def test_add_multiple_preserves_order(self) -> None:
        """Adding multiple middlewares should preserve insertion order."""
        mgr = MiddlewareManager()
        mw1 = Middleware()
        mw2 = Middleware()
        mw3 = Middleware()
        mgr.add(mw1)
        mgr.add(mw2)
        mgr.add(mw3)
        assert mgr._middlewares == [mw1, mw2, mw3]

    def test_remove_by_identity_returns_true(self) -> None:
        """Removing a middleware that exists (by 'is' identity) should return True."""
        mgr = MiddlewareManager()
        mw = Middleware()
        mgr.add(mw)
        assert mgr.remove(mw) is True
        assert len(mgr._middlewares) == 0

    def test_remove_returns_false_when_not_found(self) -> None:
        """Removing a middleware that is not in the list should return False."""
        mgr = MiddlewareManager()
        mw1 = Middleware()
        mw2 = Middleware()
        mgr.add(mw1)
        assert mgr.remove(mw2) is False
        assert len(mgr._middlewares) == 1


# === execute_before Tests ===


class TestExecuteBefore:
    """Tests for execute_before() -- onion model."""

    def test_calls_in_order(self) -> None:
        """Middlewares' before() should be called in registration order."""
        mgr = MiddlewareManager()
        order: list[str] = []

        class OrderTracker(Middleware):
            def __init__(self, name: str) -> None:
                self._name = name

            def before(self, module_id: str, inputs: dict[str, Any], context: Context) -> dict[str, Any] | None:
                order.append(self._name)
                return None

        mgr.add(OrderTracker("A"))
        mgr.add(OrderTracker("B"))
        mgr.add(OrderTracker("C"))

        ctx = MagicMock(spec=Context)
        mgr.execute_before("mod", {}, ctx)
        assert order == ["A", "B", "C"]

    def test_returned_dict_replaces_inputs(self) -> None:
        """When before() returns a dict, it replaces inputs for the next middleware."""
        mgr = MiddlewareManager()
        mw1 = TrackingMiddleware("mw1", before_return={"from_mw1": True})
        mw2 = TrackingMiddleware("mw2")
        mgr.add(mw1)
        mgr.add(mw2)

        ctx = MagicMock(spec=Context)
        final_inputs, _ = mgr.execute_before("mod", {"original": True}, ctx)
        assert final_inputs == {"from_mw1": True}
        # mw2 should have received the replaced inputs
        assert mw2.before_calls[0][1] == {"from_mw1": True}

    def test_none_keeps_inputs(self) -> None:
        """When before() returns None, inputs are unchanged."""
        mgr = MiddlewareManager()
        mw1 = TrackingMiddleware("mw1")  # returns None
        mw2 = TrackingMiddleware("mw2")
        mgr.add(mw1)
        mgr.add(mw2)

        ctx = MagicMock(spec=Context)
        final_inputs, _ = mgr.execute_before("mod", {"original": True}, ctx)
        assert final_inputs == {"original": True}
        assert mw2.before_calls[0][1] == {"original": True}

    def test_exception_raises_chain_error(self) -> None:
        """When before() raises, MiddlewareChainError is raised."""
        mgr = MiddlewareManager()
        mw1 = TrackingMiddleware("mw1")
        mw2 = FailingMiddleware()
        mgr.add(mw1)
        mgr.add(mw2)

        ctx = MagicMock(spec=Context)
        try:
            mgr.execute_before("mod", {}, ctx)
            assert False, "Should have raised"
        except MiddlewareChainError as e:
            assert isinstance(e.original, RuntimeError)
            assert str(e.original) == "before failed"

    def test_tracks_executed_middlewares(self) -> None:
        """executed_middlewares includes middlewares called up to and including failure."""
        mgr = MiddlewareManager()
        mw1 = TrackingMiddleware("mw1")
        mw2 = FailingMiddleware()
        mw3 = TrackingMiddleware("mw3")
        mgr.add(mw1)
        mgr.add(mw2)
        mgr.add(mw3)

        ctx = MagicMock(spec=Context)
        try:
            mgr.execute_before("mod", {}, ctx)
        except MiddlewareChainError as e:
            assert len(e.executed_middlewares) == 2
            assert e.executed_middlewares[0] is mw1
            assert e.executed_middlewares[1] is mw2

    def test_empty_list_returns_original_inputs(self) -> None:
        """With no middlewares, returns original inputs unchanged."""
        mgr = MiddlewareManager()
        ctx = MagicMock(spec=Context)
        final_inputs, executed = mgr.execute_before("mod", {"a": 1}, ctx)
        assert final_inputs == {"a": 1}
        assert executed == []


# === execute_after Tests ===


class TestExecuteAfter:
    """Tests for execute_after() -- reverse order."""

    def test_calls_in_reverse_order(self) -> None:
        """Middlewares' after() should be called in reverse registration order."""
        mgr = MiddlewareManager()
        order: list[str] = []

        class OrderTracker(Middleware):
            def __init__(self, name: str) -> None:
                self._name = name

            def after(
                self,
                module_id: str,
                inputs: dict[str, Any],
                output: dict[str, Any],
                context: Context,
            ) -> dict[str, Any] | None:
                order.append(self._name)
                return None

        mgr.add(OrderTracker("A"))
        mgr.add(OrderTracker("B"))
        mgr.add(OrderTracker("C"))

        ctx = MagicMock(spec=Context)
        mgr.execute_after("mod", {}, {"result": 1}, ctx)
        assert order == ["C", "B", "A"]

    def test_returned_dict_replaces_output(self) -> None:
        """When after() returns a dict, it replaces output for the next middleware."""
        mgr = MiddlewareManager()
        mw1 = TrackingMiddleware("mw1")
        mw2 = TrackingMiddleware("mw2", after_return={"from_mw2": True})
        mgr.add(mw1)
        mgr.add(mw2)

        ctx = MagicMock(spec=Context)
        # Reverse order: mw2 runs first, then mw1
        final_output = mgr.execute_after("mod", {}, {"original": True}, ctx)
        assert final_output == {"from_mw2": True}
        # mw1 (runs second in reverse) should see the modified output
        assert mw1.after_calls[0][2] == {"from_mw2": True}

    def test_none_keeps_output(self) -> None:
        """When after() returns None, output is unchanged."""
        mgr = MiddlewareManager()
        mw1 = TrackingMiddleware("mw1")
        mgr.add(mw1)

        ctx = MagicMock(spec=Context)
        final_output = mgr.execute_after("mod", {}, {"original": True}, ctx)
        assert final_output == {"original": True}

    def test_exception_stops_and_raises(self) -> None:
        """When after() raises, execution stops and exception propagates."""
        mgr = MiddlewareManager()
        mw1 = TrackingMiddleware("mw1")
        mw2 = FailingAfterMiddleware()
        mgr.add(mw1)
        mgr.add(mw2)

        ctx = MagicMock(spec=Context)
        try:
            mgr.execute_after("mod", {}, {}, ctx)
            assert False, "Should have raised"
        except RuntimeError as e:
            assert str(e) == "after failed"

    def test_empty_list_returns_original_output(self) -> None:
        """With no middlewares, returns original output unchanged."""
        mgr = MiddlewareManager()
        ctx = MagicMock(spec=Context)
        result = mgr.execute_after("mod", {}, {"a": 1}, ctx)
        assert result == {"a": 1}


# === execute_on_error Tests ===


class TestExecuteOnError:
    """Tests for execute_on_error() -- reverse from failure."""

    def test_iterates_reverse(self) -> None:
        """on_error is called on executed_middlewares in reverse order."""
        order: list[str] = []

        class OrderTracker(Middleware):
            def __init__(self, name: str) -> None:
                self._name = name

            def on_error(
                self,
                module_id: str,
                inputs: dict[str, Any],
                error: Exception,
                context: Context,
            ) -> dict[str, Any] | None:
                order.append(self._name)
                return None

        mgr = MiddlewareManager()
        mw_a = OrderTracker("A")
        mw_b = OrderTracker("B")
        mw_c = OrderTracker("C")
        executed = [mw_a, mw_b, mw_c]

        ctx = MagicMock(spec=Context)
        mgr.execute_on_error("mod", {}, RuntimeError("boom"), ctx, executed)
        assert order == ["C", "B", "A"]

    def test_first_dict_is_recovery(self) -> None:
        """When on_error returns a dict, it's returned immediately."""
        mgr = MiddlewareManager()
        mw1 = TrackingMiddleware("mw1")
        mw2 = TrackingMiddleware("mw2", on_error_return={"recovered": True})
        mw3 = TrackingMiddleware("mw3")
        executed = [mw1, mw2, mw3]

        ctx = MagicMock(spec=Context)
        # Reverse order: mw3, mw2, mw1. mw2 returns recovery, so mw1 is skipped.
        result = mgr.execute_on_error("mod", {}, RuntimeError(), ctx, executed)
        assert result == {"recovered": True}
        # mw1 should not have been called (mw2 returned recovery first in reverse)
        assert len(mw1.on_error_calls) == 0

    def test_none_continues(self) -> None:
        """When on_error returns None, execution continues."""
        mgr = MiddlewareManager()
        mw1 = TrackingMiddleware("mw1")
        mw2 = TrackingMiddleware("mw2")
        executed = [mw1, mw2]

        ctx = MagicMock(spec=Context)
        result = mgr.execute_on_error("mod", {}, RuntimeError(), ctx, executed)
        assert result is None
        assert len(mw1.on_error_calls) == 1
        assert len(mw2.on_error_calls) == 1

    def test_exception_logged_continues(self, caplog: Any) -> None:
        """Exceptions in on_error handlers are logged and execution continues."""
        mgr = MiddlewareManager()
        mw1 = TrackingMiddleware("mw1")
        mw2 = FailingOnErrorMiddleware()
        executed = [mw1, mw2]

        ctx = MagicMock(spec=Context)
        with caplog.at_level(logging.ERROR):
            result = mgr.execute_on_error("mod", {}, RuntimeError("original"), ctx, executed)
        assert result is None
        # mw1 should still have been called (failure in mw2's on_error doesn't stop chain)
        assert len(mw1.on_error_calls) == 1
        assert "on_error failed" in caplog.text

    def test_returns_none_when_no_recovery(self) -> None:
        """When no handler returns a dict, returns None."""
        mgr = MiddlewareManager()
        mw1 = TrackingMiddleware("mw1")
        executed = [mw1]

        ctx = MagicMock(spec=Context)
        result = mgr.execute_on_error("mod", {}, RuntimeError(), ctx, executed)
        assert result is None

    def test_empty_list_returns_none(self) -> None:
        """With no executed middlewares, returns None."""
        mgr = MiddlewareManager()
        ctx = MagicMock(spec=Context)
        result = mgr.execute_on_error("mod", {}, RuntimeError(), ctx, [])
        assert result is None


# === Thread Safety Tests ===


class TestMiddlewareManagerThreadSafety:
    """Tests for thread-safe add/remove/snapshot operations."""

    def test_concurrent_add_no_lost_middlewares(self) -> None:
        """Concurrent add() calls should not lose any middlewares."""
        mgr = MiddlewareManager()
        num_threads = 10
        adds_per_thread = 50

        def adder() -> None:
            for _ in range(adds_per_thread):
                mgr.add(Middleware())

        threads = [threading.Thread(target=adder) for _ in range(num_threads)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

        assert len(mgr.snapshot()) == num_threads * adds_per_thread

    def test_snapshot_returns_consistent_copy(self) -> None:
        """snapshot() returns a list that is not affected by later mutations."""
        mgr = MiddlewareManager()
        mw1 = Middleware()
        mw2 = Middleware()
        mgr.add(mw1)
        mgr.add(mw2)

        snap = mgr.snapshot()
        assert len(snap) == 2

        mgr.remove(mw1)
        # snapshot is a copy, still has 2 items
        assert len(snap) == 2
        # but the manager now has 1
        assert len(mgr.snapshot()) == 1

    def test_concurrent_add_and_snapshot(self) -> None:
        """Concurrent add() and snapshot() should not raise."""
        mgr = MiddlewareManager()
        errors: list[Exception] = []

        def adder() -> None:
            try:
                for _ in range(100):
                    mgr.add(Middleware())
            except Exception as e:
                errors.append(e)

        def reader() -> None:
            try:
                for _ in range(100):
                    mgr.snapshot()
            except Exception as e:
                errors.append(e)

        threads = [threading.Thread(target=adder) for _ in range(5)]
        threads += [threading.Thread(target=reader) for _ in range(5)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

        assert errors == []
