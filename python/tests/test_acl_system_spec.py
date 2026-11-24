"""Spec-traced contract tests for the apcore ACL system.

Generated from /apcore/docs/features/acl-system.md (## Contract: blocks).
Each test embeds a verbatim clause id of the form
``acl_system.<method>.<kind>.<detail>`` so cross-language diffs line up.

These are TESTS ONLY — they never modify production source.
"""

from __future__ import annotations

import asyncio
import textwrap
from pathlib import Path

import pytest

from apcore.acl import ACL, ACLRule
from apcore.errors import ACLRuleError, ConfigNotFoundError


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _write_yaml(tmp_path: Path, body: str, name: str = "acl.yaml") -> str:
    path = tmp_path / name
    path.write_text(textwrap.dedent(body), encoding="utf-8")
    return str(path)


_VALID_YAML = """\
version: "1.0"
default_effect: deny
rules:
  - callers: ["api.*"]
    targets: ["db.*"]
    effect: allow
    description: "API to DB"
"""


# ===========================================================================
# Contract: ACL.check
# ===========================================================================


class TestACLCheckContract:
    # acl_system.check.property.async
    def test_acl_system_check_property_async_is_sync(self) -> None:
        """acl_system.check.property.async — check() is declared async: false (plain bool)."""
        acl = ACL(rules=[ACLRule(callers=["*"], targets=["*"], effect="allow")])
        result = acl.check(caller_id="api.x", target_id="db.y")
        assert not asyncio.iscoroutine(result)
        assert result is True

    # acl_system.check.property.thread_safe
    def test_acl_system_check_property_thread_safe_concurrent(self) -> None:
        """acl_system.check.property.thread_safe — N>=8 concurrent checks, no panic, consistent state."""
        acl = ACL(
            rules=[ACLRule(callers=["api.*"], targets=["db.*"], effect="allow")],
            default_effect="deny",
        )

        async def run() -> list[bool]:
            async def one(i: int) -> bool:
                # Distinct inputs per coroutine.
                return await asyncio.to_thread(acl.check, f"api.{i}", "db.read")

            return await asyncio.gather(*[one(i) for i in range(16)])

        results = asyncio.run(run())
        assert len(results) == 16
        assert all(r is True for r in results)
        # Final state unchanged: rule list still intact.
        assert acl.check("api.gateway", "db.read") is True
        assert acl.check("other", "db.read") is False

    # acl_system.check.property.idempotent
    def test_acl_system_check_property_idempotent(self) -> None:
        """acl_system.check.property.idempotent — identical inputs yield identical decisions."""
        acl = ACL(
            rules=[ACLRule(callers=["api.*"], targets=["db.*"], effect="allow")],
            default_effect="deny",
        )
        first = acl.check("api.gateway", "db.query")
        second = acl.check("api.gateway", "db.query")
        assert first == second == True  # noqa: E712
        # Observable state after repeated calls is identical (deny path too).
        assert acl.check("nope", "db.query") == acl.check("nope", "db.query") == False  # noqa: E712

    # acl_system.check.property.pure
    def test_acl_system_check_property_pure_no_self_mutation(self) -> None:
        """acl_system.check.property.pure — pure:false BUT must not mutate rule list / default via public query."""
        acl = ACL(
            rules=[
                ACLRule(callers=["api.*"], targets=["db.*"], effect="allow"),
                ACLRule(callers=["*"], targets=["*"], effect="deny"),
            ],
            default_effect="deny",
        )
        before_allow = acl.check("api.gateway", "db.read")
        before_deny = acl.check("evil", "secret")
        acl.check("api.gateway", "db.read")
        acl.check("evil", "secret")
        # Decisions are stable across repeated calls — no self-mutation visible.
        assert acl.check("api.gateway", "db.read") == before_allow
        assert acl.check("evil", "secret") == before_deny

    # acl_system.check.side_effect.4.evaluate_first_match_wins
    def test_acl_system_check_side_effect_4_first_match_wins(self) -> None:
        """acl_system.check.side_effect.4.evaluate_first_match_wins — first matching rule decides."""
        acl = ACL(
            rules=[
                ACLRule(callers=["*"], targets=["*"], effect="allow"),
                ACLRule(callers=["*"], targets=["*"], effect="deny"),
            ],
            default_effect="deny",
        )
        # First rule is allow; despite later deny rule, first-match-wins => allow.
        assert acl.check("anyone", "anything") is True

    # acl_system.check.side_effect.5.emit_audit_event
    def test_acl_system_check_side_effect_5_emit_audit_event(self) -> None:
        """acl_system.check.side_effect.5.emit_audit_event — audit event emitted carrying decision."""
        captured: list[object] = []
        acl = ACL(
            rules=[ACLRule(callers=["api.*"], targets=["db.*"], effect="allow")],
            default_effect="deny",
            audit_logger=captured.append,
        )
        acl.check("api.gateway", "db.read")
        assert len(captured) == 1
        entry = captured[0]
        assert entry.decision == "allow"
        assert entry.caller_id == "api.gateway"
        assert entry.target_id == "db.read"

    # acl_system.check.input.caller_id.none_maps_to_external
    def test_acl_system_check_input_caller_id_none_maps_external(self) -> None:
        """acl_system.check.input.caller_id.none_maps_to_external — None caller => @external."""
        acl = ACL(
            rules=[ACLRule(callers=["@external"], targets=["public.*"], effect="allow")],
            default_effect="deny",
        )
        assert acl.check(caller_id=None, target_id="public.docs") is True
        # A real caller_id must NOT match @external.
        assert acl.check(caller_id="api.handler", target_id="public.docs") is False

    # acl_system.check.error.no_raise_returns_false
    def test_acl_system_check_error_no_raise_returns_false(self) -> None:
        """acl_system.check.error.no_raise_returns_false — deny is a False return, never a raise."""
        acl = ACL(rules=[], default_effect="deny")
        # No rules + default deny: must return False, must NOT raise.
        result = acl.check("api.x", "db.y")
        assert result is False


# ===========================================================================
# Contract: ACL.load
# ===========================================================================


class TestACLLoadContract:
    # acl_system.load.input.yaml_path.file_must_exist
    def test_acl_system_load_input_yaml_path_missing_file(self, tmp_path: Path) -> None:
        """acl_system.load.input.yaml_path.file_must_exist — missing file rejects with ConfigNotFoundError."""
        missing = str(tmp_path / "does_not_exist.yaml")
        with pytest.raises(ConfigNotFoundError) as exc_info:
            ACL.load(missing)
        assert exc_info.value.code == "CONFIG_NOT_FOUND"
        # reject_with carries config_path=yaml_path
        assert exc_info.value.details.get("config_path") == missing

    # acl_system.load.error.CONFIG_NOT_FOUND
    def test_acl_system_load_error_config_not_found(self, tmp_path: Path) -> None:
        """acl_system.load.error.CONFIG_NOT_FOUND — nonexistent path => ConfigNotFoundError + code."""
        missing = str(tmp_path / "nope.yaml")
        with pytest.raises(ConfigNotFoundError) as exc_info:
            ACL.load(missing)
        assert exc_info.value.code == "CONFIG_NOT_FOUND"

    # acl_system.load.error.ACL_RULE_ERROR.not_a_mapping
    def test_acl_system_load_error_acl_rule_error_not_mapping(self, tmp_path: Path) -> None:
        """acl_system.load.error.ACL_RULE_ERROR.not_a_mapping — top-level non-mapping => ACLRuleError."""
        path = _write_yaml(tmp_path, "- just\n- a\n- list\n")
        with pytest.raises(ACLRuleError) as exc_info:
            ACL.load(path)
        assert exc_info.value.code == "ACL_RULE_ERROR"

    # acl_system.load.error.ACL_RULE_ERROR.rules_key_absent
    def test_acl_system_load_error_acl_rule_error_rules_absent(self, tmp_path: Path) -> None:
        """acl_system.load.error.ACL_RULE_ERROR.rules_key_absent — missing 'rules' => ACLRuleError."""
        path = _write_yaml(tmp_path, 'version: "1.0"\ndefault_effect: deny\n')
        with pytest.raises(ACLRuleError) as exc_info:
            ACL.load(path)
        assert exc_info.value.code == "ACL_RULE_ERROR"

    # acl_system.load.error.ACL_RULE_ERROR.rules_not_list
    def test_acl_system_load_error_acl_rule_error_rules_not_list(self, tmp_path: Path) -> None:
        """acl_system.load.error.ACL_RULE_ERROR.rules_not_list — 'rules' non-list => ACLRuleError."""
        path = _write_yaml(tmp_path, "rules:\n  foo: bar\n")
        with pytest.raises(ACLRuleError) as exc_info:
            ACL.load(path)
        assert exc_info.value.code == "ACL_RULE_ERROR"

    # acl_system.load.error.ACL_RULE_ERROR.rule_missing_required_key
    def test_acl_system_load_error_acl_rule_error_missing_key(self, tmp_path: Path) -> None:
        """acl_system.load.error.ACL_RULE_ERROR.rule_missing_required_key — rule missing 'effect'."""
        path = _write_yaml(
            tmp_path,
            'rules:\n  - callers: ["a.*"]\n    targets: ["b.*"]\n',
        )
        with pytest.raises(ACLRuleError) as exc_info:
            ACL.load(path)
        assert exc_info.value.code == "ACL_RULE_ERROR"

    # acl_system.load.error.ACL_RULE_ERROR.invalid_effect
    def test_acl_system_load_error_acl_rule_error_invalid_effect(self, tmp_path: Path) -> None:
        """acl_system.load.error.ACL_RULE_ERROR.invalid_effect — effect not allow/deny => ACLRuleError."""
        path = _write_yaml(
            tmp_path,
            'rules:\n  - callers: ["a.*"]\n    targets: ["b.*"]\n    effect: maybe\n',
        )
        with pytest.raises(ACLRuleError) as exc_info:
            ACL.load(path)
        assert exc_info.value.code == "ACL_RULE_ERROR"

    # acl_system.load.error.ACL_RULE_ERROR.callers_not_list
    def test_acl_system_load_error_acl_rule_error_callers_not_list(self, tmp_path: Path) -> None:
        """acl_system.load.error.ACL_RULE_ERROR.callers_not_list — callers non-list => ACLRuleError."""
        path = _write_yaml(
            tmp_path,
            'rules:\n  - callers: "a.*"\n    targets: ["b.*"]\n    effect: allow\n',
        )
        with pytest.raises(ACLRuleError) as exc_info:
            ACL.load(path)
        assert exc_info.value.code == "ACL_RULE_ERROR"

    # acl_system.load.side_effect.4.set_yaml_path
    def test_acl_system_load_side_effect_4_sets_yaml_path(self, tmp_path: Path) -> None:
        """acl_system.load.side_effect.4.set_yaml_path — returned instance has _yaml_path set (enables reload)."""
        path = _write_yaml(tmp_path, _VALID_YAML)
        acl = ACL.load(path)
        assert acl._yaml_path == path
        # Observable via reload() not raising "not loaded from YAML".
        acl.reload()  # should succeed, proving the path is wired

    # acl_system.load.postcondition.default_effect_deny
    def test_acl_system_load_postcondition_default_effect_deny(self, tmp_path: Path) -> None:
        """acl_system.load.postcondition.default_effect_deny — absent default_effect => deny."""
        path = _write_yaml(
            tmp_path,
            'rules:\n  - callers: ["a.*"]\n    targets: ["b.*"]\n    effect: allow\n',
        )
        acl = ACL.load(path)
        # No rule matches "x" -> falls to default; must be deny.
        assert acl.check("x", "y") is False

    # acl_system.load.postcondition.rules_order_preserved
    def test_acl_system_load_postcondition_rules_order_preserved(self, tmp_path: Path) -> None:
        """acl_system.load.postcondition.rules_order_preserved — rules keep YAML order (first-match-wins)."""
        path = _write_yaml(
            tmp_path,
            """\
            default_effect: deny
            rules:
              - callers: ["*"]
                targets: ["*"]
                effect: allow
              - callers: ["*"]
                targets: ["*"]
                effect: deny
            """,
        )
        acl = ACL.load(path)
        # If order preserved, the first allow wins.
        assert acl.check("anyone", "anything") is True

    # acl_system.load.property.async
    def test_acl_system_load_property_async_is_sync(self, tmp_path: Path) -> None:
        """acl_system.load.property.async — load() is declared async: false."""
        path = _write_yaml(tmp_path, _VALID_YAML)
        result = ACL.load(path)
        assert not asyncio.iscoroutine(result)
        assert isinstance(result, ACL)

    # acl_system.load.property.idempotent
    def test_acl_system_load_property_idempotent(self, tmp_path: Path) -> None:
        """acl_system.load.property.idempotent — same file content => equivalent instances."""
        path = _write_yaml(tmp_path, _VALID_YAML)
        a = ACL.load(path)
        b = ACL.load(path)
        # Equivalent observable behavior.
        assert a.check("api.x", "db.y") == b.check("api.x", "db.y") == True  # noqa: E712
        assert a.check("x", "y") == b.check("x", "y") == False  # noqa: E712

    # acl_system.load.property.thread_safe
    def test_acl_system_load_property_thread_safe(self, tmp_path: Path) -> None:
        """acl_system.load.property.thread_safe — concurrent loads create independent instances."""
        path = _write_yaml(tmp_path, _VALID_YAML)

        async def run() -> list[ACL]:
            return await asyncio.gather(*[asyncio.to_thread(ACL.load, path) for _ in range(8)])

        instances = asyncio.run(run())
        assert len(instances) == 8
        assert all(isinstance(i, ACL) for i in instances)
        # Distinct objects, no shared mutable state.
        assert len({id(i) for i in instances}) == 8
        assert all(i.check("api.x", "db.y") is True for i in instances)


# ===========================================================================
# Contract: ACL.add_rule
# ===========================================================================


class TestACLAddRuleContract:
    # acl_system.add_rule.side_effect.2.insert_at_index_0
    def test_acl_system_add_rule_side_effect_2_insert_front(self) -> None:
        """acl_system.add_rule.side_effect.2.insert_at_index_0 — new rule evaluated before existing ones."""
        acl = ACL(
            rules=[ACLRule(callers=["*"], targets=["*"], effect="deny")],
            default_effect="deny",
        )
        # Without the new rule, deny.
        assert acl.check("admin.root", "anything") is False
        acl.add_rule(ACLRule(callers=["admin.*"], targets=["*"], effect="allow"))
        # New rule sits at index 0 => evaluated first => allow.
        assert acl.check("admin.root", "anything") is True

    # acl_system.add_rule.postcondition.shifts_prior_rules
    def test_acl_system_add_rule_postcondition_shifts_prior(self) -> None:
        """acl_system.add_rule.postcondition.shifts_prior_rules — prior rules shift up; new rule first."""
        acl = ACL(default_effect="deny")
        acl.add_rule(ACLRule(callers=["*"], targets=["*"], effect="deny"))
        acl.add_rule(ACLRule(callers=["*"], targets=["*"], effect="allow"))
        # Latest add_rule is at index 0 => allow wins via first-match.
        assert acl.check("x", "y") is True

    # acl_system.add_rule.property.idempotent
    def test_acl_system_add_rule_property_not_idempotent(self) -> None:
        """acl_system.add_rule.property.idempotent — declared false: two identical calls add two rules."""
        acl = ACL(default_effect="deny")
        rule = ACLRule(callers=["a.*"], targets=["b.*"], effect="allow")
        acl.add_rule(rule)
        acl.add_rule(rule)
        # Observe: two rules present. Removing once still leaves a matching rule.
        assert acl.remove_rule(["a.*"], ["b.*"]) is True
        assert acl.check("a.x", "b.y") is True  # second copy still present
        assert acl.remove_rule(["a.*"], ["b.*"]) is True
        assert acl.check("a.x", "b.y") is False  # now gone

    # acl_system.add_rule.property.async
    def test_acl_system_add_rule_property_async_is_sync(self) -> None:
        """acl_system.add_rule.property.async — add_rule() is declared async: false, returns None."""
        acl = ACL(default_effect="deny")
        result = acl.add_rule(ACLRule(callers=["a.*"], targets=["b.*"], effect="allow"))
        assert result is None
        assert acl.check("a.x", "b.y") is True

    # acl_system.add_rule.property.thread_safe
    def test_acl_system_add_rule_property_thread_safe(self) -> None:
        """acl_system.add_rule.property.thread_safe — N>=8 concurrent inserts, list not corrupted."""
        acl = ACL(default_effect="deny")

        async def run() -> None:
            def insert(i: int) -> None:
                acl.add_rule(ACLRule(callers=[f"svc.{i}"], targets=["t.*"], effect="allow"))

            await asyncio.gather(*[asyncio.to_thread(insert, i) for i in range(16)])

        asyncio.run(run())
        # All 16 rules landed; each is independently matchable.
        for i in range(16):
            assert acl.check(f"svc.{i}", "t.x") is True
        assert acl.check("svc.absent", "t.x") is False

    # acl_system.add_rule.error.value_error_kwargs_path
    def test_acl_system_add_rule_error_value_error_kwargs(self) -> None:
        """acl_system.add_rule.error.value_error_kwargs_path — Python kwargs path: rule None + callers None => ValueError."""
        acl = ACL(default_effect="deny")
        with pytest.raises(ValueError):
            acl.add_rule()  # no rule, no callers/targets


# ===========================================================================
# Contract: ACL.remove_rule
# ===========================================================================


class TestACLRemoveRuleContract:
    # acl_system.remove_rule.side_effect.2.find_first_match
    def test_acl_system_remove_rule_side_effect_2_first_match(self) -> None:
        """acl_system.remove_rule.side_effect.2.find_first_match — removes by exact callers/targets equality."""
        acl = ACL(
            rules=[
                ACLRule(callers=["a.*"], targets=["b.*"], effect="allow"),
                ACLRule(callers=["c.*"], targets=["d.*"], effect="allow"),
            ],
            default_effect="deny",
        )
        assert acl.remove_rule(["a.*"], ["b.*"]) is True
        # Removed rule no longer effective; sibling rule intact.
        assert acl.check("a.x", "b.y") is False
        assert acl.check("c.x", "d.y") is True

    # acl_system.remove_rule.return.true_when_found
    def test_acl_system_remove_rule_return_true_when_found(self) -> None:
        """acl_system.remove_rule.return.true_when_found — returns True when a matching rule removed."""
        acl = ACL(
            rules=[ACLRule(callers=["a.*"], targets=["b.*"], effect="allow")],
            default_effect="deny",
        )
        assert acl.remove_rule(["a.*"], ["b.*"]) is True

    # acl_system.remove_rule.return.false_when_absent
    def test_acl_system_remove_rule_return_false_when_absent(self) -> None:
        """acl_system.remove_rule.return.false_when_absent — returns False when no rule matches."""
        acl = ACL(
            rules=[ACLRule(callers=["a.*"], targets=["b.*"], effect="allow")],
            default_effect="deny",
        )
        assert acl.remove_rule(["nope.*"], ["nope.*"]) is False

    # acl_system.remove_rule.postcondition.at_most_one_removed
    def test_acl_system_remove_rule_postcondition_at_most_one(self) -> None:
        """acl_system.remove_rule.postcondition.at_most_one_removed — only first match removed per call."""
        rule = ACLRule(callers=["a.*"], targets=["b.*"], effect="allow")
        acl = ACL(rules=[rule, rule], default_effect="deny")
        assert acl.remove_rule(["a.*"], ["b.*"]) is True
        # One duplicate remains.
        assert acl.check("a.x", "b.y") is True
        assert acl.remove_rule(["a.*"], ["b.*"]) is True
        assert acl.check("a.x", "b.y") is False

    # acl_system.remove_rule.property.idempotent
    def test_acl_system_remove_rule_property_not_idempotent(self) -> None:
        """acl_system.remove_rule.property.idempotent — declared false: first True, second False."""
        acl = ACL(
            rules=[ACLRule(callers=["a.*"], targets=["b.*"], effect="allow")],
            default_effect="deny",
        )
        first = acl.remove_rule(["a.*"], ["b.*"])
        second = acl.remove_rule(["a.*"], ["b.*"])
        assert first is True
        assert second is False

    # acl_system.remove_rule.property.async
    def test_acl_system_remove_rule_property_async_is_sync(self) -> None:
        """acl_system.remove_rule.property.async — remove_rule() is declared async: false."""
        acl = ACL(
            rules=[ACLRule(callers=["a.*"], targets=["b.*"], effect="allow")],
            default_effect="deny",
        )
        result = acl.remove_rule(["a.*"], ["b.*"])
        assert not asyncio.iscoroutine(result)
        assert result is True

    # acl_system.remove_rule.property.thread_safe
    def test_acl_system_remove_rule_property_thread_safe(self) -> None:
        """acl_system.remove_rule.property.thread_safe — N>=8 concurrent removals, no corruption."""
        rules = [ACLRule(callers=[f"svc.{i}"], targets=["t.*"], effect="allow") for i in range(16)]
        acl = ACL(rules=list(rules), default_effect="deny")

        async def run() -> list[bool]:
            def rm(i: int) -> bool:
                return acl.remove_rule([f"svc.{i}"], ["t.*"])

            return await asyncio.gather(*[asyncio.to_thread(rm, i) for i in range(16)])

        results = asyncio.run(run())
        assert all(r is True for r in results)
        # Every rule removed => all deny now.
        for i in range(16):
            assert acl.check(f"svc.{i}", "t.x") is False


# ===========================================================================
# Contract: ACL.reload
# ===========================================================================


class TestACLReloadContract:
    # acl_system.reload.precondition.requires_yaml_path
    def test_acl_system_reload_precondition_no_yaml_path(self) -> None:
        """acl_system.reload.precondition.requires_yaml_path — reload on non-loaded ACL => ACLRuleError."""
        acl = ACL(default_effect="deny")  # not created via load()
        with pytest.raises(ACLRuleError) as exc_info:
            acl.reload()
        assert exc_info.value.code == "ACL_RULE_ERROR"

    # acl_system.reload.error.ACL_RULE_ERROR.not_loaded_from_yaml
    def test_acl_system_reload_error_acl_rule_error_not_loaded(self) -> None:
        """acl_system.reload.error.ACL_RULE_ERROR.not_loaded_from_yaml — no stored path => ACLRuleError + code."""
        acl = ACL(rules=[ACLRule(callers=["*"], targets=["*"], effect="allow")])
        with pytest.raises(ACLRuleError) as exc_info:
            acl.reload()
        assert exc_info.value.code == "ACL_RULE_ERROR"

    # acl_system.reload.error.CONFIG_NOT_FOUND.file_removed
    def test_acl_system_reload_error_config_not_found(self, tmp_path: Path) -> None:
        """acl_system.reload.error.CONFIG_NOT_FOUND.file_removed — file deleted after load => ConfigNotFoundError."""
        path = _write_yaml(tmp_path, _VALID_YAML)
        acl = ACL.load(path)
        Path(path).unlink()
        with pytest.raises(ConfigNotFoundError) as exc_info:
            acl.reload()
        assert exc_info.value.code == "CONFIG_NOT_FOUND"

    # acl_system.reload.postcondition.rules_reflect_file
    def test_acl_system_reload_postcondition_rules_reflect_file(self, tmp_path: Path) -> None:
        """acl_system.reload.postcondition.rules_reflect_file — reload re-reads YAML and updates rules."""
        path = _write_yaml(
            tmp_path,
            'default_effect: deny\nrules:\n  - callers: ["a.*"]\n    targets: ["b.*"]\n    effect: allow\n',
        )
        acl = ACL.load(path)
        assert acl.check("a.x", "b.y") is True
        # Rewrite the file with a different ruleset.
        _write_yaml(
            tmp_path,
            'default_effect: deny\nrules:\n  - callers: ["c.*"]\n    targets: ["d.*"]\n    effect: allow\n',
        )
        acl.reload()
        assert acl.check("a.x", "b.y") is False  # old rule gone
        assert acl.check("c.x", "d.y") is True  # new rule active

    # acl_system.reload.postcondition.discards_runtime_mutations
    def test_acl_system_reload_postcondition_discards_mutations(self, tmp_path: Path) -> None:
        """acl_system.reload.postcondition.discards_runtime_mutations — add_rule before reload is discarded."""
        path = _write_yaml(tmp_path, _VALID_YAML)
        acl = ACL.load(path)
        acl.add_rule(ACLRule(callers=["runtime.*"], targets=["*"], effect="allow"))
        assert acl.check("runtime.x", "anything") is True
        acl.reload()
        # Runtime-added rule is discarded by reload (file content wins).
        assert acl.check("runtime.x", "anything") is False

    # acl_system.reload.property.async
    def test_acl_system_reload_property_async_is_sync(self, tmp_path: Path) -> None:
        """acl_system.reload.property.async — reload() is declared async: false, returns None."""
        path = _write_yaml(tmp_path, _VALID_YAML)
        acl = ACL.load(path)
        result = acl.reload()
        assert result is None

    # acl_system.reload.property.idempotent
    def test_acl_system_reload_property_idempotent(self, tmp_path: Path) -> None:
        """acl_system.reload.property.idempotent — same file content => same rule list across reloads."""
        path = _write_yaml(tmp_path, _VALID_YAML)
        acl = ACL.load(path)
        acl.reload()
        first_allow = acl.check("api.x", "db.y")
        first_deny = acl.check("x", "y")
        acl.reload()
        assert acl.check("api.x", "db.y") == first_allow == True  # noqa: E712
        assert acl.check("x", "y") == first_deny == False  # noqa: E712

    # acl_system.reload.property.thread_safe
    def test_acl_system_reload_property_thread_safe(self, tmp_path: Path) -> None:
        """acl_system.reload.property.thread_safe — concurrent reload + check, no corruption."""
        path = _write_yaml(tmp_path, _VALID_YAML)
        acl = ACL.load(path)

        async def run() -> list[object]:
            def reload_op(_: int) -> str:
                acl.reload()
                return "ok"

            def check_op(_: int) -> bool:
                return acl.check("api.x", "db.y")

            tasks = []
            for i in range(8):
                tasks.append(asyncio.to_thread(reload_op, i))
                tasks.append(asyncio.to_thread(check_op, i))
            return await asyncio.gather(*tasks)

        results = asyncio.run(run())
        assert len(results) == 16
        # Final state consistent: the file's allow rule still applies.
        assert acl.check("api.x", "db.y") is True
