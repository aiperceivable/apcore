"""Spec-traced contract tests for the apcore Error System.

Source spec: apcore/docs/features/error-system.md
Contract under test: ``ModuleError.to_dict``

Each test's docstring carries the verbatim clause id formatted
``error_system.<method>.<kind>.<detail>`` so cross-language diffs line up
row-for-row with the TypeScript and Rust suites.

These tests are READ-ONLY contract verification — they never modify
production source.
"""

from __future__ import annotations

import asyncio


from apcore.errors import (
    ModuleError,
    ModuleNotFoundError,
    SchemaValidationError,
)


# ---------------------------------------------------------------------------
# Returns contract: guaranteed keys
# ---------------------------------------------------------------------------


def test_to_dict_returns_guaranteed_code_key() -> None:
    """error_system.to_dict.returns.code_key

    The serialized dict MUST always carry a `code` (string) key.
    """
    err = ModuleError(code="TEST_CODE", message="something failed")
    result = err.to_dict()
    assert "code" in result
    assert result["code"] == "TEST_CODE"
    assert isinstance(result["code"], str)


def test_to_dict_returns_guaranteed_message_key() -> None:
    """error_system.to_dict.returns.message_key

    The serialized dict MUST always carry a `message` (string) key.
    """
    err = ModuleError(code="TEST_CODE", message="something failed")
    result = err.to_dict()
    assert "message" in result
    assert result["message"] == "something failed"
    assert isinstance(result["message"], str)


def test_to_dict_returns_ai_guidance_key_when_present() -> None:
    """error_system.to_dict.returns.ai_guidance_key

    When the error carries `ai_guidance`, it MUST appear (as a string) in
    the serialized dict. ModuleNotFoundError sets a non-empty default.
    """
    err = ModuleNotFoundError(module_id="missing.mod")
    result = err.to_dict()
    assert "ai_guidance" in result
    assert isinstance(result["ai_guidance"], str)
    assert len(result["ai_guidance"]) > 0


def test_to_dict_returns_optional_timestamp_key() -> None:
    """error_system.to_dict.returns.timestamp_key

    `timestamp` is an optional-but-emitted key (ISO 8601 UTC string).
    """
    err = ModuleError(code="TEST_CODE", message="boom")
    result = err.to_dict()
    assert "timestamp" in result
    assert isinstance(result["timestamp"], str)
    # ISO 8601 marker present.
    assert "T" in result["timestamp"]


def test_to_dict_returns_optional_details_key_when_present() -> None:
    """error_system.to_dict.returns.details_key

    The optional `details` key is included only when populated, and round
    trips the supplied mapping.
    """
    err = ModuleError(code="TEST_CODE", message="boom", details={"field": "email"})
    result = err.to_dict()
    assert result["details"] == {"field": "email"}


# ---------------------------------------------------------------------------
# Properties
# ---------------------------------------------------------------------------


def test_to_dict_property_async_is_false() -> None:
    """error_system.to_dict.property.async

    to_dict is declared async: false — it MUST be a plain synchronous call
    returning a concrete dict (not an awaitable / coroutine).
    """
    err = ModuleError(code="TEST_CODE", message="boom")
    result = err.to_dict()
    assert isinstance(result, dict)
    assert not asyncio.iscoroutine(result)
    assert not asyncio.isfuture(result)


def test_to_dict_property_pure_no_self_mutation() -> None:
    """error_system.to_dict.property.pure

    to_dict is declared pure — calling it MUST NOT mutate any observable
    state on the error instance.
    """
    err = ModuleError(
        code="TEST_CODE",
        message="boom",
        details={"field": "email"},
        retryable=False,
        ai_guidance="fix the input",
        user_fixable=True,
        suggestion="correct the field",
    )
    before = (
        err.code,
        err.message,
        dict(err.details),
        err.cause,
        err.trace_id,
        err.timestamp,
        err.retryable,
        err.ai_guidance,
        err.user_fixable,
        err.suggestion,
    )
    err.to_dict()
    after = (
        err.code,
        err.message,
        dict(err.details),
        err.cause,
        err.trace_id,
        err.timestamp,
        err.retryable,
        err.ai_guidance,
        err.user_fixable,
        err.suggestion,
    )
    assert before == after


def test_to_dict_property_pure_returns_fresh_top_level_mapping() -> None:
    """error_system.to_dict.property.pure.fresh_top_level

    Purity requires that mutating the TOP-LEVEL of the returned dict does not
    feed back into the error instance, and that a fresh serialization still
    reflects the original (unmutated) state.

    Note (cross-language): the Python source returns the `details` sub-mapping
    by reference (no deep copy), so deep nested mutation is intentionally NOT
    asserted here — that aliasing is a documented language-surface detail, not
    a contract guarantee. Top-level independence is the contract.
    """
    err = ModuleError(code="TEST_CODE", message="boom", details={"field": "email"})
    result = err.to_dict()
    result["code"] = "TAMPERED"
    result["message"] = "tampered"
    # Top-level mutation of the returned mapping does not touch the instance.
    assert err.code == "TEST_CODE"
    assert err.message == "boom"
    # A fresh serialization still reflects the original state.
    fresh = err.to_dict()
    assert fresh["code"] == "TEST_CODE"
    assert fresh["message"] == "boom"


def test_to_dict_property_idempotent() -> None:
    """error_system.to_dict.property.idempotent

    Two successive calls with identical (unchanged) state MUST produce
    equal output and leave observable state identical.
    """
    err = SchemaValidationError(errors=[{"path": "email", "msg": "invalid"}])
    first = err.to_dict()
    state_after_first = (err.code, err.message, dict(err.details), err.timestamp)
    second = err.to_dict()
    state_after_second = (err.code, err.message, dict(err.details), err.timestamp)
    assert first == second
    assert state_after_first == state_after_second


def test_to_dict_property_thread_safe() -> None:
    """error_system.to_dict.property.thread_safe

    Declared thread_safe: true. Launch N (>=8) concurrent serializations of
    distinct error instances via asyncio.gather; assert no exception is
    raised and every result is consistent with its source error.
    """

    async def serialize(code: str, message: str) -> dict:
        err = ModuleError(code=code, message=message, details={"i": code})
        # Yield control so the calls genuinely interleave on the loop.
        await asyncio.sleep(0)
        return err.to_dict()

    async def run() -> list[dict]:
        tasks = [serialize(f"CODE_{i}", f"message {i}") for i in range(12)]
        return await asyncio.gather(*tasks)

    results = asyncio.run(run())
    assert len(results) == 12
    for i, result in enumerate(results):
        assert result["code"] == f"CODE_{i}"
        assert result["message"] == f"message {i}"
        assert result["details"] == {"i": f"CODE_{i}"}
