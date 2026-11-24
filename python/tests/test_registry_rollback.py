"""Regression tests for Registry on_load failure rollback."""

from __future__ import annotations

import pytest
from apcore.registry.registry import Registry
from apcore.errors import ModuleError


class _BadModule:
    module_id = "test.rollback.bad"
    description = "bad module that fails on_load"

    def execute(self, inputs, context):
        pass

    def on_load(self):
        raise ModuleError("fail", code="LOAD_FAIL")

    def on_unload(self):
        pass


def test_register_rollback_clears_lowercase_map():
    """After on_load failure, lowercase_map must not retain the entry."""
    registry = Registry()
    with pytest.raises(Exception):
        registry.register("test.rollback.bad", _BadModule())
    # lowercase_map must not retain the entry
    assert "test.rollback.bad" not in registry._lowercase_map


class _AsyncOnLoadModule:
    """A module written the way apcore-typescript accepts: `async onLoad`."""

    description = "module with an async on_load"
    input_schema: dict = {"type": "object"}
    output_schema: dict = {"type": "object"}

    def __init__(self) -> None:
        self.ran = False

    async def on_load(self) -> None:
        self.ran = True

    def execute(self, inputs, context):
        return {}


def test_async_on_load_is_refused_not_silently_dropped() -> None:
    """An awaitable `on_load` must fail loudly rather than never running.

    `on_load` is synchronous in this SDK and in apcore-rust, where the trait
    signature enforces it; apcore-typescript accepts an async `onLoad` and has
    `register` return a promise that resolves once it completes. A module author
    following that shape used to get silence here — the coroutine was created,
    never awaited, and discarded, so the module was published and callable with
    none of its initialisation having run, leaving only a `RuntimeWarning` at
    the next garbage collection, attributed to unrelated code.

    That is the half-initialised module the deferred-publish design exists to
    prevent, reached through the one path that skipped the check
    (sync finding A-C-002).
    """
    registry = Registry()
    module = _AsyncOnLoadModule()

    with pytest.raises(ModuleError) as excinfo:
        registry.register("executor.async_on_load", module)

    assert "async on_load" in str(excinfo.value)
    assert not module.ran, "the coroutine must not have been driven"
    assert not registry.has("executor.async_on_load"), "a module whose on_load could not run must not be published"


def test_sync_on_load_still_runs_and_publishes() -> None:
    """The refusal must be specific to awaitables, not to on_load itself."""

    class _SyncOnLoadModule(_AsyncOnLoadModule):
        def on_load(self) -> None:  # type: ignore[override]
            self.ran = True

    registry = Registry()
    module = _SyncOnLoadModule()
    registry.register("executor.sync_on_load", module)

    assert module.ran
    assert registry.has("executor.sync_on_load")
