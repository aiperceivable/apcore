"""Drive `multi_module_discovery.json` — multi-class ID derivation (§2.1.1).

`tests/test_multi_module_discovery_spec.py` covers this contract already, but
under its own clause-ID scheme (`multi_module_discovery.<method>.<kind>.<detail>`
docstrings) which does not correspond one-to-one with the fixture's case ids:
the spec file has ~20 clauses to the fixture's 8, several fixture cases map onto
one parametrized clause (the three `class_name_snake_case_*` cases), and one
fixture case (`disabled_by_default`) has no clause at all because it describes
the *non*-multi-class path. There is no clean automatic mapping, so this driver
reads the fixture directly and dispatches on the fields each case carries — the
mapping is the `_MODE_*` split below, stated explicitly rather than guessed.

Three case shapes:

* `class_name` only            → `class_name_to_segment` (pure conversion)
* `classes` with a marked class → `discover_multi_class` over a synthesized file
* `classes` with none marked    → plain `Registry.discover()` (single-class mode)

Opt-in is the PER-CLASS `multi_class` marker (D-107, spec v1.50.0). The fixture
used to carry a file-level `multi_class_enabled` boolean, which was the toggle
decision-log D-06 withdrew — pinning it kept all three SDKs green against a model
the spec says does not exist, and left apcore-rust with no per-class marker at
all. Reading the marker off each class is what makes this fixture discriminate
between the two models instead of tolerating both.
"""

from __future__ import annotations

import re
import textwrap
from pathlib import Path
from typing import Any

import pytest

from apcore.errors import ModuleIdConflictError, ModuleLoadError
from apcore.registry import Registry
from apcore.registry.entry_point import resolve_entry_point
from apcore.registry.multi_class import (
    _compute_base_id,
    class_name_to_segment,
    discover_multi_class,
)

from .canonical_fixtures import load_fixture, reject_unknown_expectations

FIXTURE_NAME = "multi_module_discovery.json"
FIXTURE = load_fixture(FIXTURE_NAME)
CASES: list[dict[str, Any]] = FIXTURE["test_cases"]

# PROTOCOL_SPEC §2.7 canonical ID grammar, as quoted by the fixture itself.
_CANONICAL_ID_RE = re.compile(r"^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$")

_MODULE_STUB = textwrap.dedent(
    """\
    from pydantic import BaseModel

    from apcore.registry.multi_class import multi_class


    class _Schema(BaseModel):
        pass

    {body}
    """
)

_CLASS_TEMPLATE = textwrap.dedent(
    """\
    {decorator}class {name}:
        input_schema = _Schema
        output_schema = _Schema
        description = "conformance stub"

        def execute(self, inputs, context):
            return {{}}
    """
)


def _is_marked(entry: dict[str, Any]) -> bool:
    """Per-class opt-in marker (D-107). Absent means not participating."""
    return bool(entry.get("multi_class", False))


def _any_marked(case: dict[str, Any]) -> bool:
    return any(_is_marked(e) for e in case["input"].get("classes", []))


def _write_module_file(root: Path, case: dict[str, Any]) -> Path:
    """Materialize the case's file, decorating each class by its own marker."""
    body = "\n\n".join(
        _CLASS_TEMPLATE.format(
            decorator="@multi_class\n" if _is_marked(entry) else "",
            name=entry["name"],
        )
        for entry in case["input"]["classes"]
        if entry["implements_module"]
    )
    path = root / case["input"]["file_path"]
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(_MODULE_STUB.format(body=body), encoding="utf-8")
    return path


def _segment_cases() -> list[dict[str, Any]]:
    return [c for c in CASES if "class_name" in c["input"]]


def _multi_class_cases() -> list[dict[str, Any]]:
    return [c for c in CASES if "classes" in c["input"] and _any_marked(c)]


def _single_class_cases() -> list[dict[str, Any]]:
    return [c for c in CASES if "classes" in c["input"] and not _any_marked(c)]


@pytest.mark.parametrize("case", _segment_cases(), ids=lambda c: c["id"])
def test_class_name_to_segment(case: dict[str, Any]) -> None:
    got = class_name_to_segment(case["input"]["class_name"])
    assert got == case["expected"]["class_segment"], (
        f"[{case['id']}] {case['input']['class_name']!r} → {got!r}, " f"expected {case['expected']['class_segment']!r}"
    )


@pytest.mark.parametrize("case", _multi_class_cases(), ids=lambda c: c["id"])
def test_multi_class_discovery(case: dict[str, Any], tmp_path: Path) -> None:
    cid = case["id"]
    expected: dict[str, Any] = case["expected"]
    path = _write_module_file(tmp_path, case)
    extensions_root = case["input"]["extensions_root"]

    if expected.get("error"):
        with pytest.raises(ModuleIdConflictError) as excinfo:
            discover_multi_class(path, extensions_root=extensions_root)
        assert (
            excinfo.value.code == expected["error"]["code"]
        ), f"[{cid}] error code: got {excinfo.value.code!r}, expected {expected['error']['code']!r}"
        conflicting = expected["error"]["conflicting_segment"]
        assert conflicting in str(
            excinfo.value
        ), f"[{cid}] the conflict must name the colliding segment {conflicting!r}; got: {excinfo.value}"
        assert expected["module_ids"] == [], f"[{cid}] a conflict must register nothing from the file"
        return

    assert expected["error"] is None
    result = discover_multi_class(path, extensions_root=extensions_root)
    ids = sorted(module_id for module_id, _ in result)
    assert ids == sorted(
        expected["module_ids"]
    ), f"[{cid}] derived module ids {ids}, expected {sorted(expected['module_ids'])}"

    if expected.get("grammar_valid"):
        for module_id in ids:
            assert _CANONICAL_ID_RE.match(module_id), f"[{cid}] {module_id!r} violates the canonical_id grammar"


@pytest.mark.parametrize("case", _single_class_cases(), ids=lambda c: c["id"])
def test_single_class_mode_never_suffixes_base_id(case: dict[str, Any], tmp_path: Path) -> None:
    """multi_class disabled: the base_id is never suffixed.

    The fixture's own `note` allows two SDK policies for a second qualifying
    class — "silently ignored **or** causes an error per SDK policy" — and pins
    only one thing unconditionally: "The base_id is never suffixed."
    apcore-python takes the error branch (`resolve_entry_point` raises
    `Ambiguous entry point: multiple Module subclasses found`, which discovery
    logs and skips), so nothing is registered. Both branches are accepted here;
    a suffixed id is accepted in neither.
    """
    cid = case["id"]
    reject_unknown_expectations(FIXTURE_NAME, case, {"expected"})
    path = _write_module_file(tmp_path, case)
    extensions_root = tmp_path / case["input"]["extensions_root"]
    expected_ids = sorted(case["expected"]["module_ids"])
    base_id = case["expected"]["module_ids"][0]

    # apcore#93. Only the ignore branch below ever consulted the fixture's
    # declared ``module_ids``, and apcore-python takes the ERROR branch — so
    # every assertion the case actually reached was the driver's own literal
    # and mutating ``expected.module_ids`` left this test green.
    #
    # The fixture's ``note`` permits either SDK policy for the second class
    # ("silently ignored or causes an error per SDK policy") but pins one thing
    # unconditionally: "The base_id is never suffixed." That is a statement
    # about ID DERIVATION, which apcore-python performs on both branches, so
    # assert it directly against the declared value. This is the observable
    # post-condition the error branch was missing: "resolve_entry_point raised"
    # is satisfied by an implementation that derives nothing at all.
    derived_base_id = _compute_base_id(path, case["input"]["extensions_root"])
    assert derived_base_id == base_id, (
        f"[{cid}] with multi_class disabled the fixture requires the bare base_id "
        f"{base_id!r}; the SDK derived {derived_base_id!r} for {case['input']['file_path']!r}"
    )
    assert "." in derived_base_id and not derived_base_id.startswith(f"{base_id}."), (
        f"[{cid}] {derived_base_id!r} must be the bare base_id, never suffixed with a " f"class segment"
    )

    registry = Registry(extensions_dir=str(extensions_root))
    registry.discover()
    registered = sorted(registry.module_ids)

    for module_id in registered:
        assert not module_id.startswith(
            f"{base_id}."
        ), f"[{cid}] multi_class is disabled, so {module_id!r} must not carry a class segment"

    if registered == expected_ids:
        return  # ignore-the-second-class branch

    # Error branch: nothing registered, and the SDK must say why rather than
    # registering a partial or suffixed id.
    assert registered == [], (
        f"[{cid}] single-class discovery registered {registered}; the fixture allows only "
        f"{expected_ids} (ignore branch) or nothing (error branch)"
    )
    with pytest.raises(ModuleLoadError) as excinfo:
        resolve_entry_point(path)
    assert "Ambiguous entry point" in str(
        excinfo.value
    ), f"[{cid}] the error branch must report the ambiguity; got: {excinfo.value}"


def test_every_fixture_case_is_dispatched() -> None:
    """No case may fall between the three dispatch predicates."""
    dispatched = {c["id"] for c in _segment_cases() + _multi_class_cases() + _single_class_cases()}
    all_ids = {c["id"] for c in CASES}
    assert (
        dispatched == all_ids
    ), f"multi_module_discovery.json cases no dispatch predicate matches: {sorted(all_ids - dispatched)}"
