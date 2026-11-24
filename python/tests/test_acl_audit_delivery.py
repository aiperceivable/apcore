"""PROTOCOL_SPEC §6.3.2 — ACL audit delivery (apcore#118, decision D-66).

§6.3.1 has always specified the *record*. Nothing specified **delivery**:
``ACL(audit_logger=…)`` was the whole surface, with no default sink, no
statement of what happens when delivery fails, and no meaning for the ``audit:``
block's three settings — which were declared in two places and read in neither.

The sharpest consequence was measured, not inferred: a raising audit callback
propagated out of ``check()`` and turned an **allowed** call into an error. That
is the one behaviour change here, and it is a fix — auditing is a side channel
and does not hold a veto over access.
"""

from __future__ import annotations

import logging
from pathlib import Path
from typing import Any

import pytest
import yaml

from apcore.acl import ACL, AUDIT_EVENT_NAME, ACLRule, AuditEntry
from apcore.errors import ConfigError

_RULES = [{"callers": ["api.*"], "targets": ["executor.*"], "effect": "allow"}]


def _write(tmp_path: Path, audit: dict[str, Any] | None = None, **extra: Any) -> str:
    doc: dict[str, Any] = {"version": "1.0.0", "rules": _RULES, **extra}
    if audit is not None:
        doc["audit"] = audit
    path = tmp_path / "acl.yaml"
    path.write_text(yaml.safe_dump(doc), encoding="utf-8")
    return str(path)


def _allow_rule() -> ACLRule:
    return ACLRule(callers=["api.*"], targets=["executor.*"], effect="allow")


def _audit_records(caplog: pytest.LogCaptureFixture) -> list[logging.LogRecord]:
    return [r for r in caplog.records if r.name == AUDIT_EVENT_NAME]


# ---------------------------------------------------------------------------
# Requirement 2 — declaration activates the default sink, not the default value
# ---------------------------------------------------------------------------


def test_no_audit_block_produces_no_audit_output(tmp_path: Path, caplog: pytest.LogCaptureFixture) -> None:
    """The compatibility boundary, and the reason declaration is the switch.

    `enabled` defaults to True, so a merged-view reading would switch a log
    record per ACL check on for every ACL file in existence — a behaviour change
    measured in volume, on projects that asked for nothing.
    """
    acl = ACL.load(_write(tmp_path))
    with caplog.at_level(logging.DEBUG):
        acl.check("api.x", "executor.y")
    assert _audit_records(caplog) == []


def test_a_declared_block_activates_the_default_sink(tmp_path: Path, caplog: pytest.LogCaptureFixture) -> None:
    acl = ACL.load(_write(tmp_path, audit={"enabled": True}))
    with caplog.at_level(logging.DEBUG):
        acl.check("api.x", "executor.y")
    assert len(_audit_records(caplog)) == 1


def test_an_empty_block_is_declared_with_every_setting_at_its_default(
    tmp_path: Path, caplog: pytest.LogCaptureFixture
) -> None:
    """`audit:` with nothing under it parses to None, and the operator still
    wrote the block. Requirement 2 makes DECLARATION the switch, so presence —
    not truthiness — is what activates the default sink."""
    path = tmp_path / "acl.yaml"
    path.write_text(
        "version: '1.0.0'\nrules:\n  - callers: ['api.*']\n    " "targets: ['executor.*']\n    effect: allow\naudit:\n",
        encoding="utf-8",
    )
    acl = ACL.load(str(path))
    with caplog.at_level(logging.DEBUG):
        acl.check("api.x", "executor.y")
    assert len(_audit_records(caplog)) == 1


def test_a_declared_block_with_enabled_false_is_silent(tmp_path: Path, caplog: pytest.LogCaptureFixture) -> None:
    acl = ACL.load(_write(tmp_path, audit={"enabled": False}))
    with caplog.at_level(logging.DEBUG):
        acl.check("api.x", "executor.y")
    assert _audit_records(caplog) == []


def test_the_default_sink_carries_all_thirteen_fields_as_structured_data(
    tmp_path: Path, caplog: pytest.LogCaptureFixture
) -> None:
    """§6.3.2 requirement 2 — the half that keeps three SDKs consumable.

    Under their `snake_case` wire names, and not interpolated into the message:
    without that, one specification yields a Python `logging` call, a TypeScript
    `console` line and a Rust `tracing` event that no downstream consumer can
    read all three of.
    """
    acl = ACL.load(_write(tmp_path, audit={"enabled": True}))
    with caplog.at_level(logging.DEBUG):
        acl.check("api.x", "executor.y")

    record = _audit_records(caplog)[0]
    assert record.getMessage() == AUDIT_EVENT_NAME
    payload = getattr(record, "apcore_audit")
    assert set(payload) == {
        "timestamp",
        "caller_id",
        "target_id",
        "decision",
        "reason",
        "matched_rule",
        "matched_rule_index",
        "identity_type",
        "roles",
        "call_depth",
        "trace_id",
        "handler_error",
        "approval_required",
    }
    assert payload["caller_id"] == "api.x"
    assert payload["decision"] == "allow"


@pytest.mark.parametrize(
    ("level", "expected"),
    [
        ("trace", logging.DEBUG),
        ("debug", logging.DEBUG),
        ("info", logging.INFO),
        ("warn", logging.WARNING),
        ("error", logging.ERROR),
    ],
)
def test_log_level_sets_the_default_sinks_level(
    tmp_path: Path, caplog: pytest.LogCaptureFixture, level: str, expected: int
) -> None:
    acl = ACL.load(_write(tmp_path, audit={"enabled": True, "log_level": level}))
    with caplog.at_level(logging.DEBUG):
        acl.check("api.x", "executor.y")
    assert _audit_records(caplog)[0].levelno == expected


# ---------------------------------------------------------------------------
# Requirement 1 — one effective sink, never two
# ---------------------------------------------------------------------------


def test_a_callback_receives_every_entry_and_the_block_does_not_apply(
    tmp_path: Path, caplog: pytest.LogCaptureFixture
) -> None:
    """The API-beats-configuration rule, in the direction that matters.

    `include_denied: false` in a file must not silently truncate a compliance
    sink a developer installed deliberately.
    """
    seen: list[AuditEntry] = []
    acl = ACL.load(
        _write(tmp_path, audit={"enabled": False, "include_denied": False}),
        audit_logger=seen.append,
    )

    with caplog.at_level(logging.DEBUG):
        acl.check("api.x", "executor.y")  # allow
        acl.check("worker.x", "executor.y")  # deny
    assert [e.decision for e in seen] == ["allow", "deny"]
    assert _audit_records(caplog) == []


def test_an_overridden_block_names_every_field_that_does_not_apply(
    caplog: pytest.LogCaptureFixture,
) -> None:
    """Not only the most visible one — an operator told about one of three
    settings has been told the smaller half of what happened."""
    with caplog.at_level(logging.WARNING):
        ACL(
            rules=[_allow_rule()],
            audit_logger=lambda entry: None,
            audit_config={"enabled": True, "include_denied": False, "log_level": "error"},
        )
    hits = [r.getMessage() for r in caplog.records if "does not apply" in r.getMessage()]
    assert len(hits) == 1
    for field in ("audit.enabled", "audit.include_denied", "audit.log_level"):
        assert field in hits[0]


def test_no_override_notice_without_a_block(caplog: pytest.LogCaptureFixture) -> None:
    with caplog.at_level(logging.WARNING):
        ACL(rules=[_allow_rule()], audit_logger=lambda entry: None)
    assert [r for r in caplog.records if "does not apply" in r.getMessage()] == []


# ---------------------------------------------------------------------------
# Requirement 3 — delivery never changes the access decision
# ---------------------------------------------------------------------------


def _raising(_entry: AuditEntry) -> None:
    raise RuntimeError("the audit sink is down")


@pytest.mark.parametrize(("caller", "expected"), [("api.x", True), ("worker.x", False)])
def test_a_raising_callback_does_not_change_the_decision(
    caplog: pytest.LogCaptureFixture, caller: str, expected: bool
) -> None:
    """Measured before spec v1.45.0: this raised, and an ALLOWED call became an
    error. Both decisions are driven, because the deny path is a different
    branch of `_finalize_check`."""
    acl = ACL(rules=[_allow_rule()], audit_logger=_raising)
    with caplog.at_level(logging.WARNING):
        assert acl.check(caller, "executor.y") is expected


@pytest.mark.asyncio
@pytest.mark.parametrize(("caller", "expected"), [("api.x", True), ("worker.x", False)])
async def test_a_raising_callback_does_not_change_the_async_decision(
    caplog: pytest.LogCaptureFixture, caller: str, expected: bool
) -> None:
    acl = ACL(rules=[_allow_rule()], audit_logger=_raising)
    with caplog.at_level(logging.WARNING):
        assert await acl.async_check(caller, "executor.y") is expected


def test_a_failing_sink_is_reported_once(caplog: pytest.LogCaptureFixture) -> None:
    """§6.3.2 requirement 5. A sink that is down otherwise produces one
    diagnostic per check — the flood §9.2.2 rejects."""
    acl = ACL(rules=[_allow_rule()], audit_logger=_raising)
    with caplog.at_level(logging.WARNING):
        for _ in range(5):
            acl.check("api.x", "executor.y")
    hits = [r for r in caplog.records if "audit delivery failed" in r.getMessage()]
    assert len(hits) == 1


# ---------------------------------------------------------------------------
# Requirement 4 — the callback must be synchronous
# ---------------------------------------------------------------------------


def test_an_async_callback_is_an_invalid_delivery(caplog: pytest.LogCaptureFixture) -> None:
    """Its failure would surface after the decision has been returned, outside
    the containment requirement 3 promises."""

    async def later(_entry: AuditEntry) -> None:  # pragma: no cover - never awaited
        raise RuntimeError("too late to matter")

    acl = ACL(rules=[_allow_rule()], audit_logger=later)  # type: ignore[arg-type]
    with caplog.at_level(logging.WARNING):
        assert acl.check("api.x", "executor.y") is True
        acl.check("api.x", "executor.y")
    hits = [r for r in caplog.records if "returned an awaitable" in r.getMessage()]
    assert len(hits) == 1, [r.getMessage() for r in caplog.records]
    assert "§6.3.2" in hits[0].getMessage()


# ---------------------------------------------------------------------------
# Requirement 6 — include_denied
# ---------------------------------------------------------------------------


def test_include_denied_false_withholds_denials_from_the_default_sink(
    tmp_path: Path, caplog: pytest.LogCaptureFixture
) -> None:
    acl = ACL.load(_write(tmp_path, audit={"enabled": True, "include_denied": False}))
    with caplog.at_level(logging.DEBUG):
        acl.check("api.x", "executor.y")  # allow — delivered
        acl.check("worker.x", "executor.y")  # deny  — withheld
    payloads = [getattr(r, "apcore_audit") for r in _audit_records(caplog)]
    assert [p["decision"] for p in payloads] == ["allow"]


def test_include_denied_false_warns_once_per_load(tmp_path: Path, caplog: pytest.LogCaptureFixture) -> None:
    """Security friction, not a refusal: it withholds the security-relevant
    half, so the operator is told once rather than stopped."""
    with caplog.at_level(logging.WARNING):
        ACL.load(_write(tmp_path, audit={"include_denied": False}))
    hits = [r for r in caplog.records if "include_denied" in r.getMessage()]
    assert len(hits) == 1
    assert "DENIED" in hits[0].getMessage()


def test_include_denied_true_is_silent(tmp_path: Path, caplog: pytest.LogCaptureFixture) -> None:
    with caplog.at_level(logging.WARNING):
        ACL.load(_write(tmp_path, audit={"include_denied": True}))
    assert [r for r in caplog.records if "include_denied" in r.getMessage()] == []


# ---------------------------------------------------------------------------
# Requirement 7 — reload
# ---------------------------------------------------------------------------


def test_reload_refreshes_the_block_and_preserves_the_callback(
    tmp_path: Path, caplog: pytest.LogCaptureFixture
) -> None:
    """Before spec v1.45.0 `reload()` refreshed only rules and the default
    effect, so `audit:` was the one part of the document a reload missed."""
    path = _write(tmp_path, audit={"enabled": True, "log_level": "info"})
    acl = ACL.load(path)
    with caplog.at_level(logging.DEBUG):
        acl.check("api.x", "executor.y")
    assert _audit_records(caplog)[0].levelno == logging.INFO

    Path(path).write_text(
        yaml.safe_dump({"version": "1.0.0", "rules": _RULES, "audit": {"enabled": True, "log_level": "error"}}),
        encoding="utf-8",
    )
    acl.reload()
    caplog.clear()
    with caplog.at_level(logging.DEBUG):
        acl.check("api.x", "executor.y")
    assert _audit_records(caplog)[0].levelno == logging.ERROR


def test_reload_starts_a_new_failure_report_scope(tmp_path: Path, caplog: pytest.LogCaptureFixture) -> None:
    """§6.3.2 requirement 5's scoping: per ACL instance AND per effective sink
    configuration, so a new failure is never hidden behind an old one."""
    path = _write(tmp_path, audit={"enabled": True})
    acl = ACL.load(path, audit_logger=_raising)

    with caplog.at_level(logging.WARNING):
        acl.check("api.x", "executor.y")
        acl.check("api.x", "executor.y")
    assert len([r for r in caplog.records if "audit delivery failed" in r.getMessage()]) == 1

    caplog.clear()
    acl.reload()
    with caplog.at_level(logging.WARNING):
        acl.check("api.x", "executor.y")
    assert len([r for r in caplog.records if "audit delivery failed" in r.getMessage()]) == 1


# ---------------------------------------------------------------------------
# Requirement 8 — the block is validated, nothing else gets stricter
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "bad",
    [
        {"enabled": "yes"},
        {"log_level": "verbose"},
        {"include_denied": 1},
        {"enabled": True, "unknown_key": True},
        "not-a-mapping",
    ],
)
def test_a_malformed_audit_block_is_rejected_at_load(tmp_path: Path, bad: Any) -> None:
    with pytest.raises(ConfigError):
        ACL.load(_write(tmp_path, audit=bad))


def test_other_unknown_root_keys_are_still_ignored(tmp_path: Path, caplog: pytest.LogCaptureFixture) -> None:
    """Wiring one block is not unknown-key closure for ACL files — the same
    scoping §9.2.4.1 gave its own notice."""
    path = _write(
        tmp_path, audit={"enabled": True}, telemetry={"enabled": True}, x_vendor_note="kept for the deploy tooling"
    )
    with caplog.at_level(logging.WARNING):
        acl = ACL.load(path)
    assert len(acl.rules) == 1
    assert [r.getMessage() for r in caplog.records if "telemetry" in r.getMessage()] == []
