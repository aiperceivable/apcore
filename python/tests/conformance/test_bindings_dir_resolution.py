"""Cross-language driver for ``bindings_dir_resolution.json``.

PROTOCOL_SPEC §5.12.6 (spec v1.35.0, corrected v1.36.0, apcore#114): a binding
loader invoked **without an explicit directory argument** resolves the scan
directory from ``bindings.dir`` under §9.2 precedence (``APCORE_BINDINGS_DIR`` >
config file > default ``./bindings``), matches candidates against
``bindings.pattern`` through the same chain (default ``*.binding.yaml``), yields
to an explicit argument when one is given, **MUST NOT** be triggered
automatically at client or framework initialisation, and **MUST** raise rather
than return empty when the resolved directory does not exist (clause 5).

**The defect this pins is a MUST with no subject.** Through v1.34.0 the section
read "if ``bindings.dir`` is configured, implementations MUST scan files
matching ``pattern`` in that directory" and never said *who* scans or *when*. No
SDK satisfied it: ``bindings.dir`` was registered in all three key surfaces and
read by no code path, while ``BindingLoader`` was exported public API in all
three and called from no internal one. v1.35.0 names the subject (the loader)
and the trigger (an invocation that does not name a directory), which is what
makes the requirement testable at all.

**What makes a case here discriminating, and why the pre-existing tests are
not.** Every ``load_binding_dir`` test that predates apcore#114 passes an
explicit directory, and that path behaves identically before and after the fix —
so it proves nothing. The fixture's ``no_explicit_argument`` clause is therefore
load-bearing: cases with ``explicit_dir: null`` invoke the loader with the
directory argument genuinely *absent*, never with a directory this file computed
for it. ``config_construction`` is load-bearing for the same reason: each case's
``config_file`` block is written to a real file on disk and loaded through
``Config``, because the FILE tier is the one that was broken, and an in-memory
mapping bypasses it.

``env_isolation`` is honoured by :func:`_isolated_environment`, which *deletes*
every ``APCORE_*`` variable rather than blanking it. That was already true for
the reason §9.2 gives — a set-but-empty variable is still an override — and
since §9.2.1 requirement 5 it is true for a second reason: this SDK now
*discards* an empty path-typed override, so blanking would neither shadow the
file nor stand aside in the way the old comment described. Deleting is the one
form that means "not set" under both readings.

The loader is driven through ``BindingLoader.load_binding_dir``, the public
entry point an application calls. The fixture's ``entry_point`` clause forbids
reaching into a private resolution helper: §5.12.6's subject is the loader *as
invoked*, and a helper that computes the right directory while the public entry
point ignores it satisfies nothing.

**No divergence remains.** The two strict xfails this file carried are both
gone, and neither was closed by changing the SDK:

* ``missing_configured_dir_is_not_an_error`` expected an empty result where this
  SDK raises. §5.12.6 stated no outcome, so neither side was provably wrong;
  v1.36.0's clause 5 states one, and it is the raise. The case is now
  ``missing_configured_dir_raises`` and is driven as an ordinary case.
* the fixture's descriptor spelled the binding field ``target_id`` while this
  SDK's loader — and ``binding_errors.json``, and both other SDKs — require
  ``target``. §5.12.2 was the sole outlier and was corrected; the fixture's
  descriptors now spell ``target``, so the driver's rewrite shim is deleted and
  the descriptors are used as written.
"""

from __future__ import annotations

import os
from pathlib import Path
from typing import Any

import pytest
import yaml

from apcore.bindings import BindingLoader
from apcore.client import APCore
from apcore.config import Config
from apcore.registry import Registry

from .canonical_fixtures import case_ids, load_fixture, reject_unknown_expectations

FIXTURE = "bindings_dir_resolution.json"

_FIXTURE = load_fixture(FIXTURE)
_CASES: dict[str, dict[str, Any]] = {case["id"]: case for case in _FIXTURE["test_cases"]}

#: Named binding descriptors, ``name -> document``. Every ``fs`` value names one
#: of these. They carry DISTINCT ``module_id``s on purpose: a case that plants
#: files in two candidate directories can then tell which directory was
#: enumerated, where a single shared id made such a case pass whichever
#: directory won.
#: The block's own ``comment`` key documents the map; it is prose, not a
#: descriptor, and iterating it as one is how a driver ends up asserting against
#: a string.
_BINDING_FILES: dict[str, dict[str, Any]] = {
    name: document for name, document in _FIXTURE["binding_files"].items() if isinstance(document, dict)
}

#: A real callable for the descriptors' target to resolve to. The fixture names
#: ``fixture_targets:greet``, which is a placeholder — the spec repo ships no
#: such module, and each SDK supplies its own. ``auto_schema: true`` means the
#: target must carry type annotations, so this points at the annotated helper
#: the rest of this repo's binding tests use. Only the target *module* is
#: substituted; the field name is the fixture's own (§5.12.2 ``target``).
_REAL_TARGET = "binding_helpers:typed_function"


def _case(case_id: str) -> dict[str, Any]:
    assert case_id in _CASES, f"canonical fixture {FIXTURE} no longer defines case {case_id!r}"
    return _CASES[case_id]


def _descriptor(name: str) -> dict[str, Any]:
    assert name in _BINDING_FILES, f"case names binding descriptor {name!r}, which {FIXTURE} does not define"
    return _BINDING_FILES[name]


def _module_ids_of(descriptor_name: str) -> list[str]:
    """The module IDs the named descriptor registers."""
    return [entry["module_id"] for entry in _descriptor(descriptor_name)["bindings"]]


# ---------------------------------------------------------------------------
# Environment isolation (driver_contract: env_isolation)
# ---------------------------------------------------------------------------


@pytest.fixture(autouse=True)
def _isolated_environment(monkeypatch: pytest.MonkeyPatch) -> None:
    """Delete every ``APCORE_*`` variable; a case's ``env`` block puts its own back.

    ``delenv``, never ``setenv(name, "")``. An inherited ``APCORE_BINDINGS_DIR``
    would turn the discriminating case into the case that already passes; an
    inherited ``APCORE_CONFIG_FILE`` would point discovery somewhere else
    entirely. Blanking is not a substitute: §9.2 treats an empty variable as an
    override, and §9.2.1 requirement 5 makes this SDK discard it — two different
    behaviours, neither of which is "the variable is not set".
    """
    for name in [n for n in os.environ if n.startswith("APCORE_")]:
        monkeypatch.delenv(name, raising=False)


# ---------------------------------------------------------------------------
# Building a case's layout on disk
# ---------------------------------------------------------------------------


def _binding_document(descriptor_name: str) -> dict[str, Any]:
    """The named descriptor, with only its placeholder target substituted.

    §5.12.2 declares the binding-item field ``target``, which is what the
    descriptors spell and what this SDK's loader reads, so nothing about the
    document's *shape* is rewritten here. Through spec v1.35.0 the section said
    ``target_id`` — the sole outlier against the canonical schema, both binding
    fixtures and all three SDKs — and this driver carried a rename shim and an
    xfail for it. v1.36.0 corrected the section; the shim is deleted.
    """
    document = _descriptor(descriptor_name)
    return {
        "bindings": [
            {**entry, "target": _REAL_TARGET if entry["target"].startswith("fixture_targets:") else entry["target"]}
            for entry in document["bindings"]
        ]
    }


def _write_layout(root: Path, case: dict[str, Any]) -> None:
    """Materialise a case's ``config_file`` and ``fs`` blocks under *root*.

    ``fs_values_name_a_descriptor``: each ``fs`` value is a key in
    ``binding_files``, never a literal document and never one shared descriptor
    reused everywhere.
    """
    config_file = case.get("config_file")
    if config_file is not None:
        path = root / config_file["path"]
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(yaml.safe_dump(config_file["content"] or {}, sort_keys=False), encoding="utf-8")

    for relative, descriptor_name in (case.get("fs") or {}).items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(yaml.safe_dump(_binding_document(descriptor_name), sort_keys=False), encoding="utf-8")


def _load_config(root: Path, case: dict[str, Any], monkeypatch: pytest.MonkeyPatch) -> Config | None:
    """Load the case's config file through ``Config``, with its ``env`` applied.

    ``validate=False``: the fixture's ``config_file.content`` blocks declare no
    ``version``, which §9.1's required-field check rejects. The requirement
    under test is that the FILE tier of ``bindings.dir`` reaches the loader, and
    validation of unrelated required fields would stop every case before it
    started.

    ``env_set_after_config_load`` is deliberately NOT applied here — see
    :func:`_drive`.
    """
    for name, value in (case.get("env") or {}).items():
        monkeypatch.setenv(name, value)

    config_file = case.get("config_file")
    if config_file is None:
        return None
    return Config.load(str(root / config_file["path"]), validate=False)


def _module_ids(modules: list[Any]) -> list[str]:
    return sorted(module.module_id for module in modules)


# ---------------------------------------------------------------------------
# Driving one case
# ---------------------------------------------------------------------------


def _drive(
    case_id: str,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    *,
    known_expectations: set[str] | None = None,
) -> tuple[dict[str, Any], list[Any]]:
    """Build the case's layout, invoke the loader as the case dictates, return its result.

    The explicit-directory argument is passed **only** when the case declares
    one. When ``explicit_dir`` is null the call omits the parameter entirely
    rather than passing ``None`` alongside a directory this driver worked out —
    the fixture's ``no_explicit_argument`` clause, which is the difference
    between exercising §5.12.6 and exercising the call shape that already
    worked.

    ``env_set_after_config_load`` variables are applied *between* the ``Config``
    construction and the loader call, which is the whole content of the clause-2
    case: the merged ``Config`` therefore holds the FILE value while the raw
    environment holds a different one, so a loader that reads the variable
    itself is distinguishable from one that reads the merged configuration.
    """
    case = _case(case_id)
    reject_unknown_expectations(FIXTURE, case, known_expectations or {"expected"})

    monkeypatch.chdir(tmp_path)
    _write_layout(tmp_path, case)
    config = _load_config(tmp_path, case, monkeypatch)

    for name, value in (case.get("env_set_after_config_load") or {}).items():
        monkeypatch.setenv(name, value)

    registry = Registry()
    loader = BindingLoader()

    explicit_dir = case.get("explicit_dir")
    if explicit_dir is None:
        modules = loader.load_binding_dir(registry=registry, config=config)
    else:
        modules = loader.load_binding_dir(explicit_dir, registry, config=config)

    return case, modules


def _assert_scan_was_of(case: dict[str, Any], modules: list[Any]) -> None:
    """The enumerated directory, read off the loader's result and the layout.

    The fixture's ``scan_observation`` clause forbids reading ``scanned_dir``
    back off the config value this driver supplied. It is derived instead from
    the two things the fixture *does* provide: the module IDs the loader
    produced, and the layout that says which descriptor sits in which directory.
    Every ID produced must be declared by a descriptor under ``scanned_dir``,
    and no ID declared only by a descriptor in some other candidate directory
    may appear — which is what makes the multi-directory layouts
    (``from_file`` / ``from_env`` / ``from_argument``) prove that one directory
    was enumerated rather than merely that one file was found.
    """
    scanned_dir = case["expected"]["scanned_dir"]
    produced = set(_module_ids(modules))

    inside: set[str] = set()
    outside: set[str] = set()
    for relative, descriptor_name in (case.get("fs") or {}).items():
        target = inside if relative.rsplit("/", 1)[0] == scanned_dir else outside
        target.update(_module_ids_of(descriptor_name))

    assert (
        produced <= inside
    ), f"loader produced {sorted(produced - inside)}, which no descriptor under {scanned_dir!r} declares"
    assert (
        produced & (outside - inside) == set()
    ), f"loader enumerated a directory other than {scanned_dir!r}: it produced {sorted(produced & outside)}"


def _assert_loaded(case: dict[str, Any], modules: list[Any]) -> None:
    assert case["expected"]["scanned"] is True
    assert _module_ids(modules) == sorted(case["expected"]["loaded_module_ids"])
    _assert_scan_was_of(case, modules)


# ---------------------------------------------------------------------------
# Clause 1 — resolve the directory from ``bindings.dir``
# ---------------------------------------------------------------------------


def test_config_file_dir_is_scanned_with_env_unset(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """THE discriminating case: file tier, env unset, no explicit argument.

    This is the case no SDK passed before v1.35.0 and the only one that
    separates the corrected behaviour from the status quo. Every pre-existing
    loader test passes an explicit directory (works either way) and TypeScript's
    pre-existing raw ``process.env`` read covered the env tier alone.
    """
    _assert_loaded(*_drive("config_file_dir_is_scanned_with_env_unset", tmp_path, monkeypatch))


def test_default_dir_when_key_absent(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """No ``bindings.dir`` anywhere and no explicit argument: ``./bindings`` applies.

    Since spec v1.36.0 that default reaches the loader by two routes at once —
    the merged ``_DEFAULTS`` table and the loader's own last tier — and the case
    is satisfied by either, which is the point: the *value* is canonical, the
    tier that supplies it is not normative.
    """
    _assert_loaded(*_drive("default_dir_when_key_absent", tmp_path, monkeypatch))


def test_env_overrides_config_file_dir(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """§9.2 precedence, top tier: ``APCORE_BINDINGS_DIR`` beats the config file.

    Both candidate directories exist and each holds a binding file declaring a
    DIFFERENT module id, so exactly one answer passes. With a shared id — which
    is how the fixture first shipped — this case passed whichever directory the
    implementation scanned and could not detect one that ignored the env tier.
    """
    _assert_loaded(*_drive("env_overrides_config_file_dir", tmp_path, monkeypatch))


def test_env_var_must_not_be_read_directly_at_the_loader(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """§5.12.6 clause 2: the env tier arrives through §9.2, never through the loader.

    ``APCORE_BINDINGS_DIR`` is set **after** the ``Config`` is built, so the
    merged configuration holds the FILE value while the raw environment holds a
    different one. A conforming loader scans ``from_file``; one that reads the
    variable itself scans ``from_env``. Without this case clause 2 has no
    coverage at all — an implementation that reads the raw variable satisfies
    every other case in the fixture, which is the defect apcore-typescript#36
    was filed for.
    """
    _assert_loaded(*_drive("env_var_must_not_be_read_directly_at_the_loader", tmp_path, monkeypatch))


def test_explicit_argument_wins_over_config(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """§5.12.6 clause 2: explicit > env > file > default, with all three present."""
    _assert_loaded(*_drive("explicit_argument_wins_over_config", tmp_path, monkeypatch))


def test_config_file_pattern_is_honoured(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """``bindings.pattern`` travels the same chain, not a loader-signature default.

    The directory holds one file matching the configured pattern and one
    matching only the ``*.binding.yaml`` default, so an implementation that
    keeps the pattern in its signature loads the *wrong* file rather than none —
    a distinguishable wrong answer instead of an empty result.
    """
    _assert_loaded(*_drive("config_file_pattern_is_honoured", tmp_path, monkeypatch))


def test_default_pattern_when_key_absent(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """No ``bindings.pattern``: ``*.binding.yaml`` applies and a sibling is skipped."""
    _assert_loaded(*_drive("default_pattern_when_key_absent", tmp_path, monkeypatch))


# ---------------------------------------------------------------------------
# The pattern DIALECT — PROTOCOL_SPEC 9.2.3, Algorithm A25 (#116)
#
# Every pattern the corpus carried before v1.37.0 was `*.binding.yaml` or
# `*.bind.yaml`: leading star plus literal suffix, the one family on which a
# real glob, a leading-star strip and a first-star removal all coincide. So
# `config_file_pattern_is_honoured` could prove the pattern is READ and could
# not prove it MEANS the same thing in three languages. Each case below is
# drawn from outside that family and separates at least one SDK as shipped.
# ---------------------------------------------------------------------------


def test_pattern_star_in_the_middle_is_honoured(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """``data*.yaml`` — the ordinary glob a suffix match cannot express."""
    _assert_loaded(*_drive("pattern_star_in_the_middle_is_honoured", tmp_path, monkeypatch))


def test_pattern_first_star_must_not_be_removed_from_the_middle(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """``a*b.yaml`` must not match ``zab.yaml``.

    The failure this pins is the one that LOADS A FILE NOBODY ASKED FOR:
    ``replace('*', '')`` deletes the star wherever it sits, so the pattern
    collapses to ``ab.yaml`` and a suffix comparison accepts a name that an
    anchored pattern cannot reach.
    """
    _assert_loaded(*_drive("pattern_first_star_must_not_be_removed_from_the_middle", tmp_path, monkeypatch))


def test_pattern_question_mark_is_a_wildcard(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """``?`` matches exactly one character (A25), and the two-character sibling is skipped."""
    _assert_loaded(*_drive("pattern_question_mark_is_a_wildcard", tmp_path, monkeypatch))


def test_pattern_bracket_is_a_literal_not_a_character_class(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """``[ab].bind.yaml`` names a literal filename; A25 has no character classes.

    This is the case ``Path.glob`` fails: it expands the class and loads the
    decoy. It must also not RAISE — every string is a valid A25 pattern.
    """
    _assert_loaded(*_drive("pattern_bracket_is_a_literal_not_a_character_class", tmp_path, monkeypatch))


# ---------------------------------------------------------------------------
# Clause 5 — a missing resolved directory raises
# ---------------------------------------------------------------------------


def test_missing_configured_dir_raises(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """§5.12.6 clause 5: an error naming the resolved directory, not an empty result.

    Was a strict xfail. Through v1.35.0 the fixture expected an empty result
    where this SDK raises, and §5.12.6 stated no outcome, so neither side was
    provably wrong and the divergence was recorded rather than papered over.
    v1.36.0 states an outcome and it is the raise — binding loading is
    user-invoked, so a directory that is not there is a mistake, in deliberate
    contrast with ``ACL.discover``'s no-op (D-64), where discovery is automatic
    and a missing ``acl.root`` is the ordinary case.

    The ``error_code`` and ``error_message_names_resolved_dir`` expectations are
    both read from the fixture and both asserted: a raise carrying the right
    code but a message that does not name the directory leaves the operator
    unable to tell *which* directory the loader resolved, which is the whole
    content of the clause's second half.
    """
    case = _case("missing_configured_dir_raises")
    reject_unknown_expectations(FIXTURE, case, {"expected"})

    monkeypatch.chdir(tmp_path)
    _write_layout(tmp_path, case)
    config = _load_config(tmp_path, case, monkeypatch)

    assert case["expected"]["scanned"] is True
    with pytest.raises(Exception) as raised:  # noqa: PT011 - the code is what the fixture pins
        BindingLoader().load_binding_dir(registry=Registry(), config=config)

    assert getattr(raised.value, "code", None) == case["expected"]["error_code"]
    assert case["expected"]["error_message_names_resolved_dir"] is True
    assert case["expected"]["scanned_dir"] in str(raised.value), (
        "the raise must name the directory the loader resolved from bindings.dir; "
        "otherwise the operator cannot tell which directory was looked for"
    )


# ---------------------------------------------------------------------------
# Clause 3 — no scan at initialisation
# ---------------------------------------------------------------------------


def test_no_auto_scan_at_init(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """§5.12.6 clause 3: constructing a client MUST NOT enumerate a binding directory.

    The fixture's ``no_startup_scan`` clause requires more than "construction
    succeeded", which passes trivially: the configured directory exists and
    holds a well-formed binding file that *would* load cleanly if anything
    scanned, and its module ID must be absent from the registry afterwards. This
    is the reading the pre-v1.35.0 wording invited, and adopting it would put
    filesystem I/O in every client's startup.
    """
    case = _case("no_auto_scan_at_init")
    reject_unknown_expectations(FIXTURE, case, {"expected"})
    assert case.get("invoke_loader") is False, "this case is defined by NOT invoking the loader"

    monkeypatch.chdir(tmp_path)
    _write_layout(tmp_path, case)
    config = _load_config(tmp_path, case, monkeypatch)

    planted = sorted({mid for name in case["fs"].values() for mid in _module_ids_of(name)})
    # The file that would be loaded, proving the layout is not the reason
    # nothing was registered.
    would_load = sorted((tmp_path / "custom_bindings").glob("*.binding.yaml"))
    assert would_load, "layout is wrong: the case needs a loadable binding file under the configured dir"

    client = APCore(config=config)

    assert case["expected"]["scanned"] is False
    registered = client.registry.list()
    for module_id in planted:
        assert (
            module_id not in registered
        ), f"client construction scanned a binding directory and registered {module_id!r}"
    assert case["expected"]["registered_module_ids"] == [], "fixture expects an empty registry contribution"

    # And the loader still works when the application asks for it — clause 3
    # forbids the implicit scan, not the explicit one.
    modules = BindingLoader().load_binding_dir(registry=Registry(), config=config)
    assert _module_ids(modules) == planted


# ---------------------------------------------------------------------------
# Coverage cross-check and the runner contract
# ---------------------------------------------------------------------------


COVERED: dict[str, str] = {
    "config_file_dir_is_scanned_with_env_unset": "test_config_file_dir_is_scanned_with_env_unset",
    "default_dir_when_key_absent": "test_default_dir_when_key_absent",
    "env_overrides_config_file_dir": "test_env_overrides_config_file_dir",
    "env_var_must_not_be_read_directly_at_the_loader": "test_env_var_must_not_be_read_directly_at_the_loader",
    "explicit_argument_wins_over_config": "test_explicit_argument_wins_over_config",
    "config_file_pattern_is_honoured": "test_config_file_pattern_is_honoured",
    "pattern_star_in_the_middle_is_honoured": "test_pattern_star_in_the_middle_is_honoured",
    "pattern_first_star_must_not_be_removed_from_the_middle": "test_pattern_first_star_must_not_be_removed_from_the_middle",
    "pattern_question_mark_is_a_wildcard": "test_pattern_question_mark_is_a_wildcard",
    "pattern_bracket_is_a_literal_not_a_character_class": "test_pattern_bracket_is_a_literal_not_a_character_class",
    "default_pattern_when_key_absent": "test_default_pattern_when_key_absent",
    "missing_configured_dir_raises": "test_missing_configured_dir_raises",
    "no_auto_scan_at_init": "test_no_auto_scan_at_init",
}

#: The canonical fixture's case count, asserted so that an upstream addition or
#: removal is a named failure rather than a quietly smaller run.
EXPECTED_CASE_COUNT = 13


def test_every_canonical_case_is_driven() -> None:
    """A case added upstream is a failure here, never a silent gap."""
    canonical = set(case_ids(FIXTURE))
    claimed = set(COVERED)
    assert canonical - claimed == set(), f"canonical fixture {FIXTURE} gained case(s) with no driver here"
    assert claimed - canonical == set(), f"this file claims case(s) {FIXTURE} no longer defines"
    assert len(canonical) == EXPECTED_CASE_COUNT


def test_every_claimed_driver_exists() -> None:
    import sys

    module = sys.modules[__name__]
    missing = [name for name in COVERED.values() if not hasattr(module, name)]
    assert missing == [], f"claimed driver function(s) not defined: {missing}"


def test_descriptors_carry_distinct_module_ids() -> None:
    """The property that makes the multi-directory cases discriminate at all.

    ``binding_files`` replaced a single shared ``binding_file`` precisely so a
    case placing files in two candidate directories can tell WHICH directory was
    enumerated. Collapse the ids back to one and every such case passes whichever
    directory won, silently — so the distinctness is pinned rather than assumed.
    """
    ids = [mid for descriptor in _BINDING_FILES.values() for mid in (e["module_id"] for e in descriptor["bindings"])]
    assert len(ids) == len(set(ids)), f"binding_files reuses module id(s): {sorted(ids)}"

    for case in _FIXTURE["test_cases"]:
        directories = {relative.rsplit("/", 1)[0] for relative in (case.get("fs") or {})}
        if len(directories) < 2:
            continue
        per_directory = {
            directory: {
                mid
                for relative, name in case["fs"].items()
                if relative.rsplit("/", 1)[0] == directory
                for mid in _module_ids_of(name)
            }
            for directory in directories
        }
        merged: set[str] = set()
        for module_ids in per_directory.values():
            assert merged & module_ids == set(), (
                f"case {case['id']!r} plants the same module id in two candidate directories, "
                f"so it cannot tell which one was scanned"
            )
            merged |= module_ids


def test_every_descriptor_uses_the_spec_field_spelling() -> None:
    """§5.12.2's binding-item field is ``target``.

    This replaces a strict xfail. Through spec v1.35.0 §5.12.2 declared
    ``target_id`` a MUST while the canonical schema, ``binding_errors.json`` and
    all three SDK loaders used ``target``, so a binding file written from the
    section that defines the format loaded nowhere; this driver rewrote the
    field and recorded the divergence. v1.36.0 corrected the section and the
    fixture, so the descriptors now load through this SDK as written, and the
    assertion is that they do.
    """
    for name, descriptor in _BINDING_FILES.items():
        for entry in descriptor["bindings"]:
            assert "target" in entry, f"descriptor {name!r} does not spell the §5.12.2 field 'target'"
            assert "target_id" not in entry, f"descriptor {name!r} still carries the withdrawn 'target_id' spelling"


def test_fixture_descriptors_load_verbatim_but_for_the_placeholder_target(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Each descriptor, unmodified except for its placeholder target, loads here.

    ``fixture_targets:greet`` is a placeholder the spec repo does not ship, so
    every SDK substitutes its own annotated callable; nothing else about the
    document is touched. Driving each descriptor through the loader proves the
    substitution is the *only* difference — a driver that quietly reshaped a
    descriptor would otherwise be testing a document the fixture never wrote.
    """
    monkeypatch.chdir(tmp_path)
    for name in _BINDING_FILES:
        directory = tmp_path / name
        directory.mkdir()
        (directory / "probe.binding.yaml").write_text(
            yaml.safe_dump(_binding_document(name), sort_keys=False), encoding="utf-8"
        )
        modules = BindingLoader().load_binding_dir(str(directory), Registry())
        assert _module_ids(modules) == sorted(_module_ids_of(name))


def test_driver_contract_is_not_a_case() -> None:
    """``driver_contract`` and ``binding_files`` are runner inputs, not test cases."""
    assert "driver_contract" in _FIXTURE
    assert "binding_files" in _FIXTURE
    assert set(_FIXTURE["binding_files"]) - set(_BINDING_FILES) == {
        "comment"
    }, "binding_files gained a non-descriptor key this driver does not know to skip"
    assert "binding_file" not in _FIXTURE, "the singular descriptor was replaced by the named map in spec v1.36.0"
    ids = set(case_ids(FIXTURE))
    assert "driver_contract" not in ids
    assert "binding_files" not in ids
    assert {
        "entry_point",
        "config_construction",
        "env_isolation",
        "no_explicit_argument",
        "scan_observation",
        "no_startup_scan",
        "fs_values_name_a_descriptor",
        "env_set_after_config_load",
    } <= set(_FIXTURE["driver_contract"])
