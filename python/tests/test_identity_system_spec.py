"""Spec-traced contract tests for the Identity System feature.

Source spec: apcore/docs/features/identity-system.md
Contract under test: ``ContextFactory.create_context`` (the only
``## Contract:`` block in the spec).

Each test's docstring carries the verbatim clause id formatted as
``identity_system.<method>.<kind>.<detail>`` so cross-language diffs line up.

The spec's contract block describes ``create_context``: a ``ContextFactory``
extracts an ``Identity`` from a runtime request and returns a ``Context``.
The declared inputs (``identity``, ``caller_id``, ``data``) and the declared
return semantics ("assigned trace ID", "defaults to @external", "generates a
new trace ID on each call") are realized by ``Context.create``, which a
factory delegates to. We therefore exercise the contract through a concrete
factory built on top of ``Context.create`` (the public, observable surface),
plus the ``runtime_checkable`` Protocol conformance.

framework: pytest + pytest-asyncio (asyncio_mode=auto). Properties the spec
marks ``async: false`` stay synchronous; the ``thread_safe`` clause exercises
concurrency via ``asyncio.gather`` per the shared contract-spec rules.

TESTS ONLY — no production source is modified here.
"""

from __future__ import annotations

import asyncio
from typing import Any


from apcore.context import Context, ContextFactory, Identity


class _Request:
    """Minimal runtime request object carrying caller fields."""

    def __init__(
        self,
        *,
        identity: Identity | None = None,
        caller_id: str | None = None,
        data: dict[str, Any] | None = None,
    ) -> None:
        self.identity = identity
        self.caller_id = caller_id
        self.data = data


class _SpecFactory:
    """Concrete ``ContextFactory`` exercising the declared contract inputs.

    Mirrors the spec's documented Django/Express factories: pull the optional
    ``identity`` / ``caller_id`` / ``data`` off the request and delegate to
    ``Context.create`` (which assigns the trace ID).
    """

    def create_context(self, request: Any) -> Context:
        ctx = Context.create(
            identity=getattr(request, "identity", None),
            data=getattr(request, "data", None),
        )
        caller_id = getattr(request, "caller_id", None)
        if caller_id is not None:
            ctx.caller_id = caller_id
        return ctx


# ---------------------------------------------------------------------------
# Inputs — the contract declares NO ``reject_with`` rules: "invalid identity
# fields are sanitized, not rejected". So an "invalid"/absent-style input must
# NOT raise; instead we assert the declared graceful fallback behavior.
# ---------------------------------------------------------------------------


def test_input_identity_absent_defaults_to_external() -> None:
    """identity_system.create_context.input.identity.absent_defaults_to_external."""
    factory = _SpecFactory()
    # Spec Inputs: identity is optional and "defaults to @external when absent".
    # The @external pattern (per the feature spec) is "identity is None".
    ctx = factory.create_context(_Request(identity=None))
    assert ctx.identity is None


def test_input_caller_id_optional_absent_yields_none() -> None:
    """identity_system.create_context.input.caller_id.optional_absent_is_none."""
    factory = _SpecFactory()
    # Spec Inputs: caller_id is optional for call-chain tracking; absent -> None.
    ctx = factory.create_context(_Request(caller_id=None))
    assert ctx.caller_id is None
    # When supplied, it is carried onto the produced Context.
    ctx2 = factory.create_context(_Request(caller_id="api.gateway"))
    assert ctx2.caller_id == "api.gateway"


def test_input_data_optional_absent_yields_empty_payload() -> None:
    """identity_system.create_context.input.data.optional_absent_is_empty."""
    factory = _SpecFactory()
    # Spec Inputs: data is the optional initial context payload; absent -> {}.
    ctx = factory.create_context(_Request(data=None))
    assert ctx.data == {}
    # When supplied, the payload is carried through verbatim.
    ctx2 = factory.create_context(_Request(data={"k": "v"}))
    assert ctx2.data["k"] == "v"


def test_input_identity_sanitized_not_rejected() -> None:
    """identity_system.create_context.input.identity.sanitized_not_rejected."""
    factory = _SpecFactory()
    # Spec Inputs/Errors: "invalid identity fields are sanitized, not rejected".
    # A non-dict attrs is sanitized to {} by Identity.__post_init__ rather than
    # raising; create_context must accept the resulting identity without error.
    ident = Identity(id="svc-1", type="service", attrs="not-a-dict")  # type: ignore[arg-type]
    assert ident.attrs == {}
    ctx = factory.create_context(_Request(identity=ident))
    assert ctx.identity is ident
    assert ctx.identity.attrs == {}


# ---------------------------------------------------------------------------
# Errors — the contract declares NONE ("No errors raised"). We assert the
# absence of raises across the input surface so a future regression that adds a
# raise is caught.
# ---------------------------------------------------------------------------


def test_error_none_no_raise_on_any_optional_input() -> None:
    """identity_system.create_context.error.none.no_error_raised."""
    factory = _SpecFactory()
    # Every optional-input combination must complete without raising.
    factory.create_context(_Request())
    factory.create_context(_Request(identity=None, caller_id=None, data=None))
    ident = Identity(id="u-1", type="user", roles=("admin",), attrs={"d": "eng"})
    ctx = factory.create_context(_Request(identity=ident, caller_id="orchestrator.run", data={"x": 1}))
    # And it still produced a usable Context (proves we reached the return, not
    # an early raise).
    assert isinstance(ctx, Context)
    assert ctx.identity is ident


# ---------------------------------------------------------------------------
# Returns — "Context with assigned trace ID and caller identity".
# ---------------------------------------------------------------------------


def test_returns_context_with_assigned_trace_id() -> None:
    """identity_system.create_context.returns.context.assigned_trace_id."""
    factory = _SpecFactory()
    ident = Identity(id="admin@example.com", type="user", roles=("admin",))
    ctx = factory.create_context(_Request(identity=ident))
    assert isinstance(ctx, Context)
    # A non-empty trace_id is assigned, and the caller identity is attached.
    assert isinstance(ctx.trace_id, str) and ctx.trace_id
    assert ctx.identity is ident
    assert ctx.identity.id == "admin@example.com"
    assert ctx.identity.roles == ("admin",)


def test_returns_identity_propagates_to_child_context() -> None:
    """identity_system.create_context.returns.context.identity_propagates_to_child."""
    factory = _SpecFactory()
    ident = Identity(id="admin@example.com", type="user", roles=("admin", "operator"))
    ctx = factory.create_context(_Request(identity=ident))
    # Spec requirement: Identity propagates to child contexts by identity.
    child = ctx.child("target.module")
    assert child.identity is ctx.identity
    assert child.trace_id == ctx.trace_id


# ---------------------------------------------------------------------------
# Properties — async:false, thread_safe:true, pure:false, idempotent:false.
# ---------------------------------------------------------------------------


def test_property_async_false_create_context_is_synchronous() -> None:
    """identity_system.create_context.property.async.synchronous_not_awaitable."""
    factory = _SpecFactory()
    # Spec Properties: async: false. Result is a plain Context, not a coroutine.
    result = factory.create_context(_Request())
    assert not asyncio.iscoroutine(result)
    assert isinstance(result, Context)


async def test_property_thread_safe_concurrent_distinct_inputs() -> None:
    """identity_system.create_context.property.thread_safe.concurrent_distinct_inputs."""
    factory = _SpecFactory()
    n = 16

    async def make(i: int) -> Context:
        ident = Identity(id=f"user-{i}", type="user", roles=(f"role-{i}",))
        return factory.create_context(_Request(identity=ident, caller_id=f"caller-{i}"))

    contexts = await asyncio.gather(*(make(i) for i in range(n)))
    # No panic/exception; each call produced its own consistent Context.
    assert len(contexts) == n
    for i, ctx in enumerate(contexts):
        assert ctx.identity is not None
        assert ctx.identity.id == f"user-{i}"
        assert ctx.caller_id == f"caller-{i}"
    # Spec Properties: a new trace ID per call -> all trace_ids are distinct.
    trace_ids = {ctx.trace_id for ctx in contexts}
    assert len(trace_ids) == n


def test_property_idempotent_false_new_trace_id_each_call() -> None:
    """identity_system.create_context.property.idempotent.distinct_trace_id_per_call."""
    factory = _SpecFactory()
    ident = Identity(id="admin@example.com", type="user")
    req = _Request(identity=ident)
    # Spec Properties: idempotent: false — "generates a new trace ID on each
    # call". Two calls with identical input yield distinct trace IDs.
    first = factory.create_context(req)
    second = factory.create_context(req)
    assert first.trace_id != second.trace_id
    # Identity is still attached identically (only the trace ID differs).
    assert first.identity is second.identity is ident


def test_property_pure_false_distinct_context_objects_per_call() -> None:
    """identity_system.create_context.property.pure.not_pure_fresh_context_each_call."""
    factory = _SpecFactory()
    ident = Identity(id="svc", type="service")
    # Spec Properties: pure: false — the call is observably non-deterministic in
    # its output (a new trace ID per call) and yields a *fresh* Context object
    # each time. Use independent request payloads so each call's data is its own
    # object (Context.create does not copy a supplied data dict by design).
    a = factory.create_context(_Request(identity=ident, data={"shared": 1}))
    b = factory.create_context(_Request(identity=ident, data={"shared": 1}))
    # Distinct Context instances with distinct data objects and distinct trace.
    assert a is not b
    assert a.data is not b.data
    assert a.trace_id != b.trace_id
    # The same input identity object is attached to both (only trace ID varies).
    assert a.identity is b.identity is ident


# ---------------------------------------------------------------------------
# Protocol conformance — ContextFactory is a runtime_checkable Protocol; the
# spec requires implementations expose ``create_context``.
# ---------------------------------------------------------------------------


def test_protocol_runtime_checkable_factory_conforms() -> None:
    """identity_system.create_context.property.protocol.runtime_checkable_conformance."""
    factory = _SpecFactory()
    # Spec ContextFactory Protocol is @runtime_checkable: a class exposing
    # create_context isinstance-matches; one lacking it does not.
    assert isinstance(factory, ContextFactory)

    class _NotAFactory:
        pass

    assert not isinstance(_NotAFactory(), ContextFactory)
