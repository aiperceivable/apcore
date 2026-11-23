"""Locate and load conformance fixtures from the canonical apcore spec repo.

Every conformance driver in this package resolves its fixture through here.

Why not a vendored copy: apcore-typescript and apcore-rust both read the
canonical file directly (via ``$CONFORMANCE_SPEC_REPO`` or a sibling checkout), so a
spec-side edit reaches them on the next test run. apcore-python used to keep
private copies under ``tests/conformance/fixtures/``, which meant a fixture that
gained a case in the spec repo silently left Python asserting the old snapshot —
the exact drift conformance fixtures exist to prevent. Five drivers had already
open-coded this resolver; the other four read the vendored copy.

Search order (identical to ``tests/test_conformance.py``):

1. ``$CONFORMANCE_FIXTURES`` — a fixtures directory, used by CI matrix jobs.
2. ``$CONFORMANCE_SPEC_REPO`` — the spec repo root.
3. ``../apcore/`` beside this repo — the standard workspace and CI layout.
"""

from __future__ import annotations

import json
import os
from functools import lru_cache
from pathlib import Path
from typing import Any

import pytest

__all__ = [
    "fixtures_dir",
    "schemas_dir",
    "fixture_path",
    "load_fixture",
    "case_ids",
    "spec_repo_env",
    "expectation_keys",
    "reject_unknown_expectations",
    "dispatch_or_fail",
]

_FIXTURES_ENV = "CONFORMANCE_FIXTURES"
_SPEC_REPO_ENV = "CONFORMANCE_SPEC_REPO"

# Transitional fallback (apcore#88), the exact twin of the one below: the
# fixtures-directory locator used to be ``APCORE_FIXTURES``, which §9.2 lowers
# to the config key ``fixtures`` — declared by no schema. Same reasoning, same
# transitional read of the old name.
# REMOVE once all three SDK CI workflows are on CONFORMANCE_FIXTURES.
_LEGACY_FIXTURES_ENV = "APCORE_FIXTURES"

# Transitional fallback (apcore#86). The spec-repo locator used to be
# ``APCORE_SPEC_REPO``, but PROTOCOL_SPEC §9.2 makes *every* ``APCORE_*``
# variable a config override: the suffix is lowercased and split into a dot
# path, so ``APCORE_SPEC_REPO=/path`` injected ``spec.repo`` into the declared
# config document that §9.1's required-field check runs against. The locator is
# test infrastructure, not configuration, so it moved out of the claimed
# prefix. Reading the old name keeps a developer who still exports it working.
# REMOVE once all three SDK CI workflows are on CONFORMANCE_SPEC_REPO.
_LEGACY_SPEC_REPO_ENV = "APCORE_SPEC_REPO"


def _fixtures_env() -> tuple[str, str] | None:
    """Return ``(variable_name, value)`` for the fixtures-directory override.

    Same shape and same reason as :func:`spec_repo_env`: the name travels with
    the value so a failure message names the variable that was actually set.
    """
    for name in (_FIXTURES_ENV, _LEGACY_FIXTURES_ENV):
        value = os.environ.get(name)
        if value:
            return name, value
    return None


def spec_repo_env() -> tuple[str, str] | None:
    """Return ``(variable_name, value)`` for the spec-repo override, if set.

    The name is returned alongside the value so failure messages can name the
    variable the developer actually set rather than the one they did not.
    """
    for name in (_SPEC_REPO_ENV, _LEGACY_SPEC_REPO_ENV):
        value = os.environ.get(name)
        if value:
            return name, value
    return None


@lru_cache(maxsize=1)
def fixtures_dir() -> Path:
    """Return the canonical ``conformance/fixtures`` directory."""
    env_fixtures = _fixtures_env()
    if env_fixtures:
        name, value = env_fixtures
        candidate = Path(value)
        if candidate.is_dir():
            return candidate
        pytest.fail(f"${name}={value} is not a directory.")

    env_repo = spec_repo_env()
    if env_repo:
        name, value = env_repo
        candidate = Path(value) / "conformance" / "fixtures"
        if candidate.is_dir():
            return candidate
        pytest.fail(
            f"${name}={value} does not contain conformance/fixtures/. "
            f"Ensure the apcore protocol spec repo is at that path."
        )

    # apcore-python/tests/conformance/ → apcore-python/ → workspace root
    repo_root = Path(__file__).resolve().parent.parent.parent
    sibling = repo_root.parent / "apcore" / "conformance" / "fixtures"
    if sibling.is_dir():
        return sibling

    pytest.fail(
        "Cannot find the canonical apcore conformance fixtures.\n\n"
        "Fix one of:\n"
        f"  1. Set ${_SPEC_REPO_ENV} to the apcore spec repo path\n"
        f"  2. Set ${_FIXTURES_ENV} to a conformance/fixtures directory\n"
        "  3. Check out the apcore spec repo beside apcore-python/"
    )


@lru_cache(maxsize=1)
def schemas_dir() -> Path:
    """Return the canonical ``schemas`` directory of the apcore spec repo.

    Deliberately does **not** consult ``CONFORMANCE_FIXTURES``
    (``docs/spec/conformance.md`` §8.2.1 rule 4): that variable names one
    directory rather than a repo, so there is nothing to append ``schemas/``
    to. Four call sites used to reach here as ``fixtures_dir().parent.parent /
    "schemas"``, which is correct only when the fixtures were resolved through
    a repo root — set ``CONFORMANCE_FIXTURES`` to a bare directory and they read
    ``<that directory>/../../schemas``, which is somewhere else entirely.
    """
    env_repo = spec_repo_env()
    if env_repo:
        name, value = env_repo
        candidate = Path(value) / "schemas"
        if candidate.is_dir():
            return candidate
        pytest.fail(f"${name}={value} does not contain schemas/.")

    repo_root = Path(__file__).resolve().parent.parent.parent
    sibling = repo_root.parent / "apcore" / "schemas"
    if sibling.is_dir():
        return sibling

    pytest.fail(
        "Cannot find the canonical apcore schemas.\n\n"
        "Fix one of:\n"
        f"  1. Set ${_SPEC_REPO_ENV} to the apcore spec repo path\n"
        "  2. Check out the apcore spec repo beside apcore-python/\n\n"
        f"Note: ${_FIXTURES_ENV} does not help here — it names a fixtures "
        "directory, not a repo (conformance.md §8.2.1 rule 4)."
    )


def fixture_path(name: str) -> Path:
    """Return the path to canonical fixture *name* (e.g. ``acl_evaluation.json``)."""
    path = fixtures_dir() / name
    if not path.is_file():
        pytest.fail(f"Canonical conformance fixture '{name}' not found at {path}")
    return path


def load_fixture(name: str) -> dict[str, Any]:
    """Load and parse canonical fixture *name*."""
    return json.loads(fixture_path(name).read_text())  # type: ignore[no-any-return]


def case_ids(name: str) -> list[str]:
    """Return the ``id`` of every test case in canonical fixture *name*.

    Used by the drivers that assert behaviour in hand-written form rather than
    by iterating the fixture: comparing this list against the cases they cover
    turns a spec-side addition into a failing test instead of a silent gap.
    """
    fixture = load_fixture(name)
    cases = fixture.get("test_cases") or fixture.get("cases") or []
    return [case["id"] for case in cases]


# ---------------------------------------------------------------------------
# Fixture-expectation helpers (apcore#92, apcore#93)
# ---------------------------------------------------------------------------
#
# These live here, beside the loader, because every conformance driver in this
# repo already imports this module — ``tests/test_conformance.py`` and the
# per-fixture drivers under ``tests/conformance/``, ``tests/events/`` and
# ``tests/observability/`` alike. They were introduced privately in
# ``tests/test_conformance.py`` for apcore#92 and moved here for apcore#93 so
# there is ONE mechanism rather than a copy per driver file.
#
# A fixture's declared expectation is a VALUE — a wire code, a state name, a
# count. Five driver shapes look like they check it and do not:
#
#   1. ``if "expected_error" in case:`` branches on the KEY EXISTING, so the
#      value the fixture declares never reaches an assertion.
#   2. ``if code == "X": ...`` with no ``else`` silently skips the whole
#      assertion block for any value the driver does not recognise.
#   3. ``else: pytest.raises(Exception)`` / "assert it did not crash" accepts
#      anything at all.
#   4. A positive case whose entire assertion is "did not raise" — an
#      implementation that does nothing also passes. These need an observable
#      POST-CONDITION.
#   5. Asserting the fixture's own input back at itself — a tautology that
#      cannot fail on SDK behaviour.
#
# All five make the driver's own literal the contract and the fixture
# decoration: mutate the declared value in the JSON and the suite stays green,
# which is exactly what the spec repo's ``conformance/check_case_pinning.py``
# measures.


def expectation_keys(case: dict[str, Any]) -> list[str]:
    """Top-level keys of *case* that state an expectation rather than an input.

    Mirrors ``check_case_pinning.expectation_keys`` — anything spelled
    ``expected`` or ``expected_*``.
    """
    return sorted(k for k in case if k == "expected" or k.startswith("expected_"))


def reject_unknown_expectations(fixture: str, case: dict[str, Any], known: set[str]) -> None:
    """Fail when a case states an expectation this driver does not read.

    A key nobody reads asserts nothing while reading as covered in the fixture
    and in every count derived from it.
    """
    unknown = sorted(set(expectation_keys(case)) - known)
    if unknown:
        pytest.fail(
            f"[{fixture} :: {case.get('id')!r}] states expectation key(s) {unknown} "
            f"that this driver does not read. Teach the driver, do not skip it. "
            f"Known keys: {sorted(known)}"
        )


def dispatch_or_fail(
    fixture: str,
    case_id: str,
    declared: Any,
    mapping: dict[Any, Any],
    what: str,
) -> Any:
    """Resolve a fixture's declared expectation *declared* through *mapping*.

    The generic form of ``_exc_class_for``: an unrecognised declared value is a
    hard failure, never a skipped branch. A dispatch with no ``else`` is what
    turns a wrong fixture value into a passing test — shape 2 above.
    """
    if declared not in mapping:
        pytest.fail(
            f"[{fixture} :: {case_id}] fixture declares {what} {declared!r}, which this "
            f"driver does not know how to check. Teach the driver, do not skip it. "
            f"Known values: {sorted(mapping, key=repr)}"
        )
    return mapping[declared]
