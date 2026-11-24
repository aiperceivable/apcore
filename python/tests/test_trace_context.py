"""Tests for W3C Trace Context support (TraceParent, TraceContext)."""

from __future__ import annotations

import re

import pytest

from apcore.context import Context
from apcore.errors import ErrorCodes, InvalidParentIdError
from apcore.trace_context import (
    TraceContext,
    TraceParent,
    format_tracestate,
    parse_tracestate,
)

_TRACEPARENT_FORMAT = re.compile(r"^[0-9a-f]{2}-[0-9a-f]{32}-[0-9a-f]{16}-[0-9a-f]{2}$")


class TestTraceContextInject:
    """TraceContext.inject() produces valid traceparent headers."""

    def test_inject_produces_valid_traceparent_format(self):
        ctx = Context.create()
        headers = TraceContext.inject(ctx)
        assert "traceparent" in headers
        assert _TRACEPARENT_FORMAT.match(headers["traceparent"])

    def test_inject_uses_context_trace_id(self):
        ctx = Context.create()
        headers = TraceContext.inject(ctx)
        parts = headers["traceparent"].split("-")
        assert parts[1] == ctx.trace_id

    def test_inject_version_is_00(self):
        ctx = Context.create()
        headers = TraceContext.inject(ctx)
        assert headers["traceparent"].startswith("00-")

    def test_inject_trace_flags_is_01(self):
        ctx = Context.create()
        headers = TraceContext.inject(ctx)
        assert headers["traceparent"].endswith("-01")

    def test_inject_uses_span_id_from_tracing_stack(self):
        ctx = Context.create()
        # Simulate a tracing span on the stack
        from apcore.observability.tracing import Span

        span = Span(trace_id=ctx.trace_id, name="test", start_time=0.0, span_id="abcdef0123456789")
        ctx.data["_apcore.mw.tracing.spans"] = [span]

        headers = TraceContext.inject(ctx)
        parts = headers["traceparent"].split("-")
        assert parts[2] == "abcdef0123456789"

    def test_inject_generates_parent_id_when_no_spans(self):
        ctx = Context.create()
        headers = TraceContext.inject(ctx)
        parts = headers["traceparent"].split("-")
        parent_id = parts[2]
        assert len(parent_id) == 16
        assert re.match(r"^[0-9a-f]{16}$", parent_id)


class TestTraceContextExtract:
    """TraceContext.extract() parses traceparent headers."""

    def test_extract_valid_traceparent(self):
        headers = {"traceparent": "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"}
        result = TraceContext.extract(headers)
        assert result is not None
        assert result.version == "00"
        assert result.trace_id == "4bf92f3577b34da6a3ce929d0e0e4736"
        assert result.parent_id == "00f067aa0ba902b7"
        assert result.trace_flags == "01"

    def test_extract_returns_none_for_missing_header(self):
        result = TraceContext.extract({})
        assert result is None

    def test_extract_returns_none_for_empty_dict(self):
        result = TraceContext.extract({"other-header": "value"})
        assert result is None

    def test_extract_returns_none_for_malformed_traceparent(self):
        result = TraceContext.extract({"traceparent": "invalid-format"})
        assert result is None

    def test_extract_returns_none_for_short_trace_id(self):
        result = TraceContext.extract({"traceparent": "00-abc-00f067aa0ba902b7-01"})
        assert result is None

    def test_extract_normalizes_uppercase_to_lowercase(self):
        # The regex requires lowercase; raw uppercase should still parse
        # because extract normalizes to lowercase
        headers = {"traceparent": "00-4BF92F3577B34DA6A3CE929D0E0E4736-00F067AA0BA902B7-01"}
        result = TraceContext.extract(headers)
        assert result is not None
        assert result.trace_id == "4bf92f3577b34da6a3ce929d0e0e4736"

    def test_extract_returns_none_for_all_zero_trace_id(self):
        headers = {"traceparent": "00-00000000000000000000000000000000-00f067aa0ba902b7-01"}
        result = TraceContext.extract(headers)
        assert result is None

    def test_extract_returns_none_for_all_zero_parent_id(self):
        headers = {"traceparent": "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01"}
        result = TraceContext.extract(headers)
        assert result is None

    def test_extract_returns_none_for_version_ff(self):
        headers = {"traceparent": "ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"}
        result = TraceContext.extract(headers)
        assert result is None

    def test_extract_with_unsampled_flags(self):
        headers = {"traceparent": "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00"}
        result = TraceContext.extract(headers)
        assert result is not None
        assert result.trace_flags == "00"


class TestTraceContextFromTraceparent:
    """TraceContext.from_traceparent() strict parsing."""

    def test_from_traceparent_valid(self):
        tp = TraceContext.from_traceparent("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01")
        assert tp.version == "00"
        assert tp.trace_id == "4bf92f3577b34da6a3ce929d0e0e4736"
        assert tp.parent_id == "00f067aa0ba902b7"
        assert tp.trace_flags == "01"

    def test_from_traceparent_raises_on_invalid(self):
        with pytest.raises(ValueError, match="Malformed traceparent"):
            TraceContext.from_traceparent("not-a-valid-traceparent")

    def test_from_traceparent_raises_on_empty(self):
        with pytest.raises(ValueError, match="Malformed traceparent"):
            TraceContext.from_traceparent("")

    def test_from_traceparent_raises_on_missing_parts(self):
        with pytest.raises(ValueError, match="Malformed traceparent"):
            TraceContext.from_traceparent("00-4bf92f3577b34da6a3ce929d0e0e4736")

    def test_from_traceparent_raises_on_all_zero_trace_id(self):
        with pytest.raises(ValueError, match="all-zero trace_id or parent_id"):
            TraceContext.from_traceparent("00-00000000000000000000000000000000-00f067aa0ba902b7-01")

    def test_from_traceparent_raises_on_all_zero_parent_id(self):
        with pytest.raises(ValueError, match="all-zero trace_id or parent_id"):
            TraceContext.from_traceparent("00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01")

    def test_from_traceparent_raises_on_version_ff(self):
        with pytest.raises(ValueError, match="version ff is not allowed"):
            TraceContext.from_traceparent("ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01")


class TestTraceParentDataclass:
    """TraceParent is frozen and immutable."""

    def test_trace_parent_frozen(self):
        tp = TraceParent(version="00", trace_id="a" * 32, parent_id="b" * 16, trace_flags="01")
        with pytest.raises(AttributeError):
            tp.version = "01"  # type: ignore[misc]

    def test_trace_parent_equality(self):
        tp1 = TraceParent(version="00", trace_id="a" * 32, parent_id="b" * 16, trace_flags="01")
        tp2 = TraceParent(version="00", trace_id="a" * 32, parent_id="b" * 16, trace_flags="01")
        assert tp1 == tp2

    def test_trace_parent_tracestate_defaults_to_empty(self):
        tp = TraceParent(version="00", trace_id="a" * 32, parent_id="b" * 16, trace_flags="01")
        assert tp.tracestate == ()


class TestRoundTrip:
    """inject() -> extract() round-trip preserves trace_id."""

    def test_inject_then_extract_preserves_trace_id(self):
        ctx = Context.create()
        headers = TraceContext.inject(ctx)
        parsed = TraceContext.extract(headers)

        assert parsed is not None
        assert parsed.trace_id == ctx.trace_id

    def test_inject_then_extract_preserves_parent_id(self):
        ctx = Context.create()
        headers = TraceContext.inject(ctx)
        parsed = TraceContext.extract(headers)

        assert parsed is not None
        # parent_id in the round-trip should match what was injected
        parts = headers["traceparent"].split("-")
        assert parsed.parent_id == parts[2]


class TestContextCreateWithTraceParent:
    """Context.create(trace_parent=...) uses the trace_parent's trace_id."""

    def test_context_create_with_trace_parent_uses_trace_id(self):
        tp = TraceParent(
            version="00",
            trace_id="4bf92f3577b34da6a3ce929d0e0e4736",
            parent_id="00f067aa0ba902b7",
            trace_flags="01",
        )
        ctx = Context.create(trace_parent=tp)
        # trace_id should be the 32 hex chars directly
        assert ctx.trace_id == "4bf92f3577b34da6a3ce929d0e0e4736"

    def test_context_create_with_trace_parent_produces_valid_32_hex(self):
        tp = TraceParent(
            version="00",
            trace_id="4bf92f3577b34da6a3ce929d0e0e4736",
            parent_id="00f067aa0ba902b7",
            trace_flags="01",
        )
        ctx = Context.create(trace_parent=tp)
        # Should be 32 lowercase hex chars
        assert len(ctx.trace_id) == 32
        assert all(c in "0123456789abcdef" for c in ctx.trace_id)

    def test_context_create_without_trace_parent_still_works(self):
        ctx = Context.create()
        # Should produce a valid 32-hex trace_id
        assert len(ctx.trace_id) == 32
        assert all(c in "0123456789abcdef" for c in ctx.trace_id)

    def test_context_create_without_trace_parent_generates_unique_ids(self):
        ctx1 = Context.create()
        ctx2 = Context.create()
        assert ctx1.trace_id != ctx2.trace_id

    def test_full_round_trip_context_to_headers_and_back(self):
        """Create context -> inject headers -> extract -> create new context."""
        original = Context.create()
        headers = TraceContext.inject(original)
        parsed = TraceContext.extract(headers)
        assert parsed is not None

        restored = Context.create(trace_parent=parsed)
        assert restored.trace_id == original.trace_id


class TestParseTracestate:
    """parse_tracestate() handles W3C tracestate header format."""

    def test_parse_simple_single_entry(self):
        assert parse_tracestate("vendor=value") == [("vendor", "value")]

    def test_parse_multiple_entries(self):
        assert parse_tracestate("a=1,b=2,c=3") == [
            ("a", "1"),
            ("b", "2"),
            ("c", "3"),
        ]

    def test_parse_trims_whitespace(self):
        assert parse_tracestate(" a = 1 , b = 2 ") == [("a", "1"), ("b", "2")]

    def test_parse_empty_string(self):
        assert parse_tracestate("") == []

    def test_parse_drops_malformed_entries_without_equals(self):
        # "novalue" lacks '=' and is dropped; valid entry kept.
        assert parse_tracestate("novalue,a=1") == [("a", "1")]

    def test_parse_drops_entries_with_empty_key(self):
        assert parse_tracestate("=v,a=1") == [("a", "1")]

    def test_parse_caps_at_32_entries(self):
        raw = ",".join(f"k{i}=v{i}" for i in range(40))
        result = parse_tracestate(raw)
        assert len(result) == 32
        assert result[0] == ("k0", "v0")
        assert result[31] == ("k31", "v31")

    def test_parse_value_may_contain_equals(self):
        # Per W3C, value may contain '=' — split only on the first '='.
        assert parse_tracestate("vendor=key=val") == [("vendor", "key=val")]


class TestFormatTracestate:
    """format_tracestate() serializes back to W3C header format."""

    def test_format_empty_list(self):
        assert format_tracestate([]) == ""

    def test_format_single_entry(self):
        assert format_tracestate([("vendor", "value")]) == "vendor=value"

    def test_format_multiple_entries_comma_separated(self):
        assert format_tracestate([("a", "1"), ("b", "2")]) == "a=1,b=2"

    def test_round_trip_parse_format(self):
        raw = "a=1,b=2,c=3"
        assert format_tracestate(parse_tracestate(raw)) == raw


class TestExtractTracestate:
    """TraceContext.extract() also reads tracestate header."""

    def test_extract_populates_tracestate_when_present(self):
        headers = {
            "traceparent": "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            "tracestate": "vendor1=val1,vendor2=val2",
        }
        result = TraceContext.extract(headers)
        assert result is not None
        assert list(result.tracestate) == [("vendor1", "val1"), ("vendor2", "val2")]

    def test_extract_tracestate_empty_when_header_missing(self):
        headers = {"traceparent": "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"}
        result = TraceContext.extract(headers)
        assert result is not None
        assert list(result.tracestate) == []


class TestCaseInsensitiveHeaders:
    """extract() must lookup headers case-insensitively."""

    def test_extract_with_capitalized_traceparent_key(self):
        headers = {"Traceparent": "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"}
        result = TraceContext.extract(headers)
        assert result is not None
        assert result.trace_id == "4bf92f3577b34da6a3ce929d0e0e4736"

    def test_extract_with_uppercase_traceparent_key(self):
        headers = {"TRACEPARENT": "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"}
        result = TraceContext.extract(headers)
        assert result is not None
        assert result.trace_id == "4bf92f3577b34da6a3ce929d0e0e4736"

    def test_extract_with_capitalized_tracestate_key(self):
        headers = {
            "Traceparent": "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            "TraceState": "vendor=value",
        }
        result = TraceContext.extract(headers)
        assert result is not None
        assert list(result.tracestate) == [("vendor", "value")]


class TestDynamicSamplingFlags:
    """inject() must propagate trace_flags rather than hardcoding '01'."""

    def test_unsampled_flag_round_trips(self):
        # Inbound traceparent has flags=00; downstream inject must preserve.
        headers_in = {
            "traceparent": "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00",
        }
        parsed = TraceContext.extract(headers_in)
        assert parsed is not None
        assert parsed.trace_flags == "00"

        ctx = Context.create(trace_parent=parsed)
        headers_out = TraceContext.inject(ctx)
        assert headers_out["traceparent"].endswith("-00")

    def test_sampled_flag_round_trips(self):
        headers_in = {
            "traceparent": "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        }
        parsed = TraceContext.extract(headers_in)
        assert parsed is not None
        ctx = Context.create(trace_parent=parsed)
        headers_out = TraceContext.inject(ctx)
        assert headers_out["traceparent"].endswith("-01")

    def test_inject_root_context_defaults_to_sampled(self):
        # No inbound traceparent -> default flags 01 for new roots.
        ctx = Context.create()
        headers = TraceContext.inject(ctx)
        assert headers["traceparent"].endswith("-01")


class TestInjectTracestate:
    """inject() returns tracestate header alongside traceparent."""

    def test_inject_round_trips_tracestate(self):
        headers_in = {
            "traceparent": "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            "tracestate": "vendor1=val1,vendor2=val2",
        }
        parsed = TraceContext.extract(headers_in)
        assert parsed is not None
        ctx = Context.create(trace_parent=parsed)
        headers_out = TraceContext.inject(ctx)
        assert headers_out.get("tracestate") == "vendor1=val1,vendor2=val2"

    def test_inject_omits_tracestate_when_absent(self):
        ctx = Context.create()
        headers = TraceContext.inject(ctx)
        assert "tracestate" not in headers


class TestInjectParentIdOverride:
    """inject(context, parent_id=...) accepts an explicit parent_id.

    The rejection tests below assert BOTH halves of the D-51 contract. They used
    to check `pytest.raises(ValueError)` only, which was satisfied by the bare
    `ValueError` this method raised before 0.27 — an exception carrying no
    `code`, so a caller matching on `INVALID_PARENT_ID` got nothing (Issue #32).
    `ValueError` is still asserted deliberately: `InvalidParentIdError` inherits
    it so pre-0.27 `except ValueError` callers keep working.
    """

    def test_explicit_parent_id_used_verbatim(self):
        ctx = Context.create()
        headers = TraceContext.inject(ctx, parent_id="aaaaaaaaaaaaaaaa")
        parts = headers["traceparent"].split("-")
        assert parts[2] == "aaaaaaaaaaaaaaaa"

    @pytest.mark.parametrize(
        "bad_parent_id",
        ["not-hex", "abcdef", "AAAAAAAAAAAAAAAA", "", "aaaaaaaaaaaaaaaaa"],
        ids=["non_hex", "too_short", "uppercase", "empty", "too_long"],
    )
    def test_explicit_parent_id_rejected_with_invalid_parent_id_code(self, bad_parent_id):
        ctx = Context.create()
        with pytest.raises(ValueError) as excinfo:
            TraceContext.inject(ctx, parent_id=bad_parent_id)
        raised = excinfo.value
        assert isinstance(raised, InvalidParentIdError)
        assert (
            raised.code == ErrorCodes.INVALID_PARENT_ID
        ), f"the wire code is the cross-SDK contract; got {raised.code!r}"
        assert raised.details["parent_id"] == bad_parent_id
        assert repr(bad_parent_id) in str(raised)
