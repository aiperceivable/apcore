"""Typed key for type-safe access to context.data."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Generic, Protocol, TypeVar, overload

T = TypeVar("T")

_MISSING: Any = object()


class _ContextLike(Protocol):
    """Structural protocol for any object with a ``data`` mapping."""

    data: dict[str, Any]


@dataclass(frozen=True)
class ContextKey(Generic[T]):
    """Typed key for context.data with namespace isolation.

    Provides type-safe get/set/delete/exists operations on a named slot
    within ``Context.data``.  Immutable (frozen dataclass).
    """

    name: str

    @overload
    def get(self, ctx: _ContextLike) -> T | None: ...

    @overload
    def get(self, ctx: _ContextLike, default: T) -> T: ...

    def get(self, ctx: _ContextLike, default: T | None = None) -> T | None:
        """Return the value for this key, or *default* if absent.

        Overloaded so that ``key.get(ctx)`` returns ``T | None`` while
        ``key.get(ctx, default=value)`` narrows the return type to ``T``.
        """
        value = ctx.data.get(self.name, _MISSING)
        return default if value is _MISSING else value

    def set(self, ctx: _ContextLike, value: T) -> None:
        """Store *value* under this key in context.data."""
        ctx.data[self.name] = value

    def delete(self, ctx: _ContextLike) -> None:
        """Remove this key from context.data (no-op if absent)."""
        ctx.data.pop(self.name, None)

    def exists(self, ctx: _ContextLike) -> bool:
        """Return True if this key is present in context.data."""
        return self.name in ctx.data

    def scoped(self, suffix: str) -> ContextKey[T]:
        """Create a sub-key with ``{name}.{suffix}``."""
        return ContextKey(f"{self.name}.{suffix}")
