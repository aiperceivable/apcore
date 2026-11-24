"""Tests for the pluggable ``OverridesStore`` interface (cross-language alignment).

Mirrors apcore-typescript's ``OverridesStore`` / ``InMemoryOverridesStore``
/ ``FileOverridesStore`` contract. Validates:

1. ``InMemoryOverridesStore`` and ``FileOverridesStore`` both satisfy the
   :class:`OverridesStore` Protocol.
2. ``save()`` persists overrides; subsequent ``load()`` returns the same map.
3. ``register_sys_modules`` accepts an ``overrides_store=`` kwarg, applies
   loaded overrides to ``Config`` / ``ToggleState`` on startup, and persists
   subsequent ``update_config`` / ``toggle_feature`` calls back through the
   store.
4. The legacy ``overrides_path=`` kwarg path on ``UpdateConfigModule`` /
   ``ToggleFeatureModule`` continues to work (backwards-compat shim).
"""

from __future__ import annotations

import json
import logging
from pathlib import Path

import pytest
import yaml

from apcore.sys_modules import overrides as overrides_module

from apcore.config import Config
from apcore.executor import Executor
from apcore.registry.registry import Registry
from apcore.sys_modules.overrides import (
    FileOverridesStore,
    InMemoryOverridesStore,
    OverridesStore,
)
from apcore.sys_modules.registration import register_sys_modules


# ---------------------------------------------------------------------------
# Protocol satisfaction
# ---------------------------------------------------------------------------


class TestProtocol:
    def test_in_memory_satisfies_protocol(self) -> None:
        store = InMemoryOverridesStore()
        assert isinstance(store, OverridesStore)

    def test_file_satisfies_protocol(self, tmp_path: Path) -> None:
        store = FileOverridesStore(tmp_path / "overrides.yaml")
        assert isinstance(store, OverridesStore)


# ---------------------------------------------------------------------------
# InMemoryOverridesStore
# ---------------------------------------------------------------------------


class TestInMemoryStore:
    def test_load_returns_empty_dict_when_unset(self) -> None:
        store = InMemoryOverridesStore()
        assert store.load() == {}

    def test_save_then_load_returns_same_mapping(self) -> None:
        store = InMemoryOverridesStore()
        store.save({"a.b": 1, "toggle.x": True})
        assert store.load() == {"a.b": 1, "toggle.x": True}

    def test_load_returns_a_copy_so_callers_cannot_mutate_internal_state(self) -> None:
        store = InMemoryOverridesStore()
        store.save({"a.b": 1})
        snapshot = store.load()
        snapshot["a.b"] = 999  # type: ignore[index]
        assert store.load()["a.b"] == 1


# ---------------------------------------------------------------------------
# FileOverridesStore
# ---------------------------------------------------------------------------


class TestFileStore:
    def test_load_missing_file_returns_empty_dict(self, tmp_path: Path) -> None:
        store = FileOverridesStore(tmp_path / "missing.yaml")
        assert store.load() == {}

    def test_save_then_load_yaml_roundtrip(self, tmp_path: Path) -> None:
        path = tmp_path / "overrides.yaml"
        store = FileOverridesStore(path)
        store.save({"executor.default_timeout": 60000, "toggle.foo": False})
        assert path.exists()
        loaded = store.load()
        assert loaded == {"executor.default_timeout": 60000, "toggle.foo": False}

    def test_save_uses_yaml_format(self, tmp_path: Path) -> None:
        path = tmp_path / "overrides.yaml"
        store = FileOverridesStore(path)
        store.save({"k": "v"})
        # File must be parseable as YAML (which is also valid JSON for this case).
        with path.open() as f:
            assert yaml.safe_load(f) == {"k": "v"}

    def test_save_overwrites_existing_keys(self, tmp_path: Path) -> None:
        path = tmp_path / "overrides.yaml"
        store = FileOverridesStore(path)
        store.save({"k": 1})
        store.save({"k": 2, "j": 3})
        assert store.load() == {"k": 2, "j": 3}

    def test_load_falls_back_to_json_when_yaml_unavailable(self, tmp_path: Path) -> None:
        """If the file contains valid JSON, ``load()`` returns it.

        YAML is a superset of JSON, so a ``.yaml`` file containing JSON must
        round-trip correctly.
        """
        path = tmp_path / "overrides.yaml"
        path.write_text(json.dumps({"a.b": 1}), encoding="utf-8")
        store = FileOverridesStore(path)
        assert store.load() == {"a.b": 1}

    def test_path_property_exposes_the_underlying_file(self, tmp_path: Path) -> None:
        path = tmp_path / "overrides.yaml"
        store = FileOverridesStore(str(path))
        assert store.path == path
        assert isinstance(store.path, Path)


# ---------------------------------------------------------------------------
# FileOverridesStore — degradation paths
#
# Every branch below is documented by the store as "degrade, do not raise":
# an unreadable, corrupt, or unwritable overrides file must never take the
# process down, because the sys-modules apply overrides during startup.
# ---------------------------------------------------------------------------


class TestFileStoreLoadDegradation:
    def test_load_returns_empty_and_warns_when_read_raises_oserror(
        self,
        tmp_path: Path,
        monkeypatch: pytest.MonkeyPatch,
        caplog: pytest.LogCaptureFixture,
    ) -> None:
        """An unreadable (but existing) file degrades to ``{}`` with a warning."""
        path = tmp_path / "overrides.yaml"
        path.write_text("a.b: 1", encoding="utf-8")

        def _boom(self: Path, *args: object, **kwargs: object) -> str:
            raise OSError("permission denied")

        monkeypatch.setattr(Path, "read_text", _boom)
        store = FileOverridesStore(path)

        with caplog.at_level(logging.WARNING):
            assert store.load() == {}
        assert any("Failed to read overrides file" in msg for msg in caplog.messages)

    def test_load_returns_empty_for_an_empty_file(self, tmp_path: Path) -> None:
        path = tmp_path / "overrides.yaml"
        path.write_text("", encoding="utf-8")
        assert FileOverridesStore(path).load() == {}

    def test_load_returns_empty_when_yaml_parses_to_null(self, tmp_path: Path) -> None:
        """``null`` / a comment-only file parses to ``None``, not a mapping."""
        path = tmp_path / "overrides.yaml"
        path.write_text("# nothing here\n", encoding="utf-8")
        assert FileOverridesStore(path).load() == {}

    def test_load_falls_back_to_json_when_the_yaml_loader_raises(
        self,
        tmp_path: Path,
        monkeypatch: pytest.MonkeyPatch,
    ) -> None:
        """A YAMLError on JSON content falls through to ``json.loads``.

        The fallback exists for JSON edge cases the YAML loader rejects; it is
        forced here rather than hunted for, so the branch is covered without
        depending on a particular PyYAML version's edge-case behaviour.
        """
        path = tmp_path / "overrides.yaml"
        path.write_text(json.dumps({"a.b": 1}), encoding="utf-8")

        def _raise_yaml(*args: object, **kwargs: object) -> object:
            raise yaml.YAMLError("synthetic")

        monkeypatch.setattr(overrides_module.yaml, "safe_load", _raise_yaml)
        assert FileOverridesStore(path).load() == {"a.b": 1}

    def test_load_returns_empty_and_warns_when_neither_yaml_nor_json(
        self,
        tmp_path: Path,
        caplog: pytest.LogCaptureFixture,
    ) -> None:
        path = tmp_path / "overrides.yaml"
        path.write_text("a: b\n- c\n", encoding="utf-8")
        store = FileOverridesStore(path)

        with caplog.at_level(logging.WARNING):
            assert store.load() == {}
        assert any("not valid YAML/JSON" in msg for msg in caplog.messages)

    @pytest.mark.parametrize("content", ["- a\n- b\n", "42\n", "just a string\n"])
    def test_load_returns_empty_and_warns_when_root_is_not_a_mapping(
        self,
        tmp_path: Path,
        caplog: pytest.LogCaptureFixture,
        content: str,
    ) -> None:
        path = tmp_path / "overrides.yaml"
        path.write_text(content, encoding="utf-8")
        store = FileOverridesStore(path)

        with caplog.at_level(logging.WARNING):
            assert store.load() == {}
        assert any("root is not a mapping" in msg for msg in caplog.messages)


class TestFileStoreSaveDegradation:
    def test_save_returns_without_raising_when_mkdir_fails(
        self,
        tmp_path: Path,
        monkeypatch: pytest.MonkeyPatch,
        caplog: pytest.LogCaptureFixture,
    ) -> None:
        path = tmp_path / "nested" / "overrides.yaml"

        def _boom(self: Path, *args: object, **kwargs: object) -> None:
            raise OSError("read-only filesystem")

        monkeypatch.setattr(Path, "mkdir", _boom)
        store = FileOverridesStore(path)

        with caplog.at_level(logging.ERROR):
            store.save({"k": 1})  # must not raise
        assert not path.exists()
        assert any("Failed to create parent directory" in msg for msg in caplog.messages)

    def test_save_returns_without_raising_when_mkstemp_fails(
        self,
        tmp_path: Path,
        monkeypatch: pytest.MonkeyPatch,
        caplog: pytest.LogCaptureFixture,
    ) -> None:
        path = tmp_path / "overrides.yaml"

        def _boom(*args: object, **kwargs: object) -> tuple[int, str]:
            raise OSError("no space left on device")

        monkeypatch.setattr(overrides_module.tempfile, "mkstemp", _boom)
        store = FileOverridesStore(path)

        with caplog.at_level(logging.ERROR):
            store.save({"k": 1})  # must not raise
        assert not path.exists()
        assert any("Failed to create tempfile" in msg for msg in caplog.messages)

    def test_save_removes_the_tempfile_when_the_write_fails(
        self,
        tmp_path: Path,
        monkeypatch: pytest.MonkeyPatch,
        caplog: pytest.LogCaptureFixture,
    ) -> None:
        """A failing ``os.replace`` leaves neither a partial file nor a tempfile."""
        path = tmp_path / "overrides.yaml"

        def _boom(*args: object, **kwargs: object) -> None:
            raise OSError("cross-device link")

        monkeypatch.setattr(overrides_module.os, "replace", _boom)
        store = FileOverridesStore(path)

        with caplog.at_level(logging.ERROR):
            store.save({"k": 1})  # must not raise
        assert not path.exists()
        assert list(tmp_path.iterdir()) == []
        assert any("Failed to write overrides file" in msg for msg in caplog.messages)

    def test_save_degrades_when_a_value_is_not_yaml_serializable(
        self,
        tmp_path: Path,
        caplog: pytest.LogCaptureFixture,
    ) -> None:
        """``safe_dump`` raising a non-OSError still hits the cleanup path."""
        path = tmp_path / "overrides.yaml"
        store = FileOverridesStore(path)

        with caplog.at_level(logging.ERROR):
            store.save({"k": object()})  # must not raise
        assert not path.exists()
        assert list(tmp_path.iterdir()) == []
        assert any("Failed to write overrides file" in msg for msg in caplog.messages)

    def test_save_tolerates_a_failing_tempfile_cleanup(
        self,
        tmp_path: Path,
        monkeypatch: pytest.MonkeyPatch,
    ) -> None:
        """The ``unlink`` guarding the cleanup must not turn into a raise."""
        path = tmp_path / "overrides.yaml"

        def _boom_replace(*args: object, **kwargs: object) -> None:
            raise OSError("cross-device link")

        def _boom_unlink(*args: object, **kwargs: object) -> None:
            raise OSError("already gone")

        monkeypatch.setattr(overrides_module.os, "replace", _boom_replace)
        monkeypatch.setattr(overrides_module.os, "unlink", _boom_unlink)
        store = FileOverridesStore(path)

        store.save({"k": 1})  # must not raise
        assert not path.exists()


# ---------------------------------------------------------------------------
# Wiring through register_sys_modules
# ---------------------------------------------------------------------------


def _enable_sys_modules(config: Config) -> None:
    config.set("sys_modules.enabled", True)
    config.set("sys_modules.events.enabled", True)


class TestRegisterSysModulesWithStore:
    def test_overrides_store_kwarg_loads_existing_overrides_on_startup(self) -> None:
        """Pre-existing overrides in the store are applied to Config + ToggleState."""
        store = InMemoryOverridesStore()
        store.save(
            {
                "executor.default_timeout": 12345,
                "toggle.system.health.module": False,
            }
        )

        config = Config.from_defaults()
        _enable_sys_modules(config)
        registry = Registry()
        executor = Executor(registry=registry)

        ctx = register_sys_modules(
            registry=registry,
            executor=executor,
            config=config,
            overrides_store=store,
        )

        assert config.get("executor.default_timeout") == 12345
        # The toggle override must have been applied to the live ToggleState
        # owned by the toggle_feature sys-module.
        toggle_module = registry.get("system.control.toggle_feature")
        assert toggle_module is not None
        assert toggle_module._toggle_state.is_disabled("system.health.module") is True
        # Returned context dict shape — registration succeeded
        assert "event_emitter" in ctx

    def test_update_config_persists_through_store(self) -> None:
        """A successful update_config call writes back to the store."""
        store = InMemoryOverridesStore()
        config = Config.from_defaults()
        _enable_sys_modules(config)
        registry = Registry()
        executor = Executor(registry=registry)

        register_sys_modules(
            registry=registry,
            executor=executor,
            config=config,
            overrides_store=store,
        )

        update_mod = registry.get("system.control.update_config")
        assert update_mod is not None
        update_mod.execute(
            {
                "key": "executor.default_timeout",
                "value": 9999,
                "reason": "test persist",
            },
            None,
        )

        assert store.load()["executor.default_timeout"] == 9999

    def test_toggle_feature_persists_through_store(self) -> None:
        """A successful toggle_feature call writes back to the store."""
        store = InMemoryOverridesStore()
        config = Config.from_defaults()
        _enable_sys_modules(config)
        registry = Registry()
        executor = Executor(registry=registry)

        register_sys_modules(
            registry=registry,
            executor=executor,
            config=config,
            overrides_store=store,
        )

        toggle_mod = registry.get("system.control.toggle_feature")
        assert toggle_mod is not None
        toggle_mod.execute(
            {
                "module_id": "system.health.module",
                "enabled": False,
                "reason": "test persist",
            },
            None,
        )

        loaded = store.load()
        assert loaded["toggle.system.health.module"] is False

    def test_overrides_path_kwarg_back_compat_constructs_file_store(self, tmp_path: Path) -> None:
        """The legacy ``overrides_path`` kwarg still loads + persists via FileOverridesStore."""
        path = tmp_path / "overrides.yaml"
        # Pre-seed: existing override applied on startup.
        path.write_text(yaml.safe_dump({"executor.default_timeout": 7777}), encoding="utf-8")

        config = Config.from_defaults()
        _enable_sys_modules(config)
        config.set("sys_modules.control.overrides_path", str(path))
        registry = Registry()
        executor = Executor(registry=registry)

        register_sys_modules(
            registry=registry,
            executor=executor,
            config=config,
        )

        assert config.get("executor.default_timeout") == 7777

        # Subsequent update writes back to the same file.
        update_mod = registry.get("system.control.update_config")
        assert update_mod is not None
        update_mod.execute(
            {"key": "executor.default_timeout", "value": 8888, "reason": "shim"},
            None,
        )
        with path.open() as f:
            written = yaml.safe_load(f)
        assert written["executor.default_timeout"] == 8888


# ---------------------------------------------------------------------------
# Top-level re-exports
# ---------------------------------------------------------------------------


class TestPackageReExports:
    def test_overrides_classes_exported_from_apcore(self) -> None:
        import apcore

        assert apcore.OverridesStore is OverridesStore
        assert apcore.InMemoryOverridesStore is InMemoryOverridesStore
        assert apcore.FileOverridesStore is FileOverridesStore
