"""Drive the canonical identity delegation fixture through public entry points."""

from __future__ import annotations

from typing import Any

import pytest

from apcore.acl import ACL, ACLRule
from apcore.context import Context
from apcore.errors import InvalidInputError

from .canonical_fixtures import load_fixture

CASES = load_fixture("identity_delegation.json")["test_cases"]


@pytest.mark.asyncio
@pytest.mark.parametrize("case", CASES, ids=[case["id"] for case in CASES])
async def test_identity_delegation(case: dict[str, Any]) -> None:
    inputs, expected = case["input"], case["expected"]
    if not expected["valid"]:
        with pytest.raises(InvalidInputError):
            Context.deserialize({"identity": inputs["identity"]})
        return

    context = Context.deserialize({"identity": inputs["identity"]})
    assert context.identity is not None
    assert context.identity.delegation_depth == expected["depth"]
    assert context.identity.root_subject.id == expected["subject_id"]
    if "effect" in expected:
        acl = ACL(
            [ACLRule(**rule) for rule in inputs["rules"]],
            default_effect=inputs.get("default_effect", "deny"),
        )
        assert acl.check_access(None, "delegation.target", context).access == expected["effect"]
        assert (await acl.async_check_access(None, "delegation.target", context)).access == expected["effect"]
        assert acl.evaluate_visibility(None, "delegation.target", context).visible is (expected["effect"] == "allow")
