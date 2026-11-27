"""Read and validate filesystem-backed ACL configuration."""

from __future__ import annotations

import logging
import os
import stat
from pathlib import Path
from typing import Any

from apcore.acl import (
    APPROVAL_NOT_REQUIRED,
    ACLRule,
    _AUDIT_FIELDS,
    _AUDIT_LEVELS,
    _validate_approval,
    _validate_default_effect,
    _validate_effect,
    _validate_pattern_array,
)
from apcore.config import Config
from apcore.errors import ACLRuleError, ConfigError, ConfigNotFoundError

# Keep the ACL diagnostics on the same logger for file-backed and in-memory use.
_logger = logging.getLogger("apcore.acl")

_AUDIT_DEFAULTS: dict[str, Any] = {"enabled": True, "include_denied": True, "log_level": "info"}

#: The complete set of keys an ACL rule may carry (PROTOCOL_SPEC §6.1).
#: Closed on purpose: a key nothing evaluates is otherwise dropped in silence,
#: which widens an ``allow`` rule with no warning (#107). ``approval`` joined
#: the set in spec v1.28.0 (§6.1.6) — adding it was only safe *because* the set
#: was closed first, since an SDK that still dropped unknown keys would read a
#: ``deny``-with-``approval`` rule as a bare rule and act on half of it.
_RULE_KEYS: frozenset[str] = frozenset({"callers", "targets", "effect", "description", "conditions", "approval"})

#: Reserved in earlier revisions of §6.1 and evaluated by no implementation.
#: Rejected like any other unknown key, but named as reserved in the message:
#: an operator who wrote ``actions: ["describe"]`` meant to restrict the rule and
#: is better served by "not implemented" than by "unknown key".
_RESERVED_RULE_KEYS: frozenset[str] = frozenset({"id", "actions", "priority"})


def _reject_unknown_rule_keys(index: int, raw_rule: dict) -> None:
    """Raise ``ACLRuleError`` for any key outside :data:`_RULE_KEYS`."""
    unknown = sorted(set(raw_rule) - _RULE_KEYS)
    if not unknown:
        return
    reserved = [k for k in unknown if k in _RESERVED_RULE_KEYS]
    other = [k for k in unknown if k not in _RESERVED_RULE_KEYS]
    parts = []
    if reserved:
        names = ", ".join(repr(k) for k in reserved)
        parts.append(f"{names} reserved for a future specification version and evaluated by no implementation")
    if other:
        parts.append(f"{', '.join(repr(k) for k in other)} unrecognised")
    raise ACLRuleError(
        f"Rule {index} carries {'; '.join(parts)}. The rule key set is closed "
        f"({', '.join(sorted(_RULE_KEYS))}); a key nothing evaluates would be "
        f"dropped silently and leave the rule wider than written."
    )


def _parse_audit_block(data: dict[str, Any], yaml_path: str) -> dict[str, Any] | None:
    """Validate an ACL file's ``audit:`` block (§6.3.2 requirement 8).

    Returns ``None`` only when the document declares no ``audit`` key at all —
    the distinction requirement 2 turns on, because `enabled` defaults to
    ``True`` and reading the merged view would switch a log record per check on
    for every ACL file in existence.

    **Presence, not truthiness.** ``audit:`` with nothing under it parses to
    ``None``, and the operator still wrote the block: that is a declaration with
    every setting at its default, not an absence.

    Validates the SUBTREE only: types and unknown keys inside the block. Every
    other unrecognised root key in an ACL file keeps being ignored.
    """
    if "audit" not in data:
        return None
    raw = data["audit"]
    if raw is None:
        return dict(_AUDIT_DEFAULTS)
    if not isinstance(raw, dict):
        raise ConfigError(f"{yaml_path}: 'audit' must be a mapping (PROTOCOL_SPEC §6.3.2), got {type(raw).__name__}")

    unknown = sorted(set(raw) - set(_AUDIT_FIELDS))
    if unknown:
        raise ConfigError(
            f"{yaml_path}: unknown key(s) in the 'audit' block: {', '.join(unknown)}. "
            f"The block accepts exactly {', '.join(_AUDIT_FIELDS)} "
            f"($defs/AuditConfig in schemas/acl-config.schema.json)."
        )

    config = dict(_AUDIT_DEFAULTS)
    for key in ("enabled", "include_denied"):
        if key in raw:
            if not isinstance(raw[key], bool):
                raise ConfigError(f"{yaml_path}: 'audit.{key}' must be a boolean, got {raw[key]!r}")
            config[key] = raw[key]
    if "log_level" in raw:
        if raw["log_level"] not in _AUDIT_LEVELS:
            raise ConfigError(
                f"{yaml_path}: 'audit.log_level' must be one of {', '.join(_AUDIT_LEVELS)}, got {raw['log_level']!r}"
            )
        config["log_level"] = raw["log_level"]

    if not config["include_denied"]:
        # §6.3.2 requirement 6 — a notice, not a refusal. It withholds the
        # security-relevant half of the record, so an operator who wrote it
        # deliberately gets told once per load rather than stopped.
        _logger.warning(
            "%s sets audit.include_denied: false, so DENIED access attempts will not be "
            "recorded by the default audit sink (PROTOCOL_SPEC §6.3.2 requirement 6). "
            "Allowed calls are still recorded. Remove the entry to restore denials.",
            yaml_path,
        )
    return config


def read_acl_file(yaml_path: str) -> tuple[list[ACLRule], str, dict[str, Any] | None]:
    """Parse one ACL file before publishing its rules or audit settings."""
    if not os.path.isfile(yaml_path):
        raise ConfigNotFoundError(config_path=yaml_path)

    import yaml

    with open(yaml_path, encoding="utf-8") as f:
        try:
            data = yaml.safe_load(f)
        except yaml.YAMLError as e:
            raise ACLRuleError(f"Invalid YAML in {yaml_path}: {e}") from e

    if not isinstance(data, dict):
        raise ACLRuleError(f"ACL config must be a mapping, got {type(data).__name__}")

    audit_config = _parse_audit_block(data, yaml_path)

    # PROTOCOL_SPEC §6.2.1: `default_effect` is judged FIRST, before any
    # rule, at every door. It is not a rule and has no index, so the rule
    # ordering below never reaches it — and this used to be left entirely to
    # the `ACL` constructor at the bottom of this method, which runs after
    # every rule has been parsed and validated, so a file carrying a bad
    # `default_effect` AND a bad rule 0 was refused for the RULE here and
    # for `default_effect` through direct construction: one configuration,
    # two answers, from two doors of the same SDK.
    #
    # Placed above the `rules` checks and not merely above the loop:
    # "first" is ahead of the file-level checks on the `rules` COLLECTION
    # too, so a document both missing `rules` and carrying a bad
    # `default_effect` names the `default_effect`.
    default_effect: str = data.get("default_effect", "deny")
    _validate_default_effect(default_effect)

    if "rules" not in data:
        raise ACLRuleError("ACL config missing required 'rules' key")

    raw_rules = data["rules"]
    if not isinstance(raw_rules, list):
        raise ACLRuleError(f"'rules' must be a list, got {type(raw_rules).__name__}")

    rules: list[ACLRule] = []

    for i, raw_rule in enumerate(raw_rules):
        if not isinstance(raw_rule, dict):
            raise ACLRuleError(f"Rule {i} must be a mapping, got {type(raw_rule).__name__}")

        for key in ("callers", "targets", "effect"):
            if key not in raw_rule:
                raise ACLRuleError(f"Rule {i} missing required key '{key}'")

        # A missing key was already rejected above so an omission cannot
        # render a rule inert; an unknown key is the same hazard pointing
        # the other way, and was dropped in silence until #107.
        _reject_unknown_rule_keys(i, raw_rule)

        # PROTOCOL_SPEC §6.1.5. Checked here as well as in
        # ACLRule.__post_init__ so a file fault names the offending rule's
        # index, which the dataclass cannot know — the same split
        # _validate_approval already uses.
        effect = raw_rule["effect"]
        _validate_effect(effect, where=f"Rule {i}")

        # PROTOCOL_SPEC §6.1.6. Checked here as well as in ACLRule.__post_init__
        # so a file fault names the offending rule's index, which the
        # dataclass cannot know.
        approval = raw_rule.get("approval", APPROVAL_NOT_REQUIRED)
        _validate_approval(approval, effect, where=f"Rule {i}")

        # PROTOCOL_SPEC §6.2.1 (spec v1.31.0, #112). Checked here as well as
        # in ACLRule.__post_init__ so a file fault names the offending rule's
        # index, which the dataclass cannot know — the same split
        # _validate_effect and _validate_approval already use. `ACL.load`
        # rejected an OMITTED callers / targets and permitted an EMPTY one,
        # so a plain YAML file reached the fail-open; the arity closure is
        # what shuts that route.
        callers = raw_rule["callers"]
        if not isinstance(callers, list):
            raise ACLRuleError(f"Rule {i} 'callers' must be a list, got {type(callers).__name__}")
        _validate_pattern_array(callers, "callers", where=f"Rule {i}")

        targets = raw_rule["targets"]
        if not isinstance(targets, list):
            raise ACLRuleError(f"Rule {i} 'targets' must be a list, got {type(targets).__name__}")
        _validate_pattern_array(targets, "targets", where=f"Rule {i}")

        raw_conditions = raw_rule.get("conditions")
        if raw_conditions is not None and not isinstance(raw_conditions, dict):
            # A YAML file cannot reach §6.1.1 case 5 — the parser rejects it
            # here, as apcore-typescript's `_parseAclRule` does. Direct
            # construction and add_rule() still can, which is what the
            # runtime precheck (§6.1.4) is for.
            raise ACLRuleError(f"Rule {i} 'conditions' must be a mapping, got {type(raw_conditions).__name__}")

        rules.append(
            ACLRule(
                callers=callers,
                targets=targets,
                effect=effect,
                description=raw_rule.get("description", ""),
                conditions=raw_conditions,
                approval=approval,
            )
        )

    return rules, default_effect, audit_config


def find_acl_file(config: Config) -> str | None:
    """Resolve a configured file or the global ACL under its root directory."""
    root = config.get("acl.root", Config.get_default("acl.root"))
    if root is None:
        return None
    if root == "":
        root = Config.get_default("acl.root")

    path = Path(config.resolve_path(str(root)))
    mode = _discovery_mode(path)
    if mode is None:
        return None
    if stat.S_ISDIR(mode):
        path /= "global_acl.yaml"
        mode = _discovery_mode(path)
        if mode is None:
            return None
    if not stat.S_ISREG(mode):
        raise ConfigError(f"ACL path {str(path)!r} is not a regular file")
    return str(path)


def _discovery_mode(path: Path) -> int | None:
    try:
        mode = path.lstat().st_mode
    except FileNotFoundError:
        return None
    except OSError as error:
        raise ConfigError(f"Cannot inspect ACL path {str(path)!r}: {error}") from error
    if stat.S_ISLNK(mode):
        try:
            return path.stat().st_mode
        except OSError as error:
            raise ConfigError(f"Cannot resolve ACL path {str(path)!r}: {error}") from error
    return mode
