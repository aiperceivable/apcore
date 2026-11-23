"""Conformance tests for System Modules Hardening (Issue #45).

Drives the canonical ``apcore/conformance/fixtures/system_modules_hardening.json``.
Each case needs its own registry / config / filesystem wiring, so the assertions
are hand-written rather than generated from the fixture; ``TestFixtureCoverage``
at the bottom holds the two in step, so a case added on the spec side fails here
instead of going unnoticed.

Covers all 11 test cases defined in the canonical conformance fixture:

  1.  overrides_persisted_on_update
  2.  overrides_loaded_on_startup
  3.  audit_entry_records_actor
  4.  audit_entry_records_change
  5.  prometheus_usage_exports_calls_total
  6.  reload_with_path_filter
  7.  reload_module_id_and_filter_conflict
  8.  startup_fail_on_error_true_raises
  9.  startup_fail_on_error_false_continues
  10. rust_register_returns_result  (language=rust — skipped for Python)
  11. reload_order_is_topological_not_alphabetical
"""

from __future__ import annotations

import logging
import os
import sys
import time
from pathlib import Path
from typing import Any
from unittest.mock import MagicMock, patch

import pytest
import yaml

from apcore import APCore
from apcore.config import Config
from apcore.errors import ModuleReloadConflictError, SysModuleRegistrationError
from apcore.events.emitter import EventEmitter
from apcore.observability.metrics import MetricsCollector
from apcore.observability.usage import UsageCollector
from apcore.registry.registry import Registry
from apcore.sys_modules.audit import InMemoryAuditStore
from apcore.sys_modules.control import (
    ReloadModule,
    ToggleFeatureModule,
    ToggleState,
    UpdateConfigModule,
)
from apcore.sys_modules.health import HealthSummaryModule
from apcore.sys_modules.manifest import ManifestFullModule
from apcore.sys_modules.registration import register_sys_modules

from conformance.canonical_fixtures import case_ids, load_fixture

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

FIXTURE = "system_modules_hardening.json"

#: canonical case id -> case body. The assertions below stay hand-written (each
#: case needs its own registry / config / filesystem wiring), but the values
#: they compare against come from here — an ``expected`` key that no assertion
#: reads is not a contract, it only looks like one in the fixture.
_CASES: dict[str, Any] = {case["id"]: case for case in load_fixture(FIXTURE)["test_cases"]}


def _case(case_id: str) -> dict[str, Any]:
    return _CASES[case_id]


def _assert_topological_order(
    order: list[str],
    matched: list[str],
    declared_dependencies: dict[str, list[str]],
) -> None:
    """Assert *order* is a valid topological order of *matched* under the deps.

    Used by the ``reload_order: "topological"`` contract. Kept as a helper so
    the fixture's dependency-free module set and the dependency-bearing case
    below are judged by the same rule.
    """
    assert sorted(order) == sorted(
        matched
    ), f"every matched module must be reloaded exactly once: got {order!r}, expected {matched!r}"
    for dependent, deps in declared_dependencies.items():
        for dep in deps:
            assert order.index(dep) < order.index(
                dependent
            ), f"{dependent} declares a dependency on {dep}, which must reload first; order was {order!r}"
    if not declared_dependencies:
        # With no edges, Kahn's sort over a deterministic (sorted) queue has
        # exactly one valid output.
        assert order == sorted(matched), f"dependency-free reload must be deterministic; got {order!r}"


#: ``reload_order`` value -> the checker that enforces it. Dispatching through
#: this map means an unknown contract value from the spec side raises KeyError
#: here instead of quietly asserting nothing.
_RELOAD_ORDER_CHECKERS = {"topological": _assert_topological_order}


def _dotted_to_nested(flat: dict[str, Any]) -> dict[str, Any]:
    """Expand {"executor.default_timeout": 30000} into nested YAML form."""
    nested: dict[str, Any] = {}
    for dotted, value in flat.items():
        cursor = nested
        parts = dotted.split(".")
        for part in parts[:-1]:
            cursor = cursor.setdefault(part, {})
        cursor[parts[-1]] = value
    return nested


def _make_config(**overrides: Any) -> Config:
    config = Config.from_defaults()
    for key, value in overrides.items():
        config.set(key, value)
    return config


def _make_update_config_module(
    config: Config,
    overrides_path: str | None = None,
    audit_store: InMemoryAuditStore | None = None,
) -> UpdateConfigModule:
    return UpdateConfigModule(
        config=config,
        event_emitter=EventEmitter(),
        overrides_path=overrides_path,
        audit_store=audit_store,
    )


def _make_toggle_module(
    registry: Registry,
    toggle_state: ToggleState | None = None,
    audit_store: InMemoryAuditStore | None = None,
) -> ToggleFeatureModule:
    return ToggleFeatureModule(
        registry=registry,
        event_emitter=EventEmitter(),
        toggle_state=toggle_state,
        audit_store=audit_store,
    )


def _make_reload_module(
    registry: Registry,
    audit_store: InMemoryAuditStore | None = None,
) -> ReloadModule:
    return ReloadModule(
        registry=registry,
        event_emitter=EventEmitter(),
        audit_store=audit_store,
    )


def _make_context(identity_id: str = "unknown", identity_type: str = "user") -> Any:
    """Create a minimal context object with identity."""
    context = MagicMock()
    context.trace_id = "test-trace-id"
    identity = MagicMock()
    identity.id = identity_id
    identity.type = identity_type
    context.identity = identity
    return context


# ---------------------------------------------------------------------------
# 1. overrides_persisted_on_update
# ---------------------------------------------------------------------------


class TestOverridesPersistOnUpdate:
    """Case: overrides_persisted_on_update"""

    def test_update_config_writes_overrides_file(self, tmp_path: Path) -> None:
        """update_config with overrides_path writes the change to the YAML file."""
        case = _case("overrides_persisted_on_update")
        expected = case["expected"]
        overrides_path = str(tmp_path / "overrides.yaml")
        config = _make_config()
        mod = _make_update_config_module(config, overrides_path=overrides_path)

        result = mod.execute(dict(case["action"]["input"]), _make_context())

        assert result["success"] is expected["call_success"]
        assert os.path.exists(overrides_path) is expected["overrides_file_written"]
        with open(overrides_path) as f:
            written = yaml.safe_load(f)
        for key, value in expected["overrides_file_contains"].items():
            assert written.get(key) == value, f"overrides file missing {key}={value!r}; got {written!r}"

    def test_overrides_file_accumulates_multiple_keys(self, tmp_path: Path) -> None:
        """Multiple update_config calls accumulate keys in the overrides file."""
        overrides_path = str(tmp_path / "overrides.yaml")
        config = _make_config()
        mod = _make_update_config_module(config, overrides_path=overrides_path)

        mod.execute(
            {"key": "executor.default_timeout", "value": 60000, "reason": "r1"},
            _make_context(),
        )
        mod.execute(
            {"key": "executor.max_workers", "value": 8, "reason": "r2"},
            _make_context(),
        )

        with open(overrides_path) as f:
            written = yaml.safe_load(f)
        assert written["executor.default_timeout"] == 60000
        assert written["executor.max_workers"] == 8

    def test_no_overrides_path_does_not_create_file(self, tmp_path: Path) -> None:
        """When overrides_path is None, no file is created."""
        config = _make_config()
        mod = _make_update_config_module(config, overrides_path=None)
        mod.execute(
            {"key": "executor.default_timeout", "value": 60000, "reason": "r"},
            _make_context(),
        )
        # No file created
        assert not any(tmp_path.iterdir())


# ---------------------------------------------------------------------------
# 2. overrides_loaded_on_startup
# ---------------------------------------------------------------------------


class TestOverridesLoadedOnStartup:
    """Case: overrides_loaded_on_startup"""

    def test_overrides_applied_after_base_config(self, tmp_path: Path) -> None:
        """When overrides.yaml exists at startup, its values override base config."""
        case = _case("overrides_loaded_on_startup")
        setup, expected = case["setup"], case["expected"]
        resolved = expected["resolved_value"]

        overrides_file = tmp_path / "overrides.yaml"
        overrides_file.write_text(yaml.safe_dump(setup["overrides_file_content"]))

        config = _make_config(**{})
        for key, value in setup["base_config"].items():
            config.set(key, value)
        config.set("sys_modules.enabled", True)
        config.set("sys_modules.control.overrides_path", str(overrides_file))

        assert config.get(resolved["key"]) == setup["base_config"][resolved["key"]]

        executor = MagicMock()
        registry = Registry()
        register_sys_modules(registry=registry, executor=executor, config=config)

        assert (
            config.get(resolved["key"]) == resolved["value"]
        ), f"{resolved['key']} must resolve to the override value after startup"

    def test_base_config_file_is_not_modified(self, tmp_path: Path) -> None:
        """`base_not_modified`: applying overrides MUST NOT write back to the base config.

        Corrected: this test used to load config from defaults and then assert
        that the *overrides* file still held the value it had just been written
        with — which is true no matter what the base config does, so it could
        not fail on the behaviour it claimed to cover. It now loads a real base
        config file and asserts that file is byte-identical afterwards.
        """
        case = _case("overrides_loaded_on_startup")
        setup, expected = case["setup"], case["expected"]
        resolved = expected["resolved_value"]

        base_file = tmp_path / "apcore.yaml"
        # `version` / `project.name` are required by the canonical config schema;
        # the fixture's base_config carries only the key under test.
        base_document = {"version": "1.0", "project": {"name": "conformance-base"}}
        base_document.update(_dotted_to_nested(setup["base_config"]))
        base_file.write_text(yaml.safe_dump(base_document))
        base_before = base_file.read_bytes()

        overrides_file = tmp_path / "overrides.yaml"
        overrides_file.write_text(yaml.safe_dump(setup["overrides_file_content"]))

        config = Config.load(str(base_file))
        config.set("sys_modules.enabled", True)
        config.set("sys_modules.control.overrides_path", str(overrides_file))
        assert config.get(resolved["key"]) == setup["base_config"][resolved["key"]]

        executor = MagicMock()
        registry = Registry()
        register_sys_modules(registry=registry, executor=executor, config=config)

        # In-memory value moved to the override...
        assert config.get(resolved["key"]) == resolved["value"]
        # ...while the base config on disk is untouched.
        base_not_modified = base_file.read_bytes() == base_before
        assert (
            base_not_modified is expected["base_not_modified"]
        ), f"base config file was rewritten during startup: {base_file.read_text()!r}"
        assert (
            yaml.safe_load(base_file.read_text())["executor"]["default_timeout"]
            == (setup["base_config"][resolved["key"]])
        )

    def test_missing_overrides_file_does_not_error(self, tmp_path: Path) -> None:
        """When the overrides file does not exist, startup continues without error."""
        nonexistent = str(tmp_path / "no_such_file.yaml")
        config = _make_config()
        config.set("sys_modules.enabled", True)
        config.set("sys_modules.control.overrides_path", nonexistent)

        executor = MagicMock()
        registry = Registry()
        # Should not raise
        register_sys_modules(registry=registry, executor=executor, config=config)


# ---------------------------------------------------------------------------
# 3. audit_entry_records_actor
# ---------------------------------------------------------------------------


class TestAuditEntryRecordsActor:
    """Case: audit_entry_records_actor"""

    def test_update_config_records_audit_entry_with_actor(self) -> None:
        """update_config call produces an audit entry with actor_id from context.identity."""
        case = _case("audit_entry_records_actor")
        action, expected = case["action"], case["expected"]
        identity = action["context_identity"]

        audit_store = InMemoryAuditStore()
        config = _make_config()
        mod = _make_update_config_module(config, audit_store=audit_store)
        context = _make_context(identity_id=identity["id"], identity_type=identity["type"])

        mod.execute(dict(action["input"]), context)

        entries = audit_store.query()
        assert len(entries) == expected["audit_entries_count"]
        entry = entries[0]
        audit_entry = expected["audit_entry"]
        assert entry.action == audit_entry["action"]
        assert entry.target_module_id == audit_entry["target_module_id"]
        assert entry.actor_id == audit_entry["actor_id"]
        assert entry.actor_type == audit_entry["actor_type"]

    def test_audit_entry_has_timestamp(self) -> None:
        """Audit entry timestamp is an ISO 8601 string."""
        case = _case("audit_entry_records_actor")
        expected = case["expected"]
        audit_store = InMemoryAuditStore()
        config = _make_config()
        mod = _make_update_config_module(config, audit_store=audit_store)

        mod.execute(dict(case["action"]["input"]), _make_context())

        entry = audit_store.query()[0]
        timestamp_present = bool(entry.timestamp) and "T" in entry.timestamp  # ISO 8601
        assert (
            timestamp_present is expected["timestamp_present"]
        ), f"audit entry timestamp {entry.timestamp!r} is not a present ISO 8601 value"

    def test_audit_entry_has_trace_id(self) -> None:
        """Audit entry contains the trace_id from context."""
        case = _case("audit_entry_records_actor")
        expected = case["expected"]
        audit_store = InMemoryAuditStore()
        config = _make_config()
        mod = _make_update_config_module(config, audit_store=audit_store)
        context = _make_context()
        context.trace_id = "abc123"

        mod.execute(dict(case["action"]["input"]), context)

        entry = audit_store.query()[0]
        assert (entry.trace_id == "abc123") is expected["trace_id_present"]


# ---------------------------------------------------------------------------
# 4. audit_entry_records_change
# ---------------------------------------------------------------------------


class TestAuditEntryRecordsChange:
    """Case: audit_entry_records_change"""

    def test_toggle_feature_records_before_after_change(self) -> None:
        """toggle_feature produces an audit entry with before/after change values."""
        case = _case("audit_entry_records_change")
        setup, action, expected = case["setup"], case["action"], case["expected"]
        identity = action["context_identity"]
        initial = setup["initial_module_state"]

        audit_store = InMemoryAuditStore()
        registry = Registry()
        toggle_state = ToggleState()
        if not initial["enabled"]:
            toggle_state.disable(initial["module_id"])

        # Register a dummy module so toggle can find it
        dummy = MagicMock()
        dummy.input_schema = {"type": "object", "properties": {}}
        dummy.output_schema = {"type": "object", "properties": {}}
        registry.register_internal(initial["module_id"], dummy)

        mod = _make_toggle_module(registry, toggle_state=toggle_state, audit_store=audit_store)
        context = _make_context(identity_id=identity["id"], identity_type=identity["type"])

        mod.execute(dict(action["input"]), context)

        entries = audit_store.query()
        assert len(entries) == expected["audit_entries_count"]
        entry = entries[0]
        audit_entry = expected["audit_entry"]
        assert entry.action == audit_entry["action"]
        assert entry.target_module_id == audit_entry["target_module_id"]
        assert entry.actor_id == audit_entry["actor_id"]
        assert entry.actor_type == audit_entry["actor_type"]
        assert entry.change["before"] is audit_entry["change"]["before"]
        assert entry.change["after"] is audit_entry["change"]["after"]

    def test_toggle_feature_enable_records_before_false(self) -> None:
        """Enabling a previously disabled module records before=False, after=True."""
        audit_store = InMemoryAuditStore()
        registry = Registry()
        toggle_state = ToggleState()
        toggle_state.disable("risky.module")

        dummy = MagicMock()
        dummy.input_schema = {"type": "object", "properties": {}}
        dummy.output_schema = {"type": "object", "properties": {}}
        registry.register_internal("risky.module", dummy)

        mod = _make_toggle_module(registry, toggle_state=toggle_state, audit_store=audit_store)

        mod.execute(
            {"module_id": "risky.module", "enabled": True, "reason": "re-enable"},
            _make_context(),
        )

        entry = audit_store.query()[0]
        assert entry.change["before"] is False
        assert entry.change["after"] is True


# ---------------------------------------------------------------------------
# 5. prometheus_usage_exports_calls_total
# ---------------------------------------------------------------------------


def _collector_from_usage_setup(setup: dict[str, Any]) -> UsageCollector:
    """Replay the fixture's ``usage_collector_data`` into a real UsageCollector."""
    collector = UsageCollector()
    for entry in setup["usage_collector_data"]:
        breakdown = entry["status_breakdown"]
        for _ in range(breakdown["success"]):
            collector.record(entry["module_id"], "caller", 10.0, success=True)
        for _ in range(breakdown["error"]):
            collector.record(entry["module_id"], "caller", 45.0, success=False)
    return collector


class TestPrometheusUsageExportsCallsTotal:
    """Case: prometheus_usage_exports_calls_total"""

    @pytest.mark.parametrize(
        "metric_line",
        _case("prometheus_usage_exports_calls_total")["expected"]["metrics_endpoint_contains"],
    )
    def test_metrics_endpoint_contains(self, metric_line: str) -> None:
        """Every metric line the fixture names is present in export_prometheus().

        Parametrized off ``metrics_endpoint_contains`` rather than transcribed:
        a metric added on the spec side then fails here instead of silently
        going unchecked.
        """
        case = _case("prometheus_usage_exports_calls_total")
        collector = _collector_from_usage_setup(case["setup"])

        output = collector.export_prometheus()
        assert metric_line in output, f"/metrics output is missing {metric_line!r}"

    def test_export_completes_within_timeout(self) -> None:
        """export_prometheus completes within the fixture's export_timeout_ms budget."""
        case = _case("prometheus_usage_exports_calls_total")
        expected = case["expected"]
        collector = _collector_from_usage_setup(case["setup"])

        start = time.monotonic()
        collector.export_prometheus()
        elapsed_ms = (time.monotonic() - start) * 1000.0
        assert (
            elapsed_ms < expected["export_within_timeout_ms"]
        ), f"export_prometheus took {elapsed_ms:.1f}ms, budget is {expected['export_within_timeout_ms']}ms"


# ---------------------------------------------------------------------------
# 6. reload_with_path_filter
# ---------------------------------------------------------------------------


class TestReloadWithPathFilter:
    """Case: reload_with_path_filter"""

    def _make_registry_with_modules(
        self, module_ids: list[str], dependencies: dict[str, list[str]] | None = None
    ) -> Registry:
        """Create a registry with dummy modules registered."""
        dependencies = dependencies or {}
        registry = Registry()
        for mid in module_ids:
            dummy = MagicMock()
            dummy.input_schema = {"type": "object", "properties": {}}
            dummy.output_schema = {"type": "object", "properties": {}}
            dummy.version = "1.0.0"
            deps = dependencies.get(mid)
            if deps:
                registry.register(
                    mid,
                    dummy,
                    metadata={"dependencies": [{"module_id": dep} for dep in deps]},
                )
            else:
                registry.register_internal(mid, dummy)
        return registry

    def test_path_filter_reloads_matching_modules(self) -> None:
        """path_filter 'executor.*' reloads all matching modules."""
        case = _case("reload_with_path_filter")
        expected = case["expected"]
        registry = self._make_registry_with_modules(case["setup"]["registered_modules"])
        mod = _make_reload_module(registry)

        with patch.object(mod, "_reload_one"):
            result = mod.execute(dict(case["action"]["input"]), _make_context())

        assert result["success"] is expected["call_success"]
        reloaded = result["reloaded_modules"]
        assert sorted(reloaded) == sorted(expected["reloaded_modules"])
        for module_id in expected["not_reloaded"]:
            assert module_id not in reloaded, f"{module_id} must not match {case['action']['input']['path_filter']!r}"

    def test_path_filter_excludes_non_matching(self) -> None:
        """Modules outside the path_filter are not reloaded."""
        case = _case("reload_with_path_filter")
        expected = case["expected"]
        registry = self._make_registry_with_modules(case["setup"]["registered_modules"])
        mod = _make_reload_module(registry)

        with patch.object(mod, "_reload_one"):
            result = mod.execute(
                {"path_filter": case["action"]["input"]["path_filter"], "reason": "deploy"},
                _make_context(),
            )

        for module_id in expected["not_reloaded"]:
            assert module_id not in result["reloaded_modules"]

    def test_path_filter_reload_order_is_topological(self) -> None:
        """`reload_order: "topological"` — reload order is a valid topological order.

        The fixture's module set declares no dependencies, so the only order it
        can pin is the deterministic one (alphabetical, which Kahn's sort emits
        for a dependency-free set). The dependency-bearing half of this contract
        is now a canonical case of its own —
        ``reload_order_is_topological_not_alphabetical``, driven by
        :class:`TestReloadOrderIsTopologicalNotAlphabetical` below.
        """
        case = _case("reload_with_path_filter")
        expected = case["expected"]
        matched = expected["reloaded_modules"]
        registry = self._make_registry_with_modules(case["setup"]["registered_modules"])
        mod = _make_reload_module(registry)

        reload_order: list[str] = []

        # side_effect must tolerate the method's full signature, not just its
        # first argument — see the note on the dependency-bearing case below.
        with patch.object(mod, "_reload_one", side_effect=lambda mid, *_a, **_kw: reload_order.append(mid)):
            mod.execute(dict(case["action"]["input"]), _make_context())

        # The contract value is the dispatch key; the checker takes only the
        # observation. Passing it again as a fourth argument was a slip.
        _RELOAD_ORDER_CHECKERS[expected["reload_order"]](reload_order, matched, {})

    # Was a strict xfail: `merge_module_metadata` omitted `dependencies` from
    # the stored metadata, so `_topo_sort_modules` topologically sorted an
    # always-empty graph and bulk reload degenerated to alphabetical. The
    # merge now carries it; the marker turned into an XPASS the moment it did,
    # which is how a strict xfail announces its own removal.
    def test_declared_dependency_reloads_before_its_dependent(self) -> None:
        """A declared dependency MUST be reloaded before the module that needs it."""
        modules = ["executor.alpha", "executor.zulu"]
        declared = {"executor.alpha": ["executor.zulu"]}
        registry = self._make_registry_with_modules(modules, dependencies=declared)
        mod = _make_reload_module(registry)

        reload_order: list[str] = []
        # `_reload_one(module_id, context)` — record only the id. `side_effect`
        # must accept every argument the real method takes, so a bare
        # `reload_order.append` breaks the moment the signature grows (it did,
        # when `context` was threaded through for A-D-017) and the resulting
        # TypeError is swallowed upstream, leaving `order` silently empty.
        with patch.object(mod, "_reload_one", side_effect=lambda mid, *_a, **_kw: reload_order.append(mid)):
            mod.execute({"path_filter": "executor.*", "reason": "topo test"}, _make_context())

        _assert_topological_order(reload_order, modules, declared_dependencies=declared)


# ---------------------------------------------------------------------------
# Supplemental: reload_module audit entry (not a fixture case, covers §1.2)
# ---------------------------------------------------------------------------


class TestReloadModuleAuditEntry:
    """Supplemental: reload_module records an audit entry with actor from context.identity."""

    def test_single_reload_records_audit_entry(self) -> None:
        """A single-module reload produces an audit entry with version change info."""
        audit_store = InMemoryAuditStore()
        registry = Registry()
        dummy = MagicMock()
        dummy.input_schema = {"type": "object", "properties": {}}
        dummy.output_schema = {"type": "object", "properties": {}}
        dummy.version = "1.0.0"
        registry.register_internal("math.add", dummy)

        mod = _make_reload_module(registry, audit_store=audit_store)
        context = _make_context(identity_id="ops-user", identity_type="user")

        new_dummy = MagicMock()
        new_dummy.version = "1.1.0"

        with (
            patch.object(mod, "_rediscover_module", return_value=new_dummy),
            patch.object(mod, "_reregister_module"),
        ):
            mod.execute(
                {"module_id": "math.add", "reason": "hotfix deploy"},
                context,
            )

        entries = audit_store.query()
        assert len(entries) == 1
        entry = entries[0]
        assert entry.action == "reload_module"
        assert entry.target_module_id == "math.add"
        assert entry.actor_id == "ops-user"
        assert entry.actor_type == "user"
        assert entry.change["before"] == "1.0.0"
        assert entry.change["after"] == "1.1.0"

    def test_bulk_reload_records_one_audit_entry_per_module(self) -> None:
        """D-111: one entry PER MODULE, sharing one correlation id.

        This test used to assert ``len(entries) == 1`` — it pinned the aggregate
        entry the decision forbids, whose ``target_module_id`` was the glob. An
        entry keyed on ``executor.*`` is unfindable by
        ``AuditStore.query(module_id=...)``, which filters on a concrete id: the
        operation was audited and could not be looked up. A test asserting the
        old shape is how that survived, the same way apcore-rust's unit test
        kept D-106's wrong p99 green.
        """
        audit_store = InMemoryAuditStore()
        registry = Registry()
        for mid in ["executor.a", "executor.b"]:
            dummy = MagicMock()
            dummy.input_schema = {"type": "object", "properties": {}}
            dummy.output_schema = {"type": "object", "properties": {}}
            registry.register_internal(mid, dummy)

        mod = _make_reload_module(registry, audit_store=audit_store)

        with patch.object(mod, "_reload_one"):
            mod.execute(
                {"path_filter": "executor.*", "reason": "bulk deploy"},
                _make_context(identity_id="ci-agent", identity_type="service"),
            )

        entries = audit_store.query()
        assert len(entries) == 2
        assert {e.target_module_id for e in entries} == {"executor.a", "executor.b"}
        assert all(e.action == "reload_module" for e in entries)
        assert all(e.actor_id == "ci-agent" for e in entries)

        # Each entry is reachable by the accessor the store exists for.
        for mid in ("executor.a", "executor.b"):
            found = audit_store.query(module_id=mid)
            assert [e.target_module_id for e in found] == [mid]

        # Per-module entries alone lose the fact that they were ONE deploy.
        correlation_ids = {e.correlation_id for e in entries}
        assert len(correlation_ids) == 1
        assert correlation_ids != {""}

    def test_a_single_module_reload_needs_no_correlation_id(self) -> None:
        """Control: the id groups a MULTI-target operation, so a single reload
        leaves it empty rather than minting one nothing else shares."""
        audit_store = InMemoryAuditStore()
        registry = Registry()
        dummy = MagicMock()
        dummy.input_schema = {"type": "object", "properties": {}}
        dummy.output_schema = {"type": "object", "properties": {}}
        registry.register_internal("executor.only", dummy)

        mod = _make_reload_module(registry, audit_store=audit_store)
        with (
            patch.object(mod, "_rediscover_module", return_value=MagicMock()),
            patch.object(mod, "_reregister_module"),
        ):
            mod.execute(
                {"module_id": "executor.only", "reason": "single deploy"},
                _make_context(identity_id="ci-agent", identity_type="service"),
            )

        entries = audit_store.query()
        assert len(entries) == 1
        assert entries[0].correlation_id == ""

    def test_two_bulk_reloads_get_different_correlation_ids(self) -> None:
        """Control: the id must group ONE operation, not all of them.

        A constant — or an id minted once per process — would satisfy the
        sharing assertion above while making "what did this deploy touch"
        return every deploy.
        """
        audit_store = InMemoryAuditStore()
        registry = Registry()
        for mid in ["executor.a", "executor.b"]:
            dummy = MagicMock()
            dummy.input_schema = {"type": "object", "properties": {}}
            dummy.output_schema = {"type": "object", "properties": {}}
            registry.register_internal(mid, dummy)

        mod = _make_reload_module(registry, audit_store=audit_store)
        ctx = _make_context(identity_id="ci-agent", identity_type="service")
        with patch.object(mod, "_reload_one"):
            mod.execute({"path_filter": "executor.*", "reason": "deploy 1"}, ctx)
            mod.execute({"path_filter": "executor.*", "reason": "deploy 2"}, ctx)

        ids = {e.correlation_id for e in audit_store.query()}
        assert len(ids) == 2, ids


# ---------------------------------------------------------------------------
# 7. reload_module_id_and_filter_conflict
# ---------------------------------------------------------------------------


class TestReloadModuleIdAndFilterConflict:
    """Case: reload_module_id_and_filter_conflict"""

    def test_both_module_id_and_path_filter_raises(self) -> None:
        """Providing both module_id and path_filter raises MODULE_RELOAD_CONFLICT."""
        case = _case("reload_module_id_and_filter_conflict")
        expected = case["expected"]
        registry = Registry()
        mod = _make_reload_module(registry)

        call_success = False
        with pytest.raises(ModuleReloadConflictError) as exc_info:
            mod.execute(dict(case["action"]["input"]), _make_context())
            call_success = True

        assert call_success is expected["call_success"]
        error = exc_info.value
        assert error.code == expected["error_code"]
        assert expected["error_message_contains"] in error.message.lower()


# ---------------------------------------------------------------------------
# 8. startup_fail_on_error_true_raises
# ---------------------------------------------------------------------------


class TestStartupFailOnErrorTrueRaises:
    """Case: startup_fail_on_error_true_raises"""

    def test_fail_on_error_true_raises_immediately(self) -> None:
        """When fail_on_error=True and a module fails, register_sys_modules raises."""
        case = _case("startup_fail_on_error_true_raises")
        failure, expected = case["setup"]["simulated_failure"], case["expected"]
        config = _make_config()
        config.set("sys_modules.enabled", True)
        executor = MagicMock()
        registry = Registry()

        original_register = registry.register_internal

        def failing_register(module_id: str, module: Any) -> None:
            if module_id == failure["module_id"]:
                raise ValueError(failure["error"])
            original_register(module_id, module)

        raised: SysModuleRegistrationError | None = None
        with patch.object(registry, "register_internal", side_effect=failing_register):
            try:
                register_sys_modules(
                    registry=registry,
                    executor=executor,
                    config=config,
                    fail_on_error=case["action"]["params"]["fail_on_error"],
                )
            except SysModuleRegistrationError as exc:
                raised = exc

        assert (raised is not None) is expected["raises"]
        assert raised is not None
        assert raised.code == expected["error_code"]
        assert expected["error_includes_module_id"] in raised.message

    def test_fail_on_error_true_error_includes_module_id(self) -> None:
        """SysModuleRegistrationError includes the failed module's ID."""
        case = _case("startup_fail_on_error_true_raises")
        failure, expected = case["setup"]["simulated_failure"], case["expected"]
        config = _make_config()
        config.set("sys_modules.enabled", True)
        executor = MagicMock()
        registry = Registry()

        original_register = registry.register_internal

        def failing_register(module_id: str, module: Any) -> None:
            if module_id == failure["module_id"]:
                raise RuntimeError("registration failed")
            original_register(module_id, module)

        with patch.object(registry, "register_internal", side_effect=failing_register):
            with pytest.raises(SysModuleRegistrationError) as exc_info:
                register_sys_modules(
                    registry=registry,
                    executor=executor,
                    config=config,
                    fail_on_error=case["action"]["params"]["fail_on_error"],
                )

        assert exc_info.value.details.get("module_id", "") == expected["error_includes_module_id"]


# ---------------------------------------------------------------------------
# 9. startup_fail_on_error_false_continues
# ---------------------------------------------------------------------------


class TestStartupFailOnErrorFalseContinues:
    """Case: startup_fail_on_error_false_continues"""

    def test_fail_on_error_false_does_not_raise(self, caplog: Any) -> None:
        """When fail_on_error=False and a module fails, register_sys_modules does not raise."""
        case = _case("startup_fail_on_error_false_continues")
        failure, expected = case["setup"]["simulated_failure"], case["expected"]
        config = _make_config()
        config.set("sys_modules.enabled", True)
        executor = MagicMock()
        registry = Registry()

        original_register = registry.register_internal

        def failing_register(module_id: str, module: Any) -> None:
            if module_id == failure["module_id"]:
                raise ValueError(failure["error"])
            original_register(module_id, module)

        raised = False
        with patch.object(registry, "register_internal", side_effect=failing_register):
            with caplog.at_level(logging.ERROR):
                try:
                    result = register_sys_modules(
                        registry=registry,
                        executor=executor,
                        config=config,
                        fail_on_error=case["action"]["params"]["fail_on_error"],
                    )
                except SysModuleRegistrationError:
                    raised = True

        assert raised is expected["raises"]
        # Should return normally
        assert isinstance(result, dict)

    def test_fail_on_error_false_logs_at_error_level(self, caplog: Any) -> None:
        """When fail_on_error=False, failure is logged at the fixture's level."""
        case = _case("startup_fail_on_error_false_continues")
        failure, expected = case["setup"]["simulated_failure"], case["expected"]
        config = _make_config()
        config.set("sys_modules.enabled", True)
        executor = MagicMock()
        registry = Registry()

        original_register = registry.register_internal

        def failing_register(module_id: str, module: Any) -> None:
            if module_id == failure["module_id"]:
                raise ValueError(failure["error"])
            original_register(module_id, module)

        level = getattr(logging, expected["log_level_on_failure"])
        with patch.object(registry, "register_internal", side_effect=failing_register):
            with caplog.at_level(level, logger="apcore.sys_modules.registration"):
                register_sys_modules(
                    registry=registry,
                    executor=executor,
                    config=config,
                    fail_on_error=case["action"]["params"]["fail_on_error"],
                )

        at_level = [r.message for r in caplog.records if r.levelno == level]
        assert any(failure["module_id"] in m for m in at_level), (
            f"no {expected['log_level_on_failure']} record naming {failure['module_id']!r}; "
            f"saw {[(r.levelname, r.message) for r in caplog.records]!r}"
        )

    def test_fail_on_error_false_remaining_modules_registered(self) -> None:
        """When fail_on_error=False, modules after the failing one are still registered."""
        case = _case("startup_fail_on_error_false_continues")
        failure, expected = case["setup"]["simulated_failure"], case["expected"]
        config = _make_config()
        config.set("sys_modules.enabled", True)
        executor = MagicMock()
        registry = Registry()

        original_register = registry.register_internal
        registered: list[str] = []

        def tracking_register(module_id: str, module: Any) -> None:
            if module_id == failure["module_id"]:
                raise ValueError(failure["error"])
            original_register(module_id, module)
            registered.append(module_id)

        with patch.object(registry, "register_internal", side_effect=tracking_register):
            register_sys_modules(
                registry=registry,
                executor=executor,
                config=config,
                fail_on_error=case["action"]["params"]["fail_on_error"],
            )

        # Other modules are still registered despite the failure, and the failed
        # one is not.
        remaining_modules_registered = bool(registered) and failure["module_id"] not in registered
        assert (
            remaining_modules_registered is expected["remaining_modules_registered"]
        ), f"modules registered after the simulated failure: {registered!r}"


# ---------------------------------------------------------------------------
# 10. rust_register_returns_result
# ---------------------------------------------------------------------------


@pytest.mark.skip(reason="language=rust — not applicable to Python implementation")
class TestRustRegisterReturnsResult:
    """Case: rust_register_returns_result — Rust-specific, skipped for Python.

    The case carries ``"language": "rust"`` and its ``expected`` keys —
    ``return_type``, ``ok_variant``, ``err_variant``, ``panics``,
    ``returns_option`` — describe the shape of a Rust ``Result<(),
    SysModuleError>``. They belong to the apcore-rust driver; Python has no
    equivalent to assert and MUST NOT invent one.
    """

    def test_rust_returns_result_type(self) -> None:
        """Rust register_sys_modules returns Result<(), SysModuleError>."""
        pass


# ---------------------------------------------------------------------------
# 11. reload_order_is_topological_not_alphabetical
# ---------------------------------------------------------------------------


class TestReloadOrderIsTopologicalNotAlphabetical:
    """Case: reload_order_is_topological_not_alphabetical

    The discriminating half of ``reload_order: "topological"``. Everything the
    case needs — the module ids, the edge between them, and the order the two
    disagree on — comes out of the fixture; the driver hand-writes none of it.

    Two rules from the fixture's ``driver_contract`` shape the assertions:

    ``ordering_needs_a_disagreeing_graph``
        The observation is the sequence of registry mutations the reload
        actually performed (``safe_unregister`` / ``register_internal``), not
        ``result["reloaded_modules"]``. That field is built by appending inside
        the same loop, so reading it back would assert a relabelling of the
        report rather than the order the work happened in.

    ``dependencies_must_survive_registration``
        The edge is declared through :meth:`Registry.register` — the path an
        application uses — and read back through
        :meth:`Registry.get_module_metadata`, the post-registration accessor
        ``ReloadModule._topo_sort_modules`` itself reads. Handing the graph
        straight to the sort would pass against the merge bug this case exists
        to catch (``merge_module_metadata`` used to drop ``dependencies``,
        leaving discovery-time ordering working and reload ordering degenerate).
    """

    CASE = "reload_order_is_topological_not_alphabetical"

    @staticmethod
    def _dummy() -> Any:
        dummy = MagicMock()
        dummy.input_schema = {"type": "object", "properties": {}}
        dummy.output_schema = {"type": "object", "properties": {}}
        dummy.version = "1.0.0"
        dummy.dependencies = []
        return dummy

    def _register_from_setup(self) -> tuple[Registry, dict[str, Any]]:
        """Register the fixture's modules through the public application path."""
        setup = _case(self.CASE)["setup"]
        declared: dict[str, list[str]] = setup.get("declared_dependencies", {})
        registry = Registry()
        modules: dict[str, Any] = {}
        for module_id in setup["registered_modules"]:
            module = self._dummy()
            modules[module_id] = module
            deps = declared.get(module_id, [])
            registry.register(
                module_id,
                module,
                metadata={"dependencies": [{"module_id": dep} for dep in deps]} if deps else None,
            )
        return registry, modules

    def test_declared_dependencies_survive_registration(self) -> None:
        """`declared_dependencies` must be readable back off the registry.

        The accessor, not the input: an implementation that drops the edge
        during the metadata merge still sorts correctly at discovery time, so
        only the post-registration view can tell the two apart.
        """
        from apcore.registry.metadata import parse_dependencies

        setup = _case(self.CASE)["setup"]
        declared: dict[str, list[str]] = setup["declared_dependencies"]
        registry, _ = self._register_from_setup()

        for module_id, deps in declared.items():
            stored = registry.get_module_metadata(module_id).get("dependencies", [])
            assert [d.module_id for d in parse_dependencies(stored)] == deps, (
                f"{module_id} declared dependencies {deps} through Registry.register, but "
                f"get_module_metadata() reports {stored!r} — the merge dropped them"
            )

    def test_the_two_candidate_orders_really_disagree(self) -> None:
        """`alphabetical_order_would_be` / `orders_differ` — the case discriminates.

        A graph whose topological order coincides with the alphabet proves
        nothing, which is the state ``reload_with_path_filter`` is in. Pin the
        fixture's claim against a real sort so that editing the module ids into
        agreement fails here instead of quietly disarming the case below.
        """
        case = _case(self.CASE)
        expected = case["expected"]
        alphabetical = expected["alphabetical_order_would_be"]

        assert (
            sorted(case["setup"]["registered_modules"]) == alphabetical
        ), "alphabetical_order_would_be must be the plain sort of the registered modules"
        assert (expected["reload_order_observed"] != alphabetical) is expected[
            "orders_differ"
        ], "orders_differ must describe whether the topological and alphabetical orders disagree"

    def test_reload_order_observed_is_topological(self, monkeypatch: Any) -> None:
        """`reload_order_observed` — the registry mutations, in order.

        Reverting ``_topo_sort_modules`` to ``sorted(module_ids)`` reverses both
        recorded sequences and fails on the first assertion below.
        """
        case = _case(self.CASE)
        expected = case["expected"]
        registry, modules = self._register_from_setup()
        mod = _make_reload_module(registry)

        unregistered: list[str] = []
        reregistered: list[str] = []
        real_safe_unregister = registry.safe_unregister
        real_register_internal = registry.register_internal

        def recording_safe_unregister(module_id: str, *args: Any, **kwargs: Any) -> Any:
            unregistered.append(module_id)
            return real_safe_unregister(module_id, *args, **kwargs)

        def recording_register_internal(module_id: str, module: Any, *args: Any, **kwargs: Any) -> Any:
            reregistered.append(module_id)
            return real_register_internal(module_id, module, *args, **kwargs)

        monkeypatch.setattr(registry, "safe_unregister", recording_safe_unregister)
        monkeypatch.setattr(registry, "register_internal", recording_register_internal)

        # Re-discovery of an unchanged source tree hands back the same module.
        # Only rediscovery is stubbed — the unregister and re-register both run
        # against the real registry, which is what is being observed.
        with patch.object(mod, "_rediscover_module", side_effect=lambda mid: modules[mid]):
            result = mod.execute(dict(case["action"]["input"]), _make_context())

        assert result["success"] is expected["call_success"]
        observed = expected["reload_order_observed"]
        assert unregistered == observed, (
            f"modules were unregistered in {unregistered!r}; the declared dependency graph " f"requires {observed!r}"
        )
        assert reregistered == observed, f"modules were re-registered in {reregistered!r}, expected {observed!r}"
        if expected["orders_differ"]:
            assert (
                unregistered != expected["alphabetical_order_would_be"]
            ), "reload order collapsed to the alphabetical order — the dependency graph was ignored"


# ---------------------------------------------------------------------------
# Fixture coverage guard
# ---------------------------------------------------------------------------


class TestFixtureCoverage:
    """Every case in the canonical fixture has a driver class in this file.

    The assertions above are hand-written rather than generated from the fixture
    (each case needs its own registry / config / filesystem wiring). That is
    fine, but it used to mean the fixture was named only in the module docstring
    while a vendored copy sat unread in ``tests/conformance/fixtures/`` — so a
    case added on the spec side left no trace here at all. This guard closes
    that gap: a new canonical case fails until someone writes the class for it.
    """

    FIXTURE = "system_modules_hardening.json"

    #: canonical case id → the class in this module that asserts it.
    COVERED: dict[str, str] = {
        "overrides_persisted_on_update": "TestOverridesPersistOnUpdate",
        "overrides_loaded_on_startup": "TestOverridesLoadedOnStartup",
        "audit_entry_records_actor": "TestAuditEntryRecordsActor",
        "audit_entry_records_change": "TestAuditEntryRecordsChange",
        "prometheus_usage_exports_calls_total": "TestPrometheusUsageExportsCallsTotal",
        "reload_with_path_filter": "TestReloadWithPathFilter",
        "reload_module_id_and_filter_conflict": "TestReloadModuleIdAndFilterConflict",
        "startup_fail_on_error_true_raises": "TestStartupFailOnErrorTrueRaises",
        "startup_fail_on_error_false_continues": "TestStartupFailOnErrorFalseContinues",
        # language=rust — asserted here only as a documented cross-language note.
        "rust_register_returns_result": "TestRustRegisterReturnsResult",
        "reload_order_is_topological_not_alphabetical": ("TestReloadOrderIsTopologicalNotAlphabetical"),
        "manifest_full_project_name_defaults_to_apcore": ("TestProjectNameAndOpenWorldDeclarations"),
        "health_summary_project_name_agrees_with_manifest_full": ("TestProjectNameAndOpenWorldDeclarations"),
        "system_modules_declare_open_world_false": ("TestProjectNameAndOpenWorldDeclarations"),
        "error_rate_threshold_moves_only_the_healthy_boundary": ("TestErrorRateThresholdMovesOneBoundary"),
        "the_error_boundary_stays_at_the_table_value": ("TestErrorRateThresholdMovesOneBoundary"),
    }

    def test_every_canonical_case_is_claimed(self) -> None:
        canonical = set(case_ids(self.FIXTURE))
        claimed = set(self.COVERED)
        assert canonical - claimed == set(), f"canonical fixture {self.FIXTURE} gained case(s) with no driver here"
        assert claimed - canonical == set(), f"this file claims case(s) {self.FIXTURE} no longer defines"

    def test_every_claimed_class_exists(self) -> None:
        module = sys.modules[__name__]
        missing = [cls for cls in self.COVERED.values() if not hasattr(module, cls)]
        assert missing == [], f"claimed driver class(es) not defined: {missing}"


class TestErrorRateThresholdMovesOneBoundary:
    """D-109: `error_rate_threshold` moves the HEALTHY/DEGRADED boundary only.

    The degraded/error boundary is the classification table's fixed 0.10. This
    SDK and apcore-typescript computed it as ``threshold * 10``, so a module
    erroring 5% of the time was ``error`` here and ``degraded`` on apcore-rust —
    same module, same metrics, same configuration.
    """

    @staticmethod
    def _status_for(case: dict[str, Any]) -> str:
        observed = case["action"]["observed"]
        module_id = observed["module_id"]

        registry = Registry()
        dummy = MagicMock()
        dummy.input_schema = {"type": "object", "properties": {}}
        dummy.output_schema = {"type": "object", "properties": {}}
        registry.register_internal(module_id, dummy)

        from apcore.observability.error_history import ErrorHistory

        metrics = MetricsCollector()
        for i in range(observed["total_calls"]):
            metrics.increment_calls(module_id, "error" if i < observed["error_calls"] else "success")

        summary = HealthSummaryModule(registry, metrics, ErrorHistory(), Config({})).execute(
            dict(case["action"]["input"]), _make_context()
        )
        entry = next(m for m in summary["modules"] if m["module_id"] == module_id)
        return str(entry["status"])

    def test_a_rate_between_the_two_boundaries_is_degraded(self) -> None:
        case = _CASES["error_rate_threshold_moves_only_the_healthy_boundary"]
        assert self._status_for(case) == case["expected"]["module_status"]

    def test_the_error_boundary_stays_at_the_table_value(self) -> None:
        # Without this, "not scaled" is also satisfied by an implementation that
        # removed the error boundary and called everything above healthy
        # `degraded`.
        case = _CASES["the_error_boundary_stays_at_the_table_value"]
        assert self._status_for(case) == case["expected"]["module_status"]


class TestBulkReloadCarriesCallerIdentity:
    """A bulk reload's `apcore.module.reloaded` must name the real caller.

    Sync finding A-D-017. `_execute_bulk` passes `context` into the AuditStore
    entry but `_reload_one` omitted it at the `_emit_module_reloaded` call, so
    `_audit_payload_extras(None)` fell back to `caller_id="@external"`. Two
    records of the same action — the audit entry and the event — disagreed
    about who performed it, and only the event was wrong.
    """

    def test_bulk_reload_event_carries_the_real_caller(self) -> None:
        from apcore.events import EventEmitter

        events: list[Any] = []

        class Recorder:
            subscriber_id = "rec"
            event_pattern = "apcore.module.*"

            async def on_event(self, event: Any) -> None:
                events.append(event)

        registry = self._registry_with(["executor.alpha"])
        emitter = EventEmitter()
        emitter.subscribe(Recorder())
        mod = ReloadModule(registry=registry, event_emitter=emitter)

        with patch.object(mod, "_rediscover_module", side_effect=lambda mid: self._fresh_module()):
            mod.execute({"path_filter": "executor.*", "reason": "deploy"}, _make_context())
        emitter.flush(2.0)

        reloaded = [e for e in events if e.event_type == "apcore.module.reloaded"]
        assert reloaded, f"a bulk reload must emit the event; got {[e.event_type for e in events]}"
        caller = reloaded[0].data.get("caller_id")
        assert caller != "@external", (
            "an authenticated bulk reload must not be attributed to @external — " f"got {caller!r}"
        )

    @staticmethod
    def _registry_with(module_ids: list[str]) -> Any:
        registry = Registry()
        for mid in module_ids:
            dummy = MagicMock()
            dummy.input_schema = {"type": "object", "properties": {}}
            dummy.output_schema = {"type": "object", "properties": {}}
            dummy.version = "1.0.0"
            registry.register(mid, dummy)
        return registry

    @staticmethod
    def _fresh_module() -> Any:
        m = MagicMock()
        m.input_schema = {"type": "object", "properties": {}}
        m.output_schema = {"type": "object", "properties": {}}
        m.version = "1.0.0"
        return m


class TestProjectNameAndOpenWorldDeclarations:
    """D-110 and D-119, driven from `system_modules_hardening.json`.

    Both are v1.51.0 policy decisions: the spec was silent and each SDK had
    answered reasonably. Neither is a bug report against any one implementation,
    which is why the cases are written from the fixture rather than from what any
    SDK happens to do.
    """

    def test_manifest_full_project_name_defaults_to_apcore(self) -> None:
        case = _CASES["manifest_full_project_name_defaults_to_apcore"]
        module = ManifestFullModule(Registry(), Config({}))

        result = module.execute(dict(case["action"]["input"]), _make_context())

        assert result["project_name"] == case["expected"]["project_name"], (
            f"[{case['id']}] project_name: got {result['project_name']!r}, "
            f"expected {case['expected']['project_name']!r}"
        )

    def test_health_summary_project_name_agrees_with_manifest_full(self) -> None:
        # The pairing is the decision. `manifest.full` alone could be satisfied
        # while the two system modules still disagreed, which is the state D-110
        # exists to remove.
        case = _CASES["health_summary_project_name_agrees_with_manifest_full"]
        registry, config = Registry(), Config({})

        summary = HealthSummaryModule(registry, config, MetricsCollector()).execute({}, _make_context())
        manifest = ManifestFullModule(registry, config).execute({}, _make_context())

        assert summary["project"]["name"] == case["expected"]["project_name"]
        assert summary["project"]["name"] == manifest["project_name"], (
            f"[{case['id']}] health.summary and manifest.full disagree: "
            f"{summary['project']['name']!r} vs {manifest['project_name']!r}"
        )

    def test_system_modules_declare_open_world_false(self) -> None:
        case = _CASES["system_modules_declare_open_world_false"]
        client = APCore(
            config=Config(
                {
                    "version": "1.0",
                    "project": {"name": "p"},
                    "sys_modules": {"enabled": True, "control": {"enabled": True}},
                }
            )
        )
        system_ids = [m for m in client.registry.module_ids if m.startswith("system.")]

        assert (
            len(system_ids) >= case["expected"]["at_least"]
        ), f"[{case['id']}] no system modules registered; the case would pass vacuously"
        # Read the DECLARED annotation, not an effective value computed with
        # defaults applied: the language default is True, which means the
        # opposite of the intended value, and inheriting it is how the
        # divergence arose.
        # get_definition().annotations, not a class attribute: that is the
        # surface system.manifest.* publishes, so it is what a consumer sees,
        # and it is the one shape all three SDKs share.
        wrong = {
            mid: getattr(getattr(client.registry.get_definition(mid), "annotations", None), "open_world", None)
            for mid in system_ids
        }
        wrong = {m: v for m, v in wrong.items() if v is not case["expected"]["every_value"]}
        assert not wrong, f"[{case['id']}] these system modules do not declare open_world=False: {wrong}"
