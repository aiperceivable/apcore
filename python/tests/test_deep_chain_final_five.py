"""The last five deep-chain decisions, pinned per SDK.

D-76, D-78, D-80 and D-106 were each implemented here and pinned only in
apcore-rust — the fourth, fifth and sixth instance of the shape this audit kept
finding: a decision's authority is the SDK least likely to be covered, because
the SDKs that had to CHANGE got tests as part of the change.

D-104 is already covered in all three (see ``test_deep_chain_v150.py``) and is
linked rather than rewritten.
"""

from __future__ import annotations

import inspect
from typing import Any

import pytest

# ---------------------------------------------------------------------------
# D-76 — `ContextFactory.create_context(request)`
# ---------------------------------------------------------------------------


class TestContextFactoryTakesARequest:
    """The Contract declared ``(identity, caller_id, data)``; no SDK took that.

    The decision rewrote it to ``create_context(request) -> Context`` because
    that is what the interface is FOR: a web-framework integration exists to
    extract an identity *from a request*, and a signature that already receives
    an ``Identity`` has had that work done for it and has nothing left to do.
    apcore-rust's own doc comment quoted the spec's
    ``ContextFactory.create_context(request)`` while its signature took no
    request — the shape the decision names.
    """

    def test_the_published_signature_takes_exactly_one_request(self) -> None:
        """A signature assertion, because the decision IS about the signature.

        Goes red if a parameter is added, renamed, or if the protocol reverts to
        receiving an already-built ``Identity``.
        """
        from apcore.context import ContextFactory

        params = list(inspect.signature(ContextFactory.create_context).parameters)
        assert params == ["self", "request"], params

    def test_the_factory_reads_the_identity_out_of_the_request(self) -> None:
        """The behavioural half: the FACTORY extracts, not the protocol."""
        from apcore.context import Context, ContextFactory, Identity

        class FakeRequest:
            def __init__(self, user_id: str | None, roles: list[str]) -> None:
                self.user_id = user_id
                self.roles = roles

        class WebFactory:
            def create_context(self, request: Any) -> Context:
                if request.user_id is None:
                    return Context.create()
                return Context.create(identity=Identity(id=request.user_id, type="user", roles=request.roles))

        factory: ContextFactory = WebFactory()

        ctx = factory.create_context(FakeRequest("u-7", ["viewer"]))
        assert ctx.identity is not None
        assert ctx.identity.id == "u-7"
        assert ctx.identity.roles == ("viewer",)

    def test_control_an_unauthenticated_request_yields_no_identity(self) -> None:
        """Without this, "the identity came from the request" is also satisfied
        by a factory that manufactures one regardless of what it was handed.

        A null identity stays null (D-103) — ``@external`` is the caller-side
        ACL sentinel, not a principal.
        """
        from apcore.context import Context

        class FakeRequest:
            user_id = None
            roles: list[str] = []

        class WebFactory:
            def create_context(self, request: Any) -> Context:
                if request.user_id is None:
                    return Context.create()
                raise AssertionError("unreachable in this test")

        assert WebFactory().create_context(FakeRequest()).identity is None


# ---------------------------------------------------------------------------
# D-78 — `ExtensionManager.apply` must not drain the store
# ---------------------------------------------------------------------------


class TestApplyRetainsTheStore:
    """No Postconditions section existed, and apcore-rust consumed its
    registrations. Applying one manager to two executors wired both here and
    only the first there, silently.

    The decision cites the block's own existing ``idempotent: false`` row as
    decisive: it promises that calling ``apply`` twice STACKS middleware, and
    only a non-consuming implementation can produce that observable. The spec
    had already chosen; it just had not said so where an implementer looks.
    """

    @staticmethod
    def _manager_with_one_middleware():
        from apcore.extensions import ExtensionManager
        from apcore.middleware import Middleware

        class Probe(Middleware):
            pass

        mgr = ExtensionManager()
        probe = Probe()
        mgr.register("middleware", probe)
        return mgr, probe

    @staticmethod
    def _executor():
        from apcore.executor import Executor
        from apcore.registry.registry import Registry

        return Executor(registry=Registry())

    def test_the_store_survives_apply(self) -> None:
        from apcore.registry.registry import Registry

        mgr, probe = self._manager_with_one_middleware()
        mgr.apply(Registry(), self._executor())

        assert mgr.get_all("middleware") == [probe]

    def test_one_manager_wires_two_executors(self) -> None:
        """The consequence the decision is about: a drained store wires the
        first executor and silently leaves the second bare."""
        from apcore.registry.registry import Registry

        mgr, probe = self._manager_with_one_middleware()

        first = self._executor()
        second = self._executor()
        mgr.apply(Registry(), first)
        mgr.apply(Registry(), second)

        assert probe in first.middlewares
        assert probe in second.middlewares

    def test_applying_twice_to_one_executor_stacks(self) -> None:
        """``idempotent: false`` — the row that settled the decision.

        This is the assertion a consuming implementation cannot satisfy: the
        second apply has nothing left to wire.
        """
        from apcore.registry.registry import Registry

        mgr, probe = self._manager_with_one_middleware()
        executor = self._executor()

        mgr.apply(Registry(), executor)
        mgr.apply(Registry(), executor)

        assert [m for m in executor.middlewares if m is probe] == [probe, probe]

    def test_control_an_empty_manager_wires_nothing(self) -> None:
        """Without this, "both executors have the middleware" is also satisfied
        by an executor that arrives with middleware of its own."""
        from apcore.extensions import ExtensionManager
        from apcore.registry.registry import Registry

        executor = self._executor()
        before = list(executor.middlewares)
        ExtensionManager().apply(Registry(), executor)

        assert list(executor.middlewares) == before


# ---------------------------------------------------------------------------
# D-80 — the registry event set is closed
# ---------------------------------------------------------------------------


class TestRegistryEventSetIsClosed:
    """``on``/``off`` MUST reject anything outside the set, and MUST accept
    everything the same implementation can emit.

    The second half is the one that caught the original defect:
    apcore-typescript's ``watch()`` emitted ``file_changed`` while its own
    ``on()`` rejected that name, so every hot-reload notification fired into an
    empty callback list. ``file_changed`` is CONDITIONAL — emitted only by an
    implementation whose ``watch()`` is notify-only — and this SDK's is not, so
    it neither emits nor accepts it.
    """

    @staticmethod
    def _registry():
        from apcore.registry.registry import Registry

        return Registry()

    def test_the_two_universal_events_are_accepted(self) -> None:
        reg = self._registry()
        reg.on("register", lambda *a, **k: None)
        reg.on("unregister", lambda *a, **k: None)

    @pytest.mark.parametrize("name", ["change", "add", "remove", "file_changed", "regsiter", ""])
    def test_everything_outside_the_set_is_rejected(self, name: str) -> None:
        """``change``/``add``/``remove`` are the three names registry-system.md's
        own example told readers to use, and every SDK rejects them; the typo
        stands for the silent-subscription failure the decision forbids."""
        from apcore.errors import InvalidInputError

        reg = self._registry()
        with pytest.raises(InvalidInputError):
            reg.on(name, lambda *a, **k: None)

    def test_every_event_this_sdk_emits_is_one_on_accepts(self) -> None:
        """The invariant that catches the real defect.

        Rather than listing names twice, this reads the emit-side registry and
        requires `on()` to accept each key. An implementation that starts
        emitting a new event without opening `on()` to it goes red here instead
        of firing into an empty callback list.
        """
        reg = self._registry()
        emitted = list(reg._callbacks.keys())
        assert emitted, "the registry must have an emit-side event set"

        for name in emitted:
            reg.on(name, lambda *a, **k: None)

    def test_off_answers_on_the_same_set(self) -> None:
        from apcore.errors import InvalidInputError

        reg = self._registry()

        def cb(*a: Any, **k: Any) -> None:
            return None

        reg.on("register", cb)
        assert reg.off("register", cb) is True
        with pytest.raises(InvalidInputError):
            reg.off("file_changed", cb)


# ---------------------------------------------------------------------------
# D-106 — a p99 beyond the largest bucket is that bucket, not zero
# ---------------------------------------------------------------------------


class TestP99FallsBackToTheLargestBucket:
    """Returning ``0.0`` reports the FASTEST possible latency for the SLOWEST
    modules, so a latency alert can never fire for a module slower than the top
    bucket. apcore-rust had pinned the wrong behaviour in a unit test, which is
    why its suite stayed green.
    """

    @staticmethod
    def _collector_with(duration_seconds: float, module_id: str):
        from apcore.observability.metrics import MetricsCollector

        metrics = MetricsCollector()
        for _ in range(5):
            metrics.observe_duration(module_id, duration_seconds)
        return metrics

    def test_every_observation_overflowing_reports_the_top_bucket(self) -> None:
        from apcore.observability.metrics import MetricsCollector, module_latency_ms

        top = MetricsCollector.DEFAULT_BUCKETS[-1]
        metrics = self._collector_with(top * 2, "executor.d106.slow")

        _avg, p99 = module_latency_ms(metrics, "executor.d106.slow")
        assert p99 == pytest.approx(
            top * 1000.0, abs=1.0
        ), f"p99 beyond the top bucket must report the largest finite bound, got {p99}"

    def test_control_no_data_still_reports_zero(self) -> None:
        """Zero must still mean "no data". Without this, the fix could be a
        blanket "always return the top bucket", which reports the SLOWEST
        possible latency for a module that was never called.

        Asserted on the ESTIMATOR as well as through the wrapper: the wrapper
        returns early when the module has no histogram entry at all, so it
        never reaches the estimator's own zero-count guard. Measured — deleting
        that guard leaves the wrapper-only assertion green.
        """
        from apcore.observability.metrics import (
            METRIC_DURATION_SECONDS,
            MetricsCollector,
            estimate_p99_latency_ms,
            module_latency_ms,
        )

        _avg, p99 = module_latency_ms(MetricsCollector(), "executor.d106.never_called")
        assert p99 == 0.0

        assert (
            estimate_p99_latency_ms(
                METRIC_DURATION_SECONDS,
                (("module_id", "executor.d106.never_called"),),
                {},
                0,
            )
            == 0.0
        )

    def test_control_an_in_range_observation_is_not_pushed_to_the_top(self) -> None:
        """The other direction: a fast module must not report the top bucket."""
        from apcore.observability.metrics import MetricsCollector, module_latency_ms

        metrics = self._collector_with(0.02, "executor.d106.fast")
        _avg, p99 = module_latency_ms(metrics, "executor.d106.fast")

        top_ms = MetricsCollector.DEFAULT_BUCKETS[-1] * 1000.0
        assert 0.0 < p99 < top_ms, p99
