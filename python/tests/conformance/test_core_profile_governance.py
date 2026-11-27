"""Drive default Core Profile approval through the public client API."""

from __future__ import annotations

from typing import Any

import pytest

from apcore.acl import ACL, ACLRule
from apcore.client import APCore
from apcore.context import Context
from apcore.errors import ModuleError
from apcore.executor import Executor
from apcore.module import ModuleAnnotations
from apcore.policy import ExecutionPolicy
from apcore.registry import Registry
from conformance.canonical_fixtures import case_ids, load_fixture

FIXTURE = "core_profile_governance.json"


@pytest.mark.asyncio
@pytest.mark.parametrize("case", load_fixture(FIXTURE)["test_cases"], ids=case_ids(FIXTURE))
async def test_approval_defaults_follow_preflight(case: dict[str, Any]) -> None:
    module_id = "example.operation"
    source = case["input"]["approval_source"]
    policy_data = case["input"].get("execution_policy")
    if source == "policy_rule":
        policy_data = {"rules": [{"pattern": module_id, "requires_approval": True}]}
    policy = ExecutionPolicy.from_dict(policy_data) if policy_data is not None else None
    if case["input"]["client"] == "injected_executor":
        client = APCore(executor=Executor(Registry(), policy=policy))
    else:
        client = APCore(policy=policy)

    if source == "acl":
        client.executor.set_acl(ACL([ACLRule(["@external"], [module_id], "allow", approval="required")]))

    calls = 0

    @client.module(
        id=module_id,
        annotations=ModuleAnnotations(requires_approval=source == "annotation", destructive=source == "destructive"),
    )
    async def operation(context: Context) -> int:
        nonlocal calls
        calls += 1
        return calls

    expected = case["expected"]
    try:
        preflight = await client.validate_async(module_id)
        assert preflight.valid
        assert preflight.requires_approval is expected["requires_approval"]
        assert calls == 0
        if expected["error_code"] is None:
            assert await client.call_async(module_id) == {"result": 1}
        else:
            with pytest.raises(ModuleError) as caught:
                await client.call_async(module_id)
            assert caught.value.code == expected["error_code"]
        assert calls == expected["executions"]
    finally:
        client.close()
