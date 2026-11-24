"""Tests for duplicate middleware detection via MiddlewareManager.use() (apcore #64)."""

from __future__ import annotations

import logging
from typing import Any

import pytest

from apcore.context import Context
from apcore.middleware import Middleware, MiddlewareManager, RetryMiddleware


class _NoopMiddleware(Middleware):
    """Minimal middleware for duplicate-detection tests."""

    def before(self, module_id: str, inputs: dict[str, Any], context: Context) -> None:
        return None

    def after(self, module_id: str, inputs: dict[str, Any], output: dict[str, Any], context: Context) -> None:
        return None

    def on_error(self, module_id: str, inputs: dict[str, Any], error: Exception, context: Context) -> None:
        return None


def test_first_registration_no_warning(caplog: pytest.LogCaptureFixture) -> None:
    mgr = MiddlewareManager()
    with caplog.at_level(logging.WARNING):
        mgr.use(RetryMiddleware())
    assert not any(r.levelno == logging.WARNING for r in caplog.records)


def test_duplicate_emits_warning(caplog: pytest.LogCaptureFixture) -> None:
    mgr = MiddlewareManager()
    mgr.use(RetryMiddleware())
    with caplog.at_level(logging.WARNING):
        mgr.use(RetryMiddleware())
    warns = [r.message for r in caplog.records if r.levelno == logging.WARNING]
    assert any("RetryMiddleware" in m for m in warns)


def test_allow_duplicate_suppresses_warning(caplog: pytest.LogCaptureFixture) -> None:
    mgr = MiddlewareManager()
    mgr.use(RetryMiddleware())
    with caplog.at_level(logging.WARNING):
        mgr.use(RetryMiddleware(), allow_duplicate=True)
    assert not any(r.levelno == logging.WARNING for r in caplog.records)


def test_distinct_identity_keys_no_warning(caplog: pytest.LogCaptureFixture) -> None:
    mgr = MiddlewareManager()
    mgr.use(RetryMiddleware(), identity_key="ext.myapp.retry.http")
    with caplog.at_level(logging.WARNING):
        mgr.use(RetryMiddleware(), identity_key="ext.myapp.retry.db")
    assert not any(r.levelno == logging.WARNING for r in caplog.records)


def test_same_identity_key_warns(caplog: pytest.LogCaptureFixture) -> None:
    mgr = MiddlewareManager()
    mgr.use(RetryMiddleware(), identity_key="ext.myapp.retry")
    with caplog.at_level(logging.WARNING):
        mgr.use(RetryMiddleware(), identity_key="ext.myapp.retry")
    warns = [r.message for r in caplog.records if r.levelno == logging.WARNING]
    assert any("ext.myapp.retry" in m for m in warns)


def test_both_instances_remain_active() -> None:
    """Both duplicate instances must still fire before/after."""
    before_calls: list[str] = []

    class _Counter(Middleware):
        def __init__(self, label: str) -> None:
            self._label = label

        def before(self, module_id: str, inputs: dict[str, Any], context: Context) -> None:
            before_calls.append(self._label)
            return None

        def after(self, module_id: str, inputs: dict[str, Any], output: dict[str, Any], context: Context) -> None:
            return None

        def on_error(self, module_id: str, inputs: dict[str, Any], error: Exception, context: Context) -> None:
            return None

    mgr = MiddlewareManager()
    mgr.use(_Counter("first"), identity_key="ext.test.counter")
    mgr.use(_Counter("second"), identity_key="ext.test.counter", allow_duplicate=True)

    ctx = Context(trace_id="t")
    mgr.execute_before("test.module", {}, ctx)

    assert "first" in before_calls
    assert "second" in before_calls


def test_use_calls_add_ordering_preserved() -> None:
    """use() must respect the same priority-based insertion order as add()."""
    mgr = MiddlewareManager()
    mw1 = _NoopMiddleware(priority=50)
    mw2 = _NoopMiddleware(priority=100)
    mgr.use(mw1, identity_key="ext.test.low")
    mgr.use(mw2, identity_key="ext.test.high")
    snapshot = mgr.snapshot()
    assert snapshot[0] is mw2  # higher priority first
    assert snapshot[1] is mw1
