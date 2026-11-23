"""Audit trail for system control modules (Issue #45 §1.2)."""

from __future__ import annotations

import threading
from dataclasses import dataclass, field
from datetime import datetime
from typing import Any, Protocol, runtime_checkable

__all__ = ["AuditEntry", "AuditStore", "InMemoryAuditStore"]


@dataclass
class AuditEntry:
    """Structured audit record for a single system control operation."""

    timestamp: str
    action: str  # "update_config" | "reload_module" | "toggle_feature"
    target_module_id: str
    actor_id: str
    actor_type: str
    trace_id: str
    change: dict[str, Any] = field(default_factory=dict)
    correlation_id: str = ""
    """Groups the entries a single multi-module operation produced (D-111).

    A bulk reload writes one entry PER MODULE, because
    ``AuditStore.query(module_id=...)`` filters on a concrete id and cannot find
    an entry keyed on the glob. Per-module entries alone lose the fact that they
    were one deploy, so every entry from one bulk reload carries the same
    correlation id and "what did this deploy touch" stays a single query.

    Empty for single-target operations, which need no grouping. Optional so an
    existing ``AuditStore`` implementation keeps working unchanged.
    """


@runtime_checkable
class AuditStore(Protocol):
    """Protocol for audit entry storage backends."""

    def append(self, entry: AuditEntry) -> None:
        """Persist a single audit entry."""
        ...

    def query(
        self,
        module_id: str | None = None,
        actor_id: str | None = None,
        since: str | None = None,
    ) -> list[AuditEntry]:
        """Return entries matching the given filters.

        Args:
            module_id: If set, return only entries for this target_module_id.
            actor_id: If set, return only entries for this actor_id.
            since: ISO 8601 timestamp string; return only entries at or after it.
        """
        ...


class InMemoryAuditStore:
    """Thread-safe in-memory implementation of AuditStore."""

    def __init__(self) -> None:
        self._entries: list[AuditEntry] = []
        self._lock = threading.Lock()

    def append(self, entry: AuditEntry) -> None:
        """Append an audit entry to the in-memory log."""
        with self._lock:
            self._entries.append(entry)

    def query(
        self,
        module_id: str | None = None,
        actor_id: str | None = None,
        since: str | None = None,
    ) -> list[AuditEntry]:
        """Return entries matching the given filters (all filters are ANDed)."""
        with self._lock:
            entries = list(self._entries)

        if module_id is not None:
            entries = [e for e in entries if e.target_module_id == module_id]
        if actor_id is not None:
            entries = [e for e in entries if e.actor_id == actor_id]
        if since is not None:
            since_dt = datetime.fromisoformat(since)
            entries = [e for e in entries if datetime.fromisoformat(e.timestamp) >= since_dt]

        return entries
