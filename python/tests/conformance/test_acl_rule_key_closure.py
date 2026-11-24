"""Cross-language driver for ``acl_rule_key_closure.json``.

PROTOCOL_SPEC §6.1 (spec v1.27.0, #107): ACL rule keys are a closed set, and a
rule carrying anything else fails to load. A key nothing evaluates was dropped
in silence before this, which widens an ``allow`` rule with no warning — the
§6.1.1 defect class on the pattern side rather than the condition side.
"""

from __future__ import annotations

import tempfile
from pathlib import Path
from typing import Any

import pytest
import yaml

from apcore.acl import ACL
from apcore.errors import ACLRuleError

from .canonical_fixtures import fixtures_dir, load_fixture

FIXTURE = "acl_rule_key_closure.json"


def _present() -> bool:
    return (fixtures_dir() / FIXTURE).is_file()


# The fixture lands in the spec repo one push after this driver, so that
# `check_driver_coverage.py --strict` has a driver to find for it. Until then
# the module skips and names the unexercised fixture — "not verified", never
# "passed".
pytestmark = pytest.mark.skipif(not _present(), reason=f"{FIXTURE} not in the spec repo yet (spec v1.27.0, #107)")


def _cases() -> list[dict[str, Any]]:
    return load_fixture(FIXTURE)["test_cases"] if _present() else []


def test_closed_key_set_matches_the_fixture() -> None:
    """The canonical set lives in the fixture, not in this SDK."""
    from apcore.acl import _RESERVED_RULE_KEYS, _RULE_KEYS

    fixture = load_fixture(FIXTURE)
    assert set(fixture["closed_rule_keys"]) == set(_RULE_KEYS)
    assert set(fixture["reserved_rule_keys"]) == set(_RESERVED_RULE_KEYS)


@pytest.mark.parametrize("case", _cases(), ids=lambda c: c["id"])
def test_acl_rule_key_closure(case: dict[str, Any]) -> None:
    doc = {"default_effect": case["default_effect"], "rules": [case["rule"]]}
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "acl.yaml"
        path.write_text(yaml.safe_dump(doc))

        if case["expected_load"] == "ok":
            acl = ACL.load(str(path))
            assert len(acl.rules) == 1, case["note"]
        else:
            with pytest.raises(ACLRuleError) as excinfo:
                ACL.load(str(path))
            offending = set(case["rule"]) - set(load_fixture(FIXTURE)["closed_rule_keys"])
            message = str(excinfo.value)
            for key in offending:
                assert key in message, f"{case['note']}\n  message did not name '{key}': {message}"
