"""Call chain safety guard (Algorithm A20)."""

from __future__ import annotations

from apcore.errors import (
    CallDepthExceededError,
    CallFrequencyExceededError,
    CircularCallError,
    ErrorCodes,
    InvalidInputError,
)

__all__ = ["guard_call_chain"]

#: Default limits matching PROTOCOL_SPEC.
DEFAULT_MAX_CALL_DEPTH = 32
DEFAULT_MAX_MODULE_REPEAT = 3


def guard_call_chain(
    module_id: str,
    call_chain: list[str] | tuple[str, ...],
    *,
    max_call_depth: int = DEFAULT_MAX_CALL_DEPTH,
    max_module_repeat: int = DEFAULT_MAX_MODULE_REPEAT,
) -> None:
    """Validate call chain safety (Algorithm A20).

    Performs three checks in order:
    1. **Depth limit** — call chain length must not exceed *max_call_depth*.
    2. **Circular detection** — strict cycles of length >= 2 (A→B→A).
    3. **Frequency throttle** — *module_id* must not appear more than
       *max_module_repeat* times in the chain.

    Args:
        module_id: The module about to be called.
        call_chain: Current call chain (should already include *module_id*
            at the end, as set by ``Context.child()``).
        max_call_depth: Maximum allowed chain length.
        max_module_repeat: Maximum times a module may appear in the chain.

    Raises:
        InvalidInputError: *max_call_depth* or *max_module_repeat* is below 1.
        CallDepthExceededError: Chain too deep.
        CircularCallError: Circular call detected.
        CallFrequencyExceededError: Module called too many times.
    """
    # D-84: the limit floor raises the typed apcore error, not a builtin
    # ``ValueError``. A builtin escapes every ``except ModuleError`` handler,
    # and Executor.validate()'s error mapper reports it as
    # ``error.code == "ValueError"`` — a string that is not an apcore error
    # code — for a misconfigured ``executor.max_call_depth: 0``.
    if max_call_depth < 1:
        raise InvalidInputError(
            message=f"max_call_depth must be >= 1, got {max_call_depth}",
            code=ErrorCodes.GENERAL_INVALID_INPUT,
        )
    if max_module_repeat < 1:
        raise InvalidInputError(
            message=f"max_module_repeat must be >= 1, got {max_module_repeat}",
            code=ErrorCodes.GENERAL_INVALID_INPUT,
        )

    chain = list(call_chain)

    # 1. Depth check
    if len(chain) > max_call_depth:
        raise CallDepthExceededError(
            depth=len(chain),
            max_depth=max_call_depth,
            call_chain=chain,
        )

    # 2. Circular detection (strict cycles of length >= 2)
    # call_chain already includes module_id at the end (from child()),
    # so check prior entries only.
    prior_chain = chain[:-1]
    if module_id in prior_chain:
        last_idx = len(prior_chain) - 1 - prior_chain[::-1].index(module_id)
        subsequence = prior_chain[last_idx + 1 :]
        if len(subsequence) > 0:
            raise CircularCallError(
                module_id=module_id,
                call_chain=chain,
            )

    # 3. Frequency check
    count = chain.count(module_id)
    if count > max_module_repeat:
        raise CallFrequencyExceededError(
            module_id=module_id,
            count=count,
            max_repeat=max_module_repeat,
            call_chain=chain,
        )
