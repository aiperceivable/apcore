"""Optional watchdog integration for filesystem-backed registry reloads."""

from __future__ import annotations

import os
import time
from collections.abc import Callable, Sequence
from typing import TYPE_CHECKING, Any
from weakref import ref

if TYPE_CHECKING:
    from watchdog.observers.api import BaseObserver

    from apcore.registry.registry import Registry


class _ModuleChangeHandler:
    """Debounce extension events and dispatch them without retaining the registry."""

    _DEBOUNCE_WINDOW_SEC: float = 0.3
    _MAX_DEBOUNCE_ENTRIES: int = 1024
    _DEBOUNCE_PRUNE_KEEP: int = 256

    def __init__(self, registry: Registry) -> None:
        self._registry_ref = ref(registry)
        self._debounce_timer: dict[str, float] = {}

    def _registry(self) -> Registry | None:
        return self._registry_ref()

    def _should_process(self, path: str) -> bool:
        if not path.endswith(".py"):
            return False
        now = time.time()
        last = self._debounce_timer.get(path, 0.0)
        if now - last < self._DEBOUNCE_WINDOW_SEC:
            return False
        self._debounce_timer[path] = now
        # Bounded memory: prune oldest entries when the dict grows too large.
        if len(self._debounce_timer) > self._MAX_DEBOUNCE_ENTRIES:
            sorted_by_age = sorted(self._debounce_timer.items(), key=lambda kv: kv[1])
            self._debounce_timer = dict(sorted_by_age[-self._DEBOUNCE_PRUNE_KEEP :])
        return True

    def on_modified(self, event: Any) -> None:
        if event.is_directory:
            return
        path = str(event.src_path)
        reg = self._registry()
        if reg is not None and not reg._defer_watched_change(path) and self._should_process(path):
            reg._handle_file_change(path)

    def on_created(self, event: Any) -> None:
        self.on_modified(event)

    def on_deleted(self, event: Any) -> None:
        if event.is_directory:
            return
        path = str(event.src_path)
        if not path.endswith(".py"):
            return
        reg = self._registry()
        if reg is not None:
            reg._handle_file_deletion(path)


def load_observer_factory() -> Callable[[Registry, Sequence[dict[str, Any]]], BaseObserver]:
    """Import watchdog before taking the registry lock and prepare its observer factory."""
    try:
        from watchdog.events import FileSystemEventHandler
        from watchdog.observers import Observer  # type: ignore[import-not-found]
    except ImportError:
        raise ImportError("watchdog is required for hot reload. Install it with: pip install watchdog")

    class WatchdogHandler(_ModuleChangeHandler, FileSystemEventHandler):
        pass

    def create_observer(registry: Registry, roots: Sequence[dict[str, Any]]) -> BaseObserver:
        observer = Observer()
        handler = WatchdogHandler(registry)
        for root_config in roots:
            root_path = root_config.get("root", "")
            if root_path and os.path.isdir(root_path):
                observer.schedule(handler, root_path, recursive=True)
        return observer

    return create_observer
