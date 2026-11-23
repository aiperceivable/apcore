"""Tests for the generic StorageBackend protocol (Issue #43 §1).

Validates the namespaced ``save / get / list / delete`` surface mirrored on
the ``TaskStore`` pattern, the default ``InMemoryStorageBackend`` implementation,
and that observability collectors accept an injected backend.
"""

from __future__ import annotations

from apcore.errors import ModuleError
from apcore.observability.error_history import ErrorHistory
from apcore.observability.storage import (
    InMemoryStorageBackend,
    StorageBackend,
)


class TestInMemoryStorageBackend:
    def test_protocol_satisfied(self) -> None:
        assert isinstance(InMemoryStorageBackend(), StorageBackend)

    def test_save_then_get(self) -> None:
        backend = InMemoryStorageBackend()
        backend.save("ns1", "k1", {"a": 1})
        assert backend.get("ns1", "k1") == {"a": 1}

    def test_get_missing_returns_none(self) -> None:
        assert InMemoryStorageBackend().get("ns", "missing") is None

    def test_namespaces_are_isolated(self) -> None:
        backend = InMemoryStorageBackend()
        backend.save("ns1", "shared", {"v": 1})
        backend.save("ns2", "shared", {"v": 2})
        assert backend.get("ns1", "shared") == {"v": 1}
        assert backend.get("ns2", "shared") == {"v": 2}

    def test_list_returns_all_entries(self) -> None:
        backend = InMemoryStorageBackend()
        backend.save("ns", "a", {"v": 1})
        backend.save("ns", "b", {"v": 2})
        items = dict(backend.list("ns"))
        assert items == {"a": {"v": 1}, "b": {"v": 2}}

    def test_list_with_prefix(self) -> None:
        backend = InMemoryStorageBackend()
        backend.save("ns", "task:1", {"v": 1})
        backend.save("ns", "task:2", {"v": 2})
        backend.save("ns", "user:1", {"v": 3})
        items = dict(backend.list("ns", prefix="task:"))
        assert set(items.keys()) == {"task:1", "task:2"}

    def test_list_empty_namespace(self) -> None:
        assert InMemoryStorageBackend().list("nonexistent") == []

    def test_delete_removes_entry(self) -> None:
        backend = InMemoryStorageBackend()
        backend.save("ns", "k", {"v": 1})
        backend.delete("ns", "k")
        assert backend.get("ns", "k") is None

    def test_delete_missing_is_noop(self) -> None:
        backend = InMemoryStorageBackend()
        backend.delete("ns", "missing")  # must not raise


class TestErrorHistoryAcceptsStorageBackend:
    def test_error_history_writes_the_canonical_namespace(self) -> None:
        """D-113: the namespace is `error_history`, not `errors`.

        This test used to assert `backend.list("errors")` — it pinned the name
        the decision renames. apcore-rust wrote `error_history`, so the same
        records were unqueryable across the two SDKs: a namespace IS the key a
        caller reads by.
        """
        from apcore.observability.storage import (
            STORAGE_NAMESPACE_ERROR_HISTORY,
            STORAGE_NAMESPACE_ERROR_HISTORY_LEGACY,
        )

        backend = InMemoryStorageBackend()
        hist = ErrorHistory(storage=backend)
        hist.record("mod.x", ModuleError(code="E1", message="boom"))

        entries = hist.get("mod.x")
        assert len(entries) == 1
        assert len(backend.list(STORAGE_NAMESPACE_ERROR_HISTORY)) >= 1
        assert backend.list(STORAGE_NAMESPACE_ERROR_HISTORY_LEGACY) == [], (
            "writes go only to the canonical namespace; the legacy one is read-only " "for the migration window"
        )

    def test_the_migration_window_reads_both_namespaces(self) -> None:
        """A rename is invisible until someone queries old data and finds nothing.

        Records written under the legacy name before the rename stay readable,
        and a fingerprint present in both is returned once.
        """
        from apcore.observability.storage import (
            STORAGE_NAMESPACE_ERROR_HISTORY,
            STORAGE_NAMESPACE_ERROR_HISTORY_LEGACY,
        )

        backend = InMemoryStorageBackend()
        hist = ErrorHistory(storage=backend)
        hist.record("mod.x", ModuleError(code="E1", message="boom"))
        new_keys = [key for key, _ in backend.list(STORAGE_NAMESPACE_ERROR_HISTORY)]

        backend.save(STORAGE_NAMESPACE_ERROR_HISTORY_LEGACY, "pre-rename-fp", {"module_id": "mod.old"})
        # The same fingerprint in BOTH namespaces must be returned once.
        backend.save(STORAGE_NAMESPACE_ERROR_HISTORY_LEGACY, new_keys[0], {"module_id": "stale"})

        keys = [key for key, _ in hist.stored_entries()]
        assert "pre-rename-fp" in keys, "legacy records must stay readable"
        assert keys.count(new_keys[0]) == 1, "a fingerprint in both namespaces is returned once"

    def test_an_omitted_backend_means_the_in_memory_one(self) -> None:
        """D-113: omitted is not "no storage".

        Only apcore-typescript honoured this, so the same omission produced a
        working store there and a silent no-op here — and a caller reading
        records back got an empty list rather than an error.
        """
        assert isinstance(ErrorHistory()._storage, InMemoryStorageBackend)
