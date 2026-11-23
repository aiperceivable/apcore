"""Unit tests for UsageCollector and UsageMiddleware."""

from __future__ import annotations

import threading
import time
from datetime import datetime, timedelta, timezone

import pytest

from apcore.context import Context
from apcore.observability.usage import (
    ModuleUsageDetail,
    ModuleUsageSummary,
    UsageCollector,
    UsageMiddleware,
    UsageRecord,
)


# ---------------------------------------------------------------------------
# UsageRecord dataclass
# ---------------------------------------------------------------------------


class TestUsageRecord:
    def test_usage_record_fields(self) -> None:
        ts = datetime.now(timezone.utc).isoformat()
        record = UsageRecord(timestamp=ts, caller_id="caller-1", latency_ms=12.5, success=True)
        assert record.timestamp == ts
        assert record.caller_id == "caller-1"
        assert record.latency_ms == 12.5
        assert record.success is True


# ---------------------------------------------------------------------------
# UsageCollector
# ---------------------------------------------------------------------------


class TestUsageCollector:
    def test_collector_default_retention(self) -> None:
        collector = UsageCollector()
        assert collector.retention_hours == 168

    def test_collector_record_stores_entry(self) -> None:
        collector = UsageCollector()
        collector.record("mod-a", "caller-1", 10.0, True)
        summary = collector.get_summary(period="24h")
        mod_summary = [s for s in summary if s.module_id == "mod-a"]
        assert len(mod_summary) == 1
        assert mod_summary[0].call_count == 1

    def test_collector_record_hourly_buckets(self) -> None:
        collector = UsageCollector()
        now = datetime.now(timezone.utc)
        ts1 = now.isoformat()
        ts2 = (now - timedelta(hours=2)).isoformat()
        collector.record("mod-a", "caller-1", 10.0, True, timestamp=ts1)
        collector.record("mod-a", "caller-1", 20.0, True, timestamp=ts2)
        detail = collector.get_module("mod-a", period="24h")
        # Should have entries in at least 2 different hourly buckets
        assert len(detail.hourly_distribution) >= 2

    def test_collector_get_summary_24h(self) -> None:
        collector = UsageCollector()
        now = datetime.now(timezone.utc)
        # Record entries within last 24h
        for i in range(5):
            ts = (now - timedelta(hours=i)).isoformat()
            success = i != 2  # one failure
            collector.record("mod-a", f"caller-{i % 3}", float(10 + i), success, timestamp=ts)

        summary = collector.get_summary(period="24h")
        assert isinstance(summary, list)
        mod_a = [s for s in summary if s.module_id == "mod-a"][0]
        assert isinstance(mod_a, ModuleUsageSummary)
        assert mod_a.call_count == 5
        assert mod_a.error_count == 1
        assert mod_a.avg_latency_ms == pytest.approx((10 + 11 + 12 + 13 + 14) / 5)
        assert mod_a.unique_callers == 3
        assert mod_a.trend in ("rising", "declining", "stable", "new", "inactive")

    def test_collector_get_summary_1h(self) -> None:
        collector = UsageCollector()
        now = datetime.now(timezone.utc)
        # One within last hour, one 2 hours ago
        collector.record("mod-a", "c1", 10.0, True, timestamp=now.isoformat())
        collector.record("mod-a", "c1", 20.0, True, timestamp=(now - timedelta(hours=2)).isoformat())
        summary = collector.get_summary(period="1h")
        mod_a = [s for s in summary if s.module_id == "mod-a"][0]
        assert mod_a.call_count == 1

    def test_collector_get_summary_7d(self) -> None:
        collector = UsageCollector()
        now = datetime.now(timezone.utc)
        for day in range(7):
            ts = (now - timedelta(days=day)).isoformat()
            collector.record("mod-a", "c1", 10.0, True, timestamp=ts)
        summary = collector.get_summary(period="7d")
        mod_a = [s for s in summary if s.module_id == "mod-a"][0]
        assert mod_a.call_count == 7

    def test_collector_get_module_detail(self) -> None:
        collector = UsageCollector()
        now = datetime.now(timezone.utc)
        collector.record("mod-a", "c1", 10.0, True, timestamp=now.isoformat())
        collector.record("mod-a", "c2", 20.0, False, timestamp=now.isoformat())
        detail = collector.get_module("mod-a", period="24h")
        assert isinstance(detail, ModuleUsageDetail)
        assert detail.module_id == "mod-a"
        assert detail.call_count == 2
        assert detail.error_count == 1
        assert len(detail.callers) == 2
        assert len(detail.hourly_distribution) >= 1

    def test_collector_get_module_per_caller_breakdown(self) -> None:
        collector = UsageCollector()
        now = datetime.now(timezone.utc)
        collector.record("mod-a", "c1", 10.0, True, timestamp=now.isoformat())
        collector.record("mod-a", "c1", 20.0, False, timestamp=now.isoformat())
        collector.record("mod-a", "c2", 30.0, True, timestamp=now.isoformat())
        detail = collector.get_module("mod-a", period="24h")
        caller_map = {c.caller_id: c for c in detail.callers}
        assert caller_map["c1"].call_count == 2
        assert caller_map["c1"].error_count == 1
        assert caller_map["c1"].avg_latency_ms == pytest.approx(15.0)
        assert caller_map["c2"].call_count == 1
        assert caller_map["c2"].error_count == 0
        assert caller_map["c2"].avg_latency_ms == pytest.approx(30.0)

    def test_collector_get_module_hourly_distribution(self) -> None:
        collector = UsageCollector()
        now = datetime.now(timezone.utc)
        collector.record("mod-a", "c1", 10.0, True, timestamp=now.isoformat())
        collector.record("mod-a", "c1", 20.0, False, timestamp=now.isoformat())
        detail = collector.get_module("mod-a", period="24h")
        assert len(detail.hourly_distribution) >= 1
        entry = detail.hourly_distribution[0]
        assert hasattr(entry, "hour")
        assert hasattr(entry, "call_count")
        assert hasattr(entry, "error_count")

    def test_collector_trend_rising(self) -> None:
        collector = UsageCollector()
        now = datetime.now(timezone.utc)
        # Previous period (24-48h ago): 2 calls
        for i in range(2):
            ts = (now - timedelta(hours=30 + i)).isoformat()
            collector.record("mod-a", "c1", 10.0, True, timestamp=ts)
        # Current period (0-24h): 10 calls (>20% more)
        for i in range(10):
            ts = (now - timedelta(hours=i)).isoformat()
            collector.record("mod-a", "c1", 10.0, True, timestamp=ts)
        summary = collector.get_summary(period="24h")
        mod_a = [s for s in summary if s.module_id == "mod-a"][0]
        assert mod_a.trend == "rising"

    def test_collector_trend_declining(self) -> None:
        collector = UsageCollector()
        now = datetime.now(timezone.utc)
        # Previous period (24-48h ago): 10 calls
        for i in range(10):
            ts = (now - timedelta(hours=25 + i)).isoformat()
            collector.record("mod-a", "c1", 10.0, True, timestamp=ts)
        # Current period: 2 calls (>20% fewer)
        for i in range(2):
            ts = (now - timedelta(hours=i)).isoformat()
            collector.record("mod-a", "c1", 10.0, True, timestamp=ts)
        summary = collector.get_summary(period="24h")
        mod_a = [s for s in summary if s.module_id == "mod-a"][0]
        assert mod_a.trend == "declining"

    def test_collector_trend_stable(self) -> None:
        collector = UsageCollector()
        now = datetime.now(timezone.utc)
        # Previous period: 10 calls
        for i in range(10):
            ts = (now - timedelta(hours=25 + i)).isoformat()
            collector.record("mod-a", "c1", 10.0, True, timestamp=ts)
        # Current period: 10 calls (same)
        for i in range(10):
            ts = (now - timedelta(hours=i)).isoformat()
            collector.record("mod-a", "c1", 10.0, True, timestamp=ts)
        summary = collector.get_summary(period="24h")
        mod_a = [s for s in summary if s.module_id == "mod-a"][0]
        assert mod_a.trend == "stable"

    def test_collector_trend_new(self) -> None:
        collector = UsageCollector()
        now = datetime.now(timezone.utc)
        # Only current period data, no previous
        collector.record("mod-a", "c1", 10.0, True, timestamp=now.isoformat())
        summary = collector.get_summary(period="24h")
        mod_a = [s for s in summary if s.module_id == "mod-a"][0]
        assert mod_a.trend == "new"

    def test_collector_trend_inactive(self) -> None:
        collector = UsageCollector()
        now = datetime.now(timezone.utc)
        # Only previous period data (25-48h ago), no current period (0-24h)
        ts = (now - timedelta(hours=30)).isoformat()
        collector.record("mod-a", "c1", 10.0, True, timestamp=ts)
        summary = collector.get_summary(period="24h")
        mod_a = [s for s in summary if s.module_id == "mod-a"][0]
        assert mod_a.trend == "inactive"

    def test_collector_auto_cleanup_expired_buckets(self) -> None:
        collector = UsageCollector(retention_hours=1)
        now = datetime.now(timezone.utc)
        # Record an old entry beyond retention
        old_ts = (now - timedelta(hours=3)).isoformat()
        collector.record("mod-a", "c1", 10.0, True, timestamp=old_ts)
        # Record a recent entry (triggers cleanup)
        collector.record("mod-a", "c1", 10.0, True, timestamp=now.isoformat())
        # Old bucket should have been cleaned up
        summary = collector.get_summary(period="24h")
        mod_a = [s for s in summary if s.module_id == "mod-a"][0]
        assert mod_a.call_count == 1  # only the recent one

    def test_collector_thread_safety(self) -> None:
        collector = UsageCollector()
        errors: list[Exception] = []

        def worker(thread_id: int) -> None:
            try:
                for i in range(50):
                    collector.record("mod-a", f"caller-{thread_id}", float(i), True)
            except Exception as e:
                errors.append(e)

        threads = [threading.Thread(target=worker, args=(t,)) for t in range(10)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

        assert len(errors) == 0
        summary = collector.get_summary(period="24h")
        mod_a = [s for s in summary if s.module_id == "mod-a"][0]
        assert mod_a.call_count == 500

    def test_collector_unique_callers_count(self) -> None:
        collector = UsageCollector()
        now = datetime.now(timezone.utc)
        for cid in ["alice", "bob", "charlie"]:
            collector.record("mod-a", cid, 10.0, True, timestamp=now.isoformat())
        summary = collector.get_summary(period="24h")
        mod_a = [s for s in summary if s.module_id == "mod-a"][0]
        assert mod_a.unique_callers == 3

    def test_collector_avg_latency_calculation(self) -> None:
        collector = UsageCollector()
        now = datetime.now(timezone.utc)
        latencies = [10.0, 20.0, 30.0, 40.0]
        for lat in latencies:
            collector.record("mod-a", "c1", lat, True, timestamp=now.isoformat())
        summary = collector.get_summary(period="24h")
        mod_a = [s for s in summary if s.module_id == "mod-a"][0]
        assert mod_a.avg_latency_ms == pytest.approx(25.0)


# ---------------------------------------------------------------------------
# UsageMiddleware
# ---------------------------------------------------------------------------


class TestUsageMiddleware:
    def _make_context(self, caller_id: str = "test-caller") -> Context:
        ctx = Context(trace_id="trace-1", caller_id=caller_id, data={})
        return ctx

    def test_middleware_records_on_after(self) -> None:
        collector = UsageCollector()
        mw = UsageMiddleware(collector)
        ctx = self._make_context()
        mw.before("mod-a", {}, ctx)
        time.sleep(0.01)
        mw.after("mod-a", {}, {"result": "ok"}, ctx)
        summary = collector.get_summary(period="1h")
        mod_a = [s for s in summary if s.module_id == "mod-a"][0]
        assert mod_a.call_count == 1
        assert mod_a.error_count == 0

    def test_middleware_records_on_error(self) -> None:
        collector = UsageCollector()
        mw = UsageMiddleware(collector)
        ctx = self._make_context()
        mw.before("mod-a", {}, ctx)
        mw.on_error("mod-a", {}, RuntimeError("boom"), ctx)
        summary = collector.get_summary(period="1h")
        mod_a = [s for s in summary if s.module_id == "mod-a"][0]
        assert mod_a.call_count == 1
        assert mod_a.error_count == 1

    def test_middleware_uses_caller_id_from_context(self) -> None:
        collector = UsageCollector()
        mw = UsageMiddleware(collector)
        ctx = self._make_context(caller_id="my-service")
        mw.before("mod-a", {}, ctx)
        mw.after("mod-a", {}, {}, ctx)
        detail = collector.get_module("mod-a", period="1h")
        caller_ids = [c.caller_id for c in detail.callers]
        assert "my-service" in caller_ids

    def test_middleware_calculates_elapsed_time(self) -> None:
        collector = UsageCollector()
        mw = UsageMiddleware(collector)
        ctx = self._make_context()
        mw.before("mod-a", {}, ctx)
        time.sleep(0.05)
        mw.after("mod-a", {}, {}, ctx)
        detail = collector.get_module("mod-a", period="1h")
        # Should be at least 40ms (allowing some tolerance for sleep imprecision)
        assert detail.avg_latency_ms >= 40.0
