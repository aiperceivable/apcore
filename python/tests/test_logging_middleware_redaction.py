"""PROTOCOL_SPEC §10.6.1 requirement 5 (D-131) — built-in logging middleware logs
the captured values.

Every case drives a REAL executor call on a module whose input AND output
schemas mark a field ``x-sensitive``, with no ``RedactionConfig`` supplied, and
asserts the emitted log record. The field names are deliberately outside the
default ``obs.redaction.sensitive_keys`` list, so the logger's own key-based pass
cannot hide a middleware that logs the raw value.
"""

from __future__ import annotations

import io
import json
import logging
from typing import Any

import pytest
from pydantic import BaseModel, Field

from apcore import Executor, Registry
from apcore.context import Context
from apcore.middleware.logging import LoggingMiddleware
from apcore.observability.context_logger import ContextLogger, ObsLoggingMiddleware

SSN = "123-45-6789"
DIAGNOSIS = "confidential-diagnosis-value"
MARKER = "***REDACTED***"


class _In(BaseModel):
    patient: str
    ssn: str = Field(json_schema_extra={"x-sensitive": True})


class _Out(BaseModel):
    patient: str
    diagnosis: str = Field(json_schema_extra={"x-sensitive": True})


class _Lookup:
    input_schema = _In
    output_schema = _Out
    description = "Returns a sensitive diagnosis for a sensitive identifier."

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"patient": inputs["patient"], "diagnosis": DIAGNOSIS}


def _run(middleware: Any) -> None:
    registry = Registry()
    registry.register("clinic.lookup", _Lookup())
    executor = Executor(registry, middlewares=[middleware])
    try:
        assert executor.call("clinic.lookup", {"patient": "p-1", "ssn": SSN})["diagnosis"] == DIAGNOSIS
    finally:
        executor.close()


def test_obs_logging_middleware_redacts_x_sensitive_input_and_output_without_config() -> None:
    buf = io.StringIO()
    _run(ObsLoggingMiddleware(logger=ContextLogger(name="d131", output=buf)))

    text = buf.getvalue()
    assert SSN not in text, "an x-sensitive INPUT field reached the log unredacted"
    assert DIAGNOSIS not in text, "an x-sensitive OUTPUT field reached the log unredacted"
    records = [json.loads(line) for line in text.splitlines() if line.strip()]
    started = next(r for r in records if r["message"] == "Module call started")
    completed = next(r for r in records if r["message"] == "Module call completed")
    assert started["extra"]["inputs"] == {"patient": "p-1", "ssn": MARKER}
    assert completed["extra"]["output"] == {"patient": "p-1", "diagnosis": MARKER}


def test_logging_middleware_redacts_x_sensitive_input_and_output() -> None:
    logger = logging.getLogger("apcore.test.d131")
    records: list[logging.LogRecord] = []

    class _Collect(logging.Handler):
        def emit(self, record: logging.LogRecord) -> None:
            records.append(record)

    handler = _Collect()
    logger.addHandler(handler)
    logger.setLevel(logging.INFO)
    try:
        with pytest.warns(DeprecationWarning):
            middleware = LoggingMiddleware(logger=logger)
        _run(middleware)
    finally:
        logger.removeHandler(handler)

    start = next(r for r in records if "START" in r.getMessage())
    end = next(r for r in records if "END" in r.getMessage())
    assert start.inputs == {"patient": "p-1", "ssn": MARKER}  # type: ignore[attr-defined]
    assert end.output == {"patient": "p-1", "diagnosis": MARKER}  # type: ignore[attr-defined]
    rendered = repr([vars(r) for r in records])
    assert SSN not in rendered and DIAGNOSIS not in rendered


def test_obs_logging_middleware_never_falls_back_to_raw_values() -> None:
    """Outside the pipeline nothing is captured, so nothing is logged — not the raw value."""
    buf = io.StringIO()
    mw = ObsLoggingMiddleware(logger=ContextLogger(name="d131", output=buf))
    ctx = Context.create()
    mw.before("clinic.lookup", {"ssn": SSN}, ctx)
    mw.after("clinic.lookup", {"ssn": SSN}, {"diagnosis": DIAGNOSIS}, ctx)

    text = buf.getvalue()
    assert SSN not in text and DIAGNOSIS not in text
