"""Project root discovery and path consumers (PROTOCOL_SPEC §9.2.2)."""

from __future__ import annotations

import sys
import textwrap
from pathlib import Path

import pytest
import yaml

from apcore.acl import ACL
from apcore.config import Config
from apcore.errors import ConfigNotFoundError
from apcore.registry import Registry
from apcore.schema.loader import SchemaLoader

_ACL_POLICY = textwrap.dedent(
    """\
    default_effect: deny
    rules:
      - callers: ["*"]
        targets: ["greet"]
        effect: allow
        description: "Allow everything to reach greet"
    """
)


@pytest.fixture(autouse=True)
def _no_ambient_config(monkeypatch: pytest.MonkeyPatch) -> None:
    """§9.14 discovery must be decided by each case, not by the environment."""
    monkeypatch.delenv("APCORE_CONFIG_FILE", raising=False)


def _write_config(directory: Path, name: str, document: dict[str, object] | None = None) -> Path:
    directory.mkdir(parents=True, exist_ok=True)
    payload: dict[str, object] = document if document is not None else {}
    payload = {"version": "1.0", "project": {"name": "project-root"}, **payload}
    target = directory / name
    target.write_text(yaml.safe_dump(payload, sort_keys=False), encoding="utf-8")
    return target


def _fake_home(root: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """Point ``Path.home()`` at ``root`` so tiers 6-7 are reachable in a test."""
    monkeypatch.setattr("pathlib.Path.home", lambda: root)
    return root


def _user_config_dir(home: Path) -> Path:
    """The §9.14 tier 6 directory for this platform."""
    if sys.platform == "darwin":
        return home / "Library" / "Application Support" / "apcore"
    return home / ".config" / "apcore"


class TestProjectRootByDiscoveryTier:
    def test_tier1_env_config_file_outside_cwd_uses_the_files_directory(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        """Tier 1 is where a config file can sit outside CWD, and where it wins."""
        config_dir = tmp_path / "config-dir"
        run_dir = tmp_path / "run-dir"
        run_dir.mkdir()
        config_file = _write_config(config_dir, "apcore.yaml")
        monkeypatch.chdir(run_dir)
        monkeypatch.setenv("APCORE_CONFIG_FILE", str(config_file))

        config = Config.load(validate=False)

        assert config.project_root == str(config_dir.resolve())
        assert config.project_root != str(run_dir.resolve())

    def test_tier1_shape_also_covers_an_explicit_path_argument(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        """``Config.load(path)`` is tier-1 shaped: pointed at, from anywhere."""
        config_dir = tmp_path / "config-dir"
        run_dir = tmp_path / "run-dir"
        run_dir.mkdir()
        config_file = _write_config(config_dir, "apcore.yaml")
        monkeypatch.chdir(run_dir)

        config = Config.load(str(config_file), validate=False)

        assert config.project_root == str(config_dir.resolve())

    @pytest.mark.parametrize("filename", ["project.yaml", "project.yml", "apcore.yaml", "apcore.yml"])
    def test_tiers2_to_5_project_local_resolve_to_cwd(
        self, filename: str, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        """The indistinguishable majority: the file's directory *is* CWD."""
        _write_config(tmp_path, filename)
        monkeypatch.chdir(tmp_path)

        config = Config.load(validate=False)

        assert config.source_path == filename
        assert config.project_root == str(tmp_path.resolve())

    def test_tier6_user_level_xdg_config_resolves_to_cwd_not_the_config_dir(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        """The case that must resolve to CWD (apcore#113 "Why Option B was replaced").

        A user-level config is shared by every project its owner runs, so
        ``./acl`` written there cannot mean ``~/.config/apcore/acl``. A naive
        "directory of ``source_path``" gets exactly this case wrong — which is
        why the tier, not the source path, selects the base.
        """
        home = _fake_home(tmp_path / "home", monkeypatch)
        user_config_dir = _user_config_dir(home)
        _write_config(user_config_dir, "config.yaml")
        run_dir = tmp_path / "project"
        run_dir.mkdir()
        monkeypatch.chdir(run_dir)

        config = Config.load(validate=False)

        assert config.source_path is not None
        assert Path(config.source_path).parent == user_config_dir
        assert config.project_root == str(run_dir.resolve())
        assert config.project_root != str(user_config_dir.resolve())

    def test_tier7_legacy_user_level_config_resolves_to_cwd(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        home = _fake_home(tmp_path / "home", monkeypatch)
        legacy_dir = home / ".apcore"
        _write_config(legacy_dir, "config.yaml")
        run_dir = tmp_path / "project"
        run_dir.mkdir()
        monkeypatch.chdir(run_dir)

        config = Config.load(validate=False)

        assert config.source_path is not None
        assert Path(config.source_path).parent == legacy_dir
        assert config.project_root == str(run_dir.resolve())

    def test_no_config_file_found_resolves_to_cwd(self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
        _fake_home(tmp_path / "empty-home", monkeypatch)
        run_dir = tmp_path / "project"
        run_dir.mkdir()
        monkeypatch.chdir(run_dir)

        config = Config.load(validate=False)

        assert config.source_path is None
        assert config.project_root == str(run_dir.resolve())


class TestProjectRootWithoutABackingFile:
    def test_path_environment_values_keep_string_spelling(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        monkeypatch.setenv("APCORE_SCHEMA_ROOT", "123")
        monkeypatch.setenv("APCORE_SYS_CONTROL_OVERRIDES__PATH", "true")
        config_path = _write_config(tmp_path, "apcore.yaml", {"apcore": {"project": {"name": "paths"}}})
        config = Config.load(str(config_path), validate=False)
        assert config.get("schema.root") == "123"
        assert config.get("sys_modules.control.overrides_path") == "true"

    @pytest.mark.parametrize("suffix", ["/", "/."])
    def test_id_map_honors_directory_qualifier_on_file(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch, suffix: str
    ) -> None:
        path = tmp_path / "ids.yaml"
        path.write_text("mappings: []", encoding="utf-8")
        monkeypatch.chdir(tmp_path)
        config = Config(data={"id_map": {"overrides": "ids.yaml" + suffix}})
        with pytest.raises(ConfigNotFoundError):
            Registry(config=config)

    def test_in_memory_config_resolves_to_cwd(self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
        monkeypatch.chdir(tmp_path)

        config = Config(data={"schema": {"root": "./schemas"}})

        assert config.project_root == str(tmp_path.resolve())

    def test_from_defaults_resolves_to_cwd(self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
        monkeypatch.chdir(tmp_path)

        assert Config.from_defaults().project_root == str(tmp_path.resolve())

    def test_project_root_is_determined_once_and_survives_chdir(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        """§9.2.2 clause 4: a chdir between load and consumption must not move it.

        Re-reading CWD at each access is precisely what leaves two consumers of
        one ``Config`` looking at two directories.
        """
        first = tmp_path / "first"
        second = tmp_path / "second"
        first.mkdir()
        second.mkdir()
        _write_config(first, "apcore.yaml")
        monkeypatch.chdir(first)
        config = Config.load(validate=False)

        monkeypatch.chdir(second)

        assert config.project_root == str(first.resolve())

    def test_reload_keeps_the_tier_the_config_was_discovered_at(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        """Reload re-reads the document, not the discovery decision.

        A tier 6 config reloaded through ``Config.load(source_path)`` would be
        read as tier 1 and re-rooted onto the user-level directory.
        """
        home = _fake_home(tmp_path / "home", monkeypatch)
        user_config_dir = _user_config_dir(home)
        _write_config(user_config_dir, "config.yaml")
        run_dir = tmp_path / "project"
        run_dir.mkdir()
        monkeypatch.chdir(run_dir)
        config = Config.load(validate=False)

        config.reload()

        assert config.project_root == str(run_dir.resolve())


class TestProjectRootConsumers:
    """Configuration and consumers use the same root before and after loading."""

    def test_schema_validation_and_loading_share_project_root_after_chdir(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        config_dir = tmp_path / "config-dir"
        run_dir = tmp_path / "run-dir"
        run_dir.mkdir()
        (config_dir / "schemas").mkdir(parents=True)
        config_file = _write_config(
            config_dir, "apcore.yaml", {"schema": {"root": "./schemas", "strategy": "yaml_only"}}
        )
        monkeypatch.chdir(run_dir)
        config = Config.load(str(config_file))
        (run_dir / "schemas").mkdir()
        monkeypatch.chdir(tmp_path)
        config.reload()

        loader = SchemaLoader(config)

        assert loader._schemas_dir == (config_dir / "schemas").resolve()
        assert config.project_root == str(config_dir.resolve())

    def test_acl_discover_uses_project_root_for_explicit_file(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        config_dir = tmp_path / "config-dir"
        run_dir = tmp_path / "run-dir"
        run_dir.mkdir()
        (config_dir / "acl").mkdir(parents=True)
        (config_dir / "acl" / "global_acl.yaml").write_text(_ACL_POLICY, encoding="utf-8")
        config_file = _write_config(config_dir, "apcore.yaml", {"acl": {"root": "./acl"}})
        monkeypatch.chdir(run_dir)
        config = Config.load(str(config_file), validate=False)

        acl = ACL.discover(config)

        # Found next to the config file even though CWD has no ./acl at all.
        assert acl is not None
        assert acl.check(caller_id="anything", target_id="greet") is True

    def test_acl_discover_uses_project_root_for_user_file(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        home = _fake_home(tmp_path / "home", monkeypatch)
        user_config_dir = _user_config_dir(home)
        _write_config(user_config_dir, "config.yaml", {"acl": {"root": "./acl"}})
        run_dir = tmp_path / "project"
        (run_dir / "acl").mkdir(parents=True)
        (run_dir / "acl" / "global_acl.yaml").write_text(_ACL_POLICY, encoding="utf-8")
        (user_config_dir / "acl").mkdir()
        (user_config_dir / "acl" / "global_acl.yaml").write_text("default_effect: deny\nrules: []\n", encoding="utf-8")
        monkeypatch.chdir(run_dir)
        config = Config.load()
        monkeypatch.chdir(tmp_path)

        acl = ACL.discover(config)

        assert acl is not None
        assert acl.check(caller_id="anything", target_id="greet") is True

    def test_reload_follows_the_config_link_with_its_original_project_root(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        run_dir = tmp_path / "project"
        run_dir.mkdir()
        first = _write_config(tmp_path / "elsewhere", "first.yaml", {"schema": {"root": "./schemas"}})
        second = _write_config(tmp_path / "elsewhere", "second.yaml", {"schema": {"root": "./new-schemas"}})
        link = run_dir / "apcore.yaml"
        link.symlink_to(first)
        monkeypatch.chdir(run_dir)
        config = Config.load("apcore.yaml")

        link.unlink()
        link.symlink_to(second)
        monkeypatch.chdir(tmp_path)
        config.reload()

        assert config.source_path == "apcore.yaml"
        assert config.project_root == str(run_dir.resolve())
        assert SchemaLoader(config)._schemas_dir == run_dir.resolve() / "new-schemas"

    def test_programmatic_empty_paths_fall_through(self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
        monkeypatch.chdir(tmp_path)
        config = Config(data={"acl": {"root": ""}, "schema": {"root": ""}})
        (tmp_path / "global_acl.yaml").write_text(_ACL_POLICY, encoding="utf-8")

        assert ACL.discover(config) is None
        assert SchemaLoader(config)._schemas_dir == tmp_path.resolve() / "schemas"
        with pytest.raises(ValueError, match="empty configuration value"):
            config.resolve_path("")

        config.set("schema.root", "./declared")
        config.set("schema.root", "")
        assert SchemaLoader(config)._schemas_dir == tmp_path.resolve() / "declared"


class TestPathTypedKeySurface:
    def test_the_declared_path_typed_set(self) -> None:
        """§9.2.1's closed set is the input, not a second hand-maintained list."""
        assert Config.path_typed_keys() == (
            "acl.root",
            "bindings.dir",
            "extensions.root",
            "extensions.roots[]",
            "id_map.overrides",
            "schema.root",
            "sys_modules.control.overrides_path",
        )
