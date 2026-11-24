"""Tests for system.health.summary and system.health.module sys modules."""

from __future__ import annotations

from typing import Any

import pytest

from apcore.config import Config
from apcore.errors import InvalidInputError, ModuleError, ModuleNotFoundError
from apcore.observability.error_history import ErrorHistory
from apcore.observability.metrics import MetricsCollector
from apcore.registry.registry import Registry
from apcore.sys_modules.health import (
    HealthModuleModule,
    HealthSummaryModule,
    classify_health_status,
)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


class _DummyModule:
    description = "dummy"

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        return {}


def _make_deps(
    *,
    register_module: bool = True,
    module_id: str = "test.mod",
) -> tuple[Registry, MetricsCollector, ErrorHistory, str]:
    """Create common dependencies and optionally register a dummy module."""
    registry = Registry()
    metrics = MetricsCollector()
    error_history = ErrorHistory()
    if register_module:
        registry.register(module_id, _DummyModule())
    return registry, metrics, error_history, module_id


def _simulate_calls(
    metrics: MetricsCollector,
    module_id: str,
    *,
    success: int = 0,
    error: int = 0,
    duration_s: float = 0.05,
) -> None:
    """Record success/error calls and durations in the metrics collector."""
    for _ in range(success):
        metrics.increment_calls(module_id, "success")
        metrics.observe_duration(module_id, duration_s)
    for _ in range(error):
        metrics.increment_calls(module_id, "error")
        metrics.increment_errors(module_id, "SOME_ERROR")
        metrics.observe_duration(module_id, duration_s)


def _record_errors(
    error_history: ErrorHistory,
    module_id: str,
    count: int,
) -> None:
    """Record distinct errors in the error history."""
    from apcore.errors import ModuleError

    for i in range(count):
        err = ModuleError(
            code=f"ERR_{i}",
            message=f"Error number {i}",
            ai_guidance=f"Fix error {i}",
        )
        error_history.record(module_id, err)


# ---------------------------------------------------------------------------
# Tests: HealthModuleModule
# ---------------------------------------------------------------------------


class TestHealthModuleRequiresModuleId:
    def test_health_module_requires_module_id(self) -> None:
        registry, metrics, error_history, _ = _make_deps(register_module=False)
        mod = HealthModuleModule(registry, metrics, error_history)
        with pytest.raises(InvalidInputError):
            mod.execute({}, None)


class TestHealthModuleNotFound:
    def test_health_module_not_found_error(self) -> None:
        registry, metrics, error_history, _ = _make_deps(register_module=False)
        mod = HealthModuleModule(registry, metrics, error_history)
        with pytest.raises(ModuleNotFoundError):
            mod.execute({"module_id": "nonexistent.mod"}, None)


class TestHealthModuleBasicInfo:
    def test_health_module_returns_basic_info(self) -> None:
        registry, metrics, error_history, module_id = _make_deps()
        _simulate_calls(metrics, module_id, success=90, error=10)
        mod = HealthModuleModule(registry, metrics, error_history)
        result = mod.execute({"module_id": module_id}, None)

        assert result["module_id"] == module_id
        assert "status" in result
        assert result["total_calls"] == 100
        assert result["error_count"] == 10
        assert result["error_rate"] == pytest.approx(0.10)


class TestHealthModuleLatencyMetrics:
    def test_health_module_returns_latency_metrics(self) -> None:
        registry, metrics, error_history, module_id = _make_deps()
        _simulate_calls(metrics, module_id, success=10, duration_s=0.1)
        mod = HealthModuleModule(registry, metrics, error_history)
        result = mod.execute({"module_id": module_id}, None)

        assert "avg_latency_ms" in result
        assert result["avg_latency_ms"] == pytest.approx(100.0)
        assert "p99_latency_ms" in result
        assert isinstance(result["p99_latency_ms"], float)


class TestHealthModuleRecentErrors:
    def test_health_module_returns_recent_errors(self) -> None:
        registry, metrics, error_history, module_id = _make_deps()
        _simulate_calls(metrics, module_id, success=10, error=2)
        _record_errors(error_history, module_id, 2)

        mod = HealthModuleModule(registry, metrics, error_history)
        result = mod.execute({"module_id": module_id}, None)

        assert "recent_errors" in result
        assert len(result["recent_errors"]) == 2
        err = result["recent_errors"][0]
        assert "code" in err
        assert "message" in err
        assert "ai_guidance" in err
        assert "count" in err
        assert "first_occurred" in err
        assert "last_occurred" in err


class TestHealthModuleErrorLimitDefault:
    def test_health_module_error_limit_default(self) -> None:
        registry, metrics, error_history, module_id = _make_deps()
        _record_errors(error_history, module_id, 15)

        mod = HealthModuleModule(registry, metrics, error_history)
        result = mod.execute({"module_id": module_id}, None)

        assert len(result["recent_errors"]) == 10


class TestHealthModuleCustomErrorLimit:
    def test_health_module_custom_error_limit(self) -> None:
        registry, metrics, error_history, module_id = _make_deps()
        _record_errors(error_history, module_id, 15)

        mod = HealthModuleModule(registry, metrics, error_history)
        result = mod.execute({"module_id": module_id, "error_limit": 5}, None)

        assert len(result["recent_errors"]) == 5


class TestHealthModuleStatusHealthy:
    def test_health_module_status_healthy(self) -> None:
        registry, metrics, error_history, module_id = _make_deps()
        # error_rate < 1%: 0 errors out of 100
        _simulate_calls(metrics, module_id, success=100, error=0)
        mod = HealthModuleModule(registry, metrics, error_history)
        result = mod.execute({"module_id": module_id}, None)
        assert result["status"] == "healthy"


class TestHealthModuleStatusDegraded:
    def test_health_module_status_degraded(self) -> None:
        registry, metrics, error_history, module_id = _make_deps()
        # error_rate 5% (between 1% and 10%)
        _simulate_calls(metrics, module_id, success=95, error=5)
        mod = HealthModuleModule(registry, metrics, error_history)
        result = mod.execute({"module_id": module_id}, None)
        assert result["status"] == "degraded"


class TestHealthModuleStatusError:
    def test_health_module_status_error(self) -> None:
        registry, metrics, error_history, module_id = _make_deps()
        # error_rate 20% (>= 10%)
        _simulate_calls(metrics, module_id, success=80, error=20)
        mod = HealthModuleModule(registry, metrics, error_history)
        result = mod.execute({"module_id": module_id}, None)
        assert result["status"] == "error"


class TestHealthModuleStatusUnknown:
    def test_health_module_status_unknown(self) -> None:
        registry, metrics, error_history, module_id = _make_deps()
        # 0 calls
        mod = HealthModuleModule(registry, metrics, error_history)
        result = mod.execute({"module_id": module_id}, None)
        assert result["status"] == "unknown"


class TestHealthModuleAnnotations:
    def test_health_module_annotations(self) -> None:
        registry, metrics, error_history, _ = _make_deps(register_module=False)
        mod = HealthModuleModule(registry, metrics, error_history)
        assert mod.annotations.readonly is True
        assert mod.annotations.idempotent is True


# ---------------------------------------------------------------------------
# Tests: classify_health_status helper
# ---------------------------------------------------------------------------


class TestClassifyStatus:
    def testclassify_health_status_unknown_zero_calls(self) -> None:
        assert classify_health_status(0.0, 0) == "unknown"

    def testclassify_health_status_healthy(self) -> None:
        assert classify_health_status(0.005, 100) == "healthy"

    def testclassify_health_status_degraded(self) -> None:
        assert classify_health_status(0.05, 100) == "degraded"

    def testclassify_health_status_error(self) -> None:
        assert classify_health_status(0.10, 100) == "error"

    def testclassify_health_status_boundary_one_percent(self) -> None:
        # Exactly 1% should be degraded
        assert classify_health_status(0.01, 100) == "degraded"

    def testclassify_health_status_boundary_ten_percent(self) -> None:
        # Exactly 10% should be error
        assert classify_health_status(0.10, 100) == "error"


# ---------------------------------------------------------------------------
# Tests: HealthSummaryModule
# ---------------------------------------------------------------------------


class TestHealthSummaryAnnotations:
    def test_health_summary_module_annotations(self) -> None:
        registry, metrics, error_history, _ = _make_deps(register_module=False)
        mod = HealthSummaryModule(registry, metrics, error_history)
        assert mod.annotations.readonly is True
        assert mod.annotations.idempotent is True


class TestHealthSummaryHasExecute:
    def test_health_summary_has_execute_method(self) -> None:
        registry, metrics, error_history, _ = _make_deps(register_module=False)
        mod = HealthSummaryModule(registry, metrics, error_history)
        assert callable(getattr(mod, "execute", None))


class TestHealthSummaryProjectInfo:
    def test_health_summary_returns_project_info(self) -> None:
        registry, metrics, error_history, _ = _make_deps(register_module=False)
        mod = HealthSummaryModule(registry, metrics, error_history)
        result = mod.execute({}, None)
        assert "project" in result
        assert result["project"]["name"] == "apcore"

    def test_health_summary_returns_project_name_from_config(self) -> None:
        registry, metrics, error_history, _ = _make_deps(register_module=False)
        config = Config.from_defaults()
        mod = HealthSummaryModule(registry, metrics, error_history, config=config)
        result = mod.execute({}, None)
        assert result["project"]["name"] == "apcore"


class TestHealthSummaryNoModules:
    def test_health_summary_no_modules_registered(self) -> None:
        registry, metrics, error_history, _ = _make_deps(register_module=False)
        mod = HealthSummaryModule(registry, metrics, error_history)
        result = mod.execute({}, None)
        assert result["summary"]["total_modules"] == 0
        assert result["summary"]["healthy"] == 0
        assert result["summary"]["degraded"] == 0
        assert result["summary"]["error"] == 0
        assert result["summary"]["unknown"] == 0
        assert result["modules"] == []


class TestHealthSummaryCounts:
    def test_health_summary_returns_summary_counts(self) -> None:
        registry = Registry()
        metrics = MetricsCollector()
        error_history = ErrorHistory()

        # Register 3 modules with different statuses
        registry.register("mod.healthy", _DummyModule())
        registry.register("mod.degraded", _DummyModule())
        registry.register("mod.error", _DummyModule())

        _simulate_calls(metrics, "mod.healthy", success=100, error=0)
        _simulate_calls(metrics, "mod.degraded", success=95, error=5)
        _simulate_calls(metrics, "mod.error", success=80, error=20)

        mod = HealthSummaryModule(registry, metrics, error_history)
        result = mod.execute({}, None)

        assert result["summary"]["total_modules"] == 3
        assert result["summary"]["healthy"] == 1
        assert result["summary"]["degraded"] == 1
        assert result["summary"]["error"] == 1


class TestHealthSummaryStatusClassification:
    def test_health_summary_status_healthy(self) -> None:
        registry, metrics, error_history, mid = _make_deps()
        _simulate_calls(metrics, mid, success=100, error=0)
        mod = HealthSummaryModule(registry, metrics, error_history)
        result = mod.execute({}, None)
        assert result["modules"][0]["status"] == "healthy"

    def test_health_summary_status_degraded(self) -> None:
        registry, metrics, error_history, mid = _make_deps()
        _simulate_calls(metrics, mid, success=95, error=5)
        mod = HealthSummaryModule(registry, metrics, error_history)
        result = mod.execute({}, None)
        assert result["modules"][0]["status"] == "degraded"

    def test_health_summary_status_error(self) -> None:
        registry, metrics, error_history, mid = _make_deps()
        _simulate_calls(metrics, mid, success=80, error=20)
        mod = HealthSummaryModule(registry, metrics, error_history)
        result = mod.execute({}, None)
        assert result["modules"][0]["status"] == "error"

    def test_health_summary_status_unknown(self) -> None:
        registry, metrics, error_history, mid = _make_deps()
        mod = HealthSummaryModule(registry, metrics, error_history)
        result = mod.execute({}, None)
        assert result["modules"][0]["status"] == "unknown"


class TestHealthSummaryModulesArray:
    def test_health_summary_modules_array(self) -> None:
        registry, metrics, error_history, mid = _make_deps()
        _simulate_calls(metrics, mid, success=90, error=10)
        mod = HealthSummaryModule(registry, metrics, error_history)
        result = mod.execute({}, None)
        entry = result["modules"][0]
        assert entry["module_id"] == mid
        assert "status" in entry
        assert "error_rate" in entry
        assert "top_error" in entry


class TestHealthSummaryTopError:
    def test_health_summary_top_error_from_error_history(self) -> None:
        registry, metrics, error_history, mid = _make_deps()
        err = ModuleError(code="E001", message="Bad input", ai_guidance="Fix the input")
        error_history.record(mid, err)
        mod = HealthSummaryModule(registry, metrics, error_history)
        result = mod.execute({}, None)
        top_error = result["modules"][0]["top_error"]
        assert top_error is not None
        assert top_error["code"] == "E001"
        assert top_error["message"] == "Bad input"
        assert top_error["ai_guidance"] == "Fix the input"
        assert top_error["count"] == 1


class TestHealthSummaryCustomThreshold:
    def test_health_summary_custom_error_rate_threshold(self) -> None:
        registry, metrics, error_history, mid = _make_deps()
        # 3% error rate — normally degraded, but with threshold=0.05 it's healthy
        _simulate_calls(metrics, mid, success=97, error=3)
        mod = HealthSummaryModule(registry, metrics, error_history)
        result = mod.execute({"error_rate_threshold": 0.05}, None)
        assert result["modules"][0]["status"] == "healthy"


class TestHealthSummaryIncludeHealthy:
    def test_health_summary_include_healthy_false(self) -> None:
        registry = Registry()
        metrics = MetricsCollector()
        error_history = ErrorHistory()
        registry.register("mod.healthy", _DummyModule())
        registry.register("mod.error", _DummyModule())
        _simulate_calls(metrics, "mod.healthy", success=100, error=0)
        _simulate_calls(metrics, "mod.error", success=80, error=20)

        mod = HealthSummaryModule(registry, metrics, error_history)
        result = mod.execute({"include_healthy": False}, None)
        module_ids = [m["module_id"] for m in result["modules"]]
        assert "mod.healthy" not in module_ids
        assert "mod.error" in module_ids

    def test_health_summary_include_healthy_true(self) -> None:
        registry = Registry()
        metrics = MetricsCollector()
        error_history = ErrorHistory()
        registry.register("mod.healthy", _DummyModule())
        registry.register("mod.error", _DummyModule())
        _simulate_calls(metrics, "mod.healthy", success=100, error=0)
        _simulate_calls(metrics, "mod.error", success=80, error=20)

        mod = HealthSummaryModule(registry, metrics, error_history)
        result = mod.execute({"include_healthy": True}, None)
        module_ids = [m["module_id"] for m in result["modules"]]
        assert "mod.healthy" in module_ids
        assert "mod.error" in module_ids
