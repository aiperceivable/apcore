"""PROTOCOL_SPEC §10.6.1 "Where the rules apply" — the configured redaction
rules must reach the executor's capture point, not only log emission
(aiperceivable/apcore#120).

Every case here builds a real ``Config``, constructs a real client, executes a
real module, and reads ``context.redacted_inputs`` / ``redacted_output``. That
shape is the point of the file, and it is stated in the issue as the acceptance
condition, because **every** pre-existing redaction test drove either the
config object or the logger — and that is exactly why a MUST written in
``docs/features/observability.md`` went unimplemented in all three SDKs for the
entire life of the keys.

The capture point is the surface that matters more. It fills what the audit
trail carries: governance events, error histories, and any middleware reading
``context.redacted_inputs``. Before this, an operator who added a
``regex_patterns`` entry for a bearer token got it redacted in the log line they
were watching and stored in the record they were not.
"""

from __future__ import annotations

from typing import Any

import pytest
from pydantic import BaseModel

from apcore import APCore
from apcore.config import Config
from apcore.context import Context
from apcore.middleware.base import Middleware


class _Capture(Middleware):
    """Reads `redacted_inputs` / `redacted_output` the way a real consumer does.

    Deliberately NOT by holding on to the `Context` handed to `call`: the
    pipeline derives a CHILD context (`base_ctx.child(module_id)`) and the
    capture point writes to that, so the caller's object stays `None` and an
    assertion against it would test the wrong object. Middleware is where
    `ObsLoggingMiddleware` itself reads these fields, so it is the surface the
    contract is actually about.
    """

    def __init__(self) -> None:
        self.inputs: dict[str, Any] | None = None
        self.output: dict[str, Any] | None = None

    def before(self, module_id: str, inputs: dict[str, Any], context: Any) -> None:
        self.inputs = context.redacted_inputs
        return None

    def after(self, module_id: str, inputs: dict[str, Any], output: dict[str, Any], context: Any) -> None:
        self.inputs = context.redacted_inputs
        self.output = context.redacted_output
        return None


SECRET = "sk-abcdef123456"


class _In(BaseModel):
    note: str
    label: str
    amount: int


class _Out(BaseModel):
    echoed: str
    label: str


class _Echo:
    input_schema = _In
    output_schema = _Out
    description = "Echo the note back so the OUTPUT capture point has something to redact."

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"echoed": inputs["note"], "label": inputs["label"]}


def _client(redaction: dict[str, Any] | None) -> tuple[APCore, _Capture]:
    cfg: dict[str, Any] = {"version": "1.0", "project": {"name": "capture-point"}}
    if redaction is not None:
        cfg["obs"] = {"redaction": redaction}
    client = APCore(config=Config(cfg))
    client.register("executor.test.echo", _Echo())
    seen = _Capture()
    client.use(seen)
    return client, seen


def _run(client: APCore, seen: _Capture) -> _Capture:
    client.call("executor.test.echo", {"note": SECRET, "label": "public", "amount": 42})
    assert seen.inputs is not None, "the capture point must have run at all"
    return seen


class TestConfiguredRulesReachTheCapturePoint:
    def test_a_configured_regex_redacts_the_captured_input(self) -> None:
        client, seen = _client({"sensitive_keys": [], "regex_patterns": ["sk-[A-Za-z0-9]{6,}"]})
        seen = _run(client, seen)
        assert seen.inputs is not None
        assert seen.inputs["note"] == "***REDACTED***"
        # The discriminating half: a field the rule does NOT match must survive,
        # or "redacted everything" would pass the assertion above.
        assert seen.inputs["label"] == "public"

    def test_a_configured_regex_redacts_the_captured_output(self) -> None:
        client, seen = _client({"sensitive_keys": [], "regex_patterns": ["sk-[A-Za-z0-9]{6,}"]})
        seen = _run(client, seen)
        assert seen.output is not None
        assert seen.output["echoed"] == "***REDACTED***"
        assert seen.output["label"] == "public"

    def test_a_configured_sensitive_key_redacts_the_captured_input(self) -> None:
        # `label` matches nothing in the shipped default list, so this case can
        # only pass if the OPERATOR's list was read.
        client, seen = _client({"sensitive_keys": ["label"]})
        seen = _run(client, seen)
        assert seen.inputs is not None
        assert seen.inputs["label"] == "***REDACTED***"
        assert seen.inputs["note"] == SECRET

    def test_a_configured_replacement_token_is_used(self) -> None:
        client, seen = _client({"sensitive_keys": ["label"], "replacement": "<<GONE>>"})
        seen = _run(client, seen)
        assert seen.inputs is not None
        assert seen.inputs["label"] == "<<GONE>>"

    def test_a_non_string_value_is_still_not_tested(self) -> None:
        """§10.6.1 requirement 2 holds at this surface too, not only at logging."""
        client, seen = _client({"sensitive_keys": [], "regex_patterns": ["[0-9]+"]})
        seen = _run(client, seen)
        assert seen.inputs is not None
        assert seen.inputs["amount"] == 42


class TestTheDefaultsStillApply:
    """Requirement 3: "no configuration" means the DEFAULTS, never no redaction.

    Wiring the capture point must not change what an unconfigured caller gets —
    that is what makes this change additive rather than a behaviour break.
    """

    def test_an_explicitly_narrowed_list_is_honoured(self) -> None:
        client, seen = _client({"sensitive_keys": []})
        seen = _run(client, seen)
        assert seen.inputs is not None
        assert seen.inputs["note"] == SECRET

    @pytest.mark.parametrize("redaction", [None, {}])
    def test_with_no_redaction_configured_the_default_list_applies(self, redaction: dict[str, Any] | None) -> None:
        cfg: dict[str, Any] = {"version": "1.0", "project": {"name": "capture-point"}}
        if redaction is not None:
            cfg["obs"] = {"redaction": redaction}
        client = APCore(config=Config(cfg))

        class _SecretIn(BaseModel):
            password: str
            keep: str

        class _SecretOut(BaseModel):
            ok: bool

        class _M:
            input_schema = _SecretIn
            output_schema = _SecretOut
            description = "A module whose input carries a field the default list covers."

            def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                return {"ok": True}

        client.register("executor.test.secret", _M())
        seen = _Capture()
        client.use(seen)
        client.call("executor.test.secret", {"password": "hunter2", "keep": "v"})

        assert seen.inputs is not None
        assert seen.inputs["password"] == "***REDACTED***"
        assert seen.inputs["keep"] == "v"
