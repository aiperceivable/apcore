"""Tests for ErrorHistory component."""

from __future__ import annotations

import threading


from apcore.errors import ModuleError
from apcore.observability.error_history import ErrorEntry, ErrorHistory


class TestErrorEntry:
    def test_error_entry_fields(self) -> None:
        entry = ErrorEntry(
            module_id="payment.charge",
            code="TIMEOUT",
            message="Connection timed out",
            ai_guidance="Increase timeout or add retry",
            timestamp="2026-03-08T10:00:00Z",
            count=1,
            first_occurred="2026-03-08T10:00:00Z",
            last_occurred="2026-03-08T10:00:00Z",
        )
        assert entry.module_id == "payment.charge"
        assert entry.code == "TIMEOUT"
        assert entry.message == "Connection timed out"
        assert entry.ai_guidance == "Increase timeout or add retry"
        assert entry.count == 1

    def test_error_entry_nullable_ai_guidance(self) -> None:
        entry = ErrorEntry(
            module_id="m",
            code="ERR",
            message="msg",
            ai_guidance=None,
            timestamp="2026-03-08T10:00:00Z",
            count=1,
            first_occurred="2026-03-08T10:00:00Z",
            last_occurred="2026-03-08T10:00:00Z",
        )
        assert entry.ai_guidance is None


class TestErrorHistoryRecord:
    def test_record_single_error(self) -> None:
        history = ErrorHistory()
        error = ModuleError(code="TIMEOUT", message="timed out", ai_guidance="retry")
        history.record("mod.a", error)
        entries = history.get("mod.a")
        assert len(entries) == 1
        assert entries[0].code == "TIMEOUT"
        assert entries[0].message == "timed out"
        assert entries[0].ai_guidance == "retry"
        assert entries[0].count == 1

    def test_record_dedup_merges_count(self) -> None:
        history = ErrorHistory()
        error = ModuleError(code="TIMEOUT", message="timed out")
        history.record("mod.a", error)
        history.record("mod.a", error)
        entries = history.get("mod.a")
        assert len(entries) == 1
        assert entries[0].count == 2

    def test_record_dedup_updates_last_occurred(self) -> None:
        history = ErrorHistory()
        error = ModuleError(code="TIMEOUT", message="timed out")
        history.record("mod.a", error)
        first = history.get("mod.a")[0]
        history.record("mod.a", error)
        updated = history.get("mod.a")[0]
        assert updated.first_occurred == first.first_occurred
        assert updated.last_occurred >= first.last_occurred

    def test_record_different_codes_stored_separately(self) -> None:
        history = ErrorHistory()
        history.record("mod.a", ModuleError(code="TIMEOUT", message="timed out"))
        history.record("mod.a", ModuleError(code="AUTH", message="unauthorized"))
        entries = history.get("mod.a")
        assert len(entries) == 2

    def test_record_preserves_ai_guidance(self) -> None:
        history = ErrorHistory()
        error = ModuleError(
            code="DB_ERROR",
            message="connection refused",
            ai_guidance="Check database connection string and ensure DB is running",
        )
        history.record("db.query", error)
        entries = history.get("db.query")
        assert entries[0].ai_guidance == "Check database connection string and ensure DB is running"


class TestErrorHistoryGet:
    def test_get_returns_newest_first(self) -> None:
        history = ErrorHistory()
        history.record("mod.a", ModuleError(code="E1", message="first"))
        history.record("mod.a", ModuleError(code="E2", message="second"))
        history.record("mod.a", ModuleError(code="E3", message="third"))
        entries = history.get("mod.a")
        assert entries[0].code == "E3"
        assert entries[-1].code == "E1"

    def test_get_with_limit(self) -> None:
        history = ErrorHistory()
        for i in range(5):
            history.record("mod.a", ModuleError(code=f"E{i}", message=f"error {i}"))
        entries = history.get("mod.a", limit=3)
        assert len(entries) == 3

    def test_get_unknown_module_returns_empty(self) -> None:
        history = ErrorHistory()
        assert history.get("nonexistent") == []


class TestErrorHistoryGetAll:
    def test_get_all_returns_newest_first(self) -> None:
        history = ErrorHistory()
        history.record("mod.a", ModuleError(code="E1", message="first"))
        history.record("mod.b", ModuleError(code="E2", message="second"))
        history.record("mod.a", ModuleError(code="E3", message="third"))
        entries = history.get_all()
        assert entries[0].code == "E3"

    def test_get_all_with_limit(self) -> None:
        history = ErrorHistory()
        for i in range(10):
            history.record(f"mod.{i}", ModuleError(code=f"E{i}", message=f"error {i}"))
        entries = history.get_all(limit=5)
        assert len(entries) == 5


class TestErrorHistoryEviction:
    def test_max_entries_per_module_eviction(self) -> None:
        history = ErrorHistory(max_entries_per_module=3)
        for i in range(5):
            history.record("mod.a", ModuleError(code=f"E{i}", message=f"error {i}"))
        entries = history.get("mod.a")
        assert len(entries) == 3
        # Oldest should be evicted
        codes = {e.code for e in entries}
        assert "E0" not in codes
        assert "E1" not in codes

    def test_max_total_entries_eviction(self) -> None:
        history = ErrorHistory(max_total_entries=5)
        for i in range(8):
            history.record(f"mod.{i}", ModuleError(code=f"E{i}", message=f"error {i}"))
        entries = history.get_all(limit=100)
        assert len(entries) <= 5


class TestErrorHistoryDefaults:
    def test_default_limits(self) -> None:
        history = ErrorHistory()
        assert history._max_entries_per_module == 50
        assert history._max_total_entries == 1000


class TestErrorHistoryThreadSafety:
    def test_thread_safety_concurrent_records(self) -> None:
        history = ErrorHistory()
        errors: list[Exception] = []

        def record_errors(thread_id: int) -> None:
            try:
                for i in range(100):
                    history.record(
                        f"mod.{thread_id}",
                        ModuleError(code=f"E{i}", message=f"error {i}"),
                    )
            except Exception as e:
                errors.append(e)

        threads = [threading.Thread(target=record_errors, args=(t,)) for t in range(10)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

        assert len(errors) == 0
        # Should have entries from all 10 threads
        all_entries = history.get_all(limit=2000)
        assert len(all_entries) > 0
