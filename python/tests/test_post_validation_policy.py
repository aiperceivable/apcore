"""Cover Python-specific dispatch and cancellation invariants for final policy."""

from __future__ import annotations

import asyncio
import time
from typing import Any

import pytest

import apcore._post_validation_policy as post_validation_policy
from apcore import (
    ACL,
    ApprovalPendingError,
    ApprovalRequest,
    ApprovalResult,
    CancelToken,
    Context,
    Executor,
    ExtensionManager,
    Identity,
    ModuleAnnotations,
    ModuleError,
    PolicyEvaluationRequest,
    PolicyVerdict,
    Registry,
)
from apcore.middleware import Middleware
from apcore.pipeline import PipelineState, StepResult


class Operation:
    description = "Observe an invocation"
    input_schema = {"type": "object", "properties": {"amount": {"type": "number"}}}
    output_schema = {"type": "object"}

    def __init__(self) -> None:
        self.starts = 0
        self.previews = 0

    async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        self.starts += 1
        return inputs

    async def stream(self, inputs: dict[str, Any], context: Context) -> Any:
        self.starts += 1
        yield inputs

    def preview(self, inputs: dict[str, Any], context: Context) -> None:
        self.previews += 1


async def _registered() -> tuple[Registry, Operation]:
    registry = Registry()
    module = Operation()
    await registry.register_async("demo.observe", module, version="1.0.0")
    return registry, module


class Allow:
    async def evaluate(self, request: PolicyEvaluationRequest) -> PolicyVerdict:
        return PolicyVerdict("allow")


@pytest.mark.asyncio
async def test_denied_acl_prevents_provider_and_preview_disclosure() -> None:
    class DoNotRead:
        async def evaluate(self, request: PolicyEvaluationRequest) -> PolicyVerdict:
            pytest.fail("ACL denial must not disclose validated inputs to the evaluator")

    registry, module = await _registered()
    with Executor(registry, acl=ACL([]), policy_evaluator=DoNotRead()) as executor:
        report = await executor.validate_async("demo.observe", {"amount": 1})
        assert any(error["code"] == "ACL_DENIED" for error in report.errors)
        with pytest.raises(ModuleError) as denied:
            await executor.call_async("demo.observe", {"amount": 1})
        assert denied.value.code == "ACL_DENIED"
    assert module.starts == module.previews == 0


@pytest.mark.asyncio
@pytest.mark.parametrize("changed", ["arguments", "identity"])
async def test_module_preflight_and_preview_use_the_policy_snapshot(changed: str) -> None:
    evaluating, resume = asyncio.Event(), asyncio.Event()
    observed: list[tuple[int, str | None]] = []

    class Inspect(Operation):
        def preflight(self, inputs: dict[str, Any], context: Context) -> list[str]:
            observed.append((inputs["order"]["amount"], context.caller_id))
            inputs["order"]["amount"] = 30
            context.caller_id = "demo.other"
            return []

        def preview(self, inputs: dict[str, Any], context: Context) -> None:
            observed.append((inputs["order"]["amount"], context.caller_id))

    class Check:
        async def evaluate(self, request: PolicyEvaluationRequest) -> PolicyVerdict:
            evaluating.set()
            await resume.wait()
            return PolicyVerdict("allow" if request.arguments["order"]["amount"] == 1 else "deny")

    registry = Registry()
    await registry.register_async("demo.observe", Inspect())
    arguments = {"order": {"amount": 1}}
    identity = Identity(id="operator", type="user", attrs={"team": "review"})
    with Executor(registry, policy_evaluator=Check()) as executor:
        pending = asyncio.create_task(executor.validate_async("demo.observe", arguments, Context.create(identity)))
        await asyncio.wait_for(evaluating.wait(), 1)
        if changed == "arguments":
            arguments["order"]["amount"] = 20
        else:
            identity.attrs["team"] = "another"
        resume.set()
        report = await pending

    if changed == "arguments":
        assert report.valid
        assert observed == [(1, None), (1, None)]
    else:
        assert not report.valid
        assert any(error["code"] == "POLICY_EVALUATION_ERROR" for error in report.errors)
        assert observed == []


@pytest.mark.asyncio
@pytest.mark.parametrize("action", ["skip", "mutate", "recover"])
async def test_step_middleware_cannot_bypass_the_final_gate(action: str) -> None:
    class Tamper:
        def after_step(self, name: str, state: PipelineState, result: StepResult) -> None:
            if action == "skip" and name == "middleware_before":
                result.action, result.skip_to = "skip_to", "return_result"
            if action == "mutate" and name == "post_validation_policy":
                assert state.context.validated_inputs is not None
                state.context.validated_inputs["amount"] = 0

        def on_step_error(self, name: str, state: PipelineState, error: Exception) -> dict[str, int]:
            return {"recovered": 1}

    class DenyIfRecovering:
        async def evaluate(self, request: PolicyEvaluationRequest) -> PolicyVerdict:
            return PolicyVerdict("deny" if action == "recover" else "allow")

    class Recover(Middleware):
        def on_error(self, *args: Any) -> dict[str, int]:
            return {"Recovered": 1}

    registry, module = await _registered()
    with Executor(registry, policy_evaluator=DenyIfRecovering(), middlewares=[Recover()]) as executor:
        executor.current_strategy.add_step_middleware(Tamper())
        with pytest.raises(ModuleError) as rejected:
            await executor.call_async("demo.observe", {"amount": 1})
        assert rejected.value.code == ("POLICY_DENIED" if action == "recover" else "POLICY_EVALUATION_ERROR")
    assert module.starts == 0


@pytest.mark.asyncio
async def test_extension_applies_only_to_its_executor_and_requires_safe_strategy() -> None:
    class Deny:
        async def evaluate(self, request: PolicyEvaluationRequest) -> PolicyVerdict:
            return PolicyVerdict("deny")

    registry, module = await _registered()
    manager = ExtensionManager()
    manager.register("policy_evaluator", Deny())
    with Executor(registry) as configured, Executor(registry) as other:
        manager.apply(registry, configured)
        with pytest.raises(ModuleError) as rejected:
            await configured.call_async("demo.observe", {"amount": 1})
        assert rejected.value.code == "POLICY_DENIED"
        with pytest.raises(ModuleError) as incompatible:
            await configured.call_async_with_trace("demo.observe", {"amount": 1}, strategy="minimal")
        assert incompatible.value.code == "PIPELINE_CONFIGURATION_ERROR"
        assert module.starts == 0
        assert await other.call_async("demo.observe", {"amount": 1}) == {"amount": 1}


@pytest.mark.asyncio
async def test_unversioned_handler_cannot_authorize_final_resumption() -> None:
    class Legacy:
        async def request_approval(self, request: ApprovalRequest) -> ApprovalResult:
            return ApprovalResult("pending", approval_id="legacy-approval")

        async def check_approval(self, approval_id: str) -> ApprovalResult:
            pytest.fail("A status lookup cannot authorize an invocation")

        async def check_approval_for_call(self, approval_id: str, request: ApprovalRequest) -> ApprovalResult:
            pytest.fail("A legacy scope cannot authorize a final credential")

    registry, module = await _registered()
    module.annotations = ModuleAnnotations(requires_approval=True)
    with Executor(registry, policy_evaluator=Allow(), approval_handler=Legacy()) as executor:
        for args in ({"amount": 1}, {"amount": 1, "_approval_token": "legacy-approval"}):
            with pytest.raises(ModuleError) as rejected:
                await executor.call_async("demo.observe", args)
            assert rejected.value.code == "APPROVAL_DENIED"
            assert "legacy-approval" not in str(rejected.value.to_dict())
    assert module.starts == 0


@pytest.mark.asyncio
async def test_final_scope_uses_lookup_version_and_preserves_native_numeric_kinds() -> None:
    class Required:
        async def evaluate(self, request: PolicyEvaluationRequest) -> PolicyVerdict:
            return PolicyVerdict("require_approval")

    class Pending:
        supports_final_scope = True

        def __init__(self) -> None:
            self.scope: tuple[str | None, str | None] | None = None

        async def request_approval(self, request: ApprovalRequest) -> ApprovalResult:
            self.scope = request.module_version, request.canonical_args_digest
            return ApprovalResult("pending", approval_id="final-approval")

        async def check_approval(self, approval_id: str) -> ApprovalResult:
            pytest.fail("A status lookup cannot authorize an invocation")

        async def check_approval_for_call(self, approval_id: str, request: ApprovalRequest) -> ApprovalResult:
            if approval_id == "final-approval" and self.scope == (
                request.module_version,
                request.canonical_args_digest,
            ):
                self.scope = None
                return ApprovalResult("approved")
            return ApprovalResult("rejected")

    registry, first = await _registered()
    second = Operation()
    await registry.register_async("demo.observe", second, version="2.0.0")
    handler = Pending()
    with Executor(registry, policy_evaluator=Required(), approval_handler=handler) as executor:
        with pytest.raises(ApprovalPendingError) as pending:
            await executor.call_async("demo.observe", {"amount": -0.0}, version_hint="1.0.0")
        assert pending.value.approval_id == "final-approval"
        assert handler.scope is not None and handler.scope[0] == "1.0.0"

        resume = {"amount": -0.0, "_approval_token": pending.value.approval_id}
        for args, version in ((resume, "2.0.0"), (dict(resume, amount=0.0), "1.0.0")):
            with pytest.raises(ModuleError) as rejected:
                await executor.call_async("demo.observe", args, version_hint=version)
            assert rejected.value.code == "APPROVAL_DENIED"
        await executor.call_async("demo.observe", resume, version_hint="1.0.0")

    assert first.starts == 1 and second.starts == 0


@pytest.mark.asyncio
@pytest.mark.parametrize("entry", ["call", "stream"])
@pytest.mark.parametrize("end", ["timeout", "cancellation"])
async def test_evaluator_timeout_and_cancellation_remain_distinct(entry: str, end: str) -> None:
    evaluating = asyncio.Event()

    class Wait:
        async def evaluate(self, request: PolicyEvaluationRequest) -> PolicyVerdict:
            evaluating.set()
            await asyncio.Future()
            pytest.fail("The policy evaluator should be bounded")

    registry, module = await _registered()
    token = CancelToken()
    deadline = time.time() + 0.25 if end == "timeout" else None
    context = Context.create(cancel_token=token, global_deadline=deadline)
    with Executor(registry, policy_evaluator=Wait()) as executor:
        if entry == "call":
            pending = asyncio.create_task(executor.call_async("demo.observe", {"amount": 1}, context))
        else:
            stream = executor.stream_with_completion("demo.observe", {"amount": 1}, context)
            pending = asyncio.ensure_future(anext(stream.chunks))
        await asyncio.wait_for(evaluating.wait(), timeout=1)
        if end == "cancellation":
            token.cancel()
        with pytest.raises(ModuleError) as rejected:
            await asyncio.wait_for(pending, timeout=2)
        expected = "EXECUTION_CANCELLED" if end == "cancellation" else "POLICY_EVALUATION_ERROR"
        assert rejected.value.code == expected
        if entry == "stream":
            assert (await stream.completion()).error_code == expected
    assert module.starts == 0


@pytest.mark.asyncio
@pytest.mark.parametrize("entry", ["call", "stream"])
async def test_late_non_yielding_policy_cannot_authorize_dispatch(entry: str, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(post_validation_policy, "_EVALUATOR_TIMEOUT_SECONDS", 0.02)

    class LateAllow:
        async def evaluate(self, request: PolicyEvaluationRequest) -> PolicyVerdict:
            time.sleep(0.05)
            return PolicyVerdict("allow")

    registry, module = await _registered()
    with Executor(registry, policy_evaluator=LateAllow()) as executor:
        with pytest.raises(ModuleError) as rejected:
            if entry == "call":
                await executor.call_async("demo.observe", {"amount": 1})
            else:
                stream = executor.stream_with_completion("demo.observe", {"amount": 1})
                await anext(stream.chunks)
        assert rejected.value.code == "POLICY_EVALUATION_ERROR"
        if entry == "stream":
            assert (await stream.completion()).error_code == "POLICY_EVALUATION_ERROR"
    assert module.starts == 0
