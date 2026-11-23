"""``Config.project_root`` and the §9.2.2 deprecation warning.

PROTOCOL_SPEC §9.2.2, tracking issue aiperceivable/apcore#113 (Option B').

Two sibling path keys resolve against two different bases today: ``acl.root``
against the config file's directory (D-64), ``schema.root`` against the process
CWD. §9.2.2 declares one project root for the whole ``Config`` and defers the
actual re-rooting to v2.0. **This is the deprecation phase**: the accessor is
additive and requirement 3 forbids adopting the target semantics in a 1.x
release, so ``TestResolutionBehaviourIsUnchanged`` below pins the divergence
rather than repairing it.

**What makes these tests discriminating.** The tier is what selects the base, so
every tier gets its own case and each one puts the config file somewhere the
wrong rule would be visible:

* tiers 2-5 are the indistinguishable majority — the file's directory *is* CWD —
  so they can only prove the *absence* of a warning, and that is what they assert;
* tier 1 places the file outside CWD, where "config dir" and "CWD" differ;
* tiers 6-7 place a user-level config outside CWD, which is the case that must
  resolve to CWD and where a naive "directory of source_path" gets it wrong.
"""

from __future__ import annotations

import subprocess
import sys
import textwrap
import warnings
from pathlib import Path

import pytest
import yaml

from apcore.acl import ACL
from apcore.config import Config
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


def _load_quietly(*args: object, **kwargs: object) -> Config:
    """``Config.load`` with §9.2.2's warning suppressed — for non-warning cases."""
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", DeprecationWarning)
        return Config.load(*args, **kwargs)  # type: ignore[arg-type]


def _deprecation_warnings(load: object) -> list[warnings.WarningMessage]:
    """Every ``DeprecationWarning`` raised while ``load()`` ran.

    ``"always"`` because these cases ask *whether the condition fired*, and
    ``"always"`` removes de-duplication from that question entirely. It is
    therefore the wrong filter for asking about CADENCE — see
    :class:`TestDeprecationWarningCadence`, which uses ``"default"`` precisely
    because that is the filter under which suppression is possible.
    """
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        load()  # type: ignore[operator]
    return [w for w in caught if issubclass(w.category, DeprecationWarning)]


def _load_through_a_shared_helper(path: str) -> Config:
    """A consumer's own ``load_config()`` wrapper — one call site, many callers.

    Deliberately a module-level function: ``warnings``' de-duplication is keyed
    on the *calling frame's* filename and line number, so every caller of a
    shared wrapper collides on one registry key. That is the ordinary shape of
    an application that loads configuration through a helper, and the shape in
    which the suppression §9.2.2 forbids actually bites.
    """
    return Config.load(path, validate=False)


def _deprecation_warnings_under_default_filter(load: object) -> list[warnings.WarningMessage]:
    """Warnings from ``load()`` under Python's ``"default"`` filter action.

    The ``"default"`` action is the one that de-duplicates: it records
    ``(message text, category, lineno)`` in the calling frame's
    ``__warningregistry__`` and drops a repeat. Every cadence assertion below
    runs under it, because under ``"always"`` there is nothing to detect.
    """
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("default")
        load()  # type: ignore[operator]
    return [w for w in caught if issubclass(w.category, DeprecationWarning)]


# ---------------------------------------------------------------------------
# §9.2.2 project_root(config), one case per §9.14 discovery tier
# ---------------------------------------------------------------------------


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

        config = _load_quietly(validate=False)

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

        config = _load_quietly(str(config_file), validate=False)

        assert config.project_root == str(config_dir.resolve())

    @pytest.mark.parametrize("filename", ["project.yaml", "project.yml", "apcore.yaml", "apcore.yml"])
    def test_tiers2_to_5_project_local_resolve_to_cwd(
        self, filename: str, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        """The indistinguishable majority: the file's directory *is* CWD."""
        _write_config(tmp_path, filename)
        monkeypatch.chdir(tmp_path)

        config = _load_quietly(validate=False)

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

        config = _load_quietly(validate=False)

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

        config = _load_quietly(validate=False)

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
        config = _load_quietly(validate=False)

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
        config = _load_quietly(validate=False)

        config.reload()

        assert config.project_root == str(run_dir.resolve())


# ---------------------------------------------------------------------------
# §9.2.2 requirement 2 — the narrow deprecation warning
# ---------------------------------------------------------------------------


class TestDeprecationWarningFires:
    def test_warns_when_project_root_differs_from_cwd_and_a_value_is_relative(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        """The population apcore#113 calls "the one genuine break"."""
        config_dir = tmp_path / "config-dir"
        run_dir = tmp_path / "run-dir"
        run_dir.mkdir()
        config_file = _write_config(config_dir, "apcore.yaml", {"schema": {"root": "./schemas"}})
        monkeypatch.chdir(run_dir)

        caught = _deprecation_warnings(lambda: Config.load(str(config_file), validate=False))

        assert len(caught) == 1
        message = str(caught[0].message)
        assert "schema.root" in message
        assert str(config_dir.resolve()) in message
        assert str(run_dir.resolve()) in message

    def test_warns_for_a_relative_default_nobody_wrote_down(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        """§9.2.2: "a relative default resolves against the project root exactly
        as an explicitly written value does". A document declaring no paths at
        all still re-roots at v2.0, because ``_DEFAULTS`` supplies ``./acl``,
        ``./schemas`` and ``./extensions``.
        """
        config_dir = tmp_path / "config-dir"
        run_dir = tmp_path / "run-dir"
        run_dir.mkdir()
        config_file = _write_config(config_dir, "apcore.yaml")
        monkeypatch.chdir(run_dir)

        caught = _deprecation_warnings(lambda: Config.load(str(config_file), validate=False))

        assert len(caught) == 1
        message = str(caught[0].message)
        for key in ("acl.root", "extensions.root", "schema.root"):
            assert key in message

    def test_warns_for_a_relative_element_of_extensions_roots(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        """``extensions.roots[]`` is list-valued; every element is path-typed."""
        config_dir = tmp_path / "config-dir"
        run_dir = tmp_path / "run-dir"
        run_dir.mkdir()
        config_file = _write_config(
            config_dir,
            "apcore.yaml",
            {
                "acl": {"root": str(tmp_path / "acl")},
                "schema": {"root": str(tmp_path / "schemas")},
                "extensions": {"roots": [{"root": "./plugins", "namespace": "p"}]},
            },
        )
        monkeypatch.chdir(run_dir)

        caught = _deprecation_warnings(lambda: Config.load(str(config_file), validate=False))

        assert len(caught) == 1
        assert "extensions.roots[]" in str(caught[0].message)


class TestDeprecationWarningStaysSilent:
    @pytest.mark.parametrize("filename", ["project.yaml", "project.yml", "apcore.yaml", "apcore.yml"])
    def test_silent_for_the_ordinary_project_local_case(
        self, filename: str, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        """Tiers 2-5: nothing changes at v2.0, so nothing may be said.

        apcore#113 rejects a blanket warning by name — firing here "trains
        operators to ignore the one warning that matters".
        """
        _write_config(tmp_path, filename, {"schema": {"root": "./schemas"}})
        monkeypatch.chdir(tmp_path)

        assert _deprecation_warnings(lambda: Config.load(validate=False)) == []

    def test_silent_for_a_user_level_config(self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
        """Tier 6 resolves to CWD, so its relative values are already correct."""
        home = _fake_home(tmp_path / "home", monkeypatch)
        _write_config(_user_config_dir(home), "config.yaml", {"schema": {"root": "./schemas"}})
        run_dir = tmp_path / "project"
        run_dir.mkdir()
        monkeypatch.chdir(run_dir)

        assert _deprecation_warnings(lambda: Config.load(validate=False)) == []

    def test_silent_when_every_path_typed_value_is_absolute(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        """Both conditions are required; an absolute value re-roots nowhere.

        EVERY key in §9.2.1's set has to be spelled absolutely, defaults
        included: the condition counts the merged view, so a key left unstated
        keeps its relative §9.1.1 default and the warning correctly fires.
        ``bindings.dir`` is here because spec v1.36.0 gave it such a default —
        before that it had none, and omitting it happened to be harmless.
        """
        config_dir = tmp_path / "config-dir"
        run_dir = tmp_path / "run-dir"
        run_dir.mkdir()
        config_file = _write_config(
            config_dir,
            "apcore.yaml",
            {
                "acl": {"root": str(tmp_path / "acl")},
                "schema": {"root": str(tmp_path / "schemas")},
                "extensions": {"root": str(tmp_path / "extensions")},
                "bindings": {"dir": str(tmp_path / "bindings")},
            },
        )
        monkeypatch.chdir(run_dir)

        assert _deprecation_warnings(lambda: Config.load(str(config_file), validate=False)) == []

    def test_silent_for_a_config_with_no_backing_file(self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
        monkeypatch.chdir(tmp_path)

        assert _deprecation_warnings(lambda: Config(data={"schema": {"root": "./schemas"}})) == []
        assert _deprecation_warnings(Config.from_defaults) == []


# ---------------------------------------------------------------------------
# §9.2.2 requirement 2 — cadence: once per configuration LOAD
# ---------------------------------------------------------------------------


class TestDeprecationWarningCadence:
    """The warning is a property of the document, not of the process.

    §9.2.2 requirement 2, "Cadence: once per configuration load, not once per
    process": every load satisfying both conditions emits, and implementations
    **MUST NOT** suppress it with process-global state. A once-per-process flag
    makes emission order-dependent — whichever load runs first consumes the
    warning, so a later affected configuration is silent and the operator cannot
    tell which document triggered it — and is a test-isolation hazard besides.

    **This SDK never wrote such a flag, and was non-conforming anyway.**
    ``warnings.warn`` carries the state on the SDK's behalf: under the
    ``"default"`` action it records ``(message text, category, lineno)`` in the
    calling frame's ``__warningregistry__``.

    **Which is why the obvious test does not detect it.** Two *different*
    affected configurations produce two *different* message texts, because the
    message embeds the project root and the working directory — so they land on
    different registry keys and both warn, before and after the fix.
    ``test_two_different_configurations_both_warn`` is kept because §9.2.2 asks
    for it, but it is not the discriminating case. What was suppressed is the
    case the section names in the same breath: the same document loaded again,
    "a reload that re-reads an edited file", which SHOULD re-evaluate.
    """

    @staticmethod
    def _affected(tmp_path: Path, name: str) -> Path:
        """A tier-1 config outside CWD holding a relative path-typed value."""
        return _write_config(tmp_path / name, "apcore.yaml", {"schema": {"root": "./schemas"}})

    def test_two_different_configurations_both_warn(self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
        """§9.2.2's own example: two affected documents, one process, two warnings.

        Under a once-per-process flag the second is silent and the operator
        cannot tell which document is affected.
        """
        first = self._affected(tmp_path, "first")
        second = self._affected(tmp_path, "second")
        run_dir = tmp_path / "run-dir"
        run_dir.mkdir()
        monkeypatch.chdir(run_dir)

        caught = _deprecation_warnings_under_default_filter(
            lambda: [Config.load(str(path), validate=False) for path in (first, second)]
        )

        assert len(caught) == 2

    def test_the_same_configuration_loaded_twice_warns_twice(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        """DISCRIMINATING. One document, one call site, two loads, two warnings.

        This is the shape ``warnings``' own de-duplication suppressed: identical
        message text raised from the same source line, which is one
        ``__warningregistry__`` key. It is also exactly the case §9.2.2 calls
        out as legitimate — "a reload that re-reads an edited file SHOULD
        re-evaluate and may legitimately warn again".
        """
        config_file = self._affected(tmp_path, "config-dir")
        run_dir = tmp_path / "run-dir"
        run_dir.mkdir()
        monkeypatch.chdir(run_dir)

        caught = _deprecation_warnings_under_default_filter(
            lambda: [Config.load(str(config_file), validate=False) for _ in range(3)]
        )

        assert len(caught) == 3

    def test_a_shared_load_helper_does_not_consume_its_next_caller_s_warning(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        """DISCRIMINATING. Two callers, one wrapper, one document, two warnings.

        The test-isolation hazard §9.2.2 names, in its production form: an
        application that loads configuration through its own ``load_config()``
        wrapper puts every load on one source line, so the first caller consumed
        the warning for all of them.
        """
        config_file = self._affected(tmp_path, "config-dir")
        run_dir = tmp_path / "run-dir"
        run_dir.mkdir()
        monkeypatch.chdir(run_dir)

        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("default")
            _load_through_a_shared_helper(str(config_file))
            first = len([w for w in caught if issubclass(w.category, DeprecationWarning)])
            _load_through_a_shared_helper(str(config_file))
            both = len([w for w in caught if issubclass(w.category, DeprecationWarning)])

        assert first == 1
        assert both == 2, "the second caller of a shared load helper was silenced"

    def test_a_reload_of_an_edited_file_warns_again(self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
        """Re-reading a document the operator edited re-evaluates the condition.

        The file is rewritten between loads with a *different* relative key, so
        this also pins that the warning reports the merged document as it now
        stands rather than a memo of the first load.
        """
        config_dir = tmp_path / "config-dir"
        run_dir = tmp_path / "run-dir"
        run_dir.mkdir()
        config_file = _write_config(config_dir, "apcore.yaml", {"schema": {"root": "./schemas"}})
        monkeypatch.chdir(run_dir)

        first = _deprecation_warnings_under_default_filter(lambda: Config.load(str(config_file), validate=False))
        _write_config(config_dir, "apcore.yaml", {"acl": {"root": "./acl"}})
        second = _deprecation_warnings_under_default_filter(lambda: Config.load(str(config_file), validate=False))

        assert len(first) == 1
        assert len(second) == 1

    def test_emission_is_not_order_dependent(self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
        """An unaffected load first must not consume the affected load's warning.

        The failure mode a once-per-process flag produces is silence for
        *whichever* affected document happens to run second, so the reverse
        order has to hold as well.
        """
        run_dir = tmp_path / "run-dir"
        run_dir.mkdir()
        unaffected = _write_config(run_dir, "apcore.yaml", {"schema": {"root": "./schemas"}})
        affected = self._affected(tmp_path, "config-dir")
        monkeypatch.chdir(run_dir)

        forwards = _deprecation_warnings_under_default_filter(
            lambda: [Config.load(str(path), validate=False) for path in (unaffected, affected)]
        )
        backwards = _deprecation_warnings_under_default_filter(
            lambda: [Config.load(str(path), validate=False) for path in (affected, unaffected)]
        )

        assert len(forwards) == 1
        assert len(backwards) == 1

    def test_the_host_filters_are_still_the_host_s(self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
        """Cadence is fixed without taking the operator's filter choice away.

        The fix must not reach for ``simplefilter`` on the global filter list:
        an SDK that forced its warning through would override ``-W ignore``,
        and one that mutated filters at import time would change the behaviour
        of warnings it has nothing to do with. So: ``ignore`` still silences it,
        ``error`` still promotes it, and importing this package leaves the
        filter list alone.
        """
        config_file = self._affected(tmp_path, "config-dir")
        run_dir = tmp_path / "run-dir"
        run_dir.mkdir()
        monkeypatch.chdir(run_dir)

        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("ignore", DeprecationWarning)
            Config.load(str(config_file), validate=False)
            Config.load(str(config_file), validate=False)
        assert [w for w in caught if issubclass(w.category, DeprecationWarning)] == []

        with warnings.catch_warnings():
            warnings.simplefilter("error", DeprecationWarning)
            with pytest.raises(DeprecationWarning):
                Config.load(str(config_file), validate=False)

        # Import-time filter mutation, checked in a subprocess: reloading
        # ``apcore.config`` in-process would rebind ``Config`` and the global
        # namespace registry underneath every other module that already holds
        # them, which is a far bigger test-isolation hazard than the one under
        # test here.
        probe = subprocess.run(
            [
                sys.executable,
                "-c",
                "import warnings, copy;"
                "before = copy.copy(warnings.filters);"
                "import apcore;"
                "print(before == warnings.filters)",
            ],
            capture_output=True,
            text=True,
            check=True,
        )
        assert probe.stdout.strip() == "True", "importing apcore mutated the process-wide warning filters"

    def test_no_module_level_suppression_state(self) -> None:
        """No process-global flag exists to be consumed, whatever the mechanism.

        The registry ``warnings`` keeps is not the SDK's to delete, but a flag
        of the SDK's own would be — and would reintroduce the defect on the
        first refactor. Named directly so a future "warn once" boolean is a
        failing test rather than a regression nobody notices.
        """
        import apcore.config as config_module

        suspects = [
            name
            for name, value in vars(config_module).items()
            if isinstance(value, (bool, set))
            and ("warn" in name.lower() or "deprecat" in name.lower() or "emitted" in name.lower())
        ]
        assert suspects == []


# ---------------------------------------------------------------------------
# §9.2.2 requirement 3 — no v1.x behaviour change
# ---------------------------------------------------------------------------


class TestResolutionBehaviourIsUnchanged:
    """The accessor is informational. Nothing resolves against it yet.

    These pin the *divergence* §9.2.2 exists to repair at v2.0, so that adopting
    the target semantics early is a test failure rather than a silent re-rooting.
    """

    def test_schema_loader_still_resolves_schema_root_against_cwd(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        config_dir = tmp_path / "config-dir"
        run_dir = tmp_path / "run-dir"
        run_dir.mkdir()
        (config_dir / "schemas").mkdir(parents=True)
        (run_dir / "schemas").mkdir()
        config_file = _write_config(config_dir, "apcore.yaml", {"schema": {"root": "./schemas"}})
        monkeypatch.chdir(run_dir)
        config = _load_quietly(str(config_file), validate=False)

        loader = SchemaLoader(config)

        assert loader._schemas_dir == (run_dir / "schemas").resolve()
        assert config.project_root == str(config_dir.resolve())

    def test_acl_discover_still_resolves_acl_root_against_the_config_file(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        """D-64's behaviour, including the tier-6 bug §9.2.2 defers fixing."""
        config_dir = tmp_path / "config-dir"
        run_dir = tmp_path / "run-dir"
        run_dir.mkdir()
        (config_dir / "acl").mkdir(parents=True)
        (config_dir / "acl" / "global_acl.yaml").write_text(_ACL_POLICY, encoding="utf-8")
        config_file = _write_config(config_dir, "apcore.yaml", {"acl": {"root": "./acl"}})
        monkeypatch.chdir(run_dir)
        config = _load_quietly(str(config_file), validate=False)

        acl = ACL.discover(config)

        # Found next to the config file even though CWD has no ./acl at all.
        assert acl is not None
        assert acl.check(caller_id="anything", target_id="greet") is True


class TestPathTypedKeySurface:
    def test_the_warning_checks_exactly_the_declared_path_typed_set(self) -> None:
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
