"""Cross-language driver for ``config_path_typed_keys.json``.

PROTOCOL_SPEC §9.2.1 (spec v1.34.0, apcore#113): a *path-typed* configuration
key is one whose value is a filesystem path, the set is **closed**, and it is
declared at the schemas — a property carrying ``"x-apcore-path": true`` in
``schemas/apcore-config.schema.json`` is path-typed and no other key is.

**The defect this pins is a missing declaration, not a missing check.** Until
v1.34.0 the specification never said which configuration values are paths, so
every consumer that had to know built its own list: ``apcore-cli`` hand-
maintained ``SANDBOX_PATH_TYPED_VARS`` with a comment saying that adding a key
means editing three SDKs, and nothing detected drift between that list and the
schemas. Requirement 1 therefore asks for a *public* accessor —
``Config.path_typed_keys()`` here — because the consumer the set exists for
lives in another repository, and a private constant holding the right values
satisfies nothing.

**Both directions, every time.** The fixture's ``driver_contract`` is explicit:
compare as a SET and report the symmetric difference, never a length or a
one-way containment. "Every expected key is present" passes an implementation
that marks *every* config key as path-typed, which is exactly as wrong as
marking none and much harder to notice — hence
``bindings_pattern_is_not_path_typed`` and
``non_path_string_keys_are_not_path_typed``, the two discriminating cases that
exist only to catch the over-broad implementation.

**Scope.** This fixture asserts WHICH keys are path-typed and that the SDK
exposes the set. It asserts nothing about what a relative value resolves
against — that base is §9.2.2's subject and is driven by
``test_config_project_root.py``. A driver that starts asserting resolved
absolute paths here is testing an undecided question.

``declared_set_matches_schemas`` guards the fixture against the schemas rather
than the SDK against the fixture: it re-projects the ``x-apcore-path`` markers
out of the canonical schema files and compares that projection with the
fixture's own ``path_typed_keys`` block. A marker added to the schema without a
fixture update turns this red in all three SDKs at once.
"""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path
from typing import Any

import pytest

from apcore.config import Config

from .canonical_fixtures import (
    case_ids,
    load_fixture,
    reject_unknown_expectations,
    schemas_dir,
)

FIXTURE = "config_path_typed_keys.json"

_FIXTURE = load_fixture(FIXTURE)
_CASES: dict[str, dict[str, Any]] = {case["id"]: case for case in _FIXTURE["test_cases"]}

#: The fixture's own declaration of the closed set. Every assertion below reads
#: this rather than a literal in this file: a driver whose contract is its own
#: literal is fixture decoration, not a conformance check.
DECLARED_KEYS: list[str] = _FIXTURE["path_typed_keys"]

#: §9.2.1 declares the set at ``apcore-config.schema.json`` ("a property
#: carrying ``x-apcore-path: true`` … and no other key is"). ``defaults.schema
#: .json`` mirrors it for the three keys that carry defaults there, which
#: ``test_defaults_schema_markers_are_a_subset`` pins.
_DECLARING_SCHEMA = "apcore-config.schema.json"
_MIRRORING_SCHEMA = "defaults.schema.json"


def _case(case_id: str) -> dict[str, Any]:
    assert case_id in _CASES, f"canonical fixture {FIXTURE} no longer defines case {case_id!r}"
    return _CASES[case_id]


# ---------------------------------------------------------------------------
# Projecting ``x-apcore-path`` out of a canonical schema
# ---------------------------------------------------------------------------


def _project_path_markers(document: dict[str, Any]) -> set[str]:
    """Dotted keys carrying ``"x-apcore-path": true`` anywhere in *document*.

    Walks ``properties`` for key segments, follows ``$ref`` into ``$defs`` (the
    canonical schema keeps every section in a ``$defs`` entry), and descends
    ``oneOf`` / ``anyOf`` / ``allOf`` without consuming a segment — composition
    keywords are alternatives for the *same* key, not children of it.

    An ``items`` descent appends ``[]`` and truncates there, which is the
    ``roots_element_form`` clause of the fixture's ``driver_contract``: both
    element forms of ``extensions.roots`` are path-typed, and both are reported
    under the single key ``extensions.roots[]``. Without the truncation the
    ``{root, namespace}`` form would project as ``extensions.roots[].root`` and
    read as a sixth key that no SDK is asked to expose.
    """
    found: set[str] = set()

    def resolve(node: dict[str, Any], defs: dict[str, Any]) -> tuple[dict[str, Any], dict[str, Any]]:
        """Follow ``$ref`` chains, local (``#/$defs/X``) and cross-file (``other.schema.json``).

        driver_contract.cross_file_ref: ``sys_modules`` delegates to
        ``sys-modules.schema.json``, and ``sys_modules.control.overrides_path``
        carries its marker there. The referenced file brings its own ``$defs``.
        """
        seen: set[str] = set()
        while isinstance(node.get("$ref"), str) and node["$ref"] not in seen:
            ref = node["$ref"]
            seen.add(ref)
            if ref.startswith("#/$defs/"):
                resolved = defs.get(ref.rsplit("/", 1)[-1])
            elif not ref.startswith("#"):
                resolved = _schema(ref.split("#", 1)[0])
                defs = resolved.get("$defs", {})
            else:
                resolved = None
            if not isinstance(resolved, dict):
                break
            node = resolved
        return node, defs

    def walk(node: Any, path: list[str], defs: dict[str, Any]) -> None:
        if not isinstance(node, dict):
            return
        node, defs = resolve(node, defs)

        if node.get("x-apcore-path") is True:
            if "[]" in path:
                found.add(".".join(path[: path.index("[]")]) + "[]")
            else:
                found.add(".".join(path))

        properties = node.get("properties")
        if isinstance(properties, dict):
            for name, child in properties.items():
                walk(child, [*path, name], defs)

        items = node.get("items")
        if isinstance(items, dict):
            walk(items, [*path, "[]"], defs)

        for keyword in ("oneOf", "anyOf", "allOf"):
            for alternative in node.get(keyword) or []:
                walk(alternative, path, defs)

    root_defs = document.get("$defs", {})
    root_properties = document.get("properties")
    if isinstance(root_properties, dict):
        for name, child in root_properties.items():
            walk(child, [name], root_defs)
    return found


def _schema(name: str) -> dict[str, Any]:
    path = schemas_dir() / name
    assert path.is_file(), f"canonical schema {name} not found at {path}"
    return json.loads(path.read_text())  # type: ignore[no-any-return]


# ---------------------------------------------------------------------------
# declared_set_matches_schemas
# ---------------------------------------------------------------------------


def test_declared_set_matches_schemas() -> None:
    """The fixture's ``path_typed_keys`` IS the schemas' ``x-apcore-path`` set.

    Guards the fixture against the source it claims to project. Asserted as a
    symmetric difference in both directions so a marker added to the schema and
    a stale entry left in the fixture are separate, named failures.
    """
    case = _case("declared_set_matches_schemas")
    reject_unknown_expectations(FIXTURE, case, {"expected"})

    expected = set(case["expected"]["path_typed_keys"])
    projected = _project_path_markers(_schema(_DECLARING_SCHEMA))

    assert (
        expected - projected == set()
    ), f"{FIXTURE} declares key(s) that carry no x-apcore-path marker in {_DECLARING_SCHEMA}"
    assert (
        projected - expected == set()
    ), f"{_DECLARING_SCHEMA} marks key(s) as x-apcore-path that {FIXTURE} does not declare"
    # The fixture's own block and its case expectation must agree, or the two
    # halves of this file are pinned to different sets.
    assert set(DECLARED_KEYS) == expected


def test_defaults_schema_markers_are_a_subset() -> None:
    """``defaults.schema.json`` mirrors the markers for the keys it declares.

    §9.2.1 names ``apcore-config.schema.json`` as the declaring authority; the
    defaults schema carries the same marker on the three keys that have a
    default there. A marker present in the mirror and absent from the authority
    would mean the set has two sources, which is the condition §9.2.1 closed.
    """
    mirrored = _project_path_markers(_schema(_MIRRORING_SCHEMA))
    declared = _project_path_markers(_schema(_DECLARING_SCHEMA))

    assert mirrored, f"{_MIRRORING_SCHEMA} carries no x-apcore-path marker at all"
    assert (
        mirrored - declared == set()
    ), f"{_MIRRORING_SCHEMA} marks key(s) that {_DECLARING_SCHEMA} does not — the set has two sources"


# ---------------------------------------------------------------------------
# sdk_accessor_matches_declared_set
# ---------------------------------------------------------------------------


def test_sdk_accessor_matches_declared_set() -> None:
    """``Config.path_typed_keys()`` equals the declared set, both directions.

    The fixture's ``both_directions_required`` clause, verbatim: the symmetric
    difference is reported as two named sets rather than collapsed into a
    boolean, because an SDK that omits one key and invents another passes any
    length check and any one-way containment.
    """
    case = _case("sdk_accessor_matches_declared_set")
    reject_unknown_expectations(FIXTURE, case, {"expected"})

    accessor = set(Config.path_typed_keys())
    declared = set(DECLARED_KEYS)

    missing_from_sdk = sorted(declared - accessor)
    extra_in_sdk = sorted(accessor - declared)

    assert missing_from_sdk == case["expected"]["missing_from_sdk"]
    assert extra_in_sdk == case["expected"]["extra_in_sdk"]


def test_accessor_is_public_api() -> None:
    """§9.2.1 requirement 1 — the set is reachable without touching internals.

    The consumer this exists for is in another repository, so the check is that
    the set arrives through the package's public surface, not through
    ``apcore.config``'s module globals.
    """
    import apcore

    assert set(apcore.Config.path_typed_keys()) == set(DECLARED_KEYS)


# ---------------------------------------------------------------------------
# The two discriminating cases
# ---------------------------------------------------------------------------


def test_bindings_pattern_is_not_path_typed() -> None:
    """DISCRIMINATING. ``bindings.pattern`` sits beside ``bindings.dir`` and is not a path.

    It carries a filesystem-looking default (``*.binding.yaml``) and lives in
    the same section as the one key in ``bindings`` that IS path-typed, so it is
    the key an implementation classifying by section — or by "looks like a
    filename" — sweeps in. §9.2.1 requirement 4: a glob matched against
    filenames *within* ``bindings.dir``, never resolved as a path itself.
    """
    case = _case("bindings_pattern_is_not_path_typed")
    reject_unknown_expectations(FIXTURE, case, {"expected"})
    assert case["expected"]["path_typed"] is False

    key = case["key"]
    assert key not in Config.path_typed_keys()
    # And the sibling that IS path-typed is still there, so this cannot pass by
    # the accessor having gone empty.
    assert "bindings.dir" in Config.path_typed_keys()


def test_non_path_string_keys_are_not_path_typed() -> None:
    """DISCRIMINATING. String-valued keys that are not paths.

    An implementation that marks every string-typed key as path-typed fails
    here and passes every presence-only assertion in this file.
    """
    case = _case("non_path_string_keys_are_not_path_typed")
    reject_unknown_expectations(FIXTURE, case, {"expected"})
    assert case["expected"]["path_typed"] is False

    accessor = set(Config.path_typed_keys())
    wrongly_included = sorted(set(case["keys"]) & accessor)
    assert wrongly_included == [], f"non-path key(s) reported as path-typed: {wrongly_included}"


def test_id_map_overrides_is_path_typed() -> None:
    """D-138: the ID map path and the runtime overrides path are path-typed."""
    case = _case("id_map_overrides_is_path_typed")
    reject_unknown_expectations(FIXTURE, case, {"expected"})
    assert case["expected"]["path_typed"] is True

    accessor = set(Config.path_typed_keys())
    missing = sorted(set(case["keys"]) - accessor)
    assert missing == [], f"path-typed key(s) the accessor does not report: {missing}"


# ---------------------------------------------------------------------------
# extensions.roots — list-valued, both element forms
# ---------------------------------------------------------------------------


def test_extensions_roots_elements_are_path_typed() -> None:
    """§9.2.1 requirement 3 — both element forms carry paths, under one key.

    The fixture's ``roots_element_form`` clause: the key is reported as
    ``extensions.roots[]`` whichever form an element takes, and an SDK that
    models only one form is a violation of that entry rather than a separate
    key. So this asserts the reported spelling *and* that a ``Config`` carrying
    both forms round-trips both — the bare string and the ``{root, namespace}``
    object — which is what catches an SDK that silently drops the form it does
    not model.
    """
    case = _case("extensions_roots_elements_are_path_typed")
    reject_unknown_expectations(FIXTURE, case, {"expected"})

    reported_key = case["expected"]["reported_key"]
    assert case["expected"]["path_typed"] is True
    assert reported_key in Config.path_typed_keys()

    # The scalar spelling is NOT the key: an SDK reporting ``extensions.roots``
    # has not said that the *elements* carry the paths.
    assert "extensions.roots" not in Config.path_typed_keys()
    assert "extensions.roots[].root" not in Config.path_typed_keys()

    config = Config(data=case["config"])
    roots = config.get("extensions.roots")
    assert isinstance(roots, list)
    assert roots == case["config"]["extensions"]["roots"], (
        "a Config carrying both element forms must preserve both; dropping the object form "
        "means only one form is modelled"
    )


def test_no_scalar_env_encoding_for_roots(monkeypatch: pytest.MonkeyPatch) -> None:
    """§9.2.1 requirement 3 — no delimiter-separated ``APCORE_EXTENSIONS_ROOTS``.

    ``extensions.roots`` is list-valued and §9.2's scalar override convention
    does not apply to it. An implementation MUST NOT invent an encoding that
    splits the variable into a list; the fixture states the outcome as
    ``roots_from_env: null`` — *no roots list is produced from the variable*.

    This SDK leaves §9.2's ordinary scalar override in place, so the key holds
    the raw string rather than disappearing. That is the absence of an invented
    encoding, which is what requirement 3 forbids; the assertion is therefore
    "not a list", and additionally "not the two-element list the delimiter
    reading would produce", rather than the literal ``None``.
    """
    case = _case("no_scalar_env_encoding_for_roots")
    reject_unknown_expectations(FIXTURE, case, {"expected"})
    assert case["expected"]["roots_from_env"] is None

    _clear_apcore_env(monkeypatch)
    for name, value in case["env"].items():
        monkeypatch.setenv(name, value)

    raw = next(iter(case["env"].values()))
    split_reading = raw.split(":")
    assert len(split_reading) == 2, "fixture env value no longer encodes two roots; re-read the case"

    for config in (Config.from_defaults(), Config(data={})):
        roots = config.get("extensions.roots")
        assert not isinstance(roots, list), (
            f"APCORE_EXTENSIONS_ROOTS produced a roots LIST ({roots!r}); §9.2.1 requirement 3 "
            f"forbids inventing a delimiter-separated encoding for a list-valued key"
        )
        assert roots != split_reading


# ---------------------------------------------------------------------------
# The set is a property of the specification, not of a document
# ---------------------------------------------------------------------------


def test_accessor_is_stable_across_config_instances(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """The set does not depend on what a given ``Config`` happens to declare.

    A ``Config`` from defaults and a ``Config`` loaded from a file that sets
    none of these keys must report the same set — an accessor that projected
    the *loaded document* rather than the specification would differ here.
    """
    case = _case("accessor_is_stable_across_config_instances")
    reject_unknown_expectations(FIXTURE, case, {"expected"})
    assert case["expected"]["same_set"] is True

    _clear_apcore_env(monkeypatch)
    # Load from inside the config file's own directory so that §9.2.2's
    # deprecation warning has no reason to fire — the project root equals CWD.
    # That warning is config_project_root.json's subject, not this fixture's.
    monkeypatch.chdir(tmp_path)

    config_file = tmp_path / "apcore.yaml"
    config_file.write_text("version: '1.0'\nproject:\n  name: fixture\n", encoding="utf-8")

    from_defaults = Config.from_defaults()
    from_file = Config.load(str(config_file), validate=False)

    assert set(from_defaults.path_typed_keys()) == set(DECLARED_KEYS)
    assert set(from_file.path_typed_keys()) == set(DECLARED_KEYS)
    assert set(from_defaults.path_typed_keys()) == set(from_file.path_typed_keys())


# ---------------------------------------------------------------------------
# Helpers and the coverage cross-check
# ---------------------------------------------------------------------------


def _clear_apcore_env(monkeypatch: pytest.MonkeyPatch) -> None:
    """Remove every ``APCORE_*`` variable from the environment.

    ``delenv``, never ``setenv(name, "")``: §9.2 makes an empty string a valid
    override, so blanking a variable does not neutralise it — it *sets* the key
    to the empty string and shadows whatever the file declared.
    """
    for name in [n for n in os.environ if n.startswith("APCORE_")]:
        monkeypatch.delenv(name, raising=False)


COVERED: dict[str, str] = {
    "declared_set_matches_schemas": "test_declared_set_matches_schemas",
    "sdk_accessor_matches_declared_set": "test_sdk_accessor_matches_declared_set",
    "bindings_pattern_is_not_path_typed": "test_bindings_pattern_is_not_path_typed",
    "non_path_string_keys_are_not_path_typed": "test_non_path_string_keys_are_not_path_typed",
    "extensions_roots_elements_are_path_typed": "test_extensions_roots_elements_are_path_typed",
    "no_scalar_env_encoding_for_roots": "test_no_scalar_env_encoding_for_roots",
    "accessor_is_stable_across_config_instances": "test_accessor_is_stable_across_config_instances",
    "id_map_overrides_is_path_typed": "test_id_map_overrides_is_path_typed",
}


#: The canonical fixture's case count, asserted so that an upstream addition or
#: removal is a named failure rather than a quietly smaller run.
EXPECTED_CASE_COUNT = 8


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
    """``driver_contract`` is a runner contract; it must never be iterated as a case.

    Root-level keys of the fixture are inputs and contracts, not test cases.
    ``case_ids`` reads ``test_cases`` only; this pins that the contract block
    exists and stays outside it.
    """
    assert "driver_contract" in _FIXTURE
    assert "driver_contract" not in set(case_ids(FIXTURE))
    assert {"accessor", "comparison", "both_directions_required", "roots_element_form"} <= set(
        _FIXTURE["driver_contract"]
    )
