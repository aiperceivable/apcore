"""Tests for the ACL (Access Control List) system."""

from __future__ import annotations

import json
import textwrap
import threading
from pathlib import Path

import pytest

from apcore.acl import ACL, ACLRule, AuditEntry
from apcore.context import Context, Identity
from apcore.errors import ACLRuleError, ConfigNotFoundError


# === Pattern Matching (delegates to foundation) ===


class TestACLPatternMatching:
    """Tests for ACL-level pattern matching, including special @external/@system patterns."""

    # Test: @external pattern matches when caller_id is None
    def test_external_pattern_matches_none_caller(self) -> None:
        """@external pattern matches when the effective caller is None (external call)."""
        acl = ACL(rules=[ACLRule(callers=["@external"], targets=["*"], effect="allow")])
        assert acl.check(caller_id=None, target_id="some.module") is True

    # Test: @external pattern does NOT match when caller_id is a string
    def test_external_pattern_does_not_match_string_caller(self) -> None:
        """@external should not match when caller_id is a real module ID."""
        acl = ACL(
            rules=[ACLRule(callers=["@external"], targets=["*"], effect="allow")],
            default_effect="deny",
        )
        assert acl.check(caller_id="api.handler", target_id="some.module") is False

    # Test: @system pattern matches when identity.type == "system"
    def test_system_pattern_matches_system_identity(self) -> None:
        """@system matches when context has an identity with type='system'."""
        acl = ACL(rules=[ACLRule(callers=["@system"], targets=["*"], effect="allow")])
        ctx = Context.create(identity=Identity(id="sys_1", type="system"))
        assert acl.check(caller_id="internal.task", target_id="db.write", context=ctx) is True

    # Test: @system pattern does NOT match when identity is None
    def test_system_pattern_no_match_when_identity_none(self) -> None:
        """@system should not match when context has no identity."""
        acl = ACL(
            rules=[ACLRule(callers=["@system"], targets=["*"], effect="allow")],
            default_effect="deny",
        )
        ctx = Context.create()
        assert acl.check(caller_id="internal.task", target_id="db.write", context=ctx) is False

    # Test: @system pattern does NOT match when identity.type != "system"
    def test_system_pattern_no_match_when_identity_not_system(self) -> None:
        """@system should not match when identity type is 'user' or other non-system types."""
        acl = ACL(
            rules=[ACLRule(callers=["@system"], targets=["*"], effect="allow")],
            default_effect="deny",
        )
        ctx = Context.create(identity=Identity(id="u_123", type="user"))
        assert acl.check(caller_id="internal.task", target_id="db.write", context=ctx) is False

    # Test: exact pattern delegates to foundation match_pattern
    def test_exact_pattern_delegates_to_foundation(self) -> None:
        """Exact caller/target patterns use foundation match_pattern for matching."""
        acl = ACL(rules=[ACLRule(callers=["api.handler"], targets=["db.read"], effect="allow")])
        assert acl.check(caller_id="api.handler", target_id="db.read") is True

    # Test: wildcard "*" delegates to foundation match_pattern
    def test_wildcard_star_delegates_to_foundation(self) -> None:
        """Wildcard '*' matches any caller or target via foundation match_pattern."""
        acl = ACL(rules=[ACLRule(callers=["*"], targets=["*"], effect="allow")])
        assert acl.check(caller_id="anything", target_id="anything.else") is True

    # Test: prefix "executor.*" delegates to foundation match_pattern
    def test_prefix_wildcard_delegates_to_foundation(self) -> None:
        """Prefix wildcard patterns delegate to foundation match_pattern."""
        acl = ACL(rules=[ACLRule(callers=["executor.*"], targets=["*"], effect="allow")])
        assert acl.check(caller_id="executor.email", target_id="some.target") is True
        assert acl.check(caller_id="api.handler", target_id="some.target") is False


# === ACL.check() -- First-Match-Wins ===


class TestACLCheck:
    """Tests for first-match-wins rule evaluation."""

    # Test: first matching allow rule returns True
    def test_first_matching_allow_returns_true(self) -> None:
        """First rule that matches should be used; allow returns True."""
        acl = ACL(
            rules=[
                ACLRule(callers=["api.*"], targets=["db.*"], effect="allow"),
                ACLRule(callers=["api.*"], targets=["db.*"], effect="deny"),
            ]
        )
        assert acl.check(caller_id="api.handler", target_id="db.read") is True

    # Test: first matching deny rule returns False
    def test_first_matching_deny_returns_false(self) -> None:
        """First rule that matches should be used; deny returns False."""
        acl = ACL(
            rules=[
                ACLRule(callers=["api.*"], targets=["db.*"], effect="deny"),
                ACLRule(callers=["api.*"], targets=["db.*"], effect="allow"),
            ]
        )
        assert acl.check(caller_id="api.handler", target_id="db.read") is False

    # Test: no match returns default_effect
    def test_no_match_returns_default_effect_deny(self) -> None:
        """When no rule matches, default_effect='deny' returns False."""
        acl = ACL(rules=[], default_effect="deny")
        assert acl.check(caller_id="api.handler", target_id="db.read") is False

    # Test: default_effect="allow" returns True when no match
    def test_no_match_returns_default_effect_allow(self) -> None:
        """When no rule matches, default_effect='allow' returns True."""
        acl = ACL(rules=[], default_effect="allow")
        assert acl.check(caller_id="api.handler", target_id="db.read") is True

    # Test: default_effect="deny" returns False when no match
    def test_default_effect_deny(self) -> None:
        """Explicit default_effect='deny' returns False when no rule matches."""
        acl = ACL(
            rules=[ACLRule(callers=["other.*"], targets=["*"], effect="allow")],
            default_effect="deny",
        )
        assert acl.check(caller_id="api.handler", target_id="db.read") is False

    # Test: rules evaluated in order (first match wins, not best match)
    def test_rules_evaluated_in_order(self) -> None:
        """Rules are evaluated sequentially; first match wins regardless of specificity."""
        acl = ACL(
            rules=[
                ACLRule(callers=["*"], targets=["*"], effect="deny"),
                ACLRule(callers=["api.handler"], targets=["db.read"], effect="allow"),
            ]
        )
        # The wildcard deny rule matches first, so even the more specific allow is ignored.
        assert acl.check(caller_id="api.handler", target_id="db.read") is False


# === ACL.load() -- YAML Loading ===


class TestACLLoad:
    """Tests for loading ACL configuration from YAML files."""

    # Test: load valid YAML with rules
    def test_load_valid_yaml(self, tmp_path: Path) -> None:
        """Load a valid YAML ACL configuration file."""
        yaml_content = textwrap.dedent(
            """\
            version: "1.0"
            default_effect: deny
            rules:
              - callers: ["api.*"]
                targets: ["db.*"]
                effect: allow
                description: "API can access DB"
              - callers: ["*"]
                targets: ["admin.*"]
                effect: deny
        """
        )
        yaml_file = tmp_path / "acl.yaml"
        yaml_file.write_text(yaml_content)

        acl = ACL.load(str(yaml_file))
        assert acl.check(caller_id="api.handler", target_id="db.read") is True
        assert acl.check(caller_id="random.caller", target_id="admin.panel") is False

    # Test: load missing file raises ConfigNotFoundError
    def test_load_missing_file_raises_config_not_found(self, tmp_path: Path) -> None:
        """Loading a nonexistent file raises ConfigNotFoundError."""
        with pytest.raises(ConfigNotFoundError):
            ACL.load(str(tmp_path / "nonexistent.yaml"))

    # Test: load invalid YAML raises ACLRuleError
    def test_load_invalid_yaml_raises_acl_rule_error(self, tmp_path: Path) -> None:
        """Malformed YAML content raises ACLRuleError."""
        yaml_file = tmp_path / "bad.yaml"
        yaml_file.write_text(":::invalid yaml{{{}}}:::")
        with pytest.raises(ACLRuleError):
            ACL.load(str(yaml_file))

    # Test: load YAML with missing "rules" key raises ACLRuleError
    def test_load_yaml_missing_rules_key(self, tmp_path: Path) -> None:
        """YAML without a 'rules' key raises ACLRuleError."""
        yaml_file = tmp_path / "no_rules.yaml"
        yaml_file.write_text("version: '1.0'\ndefault_effect: deny\n")
        with pytest.raises(ACLRuleError):
            ACL.load(str(yaml_file))

    # Test: load YAML with non-list "rules" raises ACLRuleError
    def test_load_yaml_rules_not_list(self, tmp_path: Path) -> None:
        """YAML where 'rules' is not a list raises ACLRuleError."""
        yaml_file = tmp_path / "rules_dict.yaml"
        yaml_file.write_text("version: '1.0'\nrules: not_a_list\n")
        with pytest.raises(ACLRuleError):
            ACL.load(str(yaml_file))

    # Test: load YAML with rule missing "callers" raises ACLRuleError
    def test_load_yaml_rule_missing_callers(self, tmp_path: Path) -> None:
        """A rule without 'callers' key raises ACLRuleError."""
        yaml_content = textwrap.dedent(
            """\
            version: "1.0"
            rules:
              - targets: ["*"]
                effect: allow
        """
        )
        yaml_file = tmp_path / "no_callers.yaml"
        yaml_file.write_text(yaml_content)
        with pytest.raises(ACLRuleError):
            ACL.load(str(yaml_file))

    # Test: load YAML with rule missing "targets" raises ACLRuleError
    def test_load_yaml_rule_missing_targets(self, tmp_path: Path) -> None:
        """A rule without 'targets' key raises ACLRuleError."""
        yaml_content = textwrap.dedent(
            """\
            version: "1.0"
            rules:
              - callers: ["*"]
                effect: allow
        """
        )
        yaml_file = tmp_path / "no_targets.yaml"
        yaml_file.write_text(yaml_content)
        with pytest.raises(ACLRuleError):
            ACL.load(str(yaml_file))

    # Test: load YAML with rule missing "effect" raises ACLRuleError
    def test_load_yaml_rule_missing_effect(self, tmp_path: Path) -> None:
        """A rule without 'effect' key raises ACLRuleError."""
        yaml_content = textwrap.dedent(
            """\
            version: "1.0"
            rules:
              - callers: ["*"]
                targets: ["*"]
        """
        )
        yaml_file = tmp_path / "no_effect.yaml"
        yaml_file.write_text(yaml_content)
        with pytest.raises(ACLRuleError):
            ACL.load(str(yaml_file))

    # Test: load YAML with invalid effect (not allow/deny) raises ACLRuleError
    def test_load_yaml_invalid_effect(self, tmp_path: Path) -> None:
        """A rule with effect other than 'allow'/'deny' raises ACLRuleError."""
        yaml_content = textwrap.dedent(
            """\
            version: "1.0"
            rules:
              - callers: ["*"]
                targets: ["*"]
                effect: maybe
        """
        )
        yaml_file = tmp_path / "bad_effect.yaml"
        yaml_file.write_text(yaml_content)
        with pytest.raises(ACLRuleError):
            ACL.load(str(yaml_file))

    # Test: load YAML with callers as string (not list) raises ACLRuleError
    def test_load_yaml_callers_not_list(self, tmp_path: Path) -> None:
        """A rule with 'callers' as a string instead of list raises ACLRuleError."""
        yaml_content = textwrap.dedent(
            """\
            version: "1.0"
            rules:
              - callers: "api.*"
                targets: ["*"]
                effect: allow
        """
        )
        yaml_file = tmp_path / "callers_string.yaml"
        yaml_file.write_text(yaml_content)
        with pytest.raises(ACLRuleError):
            ACL.load(str(yaml_file))

    # Test: load YAML with optional description and conditions
    def test_load_yaml_with_description_and_conditions(self, tmp_path: Path) -> None:
        """Rules with optional description and conditions fields are parsed correctly."""
        yaml_content = textwrap.dedent(
            """\
            version: "1.0"
            default_effect: deny
            rules:
              - callers: ["api.*"]
                targets: ["db.*"]
                effect: allow
                description: "API to DB access"
                conditions:
                  identity_types: ["service"]
                  max_call_depth: 5
        """
        )
        yaml_file = tmp_path / "with_conditions.yaml"
        yaml_file.write_text(yaml_content)

        acl = ACL.load(str(yaml_file))
        assert len(acl._rules) == 1
        rule = acl._rules[0]
        assert rule.description == "API to DB access"
        assert rule.conditions is not None
        assert rule.conditions["identity_types"] == ["service"]
        assert rule.conditions["max_call_depth"] == 5


# === Conditional Rules ===


class TestConditionalRules:
    """Tests for conditional rule evaluation."""

    # Test: identity_types condition matches when identity.type in list
    def test_identity_types_matches(self) -> None:
        """Condition passes when context identity type is in the allowed list."""
        acl = ACL(
            rules=[
                ACLRule(
                    callers=["*"],
                    targets=["*"],
                    effect="allow",
                    conditions={"identity_types": ["service", "system"]},
                ),
            ],
            default_effect="deny",
        )
        ctx = Context.create(identity=Identity(id="svc_1", type="service"))
        assert acl.check(caller_id="caller", target_id="target", context=ctx) is True

    # Test: identity_types condition fails when identity.type not in list
    def test_identity_types_fails(self) -> None:
        """Condition fails when context identity type is not in the allowed list."""
        acl = ACL(
            rules=[
                ACLRule(
                    callers=["*"],
                    targets=["*"],
                    effect="allow",
                    conditions={"identity_types": ["service"]},
                ),
            ],
            default_effect="deny",
        )
        ctx = Context.create(identity=Identity(id="u_1", type="user"))
        assert acl.check(caller_id="caller", target_id="target", context=ctx) is False

    # Test: roles condition matches when intersection is non-empty
    def test_roles_condition_matches(self) -> None:
        """Condition passes when identity roles intersect with required roles."""
        acl = ACL(
            rules=[
                ACLRule(
                    callers=["*"],
                    targets=["*"],
                    effect="allow",
                    conditions={"roles": ["admin", "superuser"]},
                ),
            ],
            default_effect="deny",
        )
        ctx = Context.create(identity=Identity(id="u_1", type="user", roles=("admin", "reader")))
        assert acl.check(caller_id="caller", target_id="target", context=ctx) is True

    # Test: roles condition fails when no intersection
    def test_roles_condition_fails(self) -> None:
        """Condition fails when identity roles have no intersection with required roles."""
        acl = ACL(
            rules=[
                ACLRule(
                    callers=["*"],
                    targets=["*"],
                    effect="allow",
                    conditions={"roles": ["admin"]},
                ),
            ],
            default_effect="deny",
        )
        ctx = Context.create(identity=Identity(id="u_1", type="user", roles=("reader",)))
        assert acl.check(caller_id="caller", target_id="target", context=ctx) is False

    # Test: max_call_depth condition passes when depth within limit
    def test_max_call_depth_passes(self) -> None:
        """Condition passes when call chain depth is within the limit."""
        acl = ACL(
            rules=[
                ACLRule(
                    callers=["*"],
                    targets=["*"],
                    effect="allow",
                    conditions={"max_call_depth": 5},
                ),
            ],
            default_effect="deny",
        )
        ctx = Context.create()
        ctx.call_chain = ["a", "b", "c"]  # depth 3, within limit of 5
        assert acl.check(caller_id="caller", target_id="target", context=ctx) is True

    # Test: max_call_depth condition fails when depth exceeds limit
    def test_max_call_depth_fails(self) -> None:
        """Condition fails when call chain depth exceeds the limit."""
        acl = ACL(
            rules=[
                ACLRule(
                    callers=["*"],
                    targets=["*"],
                    effect="allow",
                    conditions={"max_call_depth": 2},
                ),
            ],
            default_effect="deny",
        )
        ctx = Context.create()
        ctx.call_chain = ["a", "b", "c"]  # depth 3, exceeds limit of 2
        assert acl.check(caller_id="caller", target_id="target", context=ctx) is False

    # Test: conditions fail when context is None
    def test_conditions_fail_when_context_none(self) -> None:
        """Conditional rules fail to match when no context is provided."""
        acl = ACL(
            rules=[
                ACLRule(
                    callers=["*"],
                    targets=["*"],
                    effect="allow",
                    conditions={"identity_types": ["service"]},
                ),
            ],
            default_effect="deny",
        )
        assert acl.check(caller_id="caller", target_id="target", context=None) is False

    # Test: conditions fail when context.identity is None
    def test_conditions_fail_when_identity_none(self) -> None:
        """Conditional rules requiring identity fail when context has no identity."""
        acl = ACL(
            rules=[
                ACLRule(
                    callers=["*"],
                    targets=["*"],
                    effect="allow",
                    conditions={"identity_types": ["service"]},
                ),
            ],
            default_effect="deny",
        )
        ctx = Context.create()  # identity defaults to None
        assert acl.check(caller_id="caller", target_id="target", context=ctx) is False


# === Runtime Modification ===


class TestACLRuntimeModification:
    """Tests for runtime rule modification."""

    # Test: add_rule inserts at position 0
    def test_add_rule_inserts_at_position_0(self) -> None:
        """add_rule() inserts new rules at the highest priority (position 0)."""
        acl = ACL(rules=[ACLRule(callers=["*"], targets=["*"], effect="deny")])
        acl.add_rule(ACLRule(callers=["api.*"], targets=["db.*"], effect="allow"))
        # The new allow rule is at position 0, so it matches first.
        assert acl.check(caller_id="api.handler", target_id="db.read") is True

    # Test: remove_rule removes matching rule and returns True
    def test_remove_rule_returns_true(self) -> None:
        """remove_rule() removes a rule matching callers+targets and returns True."""
        acl = ACL(rules=[ACLRule(callers=["api.*"], targets=["db.*"], effect="allow")])
        result = acl.remove_rule(callers=["api.*"], targets=["db.*"])
        assert result is True
        assert len(acl._rules) == 0

    # Test: remove_rule returns False when no match
    def test_remove_rule_returns_false_when_no_match(self) -> None:
        """remove_rule() returns False when no rule has matching callers+targets."""
        acl = ACL(rules=[ACLRule(callers=["api.*"], targets=["db.*"], effect="allow")])
        result = acl.remove_rule(callers=["other.*"], targets=["other.*"])
        assert result is False
        assert len(acl._rules) == 1

    # Test: reload() re-reads from YAML file
    def test_reload_rereads_yaml(self, tmp_path: Path) -> None:
        """reload() re-reads the YAML file and updates rules."""
        yaml_content_v1 = textwrap.dedent(
            """\
            version: "1.0"
            default_effect: deny
            rules:
              - callers: ["*"]
                targets: ["*"]
                effect: deny
        """
        )
        yaml_file = tmp_path / "acl.yaml"
        yaml_file.write_text(yaml_content_v1)

        acl = ACL.load(str(yaml_file))
        assert acl.check(caller_id="api.handler", target_id="db.read") is False

        # Update the file
        yaml_content_v2 = textwrap.dedent(
            """\
            version: "1.0"
            default_effect: allow
            rules:
              - callers: ["*"]
                targets: ["*"]
                effect: allow
        """
        )
        yaml_file.write_text(yaml_content_v2)

        acl.reload()
        assert acl.check(caller_id="api.handler", target_id="db.read") is True


# === ACL with Context ===


class TestACLWithContext:
    """Tests for ACL check() interactions with Context."""

    # Test: check with caller_id=None uses @external
    def test_check_with_none_caller_uses_external(self) -> None:
        """When caller_id is None, effective caller becomes '@external'."""
        acl = ACL(
            rules=[ACLRule(callers=["@external"], targets=["public.*"], effect="allow")],
            default_effect="deny",
        )
        assert acl.check(caller_id=None, target_id="public.api") is True
        assert acl.check(caller_id=None, target_id="private.api") is False

    # Test: check passes context to conditional rules
    def test_check_passes_context_to_conditions(self) -> None:
        """Context is forwarded to conditional rule evaluation."""
        acl = ACL(
            rules=[
                ACLRule(
                    callers=["*"],
                    targets=["*"],
                    effect="allow",
                    conditions={"roles": ["admin"]},
                ),
            ],
            default_effect="deny",
        )
        ctx_admin = Context.create(identity=Identity(id="u_1", type="user", roles=("admin",)))
        ctx_reader = Context.create(identity=Identity(id="u_2", type="user", roles=("reader",)))
        assert acl.check(caller_id="caller", target_id="target", context=ctx_admin) is True
        assert acl.check(caller_id="caller", target_id="target", context=ctx_reader) is False


# === Thread Safety ===


class TestACLThreadSafety:
    """Tests for ACL internal thread safety."""

    def test_concurrent_check_no_error(self) -> None:
        """Concurrent check() calls should not raise."""
        acl = ACL(
            rules=[
                ACLRule(callers=["api.*"], targets=["db.*"], effect="allow"),
                ACLRule(callers=["*"], targets=["admin.*"], effect="deny"),
            ],
            default_effect="deny",
        )
        errors: list[Exception] = []

        def checker() -> None:
            try:
                for _ in range(200):
                    acl.check(caller_id="api.handler", target_id="db.read")
                    acl.check(caller_id="random", target_id="admin.panel")
            except Exception as e:
                errors.append(e)

        threads = [threading.Thread(target=checker) for _ in range(10)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

        assert errors == []

    def test_concurrent_add_rule_and_check(self) -> None:
        """Concurrent add_rule() and check() should not raise or corrupt state."""
        acl = ACL(rules=[], default_effect="deny")
        errors: list[Exception] = []

        def adder() -> None:
            try:
                for i in range(50):
                    acl.add_rule(ACLRule(callers=[f"caller.{i}"], targets=["*"], effect="allow"))
            except Exception as e:
                errors.append(e)

        def checker() -> None:
            try:
                for _ in range(200):
                    acl.check(caller_id="caller.0", target_id="target")
            except Exception as e:
                errors.append(e)

        threads = [threading.Thread(target=adder) for _ in range(3)]
        threads += [threading.Thread(target=checker) for _ in range(5)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

        assert errors == []


# === effect value closure (PROTOCOL_SPEC §6.1.5, spec v1.30.0, apcore#111) ===


class TestEffectValueClosure:
    """`effect` is `allow` or `deny` at every door that accepts a rule.

    The check itself is not new — `ACL.load` has always had it. What was new in
    spec v1.30.0 is that it is reachable from all three entry points §6.1.6
    rule 3 names. Before, `effect: "Allow"` failed from a YAML file, was
    accepted through `ACLRule(...)` and `add_rule()`, and was then read as
    `deny` at check time: under `default_effect: allow` a rule written to permit
    denied everything it matched, with nothing said.

    `conformance/fixtures/acl_effect_value_closure.json` drives the cross-
    language contract; these cover the Python-shaped routes it cannot express.
    """

    @pytest.mark.parametrize("effect", ["Allow", "DENY", "alow", "", "permit"])
    def test_direct_construction_rejects_an_out_of_enum_effect(self, effect: str) -> None:
        with pytest.raises(ACLRuleError, match=f"ACLRule has invalid effect '{effect}'"):
            ACLRule(callers=["agent.*"], targets=["orders.*"], effect=effect)

    def test_add_rule_rejects_a_pre_built_rule(self) -> None:
        """The rule cannot be built to hand in, which is the rejection."""
        acl = ACL(default_effect="deny")
        with pytest.raises(ACLRuleError):
            acl.add_rule(ACLRule(callers=["agent.*"], targets=["orders.*"], effect="Allow"))
        assert acl.rules == ()

    def test_add_rule_rejects_its_own_kwargs_path(self) -> None:
        """`add_rule` returns None; §6.1.6 rule 3 says that is not an exemption."""
        acl = ACL(default_effect="deny")
        with pytest.raises(ACLRuleError, match="invalid effect 'Allow'"):
            acl.add_rule(callers=["agent.*"], targets=["orders.*"], effect="Allow")
        assert acl.rules == ()

    def test_the_effect_is_reported_before_the_approval_pairing(self) -> None:
        """`DENY` is not a `deny` rule, so the pairing rule must not be read off it."""
        with pytest.raises(ACLRuleError, match="invalid effect 'DENY'"):
            ACLRule(callers=["agent.*"], targets=["orders.*"], effect="DENY", approval="required")

    def test_a_mutated_effect_is_not_resolved_to_a_decision(self) -> None:
        """The one door a closed value set cannot shut: assignment after construction.

        `ACLRule` is a plain mutable dataclass, so `rule.effect = "Allow"`
        reaches the evaluator whatever the constructors check. §6.1.5 forbids
        resolving an unrecognised effect to a decision, and the old
        `"allow" if effect == "allow" else "deny"` did exactly that — here
        against `default_effect: allow`, where the silent reading flips the
        answer rather than merely agreeing with the default by luck.
        """
        rule = ACLRule(callers=["agent.*"], targets=["orders.*"], effect="allow")
        acl = ACL(rules=[rule], default_effect="allow")
        assert acl.check("agent.a", "orders.create") is True

        rule.effect = "Allow"
        with pytest.raises(ACLRuleError, match="Rule 0 has invalid effect 'Allow'"):
            acl.check("agent.a", "orders.create")

    def test_a_valid_effect_still_decides_both_ways(self) -> None:
        """Control: closing the set must not disturb the decision it carries."""
        allow = ACL(rules=[ACLRule(callers=["*"], targets=["*"], effect="allow")], default_effect="deny")
        deny = ACL(rules=[ACLRule(callers=["*"], targets=["*"], effect="deny")], default_effect="allow")
        assert allow.check("agent.a", "orders.create") is True
        assert deny.check("agent.a", "orders.create") is False


# === pattern-array arity closure (PROTOCOL_SPEC §6.2.1, spec v1.31.0, apcore#112) ===


def _write_acl(tmp_path: Path, rule: dict[str, object], default_effect: str = "deny") -> str:
    """Write a one-rule ACL file, so the loader door is exercised as a file."""
    body = json.dumps({"default_effect": default_effect, "rules": [rule]})
    path = tmp_path / "acl.yaml"
    path.write_text(body)  # JSON is a YAML subset, so safe_load reads it verbatim
    return str(path)


#: Every shape §6.2.1's closure rejects, as ``(id, field, patterns)``. Mirrored
#: from `conformance/fixtures/acl_pattern_arity.json` — the ids match the
#: fixture's so a divergence found there is greppable here. Both fields carry
#: every shape: §6.2.1 constrains ``callers`` and ``targets`` identically, and
#: an implementation that validates one and infers the other passes nearly every
#: single-field case, which is what the fixture's `*_in_callers_*` mirrors exist
#: to catch.
_REJECTED_SHAPES = [
    ("empty_array", []),
    ("or_with_no_operands", ["$or"]),
    ("not_with_no_operands", ["$not"]),
    ("not_with_two_operands", ["$not", "secrets.a", "secrets.b"]),
    ("empty_pattern_string", [""]),
    ("empty_pattern_string_under_or", ["$or", ""]),
    ("reserved_token_after_operator", ["$or", "$not", "a"]),
    ("reserved_token_in_flat_list", ["api.*", "$not", "cli.*"]),
    ("reserved_token_at_index_one", ["api.*", "$or"]),
]

#: Shapes that MUST keep loading. ``$orders.*`` is the control that pins
#: reserved-token detection as **equality** rather than a ``$`` prefix or a
#: substring: an implementation testing ``p.startswith("$")`` rejects every
#: shape above correctly and fails here.
_ACCEPTED_SHAPES = [
    ("flat_single_pattern", ["executor.*"]),
    ("flat_multi_pattern", ["api.*", "worker.*"]),
    ("or_with_two_operands", ["$or", "admin", "moderator"]),
    ("or_with_one_operand", ["$or", "admin"]),
    ("not_with_one_operand", ["$not", "banned.*"]),
    ("token_lookalike_pattern", ["api.*", "$orders.*"]),
    ("not_of_wildcard", ["$not", "*"]),
    ("external_sentinel", ["@external"]),
]


@pytest.mark.parametrize("field", ["callers", "targets"])
@pytest.mark.parametrize(("shape_id", "patterns"), _REJECTED_SHAPES, ids=[s[0] for s in _REJECTED_SHAPES])
class TestPatternArrayArityIsRejectedAtEveryDoor:
    """§6.2.1's closure holds at file loading, direct construction and runtime insertion.

    The defect: a pattern array with no operands can never match, all three SDKs
    returned ``False`` from the matcher, and the rule was therefore inert — the
    decision tracked ``default_effect`` exactly and ``validate_rules()`` reported
    nothing. On an ``allow`` rule that is merely useless. On a ``deny`` rule
    under ``default_effect: allow`` it is a **fail-open**: the call the operator
    wrote the rule to block is permitted, by a rule that loaded without error.

    ``schemas/acl-config.schema.json`` had declared ``minItems: 1`` and
    ``minLength: 1`` on both fields since the file existed, and no entry point
    enforced either — the third instance of apcore#107's and apcore#111's shape.
    The mechanism is the one §6.1.5 chose for both of those: close the doors.
    """

    def _rule(self, field: str, patterns: list[str]) -> dict[str, object]:
        rule: dict[str, object] = {"callers": ["*"], "targets": ["*"], "effect": "deny"}
        rule[field] = patterns
        return rule

    def test_direct_construction_is_rejected(self, shape_id: str, patterns: list[str], field: str) -> None:
        with pytest.raises(ACLRuleError, match=rf"ACLRule has an invalid '{field}'"):
            ACLRule(**self._rule(field, patterns))  # type: ignore[arg-type]

    def test_the_loader_is_rejected_and_names_the_rule_index(
        self, shape_id: str, patterns: list[str], field: str, tmp_path: Path
    ) -> None:
        """A YAML file reaches this: `ACL.load` rejects an *omitted* field and permitted an *empty* one."""
        path = _write_acl(tmp_path, self._rule(field, patterns))
        with pytest.raises(ACLRuleError, match=rf"Rule 0 has an invalid '{field}'"):
            ACL.load(path)

    def test_add_rule_is_rejected_on_its_kwargs_path(self, shape_id: str, patterns: list[str], field: str) -> None:
        """`add_rule` returns None; §6.1.6 rule 3 says that is not an exemption."""
        acl = ACL(default_effect="deny")
        with pytest.raises(ACLRuleError, match=rf"ACLRule has an invalid '{field}'"):
            acl.add_rule(**self._rule(field, patterns))  # type: ignore[arg-type]
        assert acl.rules == ()

    def test_add_rule_is_rejected_on_its_pre_built_path(self, shape_id: str, patterns: list[str], field: str) -> None:
        """The rule cannot be built to hand in, which is the rejection."""
        acl = ACL(default_effect="deny")
        with pytest.raises(ACLRuleError):
            acl.add_rule(ACLRule(**self._rule(field, patterns)))  # type: ignore[arg-type]
        assert acl.rules == ()


@pytest.mark.parametrize("field", ["callers", "targets"])
@pytest.mark.parametrize(("shape_id", "patterns"), _ACCEPTED_SHAPES, ids=[s[0] for s in _ACCEPTED_SHAPES])
class TestLegalPatternArraysStillLoad:
    """Controls against over-rejection, at every door.

    Without these a closure that rejects everything passes every case above.
    ``or_with_one_operand`` is the boundary — ``$or`` requires at least *one*
    operand, not at least two, so an implementation that read the rule as
    ``minItems: 2`` on the array rather than as an arity rule on the operator
    fails here. ``not_of_wildcard`` and ``external_sentinel`` are tier 2: they
    match nothing and are still well-formed, so they MUST load.
    """

    def _rule(self, field: str, patterns: list[str]) -> dict[str, object]:
        rule: dict[str, object] = {"callers": ["*"], "targets": ["*"], "effect": "allow"}
        rule[field] = patterns
        return rule

    def test_direct_construction_accepts(self, shape_id: str, patterns: list[str], field: str) -> None:
        assert ACLRule(**self._rule(field, patterns)) is not None  # type: ignore[arg-type]

    def test_the_loader_accepts(self, shape_id: str, patterns: list[str], field: str, tmp_path: Path) -> None:
        acl = ACL.load(_write_acl(tmp_path, self._rule(field, patterns)))
        assert len(acl.rules) == 1

    def test_add_rule_accepts(self, shape_id: str, patterns: list[str], field: str) -> None:
        acl = ACL(default_effect="deny")
        acl.add_rule(**self._rule(field, patterns))  # type: ignore[arg-type]
        assert len(acl.rules) == 1


class TestPatternArrayArityHeadlineCases:
    """The cells apcore#112 was filed about, and the second defect found beside them."""

    def test_the_driving_case_no_longer_loads(self, tmp_path: Path) -> None:
        """`targets: []` on a `deny` rule under `default_effect: allow`.

        Written as YAML this loaded clean, `validate_rules()` returned zero
        findings and `check(None, "cli.rm", None)` returned **True** — the
        operator has a rule that says "block everything dangerous" and a
        deployment that blocks nothing. The rejection turns a silent runtime
        fail-open into a boot-time failure naming the field.
        """
        path = _write_acl(
            tmp_path,
            {"callers": ["*"], "targets": [], "effect": "deny", "description": "block everything dangerous"},
            default_effect="allow",
        )
        with pytest.raises(ACLRuleError, match=r"Rule 0 has an invalid 'targets'"):
            ACL.load(path)

    def test_the_message_names_both_readings_of_an_empty_array(self) -> None:
        """The failure is at boot, where a good message is the whole remedy (§6.2.1)."""
        with pytest.raises(ACLRuleError) as excinfo:
            ACLRule(callers=["*"], targets=[], effect="deny")
        message = str(excinfo.value)
        assert '["*"]' in message and "delete it" in message

    def test_multi_operand_not_is_rejected_rather_than_implementation_defined(self) -> None:
        """The second defect, at the door.

        §6.2.1 through v1.30.0 made `["$not", p1, p2, …]` implementation-defined
        — consult `p1`, ignore the rest — and all three SDKs did exactly that,
        so the form was uniform across implementations and uniformly **wider
        than written**: on an `allow` rule the operator excluded two targets and
        the second one was granted. `SHOULD NOT rely on this form` reported
        nothing and rejected nothing.
        """
        with pytest.raises(ACLRuleError, match=r"exactly one pattern, got 2"):
            ACLRule(callers=["*"], targets=["$not", "secrets.a", "secrets.b"], effect="allow")

    def test_multi_operand_not_is_rejected_on_a_deny_rule_too(self) -> None:
        """The same fault where the old reading was over-broad rather than escalating.

        It fails for the same reason rather than surviving because this effect
        happens to land on the safe side — the fallback is only accidentally
        right, and right only until someone flips the effect.
        """
        with pytest.raises(ACLRuleError, match=r"exactly one pattern, got 2"):
            ACLRule(callers=["*"], targets=["$not", "secrets.a", "secrets.b"], effect="deny")

    def test_a_reserved_token_outside_index_zero_does_not_nest(self) -> None:
        """A pattern array is FLAT — the operators do not nest and there is no precedence.

        `$or` / `$not` nest arbitrarily in `conditions` and not at all here, and
        the specification never said so. An operator who learned the condition
        grammar wrote `["$or", "$not", "a"]` expecting or-of-not and got an OR
        of two literals — matching `a`, and also matching a module literally
        named `$not`, which §6.2.1's own reserved-token MUST NOT forbids.
        Rejecting the token outside index 0 makes that clause hold by
        construction.
        """
        with pytest.raises(ACLRuleError, match=r"reserved token '\$not' appears at index 1"):
            ACLRule(callers=["*"], targets=["$or", "$not", "a"], effect="allow")

    def test_reserved_token_detection_is_equality_not_a_dollar_prefix(self) -> None:
        """`$orders.*` is an ordinary pattern that merely begins with the same character."""
        rule = ACLRule(callers=["api.*", "$orders.*"], targets=["*"], effect="allow")
        acl = ACL(rules=[rule], default_effect="deny")
        assert acl.check("api.gateway", "anything") is True

    def test_a_reserved_token_at_index_zero_is_still_an_operator(self) -> None:
        """Control for the case above: the closure is positional, not lexical."""
        acl = ACL(rules=[ACLRule(callers=["*"], targets=["$not", "cli.*"], effect="deny")], default_effect="allow")
        assert acl.check("api.gateway", "cli.rm") is True
        assert acl.check("api.gateway", "executor.email.send") is False

    def test_the_effect_is_still_reported_before_the_pattern_arrays(self) -> None:
        """Ordering control: a rule wrong on both axes reports the value the earlier check names."""
        with pytest.raises(ACLRuleError, match=r"invalid effect 'Allow'"):
            ACLRule(callers=[], targets=[], effect="Allow")


class TestAddRuleReValidatesTheRuleItIsHanded:
    """§6.2.1: runtime insertion validates the rule **at that moment**, whatever its history.

    Resolved normatively in spec v1.31.0 because two of three implementations
    re-validated and one did not, and `acl_pattern_arity.json` cannot express
    the difference: `entry_points` deliberately carries no per-door expectation,
    so a shape rejected at construction reads as rejected at every door listed.

    This SDK was the one that did not. The pattern closure was threaded through
    `ACLRule.__post_init__`, which reaches `add_rule` only *through
    construction* — and `ACLRule` is a non-frozen dataclass, so a rule that was
    well-formed when built and has had `callers` or `targets` assigned since
    walks straight past it. §6.1.5's v1.30.0 text leaves mutation-then-use to a
    **MAY** for `effect`, which is sound there because a closed `effect` is
    never read again; a pattern array **is** read, by the matcher, on the next
    `check()`.
    """

    def test_a_rule_mutated_after_construction_is_rejected(self) -> None:
        """The exact sequence §6.2.1 names, which inserted without raising before.

        The rule was well-formed when constructed, so `__post_init__` passed;
        the assignment is the one route no constructor intercepts. Without the
        re-validation the ACL ends up holding a rule §6.2.1 forbids, which
        `_precheck_patterns` then reports as UNEVALUABLE on every subsequent
        check — the fault landing at each `check()` instead of at the door.
        """
        rule = ACLRule(callers=["*"], targets=["*"], effect="deny")
        rule.targets = []

        acl = ACL(default_effect="allow")
        with pytest.raises(ACLRuleError, match=r"ACLRule has an invalid 'targets'"):
            acl.add_rule(rule)
        assert acl.rules == ()

    def test_a_mutated_callers_field_is_rejected_too(self) -> None:
        """Both fields, or an implementation that checks one passes half the fixture."""
        rule = ACLRule(callers=["*"], targets=["*"], effect="deny")
        rule.callers = ["$not"]

        acl = ACL(default_effect="allow")
        with pytest.raises(ACLRuleError, match=r"ACLRule has an invalid 'callers'"):
            acl.add_rule(rule)
        assert acl.rules == ()

    def test_a_pre_built_well_formed_rule_still_inserts(self) -> None:
        """The control. A door that rejects everything satisfies the case above."""
        rule = ACLRule(callers=["api.*"], targets=["executor.*"], effect="allow")

        acl = ACL(default_effect="deny")
        acl.add_rule(rule)

        assert len(acl.rules) == 1
        assert acl.rules[0] is rule
        assert acl.check("api.gateway", "executor.email.send") is True

    def test_a_rule_mutated_back_into_shape_is_accepted(self) -> None:
        """The check reads the rule's *current* value, not a verdict cached at construction."""
        rule = ACLRule(callers=["*"], targets=["*"], effect="deny")
        rule.targets = []
        rule.targets = ["cli.*"]

        acl = ACL(default_effect="allow")
        acl.add_rule(rule)

        assert len(acl.rules) == 1
        assert acl.check("api.gateway", "cli.rm") is False


class TestRuleValidationOrder:
    """§6.2.1: a rule bad on more than one axis is refused for `effect`, then `approval`, then the patterns.

    Resolved normatively in spec v1.31.0 for the same reason the door above was:
    `{callers: [], targets: [], effect: "Allow"}` was refused for its `effect` in
    one implementation and for its patterns in another, both conformant because
    nothing said otherwise. apcore-rust ordered it the other way and moves; this
    SDK already ordered it this way, so these tests exist to keep the three from
    drifting apart again rather than to change anything here.

    §6.2.1 states this order for the first time. §6.1.6 rule 2 *implies* that
    `effect` is read before `approval` — judging "`deny` plus
    `approval: required`" requires knowing the effect — but states no order, and
    an implementer reading §6.1.6 alone will not find one.
    """

    def test_the_effect_is_reported_before_the_patterns(self) -> None:
        """The rule §6.2.1 quotes verbatim as the one that got two answers."""
        with pytest.raises(ACLRuleError, match=r"invalid effect 'Allow'"):
            ACLRule(callers=[], targets=[], effect="Allow")

    def test_the_effect_is_reported_before_the_approval(self) -> None:
        """`DENY` is not a `deny` rule, so the pairing rule must not be read off it."""
        with pytest.raises(ACLRuleError, match=r"invalid effect 'DENY'"):
            ACLRule(callers=[], targets=[], effect="DENY", approval="required")

    def test_the_approval_is_reported_before_the_patterns(self) -> None:
        """The middle step, which no earlier test pinned.

        `approval: required` on a `deny` rule names a state that means nothing,
        and it is reported ahead of the pattern arrays — so a rule wrong on both
        gets the same message in every implementation.
        """
        with pytest.raises(ACLRuleError, match=r"carries approval: required on a 'deny' rule"):
            ACLRule(callers=[], targets=[], effect="deny", approval="required")

    def test_callers_is_reported_before_targets(self) -> None:
        """Field order within the last step, so the two arrays cannot swap either."""
        with pytest.raises(ACLRuleError, match=r"ACLRule has an invalid 'callers'"):
            ACLRule(callers=[], targets=[], effect="deny")

    def test_the_order_holds_at_runtime_insertion_too(self) -> None:
        """The order is a property of the rule, not of the door that refuses it.

        `add_rule` re-validates what it is handed, so it runs the same sequence;
        an implementation that open-coded a second check here could order it
        differently and pass every test above.
        """
        acl = ACL(default_effect="deny")
        rule = ACLRule(callers=["*"], targets=["*"], effect="allow")
        rule.effect = "Allow"
        rule.callers = []

        with pytest.raises(ACLRuleError, match=r"invalid effect 'Allow'"):
            acl.add_rule(rule)
        assert acl.rules == ()

    def test_the_type_fault_is_reported_before_the_shape_fault(self) -> None:
        """The pattern fields are ONE axis, and within it the type fault comes first.

        A value must be a list of strings before its arity means anything. The
        type fault is not a door rejection — it is unrepresentable in
        apcore-rust, so §6.1.4.1 leaves it to the precheck — which is why this
        is asserted where both faults can be seen at once.
        """
        captured: list[AuditEntry] = []
        rule = ACLRule(callers=["*"], targets=["*"], effect="deny")
        acl = ACL(rules=[rule], default_effect="allow", audit_logger=captured.append)

        # `""` is empty AND the wrong type. The type reading wins: a bare string
        # is not a pattern array at all, so it has no arity to be wrong about.
        rule.callers = ""  # type: ignore[assignment]
        acl.check("api.gateway", "cli.rm")

        error = captured[0].handler_error
        assert error is not None
        assert error.startswith("callers: callers must be a list of strings, got str"), error

        # And the field order within the axis, with a fault on each.
        rule.targets = []
        assert [f.condition_path for f in acl.validate_rules()] == ["callers", "targets"]


class TestTheLowestIndexedBadRuleIsTheOneRefused:
    """§6.2.1: rule index dominates all three axes.

    A rule set with more than one bad rule MUST be refused for the
    **lowest-indexed** bad rule, and an implementation MUST NOT sweep one axis
    across every rule before looking at the next. Both halves exist so that one
    file produces one error, whichever door it arrives through and whichever
    implementation reads it — an implementation that swept axis by axis reported
    a lower-indexed rule's pattern fault from its loader and a higher-indexed
    rule's `effect` fault from its constructor, for the same file.
    """

    def test_the_loader_refuses_rule_zero_even_when_rule_one_fails_an_earlier_axis(self, tmp_path: Path) -> None:
        """Rule 0's patterns, not rule 1's `effect`.

        This is the sweep the MUST NOT forbids: `effect` precedes the pattern
        axis *within a rule*, so an implementation that checked every rule's
        `effect` first would name rule 1 here.
        """
        body = json.dumps(
            {
                "default_effect": "deny",
                "rules": [
                    {"callers": [], "targets": ["*"], "effect": "allow"},
                    {"callers": ["*"], "targets": ["*"], "effect": "Allow"},
                ],
            }
        )
        path = tmp_path / "acl.yaml"
        path.write_text(body)

        with pytest.raises(ACLRuleError, match=r"Rule 0 has an invalid 'callers'"):
            ACL.load(str(path))

    def test_the_loader_refuses_rule_zero_when_the_faults_are_the_other_way_round(self, tmp_path: Path) -> None:
        """The control, so the test above cannot pass on a hardcoded axis preference."""
        body = json.dumps(
            {
                "default_effect": "deny",
                "rules": [
                    {"callers": ["*"], "targets": ["*"], "effect": "Allow"},
                    {"callers": [], "targets": ["*"], "effect": "allow"},
                ],
            }
        )
        path = tmp_path / "acl.yaml"
        path.write_text(body)

        with pytest.raises(ACLRuleError, match=r"Rule 0 has invalid effect 'Allow'"):
            ACL.load(str(path))

    def test_direct_construction_refuses_the_same_rule_the_loader_does(self) -> None:
        """The two doors MUST agree, which is the whole point of the cross-rule half.

        `ACL(rules=[...])` validates each `ACLRule` as it is constructed, so the
        list is walked in index order and the first bad rule raises — the same
        answer `ACL.load` gives for the same file.
        """
        with pytest.raises(ACLRuleError, match=r"ACLRule has an invalid 'callers'"):
            ACL(
                rules=[
                    ACLRule(callers=[], targets=["*"], effect="allow"),
                    ACLRule(callers=["*"], targets=["*"], effect="Allow"),
                ],
                default_effect="deny",
            )

    @pytest.mark.parametrize(
        ("rule_zero", "expected"),
        [
            ({"callers": ["*"], "targets": ["*"], "effect": "Allow"}, r"Rule 0 has invalid effect 'Allow'"),
            ({"callers": ["*"], "targets": ["*"], "effect": "allow", "priority": 3}, r"Rule 0 carries 'priority'"),
            ({"targets": ["*"], "effect": "allow"}, r"Rule 0 missing required key 'callers'"),
            ({"callers": [], "targets": ["*"], "effect": "allow"}, r"Rule 0 has an invalid 'callers'"),
        ],
        ids=["effect", "unknown_key", "missing_field", "pattern_shape"],
    )
    def test_a_loader_only_axis_does_not_escape_the_per_rule_loop(
        self, rule_zero: dict[str, object], expected: str, tmp_path: Path
    ) -> None:
        """ "Axis" is EVERY per-rule check a door performs, not only effect / approval / patterns.

        A loader has axes the other doors cannot have — #107's rule-key closure,
        the missing-field check, the value-type checks — and the sweep
        prohibition binds those too. apcore-rust closed the key set across the
        WHOLE FILE before any rule's `effect` was read, so a file bad at rule 0
        was refused for rule 1. Rule 1 here carries an unknown key, so any check
        that escapes the per-rule loop names it and fails this.
        """
        body = json.dumps(
            {
                "default_effect": "deny",
                "rules": [rule_zero, {"callers": ["*"], "targets": ["*"], "effect": "allow", "priority": 3}],
            }
        )
        path = tmp_path / "acl.yaml"
        path.write_text(body)

        with pytest.raises(ACLRuleError, match=expected):
            ACL.load(str(path))


class TestDefaultEffectIsJudgedBeforeAnyRule:
    """§6.2.1: `default_effect` is judged FIRST, before any rule, at every door.

    It is not a rule and has no index, so the rule ordering never reaches it —
    and a configuration wrong in both `default_effect` and a rule is exactly the
    one-file-one-error case the ordering exists for. Unstated before v1.31.0,
    and implementations differed.

    This SDK left it entirely to the `ACL` constructor, which `ACL.load` reaches
    only at the very bottom, after every rule has been parsed and validated. So
    a file carrying a bad `default_effect` AND a bad rule 0 was refused for the
    RULE through the loader and for `default_effect` through direct
    construction: one configuration, two answers, from two doors of one SDK.
    """

    def test_the_loader_names_the_default_effect_and_not_rule_zero(self, tmp_path: Path) -> None:
        body = json.dumps(
            {
                "default_effect": "Allow",
                "rules": [{"callers": [], "targets": ["*"], "effect": "allow"}],
            }
        )
        path = tmp_path / "acl.yaml"
        path.write_text(body)

        with pytest.raises(ACLRuleError, match=r"Invalid default_effect 'Allow'"):
            ACL.load(str(path))

    def test_the_constructor_judges_it_before_it_looks_at_the_rules(self) -> None:
        """The rules are already built here, so "first" is the first thing it does."""
        with pytest.raises(ACLRuleError, match=r"Invalid default_effect 'Allow'"):
            ACL(rules=[ACLRule(callers=["*"], targets=["*"], effect="allow")], default_effect="Allow")

    def test_it_is_judged_with_no_rules_at_all(self) -> None:
        """It has no index and needs no rule; a rule-less config is still refused."""
        with pytest.raises(ACLRuleError, match=r"Invalid default_effect 'Allow'"):
            ACL(default_effect="Allow")

    def test_a_good_default_effect_still_lets_the_rule_fault_through(self, tmp_path: Path) -> None:
        """Control. Judging it first must not swallow the fault that follows it."""
        body = json.dumps(
            {
                "default_effect": "allow",
                "rules": [{"callers": [], "targets": ["*"], "effect": "allow"}],
            }
        )
        path = tmp_path / "acl.yaml"
        path.write_text(body)

        with pytest.raises(ACLRuleError, match=r"Rule 0 has an invalid 'callers'"):
            ACL.load(str(path))

    @pytest.mark.parametrize(
        "document",
        [
            {"default_effect": "Allow"},
            {"default_effect": "Allow", "rules": "nope"},
            {"default_effect": "Allow", "rules": {"a": 1}},
        ],
        ids=["rules_missing", "rules_not_a_list", "rules_a_mapping"],
    )
    def test_first_means_ahead_of_the_file_level_checks_on_the_rules_collection(
        self, document: dict[str, object], tmp_path: Path
    ) -> None:
        """§6.2.1: "first" is ahead of the checks on the `rules` COLLECTION, not merely ahead of the rules.

        A document both missing (or malforming) `rules` and carrying an
        unrecognised `default_effect` is refused for the `default_effect`. No
        conformance case covers this combination — `acl_pattern_arity.json`
        always supplies a well-formed `rules` list — so the fixture cannot catch
        a drift here and this test is the only guard.
        """
        path = tmp_path / "acl.yaml"
        path.write_text(json.dumps(document))

        with pytest.raises(ACLRuleError, match=r"Invalid default_effect 'Allow'"):
            ACL.load(str(path))

    @pytest.mark.parametrize(
        ("document", "expected"),
        [
            ({"default_effect": "allow"}, r"missing required 'rules' key"),
            ({"default_effect": "allow", "rules": "nope"}, r"'rules' must be a list, got str"),
        ],
        ids=["rules_missing", "rules_not_a_list"],
    )
    def test_the_file_level_checks_still_fire_behind_a_good_default_effect(
        self, document: dict[str, object], expected: str, tmp_path: Path
    ) -> None:
        """Control for the boundary above: moving `default_effect` ahead must not swallow them."""
        path = tmp_path / "acl.yaml"
        path.write_text(json.dumps(document))

        with pytest.raises(ACLRuleError, match=expected):
            ACL.load(str(path))


class TestPatternArrayArityBackstop:
    """§6.1.4.1 classifies a shape assigned onto a rule already installed in a
    live ACL and mutated afterward through a reference the caller already holds.

    ``ACLRule`` is a non-frozen dataclass, so mutating a rule already inside a
    constructed ``ACL`` (via the public ``rules`` accessor, exactly as a real
    caller would have to) is the one route no door runs again to intercept.
    §6.1.5's v1.30.0 reasoning for why the ``effect`` closure needs no backstop
    — the value is never read again once the doors are shut — does **not**
    transfer: a mutated pattern array *is* read, because the matcher consults
    it on the next ``check()``.

    Spec v1.33.0 disambiguates "assigned onto an already-constructed rule"
    between this scenario (already past every door, safe to leave to the
    backstop) and a rule mutated *before* it is ever first offered to a door —
    which ``ACL(rules=[...])`` itself now rejects; see
    ``TestConstructionRejectsAPreviouslyMutatedRule`` below for that case. This
    class covered the wrong one before the disambiguation: every parametrized
    test here used to mutate the rule *before* the first ``ACL(...)`` call
    that would ever see it — which was never distinguishable from this,
    correct, "already installed" scenario by its outcome (both are safe at
    ``check()`` time), only by whether construction had already succeeded.
    """

    @staticmethod
    def _mutated(field: str, value: list[str], *, effect: str, default_effect: str, **kwargs: object) -> ACL:
        rule = ACLRule(callers=["*"], targets=["*"], effect=effect, **kwargs)  # type: ignore[arg-type]
        # Construct FIRST, while `rule` is still well-formed — `ACL.__init__`
        # validates every rule it is handed (spec v1.33.0), so a rule built
        # this way would be rejected if mutated before this point.
        acl = ACL(rules=[rule], default_effect=default_effect)
        # Mutate through the PUBLIC accessor, not the closed-over local `rule`
        # reference — this is the route a real caller has, and the one
        # apcore-rust's `&[ACLRule]` (no `rules_mut`) deliberately does not
        # expose. Mirrors the conformance driver's `_run_backstop` exactly.
        installed = acl.rules[0]
        setattr(installed, field, value)
        return acl

    @pytest.mark.parametrize("value", [[], ["$or"], ["$not"], ["$not", "a", "b"], [""], ["api.*", "$or"]])
    def test_a_mutated_deny_rule_takes_effect_and_denies(self, value: list[str]) -> None:
        """§6.1.1's effect table: unevaluable means a `deny` rule denies."""
        acl = self._mutated("targets", value, effect="deny", default_effect="allow")
        assert acl.check("api.gateway", "cli.rm") is False

    @pytest.mark.parametrize("value", [[], ["$or"], ["$not"], ["$not", "a", "b"], [""], ["api.*", "$or"]])
    def test_a_mutated_allow_rule_does_not_grant(self, value: list[str]) -> None:
        """The other half of the table: it does not match, so `default_effect` decides."""
        acl = self._mutated("targets", value, effect="allow", default_effect="deny")
        assert acl.check("api.gateway", "cli.rm") is False

    def test_the_multi_operand_not_escalation_is_closed_at_runtime_too(self) -> None:
        """The regression guard for the second defect.

        Through v1.30.0 this returned **True** for `secrets.b` — the second
        target the operator excluded — because the matcher read `p1` and dropped
        the rest. An answer of True here is the pre-v1.31.0 matcher.
        """
        acl = self._mutated("targets", ["$not", "secrets.a", "secrets.b"], effect="allow", default_effect="deny")
        assert acl.check("api.gateway", "secrets.b") is False

    def test_the_fault_reaches_handler_error_and_validate_rules(self) -> None:
        captured: list[AuditEntry] = []
        rule = ACLRule(callers=["*"], targets=["*"], effect="deny")
        acl = ACL(rules=[rule], default_effect="allow", audit_logger=captured.append)
        rule.targets = []

        assert acl.check("api.gateway", "cli.rm") is False
        assert captured[0].handler_error is not None
        assert captured[0].handler_error.startswith("targets: ")

        findings = acl.validate_rules()
        assert [(f.rule_index, f.condition_path) for f in findings] == [(0, "targets")]
        assert findings[0].condition_key is None
        assert findings[0].sync_resolvable is False
        assert findings[0].async_resolvable is False

    def test_a_reason_phrase_never_carries_the_handler_error_separator(self) -> None:
        """§6.1.1 rule 2 makes `"; "` the separator between diagnostics.

        A reason containing one splits into two entries, the second of which has
        no path — which is how a cross-language driver reading
        `handler_error.split("; ")` sees a path that does not exist.
        """
        captured: list[AuditEntry] = []
        rule = ACLRule(callers=["*"], targets=["*"], effect="deny")
        acl = ACL(rules=[rule], default_effect="allow", audit_logger=captured.append)
        for value in (
            [],
            ["$or"],
            ["$not"],
            ["$not", "a", "b"],
            [""],
            ["$or", ""],
            ["api.*", "$or"],
            ["$or", "$not", "a"],
        ):
            captured.clear()
            rule.targets = value
            acl.check("api.gateway", "cli.rm")
            error = captured[0].handler_error
            assert error is not None
            assert [part.split(": ", 1)[0] for part in error.split("; ")] == ["targets"]

    def test_both_fields_are_reported_without_short_circuiting(self) -> None:
        """§6.1.4 rule 3, ordered lexicographically by path (§6.1.1 rule 2)."""
        captured: list[AuditEntry] = []
        rule = ACLRule(callers=["*"], targets=["*"], effect="deny")
        acl = ACL(rules=[rule], default_effect="allow", audit_logger=captured.append)
        rule.callers = ["$not"]
        rule.targets = []

        assert acl.check("api.gateway", "cli.rm") is False
        assert captured[0].handler_error is not None
        assert [p.split(": ", 1)[0] for p in captured[0].handler_error.split("; ")] == ["callers", "targets"]
        assert [f.condition_path for f in acl.validate_rules()] == ["callers", "targets"]

    def test_an_approval_rule_raises_the_pending_requirement(self) -> None:
        """§6.1.1 rule 5 — "unknowable scope counts as scope" — applies unchanged.

        An arity fault is a malformed pattern field like any other. §6.1.4.1 has
        no partially-readable tier, and inventing one — `targets: []` is legible
        as an *empty* scope in a way `targets: 3` is not — would put back the
        per-implementation judgement call that produced three different answers
        in apcore#100, resolving toward asking a human less often.
        """
        acl = self._mutated("targets", [], effect="allow", default_effect="allow", approval="required")
        decision = acl.check_access("api.gateway", "cli.rm")
        assert decision.access == "allow"
        assert decision.approval_required is True
        assert decision.matched_rule_index is None

    def test_a_well_formed_rule_still_decides_normally(self) -> None:
        """Control, without which an implementation that faults every rule passes the rest."""
        captured: list[AuditEntry] = []
        acl = ACL(
            rules=[ACLRule(callers=["*"], targets=["cli.*"], effect="deny")],
            default_effect="allow",
            audit_logger=captured.append,
        )
        assert acl.check("api.gateway", "cli.rm") is False
        assert captured[0].handler_error is None
        assert acl.validate_rules() == ()


class TestConstructionRejectsAPreviouslyMutatedRule:
    """§6.1.4.1 / §6.2.1 (spec v1.33.0): construction history does not exempt.

    A rule mutated to a malformed shape BEFORE it is ever offered to a door —
    including ``ACL``'s own constructor, which accepts a list of already-built
    ``ACLRule`` objects exactly as ``add_rule`` accepts one — is being offered
    to that door for the first time. It MUST be rejected there like any other
    rule reaching that door, the same as ``add_rule`` already rejects a rule
    mutated after its own construction (§6.1.6 rule 3). This is the case
    ``TestPatternArrayArityBackstop`` covered by mistake before the spec
    disambiguated "assigned onto an already-constructed rule": that class now
    constructs the ACL *before* mutating (installed-then-mutated, genuinely
    uninterceptable); this class mutates *before* the ACL the rule is handed to
    has ever been constructed, which IS interceptable and must raise.
    """

    @staticmethod
    def _mutated_then_constructed(field: str, value: list[str], *, effect: str, default_effect: str) -> ACL:
        rule = ACLRule(callers=["*"], targets=["*"], effect=effect)
        setattr(rule, field, value)
        return ACL(rules=[rule], default_effect=default_effect)

    @pytest.mark.parametrize("value", [[], ["$or"], ["$not"], ["$not", "a", "b"], [""], ["api.*", "$or"]])
    def test_targets_mutated_before_first_construction_is_rejected(self, value: list[str]) -> None:
        with pytest.raises(ACLRuleError):
            self._mutated_then_constructed("targets", value, effect="deny", default_effect="allow")

    @pytest.mark.parametrize("value", [[], ["$or"], ["$not"], ["$not", "a", "b"], [""], ["api.*", "$or"]])
    def test_callers_mutated_before_first_construction_is_rejected(self, value: list[str]) -> None:
        with pytest.raises(ACLRuleError):
            self._mutated_then_constructed("callers", value, effect="allow", default_effect="deny")

    def test_a_rule_mutated_then_passed_in_a_multi_rule_list_is_rejected(self) -> None:
        """The list form: the first malformed rule is refused, same as a
        rule built with a bad value directly — construction history is per
        rule, and one well-formed neighbour does not rescue it."""
        good = ACLRule(callers=["*"], targets=["cli.*"], effect="allow")
        bad = ACLRule(callers=["*"], targets=["*"], effect="deny")
        bad.targets = []
        with pytest.raises(ACLRuleError):
            ACL(rules=[good, bad], default_effect="deny")


class TestPatternArrayNeverMatches:
    """Tier 2 (§6.2.1): well-formed, matches nothing, reported and never enforced.

    Closing the arities does not exhaust the inert class — `["$not", "*"]` has
    perfectly legal arity, exactly one operand, and matches nothing, producing
    the identical fail-open. Detecting it needs reasoning about the **match
    relation** rather than the array's shape, and that predicate cannot be closed
    without freezing the pattern language, so it is a `validate_rules()` finding:
    an incomplete predicate is survivable in a validator and not at a door, where
    it would mean the same ACL file loads in one language and fails in another.
    """

    @staticmethod
    def _findings(field: str, patterns: list[str], effect: str = "deny") -> list[tuple[str, str | None]]:
        rule: dict[str, object] = {"callers": ["*"], "targets": ["*"], "effect": effect}
        rule[field] = patterns
        acl = ACL(rules=[ACLRule(**rule)], default_effect="allow")  # type: ignore[arg-type]
        return [(f.condition_path, f.condition_key) for f in acl.validate_rules()]

    @pytest.mark.parametrize("wildcard", ["*", "**", "***"])
    def test_not_of_a_universal_pattern_is_reported(self, wildcard: str) -> None:
        """`!true` is false for every input, so the rule fires for nothing.

        The MUST-detect minimum is "any pattern consisting only of wildcards",
        not the single literal `*`: `**` is universal too, and an implementation
        that string-compares against `"*"` reports one and not the other.
        """
        assert self._findings("targets", ["$not", wildcard]) == [("targets", None)]

    def test_the_finding_carries_a_null_key_and_both_flags_false(self) -> None:
        acl = ACL(
            rules=[ACLRule(callers=["*"], targets=["$not", "*"], effect="deny")],
            default_effect="allow",
        )
        (finding,) = acl.validate_rules()
        assert finding.condition_path == "targets"
        assert finding.condition_key is None
        assert finding.sync_resolvable is False
        assert finding.async_resolvable is False
        assert finding.effect == "deny"

    def test_the_external_sentinel_is_reported_as_a_target_and_not_as_a_caller(self) -> None:
        """Field-specific: `@external` is the caller-side sentinel §6.5 substitutes for a null caller.

        No module ID is `@external`, so as a **target** pattern it matches
        nothing. In `callers` it is the documented way to write a rule about
        top-level entry points, and a finding that fires on both fields has read
        the rule as being about the token rather than about the field.
        """
        assert self._findings("targets", ["@external"]) == [("targets", None)]
        assert self._findings("callers", ["@external"]) == []

    @pytest.mark.parametrize(
        "patterns",
        [
            ["$not", "cli.*"],
            ["api.*", "@external"],
            ["$or", "@external", "api.*"],
            ["$or", "api.*", "worker.*"],
            ["*"],
            ["$not", "a*b"],
        ],
    )
    def test_tier_two_does_not_over_fire(self, patterns: list[str]) -> None:
        """The criterion judges the array AS A WHOLE, not element by element.

        §6.2.1: report an array that "matches no legal module ID for any input".
        A flat or `$or` array qualifies only when **every** operand is
        unmatchable, so `["api.*", "@external"]` is not reported — `api.*` still
        matches. An implementation that reports any occurrence of `@external` in
        `targets` has implemented the MUST-detect list rather than the criterion
        it is a minimum for. Likewise `["$not", "cli.*"]` matches every module
        outside `cli.`, which is the form's whole purpose: reporting every `$not`
        confuses *negation* with *matches nothing*.
        """
        assert self._findings("targets", patterns) == []

    def test_an_all_unmatchable_or_array_is_still_reported(self) -> None:
        """The other side of the same criterion: every operand unmatchable, so the array is."""
        assert self._findings("targets", ["$or", "@external"]) == [("targets", None)]

    def test_a_tier_two_rule_still_decides_normally(self) -> None:
        """MUST NOT change any access decision — the whole reason it is a finding.

        `["$not", "*"]` on a `deny` rule under `default_effect: allow` matches
        nothing, so an unrelated target falls through to the default exactly as
        it did before v1.31.0. A denial here means tier 2 was implemented as a
        rejection or as an UNEVALUABLE fault, which would make a well-formed
        rule deny every call.
        """
        captured: list[AuditEntry] = []
        acl = ACL(
            rules=[ACLRule(callers=["*"], targets=["$not", "*"], effect="deny")],
            default_effect="allow",
            audit_logger=captured.append,
        )
        assert acl.check("api.gateway", "cli.rm") is True
        assert captured[0].handler_error is None
        assert [f.condition_path for f in acl.validate_rules()] == ["targets"]

    def test_tier_two_is_not_consulted_where_tier_one_already_faulted(self) -> None:
        """One path, one finding: a shape-faulty array has no readable match relation."""
        rule = ACLRule(callers=["*"], targets=["*"], effect="deny")
        acl = ACL(rules=[rule], default_effect="allow")
        rule.targets = []
        assert [f.condition_path for f in acl.validate_rules()] == ["targets"]
