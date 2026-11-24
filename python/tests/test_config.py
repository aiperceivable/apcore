"""Tests for Config system (Algorithm A12)."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any
from unittest import mock

import pytest

from apcore.config import _CONSTRAINTS, _DEFAULTS, _REQUIRED_FIELDS, Config
from apcore.errors import ConfigError, ConfigNotFoundError


# ---------------------------------------------------------------------------
# Backward compatibility: Config(data=...) still works
# ---------------------------------------------------------------------------


class TestConfigBackwardCompat:
    def test_positional_dict(self) -> None:
        config = Config({"executor": {"default_timeout": 5000}})
        assert config.get("executor.default_timeout") == 5000

    def test_keyword_data(self) -> None:
        config = Config(data={"executor": {"max_call_depth": 10}})
        assert config.get("executor.max_call_depth") == 10

    def test_none_data(self) -> None:
        config = Config()
        assert config.get("anything") is None

    def test_get_default(self) -> None:
        config = Config()
        assert config.get("missing.key", 42) == 42

    def test_nested_dot_path(self) -> None:
        config = Config(data={"a": {"b": {"c": 99}}})
        assert config.get("a.b.c") == 99

    def test_partial_path_returns_default(self) -> None:
        config = Config(data={"a": {"b": 1}})
        assert config.get("a.b.c", "nope") == "nope"


# ---------------------------------------------------------------------------
# Config.load() from YAML
# ---------------------------------------------------------------------------


class TestConfigLoad:
    def test_load_valid_yaml(self, tmp_path: Path) -> None:
        yaml_file = tmp_path / "apcore.yaml"
        yaml_file.write_text(
            """
version: "1.0.0"
project:
  name: myproject
extensions:
  root: ./ext
schema:
  root: ./sch
acl:
  root: ./acl
  default_effect: allow
"""
        )
        # Create the directories so semantic validation passes
        (tmp_path / "ext").mkdir()
        (tmp_path / "sch").mkdir()
        (tmp_path / "acl").mkdir()
        config = Config.load(str(yaml_file), validate=False)
        assert config.get("version") == "1.0.0"
        assert config.get("project.name") == "myproject"
        assert config.get("acl.default_effect") == "allow"

    def test_load_nonexistent_file(self) -> None:
        with pytest.raises(ConfigNotFoundError):
            Config.load("/nonexistent/path.yaml")

    def test_load_invalid_yaml(self, tmp_path: Path) -> None:
        yaml_file = tmp_path / "bad.yaml"
        yaml_file.write_text("key: [unclosed bracket")
        with pytest.raises(ConfigError, match="Invalid YAML"):
            Config.load(str(yaml_file))

    def test_load_non_dict_yaml(self, tmp_path: Path) -> None:
        yaml_file = tmp_path / "list.yaml"
        yaml_file.write_text("- item1\n- item2\n")
        with pytest.raises(ConfigError, match="must be a mapping"):
            Config.load(str(yaml_file))

    def test_load_empty_yaml(self, tmp_path: Path) -> None:
        yaml_file = tmp_path / "empty.yaml"
        yaml_file.write_text("")
        # Empty YAML → empty dict → defaults applied, validation may warn
        config = Config.load(str(yaml_file), validate=False)
        # Defaults should be merged
        assert config.get("executor.default_timeout") == 30000

    def test_load_merges_defaults(self, tmp_path: Path) -> None:
        yaml_file = tmp_path / "partial.yaml"
        yaml_file.write_text("executor:\n  default_timeout: 5000\n")
        config = Config.load(str(yaml_file), validate=False)
        assert config.get("executor.default_timeout") == 5000
        # Defaults for fields not in file
        assert config.get("executor.max_call_depth") == 32


# ---------------------------------------------------------------------------
# Environment variable overrides
# ---------------------------------------------------------------------------


class TestConfigEnvOverrides:
    def test_env_overrides_file(self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
        yaml_file = tmp_path / "apcore.yaml"
        yaml_file.write_text("executor:\n  default_timeout: 5000\n")
        monkeypatch.setenv("APCORE_EXECUTOR_DEFAULT__TIMEOUT", "9999")
        config = Config.load(str(yaml_file), validate=False)
        assert config.get("executor.default_timeout") == 9999

    def test_env_bool_coercion(self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
        yaml_file = tmp_path / "apcore.yaml"
        yaml_file.write_text("{}")
        monkeypatch.setenv("APCORE_EXTENSIONS_AUTO__DISCOVER", "false")
        config = Config.load(str(yaml_file), validate=False)
        assert config.get("extensions.auto_discover") is False

    def test_env_float_coercion(self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
        yaml_file = tmp_path / "apcore.yaml"
        yaml_file.write_text("{}")
        monkeypatch.setenv("APCORE_OBSERVABILITY_TRACING_SAMPLING__RATE", "0.5")
        config = Config.load(str(yaml_file), validate=False)
        assert config.get("observability.tracing.sampling_rate") == 0.5

    def test_env_string_value(self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
        yaml_file = tmp_path / "apcore.yaml"
        yaml_file.write_text("{}")
        monkeypatch.setenv("APCORE_PROJECT_NAME", "envproject")
        config = Config.load(str(yaml_file), validate=False)
        assert config.get("project.name") == "envproject"


# ---------------------------------------------------------------------------
# Validation (Algorithm A12)
# ---------------------------------------------------------------------------


class TestConfigValidation:
    def _make_valid_data(self, tmp_path: Path | None = None) -> dict[str, Any]:
        return {
            "version": "1.0.0",
            "extensions": {"root": "./ext"},
            "schema": {"root": "./sch"},
            "acl": {"root": "./acl", "default_effect": "deny"},
            "project": {"name": "test"},
        }

    def test_valid_config_passes(self) -> None:
        config = Config(data=self._make_valid_data())
        config.validate()  # Should not raise

    def test_missing_required_field(self) -> None:
        data = self._make_valid_data()
        del data["version"]
        config = Config(data=data)
        with pytest.raises(ConfigError, match="Missing required field.*version"):
            config.validate()

    def test_multiple_missing_fields(self) -> None:
        config = Config(data={})
        with pytest.raises(ConfigError, match="error\\(s\\)") as exc_info:
            config.validate()
        # Both required fields are reported, not just the first. There are
        # exactly two (§9.1): every other §9.1 key carries a canonical default
        # and so is never required.
        err = exc_info.value
        assert "errors" in err.details
        assert len(err.details["errors"]) == 2

    def test_invalid_acl_default_effect(self) -> None:
        data = self._make_valid_data()
        data["acl"]["default_effect"] = "maybe"
        config = Config(data=data)
        with pytest.raises(ConfigError, match="acl.default_effect"):
            config.validate()

    def test_invalid_sampling_rate(self) -> None:
        data = self._make_valid_data()
        data["observability"] = {"tracing": {"sampling_rate": 2.0}}
        config = Config(data=data)
        with pytest.raises(ConfigError, match="sampling_rate"):
            config.validate()

    def test_invalid_max_depth(self) -> None:
        data = self._make_valid_data()
        data["extensions"]["max_depth"] = 0
        config = Config(data=data)
        with pytest.raises(ConfigError, match="max_depth"):
            config.validate()

    def test_yaml_only_strategy_missing_root(self, tmp_path: Path) -> None:
        data = self._make_valid_data()
        data["schema"]["strategy"] = "yaml_only"
        data["schema"]["root"] = str(tmp_path / "nonexistent")
        config = Config(data=data)
        with pytest.raises(ConfigError, match="yaml_only"):
            config.validate()

    def test_valid_constraints_pass(self) -> None:
        data = self._make_valid_data()
        data["observability"] = {"tracing": {"sampling_rate": 0.5}}
        data["extensions"]["max_depth"] = 8
        config = Config(data=data)
        config.validate()  # Should not raise


# ---------------------------------------------------------------------------
# Config.set()
# ---------------------------------------------------------------------------


class TestConfigSet:
    def test_set_and_get(self) -> None:
        config = Config()
        config.set("executor.default_timeout", 5000)
        assert config.get("executor.default_timeout") == 5000

    def test_set_creates_intermediate_dicts(self) -> None:
        config = Config()
        config.set("a.b.c", 42)
        assert config.get("a.b.c") == 42


# ---------------------------------------------------------------------------
# Config.reload()
# ---------------------------------------------------------------------------


class TestConfigReload:
    def test_reload_rereads_file(self, tmp_path: Path) -> None:
        # reload() re-enters Config.load() with its default validate=True, so
        # the fixture must be a valid document: version + project.name.
        header = 'version: "1.0.0"\nproject:\n  name: test\n'
        yaml_file = tmp_path / "apcore.yaml"
        yaml_file.write_text(header + "executor:\n  default_timeout: 1000\n")
        config = Config.load(str(yaml_file), validate=False)
        assert config.get("executor.default_timeout") == 1000

        # Modify file
        yaml_file.write_text(header + "executor:\n  default_timeout: 9999\n")
        config.reload()
        assert config.get("executor.default_timeout") == 9999

    def test_reload_without_yaml_path_raises(self) -> None:
        config = Config(data={"a": 1})
        with pytest.raises(ConfigError, match="Cannot reload"):
            config.reload()


# ---------------------------------------------------------------------------
# Config.from_defaults()
# ---------------------------------------------------------------------------


class TestConfigFromDefaults:
    def test_has_default_values(self) -> None:
        config = Config.from_defaults()
        assert config.get("executor.default_timeout") == 30000
        assert config.get("executor.max_call_depth") == 32
        assert config.get("schema.strategy") == "yaml_first"

    def test_env_applied(self, monkeypatch: pytest.MonkeyPatch) -> None:
        monkeypatch.setenv("APCORE_EXECUTOR_DEFAULT__TIMEOUT", "7777")
        config = Config.from_defaults()
        assert config.get("executor.default_timeout") == 7777


# ---------------------------------------------------------------------------
# Config.data property
# ---------------------------------------------------------------------------


class TestConfigData:
    def test_data_returns_copy(self) -> None:
        config = Config(data={"a": 1})
        d = config.data
        d["a"] = 999
        assert config.get("a") == 1  # Original unchanged

    def test_repr_with_yaml_path(self, tmp_path: Path) -> None:
        yaml_file = tmp_path / "apcore.yaml"
        yaml_file.write_text("{}")
        config = Config.load(str(yaml_file), validate=False)
        assert "yaml_path=" in repr(config)

    def test_repr_without_yaml_path(self) -> None:
        config = Config(data={"a": 1})
        assert "keys" in repr(config)


# ---------------------------------------------------------------------------
# sys_modules and project source fields
# ---------------------------------------------------------------------------


class TestSysModulesConfig:
    def test_config_sys_modules_enabled_default(self) -> None:
        config = Config(data=dict(_DEFAULTS))
        assert config.get("sys_modules.enabled") is False

    def test_default_table_carries_only_what_the_defaults_schema_declares(self) -> None:
        # Same rule as `test_config_declares_no_project_defaults` below:
        # `schemas/defaults.schema.json` declares `sys_modules.enabled` and
        # nothing else, and it is `additionalProperties: false`. The thirteen
        # other `sys_modules` defaults belong to
        # `schemas/sys-modules.schema.json` and reach a Config through the
        # namespace registration, not through this table.
        #
        # They used to sit here too, which made these keys readable in legacy
        # mode from apcore-python alone — apcore-typescript and apcore-rust
        # answer undefined/None over the same call, because their default
        # tables mirror `defaults.schema.json` exactly (sync finding A-D-021).
        config = Config(data=dict(_DEFAULTS))
        for key in (
            "sys_modules.error_history.max_entries_per_module",
            "sys_modules.error_history.max_total_entries",
            "sys_modules.events.enabled",
            "sys_modules.events.thresholds.error_rate",
            "sys_modules.events.thresholds.latency_p99_ms",
            "sys_modules.events.subscribers",
        ):
            assert (
                config.get(key) is None
            ), f"{key} is not declared by defaults.schema.json, so the legacy default table must not answer for it"

    def test_namespace_mode_still_supplies_the_sys_modules_defaults(self, tmp_path: Path) -> None:
        # The other half of the rule above: dropping those keys from the legacy
        # table must not make them unreachable, because §9.15.3 registers
        # `sys_modules` as a namespace that declares all fourteen.
        path = tmp_path / "apcore.json"
        path.write_text(json.dumps({"apcore": {"version": "1.0"}}))
        config = Config.load(path)
        assert config.get("sys_modules.error_history.max_entries_per_module") == 50
        assert config.get("sys_modules.error_history.max_total_entries") == 1000
        assert config.get("sys_modules.events.enabled") is False
        assert config.get("sys_modules.events.thresholds.error_rate") == 0.1
        assert config.get("sys_modules.events.thresholds.latency_p99_ms") == 5000.0

    def test_config_declares_no_project_defaults(self) -> None:
        # `schemas/defaults.schema.json` declares no `project` subtree, so
        # neither does `_DEFAULTS` (§9.1: `project.name` is required precisely
        # because it has no canonical default). Consumers that want a fallback
        # supply it at the call site.
        config = Config(data=dict(_DEFAULTS))
        assert config.get("project.source_repo") is None
        assert config.get("project.source_root") is None
        assert config.get("project.source_root", "") == ""

    def test_config_sys_modules_from_yaml(self, tmp_path: Path) -> None:
        yaml_content = "sys_modules:\n  enabled: true\nversion: '0.8.0'\nextensions:\n  root: ./extensions\nschema:\n  root: ./schemas\nacl:\n  root: ./acl\n  default_effect: deny\nproject:\n  name: test"
        config_file = tmp_path / "apcore.yaml"
        config_file.write_text(yaml_content)
        config = Config.load(str(config_file), validate=False)
        assert config.get("sys_modules.enabled") is True

    def test_config_sys_modules_env_override(self, monkeypatch: pytest.MonkeyPatch) -> None:
        monkeypatch.setenv("APCORE_SYS__MODULES_ENABLED", "true")
        config = Config.from_defaults()
        assert config.get("sys_modules.enabled") is True

    def test_config_project_source_repo_yaml(self, tmp_path: Path) -> None:
        yaml_content = "project:\n  name: test\n  source_repo: 'https://github.com/org/repo'\nversion: '0.8.0'\nextensions:\n  root: ./extensions\nschema:\n  root: ./schemas\nacl:\n  root: ./acl\n  default_effect: deny"
        config_file = tmp_path / "apcore.yaml"
        config_file.write_text(yaml_content)
        config = Config.load(str(config_file), validate=False)
        assert config.get("project.source_repo") == "https://github.com/org/repo"

    def test_config_project_source_root_yaml(self, tmp_path: Path) -> None:
        yaml_content = "project:\n  name: test\n  source_root: 'src/modules'\nversion: '0.8.0'\nextensions:\n  root: ./extensions\nschema:\n  root: ./schemas\nacl:\n  root: ./acl\n  default_effect: deny"
        config_file = tmp_path / "apcore.yaml"
        config_file.write_text(yaml_content)
        config = Config.load(str(config_file), validate=False)
        assert config.get("project.source_root") == "src/modules"


class TestNumericConstraintsRejectBooleans:
    """``docs/features/config-bus.md``: "Booleans are rejected for all numeric fields".

    ``bool`` subclasses ``int`` in Python, so ``isinstance(True, int)`` is True
    and every constraint lambda written as ``isinstance(v, int) and v >= 1``
    happily accepted ``true`` as the number 1 — silently turning
    ``executor.max_call_depth: true`` into a depth limit of 1. The four
    ``circuit_breaker`` entries already carried ``and not isinstance(v, bool)``;
    the rest did not. apcore-typescript and apcore-rust reject booleans for all
    of them.
    """

    #: Constrained keys whose value is not a number. Named rather than derived:
    #: this list used to be "everything except `acl.default_effect`", and the
    #: three `observability.tracing.*` constraints added in spec v1.44.0 turned
    #: the whole class red. Adding a NUMERIC key still cannot skip this test —
    #: that is what the derivation is for — while adding a non-numeric one now
    #: costs one deliberate line here.
    NON_NUMERIC_FIELDS = frozenset(
        {
            "acl.default_effect",
            "observability.tracing.strategy",
            "observability.tracing.exporter",
            "observability.tracing.otlp_endpoint",
        }
    )

    # Every numeric key in `_CONSTRAINTS`, derived from the table itself so a
    # newly added key cannot quietly skip this test. A set difference rather
    # than a comprehension: a comprehension's condition is evaluated outside
    # the class body and cannot see the name above it.
    NUMERIC_FIELDS = sorted(set(_CONSTRAINTS) - NON_NUMERIC_FIELDS)

    def test_the_numeric_field_list_is_complete(self) -> None:
        # Guards the derivation above.
        assert len(self.NUMERIC_FIELDS) == len(_CONSTRAINTS) - len(self.NON_NUMERIC_FIELDS)
        assert self.NON_NUMERIC_FIELDS.isdisjoint(self.NUMERIC_FIELDS)
        assert self.NON_NUMERIC_FIELDS <= set(_CONSTRAINTS)

    @pytest.mark.parametrize("field", NUMERIC_FIELDS)
    @pytest.mark.parametrize("value", [True, False])
    def test_boolean_is_rejected(self, field: str, value: bool) -> None:
        check_fn, _msg = _CONSTRAINTS[field]
        assert check_fn(value) is False, f"{field} accepted {value!r} as a number"

    @pytest.mark.parametrize("field", NUMERIC_FIELDS)
    def test_a_real_number_is_still_accepted(self, field: str) -> None:
        # 1 satisfies every numeric constraint in the table (>= 0, >= 1,
        # [0.0, 1.0] inclusive, > 0, and [1, 16]).
        check_fn, _msg = _CONSTRAINTS[field]
        assert check_fn(1) is True, f"{field} rejected the number 1"

    def test_boolean_surfaces_as_a_config_error_through_validate(self) -> None:
        data: dict[str, Any] = {
            "version": "1.0.0",
            "extensions": {"root": "./ext"},
            "schema": {"root": "./sch"},
            "acl": {"root": "./acl", "default_effect": "deny"},
            "project": {"name": "test"},
            "executor": {"max_call_depth": True},
        }
        with pytest.raises(ConfigError, match="max_call_depth"):
            Config(data=data).validate()


# ---------------------------------------------------------------------------
# Required fields are evaluated against the DECLARED document (PROTOCOL_SPEC
# §9.1 "What is required, and why so little is" / §9.3 step 1)
# ---------------------------------------------------------------------------


class TestRequiredFieldsAreReachable:
    """A required-field list that is checked after defaults are merged is dead code.

    ``_load_legacy_mode`` deep-merges ``_DEFAULTS`` into the parsed document and
    ``validate()`` then looked for required fields in the *merged* result — so
    the check could never fail, because the merge had already supplied every
    key. ``_DEFAULTS`` even carried an invented ``version: "0.16.0"`` and a
    ``project`` subtree that ``schemas/defaults.schema.json`` has never
    declared, which existed only to keep that check vacuous.

    PROTOCOL_SPEC §9.1: a key is required only when it has no canonical
    default; exactly ``version`` and ``project.name`` qualify. §9.3 step 1:
    requiredness is evaluated against the declared document, before defaults
    are merged.
    """

    # -- the invented defaults are gone -------------------------------------

    def test_defaults_do_not_invent_version_or_project(self) -> None:
        # schemas/defaults.schema.json declares neither, so neither may appear
        # in the SDK default table. Their presence is what made the
        # required-field check unreachable.
        assert "version" not in _DEFAULTS
        assert "project" not in _DEFAULTS

    def test_no_required_field_has_a_default(self) -> None:
        # The defining property of a required key (§9.1). If this ever fails,
        # the required-field check has silently become a no-op again.
        for field in _REQUIRED_FIELDS:
            assert Config.get_default(field) is None, (
                f"{field!r} is listed as required but _DEFAULTS supplies a value, "
                "which makes the required-field check unreachable"
            )

    def test_required_fields_are_exactly_the_two_without_defaults(self) -> None:
        assert _REQUIRED_FIELDS == ("version", "project.name")

    # -- the check actually fires ------------------------------------------

    def test_missing_version_in_file_raises(self, tmp_path: Path) -> None:
        yaml_file = tmp_path / "apcore.yaml"
        yaml_file.write_text("project:\n  name: test\n")
        with pytest.raises(ConfigError, match="Missing required field: 'version'") as exc_info:
            Config.load(str(yaml_file))
        assert exc_info.value.code == "CONFIG_INVALID"

    def test_missing_project_name_in_file_raises(self, tmp_path: Path) -> None:
        yaml_file = tmp_path / "apcore.yaml"
        yaml_file.write_text('version: "1.0.0"\n')
        with pytest.raises(ConfigError, match="Missing required field: 'project.name'") as exc_info:
            Config.load(str(yaml_file))
        assert exc_info.value.code == "CONFIG_INVALID"

    def test_empty_file_reports_both_missing_fields(self, tmp_path: Path) -> None:
        yaml_file = tmp_path / "apcore.yaml"
        yaml_file.write_text("{}\n")
        with pytest.raises(ConfigError) as exc_info:
            Config.load(str(yaml_file))
        assert exc_info.value.details["errors"] == [
            "Missing required field: 'version'",
            "Missing required field: 'project.name'",
        ]

    # -- and only fires for those two --------------------------------------

    def test_config_omitting_defaulted_keys_loads(self, tmp_path: Path) -> None:
        # extensions / schema / acl all carry canonical defaults, so omitting
        # them MUST NOT make the document invalid (§9.1). This passed before
        # too — but only because the merge had hidden the check entirely.
        yaml_file = tmp_path / "apcore.yaml"
        yaml_file.write_text('version: "1.0.0"\nproject:\n  name: test\n')
        config = Config.load(str(yaml_file))
        assert config.get("extensions.root") == "./extensions"
        assert config.get("schema.root") == "./schemas"
        assert config.get("acl.default_effect") == "deny"

    # -- requiredness is judged on the declared document -------------------

    def test_declared_excludes_merged_defaults(self, tmp_path: Path) -> None:
        yaml_file = tmp_path / "apcore.yaml"
        yaml_file.write_text('version: "1.0.0"\nproject:\n  name: test\n')
        config = Config.load(str(yaml_file))
        declared = config.declared
        assert declared == {"version": "1.0.0", "project": {"name": "test"}}
        # ...while the resolved view still carries the defaults.
        assert config.get("executor.default_timeout") == 30000

    def test_declared_is_a_copy(self, tmp_path: Path) -> None:
        yaml_file = tmp_path / "apcore.yaml"
        yaml_file.write_text('version: "1.0.0"\nproject:\n  name: test\n')
        config = Config.load(str(yaml_file))
        config.declared["version"] = "9.9.9"
        assert config.declared["version"] == "1.0.0"

    def test_defaults_cannot_satisfy_requiredness(self, tmp_path: Path) -> None:
        """The regression the old test could not catch.

        Simulates the pre-fix arrangement: put ``version``/``project.name``
        back into the default table and confirm the check still fails, because
        it reads the declared document rather than the merged one.
        """
        yaml_file = tmp_path / "apcore.yaml"
        yaml_file.write_text("extensions:\n  root: ./ext\n")
        patched = dict(_DEFAULTS)
        patched["version"] = "0.16.0"
        patched["project"] = {"name": "apcore"}
        with mock.patch.dict("apcore.config._DEFAULTS", patched, clear=True):
            with pytest.raises(ConfigError, match="Missing required field"):
                Config.load(str(yaml_file))

    def test_in_memory_config_declares_its_own_data(self) -> None:
        # Config(data=...) has no file behind it: the caller-supplied dict IS
        # the declared document, so validate() still guards it.
        config = Config(data={"version": "1.0.0", "project": {"name": "test"}})
        config.validate()
        with pytest.raises(ConfigError, match="Missing required field: 'project.name'"):
            Config(data={"version": "1.0.0"}).validate()


class TestDeclaredIncludesEnvOverrides:
    """PROTOCOL_SPEC §9.1 "What 'declared' means".

    The declared document is everything supplied by *someone* — file, env
    overrides, runtime set()/mount() — excluding only the default table.
    An env override therefore satisfies a required field. apcore-typescript
    and apcore-rust already behave this way; Python was the outlier.
    """

    def test_env_override_satisfies_a_required_field(self, tmp_path, monkeypatch):
        # File declares version but NOT project.name.
        cfg = tmp_path / "apcore.yaml"
        cfg.write_text('version: "1.0.0"\n')
        monkeypatch.setenv("APCORE_PROJECT_NAME", "from-env")

        config = Config.load(str(cfg))  # validate=True by default

        assert config.get("project.name") == "from-env"
        assert config.declared["project"]["name"] == "from-env"

    def test_missing_everywhere_still_fails(self, tmp_path, monkeypatch):
        cfg = tmp_path / "apcore.yaml"
        cfg.write_text('version: "1.0.0"\n')
        monkeypatch.delenv("APCORE_PROJECT_NAME", raising=False)

        with pytest.raises(ConfigError, match="project.name"):
            Config.load(str(cfg))

    def test_defaults_still_cannot_satisfy_requiredness(self, tmp_path, monkeypatch):
        """The env change must not have re-opened the defaults loophole."""
        cfg = tmp_path / "apcore.yaml"
        cfg.write_text('version: "1.0.0"\n')
        monkeypatch.delenv("APCORE_PROJECT_NAME", raising=False)
        monkeypatch.setitem(_DEFAULTS, "project", {"name": "from-defaults"})

        with pytest.raises(ConfigError, match="project.name"):
            Config.load(str(cfg))


class TestReloadHonoursOriginalValidateFlag:
    """PROTOCOL_SPEC §9.11 step 5.

    reload() re-validates only if the originating load() did. Re-imposing
    validation on a config deliberately loaded with validate=False would be a
    behaviour change disguised as a refresh.
    """

    def test_reload_does_not_validate_when_load_did_not(self, tmp_path):
        # A document that would fail validation: no project.name.
        cfg = tmp_path / "apcore.yaml"
        cfg.write_text('version: "1.0.0"\n')

        config = Config.load(str(cfg), validate=False)
        config.reload()  # must not raise

        assert config.get("version") == "1.0.0"

    def test_reload_validates_when_load_did(self, tmp_path):
        cfg = tmp_path / "apcore.yaml"
        cfg.write_text('version: "1.0.0"\nproject:\n  name: "ok"\n')
        config = Config.load(str(cfg))

        # Edit the file into an invalid state, then refresh.
        cfg.write_text('version: "1.0.0"\n')
        with pytest.raises(ConfigError, match="project.name"):
            config.reload()


class TestPathTypedConfigKeys:
    """PROTOCOL_SPEC §9.2.1 — the closed set of path-typed configuration keys."""

    EXPECTED = (
        "acl.root",
        "bindings.dir",
        "extensions.root",
        "extensions.roots[]",
        "id_map.overrides",
        "schema.root",
        "sys_modules.control.overrides_path",
    )

    def test_accessor_matches_declared_set_in_both_directions(self):
        actual = set(Config.path_typed_keys())
        expected = set(self.EXPECTED)
        assert actual - expected == set(), f"keys the SDK invented: {sorted(actual - expected)}"
        assert expected - actual == set(), f"keys the SDK is missing: {sorted(expected - actual)}"

    def test_bindings_pattern_is_not_path_typed(self):
        # Discriminating case. `bindings.pattern` sits in the same section as
        # `bindings.dir` and its default (`*.binding.yaml`) looks like a filename,
        # so an implementation that classifies by section — or by eye — sweeps it
        # in. It is a glob matched WITHIN `bindings.dir`, never resolved itself.
        assert "bindings.pattern" not in Config.path_typed_keys()

    def test_non_path_string_keys_are_not_path_typed(self):
        # An implementation that marks every string-valued key as path-typed
        # passes any presence-only assertion and fails here.
        for key in (
            "acl.default_effect",
            "schema.strategy",
            "logging.level",
            "observability.tracing.exporter",
            "project.name",
        ):
            assert key not in Config.path_typed_keys(), key

    def test_set_is_a_property_of_the_spec_not_of_a_document(self):
        # Same answer from a defaults-only Config and from an instance that
        # declares none of these keys.
        assert Config.path_typed_keys() == self.EXPECTED

    def test_every_scalar_path_typed_key_is_in_the_declared_key_surface(self):
        # `extensions.roots[]` is the element form of the list key
        # `extensions.roots`; strip the marker before checking membership.
        for key in Config.path_typed_keys():
            base = key[:-2] if key.endswith("[]") else key
            # The exempt keys have no default-table entry: `extensions.roots` is
            # list-valued, and the rest default to null (no directory, no map).
            assert Config.get_default(base, "__missing__") != "__missing__" or base in {
                "extensions.roots",
                "bindings.dir",
                "id_map.overrides",
                "sys_modules.control.overrides_path",
            }, base


class TestEmptyPathTypedValueIsDiscarded:
    """PROTOCOL_SPEC §9.2.1 requirement 5 — an empty string is not a path.

    §9.2 treats a *set but empty* ``APCORE_*`` variable as an override like any
    other, so ``export APCORE_ACL_ROOT=`` used to blank a directory the
    configuration file correctly declared — and because ``""`` is a legal
    relative path to the filesystem API, resolution then landed on the working
    directory rather than failing. Requirement 5 discards the empty value and
    falls through to the next tier, as if the variable had not been set.

    **What makes these discriminating.** Every case declares a *different*
    value at the tier below, so "fell through" and "was blanked" are two
    distinguishable answers rather than one absent one. A case whose lower tier
    held nothing would pass on an implementation that simply deleted the key.
    """

    SCALAR_KEYS = (
        "acl.root",
        "bindings.dir",
        "extensions.root",
        "id_map.overrides",
        "schema.root",
        "sys_modules.control.overrides_path",
    )

    ENV_VAR = {
        "acl.root": "APCORE_ACL_ROOT",
        "bindings.dir": "APCORE_BINDINGS_DIR",
        "extensions.root": "APCORE_EXTENSIONS_ROOT",
        "id_map.overrides": "APCORE_ID__MAP_OVERRIDES",
        "schema.root": "APCORE_SCHEMA_ROOT",
        "sys_modules.control.overrides_path": "APCORE_SYS__MODULES_CONTROL_OVERRIDES__PATH",
    }

    @staticmethod
    def _nested(key: str, value: Any) -> dict[str, Any]:
        """``"a.b.c", v`` -> ``{"a": {"b": {"c": v}}}``."""
        node: Any = value
        for part in reversed(key.split(".")):
            node = {part: node}
        return node  # type: ignore[no-any-return]

    @staticmethod
    @pytest.fixture(autouse=True)
    def _clean_env(monkeypatch: pytest.MonkeyPatch) -> None:
        import os as _os

        for name in [n for n in _os.environ if n.startswith("APCORE_")]:
            monkeypatch.delenv(name, raising=False)

    @staticmethod
    def _write(tmp_path: Path, document: dict[str, Any]) -> str:
        document = {"version": "1.0", "project": {"name": "empty-path"}, **document}
        path = tmp_path / "apcore.yaml"
        path.write_text(json.dumps(document), encoding="utf-8")
        return str(path)

    @pytest.mark.parametrize("key", SCALAR_KEYS)
    def test_empty_env_var_falls_through_to_the_file_value(
        self, key: str, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        """The tier below the variable is the configuration file, and it wins."""
        declared = f"./declared_{key.rsplit('.', 1)[-1]}"
        config_path = self._write(tmp_path, self._nested(key, declared))

        monkeypatch.setenv(self.ENV_VAR[key], "")
        config = Config.load(config_path, validate=False)

        assert config.get(key) == declared

    @pytest.mark.parametrize("key", SCALAR_KEYS)
    def test_empty_env_var_falls_through_to_the_default(
        self, key: str, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        """With nothing declared, the tier below the variable is ``_DEFAULTS``."""
        config_path = self._write(tmp_path, {})

        monkeypatch.setenv(self.ENV_VAR[key], "")
        config = Config.load(config_path, validate=False)

        assert config.get(key) == Config.get_default(key)
        assert config.get(key) != ""

    @pytest.mark.parametrize("key", SCALAR_KEYS)
    def test_empty_env_var_is_absent_from_the_declared_document(
        self, key: str, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        """ "As if the variable had been unset" reaches ``declared`` too.

        §9.1 counts an env override as a declaration, so a variable that was
        *not* discarded would show up here — and would then satisfy a required
        field with the empty string.
        """
        config_path = self._write(tmp_path, {})

        monkeypatch.setenv(self.ENV_VAR[key], "")
        config = Config.load(config_path, validate=False)

        *parents, leaf = key.split(".")
        node: Any = config.declared
        for part in parents:
            node = node.get(part, {}) if isinstance(node, dict) else {}
        assert leaf not in node

    @pytest.mark.parametrize("key", SCALAR_KEYS)
    def test_empty_env_var_does_not_resolve_to_the_working_directory(
        self, key: str, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        """The failure requirement 5 names: ``""`` resolving to CWD.

        Stated as its own assertion because it is the *consequence* operators
        actually meet — an unguarded implementation does not report an error, it
        silently scans the directory the process happens to be sitting in.
        """
        config_path = self._write(tmp_path, {})
        monkeypatch.chdir(tmp_path)

        monkeypatch.setenv(self.ENV_VAR[key], "")
        config = Config.load(config_path, validate=False)

        assert Path(str(config.get(key))).resolve() != Path.cwd().resolve()

    @pytest.mark.parametrize("key", SCALAR_KEYS)
    def test_empty_value_in_the_file_falls_through_to_the_default(self, key: str, tmp_path: Path) -> None:
        """Requirement 5 is about the value, not only about the variable."""
        config_path = self._write(tmp_path, self._nested(key, ""))

        config = Config.load(config_path, validate=False)

        assert config.get(key) == Config.get_default(key)

    @pytest.mark.parametrize("key", SCALAR_KEYS)
    def test_a_non_empty_env_var_still_overrides(
        self, key: str, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        """The guard is one value wide: ordinary §9.2 override behaviour is intact.

        Without this, "discard the empty string" and "ignore the variable
        entirely" are the same test result.
        """
        config_path = self._write(tmp_path, self._nested(key, "./declared"))

        monkeypatch.setenv(self.ENV_VAR[key], "./from_env")
        config = Config.load(config_path, validate=False)

        assert config.get(key) == "./from_env"

    def test_a_non_path_typed_key_may_still_be_set_empty(self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
        """Requirement 5 covers the path-typed set and nothing else.

        ``bindings.pattern`` sits in the same section as ``bindings.dir`` and is
        deliberately NOT path-typed (§9.2.1 requirement 4), so an implementation
        that guards by section rather than by the declared set fails here.
        """
        config_path = self._write(tmp_path, {})

        monkeypatch.setenv("APCORE_BINDINGS_PATTERN", "")
        monkeypatch.setenv("APCORE_SCHEMA_STRATEGY", "")
        config = Config.load(config_path, validate=False)

        assert config.get("bindings.pattern") == ""
        assert config.get("schema.strategy") == ""

    def test_empty_extensions_roots_elements_are_dropped(self, tmp_path: Path) -> None:
        """``extensions.roots[]`` is path-typed in both element forms.

        A list element has no tier to fall through to, so the available form of
        "MUST NOT use it as a directory" is to drop it. The non-empty siblings —
        one of each element form — must survive, or this passes on an
        implementation that discards the whole key.
        """
        config_path = self._write(
            tmp_path,
            {"extensions": {"roots": ["./kept", "", {"root": "", "namespace": "blank"}, {"root": "./also_kept"}]}},
        )

        config = Config.load(config_path, validate=False)

        assert config.get("extensions.roots") == ["./kept", {"root": "./also_kept"}]

    def test_the_guard_covers_every_scalar_path_typed_key(self) -> None:
        """The parametrisation above is the full §9.2.1 set, not a sample.

        A key added to ``path_typed_keys()`` without a case here is a silent
        gap: the fix would then cover four keys out of five and the suite would
        stay green.
        """
        scalar = {key for key in Config.path_typed_keys() if not key.endswith("[]")}
        assert scalar == set(self.SCALAR_KEYS)


# ---------------------------------------------------------------------------
# D-74 (spec v1.49.0) — `Config.get("")` is not an error
# ---------------------------------------------------------------------------


class TestEmptyKeyIsNotAnError:
    """The `Config.get` Inputs row said an empty key "is rejected with
    ``ValueError``/``ConfigInvalidError``". The same block's Errors row said
    "No errors raised under normal operation", and no SDK had ever rejected it
    — apcore-rust's ``get`` has no error channel at all. The clause described
    behaviour that never existed, and a conformance case written from it would
    have failed on all three. It was deleted; an empty key resolves no value
    and returns the default, like any other absent key.

    Two assertions, because "does not raise" alone is satisfied by a
    short-circuit ``if not key: return None`` — which would ignore a
    caller-supplied default and is NOT "like any other absent key". The second
    is what pins the decision's actual wording.
    """

    def test_an_empty_key_does_not_raise(self) -> None:
        config = Config({"a": {"b": 1}})
        assert config.get("") is None

    def test_an_empty_key_takes_the_ordinary_absent_key_path(self) -> None:
        config = Config({"a": {"b": 1}})
        assert config.get("", "SENTINEL") == "SENTINEL"

    def test_control_a_present_key_is_unaffected(self) -> None:
        """Without this, an implementation returning the default for EVERY key
        would satisfy both assertions above."""
        config = Config({"a": {"b": 1}})
        assert config.get("a.b") == 1
        assert config.get("a.b", "SENTINEL") == 1

    def test_control_an_absent_non_empty_key_behaves_identically(self) -> None:
        """ "Like any other absent key" is the decision's own wording, so the
        two paths are asserted to agree rather than each being checked alone."""
        config = Config({"a": {"b": 1}})
        assert config.get("") == config.get("no.such.key")
        assert config.get("", "SENTINEL") == config.get("no.such.key", "SENTINEL")
