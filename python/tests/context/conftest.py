"""Shared fixtures for context-related tests."""

from __future__ import annotations

import pytest

from apcore.context import Context


@pytest.fixture
def make_context():
    """Factory that creates a minimal Context for unit tests."""

    def _make() -> Context:
        return Context(trace_id="test-trace-id")

    return _make
