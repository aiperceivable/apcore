"""Tests for module version negotiation (PRD F18)."""

from __future__ import annotations

import logging
import threading
from typing import Any

import pytest

from apcore.registry.registry import Registry
from apcore.registry.version import (
    VersionedStore,
    parse_semver,
    matches_version_hint,
    select_best_version,
)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


class _VersionedModule:
    """A simple module stub with version metadata."""

    def __init__(self, version: str = "1.0.0") -> None:
        self.version = version
        self.description = f"Module v{version}"

    def execute(self, inputs: dict[str, Any], context: Any = None) -> dict[str, Any]:
        return {"version": self.version}


# ---------------------------------------------------------------------------
# Unit tests for semver parsing and matching utilities
# ---------------------------------------------------------------------------


class TestParseSemver:
    """Tests for parse_semver utility."""

    def test_parse_semver_full(self) -> None:
        assert parse_semver("1.2.3") == (1, 2, 3)

    def test_parse_semver_major_only(self) -> None:
        assert parse_semver("2") == (2, 0, 0)

    def test_parse_semver_major_minor(self) -> None:
        assert parse_semver("3.4") == (3, 4, 0)

    def test_parse_semver_zero(self) -> None:
        assert parse_semver("0.0.0") == (0, 0, 0)


class TestMatchesVersionHint:
    """Tests for matches_version_hint utility."""

    def test_matches_exact(self) -> None:
        assert matches_version_hint("1.0.0", "1.0.0") is True

    def test_matches_exact_no_match(self) -> None:
        assert matches_version_hint("2.0.0", "1.0.0") is False

    def test_matches_gte(self) -> None:
        assert matches_version_hint("1.5.0", ">=1.0.0") is True

    def test_matches_lt(self) -> None:
        assert matches_version_hint("1.5.0", "<2.0.0") is True
        assert matches_version_hint("2.0.0", "<2.0.0") is False

    def test_matches_range(self) -> None:
        assert matches_version_hint("1.5.0", ">=1.0.0,<2.0.0") is True
        assert matches_version_hint("2.0.0", ">=1.0.0,<2.0.0") is False

    def test_matches_partial_major(self) -> None:
        assert matches_version_hint("1.2.3", "1") is True
        assert matches_version_hint("2.0.0", "1") is False


class TestSelectBestVersion:
    """Tests for select_best_version utility."""

    def test_select_latest_when_no_hint(self) -> None:
        versions = ["1.0.0", "2.0.0", "1.5.0"]
        assert select_best_version(versions, version_hint=None) == "2.0.0"

    def test_select_exact_match(self) -> None:
        versions = ["1.0.0", "2.0.0"]
        assert select_best_version(versions, version_hint="1.0.0") == "1.0.0"

    def test_select_range_match(self) -> None:
        versions = ["1.0.0", "1.5.0", "2.0.0"]
        result = select_best_version(versions, version_hint=">=1.0.0,<2.0.0")
        assert result == "1.5.0"  # highest matching

    def test_select_no_match_returns_none(self) -> None:
        versions = ["1.0.0", "2.0.0"]
        assert select_best_version(versions, version_hint="3.0.0") is None


# ---------------------------------------------------------------------------
# VersionedStore tests
# ---------------------------------------------------------------------------


class TestVersionedStore:
    """Tests for VersionedStore internal data structure."""

    def test_versioned_store_add_and_get(self) -> None:
        store: VersionedStore[str] = VersionedStore()
        store.add("mod.a", "1.0.0", "value_v1")
        store.add("mod.a", "2.0.0", "value_v2")
        assert store.get("mod.a", "1.0.0") == "value_v1"
        assert store.get("mod.a", "2.0.0") == "value_v2"

    def test_versioned_store_get_latest(self) -> None:
        store: VersionedStore[str] = VersionedStore()
        store.add("mod.a", "1.0.0", "v1")
        store.add("mod.a", "2.0.0", "v2")
        assert store.get_latest("mod.a") == "v2"

    def test_versioned_store_list_versions(self) -> None:
        store: VersionedStore[str] = VersionedStore()
        store.add("mod.a", "1.0.0", "v1")
        store.add("mod.a", "2.0.0", "v2")
        assert store.list_versions("mod.a") == ["1.0.0", "2.0.0"]

    def test_versioned_store_list_ids(self) -> None:
        store: VersionedStore[str] = VersionedStore()
        store.add("mod.a", "1.0.0", "v1")
        store.add("mod.b", "1.0.0", "v1")
        assert sorted(store.list_ids()) == ["mod.a", "mod.b"]

    def test_versioned_store_remove(self) -> None:
        store: VersionedStore[str] = VersionedStore()
        store.add("mod.a", "1.0.0", "v1")
        store.remove("mod.a", "1.0.0")
        assert store.get("mod.a", "1.0.0") is None

    def test_versioned_store_resolve(self) -> None:
        store: VersionedStore[str] = VersionedStore()
        store.add("mod.a", "1.0.0", "v1")
        store.add("mod.a", "2.0.0", "v2")
        assert store.resolve("mod.a", ">=1.0.0,<2.0.0") == "v1"
        assert store.resolve("mod.a", None) == "v2"


# ---------------------------------------------------------------------------
# Registry-level version negotiation tests
# ---------------------------------------------------------------------------


class TestRegistryVersionNegotiation:
    """Tests for version negotiation integrated into the Registry."""

    def test_module_declares_version(self) -> None:
        reg = Registry(extensions_dir="/tmp/fake_ext")
        mod = _VersionedModule(version="1.2.0")
        reg.register("mod.versioned", mod, version="1.2.0")
        defn = reg.get_definition("mod.versioned")
        assert defn is not None
        assert defn.version == "1.2.0"

    def test_module_declares_compatible_versions(self) -> None:
        reg = Registry(extensions_dir="/tmp/fake_ext")
        mod = _VersionedModule(version="1.0.0")
        reg.register(
            "mod.compat",
            mod,
            version="1.0.0",
            metadata={"x-compatible-versions": [">=1.0.0", "<2.0.0"]},
        )
        defn = reg.get_definition("mod.compat")
        assert defn is not None
        assert defn.metadata.get("x-compatible-versions") == [">=1.0.0", "<2.0.0"]

    def test_module_declares_deprecation(self) -> None:
        reg = Registry(extensions_dir="/tmp/fake_ext")
        mod = _VersionedModule(version="1.0.0")
        deprecation = {
            "deprecated_since": "1.0.0",
            "sunset_version": "3.0.0",
            "migration_guide": "Use mod.new instead.",
        }
        reg.register(
            "mod.deprecated",
            mod,
            version="1.0.0",
            metadata={"x-deprecation": deprecation},
        )
        defn = reg.get_definition("mod.deprecated")
        assert defn is not None
        assert defn.metadata["x-deprecation"]["deprecated_since"] == "1.0.0"
        assert defn.metadata["x-deprecation"]["migration_guide"] == "Use mod.new instead."

    def test_call_with_version_hint_selects_matching(self) -> None:
        reg = Registry(extensions_dir="/tmp/fake_ext")
        mod_v1 = _VersionedModule(version="1.0.0")
        mod_v2 = _VersionedModule(version="2.0.0")
        reg.register("mod.multi", mod_v1, version="1.0.0")
        reg.register("mod.multi", mod_v2, version="2.0.0")
        module = reg.get("mod.multi", version_hint="1.0.0")
        assert module is not None
        assert module.version == "1.0.0"

    def test_call_without_version_hint_selects_latest(self) -> None:
        reg = Registry(extensions_dir="/tmp/fake_ext")
        mod_v1 = _VersionedModule(version="1.0.0")
        mod_v2 = _VersionedModule(version="2.0.0")
        reg.register("mod.multi", mod_v1, version="1.0.0")
        reg.register("mod.multi", mod_v2, version="2.0.0")
        module = reg.get("mod.multi")
        assert module is not None
        assert module.version == "2.0.0"

    def test_call_with_semver_range_hint(self) -> None:
        reg = Registry(extensions_dir="/tmp/fake_ext")
        mod_v1 = _VersionedModule(version="1.0.0")
        mod_v2 = _VersionedModule(version="2.0.0")
        reg.register("mod.multi", mod_v1, version="1.0.0")
        reg.register("mod.multi", mod_v2, version="2.0.0")
        module = reg.get("mod.multi", version_hint=">=1.0.0,<2.0.0")
        assert module is not None
        assert module.version == "1.0.0"

    def test_call_version_hint_no_match(self) -> None:
        reg = Registry(extensions_dir="/tmp/fake_ext")
        mod_v1 = _VersionedModule(version="1.0.0")
        mod_v2 = _VersionedModule(version="2.0.0")
        reg.register("mod.multi", mod_v1, version="1.0.0")
        reg.register("mod.multi", mod_v2, version="2.0.0")
        module = reg.get("mod.multi", version_hint="3.0.0")
        assert module is None

    def test_deprecated_module_logs_warning(self, caplog: pytest.LogCaptureFixture) -> None:
        reg = Registry(extensions_dir="/tmp/fake_ext")
        mod = _VersionedModule(version="1.0.0")
        deprecation = {
            "deprecated_since": "1.0.0",
            "sunset_version": "3.0.0",
            "migration_guide": "Use mod.new instead.",
        }
        reg.register(
            "mod.dep",
            mod,
            version="1.0.0",
            metadata={"x-deprecation": deprecation},
        )
        with caplog.at_level(logging.WARNING):
            reg.get_definition("mod.dep", version_hint="1.0.0")
        assert any("deprecated" in r.message.lower() for r in caplog.records)

    def test_deprecated_module_includes_migration_guide(self, caplog: pytest.LogCaptureFixture) -> None:
        reg = Registry(extensions_dir="/tmp/fake_ext")
        mod = _VersionedModule(version="1.0.0")
        deprecation = {
            "deprecated_since": "1.0.0",
            "sunset_version": "3.0.0",
            "migration_guide": "Use mod.new instead.",
        }
        reg.register(
            "mod.dep",
            mod,
            version="1.0.0",
            metadata={"x-deprecation": deprecation},
        )
        with caplog.at_level(logging.WARNING):
            reg.get_definition("mod.dep", version_hint="1.0.0")
        assert any("Use mod.new instead." in r.message for r in caplog.records)

    def test_multiple_versions_registered(self) -> None:
        reg = Registry(extensions_dir="/tmp/fake_ext")
        for v in ["1.0.0", "1.5.0", "2.0.0"]:
            reg.register("mod.multi", _VersionedModule(version=v), version=v)
        for v in ["1.0.0", "1.5.0", "2.0.0"]:
            m = reg.get("mod.multi", version_hint=v)
            assert m is not None
            assert m.version == v

    def test_version_hint_partial_match(self) -> None:
        reg = Registry(extensions_dir="/tmp/fake_ext")
        reg.register("mod.partial", _VersionedModule(version="1.2.3"), version="1.2.3")
        reg.register("mod.partial", _VersionedModule(version="2.0.0"), version="2.0.0")
        m = reg.get("mod.partial", version_hint="1")
        assert m is not None
        assert m.version == "1.2.3"

    def test_module_without_version_defaults(self) -> None:
        reg = Registry(extensions_dir="/tmp/fake_ext")
        mod = _VersionedModule(version="1.0.0")
        reg.register("mod.noversion", mod)  # no explicit version param
        m = reg.get("mod.noversion")
        assert m is not None

    def test_compatible_versions_validation(self) -> None:
        reg = Registry(extensions_dir="/tmp/fake_ext")
        mod = _VersionedModule(version="1.5.0")
        reg.register(
            "mod.compat",
            mod,
            version="1.5.0",
            metadata={"x-compatible-versions": [">=1.0.0", "<2.0.0"]},
        )
        defn = reg.get_definition("mod.compat")
        assert defn is not None
        compat = defn.metadata.get("x-compatible-versions", [])
        # The version 1.5.0 should be within the declared compatible range
        version_str = defn.version
        for constraint in compat:
            assert matches_version_hint(version_str, constraint) is True

    def test_version_negotiation_thread_safe(self) -> None:
        reg = Registry(extensions_dir="/tmp/fake_ext")
        for v in ["1.0.0", "2.0.0", "3.0.0"]:
            reg.register("mod.threaded", _VersionedModule(version=v), version=v)

        results: dict[str, str | None] = {}
        errors: list[Exception] = []

        def get_version(hint: str) -> None:
            try:
                m = reg.get("mod.threaded", version_hint=hint)
                results[hint] = m.version if m else None
            except Exception as e:
                errors.append(e)

        threads = [threading.Thread(target=get_version, args=(v,)) for v in ["1.0.0", "2.0.0", "3.0.0"] * 10]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

        assert not errors
        assert results.get("1.0.0") == "1.0.0"
        assert results.get("2.0.0") == "2.0.0"
        assert results.get("3.0.0") == "3.0.0"

    def test_list_returns_unique_module_ids(self) -> None:
        reg = Registry(extensions_dir="/tmp/fake_ext")
        reg.register("mod.listed", _VersionedModule(version="1.0.0"), version="1.0.0")
        reg.register("mod.listed", _VersionedModule(version="2.0.0"), version="2.0.0")
        ids = reg.list()
        assert ids.count("mod.listed") == 1


# ---------------------------------------------------------------------------
# Executor version_hint integration tests
# ---------------------------------------------------------------------------


class TestExecutorVersionHint:
    """Tests for executor.call() with version_hint parameter."""

    def test_executor_call_with_version_hint(self) -> None:
        from apcore.executor import Executor

        reg = Registry(extensions_dir="/tmp/fake_ext")
        mod_v1 = _VersionedModule(version="1.0.0")
        mod_v1.input_schema = None
        mod_v1.output_schema = None
        mod_v2 = _VersionedModule(version="2.0.0")
        mod_v2.input_schema = None
        mod_v2.output_schema = None
        reg.register("mod.exec", mod_v1, version="1.0.0")
        reg.register("mod.exec", mod_v2, version="2.0.0")

        executor = Executor(registry=reg)
        result = executor.call("mod.exec", version_hint="1.0.0")
        assert result["version"] == "1.0.0"

    def test_executor_call_without_version_hint_uses_latest(self) -> None:
        from apcore.executor import Executor

        reg = Registry(extensions_dir="/tmp/fake_ext")
        mod_v1 = _VersionedModule(version="1.0.0")
        mod_v1.input_schema = None
        mod_v1.output_schema = None
        mod_v2 = _VersionedModule(version="2.0.0")
        mod_v2.input_schema = None
        mod_v2.output_schema = None
        reg.register("mod.exec", mod_v1, version="1.0.0")
        reg.register("mod.exec", mod_v2, version="2.0.0")

        executor = Executor(registry=reg)
        result = executor.call("mod.exec")
        assert result["version"] == "2.0.0"


# ---------------------------------------------------------------------------
# Strict constraint validation (regression tests for review-flagged gaps)
# ---------------------------------------------------------------------------


class TestConstraintStrictness:
    """Constraints must fail loudly on malformed operands, not silently match."""

    def test_empty_constraint_raises(self) -> None:
        from apcore.errors import VersionConstraintError

        with pytest.raises(VersionConstraintError):
            matches_version_hint("1.0.0", "")

    def test_v_prefix_rejected(self) -> None:
        from apcore.errors import VersionConstraintError

        with pytest.raises(VersionConstraintError):
            matches_version_hint("1.0.0", "v1.0.0")

    def test_non_digit_operand_rejected(self) -> None:
        from apcore.errors import VersionConstraintError

        with pytest.raises(VersionConstraintError):
            matches_version_hint("1.0.0", ">=not_a_version")

    def test_operator_without_operand_rejected(self) -> None:
        from apcore.errors import VersionConstraintError

        with pytest.raises(VersionConstraintError):
            matches_version_hint("1.0.0", "~")

    def test_comma_list_rejects_on_any_bad_entry(self) -> None:
        from apcore.errors import VersionConstraintError

        with pytest.raises(VersionConstraintError):
            matches_version_hint("1.5.0", ">=1.0.0,latest")

    def test_caret_zero_zero_patch_lower_bound(self) -> None:
        # ^0.0.3 → >=0.0.3,<0.0.4 per npm/cargo semantics
        assert matches_version_hint("0.0.3", "^0.0.3") is True
        assert matches_version_hint("0.0.4", "^0.0.3") is False
        assert matches_version_hint("0.0.2", "^0.0.3") is False

    def test_tilde_one_part(self) -> None:
        # ~1 → >=1.0.0,<2.0.0
        assert matches_version_hint("1.0.0", "~1") is True
        assert matches_version_hint("1.9.9", "~1") is True
        assert matches_version_hint("2.0.0", "~1") is False


# ---------------------------------------------------------------------------
# D-89 — deprecation warning cadence
# ---------------------------------------------------------------------------


class TestDeprecationWarningCadence:
    """At most one warning per ``(module_id, version)`` per registry (D-89).

    ``get_definition`` is a read that hosts call in loops, so warning on every
    read produces log spam proportional to traffic — which is how operators
    learn to filter the advisory out. Mirrors apcore-typescript.
    """

    _DEPRECATION = {
        "deprecated_since": "1.0.0",
        "sunset_version": "3.0.0",
        "migration_guide": "Use mod.new instead.",
    }

    @staticmethod
    def _deprecation_warnings(caplog: pytest.LogCaptureFixture) -> list[str]:
        return [r.message for r in caplog.records if "deprecated" in r.message.lower()]

    def _register(
        self,
        reg: Registry,
        module_id: str,
        version: str = "1.0.0",
        deprecation: dict[str, str] | None = None,
    ) -> None:
        metadata = {} if deprecation == {} else {"x-deprecation": dict(deprecation or self._DEPRECATION)}
        reg.register(
            module_id,
            _VersionedModule(version=version),
            version=version,
            metadata=metadata,
        )

    def test_repeated_reads_warn_once(self, caplog: pytest.LogCaptureFixture) -> None:
        reg = Registry(extensions_dir="/tmp/fake_ext")
        self._register(reg, "cadence.once")

        with caplog.at_level(logging.WARNING):
            for _ in range(10):
                reg.get_definition("cadence.once", version_hint="1.0.0")

        assert len(self._deprecation_warnings(caplog)) == 1

    def test_the_dedupe_key_includes_the_version(self, caplog: pytest.LogCaptureFixture) -> None:
        """A newly registered version is a new key, so it warns again."""
        reg = Registry(extensions_dir="/tmp/fake_ext")
        self._register(reg, "cadence.versions", version="1.0.0")

        with caplog.at_level(logging.WARNING):
            reg.get_definition("cadence.versions")
            reg.get_definition("cadence.versions")
            self._register(reg, "cadence.versions", version="2.0.0")
            reg.get_definition("cadence.versions")
            reg.get_definition("cadence.versions")

        warnings = self._deprecation_warnings(caplog)
        assert len(warnings) == 2
        assert "v1.0.0" in warnings[0]
        assert "v2.0.0" in warnings[1]

    def test_distinct_modules_warn_separately(self, caplog: pytest.LogCaptureFixture) -> None:
        reg = Registry(extensions_dir="/tmp/fake_ext")
        self._register(reg, "cadence.first")
        self._register(reg, "cadence.second")

        with caplog.at_level(logging.WARNING):
            for _ in range(3):
                reg.get_definition("cadence.first")
                reg.get_definition("cadence.second")

        assert len(self._deprecation_warnings(caplog)) == 2

    def test_the_dedupe_is_per_registry_instance(self, caplog: pytest.LogCaptureFixture) -> None:
        reg_a = Registry(extensions_dir="/tmp/fake_ext")
        reg_b = Registry(extensions_dir="/tmp/fake_ext")
        self._register(reg_a, "cadence.instance")
        self._register(reg_b, "cadence.instance")

        with caplog.at_level(logging.WARNING):
            reg_a.get_definition("cadence.instance", version_hint="1.0.0")
            reg_b.get_definition("cadence.instance", version_hint="1.0.0")

        assert len(self._deprecation_warnings(caplog)) == 2

    def test_the_warning_fires_on_the_read_not_on_registration(self, caplog: pytest.LogCaptureFixture) -> None:
        """D-89 / spec v1.59.0: `get_definition` is the emission point.

        Registration runs at startup, frequently before the host has installed
        a log subscriber — `discover()` at import time is the ordinary case —
        and a registration-time warning is then lost with no later chance to
        re-emit, because the dedupe entry has already been written.
        apcore-rust emitted at registration; this is the assertion that pins
        the adjudicated answer rather than merely the SDK that had it.
        """
        reg = Registry(extensions_dir="/tmp/fake_ext")

        with caplog.at_level(logging.WARNING):
            self._register(reg, "cadence.read_path")
            assert self._deprecation_warnings(caplog) == [], (
                "registration must not warn; an operator who configures logging " "after discovery would never see it"
            )

            reg.get_definition("cadence.read_path", version_hint="1.0.0")

        assert len(self._deprecation_warnings(caplog)) == 1

    def test_re_registration_with_the_SAME_notice_stays_silent(self, caplog: pytest.LogCaptureFixture) -> None:
        """D-89 / spec v1.59.0: the dedupe key survives unregister.

        This SDK used to drop the marker on `unregister` and warn again. But
        `watch()` re-runs discovery as an unregister + re-register, so that
        re-warned for every deprecated module on every hot reload — the
        traffic-proportional spam D-89 exists to prevent, arriving through the
        door its wording did not close.
        """
        reg = Registry(extensions_dir="/tmp/fake_ext")
        self._register(reg, "cadence.same_notice")

        with caplog.at_level(logging.WARNING):
            reg.get_definition("cadence.same_notice", version_hint="1.0.0")
            reg.unregister("cadence.same_notice")
            self._register(reg, "cadence.same_notice")
            reg.get_definition("cadence.same_notice", version_hint="1.0.0")

        assert len(self._deprecation_warnings(caplog)) == 1

    def test_re_registration_with_a_CHANGED_notice_warns_again(self, caplog: pytest.LogCaptureFixture) -> None:
        """The other half, and the control for the test above.

        Without it, "stays silent" is equally satisfied by an implementation
        that never warns for a re-registered module at all — which is what
        apcore-typescript and apcore-rust did, swallowing a genuinely new
        notice. The block is in the key, so a changed notice is a new key.
        """
        reg = Registry(extensions_dir="/tmp/fake_ext")
        self._register(reg, "cadence.changed_notice")

        with caplog.at_level(logging.WARNING):
            reg.get_definition("cadence.changed_notice", version_hint="1.0.0")
            reg.unregister("cadence.changed_notice")
            self._register(
                reg,
                "cadence.changed_notice",
                deprecation={
                    "deprecated_since": "1.0.0",
                    "sunset_version": "2.0.0",  # brought forward
                    "migration_guide": "Use mod.new instead.",
                },
            )
            reg.get_definition("cadence.changed_notice", version_hint="1.0.0")

        warnings = self._deprecation_warnings(caplog)
        assert len(warnings) == 2
        assert "sunset in 3.0.0" in warnings[0]
        assert "sunset in 2.0.0" in warnings[1]

    def test_a_notice_ADDED_on_re_registration_warns(self, caplog: pytest.LogCaptureFixture) -> None:
        """A module that was not deprecated and now is must be announced."""
        reg = Registry(extensions_dir="/tmp/fake_ext")
        self._register(reg, "cadence.added_notice", deprecation={})

        with caplog.at_level(logging.WARNING):
            reg.get_definition("cadence.added_notice", version_hint="1.0.0")
            assert self._deprecation_warnings(caplog) == []

            reg.unregister("cadence.added_notice")
            self._register(reg, "cadence.added_notice")
            reg.get_definition("cadence.added_notice", version_hint="1.0.0")

        assert len(self._deprecation_warnings(caplog)) == 1
