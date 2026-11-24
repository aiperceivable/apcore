"""W3C Trace Context support: TraceParent parsing and TraceContext injection/extraction."""

from __future__ import annotations

import os
import re
from collections.abc import Mapping, Sequence
from dataclasses import dataclass, field
from typing import Any, TypeAlias

from apcore.errors import InvalidParentIdError

Context: TypeAlias = Any

__all__ = [
    "TraceParent",
    "TraceContext",
    "parse_tracestate",
    "format_tracestate",
]

_TRACEPARENT_RE = re.compile(r"^([0-9a-f]{2})-([0-9a-f]{32})-([0-9a-f]{16})-([0-9a-f]{2})$")
_PARENT_ID_RE = re.compile(r"^[0-9a-f]{16}$")

# Reserved keys used to carry parsed W3C trace state through the Context
# without breaking the Context dataclass surface.
_TRACE_FLAGS_KEY = "_apcore.trace.flags"
_TRACESTATE_KEY = "_apcore.trace.state"

_TRACESTATE_MAX_ENTRIES = 32


@dataclass(frozen=True)
class TraceParent:
    """Parsed W3C traceparent header fields and accompanying tracestate."""

    version: str  # "00"
    trace_id: str  # 32 lowercase hex chars
    parent_id: str  # 16 lowercase hex chars
    trace_flags: str  # "01" (sampled) or "00"
    # tracestate carried alongside traceparent.  Stored as a tuple of
    # (key, value) pairs to preserve order and remain hashable for the
    # frozen dataclass.
    tracestate: tuple[tuple[str, str], ...] = field(default_factory=tuple)


def parse_tracestate(raw: str) -> list[tuple[str, str]]:
    """Parse a W3C ``tracestate`` header into ordered ``(key, value)`` pairs.

    Per the W3C spec, entries are comma-separated ``key=value`` pairs.  Both
    sides are trimmed.  Malformed entries (missing ``=`` or empty key) are
    silently dropped.  At most :data:`_TRACESTATE_MAX_ENTRIES` (32) entries
    are returned; the remainder are discarded.
    """
    if not raw:
        return []
    entries: list[tuple[str, str]] = []
    for chunk in raw.split(","):
        if "=" not in chunk:
            continue
        # Split only on the first '=' so values containing '=' are preserved.
        key, _, value = chunk.partition("=")
        key = key.strip()
        value = value.strip()
        if not key:
            continue
        entries.append((key, value))
        if len(entries) >= _TRACESTATE_MAX_ENTRIES:
            break
    return entries


def format_tracestate(entries: Sequence[tuple[str, str]]) -> str:
    """Serialize ``(key, value)`` pairs back to a W3C ``tracestate`` header."""
    return ",".join(f"{k}={v}" for k, v in entries)


def _lookup_header_ci(headers: Mapping[str, str], name: str) -> str | None:
    """Look up *name* in *headers* case-insensitively.

    Plain ``dict`` callers may pass mixed-case keys (e.g. ``"Traceparent"``,
    ``"TRACEPARENT"``, ``"TraceState"``).  This helper walks ``items()`` and
    compares ``.lower()`` so any casing matches.
    """
    target = name.lower()
    for key, value in headers.items():
        if key.lower() == target:
            return value
    return None


class TraceContext:
    """Utilities for W3C Trace Context propagation."""

    @staticmethod
    def inject(context: Context, parent_id: str | None = None) -> dict[str, str]:
        """Build W3C trace headers from an apcore Context.

        Uses ``context.trace_id`` (already 32-hex format) for the traceparent
        ``trace-id`` field.  The ``parent-id`` is, in order of preference:

        1. The explicit *parent_id* argument when provided.  Must be a
           16-character lowercase hex string; otherwise
           :class:`~apcore.errors.InvalidParentIdError` is raised.  That class
           carries ``code == "INVALID_PARENT_ID"`` (decision D-51, matching
           apcore-typescript and apcore-rust) and still inherits
           :class:`ValueError`, so callers written against the pre-0.27
           ``except ValueError`` contract keep working.
        2. The ``span_id`` of the most recent span on the tracing stack
           (``context.data['_apcore.mw.tracing.spans']``) if present.
        3. A random 8-byte value.

        ``trace-flags`` is propagated from any inbound traceparent stored on
        the context (via :meth:`Context.create(trace_parent=...)`); root
        contexts default to ``"01"`` (sampled).  ``tracestate``, if carried
        on the context, is included in the returned header map.
        """
        if parent_id is not None:
            if not isinstance(parent_id, str) or not _PARENT_ID_RE.match(parent_id):
                raise InvalidParentIdError(parent_id)
            chosen_parent_id = parent_id
        else:
            spans_stack = context.data.get("_apcore.mw.tracing.spans")
            if spans_stack:
                chosen_parent_id = spans_stack[-1].span_id
            else:
                chosen_parent_id = os.urandom(8).hex()

        trace_id_hex = context.trace_id
        trace_flags = context.data.get(_TRACE_FLAGS_KEY, "01")
        traceparent = f"00-{trace_id_hex}-{chosen_parent_id}-{trace_flags}"
        headers: dict[str, str] = {"traceparent": traceparent}

        tracestate = context.data.get(_TRACESTATE_KEY)
        if tracestate:
            headers["tracestate"] = format_tracestate(tracestate)
        return headers

    @staticmethod
    def extract(headers: Mapping[str, str]) -> TraceParent | None:
        """Parse the ``traceparent`` (and optional ``tracestate``) headers.

        Header lookup is case-insensitive so callers passing a plain ``dict``
        with mixed-case keys (``"Traceparent"``, ``"TRACEPARENT"``,
        ``"TraceState"``) still resolve.  Returns ``None`` when the
        ``traceparent`` header is missing or malformed.
        """
        raw = _lookup_header_ci(headers, "traceparent")
        if raw is None:
            return None
        match = _TRACEPARENT_RE.match(raw.strip().lower())
        if match is None:
            return None
        version = match.group(1)
        trace_id = match.group(2)
        parent_id = match.group(3)
        if version == "ff":
            return None
        if trace_id == "0" * 32 or parent_id == "0" * 16:
            return None

        tracestate_raw = _lookup_header_ci(headers, "tracestate")
        tracestate: tuple[tuple[str, str], ...] = tuple(parse_tracestate(tracestate_raw)) if tracestate_raw else ()

        return TraceParent(
            version=version,
            trace_id=trace_id,
            parent_id=parent_id,
            trace_flags=match.group(4),
            tracestate=tracestate,
        )

    @staticmethod
    def from_traceparent(traceparent: str) -> TraceParent:
        """Strictly parse a traceparent string, raising on invalid format.

        Raises:
            ValueError: If *traceparent* does not match the expected
                ``00-{32hex}-{16hex}-{2hex}`` format.

        Note:
            This deliberately raises a *bare* ``ValueError`` rather than
            :class:`~apcore.errors.InvalidParentIdError`.  D-51 pins
            ``INVALID_PARENT_ID`` for one thing only: a caller-supplied
            ``parent_id`` override on :meth:`inject`.  Here the whole inbound
            header is malformed, which is a different failure, and
            apcore-typescript's ``fromTraceparent`` also throws a codeless
            ``Error`` (apcore-rust has no equivalent entry point).  Attaching
            ``INVALID_PARENT_ID`` here would invent a third shape rather than
            close a gap.
        """
        match = _TRACEPARENT_RE.match(traceparent.strip().lower())
        if match is None:
            raise ValueError(
                f"Malformed traceparent: {traceparent[:100]!r}. Expected format: 00-<32 hex>-<16 hex>-<2 hex>"
            )
        version = match.group(1)
        trace_id = match.group(2)
        parent_id = match.group(3)
        if version == "ff":
            raise ValueError("Invalid traceparent: version ff is not allowed")
        if trace_id == "0" * 32 or parent_id == "0" * 16:
            raise ValueError("Invalid traceparent: all-zero trace_id or parent_id")
        return TraceParent(
            version=version,
            trace_id=trace_id,
            parent_id=parent_id,
            trace_flags=match.group(4),
        )
