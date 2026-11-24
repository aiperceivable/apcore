"""Pluggable override-store interface for runtime config + toggle persistence.

Cross-language counterpart to:
- TypeScript: ``apcore-typescript/src/sys-modules/overrides.ts``
- Rust:       ``apcore-rust/src/sys_modules/overrides.rs``

Each override is a flat ``key → value`` mapping where ``key`` is the
dot-path config key (e.g. ``"executor.default_timeout"``) or a toggle
override prefixed ``toggle.`` (e.g. ``"toggle.system.health.module"``).

Embeddings (tests, multi-tenant deployments, remote config services) can
swap the storage layer without forking ``register_sys_modules`` or
the control sys-modules.
"""

from __future__ import annotations

import json
import logging
import os
import tempfile
import threading
from _thread import LockType
from pathlib import Path
from typing import Any, Protocol, runtime_checkable
from weakref import WeakValueDictionary

import yaml

logger = logging.getLogger(__name__)

_update_locks: WeakValueDictionary[Path | int, LockType] = WeakValueDictionary()
_update_locks_guard = threading.Lock()


__all__ = [
    "OverridesStore",
    "InMemoryOverridesStore",
    "FileOverridesStore",
]


@runtime_checkable
class OverridesStore(Protocol):
    """Pluggable persistence for runtime config & toggle overrides.

    Implementations must be safe to call from multiple call sites in
    sequence; the :class:`FileOverridesStore` reference implementation
    uses an atomic tempfile + rename to prevent partial-write corruption.
    """

    def load(self) -> dict[str, Any]:
        """Load all overrides from the backing store. Returns ``{}`` if empty/missing."""
        ...

    def save(self, overrides: dict[str, Any]) -> None:
        """Replace the entire override set with the given mapping."""
        ...


class InMemoryOverridesStore:
    """In-memory store, primarily for tests and embeddings without disk persistence."""

    def __init__(self, initial: dict[str, Any] | None = None) -> None:
        self._data: dict[str, Any] = dict(initial) if initial else {}

    def load(self) -> dict[str, Any]:
        return dict(self._data)

    def save(self, overrides: dict[str, Any]) -> None:
        self._data = dict(overrides)


class FileOverridesStore:
    """YAML-file-backed override store with atomic write semantics.

    Each :meth:`save` writes to a sibling tempfile and ``os.replace``-es it
    into place so an interrupted process cannot leave a half-written
    overrides file behind. ``load()`` accepts both YAML and JSON content
    (YAML is a superset of JSON, so plain JSON files round-trip cleanly).
    """

    def __init__(self, path: str | os.PathLike[str]) -> None:
        self._raw_path = os.fspath(path)
        self._path = Path(path)

    @property
    def path(self) -> Path:
        """Path of the underlying YAML file."""
        return self._path

    def load(self) -> dict[str, Any]:
        try:
            return self._read_mapping()
        except (OSError, ValueError) as exc:
            logger.warning("[apcore:overrides] %s", exc)
            return {}

    def _read_mapping(self) -> dict[str, Any]:
        """Read the authoritative mapping before replacing it."""
        try:
            with open(self._raw_path, encoding="utf-8") as source:
                raw = source.read()
        except FileNotFoundError:
            return {}
        except (OSError, UnicodeError) as exc:
            raise OSError(f"Failed to read overrides file {self._path}: {exc}") from exc
        if not raw:
            return {}
        try:
            parsed: Any = yaml.safe_load(raw)
        except yaml.YAMLError as exc:
            # Fall back to JSON in case the YAML loader is unhappy with a
            # JSON file edge case (rare, but documented for parity with TS).
            try:
                parsed = json.loads(raw)
            except json.JSONDecodeError:
                raise ValueError(f"Overrides file {self._path} is not valid YAML/JSON: {exc}") from exc
        if isinstance(parsed, dict):
            return dict(parsed)
        if parsed is None:
            return {}
        raise ValueError(f"Overrides file {self._path} root is not a mapping")

    def save(self, overrides: dict[str, Any]) -> None:
        directory = self._path.parent if str(self._path.parent) else Path(".")
        try:
            directory.mkdir(parents=True, exist_ok=True)
        except OSError as exc:
            logger.error(
                "[apcore:overrides] Failed to create parent directory %s: %s",
                directory,
                exc,
            )
            return

        # Sort keys for stable on-disk layout (parity with TS sortKeys).
        ordered: dict[str, Any] = {key: overrides[key] for key in sorted(overrides)}

        try:
            fd, tmp_path = tempfile.mkstemp(
                dir=str(directory),
                prefix=f".{self._path.name}.",
                suffix=".tmp",
            )
        except OSError as exc:
            logger.error(
                "[apcore:overrides] Failed to create tempfile next to %s: %s",
                self._path,
                exc,
            )
            return

        try:
            with os.fdopen(fd, "w", encoding="utf-8") as f:
                yaml.safe_dump(ordered, f, default_flow_style=False, sort_keys=True)
            os.replace(tmp_path, self._raw_path)
        except Exception as exc:
            logger.error(
                "[apcore:overrides] Failed to write overrides file %s: %s",
                self._path,
                exc,
            )
            try:
                os.unlink(tmp_path)
            except OSError:
                pass


def _persist_override(store: OverridesStore, key: str, value: Any) -> None:
    """Merge one override under the lock shared by all writers to the store."""
    try:
        load = store.load
        lock_key: Path | int = id(store)
        if isinstance(store, FileOverridesStore):
            lock_key = store.path.parent.resolve() / store.path.name
            if type(store).load is FileOverridesStore.load:
                load = store._read_mapping
        with _update_locks_guard:
            lock = _update_locks.get(lock_key)
            if lock is None:
                lock = _update_locks[lock_key] = threading.Lock()
        with lock:
            existing = load()
            if not isinstance(existing, dict):
                raise ValueError("Overrides store did not return a mapping")
            store.save({**existing, key: value})
    except Exception as exc:
        logger.error("Failed to persist override for key '%s': %s", key, exc)
