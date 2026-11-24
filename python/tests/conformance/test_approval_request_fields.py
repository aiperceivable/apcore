"""Drive `approval_request_fields.json` — D-03's two `ApprovalRequest` fields.

Spec decision D-03 (`docs/spec/2026-05-decision-log.md`, PROTOCOL_SPEC §7.3.1):
`ApprovalRequest` carries `caller_id` and `action`, populated by
`BuiltinApprovalGate` at Executor Step 4.5 from the Context and module ID
already in scope at that call site — `caller_id = context.caller_id`, `action =
module_id`.

The two cases pin the halves that are easy to get wrong in opposite directions:

* **nested call** — `action` is the *target*, not a handler-supplied label, and
  `caller_id` is the invoking module.
* **top-level call** — `caller_id` is `None`, NOT the `"@external"` sentinel ACL
  evaluation substitutes internally. §5.7 makes `Context.caller_id` `None` until
  `Context.child()` sets it, and the gate reads it with no substitution; a
  handler that treats `caller_id` as "who asked" would otherwise be told an
  external call came from a module named `@external`.

`tests/test_approval_executor.py` already asserts both by hand. A hand copy
cannot notice when the canonical fixture gains a case, which is why this driver
reads the fixture itself and fails on an unknown case id.

Per the fixture's `driver_contract.no_wire_assertion`, both fields are read off
the in-process `ApprovalRequest` the handler was handed, never a round-tripped
copy: Rust's `ApprovalRequest` skips `context` during serialization and neither
field has a wire-format fixture elsewhere.
"""

from __future__ import annotations

from typing import Any

import pytest
from pydantic import BaseModel, ConfigDict

from apcore.approval import ApprovalRequest, ApprovalResult, CallbackApprovalHandler
from apcore.context import Context
from apcore.executor import Executor
from apcore.module import ModuleAnnotations
from apcore.registry import Registry

from .canonical_fixtures import load_fixture, reject_unknown_expectations

FIXTURE = "approval_request_fields.json"
CASES: list[dict[str, Any]] = load_fixture(FIXTURE)["test_cases"]

#: The expectation keys this driver reads. A fixture that grows a third one
#: fails here rather than being driven as though the new key did not exist.
_KNOWN_EXPECTATIONS = {"expected_request_caller_id", "expected_request_action"}


class _Permissive(BaseModel):
    model_config = ConfigDict(extra="allow")


class _GatedModule:
    """The target: requires approval, so Step 4.5 builds an ApprovalRequest."""

    input_schema = _Permissive
    output_schema = _Permissive
    annotations = ModuleAnnotations(requires_approval=True)
    description = "gated target for the D-03 caller_id/action contract"
    tags: list[str] = []

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"status": "executed"}


def _caller_module(target_id: str) -> Any:
    """A module that invokes *target_id* through its own Context.

    The nested call is made the way a real module makes one — through
    ``context.executor``, so ``Context.child()`` sets ``caller_id`` — rather
    than by hand-building a Context with the field already set. A driver that
    pre-set the field would pass against a gate that read anything at all off
    the context it was handed.
    """

    class _CallerModule:
        input_schema = _Permissive
        output_schema = _Permissive
        description = "caller for the D-03 nested-call case"

        def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
            return context.executor.call(target_id, inputs, context)  # type: ignore[no-any-return]

    return _CallerModule()


@pytest.mark.parametrize("case", CASES, ids=[c["id"] for c in CASES])
def test_approval_request_fields(case: dict[str, Any]) -> None:
    reject_unknown_expectations(FIXTURE, case, _KNOWN_EXPECTATIONS)

    captured: list[ApprovalRequest] = []

    async def _record(request: ApprovalRequest) -> ApprovalResult:
        captured.append(request)
        return ApprovalResult(status="approved", approved_by="recorder")

    target_id = case["target_id"]
    caller_id = case["caller_id"]

    registry = Registry()
    registry.register(target_id, _GatedModule())
    executor = Executor(registry=registry, approval_handler=CallbackApprovalHandler(_record))

    if caller_id is None:
        # Top-level: a fresh Context that never passed through child().
        executor.call(target_id, {})
    else:
        registry.register(caller_id, _caller_module(target_id))
        executor.call(caller_id, {})

    assert len(captured) == 1, f"[{FIXTURE} :: {case['id']}] the gate must reach the handler exactly once"
    request = captured[0]

    assert (
        request.caller_id == case["expected_request_caller_id"]
    ), f"[{FIXTURE} :: {case['id']}] caller_id: {request.caller_id!r} != {case['expected_request_caller_id']!r}"
    assert (
        request.action == case["expected_request_action"]
    ), f"[{FIXTURE} :: {case['id']}] action: {request.action!r} != {case['expected_request_action']!r}"
