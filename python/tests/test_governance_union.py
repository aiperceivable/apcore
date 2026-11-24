"""Every governance reader sees the same two sources (PROTOCOL_SPEC §7.4, D-96).

D-96 settled that the approval gate fires on the UNION of the governance
sources, and recorded that it was "unobservable in implementations whose
descriptors are derived from the module (apcore-python, apcore-typescript),
which is why the union costs them nothing."

**That was wrong, and this file is the proof.** These descriptors are not
derived — :func:`merge_module_metadata` folds a ``*.binding.yaml`` /
``metadata=`` declaration into them with YAML > code precedence, so an operator
has a second place to declare governance that the module instance never carries.
Every governance reader here read the instance alone, so a metadata source
declaring ``requires_approval: true`` reached ``get_definition()`` and the
manifest, and reached **no gate**: the module executed with an approval handler
configured and the handler never consulted.

Why a union rather than :func:`merge_annotations`' YAML > code precedence: the
merge lets the weaker declaration win in *both* directions. A metadata ``false``
would cancel a module that asks to be gated, and a metadata ``true`` reached
only the descriptor. Both are fail-OPEN, and on an approval gate the direction
is the whole argument — requiring an approval that was not needed costs a
prompt, skipping one that was needed is a bypass. The controls at the bottom pin
both directions.
"""

from __future__ import annotations

from typing import Any

import pytest

from apcore.approval import ApprovalHandler, ApprovalResult
from apcore.config import Config
from apcore.executor import Executor
from apcore.module import ModuleAnnotations
from apcore.registry import Registry
from apcore.sys_modules.manifest import ManifestModule

MODULE_ID = "executor.probe.governance"


class PlainModule:
    """Declares no governance in code. The operator declares it in metadata."""

    input_schema: dict[str, Any] = {"type": "object"}
    output_schema: dict[str, Any] = {"type": "object"}
    description = "declares nothing in code"

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        return {"ran": True}


class DeclaringModule:
    """Declares the requirement in code, so metadata must not be able to cancel it."""

    input_schema: dict[str, Any] = {"type": "object"}
    output_schema: dict[str, Any] = {"type": "object"}
    description = "declares requires_approval in code"
    annotations = ModuleAnnotations(requires_approval=True)

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        return {"ran": True}


class CountingHandler(ApprovalHandler):
    def __init__(self) -> None:
        self.calls = 0
        self.seen: ModuleAnnotations | None = None

    async def request_approval(self, request: Any) -> ApprovalResult:
        self.calls += 1
        self.seen = request.annotations
        return ApprovalResult(status="approved", approved_by="probe")

    async def check_approval(self, approval_id: str) -> ApprovalResult:
        return ApprovalResult(status="approved")


def _executor(registry: Registry, handler: ApprovalHandler | None = None) -> Executor:
    ex = Executor(registry=registry, config=Config({}))
    if handler is not None:
        ex.set_approval_handler(handler)
    return ex


class TestMetadataDeclaredGovernanceIsEnforced:
    @pytest.mark.asyncio
    async def test_the_gate_fires_on_a_metadata_declared_requirement(self) -> None:
        """The bypass: the module executed ungated with a handler configured."""
        registry = Registry()
        registry.register(MODULE_ID, PlainModule(), metadata={"annotations": {"requires_approval": True}})
        handler = CountingHandler()

        await _executor(registry, handler).call_async(MODULE_ID, {})

        assert handler.calls == 1, (
            "an operator who declares requires_approval in the metadata source gets a "
            "descriptor that says true, a manifest that says true, and — before this "
            "fix — no gate at all"
        )

    @pytest.mark.asyncio
    async def test_the_request_carries_the_metadata_declared_destructive_flag(self) -> None:
        """Gating on one source and describing the call from another is the D-96 split."""
        registry = Registry()
        registry.register(
            MODULE_ID,
            PlainModule(),
            metadata={"annotations": {"requires_approval": True, "destructive": True}},
        )
        handler = CountingHandler()

        await _executor(registry, handler).call_async(MODULE_ID, {})

        assert handler.seen is not None
        assert handler.seen.destructive is True, (
            "a handler routing by risk — auto-approve the safe ones, escalate the rest — "
            "takes the low-risk path when told destructive=False for a call the operator "
            "marked high-risk"
        )

    def test_preflight_reports_a_metadata_declared_requirement(self) -> None:
        """§7.9.5: the preflight reports the verdict the Step-5 gate will enforce."""
        registry = Registry()
        registry.register(MODULE_ID, PlainModule(), metadata={"annotations": {"requires_approval": True}})

        report = _executor(registry).validate(MODULE_ID, {})

        assert (
            report.requires_approval is True
        ), "reporting False here sends the caller into a gate it was told would not fire"

    def test_governance_state_reads_a_metadata_declared_requirement(self) -> None:
        """A serve-time adapter may refuse to start over this flag."""
        registry = Registry()
        registry.register_internal(
            "system.control.probe",
            PlainModule(),
        )
        registry._module_meta["system.control.probe"]["annotations"] = ModuleAnnotations(requires_approval=True)

        assert _executor(registry).governance_state().all_control_modules_require_approval is True

    def test_the_manifest_advertises_what_the_gate_enforces(self) -> None:
        """The manifest is what an agent reads to decide whether to call.

        The discriminating case for this SDK is a metadata ``false`` over a code
        ``true``: ``merge_annotations`` gives the descriptor ``false`` by
        YAML > code precedence, while the gate — reading the union — fires. A
        manifest projected off the descriptor alone therefore advertised
        ``requires_approval: false`` for a call that will be stopped.
        """
        registry = Registry()
        registry.register(
            MODULE_ID,
            DeclaringModule(),
            metadata={"annotations": {"requires_approval": False}},
        )
        assert (
            registry.get_definition(MODULE_ID).annotations.requires_approval is False
        ), "precondition: the descriptor really does carry the lowered value"

        entry = ManifestModule(registry).execute({"module_id": MODULE_ID}, None)

        assert entry["annotations"]["requires_approval"] is True, (
            "the gate fires on this module; a manifest that advertises "
            "requires_approval=False for it is worse than one that says nothing"
        )


class TestTheUnionIsAUnion:
    """Both directions, so the fix cannot be read as 'the descriptor wins'."""

    @pytest.mark.asyncio
    async def test_metadata_false_does_not_cancel_a_code_declared_requirement(self) -> None:
        registry = Registry()
        registry.register(
            MODULE_ID,
            DeclaringModule(),
            metadata={"annotations": {"requires_approval": False}},
        )
        handler = CountingHandler()

        await _executor(registry, handler).call_async(MODULE_ID, {})

        assert handler.calls == 1, (
            "merge_annotations' YAML > code precedence is right for a DESCRIPTOR and "
            "wrong for a gate: it would let a metadata false cancel a module that asks "
            "to be gated"
        )

    @pytest.mark.asyncio
    async def test_no_gate_when_neither_source_declares_one(self) -> None:
        """The control — without it, a hardcoded True would pass every test above."""
        registry = Registry()
        registry.register(MODULE_ID, PlainModule())
        handler = CountingHandler()

        await _executor(registry, handler).call_async(MODULE_ID, {})

        assert handler.calls == 0
        assert _executor(registry).validate(MODULE_ID, {}).requires_approval is False
