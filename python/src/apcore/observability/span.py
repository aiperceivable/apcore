"""Span dataclass and factory.

Lives in its own module so foundational consumers (such as
:mod:`apcore.observability.batch_span_processor`) can depend on the
:class:`Span` type without importing the higher-level
:mod:`apcore.observability.tracing` module, which would otherwise create
a circular dependency.
"""

from __future__ import annotations

import os
from dataclasses import dataclass, field
from typing import Any


@dataclass
class Span:
    """A trace span representing a unit of work in the apcore pipeline."""

    trace_id: str
    name: str
    start_time: float
    span_id: str = field(default_factory=lambda: os.urandom(8).hex())
    parent_span_id: str | None = None
    attributes: dict[str, Any] = field(default_factory=dict)
    end_time: float | None = None
    status: str = "ok"
    events: list[dict[str, Any]] = field(default_factory=list)


def create_span(
    *,
    trace_id: str,
    name: str,
    start_time: float,
    span_id: str | None = None,
    parent_span_id: str | None = None,
    attributes: dict[str, Any] | None = None,
) -> Span:
    """Factory function to create a Span with sensible defaults."""
    return Span(
        trace_id=trace_id,
        name=name,
        start_time=start_time,
        span_id=span_id if span_id is not None else os.urandom(8).hex(),
        parent_span_id=parent_span_id,
        attributes=attributes if attributes is not None else {},
    )


__all__ = ["Span", "create_span"]
