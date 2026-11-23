"""StreamingModule Protocol — modules that produce output incrementally.

@runtime_checkable only verifies method presence at isinstance() time.
Actual signature compliance is enforced at registration time by
Registry.register when annotations.streaming = True.
"""

from __future__ import annotations

from collections.abc import AsyncIterator
from typing import Protocol, runtime_checkable

from apcore.context import Context


@runtime_checkable
class StreamingModule(Protocol):
    """Modules that produce output incrementally MUST satisfy this Protocol.

    The ``stream`` method is an async generator returning dict chunks.
    Signature validation (arity, async, return type) is checked at
    registration time when ``ModuleAnnotations.streaming = True``.
    """

    async def stream(
        self,
        inputs: dict,
        context: Context,
    ) -> AsyncIterator[dict]: ...
