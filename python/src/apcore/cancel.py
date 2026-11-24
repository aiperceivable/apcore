"""Cooperative cancellation support for apcore module execution."""

from __future__ import annotations

from apcore.errors import ModuleError


class ExecutionCancelledError(ModuleError):
    """Raised when a module execution is cancelled via CancelToken."""

    def __init__(self, message: str = "Execution was cancelled") -> None:
        super().__init__(code="EXECUTION_CANCELLED", message=message)


class CancelToken:
    """Cooperative cancellation token for module execution.

    Pass to Context and check periodically during long-running operations.
    A linked parent's signal is visible here; cancelling this token never
    cancels its parent. Cancellation cannot reverse already consumed resources.
    """

    def __init__(self, parent: CancelToken | None = None) -> None:
        self._cancelled: bool = False
        self._parent = parent

    @property
    def is_cancelled(self) -> bool:
        """Whether cancellation has been requested."""
        return self._cancelled or (self._parent is not None and self._parent.is_cancelled)

    def child(self) -> CancelToken:
        """Create a linked token; child cancellation never cancels its parent."""
        return CancelToken(parent=self)

    def cancel(self) -> None:
        """Request cancellation."""
        self._cancelled = True

    def check(self) -> None:
        """Raise ExecutionCancelledError if cancelled.

        Call this periodically in long-running operations.
        """
        if self.is_cancelled:
            raise ExecutionCancelledError()

    def raise_if_cancelled(self) -> None:
        """Raise ExecutionCancelledError if cancelled.

        Canonical spec method name (`docs/features/cancellation.md`, "Contract:
        CancelToken.raise_if_cancelled"). Identical behavior to :meth:`check`,
        which is used throughout this SDK and elsewhere by external callers and
        is NOT deprecated by this method's addition — both names are supported.
        """
        self.check()

    def reset(self) -> None:
        """Reset the token for reuse."""
        self._cancelled = False
