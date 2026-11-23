"""PROTOCOL_SPEC §9.1.2 requirement 6 — the pinned SemVer grammar, both halves.

The rejection half alone is not coverage, and this file exists because that is
exactly how a defect shipped in apcore-rust: the only case anyone wrote was
``version: "1.0"`` expecting a rejection, and it passed against a pattern that
rejected *everything*, valid SemVer included. A constraint that refuses valid
input is as broken as one that accepts invalid input, and only the acceptance
half can tell the two apart.

Python was never affected — it writes the grammar as adjacent raw literals with
no line continuation — but the assertion belongs in all three SDKs, because the
grammar is pinned in the specification precisely so three implementations cannot
diverge, and an untested implementation cannot show that it has not.
"""

from __future__ import annotations

import pytest

from apcore.bindings import _SEMVER_RE

ACCEPTED = [
    "0.0.0",
    "1.0.0",
    "0.1.0",
    "10.20.30",
    "1.2.3-rc.1",
    "1.2.3-0.3.7",
    "1.2.3+build.5",
    "1.2.3-beta.1+exp.sha.5114f85",
]

REJECTED = ["1.0", "1", "v1.0.0", "1.0.0.0", "01.0.0", "", "1.0.0 "]


@pytest.mark.parametrize("version", ACCEPTED)
def test_valid_semver_is_accepted(version: str) -> None:
    assert _SEMVER_RE.match(version) is not None, f"§9.1.2's grammar must accept the valid SemVer {version!r}"


@pytest.mark.parametrize("version", REJECTED)
def test_invalid_semver_is_rejected(version: str) -> None:
    assert _SEMVER_RE.match(version) is None, f"§9.1.2's grammar must reject {version!r}"


def test_the_pattern_contains_no_whitespace() -> None:
    """There is no ``(?x)`` flag, so a stray space is a mandatory character.

    Pinned separately from the behaviour above because it is the *mechanism* of
    the apcore-rust defect: a reformat put literal spaces into the pattern and
    every version stopped matching. This assertion cannot be satisfied by a
    pattern that merely happens to work today.
    """
    assert not any(
        c.isspace() for c in _SEMVER_RE.pattern
    ), f"the §9.1.2 semver pattern must contain no whitespace: {_SEMVER_RE.pattern!r}"
