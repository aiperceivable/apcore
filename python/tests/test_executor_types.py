"""Tests for executor system types: ACLRule and ValidationResult import."""

from __future__ import annotations

from apcore.acl import ACLRule
from apcore.module import ValidationResult


# =============================================================================
# ACLRule
# =============================================================================


class TestACLRuleCreation:
    def test_creation_with_all_fields(self) -> None:
        """ACLRule creation with all fields stores them correctly."""
        rule = ACLRule(
            callers=["mod.a"],
            targets=["mod.b"],
            effect="allow",
            description="test rule",
            conditions={"roles": ["admin"]},
        )
        assert rule.callers == ["mod.a"]
        assert rule.targets == ["mod.b"]
        assert rule.effect == "allow"
        assert rule.description == "test rule"
        assert rule.conditions == {"roles": ["admin"]}

    def test_default_values(self) -> None:
        """ACLRule with only required fields has correct defaults."""
        rule = ACLRule(callers=["*"], targets=["*"], effect="deny")
        assert rule.description == ""
        assert rule.conditions is None

    def test_equality_comparison(self) -> None:
        """ACLRule instances with identical fields are equal."""
        rule1 = ACLRule(callers=["mod.a"], targets=["mod.b"], effect="allow")
        rule2 = ACLRule(callers=["mod.a"], targets=["mod.b"], effect="allow")
        rule3 = ACLRule(callers=["mod.a"], targets=["mod.b"], effect="deny")
        assert rule1 == rule2
        assert rule1 != rule3


# =============================================================================
# ValidationResult (imported from foundation)
# =============================================================================


class TestValidationResult:
    def test_valid_true_with_empty_errors(self) -> None:
        """ValidationResult(valid=True, errors=[]) stores fields correctly."""
        result = ValidationResult(valid=True, errors=[])
        assert result.valid is True
        assert result.errors == []

    def test_valid_false_captures_errors(self) -> None:
        """ValidationResult(valid=False, errors=[...]) captures error details."""
        result = ValidationResult(
            valid=False,
            errors=[{"field": "name", "code": "missing", "message": "required"}],
        )
        assert result.valid is False
        assert len(result.errors) == 1
        assert result.errors[0]["field"] == "name"

    def test_import_from_apcore_module(self) -> None:
        """ValidationResult is importable from apcore.module."""
        from apcore.module import ValidationResult as VR

        assert VR is not None
