"""Tests for ContextKey[T] public API promotion (apcore #63)."""

from __future__ import annotations

from collections.abc import Callable

import pytest

from apcore import ContextKey
from apcore.context import Context
from apcore.context_keys import (
    LOGGING_START,
    METRICS_STARTS,
    REDACTED_OUTPUT,
    RETRY_COUNT_BASE,
    TRACING_SAMPLED,
    TRACING_SPANS,
)


def test_builtin_identifiers() -> None:
    assert TRACING_SPANS.name == "_apcore.mw.tracing.spans"
    assert TRACING_SAMPLED.name == "_apcore.mw.tracing.sampled"
    assert METRICS_STARTS.name == "_apcore.mw.metrics.starts"
    assert LOGGING_START.name == "_apcore.mw.logging.start_time"
    assert REDACTED_OUTPUT.name == "_apcore.executor.redacted_output"
    assert RETRY_COUNT_BASE.name == "_apcore.mw.retry.count"


def test_key_anchored_api_roundtrip(make_context: Callable[[], Context]) -> None:
    KEY: ContextKey[int] = ContextKey("ext.test.retry.count")
    ctx = make_context()
    assert KEY.exists(ctx) is False
    KEY.set(ctx, 3)
    assert KEY.exists(ctx) is True
    assert KEY.get(ctx) == 3
    assert KEY.get(ctx, default=99) == 3
    KEY.delete(ctx)
    assert KEY.exists(ctx) is False
    assert KEY.get(ctx, default=99) == 99


def test_scoped_subkey(make_context: Callable[[], Context]) -> None:
    BASE: ContextKey[int] = ContextKey("ext.test.base")
    sub = BASE.scoped("module-a")
    assert sub.name == "ext.test.base.module-a"


def test_ext_key_works_like_builtin(make_context: Callable[[], Context]) -> None:
    """A key under ext.* must behave equivalently to built-in keys."""
    KEY: ContextKey[str] = ContextKey("ext.myapp.session.token")
    ctx = make_context()
    KEY.set(ctx, "abc123")
    assert KEY.get(ctx) == "abc123"
    assert KEY.exists(ctx) is True
    KEY.delete(ctx)
    assert KEY.exists(ctx) is False


def test_context_key_exported_from_top_level() -> None:
    """ContextKey must be importable directly from the apcore package."""
    from apcore import ContextKey as CK  # noqa: F401

    assert CK is ContextKey


def test_builtin_constants_exported_from_top_level() -> None:
    """All 6 built-in constants must be importable from the apcore package."""
    from apcore import (  # noqa: F401
        LOGGING_START as LS,
        METRICS_STARTS as MS,
        REDACTED_OUTPUT as RO,
        RETRY_COUNT_BASE as RC,
        TRACING_SAMPLED as TS,
        TRACING_SPANS as TSpans,
    )

    assert TSpans is TRACING_SPANS
    assert TS is TRACING_SAMPLED
    assert MS is METRICS_STARTS
    assert LS is LOGGING_START
    assert RO is REDACTED_OUTPUT
    assert RC is RETRY_COUNT_BASE


def test_context_key_frozen() -> None:
    """ContextKey is a frozen dataclass — mutation must raise."""
    key: ContextKey[int] = ContextKey("ext.test.immutable")
    with pytest.raises((AttributeError, TypeError)):
        key.name = "something_else"  # type: ignore[misc]
