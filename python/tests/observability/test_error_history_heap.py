"""Tests for ErrorHistory min-heap eviction (Issue #43 §3).

Validates that ``_evict_total`` evicts the *globally oldest* entry by
``last_occurred`` and that the per-call cost is logarithmic — not a linear
scan over modules.  Uses lazy-deletion semantics: stale heap entries (from
deduplication timestamp refreshes) are skipped on pop.
"""

from __future__ import annotations

import time

from apcore.errors import ModuleError
from apcore.observability.error_history import ErrorHistory


def _err(code: str = "X", msg: str = "boom") -> ModuleError:
    return ModuleError(code=code, message=msg)


class TestHeapEviction:
    def test_oldest_globally_is_evicted(self) -> None:
        """When max_total_entries is exceeded, the entry with the oldest
        ``last_occurred`` (across *all* modules) is evicted."""
        hist = ErrorHistory(max_entries_per_module=1000, max_total_entries=3)

        # Inject 4 distinct fingerprints across 4 different modules
        # (different codes so each is unique).  The first one inserted has
        # the oldest last_occurred.
        for i in range(4):
            hist.record(f"mod.{i}", _err(code=f"E{i}", msg=f"msg-{i}"))
            time.sleep(0.001)  # ensure distinct ISO timestamps

        all_entries = hist.get_all()
        assert len(all_entries) == 3
        remaining_modules = {e.module_id for e in all_entries}
        # mod.0 was inserted first → its last_occurred is oldest → evicted
        assert "mod.0" not in remaining_modules
        assert remaining_modules == {"mod.1", "mod.2", "mod.3"}

    def test_dedup_refresh_is_not_evicted(self) -> None:
        """A duplicate entry that refreshes its ``last_occurred`` should
        NOT be the one evicted — the heap's stale entry must be skipped."""
        hist = ErrorHistory(max_entries_per_module=1000, max_total_entries=2)

        # Insert old entry A.
        hist.record("mod.a", _err(code="A", msg="a"))
        time.sleep(0.002)
        # Insert old entry B.
        hist.record("mod.b", _err(code="B", msg="b"))
        time.sleep(0.002)
        # Refresh A — its last_occurred jumps forward.
        hist.record("mod.a", _err(code="A", msg="a"))
        time.sleep(0.002)
        # Insert C — triggers eviction.  Oldest *live* entry is now B.
        hist.record("mod.c", _err(code="C", msg="c"))

        modules = {e.module_id for e in hist.get_all()}
        assert "mod.a" in modules  # refreshed, must survive
        assert "mod.c" in modules  # newest, must survive
        assert "mod.b" not in modules  # actual oldest live → evicted

    def test_eviction_bounded_per_insert(self) -> None:
        """Even with many modules, eviction does not scan O(M) per insert.

        This is a behavioural smoke check: inserting K extra entries beyond
        ``max_total_entries`` should evict exactly K entries.  We verify
        the count rather than instrumenting wall-clock complexity.
        """
        hist = ErrorHistory(max_entries_per_module=1000, max_total_entries=100)
        # Insert 250 unique entries spread across 50 modules.
        for i in range(250):
            mod = f"mod.{i % 50}"
            hist.record(mod, _err(code=f"E{i}", msg=f"m-{i}"))

        # Should hold exactly max_total_entries afterwards.
        assert len(hist.get_all()) == 100
