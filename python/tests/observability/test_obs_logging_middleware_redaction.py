"""The redaction contract at the surface an operator configures.

Asserts the emitted LOG LINE from ``ObsLoggingMiddleware``, not the helper the
log line happens to call. The distinction is the point of the file:
``RedactionConfig.redact`` / ``_redact_secrets_recursive`` can be correct and
their unit tests green while the documented wiring still writes secrets, because
two rule sets are applied to two parts of one record —

* the middleware redacts ``inputs`` / ``output`` with the caller's config,
  through the FLAT ``_apply_redaction_config``;
* ``ContextLogger._emit`` then redacts the whole ``extra`` recursively, with
  **the logger's own** config.

This SDK keeps those two in step by assigning ``self._logger._redaction_config``
in ``ObsLoggingMiddleware.__init__`` (Issue #43 §5). apcore-typescript did not,
and leaked `{"items": ["sk-…"]}` and `{"nested": {"key": "sk-…"}}` under exactly
the wiring ``docs/features/observability.md`` documents. These cases exist so
this SDK's arrangement is pinned at the surface where that broke, in all three.
"""

from __future__ import annotations

import io
import json
from typing import Any

import pytest

from apcore.context import Context
from apcore.observability.context_logger import (
    ContextLogger,
    ObsLoggingMiddleware,
    RedactionConfig,
)

SECRET = "sk-abcdef123456"


class Harness:
    def __init__(self, config: RedactionConfig | None) -> None:
        self.buf = io.StringIO()
        logger = ContextLogger(name="test", output=self.buf)
        self.mw = ObsLoggingMiddleware(logger=logger, redaction_config=config)

    def records(self) -> list[dict[str, Any]]:
        return [json.loads(line) for line in self.buf.getvalue().splitlines() if line.strip()]


@pytest.fixture
def payload() -> dict[str, Any]:
    return {"top": SECRET, "items": [SECRET], "nested": {"key": SECRET}}


def _captured(inputs: dict[str, Any] | None = None, output: dict[str, Any] | None = None) -> Context:
    """A Context carrying what the pipeline's capture point would have written.

    The middleware logs ``context.redacted_inputs`` / ``redacted_output`` and never
    the raw values it is handed (PROTOCOL_SPEC §10.6.1 requirement 5, D-131), so a
    direct drive has to put the payload where the pipeline puts it.
    """
    ctx = Context.create()
    ctx.redacted_inputs = inputs
    ctx.redacted_output = output
    return ctx


def _value_rule() -> RedactionConfig:
    return RedactionConfig(sensitive_keys=[], regex_patterns=[r"sk-[A-Za-z0-9]{6,}"])


def test_a_logged_input_is_redacted_at_every_position(payload: dict[str, Any]) -> None:
    h = Harness(_value_rule())
    h.mw.before("executor.x.y", payload, _captured(inputs=payload))

    inputs = h.records()[0]["extra"]["inputs"]
    assert inputs["top"] == "***REDACTED***"
    # The two apcore-typescript leaked: neither is ever seen by the flat pass,
    # only by the logger's recursive one — so the logger has to be holding the
    # same rules for either to be redacted.
    assert inputs["items"] == ["***REDACTED***"]
    assert inputs["nested"] == {"key": "***REDACTED***"}


def test_a_logged_output_is_redacted_at_every_position(payload: dict[str, Any]) -> None:
    h = Harness(_value_rule())
    ctx = _captured(inputs={}, output=payload)
    h.mw.before("executor.x.y", {}, ctx)
    h.mw.after("executor.x.y", {}, payload, ctx)

    output = h.records()[1]["extra"]["output"]
    assert output["top"] == "***REDACTED***"
    assert output["items"] == ["***REDACTED***"]
    assert output["nested"] == {"key": "***REDACTED***"}


def test_a_narrowed_rule_set_is_not_widened_again_by_the_logger_default() -> None:
    """The reverse error the same split produces, and the reason to align rather
    than merely recurse.

    An operator writing ``sensitive_keys: []`` has disabled key-based redaction.
    If the logger keeps its own default, the shipped list is applied underneath
    and ``password`` comes back redacted from a configuration that asked for no
    key rule at all.
    """
    h = Harness(RedactionConfig(sensitive_keys=[], regex_patterns=[]))
    h.mw.before("executor.x.y", {"password": "hunter2"}, _captured(inputs={"password": "hunter2"}))

    assert h.records()[0]["extra"]["inputs"]["password"] == "hunter2"


def test_with_no_config_the_shipped_defaults_still_apply() -> None:
    """The other half: aligning the passes must not disable out-of-the-box redaction."""
    h = Harness(None)
    payload = {"password": "hunter2", "nested": {"token": "abc"}}
    h.mw.before("executor.x.y", payload, _captured(inputs=payload))

    inputs = h.records()[0]["extra"]["inputs"]
    assert inputs["password"] == "***REDACTED***"
    assert inputs["nested"] == {"token": "***REDACTED***"}


def test_correlation_fields_survive_a_rule_that_would_match_them() -> None:
    h = Harness(RedactionConfig(sensitive_keys=[], regex_patterns=[".*"]))
    h.mw.before("executor.x.y", {}, Context.create())

    assert h.records()[0]["extra"]["module_id"] == "executor.x.y"
