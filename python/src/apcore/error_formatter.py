"""ErrorFormatterRegistry -- per-adapter error formatting (§8.8)."""

from __future__ import annotations

import threading
from typing import Any, Protocol, runtime_checkable

from apcore.errors import ErrorFormatterDuplicateError, ModuleError

__all__ = ["ErrorFormatter", "ErrorFormatterRegistry"]


@runtime_checkable
class ErrorFormatter(Protocol):
    """Protocol for adapter-specific error formatters."""

    def format(self, error: ModuleError, context: object) -> dict[str, Any]:
        """Format a ModuleError into an adapter-specific dict."""
        ...


class ErrorFormatterRegistry:
    """Registry of per-adapter error formatters (§8.8).

    Thread-safe: all mutations are internally synchronized.
    """

    _registry: dict[str, ErrorFormatter] = {}
    _lock: threading.Lock = threading.Lock()

    @classmethod
    def register(cls, adapter_name: str, formatter: ErrorFormatter) -> None:
        """Register a formatter for the given adapter name.

        Args:
            adapter_name: Unique adapter identifier (e.g. "mcp", "http").
            formatter: An object implementing the ErrorFormatter protocol.

        Raises:
            ErrorFormatterDuplicateError: If a formatter is already registered
                for ``adapter_name``.
        """
        with cls._lock:
            if adapter_name in cls._registry:
                raise ErrorFormatterDuplicateError(adapter_name=adapter_name)
            cls._registry[adapter_name] = formatter

    @classmethod
    def get(cls, adapter_name: str) -> ErrorFormatter | None:
        """Return the registered formatter for ``adapter_name``, or None."""
        with cls._lock:
            return cls._registry.get(adapter_name)

    @classmethod
    def format(
        cls,
        adapter_name: str,
        error: ModuleError,
        context: object = None,
    ) -> dict[str, Any]:
        """Format ``error`` using the registered formatter for ``adapter_name``.

        Falls back to ``error.to_dict()`` if no formatter is registered.

        Args:
            adapter_name: The adapter whose formatter to use.
            error: The ModuleError to format.
            context: Optional adapter-specific context object.

        Returns:
            A dict representation of the error.
        """
        formatter = cls.get(adapter_name)
        if formatter is None:
            return error.to_dict()
        return formatter.format(error, context)

    @classmethod
    def _reset(cls) -> None:
        """Clear all registrations (for testing only)."""
        with cls._lock:
            cls._registry.clear()
