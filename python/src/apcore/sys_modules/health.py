"""system.health sys modules -- summary and single-module health."""

from __future__ import annotations

from typing import Any

from apcore.config import Config
from apcore.errors import InvalidInputError, ModuleNotFoundError
from apcore.module import ModuleAnnotations
from apcore.observability.error_history import ErrorHistory
from apcore.observability.metrics import MetricsCollector, module_call_counts, module_latency_ms
from apcore.registry.registry import Registry

__all__ = [
    "HealthSummaryModule",
    "HealthModule",
    "HealthModuleModule",
    "classify_health_status",
]

# Default thresholds for status classification.
_DEFAULT_HEALTHY_THRESHOLD = 0.01  # < 1% error rate
_DEFAULT_DEGRADED_THRESHOLD = 0.10  # < 10% error rate


def classify_health_status(
    error_rate: float,
    total_calls: int,
    healthy_threshold: float = _DEFAULT_HEALTHY_THRESHOLD,
    degraded_threshold: float = _DEFAULT_DEGRADED_THRESHOLD,
) -> str:
    """Classify a module's health status based on error rate and call count.

    Args:
        error_rate: Error rate as a float (0.0-1.0).
        total_calls: Total number of calls.
        healthy_threshold: Error rate below which status is "healthy" (default 1%).
        degraded_threshold: Error rate below which status is "degraded" (default 10%).

    Returns:
        ``"unknown"`` if no calls, ``"healthy"``, ``"degraded"``, or ``"error"``.
    """
    if total_calls == 0:
        return "unknown"
    if error_rate < healthy_threshold:
        return "healthy"
    if error_rate < degraded_threshold:
        return "degraded"
    return "error"


class HealthSummaryModule:
    """Aggregated health overview of all registered modules."""

    description = "Aggregated health overview of all registered modules"
    # D-119: written out rather than inherited — the ModuleAnnotations default is
    # `open_world=True`, which means the opposite of the intended value, and relying
    # on it is how the divergence arose. No system module reaches an external system.
    annotations = ModuleAnnotations(readonly=True, idempotent=True, open_world=False)
    input_schema: dict[str, Any] = {
        "type": "object",
        "properties": {
            "error_rate_threshold": {
                "type": "number",
                "description": "Error rate threshold for healthy status",
                "default": 0.01,
            },
            "include_healthy": {
                "type": "boolean",
                "description": "Whether to include healthy modules in output",
                "default": True,
            },
        },
    }
    output_schema: dict[str, Any] = {
        "type": "object",
        "properties": {
            "project": {"type": "object", "description": "Project information"},
            "summary": {
                "type": "object",
                "description": "Aggregated health counts by status",
            },
            "modules": {"type": "array", "description": "Per-module health entries"},
        },
        "required": ["project", "summary", "modules"],
    }

    def __init__(
        self,
        registry: Registry,
        metrics_collector: MetricsCollector,
        error_history: ErrorHistory,
        config: Config | None = None,
    ) -> None:
        self._registry = registry
        self._metrics = metrics_collector
        self._error_history = error_history
        self._config = config

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        """Produce an aggregated health summary of all registered modules."""
        healthy_threshold = float(inputs.get("error_rate_threshold", _DEFAULT_HEALTHY_THRESHOLD))
        # D-109: `error_rate_threshold` moves the FIRST boundary only. The
        # degraded/error boundary is the classification table's fixed 0.10.
        #
        # This used to be `healthy_threshold * 10.0`, which silently gave the
        # caller a second knob the table does not define: with
        # `error_rate_threshold: 0.001` a module erroring 5% of the time was
        # classified `error` here and `degraded` by apcore-rust, which reads the
        # table. The consequence is recorded in the contract because it
        # surprises — at 0.001, `degraded` spans 0.1%-10% — and a caller wanting
        # a stricter ERROR boundary is asking for a knob that deliberately does
        # not exist.
        degraded_threshold = _DEFAULT_DEGRADED_THRESHOLD
        include_healthy: bool = inputs.get("include_healthy", True)

        project_name = self._get_project_name()
        module_ids = self._registry.list()

        modules_output: list[dict[str, Any]] = []
        counts = {"healthy": 0, "degraded": 0, "error": 0, "unknown": 0}

        for mid in module_ids:
            total_calls, error_calls = self._get_call_counts(mid)
            error_rate = self._compute_error_rate(total_calls, error_calls)
            status = classify_health_status(error_rate, total_calls, healthy_threshold, degraded_threshold)
            counts[status] += 1

            if not include_healthy and status == "healthy":
                continue

            top_error = self._get_top_error(mid)
            modules_output.append(
                {
                    "module_id": mid,
                    "status": status,
                    "error_rate": error_rate,
                    "top_error": top_error,
                }
            )

        return {
            "project": {"name": project_name},
            "summary": {
                "total_modules": len(module_ids),
                **counts,
            },
            "modules": modules_output,
        }

    # ------------------------------------------------------------------
    # Private helpers
    # ------------------------------------------------------------------

    def _get_project_name(self) -> str:
        """Extract project name from config."""
        if self._config is not None:
            name = self._config.get("project.name")
            if name:
                return str(name)
        return "apcore"

    def _get_call_counts(self, module_id: str) -> tuple[int, int]:
        """Return (total_calls, error_calls) for a module from metrics."""
        return module_call_counts(self._metrics, module_id)

    @staticmethod
    def _compute_error_rate(total: int, errors: int) -> float:
        """Compute error rate; 0.0 when no calls."""
        if total == 0:
            return 0.0
        return errors / total

    def _get_top_error(self, module_id: str) -> dict[str, Any] | None:
        """Get the most frequent error for a module from ErrorHistory."""
        entries = self._error_history.get(module_id)
        if not entries:
            return None
        top = max(entries, key=lambda e: e.count)
        return {
            "code": top.code,
            "message": top.message,
            "ai_guidance": top.ai_guidance,
            "count": top.count,
        }


# ------------------------------------------------------------------
# system.health.module
# ------------------------------------------------------------------


class HealthModule:
    """Detailed health information for a single module."""

    description = "Detailed health information for a single module"
    # D-119: written out rather than inherited — the ModuleAnnotations default is
    # `open_world=True`, which means the opposite of the intended value, and relying
    # on it is how the divergence arose. No system module reaches an external system.
    annotations = ModuleAnnotations(readonly=True, idempotent=True, open_world=False)
    input_schema: dict[str, Any] = {
        "type": "object",
        "properties": {
            "module_id": {
                "type": "string",
                "description": "ID of the module to inspect",
            },
            "error_limit": {
                "type": "integer",
                "description": "Max number of recent errors to return",
                "default": 10,
            },
        },
        "required": ["module_id"],
    }
    output_schema: dict[str, Any] = {
        "type": "object",
        "properties": {
            "module_id": {"type": "string", "description": "Module identifier"},
            "status": {
                "type": "string",
                "description": "Health status: healthy, degraded, error, or unknown",
            },
            "total_calls": {"type": "integer", "description": "Total number of calls"},
            "error_count": {"type": "integer", "description": "Total number of errors"},
            "error_rate": {
                "type": "number",
                "description": "Error rate as a float (0.0-1.0)",
            },
            "avg_latency_ms": {
                "type": "number",
                "description": "Average latency in milliseconds",
            },
            "p99_latency_ms": {
                "type": "number",
                "description": "99th percentile latency in milliseconds",
            },
            "recent_errors": {"type": "array", "description": "Recent error entries"},
        },
        "required": ["module_id", "status", "total_calls", "error_count", "error_rate"],
    }

    def __init__(
        self,
        registry: Registry,
        metrics_collector: MetricsCollector,
        error_history: ErrorHistory,
    ) -> None:
        self._registry = registry
        self._metrics = metrics_collector
        self._error_history = error_history

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        """Return detailed health info for the specified module."""
        module_id = inputs.get("module_id")
        if not module_id:
            raise InvalidInputError(message="module_id is required")

        if not self._registry.has(module_id):
            raise ModuleNotFoundError(module_id=module_id)

        error_limit: int = inputs.get("error_limit", 10)

        total_calls, error_count = self._get_call_counts(module_id)
        error_rate = error_count / total_calls if total_calls > 0 else 0.0
        status = classify_health_status(error_rate, total_calls)
        avg_latency_ms, p99_latency_ms = self._get_latency(module_id)
        recent_errors = self._get_recent_errors(module_id, error_limit)

        return {
            "module_id": module_id,
            "status": status,
            "total_calls": total_calls,
            "error_count": error_count,
            "error_rate": error_rate,
            "avg_latency_ms": avg_latency_ms,
            "p99_latency_ms": p99_latency_ms,
            "recent_errors": recent_errors,
        }

    # ------------------------------------------------------------------
    # Private helpers
    # ------------------------------------------------------------------

    def _get_call_counts(self, module_id: str) -> tuple[int, int]:
        """Return (total_calls, error_calls) for a module from metrics."""
        return module_call_counts(self._metrics, module_id)

    def _get_latency(self, module_id: str) -> tuple[float, float]:
        """Return (avg_latency_ms, p99_latency_ms) from histogram data."""
        return module_latency_ms(self._metrics, module_id)

    def _get_recent_errors(self, module_id: str, limit: int) -> list[dict[str, Any]]:
        """Return recent errors formatted as dicts."""
        entries = self._error_history.get(module_id, limit=limit)
        return [
            {
                "code": e.code,
                "message": e.message,
                "ai_guidance": e.ai_guidance,
                "count": e.count,
                "first_occurred": e.first_occurred,
                "last_occurred": e.last_occurred,
            }
            for e in entries
        ]


#: Backward-compatible alias for :class:`HealthModule`.
HealthModuleModule = HealthModule
