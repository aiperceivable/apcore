"""Generic pluggable storage backend (Issue #43 §1).

Defines a namespaced ``save / get / list / delete`` Protocol that
observability collectors (and other long-lived state holders) can use
for optional persistence.  The default in-memory implementation is
process-local; external backends (Redis, SQL, S3) are user-supplied.

This abstraction is intentionally minimal — values are plain ``dict``
records and keys are opaque strings.  The Protocol mirrors the shape of
the existing :class:`apcore.async_task.TaskStore` for consistency and is
exported from :mod:`apcore.observability` for discovery.
"""

from __future__ import annotations

import threading
from typing import Protocol, runtime_checkable

__all__ = [
    "StorageBackend",
    "InMemoryStorageBackend",
    "STORAGE_NAMESPACE_METRICS",
    "STORAGE_NAMESPACE_USAGE",
    "STORAGE_NAMESPACE_ERROR_HISTORY",
    "STORAGE_NAMESPACE_ERROR_HISTORY_LEGACY",
]


# ---------------------------------------------------------------------------
# Canonical namespaces (D-113)
# ---------------------------------------------------------------------------

#: The namespace each bundled collector writes under.
#:
#: metrics-and-usage.md § "Pluggable storage backends" makes the `StorageBackend`
#: argument a MUST, but an argument is only useful if what it holds can be found
#: again: a namespace is the key a caller queries by, so an unnamed one is an
#: argument that cannot be read back.
STORAGE_NAMESPACE_METRICS = "metrics"
STORAGE_NAMESPACE_USAGE = "usage"
STORAGE_NAMESPACE_ERROR_HISTORY = "error_history"

#: The namespace this SDK wrote error records under before D-113.
#:
#: Kept for the DUAL-READ migration window: a rename is invisible until someone
#: queries old data and finds nothing, so readers check the legacy namespace
#: after the canonical one. Writes go only to the canonical name. Removed once
#: the window closes — see the migration note in `observability.md`.
STORAGE_NAMESPACE_ERROR_HISTORY_LEGACY = "errors"


@runtime_checkable
class StorageBackend(Protocol):
    """Pluggable namespaced key-value storage protocol.

    Implementations may be in-memory, Redis-backed, SQL-backed, or any
    other key-value-shaped store.  All methods are synchronous; async
    backends must adapt their I/O to a sync surface.
    """

    def save(self, namespace: str, key: str, value: dict) -> None:
        """Insert or overwrite ``value`` at ``namespace/key``."""
        ...

    def get(self, namespace: str, key: str) -> dict | None:
        """Return the value at ``namespace/key`` or ``None`` if absent."""
        ...

    def list(self, namespace: str, prefix: str = "") -> list[tuple[str, dict]]:
        """Return ``(key, value)`` pairs under ``namespace``.

        If ``prefix`` is supplied, only keys starting with ``prefix`` are
        returned.  Order is implementation-defined — callers that need a
        stable order must sort the result themselves.
        """
        ...

    def delete(self, namespace: str, key: str) -> None:
        """Remove ``namespace/key``.  No-op if the key does not exist."""
        ...


class InMemoryStorageBackend:
    """Default in-memory :class:`StorageBackend` backed by nested dicts.

    Thread-safe.  Suitable for development, tests, and single-process
    deployments.  Persistence requires a user-supplied backend.
    """

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._data: dict[str, dict[str, dict]] = {}

    def save(self, namespace: str, key: str, value: dict) -> None:
        with self._lock:
            self._data.setdefault(namespace, {})[key] = dict(value)

    def get(self, namespace: str, key: str) -> dict | None:
        with self._lock:
            ns = self._data.get(namespace)
            if ns is None:
                return None
            value = ns.get(key)
            return dict(value) if value is not None else None

    def list(self, namespace: str, prefix: str = "") -> list[tuple[str, dict]]:
        with self._lock:
            ns = self._data.get(namespace, {})
            return [(k, dict(v)) for k, v in ns.items() if k.startswith(prefix)]

    def delete(self, namespace: str, key: str) -> None:
        with self._lock:
            ns = self._data.get(namespace)
            if ns is not None:
                ns.pop(key, None)
