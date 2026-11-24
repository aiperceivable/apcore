"""Issue #34 D-11 — start_reaper cross-language signature alignment.

The TS and Rust SDKs expose ``start_reaper(ttl_seconds=, sweep_interval_ms=)``
and return a ``ReaperHandle``.  Python previously used
``interval_seconds=`` / ``max_age_seconds=`` (both seconds) and returned None.

This test guards:
1. The new keyword-only signature works.
2. The legacy aliases still accept input but emit ``DeprecationWarning``.
3. ``start_reaper`` returns a ``ReaperHandle`` whose ``stop()`` cancels the loop.
"""

from __future__ import annotations

import warnings
from typing import Any

import pytest

from apcore.async_task import AsyncTaskManager, ReaperHandle


class _StubExecutor:
    async def call_async(
        self,
        module_id: str,
        inputs: dict[str, Any] | None = None,
        context: Any = None,
        version_hint: str | None = None,
    ) -> dict[str, Any]:
        return {}


@pytest.mark.asyncio
async def test_start_reaper_new_signature_returns_handle() -> None:
    """``start_reaper(ttl_seconds=, sweep_interval_ms=)`` returns a ReaperHandle."""
    mgr = AsyncTaskManager(executor=_StubExecutor())
    handle = mgr.start_reaper(ttl_seconds=10.0, sweep_interval_ms=60_000)
    try:
        assert isinstance(handle, ReaperHandle)
        # Internal state should reflect the converted units.
        assert mgr._reaper_max_age == pytest.approx(10.0)
        assert mgr._reaper_interval == pytest.approx(60.0)  # ms → s
    finally:
        await handle.stop()


@pytest.mark.asyncio
async def test_start_reaper_legacy_aliases_emit_deprecation_warning() -> None:
    """Old ``interval_seconds`` / ``max_age_seconds`` still work but warn."""
    mgr = AsyncTaskManager(executor=_StubExecutor())
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        handle = mgr.start_reaper(interval_seconds=120.0, max_age_seconds=30.0)
        try:
            assert isinstance(handle, ReaperHandle)
            assert mgr._reaper_interval == pytest.approx(120.0)
            assert mgr._reaper_max_age == pytest.approx(30.0)
        finally:
            await handle.stop()

    deprecation = [w for w in caught if issubclass(w.category, DeprecationWarning)]
    msgs = " | ".join(str(w.message) for w in deprecation)
    assert deprecation, f"Expected DeprecationWarning, got: {[str(w.message) for w in caught]}"
    assert "interval_seconds" in msgs
    assert "max_age_seconds" in msgs


@pytest.mark.asyncio
async def test_reaper_handle_stop_cancels_loop() -> None:
    """``handle.stop()`` cancels the underlying loop and is idempotent."""
    mgr = AsyncTaskManager(executor=_StubExecutor())
    handle = mgr.start_reaper(ttl_seconds=1.0, sweep_interval_ms=1_000)
    assert handle.is_running()
    await handle.stop()
    assert not handle.is_running()
    # Idempotent — no exception on second stop.
    await handle.stop()


@pytest.mark.asyncio
async def test_start_reaper_rejects_mixed_old_and_new_kwargs() -> None:
    """Passing both legacy and new kwargs is a TypeError."""
    mgr = AsyncTaskManager(executor=_StubExecutor())
    with pytest.raises(TypeError):
        mgr.start_reaper(ttl_seconds=10.0, max_age_seconds=10.0)
    with pytest.raises(TypeError):
        mgr.start_reaper(sweep_interval_ms=10_000, interval_seconds=10.0)
