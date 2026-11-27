"""Exercise the public discovery APIs against the shared cases."""

from dataclasses import asdict
from typing import Any

import pytest

from apcore import Context, Executor, Identity, ModuleAnnotations, Registry
from apcore.acl import ACL, ACLRule
from conformance.canonical_fixtures import case_ids, load_fixture

FIXTURE = "capability_discovery.json"
INPUT_SCHEMA = {"type": "object", "properties": {"message": {"type": "string"}}}
OUTPUT_SCHEMA = {"type": "object", "properties": {"ok": {"type": "boolean"}}}


class Operation:
    input_schema = INPUT_SCHEMA
    output_schema = OUTPUT_SCHEMA
    version = "1.0.0"

    def __init__(self, row: dict[str, Any]) -> None:
        self.description = row["description"]
        self.tags = row.get("tags", [])
        self.annotations = ModuleAnnotations(
            discoverable=row.get("discoverable", True),
            readonly=row.get("readonly", False),
            requires_approval=row.get("requires_approval", False),
        )

    async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        pytest.fail("Discovery must not execute the module")


class Evaluator:
    async def evaluate(self, request: Any) -> Any:
        pytest.fail("Discovery must not evaluate final-input policy")


class Approval:
    async def request_approval(self, request: Any) -> Any:
        pytest.fail("Discovery must not request approval")


@pytest.mark.parametrize("case", load_fixture(FIXTURE)["test_cases"], ids=case_ids(FIXTURE))
def test_capability_discovery(case: dict[str, Any]) -> None:
    case_input, expected = case["input"], case["expected"]
    audits = []
    acl = ACL(rules=[ACLRule(**rule) for rule in case_input["rules"]], audit_logger=audits.append)
    registry = Registry()
    for row in load_fixture(FIXTURE)["driver_contract"]["modules"]:
        registry.register(row["module_id"], Operation(row))
    with Executor(
        registry,
        acl=acl,
        approval_handler=Approval(),
        policy_evaluator=Evaluator() if case_input.get("policy_evaluator") else None,
    ) as executor:
        context = Context.create(identity=Identity("fixture-user", roles=tuple(case_input.get("roles", []))))
        if caller_id := case_input.get("caller_id"):
            context = context.child(caller_id)
        page = executor.list_callable(context, **case_input.get("options", {}))
        definitions = executor.get_callable_definitions(
            ["demo.hidden", "demo.echo", "demo.review", "demo.unknown", "demo.echo"], context
        )
    assert [{"module_id": item.module_id, "access_hint": item.access_hint} for item in page.items] == expected["items"]
    for item in page.items:
        assert "input_schema" not in asdict(item)
        assert item.version == "1.0.0"
    assert len(audits) == expected["audit_entries"]
    assert [entry.module_id for entry in definitions] == expected.get(
        "definitions", [item["module_id"] for item in expected["items"]]
    )
    for entry in definitions:
        assert entry.input_schema == INPUT_SCHEMA
        assert entry.output_schema == OUTPUT_SCHEMA
