"""The executor's ACL step takes the ASYNC path (spec v1.50.0, D-105).

§6.1.3 defines what each ACL entry point resolves and never said which one the
pipeline calls. apcore-rust called the synchronous `check_access` from inside an
already-`async` step, which makes any condition registered through
`register_async_condition` "async only" on that path — it resolves to
UNEVALUABLE, so an `allow` rule carrying it stops granting and a `deny` rule
carrying it denies unconditionally. Both directions wrong, and the entire async
condition registry unreachable from the only path that enforces.

All three SDKs take the async path today. Nothing said so: a whole extension
point being reachable from every door except the enforcing one is invisible from
the door, and a registry that accepts a handler is not evidence anything calls it.

The discriminator is a SYNC handler answering False and an ASYNC handler
answering True for the same condition key. Counting invocations would not
separate the paths — both invoke *a* handler. Only the verdict does.
"""

from __future__ import annotations

from typing import Any

import pytest

from apcore.acl import ACL, ACLRule
from apcore.acl_handlers import AsyncACLConditionHandler, SyncACLConditionHandler
from apcore.config import Config
from apcore.errors import ACLDeniedError
from apcore.executor import Executor
from apcore.registry import Registry

MODULE_ID = "executor.probe.async_acl"
# One key per test. `ACL.register_condition` / `register_async_condition` are
# CLASSMETHODS writing into class-level dicts, so a handler registered by one
# test is visible to every later ACL instance in the process. Sharing a key here
# let the first test's async handler decide the second one, and the control
# passed the call it was written to see denied.
CONDITION_ASYNC_WINS = "probe_async_only"
CONDITION_SYNC_ONLY = "probe_sync_only"


class _Module:
    input_schema: dict[str, Any] = {"type": "object"}
    output_schema: dict[str, Any] = {"type": "object"}
    description = "probe"

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        return {"ran": True}


class _SyncSaysNo(SyncACLConditionHandler):
    """Registered under the same key, and answers the opposite."""

    def evaluate(self, value: Any, context: Any) -> bool:
        return False


class _AsyncSaysYes(AsyncACLConditionHandler):
    async def evaluate(self, value: Any, context: Any) -> bool:
        return True


def _acl_with(condition: str, sync_handler: Any, async_handler: Any | None) -> ACL:
    acl = ACL()
    # The sync handler also satisfies the structural precheck, which rejects a
    # rule naming a condition no handler claims. Without it the rule would be
    # unevaluable for a second, unrelated reason and the case would pass for the
    # wrong one.
    acl.register_condition(condition, sync_handler)
    if async_handler is not None:
        acl.register_async_condition(condition, async_handler)
    acl.add_rule(ACLRule(callers=["*"], targets=[MODULE_ID], effect="allow", conditions={condition: True}))
    return acl


def _executor(acl: ACL) -> Executor:
    registry = Registry()
    registry.register(MODULE_ID, _Module())
    return Executor(registry=registry, config=Config({}), acl=acl)


@pytest.mark.asyncio
async def test_an_async_only_condition_decides_the_call() -> None:
    executor = _executor(_acl_with(CONDITION_ASYNC_WINS, _SyncSaysNo(), _AsyncSaysYes()))

    result = await executor.call_async(MODULE_ID, {})

    assert result == {"ran": True}, (
        "the ASYNC handler answers True and the allow rule grants. Taking the "
        "synchronous accessor reaches the sync handler, which answers False — the "
        "allow rule then does not grant and the call is denied by default, which "
        "is what apcore-rust did before D-105"
    )


@pytest.mark.asyncio
async def test_the_sync_handler_is_the_one_that_would_deny() -> None:
    # The control: it proves the two handlers genuinely disagree, so the test
    # above separates the paths rather than passing because the condition is
    # satisfied either way. With no async handler registered, the async path
    # falls back to the sync one and the call is denied.
    executor = _executor(_acl_with(CONDITION_SYNC_ONLY, _SyncSaysNo(), None))

    with pytest.raises(ACLDeniedError):
        await executor.call_async(MODULE_ID, {})
