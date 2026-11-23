"""Drive `trace_context.json` — W3C TraceContext alignment (Issue #35).

Mirrors apcore-typescript's driver in `tests/conformance.test.ts`
("Trace Context (W3C, Issue #35)"): one parametrized case per fixture entry,
`TraceContext.extract()` on the (possibly synthesized) header map, then
`TraceContext.inject()` on a `Context` built from the extracted traceparent.

`inject()` reads trace flags and tracestate off the context, so the round-trip
cases go through `Context.create(trace_parent=...)` — the public way to seed
them — rather than writing the reserved `context.data` keys directly.

Error-code note: `parent_id_override_rejected_malformed` declares
`error.code: "INVALID_PARENT_ID"` (decision D-51). The WIRE CODE is the
contract, so this driver asserts the `.code` carried by the exception the SDK
actually raised. It previously asserted `expected["error"]["code"] ==
"INVALID_PARENT_ID"` — the fixture against a transcription of itself, which
cannot fail on SDK behaviour and is precisely why apcore-python shipped a
codeless `ValueError` here undetected (apcore-python#32, aiperceivable/apcore#81).

The `pytest.raises(ValueError)` is deliberate and load-bearing too: the raised
`InvalidParentIdError` inherits both `ModuleError` and `ValueError`, so callers
written against the pre-0.27 `except ValueError` contract keep working.
"""

from __future__ import annotations

from typing import Any

import pytest

from apcore.context import Context
from apcore.trace_context import TraceContext, TraceParent

from .canonical_fixtures import load_fixture

FIXTURE = load_fixture("trace_context.json")
CASES: list[dict[str, Any]] = FIXTURE["test_cases"]

# Every `expected` key this driver knows how to check; an unknown key fails.
_KNOWN_EXPECTATIONS = {
    "trace_id",
    "parent_id",
    "trace_flags",
    "tracestate_entries",
    "reinjected_tracestate",
    "tracestate_retained_count",
    "tracestate_dropped_count",
    "tracestate_first_key",
    "tracestate_last_key",
    "extract_succeeded",
    "extracted_trace_flags",
    "injected_trace_flags",
    "injected_traceparent",
    "parent_id_in_output",
    "error",
}


def _headers(case: dict[str, Any]) -> dict[str, str]:
    """Build the header map, expanding the `tracestate_entry_count` shorthand."""
    headers: dict[str, str] = {}
    for key, value in case["input"]["headers"].items():
        if key == "tracestate_entry_count":
            count = int(value)
            headers["tracestate"] = ",".join(f"vendor{i:02d}=opaque{i:02d}" for i in range(count))
        else:
            headers[key] = str(value)
    return headers


def _declared_tracestate_entries(headers: dict[str, str]) -> int:
    raw = headers.get("tracestate", "")
    return 0 if not raw else len(raw.split(","))


def _context_from(parsed: TraceParent) -> Context:
    """A Context seeded with the inbound traceparent so inject() round-trips it."""
    return Context.create(trace_parent=parsed)


@pytest.mark.parametrize("case", CASES, ids=lambda c: c["id"])
def test_trace_context_case(case: dict[str, Any]) -> None:
    cid = case["id"]
    expected: dict[str, Any] = case["expected"]
    unknown = set(expected) - _KNOWN_EXPECTATIONS
    assert not unknown, (
        f"[{cid}] trace_context.json declares expectations this driver cannot check: "
        f"{sorted(unknown)} — extend _KNOWN_EXPECTATIONS rather than ignoring them"
    )

    headers = _headers(case)
    parsed = TraceContext.extract(headers)

    if "error" in expected:
        assert parsed is not None, f"[{cid}] precondition: the inbound traceparent must parse"
        bad_parent_id = case["input"]["inject_parent_id"]
        # `ValueError` here is the back-compat half of the contract: the raised
        # error must stay catchable by callers written before it carried a code.
        with pytest.raises(ValueError) as excinfo:
            TraceContext.inject(_context_from(parsed), bad_parent_id)
        raised = excinfo.value
        actual_code = getattr(raised, "code", None)
        assert actual_code == expected["error"]["code"], (
            f"[{cid}] the WIRE CODE is the contract: the raised {type(raised).__name__} carries "
            f"code={actual_code!r}, fixture requires {expected['error']['code']!r}. "
            f"apcore-typescript sets `code = 'INVALID_PARENT_ID'` and apcore-rust returns "
            f"`ErrorCode::InvalidParentId`; a codeless exception here leaves a polyglot caller "
            f"matching on the code with nothing to match."
        )
        assert repr(bad_parent_id) in str(
            raised
        ), f"[{cid}] the rejection message must name the offending parent_id; got: {raised}"
        return

    if expected.get("extract_succeeded") is True:
        assert parsed is not None, f"[{cid}] extract() must succeed for headers {headers}"
    assert parsed is not None, f"[{cid}] extract() returned None for headers {headers}"

    if "trace_id" in expected:
        assert parsed.trace_id == expected["trace_id"], f"[{cid}] trace_id mismatch"
    if "parent_id" in expected:
        assert parsed.parent_id == expected["parent_id"], f"[{cid}] parent_id mismatch"
    if "trace_flags" in expected:
        assert parsed.trace_flags == expected["trace_flags"], f"[{cid}] trace_flags mismatch"
    if "extracted_trace_flags" in expected:
        assert parsed.trace_flags == expected["extracted_trace_flags"], (
            f"[{cid}] the inbound sampling decision must be read, not hardcoded: "
            f"got {parsed.trace_flags!r}, expected {expected['extracted_trace_flags']!r}"
        )

    entries = [list(pair) for pair in parsed.tracestate]
    if "tracestate_entries" in expected:
        assert entries == expected["tracestate_entries"], (
            f"[{cid}] tracestate entries must parse in on-the-wire order: "
            f"got {entries}, expected {expected['tracestate_entries']}"
        )
    if "tracestate_retained_count" in expected:
        assert len(entries) == expected["tracestate_retained_count"], (
            f"[{cid}] tracestate must be capped at the W3C limit: retained {len(entries)}, "
            f"expected {expected['tracestate_retained_count']}"
        )
    if "tracestate_dropped_count" in expected:
        dropped = _declared_tracestate_entries(headers) - len(entries)
        assert (
            dropped == expected["tracestate_dropped_count"]
        ), f"[{cid}] dropped {dropped} tracestate entries, expected {expected['tracestate_dropped_count']}"
    if "tracestate_first_key" in expected:
        assert (
            entries and entries[0][0] == expected["tracestate_first_key"]
        ), f"[{cid}] first retained tracestate key mismatch"
    if "tracestate_last_key" in expected:
        assert (
            entries and entries[-1][0] == expected["tracestate_last_key"]
        ), f"[{cid}] last retained tracestate key mismatch — the cap must drop the TAIL, not the head"

    inject_keys = {
        "reinjected_tracestate",
        "injected_trace_flags",
        "injected_traceparent",
        "parent_id_in_output",
    }
    if not (inject_keys & set(expected)):
        return

    injected = TraceContext.inject(_context_from(parsed), case["input"].get("inject_parent_id"))
    segments = injected["traceparent"].split("-")

    if "injected_trace_flags" in expected:
        assert (
            segments[3] == expected["injected_trace_flags"]
        ), f"[{cid}] injected trace flags: got {segments[3]!r}, expected {expected['injected_trace_flags']!r}"
    if "injected_traceparent" in expected:
        assert injected["traceparent"] == expected["injected_traceparent"], (
            f"[{cid}] injected traceparent: got {injected['traceparent']!r}, "
            f"expected {expected['injected_traceparent']!r}"
        )
    if "parent_id_in_output" in expected:
        assert segments[2] == expected["parent_id_in_output"], (
            f"[{cid}] the parent_id override must occupy the third traceparent segment: "
            f"got {segments[2]!r}, expected {expected['parent_id_in_output']!r}"
        )
    if "reinjected_tracestate" in expected:
        assert injected.get("tracestate") == expected["reinjected_tracestate"], (
            f"[{cid}] tracestate must serialize back losslessly: "
            f"got {injected.get('tracestate')!r}, expected {expected['reinjected_tracestate']!r}"
        )


def test_fixture_case_ids_are_unique() -> None:
    ids = [tc["id"] for tc in CASES]
    assert len(ids) == len(set(ids)), f"duplicate case ids: {sorted({i for i in ids if ids.count(i) > 1})}"
