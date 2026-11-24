"""Integration tests for ACL enforcement through the Executor."""

from __future__ import annotations

import pytest

from apcore.acl import ACL, ACLRule
from apcore.context import Context
from apcore.errors import ACLDeniedError
from apcore.executor import Executor


class TestACLEnforcement:
    """ACL enforcement integration tests."""

    def test_allowed_call_succeeds(self, int_acl_executor):
        result = int_acl_executor.call("greet", {"name": "Alice"})
        assert result == {"message": "Hello, Alice!"}

    def test_denied_call_raises_acl_denied_error(self, int_acl_executor):
        ctx = Context.create()
        ctx.caller_id = "unauthorized.caller"
        ctx.call_chain = ["unauthorized.caller"]
        with pytest.raises(ACLDeniedError):
            int_acl_executor.call("greet", {"name": "Alice"}, context=ctx)

    def test_acl_denied_error_contains_caller_and_target(self, int_registry):
        acl = ACL(rules=[], default_effect="deny")
        executor = Executor(registry=int_registry, acl=acl)
        ctx = Context.create()
        ctx.caller_id = "test.caller"
        ctx.call_chain = ["test.caller"]
        with pytest.raises(ACLDeniedError) as exc_info:
            executor.call("greet", {"name": "Alice"}, context=ctx)
        assert exc_info.value.target_id == "greet"

    def test_default_deny_when_no_rules_match(self, int_registry):
        acl = ACL(rules=[], default_effect="deny")
        executor = Executor(registry=int_registry, acl=acl)
        with pytest.raises(ACLDeniedError):
            executor.call("greet", {"name": "Alice"})

    def test_wildcard_patterns_match(self, int_registry):
        rule = ACLRule(callers=["*"], targets=["greet"], effect="allow")
        acl = ACL(rules=[rule], default_effect="deny")
        executor = Executor(registry=int_registry, acl=acl)
        result = executor.call("greet", {"name": "Alice"})
        assert result == {"message": "Hello, Alice!"}
