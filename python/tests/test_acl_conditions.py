"""Tests for the ACL conditions redesign — handler registry, dispatch, and compound operators."""

from __future__ import annotations

import asyncio
import logging
from typing import Any

import pytest

from apcore.acl import ACL, ACLRule
from apcore.acl_handlers import (
    SyncACLConditionHandler,
    _IdentityTypesHandler,
    _MaxCallDepthHandler,
    _RolesHandler,
)
from apcore.context import Context, Identity


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _make_context(
    identity_type: str = "user",
    roles: list[str] | None = None,
    call_chain: list[str] | None = None,
) -> Context:
    identity = Identity(id="test-user", type=identity_type, roles=tuple(roles or ()))
    ctx = Context.create(identity=identity)
    ctx.call_chain = call_chain or []
    return ctx


def _make_acl_with_condition(condition_key: str, condition_value: Any, effect: str = "allow") -> ACL:
    rule = ACLRule(
        callers=["*"],
        targets=["*"],
        effect=effect,
        conditions={condition_key: condition_value},
    )
    return ACL(rules=[rule], default_effect="deny")


# ---------------------------------------------------------------------------
# Handler Protocol conformance
# ---------------------------------------------------------------------------


class TestHandlerProtocol:
    def test_sync_handler_is_runtime_checkable(self) -> None:
        handler = _IdentityTypesHandler()
        assert isinstance(handler, SyncACLConditionHandler)

    def test_builtin_handlers_conform_to_sync_protocol(self) -> None:
        for handler_cls in (_IdentityTypesHandler, _RolesHandler, _MaxCallDepthHandler):
            handler = handler_cls()
            assert isinstance(handler, SyncACLConditionHandler)


# ---------------------------------------------------------------------------
# Handler Registry
# ---------------------------------------------------------------------------


class TestHandlerRegistry:
    def test_register_condition_adds_handler(self) -> None:
        class _CustomHandler:
            def evaluate(self, value: Any, context: Context) -> bool:
                return value is True

        ACL.register_condition("_test_custom", _CustomHandler())
        assert "_test_custom" in ACL._condition_handlers
        # Cleanup
        del ACL._condition_handlers["_test_custom"]

    def test_register_same_key_replaces_handler(self) -> None:
        """AC-031: Handler replacement."""
        calls: list[str] = []

        class _HandlerA:
            def evaluate(self, value: Any, context: Context) -> bool:
                calls.append("A")
                return True

        class _HandlerB:
            def evaluate(self, value: Any, context: Context) -> bool:
                calls.append("B")
                return True

        ACL.register_condition("_test_replace", _HandlerA())
        ACL.register_condition("_test_replace", _HandlerB())

        ctx = _make_context()
        acl = _make_acl_with_condition("_test_replace", True)
        acl.check("caller", "target", context=ctx)

        assert calls == ["B"]
        # Cleanup
        del ACL._condition_handlers["_test_replace"]

    def test_builtin_handlers_auto_registered(self) -> None:
        for key in ("identity_types", "roles", "max_call_depth", "$or", "$not"):
            assert key in ACL._condition_handlers, f"Built-in handler '{key}' not registered"


# ---------------------------------------------------------------------------
# Built-in Handlers — Unit Tests
# ---------------------------------------------------------------------------


class TestIdentityTypesHandler:
    def test_match_when_type_in_list(self) -> None:
        handler = _IdentityTypesHandler()
        ctx = _make_context(identity_type="service")
        assert handler.evaluate(["service", "admin"], ctx) is True

    def test_no_match_when_type_not_in_list(self) -> None:
        handler = _IdentityTypesHandler()
        ctx = _make_context(identity_type="user")
        assert handler.evaluate(["service", "admin"], ctx) is False

    def test_no_match_when_identity_none(self) -> None:
        handler = _IdentityTypesHandler()
        ctx = Context.create()
        assert handler.evaluate(["user"], ctx) is False

    def test_no_match_when_value_not_list(self) -> None:
        handler = _IdentityTypesHandler()
        ctx = _make_context()
        assert handler.evaluate("user", ctx) is False


class TestRolesHandler:
    def test_match_when_role_overlaps(self) -> None:
        handler = _RolesHandler()
        ctx = _make_context(roles=["admin", "viewer"])
        assert handler.evaluate(["admin"], ctx) is True

    def test_no_match_when_no_overlap(self) -> None:
        handler = _RolesHandler()
        ctx = _make_context(roles=["viewer"])
        assert handler.evaluate(["admin"], ctx) is False

    def test_no_match_when_identity_none(self) -> None:
        handler = _RolesHandler()
        ctx = Context.create()
        assert handler.evaluate(["admin"], ctx) is False

    def test_no_match_when_value_not_list(self) -> None:
        handler = _RolesHandler()
        ctx = _make_context(roles=["admin"])
        assert handler.evaluate(42, ctx) is False


class TestMaxCallDepthHandler:
    def test_within_limit(self) -> None:
        handler = _MaxCallDepthHandler()
        ctx = _make_context(call_chain=["a", "b"])
        assert handler.evaluate(5, ctx) is True

    def test_at_limit(self) -> None:
        handler = _MaxCallDepthHandler()
        ctx = _make_context(call_chain=["a", "b", "c"])
        assert handler.evaluate(3, ctx) is True

    def test_exceeds_limit(self) -> None:
        handler = _MaxCallDepthHandler()
        ctx = _make_context(call_chain=["a", "b", "c", "d"])
        assert handler.evaluate(3, ctx) is False

    def test_no_match_when_value_not_int(self) -> None:
        handler = _MaxCallDepthHandler()
        ctx = _make_context()
        assert handler.evaluate("5", ctx) is False

    def test_bool_true_does_not_match_fail_closed(self) -> None:
        """A-D-012: ``max_call_depth: true`` must NOT match (fail closed).

        Python ``bool`` is a subclass of ``int``; ``True`` coerces to 1, which
        would otherwise ALLOW a call where ``call_chain`` length <= 1. This is
        a security-relevant allow/deny flip, so a bool threshold must be
        rejected just like any non-int value.
        """
        handler = _MaxCallDepthHandler()
        ctx = _make_context(call_chain=["a"])
        assert handler.evaluate(True, ctx) is False

    def test_bool_false_does_not_match_fail_closed(self) -> None:
        """A-D-012: ``max_call_depth: false`` must NOT match either."""
        handler = _MaxCallDepthHandler()
        ctx = _make_context(call_chain=[])
        assert handler.evaluate(False, ctx) is False

    def test_bool_in_lte_does_not_match_fail_closed(self) -> None:
        """A-D-012: bool inside the ``lte`` form must also fail closed."""
        handler = _MaxCallDepthHandler()
        ctx = _make_context(call_chain=["a"])
        assert handler.evaluate({"lte": True}, ctx) is False

    def test_integral_float_threshold_matches(self) -> None:
        """A-D-004: an integral float threshold (e.g. ``5.0`` from YAML/JSON)
        must be accepted as depth 5, matching apcore-typescript.

        Cross-language consistency: TS has no int/float distinction so
        ``max_call_depth: 5.0`` evaluates identically to ``5``. Python must
        not silently reject the float and fall through to default-deny.
        """
        handler = _MaxCallDepthHandler()
        ctx = _make_context(call_chain=["a", "b", "c", "d", "e"])
        assert handler.evaluate(5.0, ctx) is True

    def test_integral_float_threshold_exceeded(self) -> None:
        """A-D-004: an integral float threshold still enforces the limit."""
        handler = _MaxCallDepthHandler()
        ctx = _make_context(call_chain=["a", "b", "c", "d", "e", "f"])
        assert handler.evaluate(5.0, ctx) is False

    def test_integral_float_in_lte_matches(self) -> None:
        """A-D-004: integral float inside the ``lte`` form is also accepted."""
        handler = _MaxCallDepthHandler()
        ctx = _make_context(call_chain=["a", "b", "c"])
        assert handler.evaluate({"lte": 3.0}, ctx) is True

    def test_non_integral_float_does_not_match_fail_closed(self) -> None:
        """A-D-004: a fractional float threshold (e.g. ``5.5``) is meaningless
        as a depth and must fail closed, matching apcore-typescript's
        ``Number.isInteger`` guard."""
        handler = _MaxCallDepthHandler()
        ctx = _make_context(call_chain=["a", "b"])
        assert handler.evaluate(5.5, ctx) is False

    def test_allow_rule_with_integral_float_threshold(self) -> None:
        """A-D-004: an allow-rule conditioned on ``max_call_depth: 5.0`` with a
        caller at depth 5 must ALLOW (rule matches), matching apcore-typescript.

        A float threshold loaded from YAML/JSON must not cause the allow-rule to
        silently fail to match and fall through to default-deny.
        """
        ctx = _make_context(call_chain=["a", "b", "c", "d", "e"])
        acl = _make_acl_with_condition("max_call_depth", 5.0)
        assert acl.check("caller", "target", context=ctx) is True


# ---------------------------------------------------------------------------
# Compound Handlers
# ---------------------------------------------------------------------------


class TestOrHandler:
    def test_or_passes_when_any_match(self) -> None:
        """AC-011: $or evaluates with OR logic."""
        ctx = _make_context(identity_type="user", roles=["admin"])
        acl = _make_acl_with_condition(
            "$or",
            [
                {"roles": ["admin"]},
                {"identity_types": ["service"]},
            ],
        )
        assert acl.check("caller", "target", context=ctx) is True

    def test_or_fails_when_none_match(self) -> None:
        ctx = _make_context(identity_type="user", roles=["viewer"])
        acl = _make_acl_with_condition(
            "$or",
            [
                {"roles": ["admin"]},
                {"identity_types": ["service"]},
            ],
        )
        assert acl.check("caller", "target", context=ctx) is False

    def test_or_empty_list_returns_false(self) -> None:
        """AC-029: $or with empty list returns False."""
        ctx = _make_context()
        acl = _make_acl_with_condition("$or", [])
        assert acl.check("caller", "target", context=ctx) is False

    def test_or_non_list_returns_false(self) -> None:
        ctx = _make_context()
        acl = _make_acl_with_condition("$or", "invalid")
        assert acl.check("caller", "target", context=ctx) is False

    def test_or_element_that_is_not_a_mapping_is_unevaluable(self) -> None:
        """CHANGED at spec v1.25.0 — this used to skip the element and return True.

        §6.1.1 made "unevaluable" a principle rather than a closed list, and an
        `$or` element that is not a condition object asks no question. §6.1.4's
        precheck therefore records a fault at `$or[0]` and the rule is
        unevaluable, so this `allow` rule does not grant even though its second
        branch would have been satisfied. See
        `test_a_non_mapping_or_element_is_unevaluable` in
        tests/test_acl_unevaluable_conditions.py for the reasoning and the
        cross-SDK caveat.
        """
        ctx = _make_context(roles=["admin"])
        acl = _make_acl_with_condition(
            "$or",
            [
                "invalid_string",
                {"roles": ["admin"]},
            ],
        )
        assert acl.check("caller", "target", context=ctx) is False


class TestNotHandler:
    def test_not_negates_conditions(self) -> None:
        """AC-012: $not negates condition set — allows user, denies service."""
        ctx_user = _make_context(identity_type="user")
        ctx_service = _make_context(identity_type="service")
        acl = _make_acl_with_condition("$not", {"identity_types": ["service"]})
        assert acl.check("caller", "target", context=ctx_user) is True
        assert acl.check("caller", "target", context=ctx_service) is False

    def test_not_non_dict_returns_false(self) -> None:
        """AC-030: $not with non-dict value returns False."""
        ctx = _make_context()
        acl = _make_acl_with_condition("$not", "invalid")
        assert acl.check("caller", "target", context=ctx) is False


class TestNestedCompound:
    def test_nested_or_with_and(self) -> None:
        """AC-032: Nested compound conditions."""
        ctx = _make_context(identity_type="service", call_chain=["a", "b"])
        acl = _make_acl_with_condition(
            "$or",
            [
                {"roles": ["admin"]},
                {"identity_types": ["service"], "max_call_depth": 5},
            ],
        )
        assert acl.check("caller", "target", context=ctx) is True

    def test_nested_or_with_and_fails_when_depth_exceeded(self) -> None:
        ctx = _make_context(identity_type="service", call_chain=["a"] * 10)
        acl = _make_acl_with_condition(
            "$or",
            [
                {"roles": ["admin"]},
                {"identity_types": ["service"], "max_call_depth": 5},
            ],
        )
        assert acl.check("caller", "target", context=ctx) is False


# ---------------------------------------------------------------------------
# Fail-closed behavior
# ---------------------------------------------------------------------------


class TestFailClosed:
    def test_unknown_condition_fails_closed(self, caplog: pytest.LogCaptureFixture) -> None:
        """AC-010: Unknown condition key fails-closed with WARN log."""
        ctx = _make_context()
        acl = _make_acl_with_condition("nonexistent", True)
        with caplog.at_level(logging.WARNING):
            result = acl.check("caller", "target", context=ctx)
        assert result is False
        # §6.1.4's precheck now reports this before evaluation, so the wording
        # differs by case between the precheck and the evaluator's own path.
        assert any("unknown acl condition" in r.message.lower() for r in caplog.records)

    def test_genuine_async_handler_suspends_fails_closed(self, caplog: pytest.LogCaptureFixture) -> None:
        """AC-014: Sync check() fails-closed when async handler genuinely suspends.

        Per registry-system.md "Sync handler resolution" cross-language note:
        a coroutine that hits an await point (genuine async work) is treated
        as unsatisfied in sync context. Use async_check() for handlers that
        need real awaits.
        """
        import asyncio

        class _GenuinelyAsyncHandler:
            async def evaluate(self, value: Any, context: Context) -> bool:
                await asyncio.sleep(0)  # forces suspension
                return True

        ACL.register_condition("_test_async_suspend", _GenuinelyAsyncHandler())
        try:
            ctx = _make_context()
            acl = _make_acl_with_condition("_test_async_suspend", True)
            with caplog.at_level(logging.WARNING):
                result = acl.check("caller", "target", context=ctx)
            assert result is False
            assert any(
                "suspended" in r.message.lower() or "async condition" in r.message.lower() for r in caplog.records
            )
        finally:
            del ACL._condition_handlers["_test_async_suspend"]

    def test_sync_body_async_fn_succeeds_in_sync_check(self) -> None:
        """A-D-023: An ``async def`` whose body completes without ``await`` is
        used in sync check() (the coroutine returns immediately via send(None)
        + StopIteration). Cross-language parity with apcore-rust poll-once.
        """

        class _SyncBodyAsyncHandler:
            async def evaluate(self, value: Any, context: Context) -> bool:
                # No await — coroutine completes synchronously
                return True

        ACL.register_condition("_test_async_sync_body", _SyncBodyAsyncHandler())
        try:
            ctx = _make_context()
            acl = _make_acl_with_condition("_test_async_sync_body", True)
            result = acl.check("caller", "target", context=ctx)
            assert result is True
        finally:
            del ACL._condition_handlers["_test_async_sync_body"]


# ---------------------------------------------------------------------------
# Custom handler integration
# ---------------------------------------------------------------------------


class TestCustomHandler:
    def test_custom_handler_invoked_during_check(self) -> None:
        """AC-009: register_condition() registers handler invoked during check()."""
        invoked: list[bool] = []

        class _TestHandler:
            def evaluate(self, value: Any, context: Context) -> bool:
                invoked.append(True)
                return value == "magic"

        ACL.register_condition("_test_magic", _TestHandler())
        try:
            ctx = _make_context()
            acl = _make_acl_with_condition("_test_magic", "magic")
            assert acl.check("caller", "target", context=ctx) is True
            assert invoked == [True]
        finally:
            del ACL._condition_handlers["_test_magic"]


# ---------------------------------------------------------------------------
# async_check
# ---------------------------------------------------------------------------


class TestAsyncCheck:
    def test_async_check_basic(self) -> None:
        ctx = _make_context(roles=["admin"])
        acl = _make_acl_with_condition("roles", ["admin"])
        result = asyncio.run(
            acl.async_check("caller", "target", context=ctx),
        )
        assert result is True

    def test_async_check_with_async_handler(self) -> None:
        """AC-013: async_check() awaits async handlers."""

        class _AsyncHandler:
            async def evaluate(self, value: Any, context: Context) -> bool:
                return value == "async_magic"

        ACL.register_condition("_test_async_magic", _AsyncHandler())
        try:
            ctx = _make_context()
            acl = _make_acl_with_condition("_test_async_magic", "async_magic")
            result = asyncio.run(
                acl.async_check("caller", "target", context=ctx),
            )
            assert result is True
        finally:
            del ACL._condition_handlers["_test_async_magic"]

    def test_async_check_default_deny(self) -> None:
        ctx = _make_context()
        acl = ACL(rules=[], default_effect="deny")
        result = asyncio.run(
            acl.async_check("caller", "target", context=ctx),
        )
        assert result is False

    def test_async_check_default_allow(self) -> None:
        ctx = _make_context()
        acl = ACL(rules=[], default_effect="allow")
        result = asyncio.run(
            acl.async_check("caller", "target", context=ctx),
        )
        assert result is True

    def test_async_check_no_context_with_conditions_denies(self) -> None:
        acl = _make_acl_with_condition("roles", ["admin"])
        result = asyncio.run(
            acl.async_check("caller", "target", context=None),
        )
        assert result is False


class TestHandlerErrorDiagnostic:
    """Every fail-closed condition path must say *why* in the audit entry.

    ``AuditEntry.handler_error`` was set only when a handler raised. An unknown
    condition key and an awaitable that suspends in the sync path both denied
    silently, so a typo'd key (``role:`` for ``roles:``) produced an identical
    DENY in all three SDKs but a null diagnostic in Python alone —
    apcore-typescript records a message on both paths (``acl.ts``
    ``_evaluateConditions`` / ``_evaluateConditionsAsync``).
    """

    @staticmethod
    def _acl_with(conditions: dict[str, Any]) -> tuple[ACL, list[Any]]:
        captured: list[Any] = []
        acl = ACL(
            rules=[
                ACLRule(
                    callers=["*"],
                    targets=["*"],
                    effect="allow",
                    description="conditional allow",
                    conditions=conditions,
                )
            ],
            default_effect="deny",
            audit_logger=captured.append,
        )
        return acl, captured

    def test_unknown_condition_key_records_a_diagnostic(self) -> None:
        acl, captured = self._acl_with({"role": ["admin"]})  # typo for "roles"
        assert acl.check("caller", "target", Context.create()) is False
        assert len(captured) == 1
        assert captured[0].handler_error is not None
        assert "role" in captured[0].handler_error

    def test_unknown_condition_key_records_a_diagnostic_on_the_async_path(self) -> None:
        acl, captured = self._acl_with({"role": ["admin"]})
        assert asyncio.run(acl.async_check("caller", "target", Context.create())) is False
        assert len(captured) == 1
        assert captured[0].handler_error is not None
        assert "role" in captured[0].handler_error

    def test_suspending_async_handler_in_sync_context_records_a_diagnostic(self) -> None:
        class _SuspendingHandler:
            async def evaluate(self, value: Any, context: Context) -> bool:
                await asyncio.sleep(0)
                return True

        key = "_test_suspending_condition"
        ACL.register_condition(key, _SuspendingHandler())  # type: ignore[arg-type]
        try:
            acl, captured = self._acl_with({key: True})
            assert acl.check("caller", "target", Context.create()) is False
            assert len(captured) == 1
            assert captured[0].handler_error is not None
            assert key in captured[0].handler_error
        finally:
            ACL._condition_handlers.pop(key, None)

    def test_a_satisfied_condition_leaves_the_diagnostic_null(self) -> None:
        acl, captured = self._acl_with({"max_call_depth": 10})
        assert acl.check("caller", "target", Context.create()) is True
        assert len(captured) == 1
        assert captured[0].handler_error is None
