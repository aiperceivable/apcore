"""Exercise the policy gate through public execution entry points."""

from __future__ import annotations

from typing import Any

import pytest

from apcore import ApprovalRequest, ApprovalResult, Context, Executor, ModuleAnnotations, Registry
from apcore.errors import ApprovalPendingError, ModuleError
from apcore.middleware import Middleware
from apcore.policy import PolicyEvaluationRequest, PolicyVerdict
from conformance.canonical_fixtures import case_ids, load_fixture

FIXTURE = "post_validation_policy.json"


def _scope(request: ApprovalRequest) -> tuple[object, ...]:
    identity = request.context.identity
    return (
        (identity.type, identity.id) if identity else None,
        request.caller_id,
        request.module_id,
        request.module_version,
        request.canonical_args_digest,
    )


@pytest.mark.asyncio
@pytest.mark.parametrize("case", load_fixture(FIXTURE)["test_cases"], ids=case_ids(FIXTURE))
async def test_post_validation_policy(case: dict[str, Any]) -> None:
    contract = load_fixture(FIXTURE)["driver_contract"]
    module_id, version = contract["module_id"], contract["module_version"]
    starts: list[int] = []
    evaluated: list[int] = []
    approvals: list[int] = []
    resume_checks: list[str] = []
    token = "fixture-policy-approval"
    pending_scope: tuple[object, ...] | None = None
    pending_result = ApprovalResult("pending", approval_id=token)

    class Operation:
        description = "Test the final invocation"
        input_schema = contract["input_schema"]
        output_schema = {"type": "object"}
        annotations = ModuleAnnotations(requires_approval=case.get("requires_approval", False))

        async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
            assert "_approval_token" not in inputs
            starts.append(inputs["amount"])
            return {"amount": inputs["amount"]}

        async def stream(self, inputs: dict[str, Any], context: Context) -> Any:
            starts.append(inputs["amount"])
            yield {"amount": inputs["amount"]}

    class Evaluator:
        async def evaluate(self, request: PolicyEvaluationRequest) -> PolicyVerdict:
            assert request.module_version == version
            assert request.module_id == module_id
            assert "_approval_token" not in request.arguments
            assert "/secret" in request.sensitive_fields
            evaluated.append(request.arguments["amount"])
            if case["verdict"] == "throw":
                raise RuntimeError(contract["business_input"]["secret"])
            request.arguments["amount"] = -1
            return PolicyVerdict(case["verdict"])

    class Handler:
        supports_final_scope = True

        async def request_approval(self, request: ApprovalRequest) -> ApprovalResult:
            nonlocal pending_scope
            assert request.module_version == version
            assert request.canonical_args_digest is not None
            assert "_approval_token" not in request.arguments
            approvals.append(request.arguments["amount"])
            if case["entry"] == "resume":
                pending_scope = _scope(request)
                return pending_result
            return ApprovalResult("approved")

        async def check_approval(self, approval_id: str) -> ApprovalResult:
            raise AssertionError("Unscoped approval status cannot authorize a call")

        async def check_approval_for_call(self, approval_id: str, request: ApprovalRequest) -> ApprovalResult:
            nonlocal pending_scope
            resume_checks.append(approval_id)
            if approval_id == token and pending_scope == _scope(request):
                pending_scope = None
                return ApprovalResult("approved")
            return ApprovalResult("rejected")

    class Rewrite(Middleware):
        async def before(self, module_id: str, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
            assert "_approval_token" not in inputs
            return {**inputs, "amount": case["rewrite_amount"]}

    registry = Registry()
    await registry.register_async(module_id, Operation(), version=version)
    executor = Executor(registry, policy_evaluator=Evaluator(), approval_handler=Handler())
    if "rewrite_amount" in case:
        executor.use(Rewrite())

    supplied = {**contract["business_input"], **case.get("inputs", {})}
    result = None
    error = None
    try:
        if case["entry"] == "preflight":
            result = await executor.validate_async(module_id, {**supplied, "_approval_token": token})
        elif case["entry"] == "stream":
            async for _ in executor.stream(module_id, supplied):
                pass
        elif case["entry"] == "resume":
            with pytest.raises(ApprovalPendingError) as pending:
                await executor.call_async(module_id, supplied)
            assert pending.value.result is pending_result
            assert pending.value.approval_id == token
            await executor.call_async(module_id, {**supplied, **case.get("retry_inputs", {}), "_approval_token": token})
        else:
            await executor.call_async(module_id, supplied)
    except ModuleError as exc:
        error = exc
    finally:
        executor.close()

    expected = case["expected"]
    assert len(starts) == expected["module_starts"]
    assert (evaluated[-1] if evaluated else None) == expected["evaluated_amount"]
    assert len(approvals) == expected["approval_requests"]
    if starts:
        assert starts == [expected["evaluated_amount"]]
    if "approved_amount" in expected:
        assert approvals == [expected["approved_amount"]]
    if "requires_approval" in expected:
        assert result is not None and result.valid
        assert result.requires_approval is expected["requires_approval"]
    if "resume_checks" in expected:
        assert resume_checks == [token] * expected["resume_checks"]
    if "error_code" in expected:
        assert error is not None and error.code == expected["error_code"]
        assert contract["business_input"]["secret"] not in str(error.to_dict())
    elif not evaluated:
        assert error is not None
    else:
        assert error is None
