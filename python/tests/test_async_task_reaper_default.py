"""Cross-language alignment: ``start_reaper`` default ``sweep_interval_ms``.

TS (`src/async-task.ts`) and Rust SDKs default to **300_000 ms (5 minutes)**.
Python previously defaulted to 3_600_000 ms (1 hour). This test enforces the
aligned default.
"""

from __future__ import annotations

from typing import Any

import pytest

from apcore.async_task import AsyncTaskManager


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
async def test_start_reaper_default_sweep_interval_is_300000_ms() -> None:
    """``start_reaper()`` with no kwargs uses 300_000 ms (5 min) — TS/Rust parity."""
    mgr = AsyncTaskManager(executor=_StubExecutor())
    handle = mgr.start_reaper()
    try:
        # Default sweep_interval_ms = 300_000 → 300 s internally.
        assert mgr._reaper_interval == pytest.approx(300.0)
    finally:
        await handle.stop()
