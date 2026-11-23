"""Middleware that records ModuleError details into ErrorHistory."""

from __future__ import annotations

from typing import Any, TypeAlias

from apcore.errors import ModuleError
from apcore.middleware.base import Middleware
from apcore.observability.error_history import ErrorHistory

Context: TypeAlias = Any


class ErrorHistoryMiddleware(Middleware):
    """Records ModuleError instances into ErrorHistory on every on_error() call.

    Generic exceptions are ignored. This middleware never recovers from errors.
    """

    def __init__(self, error_history: ErrorHistory) -> None:
        self._error_history = error_history

    def on_error(
        self,
        module_id: str,
        inputs: dict[str, Any],
        error: Exception,
        context: Context,
    ) -> dict[str, Any] | None:
        if isinstance(error, ModuleError):
            self._error_history.record(module_id, error)
        return None
