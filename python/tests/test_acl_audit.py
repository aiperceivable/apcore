"""Tests for ACL audit logging (F10: AuditEntry and audit_logger)."""

from __future__ import annotations

from datetime import datetime, timezone

from apcore.acl import ACL, ACLRule, AuditEntry
from apcore.context import Context, Identity


class TestAuditEntryOnRuleMatchAllow:
    """Audit entry is emitted when a rule matches with effect=allow."""

    def test_audit_entry_emitted_on_allow(self) -> None:
        """audit_logger is called with an AuditEntry when a rule allows access."""
        entries: list[AuditEntry] = []
        rule = ACLRule(callers=["api.*"], targets=["db.*"], effect="allow", description="API to DB")
        acl = ACL(rules=[rule], default_effect="deny", audit_logger=entries.append)

        result = acl.check(caller_id="api.handler", target_id="db.read")

        assert result is True
        assert len(entries) == 1
        assert entries[0].decision == "allow"
        assert entries[0].reason == "rule_match"


class TestAuditEntryOnRuleMatchDeny:
    """Audit entry is emitted when a rule matches with effect=deny."""

    def test_audit_entry_emitted_on_deny(self) -> None:
        """audit_logger is called with an AuditEntry when a rule denies access."""
        entries: list[AuditEntry] = []
        rule = ACLRule(callers=["*"], targets=["admin.*"], effect="deny", description="Block admin")
        acl = ACL(rules=[rule], default_effect="allow", audit_logger=entries.append)

        result = acl.check(caller_id="api.handler", target_id="admin.panel")

        assert result is False
        assert len(entries) == 1
        assert entries[0].decision == "deny"
        assert entries[0].reason == "rule_match"


class TestAuditEntryOnDefaultEffect:
    """Audit entry is emitted when no rule matches and default effect applies."""

    def test_default_effect_deny_with_rules(self) -> None:
        """audit_logger is called with reason=default_effect when rules exist but none match."""
        entries: list[AuditEntry] = []
        rule = ACLRule(callers=["other.*"], targets=["other.*"], effect="allow")
        acl = ACL(rules=[rule], default_effect="deny", audit_logger=entries.append)

        result = acl.check(caller_id="api.handler", target_id="db.read")

        assert result is False
        assert len(entries) == 1
        assert entries[0].decision == "deny"
        assert entries[0].reason == "default_effect"

    def test_default_effect_allow_with_rules(self) -> None:
        """audit_logger is called with reason=default_effect when default is allow."""
        entries: list[AuditEntry] = []
        rule = ACLRule(callers=["other.*"], targets=["other.*"], effect="deny")
        acl = ACL(rules=[rule], default_effect="allow", audit_logger=entries.append)

        result = acl.check(caller_id="api.handler", target_id="db.read")

        assert result is True
        assert len(entries) == 1
        assert entries[0].decision == "allow"
        assert entries[0].reason == "default_effect"

    def test_no_rules_reason(self) -> None:
        """audit_logger receives reason=no_rules when the ACL has zero rules."""
        entries: list[AuditEntry] = []
        acl = ACL(rules=[], default_effect="deny", audit_logger=entries.append)

        result = acl.check(caller_id="api.handler", target_id="db.read")

        assert result is False
        assert len(entries) == 1
        assert entries[0].reason == "no_rules"


class TestAuditEntryFields:
    """Audit entry contains correct caller, target, and decision fields."""

    def test_caller_and_target_in_entry(self) -> None:
        """AuditEntry reflects the caller_id and target_id from the check call."""
        entries: list[AuditEntry] = []
        rule = ACLRule(callers=["api.*"], targets=["db.*"], effect="allow")
        acl = ACL(rules=[rule], audit_logger=entries.append)

        acl.check(caller_id="api.handler", target_id="db.read")

        assert entries[0].caller_id == "api.handler"
        assert entries[0].target_id == "db.read"

    def test_external_caller_mapped(self) -> None:
        """When caller_id is None, AuditEntry.caller_id is '@external'."""
        entries: list[AuditEntry] = []
        rule = ACLRule(callers=["@external"], targets=["*"], effect="allow")
        acl = ACL(rules=[rule], audit_logger=entries.append)

        acl.check(caller_id=None, target_id="public.api")

        assert entries[0].caller_id == "@external"

    def test_decision_matches_return_value(self) -> None:
        """AuditEntry.decision is consistent with the boolean return value."""
        entries: list[AuditEntry] = []
        rule = ACLRule(callers=["*"], targets=["*"], effect="deny")
        acl = ACL(rules=[rule], audit_logger=entries.append)

        result = acl.check(caller_id="caller", target_id="target")

        assert result is False
        assert entries[0].decision == "deny"


class TestAuditEntryMatchedRule:
    """Audit entry includes matched rule info (description and index)."""

    def test_matched_rule_description(self) -> None:
        """AuditEntry.matched_rule is the description of the ACLRule that matched."""
        entries: list[AuditEntry] = []
        rule = ACLRule(callers=["api.*"], targets=["db.*"], effect="allow", description="API to DB")
        acl = ACL(rules=[rule], audit_logger=entries.append)

        acl.check(caller_id="api.handler", target_id="db.read")

        assert entries[0].matched_rule == "API to DB"

    def test_matched_rule_index(self) -> None:
        """AuditEntry.matched_rule_index reflects the position of the matched rule."""
        entries: list[AuditEntry] = []
        rules = [
            ACLRule(callers=["other.*"], targets=["other.*"], effect="deny"),
            ACLRule(callers=["api.*"], targets=["db.*"], effect="allow", description="Second rule"),
        ]
        acl = ACL(rules=rules, audit_logger=entries.append)

        acl.check(caller_id="api.handler", target_id="db.read")

        assert entries[0].matched_rule_index == 1
        assert entries[0].matched_rule == "Second rule"

    def test_no_matched_rule_on_default(self) -> None:
        """AuditEntry has None for matched_rule when falling through to default."""
        entries: list[AuditEntry] = []
        acl = ACL(rules=[], default_effect="deny", audit_logger=entries.append)

        acl.check(caller_id="caller", target_id="target")

        assert entries[0].matched_rule is None
        assert entries[0].matched_rule_index is None


class TestAuditEntryContextFields:
    """Context fields (identity_type, roles, trace_id) are populated when context provided."""

    def test_context_fields_populated(self) -> None:
        """AuditEntry extracts identity_type, roles, call_depth, and trace_id from context."""
        entries: list[AuditEntry] = []
        rule = ACLRule(callers=["*"], targets=["*"], effect="allow")
        acl = ACL(rules=[rule], audit_logger=entries.append)
        ctx = Context.create(identity=Identity(id="u_1", type="service", roles=("admin", "reader")))
        ctx.call_chain = ["a", "b"]

        acl.check(caller_id="caller", target_id="target", context=ctx)

        entry = entries[0]
        assert entry.identity_type == "service"
        assert entry.roles == ("admin", "reader")
        assert entry.call_depth == 2
        assert entry.trace_id == ctx.trace_id

    def test_context_without_identity(self) -> None:
        """AuditEntry has None identity_type and empty roles when context has no identity."""
        entries: list[AuditEntry] = []
        rule = ACLRule(callers=["*"], targets=["*"], effect="allow")
        acl = ACL(rules=[rule], audit_logger=entries.append)
        ctx = Context.create()

        acl.check(caller_id="caller", target_id="target", context=ctx)

        entry = entries[0]
        assert entry.identity_type is None
        assert entry.roles == ()
        assert entry.call_depth == 0
        assert entry.trace_id == ctx.trace_id

    def test_no_context_fields_are_none(self) -> None:
        """AuditEntry has None for context fields when no context is provided."""
        entries: list[AuditEntry] = []
        rule = ACLRule(callers=["*"], targets=["*"], effect="allow")
        acl = ACL(rules=[rule], audit_logger=entries.append)

        acl.check(caller_id="caller", target_id="target")

        entry = entries[0]
        assert entry.identity_type is None
        assert entry.roles == ()
        assert entry.call_depth is None
        assert entry.trace_id is None


class TestNoAuditWhenLoggerNone:
    """No audit entry is created when audit_logger is None (default)."""

    def test_no_audit_logger_default(self) -> None:
        """ACL works normally with no audit_logger set (default behavior)."""
        rule = ACLRule(callers=["*"], targets=["*"], effect="allow")
        acl = ACL(rules=[rule])

        # Should not raise and should return the correct decision.
        result = acl.check(caller_id="caller", target_id="target")
        assert result is True

    def test_no_callback_when_none(self) -> None:
        """Explicitly passing audit_logger=None does not emit any entries."""
        rule = ACLRule(callers=["*"], targets=["*"], effect="allow")
        acl = ACL(rules=[rule], audit_logger=None)

        result = acl.check(caller_id="caller", target_id="target")
        assert result is True
        # No way to capture entries since there's no logger -- just verify no crash.


class TestAuditEntryTimestamp:
    """AuditEntry timestamp is valid ISO 8601."""

    def test_timestamp_is_valid_iso8601(self) -> None:
        """AuditEntry.timestamp can be parsed as an ISO 8601 datetime."""
        entries: list[AuditEntry] = []
        rule = ACLRule(callers=["*"], targets=["*"], effect="allow")
        acl = ACL(rules=[rule], audit_logger=entries.append)

        acl.check(caller_id="caller", target_id="target")

        entry = entries[0]
        # datetime.fromisoformat will raise ValueError if the format is invalid.
        parsed = datetime.fromisoformat(entry.timestamp)
        assert parsed.tzinfo is not None  # Should be timezone-aware (UTC)

    def test_timestamp_is_recent(self) -> None:
        """AuditEntry.timestamp is close to the current UTC time."""
        entries: list[AuditEntry] = []
        rule = ACLRule(callers=["*"], targets=["*"], effect="allow")
        acl = ACL(rules=[rule], audit_logger=entries.append)

        before = datetime.now(timezone.utc)
        acl.check(caller_id="caller", target_id="target")
        after = datetime.now(timezone.utc)

        parsed = datetime.fromisoformat(entries[0].timestamp)
        assert before <= parsed <= after


class TestAuditEntryFrozen:
    """AuditEntry is immutable (frozen dataclass)."""

    def test_audit_entry_is_frozen(self) -> None:
        """AuditEntry fields cannot be modified after creation."""
        entry = AuditEntry(
            timestamp="2024-01-01T00:00:00+00:00",
            caller_id="caller",
            target_id="target",
            decision="allow",
            reason="rule_match",
        )
        try:
            entry.decision = "deny"  # type: ignore[misc]
            raise AssertionError("Expected FrozenInstanceError")  # pragma: no cover
        except AttributeError:
            pass  # Expected: frozen dataclass


class TestAuditEntryHandlerError:
    """AuditEntry.handler_error surfaces condition-handler failures into the audit trail."""

    def test_handler_error_none_when_no_failure(self) -> None:
        entries: list[AuditEntry] = []
        rule = ACLRule(callers=["*"], targets=["*"], effect="allow")
        acl = ACL(rules=[rule], default_effect="deny", audit_logger=entries.append)

        acl.check(caller_id="a", target_id="b")

        assert entries[0].handler_error is None

    def test_handler_error_set_when_condition_handler_raises(self) -> None:
        """When a condition handler raises, the failure is surfaced in handler_error."""

        class BoomHandler:
            def evaluate(self, value, context):  # type: ignore[no-untyped-def]
                raise RuntimeError("boom")

        ACL.register_condition("boom", BoomHandler())
        try:
            entries: list[AuditEntry] = []
            rule = ACLRule(
                callers=["*"],
                targets=["*"],
                effect="allow",
                conditions={"boom": True},
            )
            acl = ACL(rules=[rule], default_effect="deny", audit_logger=entries.append)

            ctx = Context.create()
            result = acl.check(caller_id="a", target_id="b", context=ctx)

            assert result is False  # fail-closed
            assert len(entries) == 1
            assert entries[0].handler_error is not None
            assert "boom" in entries[0].handler_error
            assert "RuntimeError" in entries[0].handler_error
        finally:
            # Restore known condition handlers (the class registry is a classvar)
            ACL._condition_handlers.pop("boom", None)
