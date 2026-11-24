"""``bindings.dir`` / ``bindings.pattern`` resolution in the binding loader.

PROTOCOL_SPEC §5.12.6, tracking issue aiperceivable/apcore#114.

Before this, ``bindings.dir`` was registered in the §9.1.1 key surface and read
by no code path in any SDK: setting it in ``apcore.yaml`` produced no scan, and
the section's MUST had no subject. §5.12.6 now names one — a loader invoked
*without* an explicit directory resolves from ``bindings.dir`` through §9.2's
precedence chain.

**What makes these tests discriminating.** Every pre-existing
``load_binding_dir`` test in ``tests/test_bindings.py`` passes an explicit
directory, which is the one path that behaves identically before and after the
fix. So each test here that matters plants a *decoy* ``./bindings`` directory in
the working directory holding a different binding file, and asserts on which
module id ended up in the registry. A loader that ignores ``config`` falls
through to the ``./bindings`` default and picks up the decoy, which is a
distinguishable wrong answer rather than a silent pass.
"""

from __future__ import annotations

import os
from pathlib import Path

import pytest
import yaml

from apcore.bindings import BindingLoader
from apcore.client import APCore
from apcore.config import Config
from apcore.errors import BindingFileInvalidError
from apcore.registry import Registry

_BINDING_TEMPLATE = (
    "bindings:\n  - module_id: {module_id}\n    target: binding_helpers:typed_function\n    auto_schema: true\n"
)


@pytest.fixture
def loader() -> BindingLoader:
    return BindingLoader()


@pytest.fixture
def registry() -> Registry:
    return Registry()


@pytest.fixture(autouse=True)
def _clean_binding_env(monkeypatch: pytest.MonkeyPatch) -> None:
    """No ambient ``APCORE_*`` state may decide any case in this file."""
    monkeypatch.delenv("APCORE_BINDINGS_DIR", raising=False)
    monkeypatch.delenv("APCORE_BINDINGS_PATTERN", raising=False)
    monkeypatch.delenv("APCORE_CONFIG_FILE", raising=False)


def _write_binding(directory: Path, filename: str, module_id: str) -> Path:
    directory.mkdir(parents=True, exist_ok=True)
    target = directory / filename
    target.write_text(_BINDING_TEMPLATE.format(module_id=module_id), encoding="utf-8")
    return target


def _write_config(directory: Path, bindings_section: dict[str, str] | None) -> Path:
    document: dict[str, object] = {"version": "1.0", "project": {"name": "bindings-dir"}}
    if bindings_section is not None:
        document["bindings"] = bindings_section
    config_path = directory / "apcore.yaml"
    config_path.write_text(yaml.safe_dump(document, sort_keys=False), encoding="utf-8")
    return config_path


def _plant_decoy(cwd: Path) -> None:
    """A ``./bindings`` directory in CWD holding a module nothing should load.

    This is what turns "the loader ignored the config" from an invisible pass
    into a failing assertion.
    """
    _write_binding(cwd / "bindings", "decoy.binding.yaml", "decoy.func")


class TestBindingsDirFromConfig:
    """§5.12.6 clause 1 — resolve the directory from ``bindings.dir``."""

    def test_config_file_dir_is_scanned_with_env_unset(
        self,
        loader: BindingLoader,
        registry: Registry,
        tmp_path: Path,
        monkeypatch: pytest.MonkeyPatch,
    ) -> None:
        """THE discriminating case (apcore#114 "Conformance").

        ``bindings.dir`` set in a config *file*, ``APCORE_BINDINGS_DIR`` unset,
        no explicit directory argument — a scan must happen, and it must be of
        the configured directory rather than the ``./bindings`` decoy sitting in
        the working directory.
        """
        monkeypatch.chdir(tmp_path)
        _plant_decoy(tmp_path)
        configured = tmp_path / "configured"
        _write_binding(configured, "real.binding.yaml", "real.func")
        config = Config.load(str(_write_config(tmp_path, {"dir": str(configured)})), validate=False)

        result = loader.load_binding_dir(registry=registry, config=config)

        assert [fm.module_id for fm in result] == ["real.func"]
        assert registry.get("real.func") is not None
        assert "decoy.func" not in registry.list()

    def test_relative_config_dir_resolves_against_cwd(
        self,
        loader: BindingLoader,
        registry: Registry,
        tmp_path: Path,
        monkeypatch: pytest.MonkeyPatch,
    ) -> None:
        """A relative ``bindings.dir`` keeps v1.x semantics: CWD-relative.

        §9.2.2 records ``bindings.dir`` as having had no consumer before
        v1.35.0, and its requirement 3 forbids adopting the project-root base in
        a 1.x release. So this must NOT consult ``config.project_root``, even
        though the config file lives elsewhere.
        """
        project = tmp_path / "project"
        project.mkdir()
        monkeypatch.chdir(project)
        _write_binding(project / "custom", "here.binding.yaml", "cwd.func")

        config_dir = tmp_path / "elsewhere"
        config_dir.mkdir()
        _write_binding(config_dir / "custom", "there.binding.yaml", "configdir.func")
        config = Config.load(str(_write_config(config_dir, {"dir": "./custom"})), validate=False)

        result = loader.load_binding_dir(registry=registry, config=config)

        assert [fm.module_id for fm in result] == ["cwd.func"]

    def test_missing_configured_dir_raises(
        self,
        loader: BindingLoader,
        registry: Registry,
        tmp_path: Path,
        monkeypatch: pytest.MonkeyPatch,
    ) -> None:
        """A configured directory that does not exist is an error, not a no-op."""
        monkeypatch.chdir(tmp_path)
        config = Config.load(
            str(_write_config(tmp_path, {"dir": str(tmp_path / "absent")})),
            validate=False,
        )

        with pytest.raises(BindingFileInvalidError) as exc:
            loader.load_binding_dir(registry=registry, config=config)

        assert str(tmp_path / "absent") in str(exc.value)


class TestMissingResolvedDirRaisesForEveryProvenance:
    """§5.12.6 clause 5, on all three provenances the clause names.

    "This holds whether the directory came from an explicit argument, from
    ``bindings.dir``, or from the ``./bindings`` default." A test that covered
    only the configured directory would pass an implementation that checked for
    existence inside the config branch alone — and the default branch is the one
    that fires for a project which never configured anything, which is exactly
    the population that would otherwise get a silent empty result.

    The message MUST name the *resolved* directory, so each case asserts the
    directory the loader worked out rather than the input it was handed.
    """

    def test_explicit_argument(self, loader: BindingLoader, registry: Registry, tmp_path: Path) -> None:
        absent = tmp_path / "absent_argument"

        with pytest.raises(BindingFileInvalidError) as exc:
            loader.load_binding_dir(str(absent), registry)

        assert str(absent) in str(exc.value)
        assert exc.value.code == "BINDING_FILE_INVALID"

    def test_bindings_dir_from_the_config_file(
        self,
        loader: BindingLoader,
        registry: Registry,
        tmp_path: Path,
        monkeypatch: pytest.MonkeyPatch,
    ) -> None:
        monkeypatch.chdir(tmp_path)
        config = Config.load(str(_write_config(tmp_path, {"dir": "./absent_from_file"})), validate=False)

        with pytest.raises(BindingFileInvalidError) as exc:
            loader.load_binding_dir(registry=registry, config=config)

        # The value as resolved, which is what clause 5 asks the message to name.
        assert "./absent_from_file" in str(exc.value)
        assert exc.value.code == "BINDING_FILE_INVALID"

    def test_bindings_dir_from_the_environment(
        self,
        loader: BindingLoader,
        registry: Registry,
        tmp_path: Path,
        monkeypatch: pytest.MonkeyPatch,
    ) -> None:
        monkeypatch.chdir(tmp_path)
        monkeypatch.setenv("APCORE_BINDINGS_DIR", "./absent_from_env")
        config = Config.load(str(_write_config(tmp_path, {"dir": "./absent_from_file"})), validate=False)

        with pytest.raises(BindingFileInvalidError) as exc:
            loader.load_binding_dir(registry=registry, config=config)

        assert "./absent_from_env" in str(exc.value)
        assert "./absent_from_file" not in str(
            exc.value
        ), "the message named the file tier, so the env tier never reached the loader"

    def test_the_bindings_default(
        self,
        loader: BindingLoader,
        registry: Registry,
        tmp_path: Path,
        monkeypatch: pytest.MonkeyPatch,
    ) -> None:
        """Nothing configured, no ``./bindings`` on disk: still an error.

        The population that would otherwise silently get an empty result.
        """
        monkeypatch.chdir(tmp_path)
        assert not (tmp_path / "bindings").exists()

        with pytest.raises(BindingFileInvalidError) as exc:
            loader.load_binding_dir(registry=registry)

        assert Config.get_default("bindings.dir") in str(exc.value)
        assert exc.value.code == "BINDING_FILE_INVALID"

    def test_the_bindings_default_reached_through_a_config(
        self,
        loader: BindingLoader,
        registry: Registry,
        tmp_path: Path,
        monkeypatch: pytest.MonkeyPatch,
    ) -> None:
        """The same default, now arriving through ``Config`` rather than the loader.

        Since spec v1.36.0 there are two routes to ``./bindings`` — the merged
        default table and the loader's own last tier — and clause 5 has to hold
        on both.
        """
        monkeypatch.chdir(tmp_path)
        config = Config.load(str(_write_config(tmp_path, None)), validate=False)
        assert config.get("bindings.dir") == "./bindings"

        with pytest.raises(BindingFileInvalidError) as exc:
            loader.load_binding_dir(registry=registry, config=config)

        assert "./bindings" in str(exc.value)


class TestBindingsPatternFromConfig:
    """§5.12.6 clause 1 — ``bindings.pattern`` travels the same chain."""

    def test_config_file_pattern_is_applied(
        self,
        loader: BindingLoader,
        registry: Registry,
        tmp_path: Path,
        monkeypatch: pytest.MonkeyPatch,
    ) -> None:
        monkeypatch.chdir(tmp_path)
        scanned = tmp_path / "scanned"
        _write_binding(scanned, "wanted.custom.yaml", "wanted.func")
        _write_binding(scanned, "ignored.binding.yaml", "ignored.func")
        config = Config.load(
            str(_write_config(tmp_path, {"dir": str(scanned), "pattern": "*.custom.yaml"})),
            validate=False,
        )

        result = loader.load_binding_dir(registry=registry, config=config)

        assert [fm.module_id for fm in result] == ["wanted.func"]

    def test_explicit_pattern_beats_config(
        self,
        loader: BindingLoader,
        registry: Registry,
        tmp_path: Path,
        monkeypatch: pytest.MonkeyPatch,
    ) -> None:
        monkeypatch.chdir(tmp_path)
        scanned = tmp_path / "scanned"
        _write_binding(scanned, "wanted.custom.yaml", "wanted.func")
        _write_binding(scanned, "other.binding.yaml", "other.func")
        config = Config.load(
            str(_write_config(tmp_path, {"dir": str(scanned), "pattern": "*.custom.yaml"})),
            validate=False,
        )

        result = loader.load_binding_dir(None, registry, "*.binding.yaml", config=config)

        assert [fm.module_id for fm in result] == ["other.func"]


class TestPrecedence:
    """§5.12.6 clause 2 — explicit argument > env > file > default."""

    def test_explicit_dir_beats_config(
        self,
        loader: BindingLoader,
        registry: Registry,
        tmp_path: Path,
        monkeypatch: pytest.MonkeyPatch,
    ) -> None:
        monkeypatch.chdir(tmp_path)
        explicit = tmp_path / "explicit"
        _write_binding(explicit, "a.binding.yaml", "explicit.func")
        configured = tmp_path / "configured"
        _write_binding(configured, "b.binding.yaml", "configured.func")
        config = Config.load(str(_write_config(tmp_path, {"dir": str(configured)})), validate=False)

        result = loader.load_binding_dir(str(explicit), registry, config=config)

        assert [fm.module_id for fm in result] == ["explicit.func"]

    def test_env_beats_config_file_through_the_config_object(
        self,
        loader: BindingLoader,
        registry: Registry,
        tmp_path: Path,
        monkeypatch: pytest.MonkeyPatch,
    ) -> None:
        """The env tier arrives through §9.2's override pass, not a loader read."""
        monkeypatch.chdir(tmp_path)
        from_env = tmp_path / "from_env"
        _write_binding(from_env, "a.binding.yaml", "env.func")
        from_file = tmp_path / "from_file"
        _write_binding(from_file, "b.binding.yaml", "file.func")
        monkeypatch.setenv("APCORE_BINDINGS_DIR", str(from_env))
        config = Config.load(str(_write_config(tmp_path, {"dir": str(from_file)})), validate=False)

        result = loader.load_binding_dir(registry=registry, config=config)

        assert [fm.module_id for fm in result] == ["env.func"]

    def test_loader_does_not_read_the_env_var_itself(
        self,
        loader: BindingLoader,
        registry: Registry,
        tmp_path: Path,
        monkeypatch: pytest.MonkeyPatch,
    ) -> None:
        """§5.12.6 clause 2: ``MUST NOT`` read ``APCORE_BINDINGS_DIR`` at the loader.

        With no ``Config`` there is no §9.2 chain to carry the variable, so the
        loader must fall through to ``./bindings`` and load the decoy — not
        reach into ``os.environ`` behind the configuration system's back. This
        is the check that keeps Python from re-introducing the raw
        ``process.env`` read TypeScript is dropping (apcore-typescript#36).
        """
        monkeypatch.chdir(tmp_path)
        _plant_decoy(tmp_path)
        from_env = tmp_path / "from_env"
        _write_binding(from_env, "a.binding.yaml", "env.func")
        monkeypatch.setenv("APCORE_BINDINGS_DIR", str(from_env))

        result = loader.load_binding_dir(registry=registry)

        assert [fm.module_id for fm in result] == ["decoy.func"]

    def test_default_dir_and_pattern_when_nothing_configured(
        self,
        loader: BindingLoader,
        registry: Registry,
        tmp_path: Path,
        monkeypatch: pytest.MonkeyPatch,
    ) -> None:
        """Last tier of both chains: ``./bindings`` and ``*.binding.yaml``.

        Spec v1.36.0 moved these two values into
        ``schemas/defaults.schema.json``, which ``_DEFAULTS`` mirrors, so
        ``Config`` now answers them. Through v1.35.0 it did not, and this test
        asserted the opposite — that ``bindings.dir`` had *no* default — because
        the loader was the only place the canonical value existed. That
        assertion is now wrong, and the corrected one is what the fixture
        ``config_key_governance.json`` pins from the other side.

        The loader keeps its own last tier regardless, and this case still
        exercises it: ``config`` is omitted here, so nothing ever consults a
        merged document.
        """
        monkeypatch.chdir(tmp_path)
        _write_binding(tmp_path / "bindings", "a.binding.yaml", "default.func")
        _write_binding(tmp_path / "bindings", "b.other.yaml", "unmatched.func")

        assert Config.get_default("bindings.dir") == "./bindings"
        assert Config.get_default("bindings.pattern") == "*.binding.yaml"
        assert Config.from_defaults().get("bindings.dir") == "./bindings"
        assert Config.from_defaults().get("bindings.pattern") == "*.binding.yaml"

        result = loader.load_binding_dir(registry=registry)

        assert [fm.module_id for fm in result] == ["default.func"]

    def test_config_without_bindings_section_falls_through_to_default(
        self,
        loader: BindingLoader,
        registry: Registry,
        tmp_path: Path,
        monkeypatch: pytest.MonkeyPatch,
    ) -> None:
        monkeypatch.chdir(tmp_path)
        _write_binding(tmp_path / "bindings", "a.binding.yaml", "default.func")
        config = Config.load(str(_write_config(tmp_path, None)), validate=False)

        result = loader.load_binding_dir(registry=registry, config=config)

        assert [fm.module_id for fm in result] == ["default.func"]


class TestBackwardCompatibility:
    """Every pre-existing call shape must behave exactly as before."""

    def test_positional_dir_and_registry_unchanged(
        self,
        loader: BindingLoader,
        registry: Registry,
        tmp_path: Path,
    ) -> None:
        _write_binding(tmp_path, "a.binding.yaml", "positional.func")

        result = loader.load_binding_dir(str(tmp_path), registry)

        assert [fm.module_id for fm in result] == ["positional.func"]

    def test_positional_pattern_unchanged(
        self,
        loader: BindingLoader,
        registry: Registry,
        tmp_path: Path,
    ) -> None:
        _write_binding(tmp_path, "a.custom.yaml", "positional.func")

        result = loader.load_binding_dir(str(tmp_path), registry, "*.custom.yaml")

        assert [fm.module_id for fm in result] == ["positional.func"]

    def test_registry_is_still_required(self, loader: BindingLoader, tmp_path: Path) -> None:
        with pytest.raises(TypeError, match="registry"):
            loader.load_binding_dir(str(tmp_path))


class TestNoImplicitStartupScan:
    """§5.12.6 clause 3 — ``MUST NOT`` scan at client initialisation."""

    def test_apcore_client_does_not_scan_bindings_dir(
        self,
        tmp_path: Path,
        monkeypatch: pytest.MonkeyPatch,
    ) -> None:
        monkeypatch.chdir(tmp_path)
        configured = tmp_path / "configured"
        _write_binding(configured, "a.binding.yaml", "startup.func")
        _plant_decoy(tmp_path)
        config = Config.load(str(_write_config(tmp_path, {"dir": str(configured)})), validate=False)

        client = APCore(config=config)

        modules = client.registry.list()
        assert "startup.func" not in modules
        assert "decoy.func" not in modules

    def test_bindings_dir_scan_adds_no_startup_filesystem_io(
        self,
        tmp_path: Path,
        monkeypatch: pytest.MonkeyPatch,
    ) -> None:
        """A ``./bindings`` directory that merely exists changes nothing.

        This is the behaviour change apcore#114 explicitly refused: an implicit
        startup scan would alter every deployment that happens to have one.
        """
        monkeypatch.chdir(tmp_path)
        _plant_decoy(tmp_path)

        client = APCore(config=Config.load(str(_write_config(tmp_path, None)), validate=False))

        assert "decoy.func" not in client.registry.list()
        assert (Path(os.getcwd()) / "bindings").is_dir()
