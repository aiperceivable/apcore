"""Regex scans are bounded consistently at payload capture and log emission."""

import io
import json
from typing import Any

import pytest

from apcore import APCore, Config, Context, Middleware, REDACTED_VALUE, redact_sensitive
from apcore.errors import ConfigError
from apcore.observability.context_logger import ContextLogger, RedactionConfig


def test_regex_scan_cap_redacts_all_oversized_values_and_counts_utf8_bytes() -> None:
    data = {"values": ["éé", "ééx", "matches"], "trace_id": "oversized", "auth": "x", "private": "x"}
    schema = {"properties": {"private": {"x-sensitive": True}}}
    result = redact_sensitive(data, schema, regex_patterns=["^never$"], max_regex_scan_length=4)
    assert result == {
        "values": ["éé", REDACTED_VALUE, REDACTED_VALUE],
        "trace_id": "oversized",
        "auth": REDACTED_VALUE,
        "private": REDACTED_VALUE,
    }
    assert redact_sensitive(data, {}, max_regex_scan_length=0)["values"] == data["values"]


def test_configured_cap_applies_to_logger_and_executor_capture() -> None:
    config = Config({"obs": {"redaction": {"regex_patterns": ["never"], "max_regex_scan_length": 2}}})
    output = io.StringIO()
    logger = ContextLogger("test", output=output, redaction_config=RedactionConfig.from_config(config))
    logger.info("message", {"payload": ["é", "éé"]})
    assert json.loads(output.getvalue())["extra"] == {"payload": ["é", REDACTED_VALUE]}

    client = APCore(config=config)

    @client.module("redaction.check")
    def check(value: str) -> str:
        return value

    class Capture(Middleware):
        def after(self, module_id: str, inputs: dict[str, Any], output: dict[str, Any], context: Context[Any]) -> None:
            assert context.redacted_inputs == {"value": REDACTED_VALUE}
            assert context.redacted_output == {"result": REDACTED_VALUE}

    client.use(Capture())
    assert client.call("redaction.check", {"value": "éé"}) == {"result": "éé"}


@pytest.mark.parametrize("invalid", [-1, True, 1.5, "2", 1 << 53])
def test_invalid_regex_scan_cap_is_rejected(invalid: Any) -> None:
    with pytest.raises(ValueError, match="max_regex_scan_length"):
        redact_sensitive({}, {}, max_regex_scan_length=invalid)
    config = Config(
        {"version": "2.0.0", "project": {"name": "test"}, "obs": {"redaction": {"max_regex_scan_length": invalid}}}
    )
    with pytest.raises(ConfigError, match="max_regex_scan_length"):
        config.validate()
