"""Conformance driver for ``preflight_disclosure.json`` (PROTOCOL_SPEC §12.8.5.1).

``Executor.validate()`` MUST NOT disclose module-level introspection to a caller
the ACL denied. ``preflight()`` and ``preview()`` are module-authored code whose
output names what the call would do — the resolved binary and argv of a
command-wrapping module, the target of a write. Module lookup is Step 3 and the
ACL check is Step 4, so gating those hooks on "lookup succeeded" alone runs them
for a denied caller and returns what they said.

Per the fixture's ``driver_contract`` this drives the real ``Executor.validate()``
against a real ``Registry`` and a real ``ACL``: the defect lives in ``validate()``'s
own gating, so a driver that assembles a ``PreflightResult`` itself asserts nothing.
"""

from __future__ import annotations

import json
from typing import Any

import pytest

from apcore.acl import ACL, ACLRule
from apcore.config import Config
from apcore.context import Context, Identity
from apcore.executor import Executor
from apcore.module import Change, PreviewResult
from apcore.registry import Registry
from apcore.schema.loader import SchemaLoader
from conformance.canonical_fixtures import load_fixture

_FIXTURE = load_fixture("preflight_disclosure.json")
_CONTRACT: dict[str, Any] = _FIXTURE["module_contract"]
_SENTINEL: str = _CONTRACT["sentinel"]

# `Module` is a Protocol whose `input_schema` is a Pydantic model class, so the
# fixture's JSON Schema goes through the same converter a real module's contract
# does — otherwise the schema case below would validate nothing and pass for the
# wrong reason.
_LOADER = SchemaLoader(Config({}))
_INPUT_SCHEMA = _LOADER.generate_model(_CONTRACT["input_schema"], "PreflightDisclosureInput")
_OUTPUT_SCHEMA = _LOADER.generate_model(_CONTRACT["output_schema"], "PreflightDisclosureOutput")


class _DestructiveModule:
    """The fixture's ``module_contract``, with an invocation recorder attached.

    ``hooks_invoked`` is observed inside the hook bodies rather than inferred
    from the absent check entries: an implementation that calls the hooks and
    then drops their results still ran module code for a denied caller, which is
    the side-effect half of the requirement.
    """

    input_schema = _INPUT_SCHEMA
    output_schema = _OUTPUT_SCHEMA
    description = "Conformance module for the preflight disclosure gate"

    def __init__(self) -> None:
        self.hooks_invoked: list[str] = []

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        raise AssertionError("validate() must never execute the module body")

    def preflight(self, inputs: dict[str, Any], context: Context) -> list[str]:
        self.hooks_invoked.append("preflight")
        return list(_CONTRACT["preflight_returns"])

    def preview(self, inputs: dict[str, Any], context: Context) -> PreviewResult:
        self.hooks_invoked.append("preview")
        return PreviewResult(changes=[Change(**_CONTRACT["preview_change"])])


def _run_case(tc: dict[str, Any]) -> tuple[Any, _DestructiveModule]:
    spec = tc["input"]
    registry = Registry(Config({}))
    module = _DestructiveModule()
    registry.register(_CONTRACT["module_id"], module)

    acl = ACL(
        rules=[ACLRule(**rule) for rule in spec["acl_rules"]],
        default_effect=spec["default_effect"],
    )
    executor = Executor(registry, acl=acl, config=Config({}))
    context = Context.create(identity=Identity(id=spec["caller_id"], type="module"))

    result = executor.validate(_CONTRACT["module_id"], spec["inputs"], context)
    return result, module


@pytest.mark.parametrize("tc", _FIXTURE["test_cases"], ids=lambda tc: tc["id"])
def test_preflight_disclosure(tc: dict[str, Any]) -> None:
    expected = tc["expected"]
    result, module = _run_case(tc)

    names = [c.check for c in result.checks]
    detail = (
        f"\n  case: {tc['id']}"
        f"\n  description: {tc.get('description', '(none)')}"
        f"\n  checks: {[(c.check, c.passed) for c in result.checks]}"
        f"\n  hooks_invoked: {module.hooks_invoked}"
    )

    assert result.valid is expected["valid"], f"valid mismatch{detail}"

    for name in expected["checks_present"]:
        assert name in names, f"check '{name}' MUST be present{detail}"

    # Absence is asserted on the check list itself: a present-but-empty
    # `module_preflight` entry is already the disclosure that the module
    # implements the hook.
    for name in expected["checks_absent"]:
        assert name not in names, f"check '{name}' MUST NOT be present{detail}"

    failed = sorted(c.check for c in result.checks if not c.passed)
    assert failed == sorted(expected["failed_checks"]), f"failed-check set mismatch{detail}"

    assert (
        len(result.predicted_changes) == expected["predicted_changes_count"]
    ), f"predicted_changes count mismatch{detail}"

    assert (
        module.hooks_invoked == expected["hooks_invoked"]
    ), f"module hook invocation mismatch — the hooks themselves must not run{detail}"

    # The sentinel appears in no value the Executor computes on its own, so
    # finding it anywhere in the serialized result proves introspection reached
    # the caller.
    serialized = json.dumps(
        {
            "checks": [
                {"check": c.check, "passed": c.passed, "error": c.error, "warnings": c.warnings} for c in result.checks
            ],
            "predicted_changes": [c.model_dump() if hasattr(c, "model_dump") else c for c in result.predicted_changes],
        },
        default=str,
    )
    if expected["sentinel_absent"]:
        assert (
            _SENTINEL not in serialized
        ), f"sentinel {_SENTINEL!r} leaked to a denied caller{detail}\n  serialized: {serialized}"
    else:
        assert _SENTINEL in serialized, (
            f"control case: sentinel {_SENTINEL!r} MUST reach a permitted caller, "
            f"otherwise the denial cases pass for an implementation that never "
            f"introspects at all{detail}"
        )
