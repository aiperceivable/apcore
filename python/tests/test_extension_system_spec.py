"""Spec-traced contract tests for the Extension System.

Generated from /apcore/docs/features/extension-system.md.
Each test carries a verbatim clause id of the form
'extension_system.<method>.<kind>.<detail>' so cross-language rows line up.

Contracts covered:
  - ExtensionManager.register
  - ExtensionManager.get
  - ExtensionManager.get_all
  - ExtensionManager.unregister
  - ExtensionManager.apply

These are TESTS ONLY; production source is never modified.
"""

from __future__ import annotations

from typing import Any
from unittest.mock import MagicMock

import pytest

from apcore.acl import ACL
from apcore.errors import InvalidInputError
from apcore.extensions import ExtensionManager
from apcore.middleware import Middleware
from apcore.observability.tracing import InMemoryExporter, TracingMiddleware


# ---------------------------------------------------------------------------
# Helpers: concrete implementations satisfying the runtime-checkable protocols
# ---------------------------------------------------------------------------


class StubDiscoverer:
    """Concrete discoverer satisfying the Discoverer protocol."""

    def discover(self, roots: list[str]) -> list[dict[str, Any]]:
        return []


class StubValidator:
    """Concrete validator satisfying the ModuleValidator protocol."""

    def validate(self, module: Any) -> list[str]:
        return []


class StubMiddleware(Middleware):
    """Concrete middleware satisfying the Middleware base."""


class StubExporter:
    """Concrete span exporter satisfying the SpanExporter protocol."""

    def __init__(self) -> None:
        self.exported: list[Any] = []

    def export(self, span: Any) -> None:
        self.exported.append(span)


# ===========================================================================
# Contract: ExtensionManager.register
# ===========================================================================


def test_extension_system_register_input_point_name_unknown() -> None:
    """extension_system.register.input.point_name.unknown

    Spec ### Inputs: unknown point_name MUST raise. Spec ### Errors lists
    `ExtensionPointNotFoundError` (or `ValueError` / `Err(ModuleError)`); the
    Python SDK raises `InvalidInputError(code=GENERAL_INVALID_INPUT)`, the same
    code apcore-rust has always used and apcore-typescript now uses (D-108).
    Assert the CODE, not just the type — the type alone was `KeyError`, which
    carries no code and is what a plain dict miss raises.
    """
    mgr = ExtensionManager()
    with pytest.raises(InvalidInputError) as exc_info:
        mgr.register("no_such_point", StubMiddleware())
    assert exc_info.value.code == "GENERAL_INVALID_INPUT"
    assert "Unknown extension point" in str(exc_info.value)


def test_extension_system_register_input_extension_wrong_type() -> None:
    """extension_system.register.input.extension.wrong_type

    Spec ### Inputs: `extension` MUST satisfy the point's declared type;
    type checking happens at registration. Build an input that FAILS the rule
    (a str registered for the `middleware` point) and assert the declared
    error TYPE (`TypeError`) plus the message identifying the type mismatch.
    """
    mgr = ExtensionManager()
    with pytest.raises(TypeError) as exc_info:
        mgr.register("middleware", "not-a-middleware")
    assert "must be an instance of" in str(exc_info.value)


def test_extension_system_register_error_ExtensionPointNotFoundError() -> None:
    """extension_system.register.error.ExtensionPointNotFoundError

    Spec ### Errors: `ExtensionPointNotFoundError` (Python:
    `InvalidInputError(code=GENERAL_INVALID_INPUT)`) when `point_name` is not a
    registered extension point. Trigger it and assert the code + the diagnostic
    identifying the offending name.
    """
    mgr = ExtensionManager()
    with pytest.raises(InvalidInputError) as exc_info:
        mgr.register("totally_unknown", StubDiscoverer())
    assert exc_info.value.code == "GENERAL_INVALID_INPUT"
    assert "totally_unknown" in str(exc_info.value)


def test_extension_system_register_error_ExtensionTypeError() -> None:
    """extension_system.register.error.ExtensionTypeError

    Spec ### Errors: `ExtensionTypeError` (Python: `TypeError`) when the
    extension does not satisfy the point's expected type. Register a
    Middleware where a Discoverer is required and assert the type.
    """
    mgr = ExtensionManager()
    with pytest.raises(TypeError) as exc_info:
        mgr.register("discoverer", StubMiddleware())
    assert "discoverer" in str(exc_info.value)


def test_extension_system_register_property_async_false() -> None:
    """extension_system.register.property.async.false

    Spec ### Properties: async=false. register() is synchronous: its return
    value is plain None, not an awaitable/coroutine.
    """
    import inspect

    mgr = ExtensionManager()
    result = mgr.register("middleware", StubMiddleware())
    assert result is None
    assert not inspect.isawaitable(result)
    assert not inspect.iscoroutinefunction(mgr.register)


def test_extension_system_register_property_idempotent_single_replaces() -> None:
    """extension_system.register.property.idempotent.single_replaces

    Spec ### Properties: idempotent=false for single-cardinality (replaces).
    Registering twice on `discoverer` (multiple=false) must leave only the
    last value observable via get().
    """
    mgr = ExtensionManager()
    first = StubDiscoverer()
    second = StubDiscoverer()
    mgr.register("discoverer", first)
    mgr.register("discoverer", second)
    assert mgr.get("discoverer") is second
    assert mgr.get("discoverer") is not first


def test_extension_system_register_property_idempotent_multi_accumulates() -> None:
    """extension_system.register.property.idempotent.multi_accumulates

    Spec ### Properties: accumulating for multi-cardinality. Registering twice
    on `middleware` (multiple=true) must keep both, in registration order.
    """
    mgr = ExtensionManager()
    mw1 = StubMiddleware()
    mw2 = StubMiddleware()
    mgr.register("middleware", mw1)
    mgr.register("middleware", mw2)
    assert mgr.get_all("middleware") == [mw1, mw2]


# ===========================================================================
# Contract: ExtensionManager.get
# ===========================================================================


def test_extension_system_get_property_async_false() -> None:
    """extension_system.get.property.async.false

    Spec ### Properties: async=false. get() returns a plain value, not an
    awaitable, and the callable is not a coroutine function.
    """
    import inspect

    mgr = ExtensionManager()
    result = mgr.get("discoverer")
    assert not inspect.isawaitable(result)
    assert not inspect.iscoroutinefunction(mgr.get)


def test_extension_system_get_returns_none_when_empty() -> None:
    """extension_system.get.error.no_error_returns_none

    Spec ### Errors: no errors raised; returns None when nothing is
    registered. Assert None for an empty single-cardinality point.
    """
    mgr = ExtensionManager()
    assert mgr.get("acl") is None


def test_extension_system_get_property_pure_true() -> None:
    """extension_system.get.property.pure.true

    Spec ### Properties: pure=true. Calling get() twice on the same state must
    not mutate the manager: the registered extension and the full point list
    are unchanged across repeated queries.
    """
    mgr = ExtensionManager()
    disc = StubDiscoverer()
    mgr.register("discoverer", disc)

    before_points = {p.name for p in mgr.list_points()}
    first = mgr.get("discoverer")
    second = mgr.get("discoverer")
    after_points = {p.name for p in mgr.list_points()}

    assert first is disc
    assert second is disc
    assert before_points == after_points
    # State for other points is untouched by the query.
    assert mgr.get_all("middleware") == []


def test_extension_system_get_property_thread_safe_concurrent_reads() -> None:
    """extension_system.get.property.thread_safe.concurrent_reads

    Spec ### Properties: thread_safe=true. Launch >=8 concurrent get() calls
    via asyncio.gather and assert no exception escapes and every call observes
    the same consistent registered value.
    """
    import asyncio

    mgr = ExtensionManager()
    disc = StubDiscoverer()
    mgr.register("discoverer", disc)

    async def reader() -> Any:
        return mgr.get("discoverer")

    async def run() -> list[Any]:
        return await asyncio.gather(*[reader() for _ in range(16)])

    results = asyncio.run(run())
    assert len(results) == 16
    assert all(r is disc for r in results)


# ===========================================================================
# Contract: ExtensionManager.get_all
# ===========================================================================


def test_extension_system_get_all_property_async_false() -> None:
    """extension_system.get_all.property.async.false

    Spec ### Properties: async=false. get_all() returns a list synchronously.
    """
    import inspect

    mgr = ExtensionManager()
    result = mgr.get_all("middleware")
    assert isinstance(result, list)
    assert not inspect.isawaitable(result)
    assert not inspect.iscoroutinefunction(mgr.get_all)


def test_extension_system_get_all_returns_empty_when_unregistered() -> None:
    """extension_system.get_all.error.no_error_returns_empty

    Spec ### Errors: no errors raised; returns empty list when nothing is
    registered. Assert [] for an empty multi-cardinality point.
    """
    mgr = ExtensionManager()
    assert mgr.get_all("middleware") == []


def test_extension_system_get_all_returns_registration_order() -> None:
    """extension_system.get_all.returns.registration_order

    Spec ### Returns: list of all registered extensions in registration order.
    """
    mgr = ExtensionManager()
    mw1 = StubMiddleware()
    mw2 = StubMiddleware()
    mw3 = StubMiddleware()
    mgr.register("middleware", mw1)
    mgr.register("middleware", mw2)
    mgr.register("middleware", mw3)
    assert mgr.get_all("middleware") == [mw1, mw2, mw3]


def test_extension_system_get_all_property_pure_true() -> None:
    """extension_system.get_all.property.pure.true

    Spec ### Properties: pure=true. get_all() must not mutate internal state,
    and the returned list must be a copy: mutating it must not affect a
    subsequent query.
    """
    mgr = ExtensionManager()
    mw1 = StubMiddleware()
    mgr.register("middleware", mw1)

    first = mgr.get_all("middleware")
    first.append(StubMiddleware())  # mutate the returned list

    second = mgr.get_all("middleware")
    assert second == [mw1]  # internal store unaffected


def test_extension_system_get_all_property_thread_safe_concurrent_reads() -> None:
    """extension_system.get_all.property.thread_safe.concurrent_reads

    Spec ### Properties: thread_safe=true. Launch >=8 concurrent get_all()
    reads with distinct local handling; assert no exception and a consistent
    snapshot for every call.
    """
    import asyncio

    mgr = ExtensionManager()
    mw1 = StubMiddleware()
    mw2 = StubMiddleware()
    mgr.register("middleware", mw1)
    mgr.register("middleware", mw2)

    async def reader() -> list[Any]:
        return mgr.get_all("middleware")

    async def run() -> list[list[Any]]:
        return await asyncio.gather(*[reader() for _ in range(12)])

    results = asyncio.run(run())
    assert len(results) == 12
    assert all(r == [mw1, mw2] for r in results)


# ===========================================================================
# Contract: ExtensionManager.unregister
# ===========================================================================


def test_extension_system_unregister_property_async_false() -> None:
    """extension_system.unregister.property.async.false

    Spec ### Properties: async=false. unregister() resolves synchronously.
    """
    import inspect

    mgr = ExtensionManager()
    mw = StubMiddleware()
    mgr.register("middleware", mw)
    result = mgr.unregister("middleware", mw)
    assert not inspect.isawaitable(result)
    assert not inspect.iscoroutinefunction(mgr.unregister)


def test_extension_system_unregister_removes_extension() -> None:
    """extension_system.unregister.removes.identity

    Spec ### Inputs: removes the exact extension object (identity). After
    unregister the extension is no longer returned by get_all().

    ``StubMiddleware`` defines no ``__eq__``, so ``==`` on these instances IS
    identity and this scenario cannot tell the two apart. See
    ``test_..._removes_by_identity_not_equality`` below for the one that can.
    """
    mgr = ExtensionManager()
    mw1 = StubMiddleware()
    mw2 = StubMiddleware()
    mgr.register("middleware", mw1)
    mgr.register("middleware", mw2)
    mgr.unregister("middleware", mw1)
    survivors = mgr.get_all("middleware")
    assert len(survivors) == 1
    assert survivors[0] is mw2


def test_extension_system_unregister_removes_by_identity_not_equality() -> None:
    """D-128 — removal is by ``is``, not ``==``.

    This used ``list.remove``, which compares with ``__eq__``. Any extension
    type that defines equality — a dataclass middleware, for one — made
    ``unregister(b)`` delete ``a``: a host removing the second of two
    identically-configured middlewares kept the one it wanted gone and lost the
    one it wanted kept, and nothing said so. apcore-typescript compares with
    ``===`` and apcore-rust by pointer address; this contract's own Inputs row
    says "identity comparison".

    The two registrations MUST be ``==`` and MUST NOT be ``is``, or the test
    cannot distinguish the two semantics — which is exactly why the test above
    could not.
    """
    from dataclasses import dataclass

    @dataclass
    class EqualMiddleware(Middleware):
        label: str = "same"

        async def before(self, *args: object, **kwargs: object) -> None:
            return None

        async def after(self, *args: object, **kwargs: object) -> None:
            return None

    first = EqualMiddleware()
    second = EqualMiddleware()
    assert first == second, "precondition: the two registrations compare equal"
    assert first is not second, "precondition: they are distinct objects"

    mgr = ExtensionManager()
    mgr.register("middleware", first)
    mgr.register("middleware", second)

    assert mgr.unregister("middleware", second) is True
    survivors = mgr.get_all("middleware")
    assert len(survivors) == 1
    assert survivors[0] is first, "unregister removed the wrong object"


def test_extension_system_unregister_an_equal_but_unregistered_object_is_a_no_op() -> None:
    """Control for D-128: equality alone must not authorise a removal.

    Without this, "removal is by identity" would also be satisfied by an
    implementation that removed the LAST equal entry rather than the first —
    still wrong, and still passing the test above by accident when only one
    registration exists.
    """
    from dataclasses import dataclass

    @dataclass
    class EqualMiddleware(Middleware):
        label: str = "same"

        async def before(self, *args: object, **kwargs: object) -> None:
            return None

        async def after(self, *args: object, **kwargs: object) -> None:
            return None

    registered = EqualMiddleware()
    never_registered = EqualMiddleware()
    assert registered == never_registered

    mgr = ExtensionManager()
    mgr.register("middleware", registered)

    assert mgr.unregister("middleware", never_registered) is False
    assert mgr.get_all("middleware")[0] is registered


def test_extension_system_unregister_missing_is_silent_no_op() -> None:
    """extension_system.unregister.error.missing_is_silent_no_op

    Spec ### Errors: no error if the extension is not found (silent no-op).
    Unregistering an extension that was never registered must not raise and
    must leave existing state intact.
    """
    mgr = ExtensionManager()
    mw1 = StubMiddleware()
    never = StubMiddleware()
    mgr.register("middleware", mw1)

    result = mgr.unregister("middleware", never)  # silent no-op
    assert result is False
    assert mgr.get_all("middleware") == [mw1]


def test_extension_system_unregister_property_pure_false_mutates() -> None:
    """extension_system.unregister.property.pure.false

    Spec ### Properties: pure=false (mutates extension store). Unregister must
    produce an observable state change via the public get_all() query.
    """
    mgr = ExtensionManager()
    mw = StubMiddleware()
    mgr.register("middleware", mw)
    assert mgr.get_all("middleware") == [mw]
    mgr.unregister("middleware", mw)
    assert mgr.get_all("middleware") == []


# ===========================================================================
# Contract: ExtensionManager.apply
# ===========================================================================


def _fresh_targets() -> tuple[MagicMock, MagicMock]:
    """Build registry/executor mocks with an empty middleware chain."""
    registry = MagicMock()
    executor = MagicMock()
    executor.middlewares = []
    return registry, executor


def test_extension_system_apply_property_async_false() -> None:
    """extension_system.apply.property.async.false

    Spec ### Properties: async=false. apply() returns None synchronously.
    """
    import inspect

    mgr = ExtensionManager()
    registry, executor = _fresh_targets()
    result = mgr.apply(registry, executor)
    assert result is None
    assert not inspect.iscoroutinefunction(mgr.apply)


def test_extension_system_apply_side_effect_1_set_discoverer() -> None:
    """extension_system.apply.side_effect.1.set_discoverer

    Spec ### Side Effects (1): registry.set_discoverer(ext) if discoverer
    registered. Observe via the registry mock call.
    """
    mgr = ExtensionManager()
    disc = StubDiscoverer()
    mgr.register("discoverer", disc)
    registry, executor = _fresh_targets()
    mgr.apply(registry, executor)
    registry.set_discoverer.assert_called_once_with(disc)


def test_extension_system_apply_side_effect_2_set_validator() -> None:
    """extension_system.apply.side_effect.2.set_validator

    Spec ### Side Effects (2): registry.set_validator(ext) if module_validator
    registered.
    """
    mgr = ExtensionManager()
    val = StubValidator()
    mgr.register("module_validator", val)
    registry, executor = _fresh_targets()
    mgr.apply(registry, executor)
    registry.set_validator.assert_called_once_with(val)


def test_extension_system_apply_side_effect_3_set_acl() -> None:
    """extension_system.apply.side_effect.3.set_acl

    Spec ### Side Effects (3): executor.set_acl(ext) if acl registered.
    """
    mgr = ExtensionManager()
    acl = ACL(rules=[])
    mgr.register("acl", acl)
    registry, executor = _fresh_targets()
    mgr.apply(registry, executor)
    executor.set_acl.assert_called_once_with(acl)


def test_extension_system_apply_side_effect_4_set_approval_handler() -> None:
    """extension_system.apply.side_effect.4.set_approval_handler

    Spec ### Side Effects (4): executor.set_approval_handler(ext) if
    approval_handler registered. Build a stub satisfying the runtime-checkable
    ApprovalHandler protocol (check_approval + request_approval).
    """

    class StubApprovalHandler:
        def check_approval(self, *args: Any, **kwargs: Any) -> Any:
            return None

        def request_approval(self, *args: Any, **kwargs: Any) -> Any:
            return None

    mgr = ExtensionManager()
    handler = StubApprovalHandler()
    mgr.register("approval_handler", handler)
    registry, executor = _fresh_targets()
    mgr.apply(registry, executor)
    executor.set_approval_handler.assert_called_once_with(handler)


def test_extension_system_apply_side_effect_5_use_middleware_in_order() -> None:
    """extension_system.apply.side_effect.5.use_middleware_in_order

    Spec ### Side Effects (5): executor.use(mw) for each middleware in
    registration order. Assert both ORDER and that each was appended.
    """
    mgr = ExtensionManager()
    mw1 = StubMiddleware()
    mw2 = StubMiddleware()
    mgr.register("middleware", mw1)
    mgr.register("middleware", mw2)
    registry, executor = _fresh_targets()
    mgr.apply(registry, executor)

    used = [c.args[0] for c in executor.use.call_args_list]
    assert used == [mw1, mw2]


def test_extension_system_apply_side_effect_6_single_span_exporter_direct() -> None:
    """extension_system.apply.side_effect.6.single_span_exporter_direct

    Spec ### Side Effects (6): locate TracingMiddleware; a single exporter is
    set directly. Observe the exporter wired onto a real TracingMiddleware.
    """
    mgr = ExtensionManager()
    exporter = StubExporter()
    mgr.register("span_exporter", exporter)

    tracing_mw = TracingMiddleware(exporter=InMemoryExporter())
    registry = MagicMock()
    executor = MagicMock()
    executor.middlewares = [tracing_mw]
    mgr.apply(registry, executor)

    assert tracing_mw._exporter is exporter


def test_extension_system_apply_side_effect_6_multiple_span_exporters_composite() -> None:
    """extension_system.apply.side_effect.6.multiple_span_exporters_composite

    Spec ### Side Effects (6): multiple exporters wrapped in CompositeExporter
    that delegates to all underlying exporters; a failure in one MUST NOT stop
    the others. Assert composition AND error-isolated fan-out.
    """
    mgr = ExtensionManager()

    class FailingExporter:
        def export(self, span: Any) -> None:
            raise RuntimeError("boom")

    failing = FailingExporter()
    good = StubExporter()
    mgr.register("span_exporter", failing)
    mgr.register("span_exporter", good)

    tracing_mw = TracingMiddleware(exporter=InMemoryExporter())
    registry = MagicMock()
    executor = MagicMock()
    executor.middlewares = [tracing_mw]
    mgr.apply(registry, executor)

    composite = tracing_mw._exporter
    assert composite is not failing and composite is not good
    assert hasattr(composite, "_exporters")
    assert composite._exporters == [failing, good]

    # Error isolation: failing exporter raises, good exporter still receives.
    sentinel = object()
    composite.export(sentinel)
    assert good.exported == [sentinel]


def test_extension_system_apply_side_effect_6_no_tracing_middleware_no_raise() -> None:
    """extension_system.apply.side_effect.6.no_tracing_middleware_no_raise

    Spec ### Errors lists `ExtensionApplyError` for a span exporter wired with
    no TracingMiddleware present. The Python SDK does NOT raise here; it logs a
    warning and leaves the executor otherwise wired. Assert the actual Python
    behavior: apply() completes without raising and no exporter wiring occurs.
    """
    mgr = ExtensionManager()
    mgr.register("span_exporter", StubExporter())
    registry, executor = _fresh_targets()  # empty middleware chain
    # Must not raise despite the spec's ExtensionApplyError clause.
    mgr.apply(registry, executor)
    # No TracingMiddleware -> nothing to wire the exporter onto.
    assert executor.middlewares == []


def test_extension_system_apply_property_idempotent_false_stacks_middleware() -> None:
    """extension_system.apply.property.idempotent.false

    Spec ### Properties: idempotent=false (calling apply twice stacks
    middleware and re-wires other extensions). Apply twice and assert the
    middleware is used once per apply call (stacking), not deduplicated.
    """
    mgr = ExtensionManager()
    mw = StubMiddleware()
    mgr.register("middleware", mw)
    registry, executor = _fresh_targets()

    mgr.apply(registry, executor)
    mgr.apply(registry, executor)

    used = [c.args[0] for c in executor.use.call_args_list]
    assert used == [mw, mw]


def test_extension_system_apply_side_effects_ordered_sequence() -> None:
    """extension_system.apply.side_effect.ordered.full_sequence

    Spec ### Side Effects (ordered): discoverer -> module_validator -> acl ->
    approval_handler -> middleware chain -> span exporters. Register one of
    each and assert the cross-target call ORDER using a shared recorder.
    """

    class StubApprovalHandler:
        def check_approval(self, *args: Any, **kwargs: Any) -> Any:
            return None

        def request_approval(self, *args: Any, **kwargs: Any) -> Any:
            return None

    order: list[str] = []

    mgr = ExtensionManager()
    mgr.register("discoverer", StubDiscoverer())
    mgr.register("module_validator", StubValidator())
    mgr.register("acl", ACL(rules=[]))
    mgr.register("approval_handler", StubApprovalHandler())
    mgr.register("middleware", StubMiddleware())

    registry = MagicMock()
    executor = MagicMock()
    executor.middlewares = []
    registry.set_discoverer.side_effect = lambda *_: order.append("discoverer")
    registry.set_validator.side_effect = lambda *_: order.append("module_validator")
    executor.set_acl.side_effect = lambda *_: order.append("acl")
    executor.set_approval_handler.side_effect = lambda *_: order.append("approval_handler")
    executor.use.side_effect = lambda *_: order.append("middleware")

    mgr.apply(registry, executor)

    assert order == [
        "discoverer",
        "module_validator",
        "acl",
        "approval_handler",
        "middleware",
    ]
