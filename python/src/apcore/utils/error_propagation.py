"""Error propagation (Algorithm A11)."""

from __future__ import annotations

from typing import TYPE_CHECKING

from apcore.errors import ModuleError, ModuleExecuteError

if TYPE_CHECKING:
    from apcore.context import Context

__all__ = ["propagate_error"]


def propagate_error(error: Exception, module_id: str, context: Context) -> ModuleError:
    """Wrap a raw exception into a standardized ModuleError (Algorithm A11).

    If the error is already a ModuleError, enriches it with trace context.
    Otherwise wraps it as a ModuleExecuteError.

    Args:
        error: The raw exception.
        module_id: Module ID where the error occurred.
        context: Current execution context.

    Returns:
        A ModuleError with trace_id, module_id, and call_chain attached.
    """
    if isinstance(error, ModuleError):
        # Already a ModuleError -- enrich with context if missing
        if error.trace_id is None:
            error.trace_id = context.trace_id
        if "module_id" not in error.details:
            error.details["module_id"] = module_id
        if "call_chain" not in error.details:
            error.details["call_chain"] = list(context.call_chain)
        return error

    # Wrap raw exception as ModuleExecuteError
    wrapped = ModuleExecuteError(
        module_id=module_id,
        message=f"Module '{module_id}' raised {type(error).__name__}: {error}",
        cause=error,
        trace_id=context.trace_id,
    )
    wrapped.details["call_chain"] = list(context.call_chain)
    return wrapped
