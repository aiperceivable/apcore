"""Helper callables for BindingLoader tests."""

from __future__ import annotations

from datetime import datetime


def typed_function(name: str, count: int = 1) -> dict:
    """A simple typed function for binding tests."""
    return {"name": name, "count": count}


def untyped_function(name, count=1):  # noqa: ANN001, ANN201
    """Function with no type hints."""
    return {"name": name, "count": count}


class SimpleService:
    """Service class with no-arg constructor."""

    def greet(self, name: str) -> str:
        return f"Hello, {name}"


class ComplexService:
    """Service class requiring constructor args."""

    def __init__(self, api_key: str) -> None:
        self.api_key = api_key

    def call(self) -> str:
        return "called"


NOT_CALLABLE = 42


def strict_incompatible_function(tags: set[str]) -> dict:
    """Input renders with `uniqueItems`, which OpenAI structured outputs rejects.

    Used by the ``auto_schema: strict`` binding tests.
    """
    return {"tags": sorted(tags)}


def strict_compatible_function(name: str, when: datetime, note: str | None = None) -> dict:
    """Optional + datetime fields — both accepted by OpenAI structured outputs.

    ``str | None`` renders as a nested ``anyOf`` and ``datetime`` as
    ``format: date-time``; the previous ad-hoc detector wrongly rejected both.
    """
    return {"name": name, "when": when.isoformat(), "note": note}
