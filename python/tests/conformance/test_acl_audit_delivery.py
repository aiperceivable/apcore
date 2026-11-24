"""Drive `acl_audit_delivery.json` — §6.3.2 (#118 D-66).

Every case loads a real ACL file and drives real ``check()`` calls. Reading the
parsed block back off a config object would prove the parser works, which was
never the question: what had no contract at all was **delivery**.
"""

from __future__ import annotations

import logging
import tempfile
from pathlib import Path
from typing import Any

import pytest
import yaml

from apcore.acl import ACL, AUDIT_EVENT_NAME, AuditEntry
from apcore.errors import ConfigError

from .canonical_fixtures import case_ids, load_fixture

FIXTURE = load_fixture("acl_audit_delivery.json")
CASES: dict[str, dict[str, Any]] = {tc["id"]: tc for tc in FIXTURE["test_cases"]}

_LEVEL_NAMES = {
    logging.DEBUG: "debug",
    logging.INFO: "info",
    logging.WARNING: "warn",
    logging.ERROR: "error",
}


def _write(doc: dict[str, Any]) -> str:
    path = Path(tempfile.mkdtemp()) / "acl.yaml"
    path.write_text(yaml.safe_dump(doc, default_flow_style=False), encoding="utf-8")
    return str(path)


@pytest.mark.parametrize("case_id", list(CASES))
def test_acl_audit_delivery(case_id: str, caplog: pytest.LogCaptureFixture) -> None:
    case = CASES[case_id]
    inp, expected = case["input"], case["expected"]
    collected: list[AuditEntry] = []

    def failing(_entry: AuditEntry) -> None:
        raise RuntimeError("the audit sink is down")

    callback = {
        "none": None,
        "collecting": collected.append,
        "failing": failing,
    }[inp["callback"]]

    path = _write(dict(inp["acl_file"]))
    caplog.clear()

    with caplog.at_level(logging.DEBUG):
        if expected.get("loads") is False:
            with pytest.raises(ConfigError) as excinfo:
                ACL.load(path, audit_logger=callback)
            assert excinfo.value.code == expected["error_code"]
            assert expected["error_message_contains"] in str(excinfo.value)
            return

        acl = ACL.load(path, audit_logger=callback)
        decisions = ["allow" if acl.check(c["caller_id"], c["target_id"]) else "deny" for c in inp["checks"]]

    if "decisions" in expected:
        assert decisions == expected["decisions"]
    if "callback_decisions" in expected:
        assert [e.decision for e in collected] == expected["callback_decisions"]

    audit = [r for r in caplog.records if r.name == AUDIT_EVENT_NAME]
    if "default_sink_records" in expected:
        assert len(audit) == expected["default_sink_records"]
    if "default_sink_field_names" in expected:
        assert set(getattr(audit[0], "apcore_audit")) == set(expected["default_sink_field_names"])
    if "default_sink_level" in expected:
        assert _LEVEL_NAMES[audit[0].levelno] == expected["default_sink_level"]
    if "default_sink_decisions" in expected:
        payloads = [getattr(r, "apcore_audit") for r in audit]
        assert [p["decision"] for p in payloads] == expected["default_sink_decisions"]

    messages = [r.getMessage() for r in caplog.records if r.name != AUDIT_EVENT_NAME]
    if "load_warning_contains" in expected:
        assert any(expected["load_warning_contains"] in m for m in messages)
    if "load_warning_absent" in expected:
        assert not any(expected["load_warning_absent"] in m for m in messages)
    if "override_warning_names" in expected:
        hits = [m for m in messages if "does not apply" in m]
        assert len(hits) == 1
        for field in expected["override_warning_names"]:
            assert field in hits[0]
    if "delivery_failure_reports" in expected:
        hits = [m for m in messages if "audit delivery failed" in m]
        assert len(hits) == expected["delivery_failure_reports"]


def test_every_fixture_case_is_driven() -> None:
    assert set(case_ids("acl_audit_delivery.json")) == set(CASES)
