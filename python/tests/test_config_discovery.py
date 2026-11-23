"""Tests for §9.14 Config Discovery (discover_config_file and Config.load without path)."""

from __future__ import annotations

from pathlib import Path

import pytest
import yaml

from apcore.config import Config, discover_config_file


class TestDiscoverConfigFile:
    def test_env_var_takes_priority(self, monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
        config_file = tmp_path / "custom.yaml"
        config_file.write_text("version: '0.8.0'\n")
        monkeypatch.setenv("APCORE_CONFIG_FILE", str(config_file))
        result = discover_config_file()
        assert result == str(config_file)

    def test_returns_none_when_no_file_found(self, monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
        monkeypatch.delenv("APCORE_CONFIG_FILE", raising=False)
        monkeypatch.chdir(tmp_path)
        monkeypatch.setattr("pathlib.Path.home", lambda: tmp_path / "nonexistent_home")
        result = discover_config_file()
        assert result is None

    def test_project_yaml_found_in_cwd(self, monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
        monkeypatch.delenv("APCORE_CONFIG_FILE", raising=False)
        monkeypatch.chdir(tmp_path)
        project_yaml = tmp_path / "project.yaml"
        project_yaml.write_text("version: '0.8.0'\n")
        result = discover_config_file()
        assert result == "project.yaml"

    def test_project_yml_found_in_cwd(self, monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
        monkeypatch.delenv("APCORE_CONFIG_FILE", raising=False)
        monkeypatch.chdir(tmp_path)
        (tmp_path / "project.yml").write_text("version: '0.8.0'\n")
        result = discover_config_file()
        assert result == "project.yml"

    def test_apcore_yaml_found_in_cwd(self, monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
        monkeypatch.delenv("APCORE_CONFIG_FILE", raising=False)
        monkeypatch.chdir(tmp_path)
        (tmp_path / "apcore.yaml").write_text("version: '0.8.0'\n")
        result = discover_config_file()
        assert result == "apcore.yaml"

    def test_apcore_yml_found_in_cwd(self, monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
        monkeypatch.delenv("APCORE_CONFIG_FILE", raising=False)
        monkeypatch.chdir(tmp_path)
        (tmp_path / "apcore.yml").write_text("version: '0.8.0'\n")
        result = discover_config_file()
        assert result == "apcore.yml"

    def test_xdg_config_found(self, monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
        import sys

        monkeypatch.delenv("APCORE_CONFIG_FILE", raising=False)
        monkeypatch.chdir(tmp_path)
        fake_home = tmp_path / "home"

        if sys.platform == "darwin":
            xdg_dir = fake_home / "Library" / "Application Support" / "apcore"
        else:
            xdg_dir = fake_home / ".config" / "apcore"

        xdg_dir.mkdir(parents=True)
        (xdg_dir / "config.yaml").write_text("version: '0.8.0'\n")
        monkeypatch.setattr("pathlib.Path.home", lambda: fake_home)
        result = discover_config_file()
        assert result is not None
        assert result.endswith("config.yaml")

    def test_legacy_config_found(self, monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
        import sys

        monkeypatch.delenv("APCORE_CONFIG_FILE", raising=False)
        monkeypatch.chdir(tmp_path)
        fake_home = tmp_path / "home"
        legacy_dir = fake_home / ".apcore"
        legacy_dir.mkdir(parents=True)
        (legacy_dir / "config.yaml").write_text("version: '0.8.0'\n")

        # Make XDG path not exist
        if sys.platform == "darwin":
            xdg_path = fake_home / "Library" / "Application Support" / "apcore" / "config.yaml"
        else:
            xdg_path = fake_home / ".config" / "apcore" / "config.yaml"
        assert not xdg_path.exists()

        monkeypatch.setattr("pathlib.Path.home", lambda: fake_home)
        result = discover_config_file()
        assert result is not None
        assert ".apcore" in result

    def test_env_var_takes_priority_over_cwd_file(self, monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
        env_file = tmp_path / "env_config.yaml"
        env_file.write_text("version: '0.8.0'\n")
        monkeypatch.setenv("APCORE_CONFIG_FILE", str(env_file))
        monkeypatch.chdir(tmp_path)
        (tmp_path / "project.yaml").write_text("version: '0.8.0'\n")
        result = discover_config_file()
        assert result == str(env_file)


class TestConfigLoad:
    def _make_valid_yaml(self, path: Path) -> None:
        data = {
            "version": "0.8.0",
            "extensions": {"root": "./extensions", "auto_discover": False},
            "schema": {"root": "./schemas"},
            "acl": {"root": "./acl", "default_effect": "deny"},
            "project": {"name": "test"},
        }
        path.write_text(yaml.dump(data))

    def test_load_without_path_returns_defaults_when_no_file(
        self, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
    ) -> None:
        monkeypatch.delenv("APCORE_CONFIG_FILE", raising=False)
        monkeypatch.chdir(tmp_path)
        monkeypatch.setattr("pathlib.Path.home", lambda: tmp_path / "nonexistent_home")
        config = Config.load()
        assert config is not None
        assert isinstance(config, Config)

    def test_load_without_path_loads_found_file(self, monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
        monkeypatch.delenv("APCORE_CONFIG_FILE", raising=False)
        monkeypatch.chdir(tmp_path)
        config_file = tmp_path / "project.yaml"
        self._make_valid_yaml(config_file)
        config = Config.load()
        assert config.get("project.name") == "test"

    def test_load_with_env_var_loads_that_file(self, monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
        config_file = tmp_path / "custom.yaml"
        self._make_valid_yaml(config_file)
        monkeypatch.setenv("APCORE_CONFIG_FILE", str(config_file))
        monkeypatch.chdir(tmp_path)
        config = Config.load()
        assert config.get("project.name") == "test"

    def test_load_with_explicit_path_still_works(self, tmp_path: Path) -> None:
        config_file = tmp_path / "explicit.yaml"
        self._make_valid_yaml(config_file)
        config = Config.load(str(config_file))
        assert config.get("project.name") == "test"


class TestConfigFileEnvVarIsNotAnOverride:
    """apcore#88: ``$APCORE_CONFIG_FILE`` selects the document, it is not in it.

    §9.2 turns every ``APCORE_*`` variable into a configuration override, so the
    file selector used to lower to the dot-path ``config.file`` and land in the
    *declared* document — the view §9.1's required-field check runs against.
    ``config.file`` is declared by no schema
    (``conformance/fixtures/config_key_governance.json``).

    These tests assert the **exact** declared key set, not merely that
    ``config.file`` is gone: absence alone would also hold for an
    implementation that dropped a key the file really does declare.
    """

    @staticmethod
    def _flatten(data: dict[str, object], prefix: str = "") -> set[str]:
        paths: set[str] = set()
        for key, value in data.items():
            path = f"{prefix}{key}"
            if isinstance(value, dict) and value:
                paths |= TestConfigFileEnvVarIsNotAnOverride._flatten(value, f"{path}.")
            else:
                paths.add(path)
        return paths

    @staticmethod
    def _clear_apcore_env(monkeypatch: pytest.MonkeyPatch) -> None:
        """Remove every ``APCORE_*`` variable a developer's shell may carry."""
        import os

        for name in list(os.environ):
            if name.startswith("APCORE_"):
                monkeypatch.delenv(name, raising=False)

    def test_legacy_mode_declares_exactly_the_file_keys(self, monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
        self._clear_apcore_env(monkeypatch)
        config_file = tmp_path / "custom.yaml"
        config_file.write_text("version: '1.0.0'\nproject:\n  name: demo\n")
        monkeypatch.setenv("APCORE_CONFIG_FILE", str(config_file))

        config = Config.load(str(config_file))

        assert self._flatten(config._declared) == {"version", "project.name"}

    def test_namespace_mode_declares_exactly_the_file_keys(
        self, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
    ) -> None:
        self._clear_apcore_env(monkeypatch)
        config_file = tmp_path / "ns.yaml"
        config_file.write_text("apcore:\n  version: '1.0.0'\n  project:\n    name: demo\n")
        monkeypatch.setenv("APCORE_CONFIG_FILE", str(config_file))

        config = Config.load(str(config_file))

        assert self._flatten(config._declared) == {"apcore.version", "apcore.project.name"}

    def test_real_overrides_still_reach_the_declared_document(
        self, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
    ) -> None:
        """The exemption is one variable wide — ``bindings.dir`` is a declared
        key, so ``APCORE_BINDINGS_DIR`` is §9.2 working as designed."""
        self._clear_apcore_env(monkeypatch)
        config_file = tmp_path / "custom.yaml"
        config_file.write_text("version: '1.0.0'\nproject:\n  name: demo\n")
        monkeypatch.setenv("APCORE_CONFIG_FILE", str(config_file))
        monkeypatch.setenv("APCORE_BINDINGS_DIR", "./generated")

        config = Config.load(str(config_file))

        assert self._flatten(config._declared) == {"version", "project.name", "bindings.dir"}
