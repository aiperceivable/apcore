"""Regression tests for MiddlewareManager priority validation."""

import pytest
from apcore.middleware.manager import MiddlewareManager


class DummyMiddleware:
    def __init__(self, priority=500, name="dummy"):
        self.priority = priority
        self.name = name

    def before(self, *a, **kw):
        pass

    def after(self, *a, **kw):
        pass

    def on_error(self, *a, **kw):
        return None


def test_add_accepts_valid_priority():
    m = MiddlewareManager()
    m.add(DummyMiddleware(priority=0))
    m.add(DummyMiddleware(priority=500))
    m.add(DummyMiddleware(priority=1000))


def test_add_rejects_priority_above_1000():
    m = MiddlewareManager()
    with pytest.raises(ValueError, match="exceeds the maximum allowed value of 1000"):
        m.add(DummyMiddleware(priority=1001))
