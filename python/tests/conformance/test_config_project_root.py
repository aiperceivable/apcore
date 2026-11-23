"""Cross-language driver for ``config_project_root.json``.

PROTOCOL_SPEC §9.2.2 (spec v1.35.0, apcore#113): every ``Config`` has exactly
one **project root**, determined at load — the directory containing the
configuration file when that file was selected by §9.14 discovery tiers 1-5
(``$APCORE_CONFIG_FILE`` or a project-local
``./project.yaml|.yml|apcore.yaml|.yml``), and the process CWD when it came from
the user-level tiers 6-7, when no file was found, or when the ``Config`` has no
backing file at all.

**The tier is the input under test.** That is why the fixture carries one case
per §9.14 tier and why the fixture's ``discovery_required`` clause insists every
tier case reach the SDK through ``Config.load()`` with **no path argument**:
handing the loader an explicit path replaces the thing being tested. Tiers 2-5
are indistinguishable from CWD and prove nothing on their own; tier 1 is where a
config file can sit outside CWD; tiers 6-7 are where a file-relative base would
be actively wrong, because ``extensions.root: ./extensions`` in a user-level
config shared by every project its owner runs cannot mean
``~/.config/apcore/extensions``.

``cwd_must_differ`` is honoured throughout: the process CWD is ``project/`` and
never the configuration file's directory in the cases where the two candidate
rules disagree. ``home_isolation`` is honoured by redirecting ``HOME`` /
``XDG_CONFIG_HOME`` / ``USERPROFILE`` at a fixture-owned tree — a driver that
read the real user's home would be testing the machine it runs on.
``env_isolation`` deletes every ``APCORE_*`` variable rather than blanking it,
because §9.2 makes an empty string a valid override: an inherited
``APCORE_CONFIG_FILE`` silently converts every other case into tier 1.

**Scope, and it is narrow on purpose.** v1.35.0 is the DEPRECATION phase. This
drives the ``project_root`` accessor (MUST, requirement 1) and the
deprecation-warning CONDITION (SHOULD, requirement 2). It must never grow
assertions that relative path-typed values *resolve* against ``project_root`` —
that is v2.0 behaviour, forbidden in the 1.x line by requirement 3, and
``v1x_current_bases_unchanged`` pins the opposite: ``acl.root`` still resolves
against the configuration file's directory (D-64) and ``schema.root`` still
against CWD.

**No divergence remains, and neither of the two was closed by changing this
SDK.** §9.14 tier 6 is platform-varying, and the fixture used to spell the POSIX
path, which failed every driver on macOS; it now names the TIER through
``<tier6_config>`` / ``<tier6_dir>`` tokens the driver materialises at the
running platform's location (see :func:`_tier6_dir`). And
``no_warning_when_all_path_values_absolute`` used to make only ``schema.root``
and ``acl.root`` absolute while §9.2.2's condition counts the §9.1.1 defaults
too, so ``extensions.root`` stayed relative and the case was unsatisfiable — all
three SDKs reported it; it now spells every key in §9.2.1's set.
"""

from __future__ import annotations

import os
import sys
import warnings
from pathlib import Path
from typing import Any

import pytest
import yaml

from apcore.acl import ACL
from apcore.config import Config
from apcore.schema import SchemaLoader

from .canonical_fixtures import case_ids, load_fixture, reject_unknown_expectations

FIXTURE = "config_project_root.json"

_FIXTURE = load_fixture(FIXTURE)
_CASES: dict[str, dict[str, Any]] = {case["id"]: case for case in _FIXTURE["test_cases"]}

#: The directory tree the tier cases share. ``project/`` is the process CWD
#: throughout; ``elsewhere/`` and the redirected home are the non-CWD locations
#: a configuration file can come from.
_LAYOUT: dict[str, Any] = _FIXTURE["layout"]

_ENV_CONFIG_FILE = "APCORE_CONFIG_FILE"


def _case(case_id: str) -> dict[str, Any]:
    assert case_id in _CASES, f"canonical fixture {FIXTURE} no longer defines case {case_id!r}"
    return _CASES[case_id]


# ---------------------------------------------------------------------------
# Layout, home redirection and environment isolation
# ---------------------------------------------------------------------------


def _tier6_dir(root: Path) -> Path:
    """The §9.14 tier-6 user-level directory under the redirected home.

    §9.14 tier 6 is explicitly platform-varying — ``~/.config/apcore`` on Linux,
    ``~/Library/Application Support/apcore`` on macOS — and this SDK implements
    exactly that split. The fixture therefore names the TIER and leaves the
    spelling to the driver: ``<tier6_config>`` is a token, and this is where it
    is resolved. Its first published form hardcoded the POSIX path, which failed
    every driver on macOS while asserting nothing extra on Linux.
    """
    home = _home(root)
    if sys.platform == "darwin":
        return home / "Library" / "Application Support" / "apcore"
    return home / ".config" / "apcore"


def _home(root: Path) -> Path:
    """The directory ``layout.home`` names, which ``$HOME`` is redirected to.

    Read from the fixture rather than spelled here: ``home_isolation`` requires
    a fixture-owned tree, and the fixture is the thing that owns the name.
    """
    return root / _LAYOUT["home"]


def _tier7_dir(root: Path) -> Path:
    """The §9.14 tier-7 legacy user-level directory; the same on every platform.

    Not platform-varying, but HOME-relative, which is why the fixture gives it
    the same token treatment as tier 6 rather than a path under the workspace.
    """
    return _home(root) / ".apcore"


#: ``fs`` tokens (``home_relative_tokens`` clause) and the directory each one's
#: file is materialised in. The filename is the tier's own, per §9.14.
_FS_TOKENS: dict[str, tuple[Any, str]] = {
    "<tier6_config>": (_tier6_dir, "config.yaml"),
    "<tier7_config>": (_tier7_dir, "config.yaml"),
}

#: ``expected`` tokens naming the directory a tier's file was materialised in.
_DIR_TOKENS: dict[str, Any] = {
    "<tier6_dir>": _tier6_dir,
    "<tier7_dir>": _tier7_dir,
}


def _fs_target(root: Path, relative: str) -> Path:
    """Map an ``fs`` key onto a real path, resolving a tier token if it is one."""
    if relative in _FS_TOKENS:
        directory, filename = _FS_TOKENS[relative]
        return directory(root) / filename
    return root / relative


def _expected_dir(root: Path, relative: str | None) -> Path | None:
    """Map an expectation's directory onto a resolved absolute path.

    ``comparison`` clause: resolved absolute, after symlink normalisation on
    both sides — macOS temporary directories are symlinked through ``/private``,
    which otherwise produces spurious inequality. A ``<tierN_dir>`` token
    resolves to the same location ``<tierN_config>`` was materialised at, which
    is the point of the pairing: asserting a literal spelling would be asserting
    the platform the suite happens to run on.
    """
    if relative is None:
        return None
    if relative in _DIR_TOKENS:
        return _DIR_TOKENS[relative](root).resolve()
    return (root / relative).resolve()


@pytest.fixture
def layout(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """Build the fixture's ``layout`` block, redirect ``$HOME``, clear ``APCORE_*``.

    Returns the layout root. The process CWD is set to ``layout.cwd``
    (``project/``), which is never the configuration file's directory in any
    case where the two candidate rules disagree — the ``cwd_must_differ``
    clause, and the same discriminating-case requirement as apcore#112.
    """
    root = tmp_path
    for relative in _LAYOUT["dirs"]:
        (root / relative).mkdir(parents=True, exist_ok=True)
    _tier6_dir(root).mkdir(parents=True, exist_ok=True)
    _tier7_dir(root).mkdir(parents=True, exist_ok=True)

    fake_home = _home(root)
    monkeypatch.setenv("HOME", str(fake_home))
    monkeypatch.setenv("USERPROFILE", str(fake_home))
    monkeypatch.setenv("XDG_CONFIG_HOME", str(fake_home / ".config"))

    for name in [n for n in os.environ if n.startswith("APCORE_")]:
        monkeypatch.delenv(name, raising=False)

    monkeypatch.chdir(root / _LAYOUT["cwd"])
    return root


def _apply(root: Path, case: dict[str, Any], monkeypatch: pytest.MonkeyPatch) -> None:
    """Write the case's ``fs`` block and apply its ``env`` block.

    ``$APCORE_CONFIG_FILE`` is absolutised against the layout root: the fixture
    states it as a layout-relative path (``elsewhere/apcore.yaml``) while the
    process CWD is ``project/``, so the literal value would name
    ``project/elsewhere/apcore.yaml``. Every other environment value is applied
    verbatim — ``APCORE_ACL_ROOT: "./x"`` is deliberately relative and is the
    whole point of the case that carries it.
    """
    for relative, content in (case.get("fs") or {}).items():
        path = _fs_target(root, relative)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")

    for name, value in (case.get("env") or {}).items():
        if name == _ENV_CONFIG_FILE:
            assert value in (
                case.get("fs") or {}
            ), f"case {case['id']!r} points $APCORE_CONFIG_FILE at {value!r}, which its fs block does not create"
            monkeypatch.setenv(name, str(_fs_target(root, value)))
        else:
            monkeypatch.setenv(name, value)


def _load_discovered() -> tuple[Config, list[warnings.WarningMessage]]:
    """``Config.load()`` with no path argument, capturing the warnings it emits.

    ``validate=False``: the fixture's config-file contents declare no
    ``version``, which §9.1's required-field check rejects, and the requirement
    under test is which directory the loaded ``Config`` calls its project root.
    ``validate`` is not a path argument, so ``discovery_required`` still holds —
    discovery is what selects the file.

    ``simplefilter("always")`` because Python's default filter deduplicates by
    (message, category, module, lineno): the second case to trigger the same
    warning line would otherwise observe nothing and read as a passing negative.
    """
    with warnings.catch_warnings(record=True) as captured:
        warnings.simplefilter("always")
        config = Config.load(validate=False)
    return config, list(captured)


def _source_dir(config: Config) -> Path | None:
    source = config.source_path
    return Path(source).resolve().parent if source is not None else None


def _deprecation_warnings(captured: list[warnings.WarningMessage]) -> list[warnings.WarningMessage]:
    """§9.2.2 requirement 2 warnings, by CATEGORY — the text is not normative."""
    return [entry for entry in captured if issubclass(entry.category, DeprecationWarning)]


def _relative_path_typed_values(config: Config) -> dict[str, str]:
    """Path-typed keys (§9.2.1) whose MERGED value is relative, and their values.

    The merged view, not the declared one: §9.2.2's target rule covers
    "file-declared, environment-sourced, API-supplied, and the §9.1.1 defaults
    alike", so a key nobody wrote is still affected while it carries a relative
    default. That reading is what made the all-absolute case unsatisfiable until
    the fixture named every key in the set.
    """
    found: dict[str, str] = {}
    for key in Config.path_typed_keys():
        if key.endswith("[]"):
            for element in config.get(key[:-2]) or []:
                candidate = element.get("root") if isinstance(element, dict) else element
                if isinstance(candidate, str) and candidate and not Path(candidate).is_absolute():
                    found[key] = candidate
                    break
            continue
        value = config.get(key)
        if isinstance(value, str) and value and not Path(value).is_absolute():
            found[key] = value
    return found


def _assert_root(root: Path, case: dict[str, Any], config: Config) -> None:
    """Assert ``project_root`` and, when stated, ``config_source_dir``.

    Both are asserted for tiers 6-7, and that is the ``tier_6_user_level_xdg``
    comment's explicit instruction: there the two DIFFER, so asserting only the
    source directory passes an implementation that never applies the tier split
    at all.
    """
    expected = case["expected"]

    assert Path(config.project_root).resolve() == _expected_dir(root, expected["project_root"])

    if "config_source_dir" in expected:
        assert _source_dir(config) == _expected_dir(root, expected["config_source_dir"])

    if "project_root_equals_cwd" in expected:
        equal = Path(config.project_root).resolve() == Path.cwd().resolve()
        assert equal is expected["project_root_equals_cwd"]


# ---------------------------------------------------------------------------
# One case per §9.14 discovery tier
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "case_id",
    [
        "tier_1_explicit_env_config_file",
        "tier_2_project_yaml",
        "tier_3_project_yml",
        "tier_4_apcore_yaml",
        "tier_5_apcore_yml",
        "tier_6_user_level_xdg",
        "tier_7_legacy_user_level",
        "no_config_file_found",
    ],
)
def test_project_root_by_discovery_tier(case_id: str, layout: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """§9.2.2's algorithm, one case per §9.14 tier, all reached through discovery.

    Tier 1 and tiers 6-7 are the cases that carry information: tier 1 is where
    the config file sits outside CWD and its own directory is the answer, tiers
    6-7 are where it sits outside CWD and CWD is the answer. Tiers 2-5 are
    driven because an implementation could get them wrong by getting the *tier
    detection* wrong, not because the two candidate rules differ there.
    """
    case = _case(case_id)
    reject_unknown_expectations(FIXTURE, case, {"expected"})
    assert case.get("cwd") == _LAYOUT["cwd"]

    _apply(layout, case, monkeypatch)
    config, _ = _load_discovered()

    _assert_root(layout, case, config)


def test_config_without_backing_file(layout: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """A ``Config`` with no source path: no discovery ran, so the root is CWD.

    Driven through both routes the case names — construction from an in-memory
    mapping, and ``from_defaults()`` — because they are two different ways to
    end up with no backing file and an implementation can set the root on only
    one of them.
    """
    case = _case("config_without_backing_file")
    reject_unknown_expectations(FIXTURE, case, {"expected"})

    _apply(layout, case, monkeypatch)

    for config in (Config(data=case["config_from_mapping"]), Config.from_defaults()):
        assert config.source_path is None
        _assert_root(layout, case, config)


# ---------------------------------------------------------------------------
# §9.2.2 requirement 2 — the NARROW deprecation warning
# ---------------------------------------------------------------------------


def test_deprecation_warning_fires_when_root_differs_and_value_is_relative(
    layout: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The positive half: root != CWD **and** a relative path-typed value present.

    This is the population whose resolution changes at v2.0 and the only
    population the warning is for.
    """
    case = _case("deprecation_warning_fires_when_root_differs_and_value_is_relative")
    reject_unknown_expectations(FIXTURE, case, {"expected"})

    _apply(layout, case, monkeypatch)
    config, captured = _load_discovered()

    _assert_root(layout, case, config)
    assert case["expected"]["relative_path_typed_values_present"] is True
    assert case["expected"]["deprecation_warning"] is True
    assert _deprecation_warnings(captured), "§9.2.2 requirement 2's positive condition emitted no warning"


def test_no_warning_when_root_equals_cwd(layout: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """First negative half: a project-local config with a relative ``schema.root``.

    ``project_root == CWD``, so nothing changes at v2.0. Warning here would fire
    for nearly every apcore project — the blanket warning §9.2.2 explicitly
    rejects, which trains operators to ignore the one warning that matters.
    """
    case = _case("no_warning_when_root_equals_cwd")
    reject_unknown_expectations(FIXTURE, case, {"expected"})

    _apply(layout, case, monkeypatch)
    config, captured = _load_discovered()

    _assert_root(layout, case, config)
    assert case["expected"]["relative_path_typed_values_present"] is True
    assert case["expected"]["deprecation_warning"] is False
    assert (
        _deprecation_warnings(captured) == []
    ), "a warning fired where project_root == CWD; this is the blanket warning §9.2.2 rejects"


def test_no_warning_when_all_path_values_absolute(layout: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """Second negative half: root != CWD, but every path-typed value is absolute.

    BOTH conditions are required. A driver that asserts only the first negative
    case passes an implementation that warns on the project-root difference
    alone.

    **Was a strict xfail, and the fixture is what changed.** The case's first
    published form spelled only ``schema.root`` and ``acl.root`` absolutely and
    was unsatisfiable for that reason: §9.2.2's target semantics count the
    §9.1.1 defaults as well as declared values, so ``extensions.root`` kept its
    relative ``./extensions`` and this SDK correctly warned. All three SDKs
    reported it independently. v1.36.0 spells every key in §9.2.1's set, and no
    SDK behaviour changed to close it.

    The companion assertion below is what makes this a check rather than a
    coincidence: every path-typed key really is absolute in the MERGED view, so
    the silence is the second condition failing and not the first.
    """
    case = _case("no_warning_when_all_path_values_absolute")
    reject_unknown_expectations(FIXTURE, case, {"expected"})

    _apply(layout, case, monkeypatch)
    config, captured = _load_discovered()

    _assert_root(layout, case, config)
    assert case["expected"]["relative_path_typed_values_present"] is False
    assert _relative_path_typed_values(config) == {}, (
        "a path-typed value is still relative in the merged view, so this case would be "
        "asserting the absence of a warning it has no right to expect"
    )
    assert case["expected"]["deprecation_warning"] is False
    assert _deprecation_warnings(captured) == []


def test_env_sourced_relative_value_counts_toward_the_warning(layout: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """The condition asks whether a relative value is PRESENT, not which tier supplied it.

    ``APCORE_ACL_ROOT=./x`` is exactly the population §9.2.2 changes, so an
    implementation that inspects only file-declared values misses the operators
    most likely to be affected.
    """
    case = _case("env_sourced_relative_value_counts_toward_the_warning")
    reject_unknown_expectations(FIXTURE, case, {"expected"})

    _apply(layout, case, monkeypatch)
    config, captured = _load_discovered()

    _assert_root(layout, case, config)
    # The config file declares no path-typed value at all; the relative one
    # arrived through the environment.
    assert config.get("acl.root") == case["env"]["APCORE_ACL_ROOT"]
    assert "acl" not in (yaml.safe_load(_fs_target(layout, "elsewhere/apcore.yaml").read_text()) or {})
    assert case["expected"]["relative_path_typed_values_present"] is True
    assert case["expected"]["deprecation_warning"] is True
    assert _deprecation_warnings(captured), "an env-sourced relative path-typed value did not reach the condition"


# ---------------------------------------------------------------------------
# §9.2.2 requirement 3 — the deprecation phase changes NO behaviour
# ---------------------------------------------------------------------------


def test_v1x_current_bases_unchanged(layout: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """``acl.root`` still resolves file-relative and ``schema.root`` still against CWD.

    §9.2.2 requirement 3 forbids adopting the target semantics in a 1.x release,
    and this is the case that pins it: an implementation that shipped the
    migration ahead of its window passes every accessor case above and fails
    here.

    **Making the case observable.** The fixture's ``fs`` block writes byte-
    identical ``global_acl.yaml`` files under ``elsewhere/acl/`` and
    ``project/acl/``, and an empty ``.keep`` under each ``schemas/``. Two
    identical files cannot tell you which one was read, so each tree's declared
    content is written verbatim *plus* an identifying payload: one ACL rule
    naming its own tree, and one loadable schema whose description names its own
    tree. ``default_effect: deny`` is preserved exactly as declared — the
    augmentation identifies the file, it does not change what the case asserts.
    """
    case = _case("v1x_current_bases_unchanged")
    reject_unknown_expectations(FIXTURE, case, {"expected"})

    _apply(layout, case, monkeypatch)

    for tree in ("elsewhere", "project"):
        acl_file = layout / tree / "acl" / "global_acl.yaml"
        declared = yaml.safe_load(acl_file.read_text()) or {}
        assert declared.get("default_effect") == "deny", "fixture no longer declares a default-deny ACL here"
        assert declared.get("rules") == [], "fixture ACL is no longer empty; re-read the case before marking it"
        declared["rules"] = [{"effect": "allow", "callers": [tree], "targets": ["*"]}]
        acl_file.write_text(yaml.safe_dump(declared, sort_keys=False), encoding="utf-8")

        schema_file = layout / tree / "schemas" / "probe.schema.yaml"
        schema_file.write_text(
            yaml.safe_dump(
                {
                    "description": tree,
                    "input_schema": {"type": "object"},
                    "output_schema": {"type": "object"},
                },
                sort_keys=False,
            ),
            encoding="utf-8",
        )

    config, _ = _load_discovered()

    assert Path(config.project_root).resolve() == _expected_dir(layout, case["expected"]["project_root"])

    acl = ACL.discover(config)
    assert acl is not None, "no ACL was discovered; acl.root did not resolve to either candidate tree"
    resolved_acl_tree = [rule.callers for rule in acl.rules]
    expected_acl_tree = case["expected"]["resolved_acl_root"].split("/")[0]
    assert resolved_acl_tree == [[expected_acl_tree]], (
        f"acl.root resolved against the wrong base: the loaded ACL came from "
        f"{resolved_acl_tree!r}, expected the tree under {case['expected']['resolved_acl_root']!r}"
    )

    expected_schema_tree = case["expected"]["resolved_schema_root"].split("/")[0]
    assert SchemaLoader(config).load("probe").description == expected_schema_tree, (
        f"schema.root resolved against the wrong base; expected the tree under "
        f"{case['expected']['resolved_schema_root']!r}"
    )

    # The two bases genuinely differ, which is the divergence §9.2.2 repairs at
    # v2.0. A case where they coincided would prove nothing.
    assert expected_acl_tree != expected_schema_tree


# ---------------------------------------------------------------------------
# Fixture/runner-contract cross-checks
# ---------------------------------------------------------------------------


def test_tier_cases_name_a_tier_and_never_a_spelling() -> None:
    """§9.14 tier 6 is platform-varying, so the fixture may not spell it.

    The ``home_relative_tokens`` clause: ``<tier6_config>`` / ``<tier7_config>``
    in ``fs`` and ``<tier6_dir>`` / ``<tier7_dir>`` in ``expected`` are tokens the
    driver resolves at the running platform's location. Pinned here because a
    fixture that reverted to a literal POSIX path would otherwise fail on macOS
    with a message about a missing directory rather than about the regression.
    """
    for case_id, config_token, dir_token in (
        ("tier_6_user_level_xdg", "<tier6_config>", "<tier6_dir>"),
        ("tier_7_legacy_user_level", "<tier7_config>", "<tier7_dir>"),
    ):
        case = _case(case_id)
        assert list(case["fs"]) == [config_token], f"{case_id} no longer names its config file by tier token"
        assert case["expected"]["config_source_dir"] == dir_token
        assert config_token in _FS_TOKENS
        assert dir_token in _DIR_TOKENS

    root = Path("/layout")
    home = root / _LAYOUT["home"]
    assert _tier7_dir(root) == home / ".apcore"
    if sys.platform == "darwin":
        assert _tier6_dir(root) == home / "Library" / "Application Support" / "apcore"
    else:
        assert _tier6_dir(root) == home / ".config" / "apcore"


def test_layout_names_the_home_to_redirect() -> None:
    """``layout.home`` is the fixture's, not this driver's.

    The tier-6/7 subdirectories are deliberately absent from ``layout.dirs``
    because tier 6's location is platform-varying; the driver creates them when
    it materialises the tokens. What the fixture does state is which directory
    ``$HOME`` points at, and reading it from there is what keeps the two in step.
    """
    assert _LAYOUT["home"] in _LAYOUT["dirs"]
    assert _home(Path("/layout")) == Path("/layout") / _LAYOUT["home"]


COVERED: dict[str, str] = {
    "tier_1_explicit_env_config_file": "test_project_root_by_discovery_tier",
    "tier_2_project_yaml": "test_project_root_by_discovery_tier",
    "tier_3_project_yml": "test_project_root_by_discovery_tier",
    "tier_4_apcore_yaml": "test_project_root_by_discovery_tier",
    "tier_5_apcore_yml": "test_project_root_by_discovery_tier",
    "tier_6_user_level_xdg": "test_project_root_by_discovery_tier",
    "tier_7_legacy_user_level": "test_project_root_by_discovery_tier",
    "no_config_file_found": "test_project_root_by_discovery_tier",
    "config_without_backing_file": "test_config_without_backing_file",
    "deprecation_warning_fires_when_root_differs_and_value_is_relative": (
        "test_deprecation_warning_fires_when_root_differs_and_value_is_relative"
    ),
    "no_warning_when_root_equals_cwd": "test_no_warning_when_root_equals_cwd",
    "no_warning_when_all_path_values_absolute": "test_no_warning_when_all_path_values_absolute",
    "env_sourced_relative_value_counts_toward_the_warning": (
        "test_env_sourced_relative_value_counts_toward_the_warning"
    ),
    "v1x_current_bases_unchanged": "test_v1x_current_bases_unchanged",
}


#: The canonical fixture's case count, asserted so that an upstream addition or
#: removal is a named failure rather than a quietly smaller run.
EXPECTED_CASE_COUNT = 14


def test_every_canonical_case_is_driven() -> None:
    """A case added upstream is a failure here, never a silent gap."""
    canonical = set(case_ids(FIXTURE))
    claimed = set(COVERED)
    assert canonical - claimed == set(), f"canonical fixture {FIXTURE} gained case(s) with no driver here"
    assert claimed - canonical == set(), f"this file claims case(s) {FIXTURE} no longer defines"
    assert len(canonical) == EXPECTED_CASE_COUNT


def test_every_claimed_driver_exists() -> None:
    module = sys.modules[__name__]
    missing = [name for name in COVERED.values() if not hasattr(module, name)]
    assert missing == [], f"claimed driver function(s) not defined: {missing}"


def test_driver_contract_is_not_a_case() -> None:
    """``driver_contract`` and ``layout`` are runner inputs, not test cases."""
    assert "driver_contract" in _FIXTURE
    assert "layout" in _FIXTURE
    ids = set(case_ids(FIXTURE))
    assert "driver_contract" not in ids
    assert "layout" not in ids
    assert {
        "accessor",
        "discovery_required",
        "cwd_must_differ",
        "home_isolation",
        "env_isolation",
        "comparison",
        "warning_observation",
        "home_relative_tokens",
    } <= set(_FIXTURE["driver_contract"])
