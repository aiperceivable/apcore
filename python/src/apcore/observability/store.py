"""Pluggable observability storage backends."""

from __future__ import annotations

import threading
from dataclasses import dataclass, field
from typing import Any, Protocol, runtime_checkable


@dataclass
class MetricPoint:
    """A single recorded metric observation."""

    name: str
    value: float
    module_id: str | None = None
    labels: dict[str, str] = field(default_factory=dict)
    timestamp: str = ""


@runtime_checkable
class ObservabilityStore(Protocol):
    """Protocol for pluggable observability storage backends."""

    def record_error(self, entry: Any) -> None: ...

    def get_errors(self, module_id: str | None = None, limit: int | None = None) -> list[Any]: ...

    def record_metric(self, metric: MetricPoint) -> None: ...

    def get_metrics(
        self,
        module_id: str | None = None,
        metric_name: str | None = None,
    ) -> list[MetricPoint]: ...

    def flush(self) -> None: ...

    def clear(self) -> None: ...


class InMemoryObservabilityStore:
    """Default in-memory observability store."""

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._errors: list[Any] = []
        self._metrics: list[MetricPoint] = []

    def record_error(self, entry: Any) -> None:
        with self._lock:
            self._errors.append(entry)

    def get_errors(self, module_id: str | None = None, limit: int | None = None) -> list[Any]:
        with self._lock:
            entries = self._errors if module_id is None else [e for e in self._errors if e.module_id == module_id]
            return list(entries[:limit] if limit is not None else entries)

    def record_metric(self, metric: MetricPoint) -> None:
        with self._lock:
            self._metrics.append(metric)

    def get_metrics(
        self,
        module_id: str | None = None,
        metric_name: str | None = None,
    ) -> list[MetricPoint]:
        with self._lock:
            metrics: list[MetricPoint] = list(self._metrics)
        if module_id is not None:
            metrics = [m for m in metrics if m.module_id == module_id]
        if metric_name is not None:
            metrics = [m for m in metrics if m.name == metric_name]
        return metrics

    def flush(self) -> None:
        pass

    def clear(self) -> None:
        with self._lock:
            self._errors.clear()
            self._metrics.clear()


__all__ = ["MetricPoint", "ObservabilityStore", "InMemoryObservabilityStore"]
