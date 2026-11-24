"""Drive `event_naming.json` — canonical event names (§9.16, Issue #36 / D-34).

All seven cases assert the canonical `apcore.<subsystem>.<event>` naming and
glob-subscription behaviour.

Two earlier cases — ``legacy_dual_emit`` and ``legacy_health_dual_emit`` —
required the SDK to *also* emit the legacy unprefixed names with
``data.deprecated: true``. That was the v0.21.x deprecation-window rule;
v0.22.0 ended it, and apcore#78 removed dual-emission. Satisfying them would
have violated a current MUST, so they were driven here under a strict xfail
until the spec repo replaced them with ``legacy_names_are_not_emitted`` — the
inverse assertion. Pinning the removal is worth more than deleting the cases:
a deleted case cannot notice dual-emission coming back.

Payload note: the fixture's ``data_contains.module_id`` lives in the event
*envelope* (``ApCoreEvent.module_id``) in apcore-python, not inside ``data``.
:func:`_assert_data_contains` accepts either location for that one key and
requires an exact ``data`` match for every other key. ``data_at_least`` is the
inequality form, used for values the spec deliberately leaves implementation-
defined (see :func:`_assert_data_at_least`).

``forbidden_event_types`` note: the spec repo redistributed these so that each
forbidden name sits on the case whose trigger can actually emit it — the
registry legacy names on ``legacy_names_are_not_emitted`` (trigger
``registry.register``), the health legacy names on
``health_threshold_canonical`` (trigger ``platform_notify.*``). Both drivers now
read the list from the fixture instead of hard-coding it, and
:func:`_assert_not_emitted` refuses an empty list so a spec-side deletion
surfaces as a failure rather than as a silently vacuous assertion.
"""

from __future__ import annotations

import asyncio
from typing import Any

import pytest

from apcore.events.emitter import ApCoreEvent, EventEmitter
from apcore.events.subscribers import FilterSubscriber
from apcore.middleware.platform_notify import PlatformNotifyMiddleware
from apcore.observability.metrics import MetricsCollector
from apcore.registry.registry import Registry
from apcore.sys_modules.registration import _bridge_registry_events

from .canonical_fixtures import load_fixture

FIXTURE = load_fixture("event_naming.json")
CASES: dict[str, dict[str, Any]] = {tc["id"]: tc for tc in FIXTURE["test_cases"]}


class _RecordingEmitter:
    """Captures every emitted event instead of fanning out to a thread pool."""

    def __init__(self) -> None:
        self.events: list[ApCoreEvent] = []

    def emit(self, event: ApCoreEvent) -> None:
        self.events.append(event)


# ---------------------------------------------------------------------------
# Triggers named by the fixture
# ---------------------------------------------------------------------------


def _registry_trigger(emitter: _RecordingEmitter, action: str, module_id: str) -> None:
    registry = Registry()
    if action == "registry.unregister":
        registry.register_internal(module_id, object())
        _bridge_registry_events(registry, emitter)
        registry.unregister(module_id)
        return
    if action == "registry.register":
        _bridge_registry_events(registry, emitter)
        registry.register_internal(module_id, object())
        return
    pytest.fail(f"unhandled registry trigger action {action!r}")


def _unhealthy_middleware(emitter: _RecordingEmitter, module_id: str) -> PlatformNotifyMiddleware:
    """Middleware whose module sits above both thresholds the fixture names.

    Error rate is exactly 15/100 = 0.15 against a 0.10 threshold, so the fixture
    asserts that number verbatim through ``data_contains``.

    The p99 is asserted as ``data_at_least`` instead: every SDK estimates p99
    from histogram BUCKET BOUNDS and the bucket layout is not fixed by the spec,
    so apcore-rust's default buckets (no 6.0 s boundary) report 10000.0 for the
    same 6 s sample. The single 6.0 s bucket used here puts Python's estimate at
    6000.0 ms, above the 5000.0 ms threshold either way; the canonical event
    NAME is what this case exists to pin.
    """
    metrics = MetricsCollector(buckets=[6.0])
    for _ in range(85):
        metrics.increment("apcore_module_calls_total", {"module_id": module_id, "status": "success"})
    for _ in range(15):
        metrics.increment("apcore_module_calls_total", {"module_id": module_id, "status": "error"})
        metrics.increment("apcore_module_errors_total", {"module_id": module_id, "error_code": "E"})
    for _ in range(100):
        metrics.observe("apcore_module_duration_seconds", {"module_id": module_id}, 6.0)
    return PlatformNotifyMiddleware(
        event_emitter=emitter,  # type: ignore[arg-type]
        metrics_collector=metrics,
        error_rate_threshold=0.10,
        latency_p99_threshold_ms=5000.0,
    )


def _run_trigger(
    emitter: _RecordingEmitter,
    trigger: dict[str, Any],
    middlewares: dict[str, PlatformNotifyMiddleware],
) -> None:
    """Apply one fixture trigger.

    ``middlewares`` is keyed by module id and shared across a trigger sequence:
    PlatformNotifyMiddleware carries per-module alert hysteresis, so recovery
    can only be observed on the same instance that raised the alert.
    """
    action = trigger["action"]
    module_id = trigger["target_id"]
    if action.startswith("registry."):
        _registry_trigger(emitter, action, module_id)
        return

    middleware = middlewares.setdefault(module_id, _unhealthy_middleware(emitter, module_id))
    if action == "platform_notify.error_threshold_crossed":
        middleware.on_error(module_id, {}, RuntimeError("boom"), None)
    elif action == "platform_notify.latency_threshold_crossed":
        middleware.after(module_id, {}, {}, None)
    elif action == "platform_notify.recovered":
        # Recovery fires only for an already-alerted module whose error rate has
        # fallen below half the threshold — swap in a clean metrics view.
        middleware._metrics_collector = MetricsCollector()
        middleware.after(module_id, {}, {}, None)
    else:
        pytest.fail(f"unhandled event_naming trigger action {action!r}")


def _triggers(case: dict[str, Any]) -> list[dict[str, Any]]:
    if "trigger_sequence" in case:
        return list(case["trigger_sequence"])
    return [case["trigger"]]


def _emitted(case: dict[str, Any]) -> list[ApCoreEvent]:
    emitter = _RecordingEmitter()
    middlewares: dict[str, PlatformNotifyMiddleware] = {}
    for trigger in _triggers(case):
        _run_trigger(emitter, trigger, middlewares)
    return emitter.events


def _assert_data_contains(case_id: str, event: ApCoreEvent, expected: dict[str, Any]) -> None:
    for key, value in expected.items():
        if key == "module_id":
            observed = event.data.get("module_id", event.module_id)
        else:
            assert (
                key in event.data
            ), f"[{case_id}] {event.event_type} payload is missing {key!r}; got keys {sorted(event.data)}"
            observed = event.data[key]
        assert observed == value, f"[{case_id}] {event.event_type} payload {key}: got {observed!r}, expected {value!r}"


def _assert_data_at_least(case_id: str, event: ApCoreEvent, expected: dict[str, Any]) -> None:
    """Assert numeric payload fields meet a lower bound rather than an exact value.

    Used for ``p99_latency_ms``: p99 is estimated from histogram bucket bounds
    and the bucket layout is implementation-defined, so an exact match would
    pin apcore-python's bucket choice rather than the spec's requirement.
    """
    for key, floor in expected.items():
        assert (
            key in event.data
        ), f"[{case_id}] {event.event_type} payload is missing {key!r}; got keys {sorted(event.data)}"
        observed = event.data[key]
        assert isinstance(observed, (int, float)) and not isinstance(
            observed, bool
        ), f"[{case_id}] {event.event_type} payload {key}: expected a number, got {observed!r}"
        assert observed >= floor, (
            f"[{case_id}] {event.event_type} payload {key}: got {observed!r}, " f"expected at least {floor!r}"
        )


def _assert_not_emitted(case_id: str, emitted_types: list[str], forbidden: list[str]) -> None:
    """Assert none of *forbidden* was emitted, and that the list is non-empty.

    The empty-list guard is the point: a `forbidden_event_types` entry only
    asserts something when the case's trigger could plausibly emit it, so the
    spec repo moved each name onto the case that can produce it. If a future
    edit drops the list from a case this driver claims to check, that must fail
    here rather than pass vacuously.
    """
    assert forbidden, (
        f"[{case_id}] the fixture no longer lists forbidden_event_types for this case; "
        f"the inverse assertion this driver claims to make would be vacuous"
    )
    leaked = sorted(set(emitted_types) & set(forbidden))
    assert not leaked, (
        f"[{case_id}] these pre-canonical names MUST NOT be emitted but were: {leaked}. " f"Emitted: {emitted_types}"
    )


def _received_types(pattern: str, events: list[ApCoreEvent]) -> list[str]:
    """Event types a `FilterSubscriber` on *pattern* actually delivers."""
    received: list[str] = []

    class _Capture:
        async def on_event(self, event: ApCoreEvent) -> None:
            received.append(event.event_type)

    subscriber = FilterSubscriber(delegate=_Capture(), include_events=[pattern])

    async def _deliver() -> None:
        for event in events:
            await subscriber.on_event(event)

    asyncio.run(_deliver())
    return received


# ---------------------------------------------------------------------------
# Cases
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "case_id",
    ["canonical_module_registered", "canonical_module_unregistered"],
)
def test_canonical_event_emitted(case_id: str) -> None:
    case = CASES[case_id]
    expected = case["expected"]["canonical_event"]
    events = _emitted(case)
    types = [e.event_type for e in events]

    matches = [e for e in events if e.event_type == expected["event_type"]]
    assert matches, f"[{case_id}] expected {expected['event_type']!r} to be emitted; got {types}"
    _assert_data_contains(case_id, matches[0], expected["data_contains"])


def test_legacy_names_are_not_emitted() -> None:
    """The canonical name is emitted and the legacy REGISTRY names are NOT (apcore#78).

    The fixture's forbidden list for this case is now only ``module_registered``
    / ``module_unregistered``. It previously also carried
    ``apcore.error.threshold_exceeded`` / ``apcore.latency.threshold_exceeded``,
    which this case's ``registry.register`` trigger could never emit under any
    implementation — half the assertion was vacuous. Those two moved to
    ``health_threshold_canonical``, whose trigger can actually produce them.
    """
    case = CASES["legacy_names_are_not_emitted"]
    events = _emitted(case)
    types = [e.event_type for e in events]

    for expected in case["expected"]["events"]:
        matches = [e for e in events if e.event_type == expected["event_type"]]
        assert matches, f"[{case['id']}] expected {expected['event_type']!r} to be emitted; got {types}"
        _assert_data_contains(case["id"], matches[0], expected["data_contains"])

    _assert_not_emitted(case["id"], types, case["expected"]["forbidden_event_types"])


def test_health_threshold_canonical() -> None:
    """Threshold events use apcore.health.*, and the retired aliases are gone.

    This is the only fixture case whose trigger can emit
    ``apcore.error.threshold_exceeded`` / ``apcore.latency.threshold_exceeded``,
    so it is where those forbidden names now live. The list is read from the
    fixture rather than hard-coded here, so a spec-side change to the retired
    set reaches this driver on the next run.
    """
    case = CASES["health_threshold_canonical"]
    events = _emitted(case)
    types = [e.event_type for e in events]

    for expected in case["expected"]["events"]:
        matches = [e for e in events if e.event_type == expected["event_type"]]
        assert matches, (
            f"[{case['id']}] threshold events MUST use the apcore.health.* subsystem; "
            f"expected {expected['event_type']!r}, got {types}"
        )
        _assert_data_contains(case["id"], matches[0], expected["data_contains"])
        _assert_data_at_least(case["id"], matches[0], expected.get("data_at_least", {}))

    _assert_not_emitted(case["id"], types, case["expected"]["forbidden_event_types"])


@pytest.mark.parametrize(
    "case_id",
    ["glob_subscription_registry", "glob_subscription_health", "glob_does_not_match_other_subsystem"],
)
def test_glob_subscription(case_id: str) -> None:
    case = CASES[case_id]
    events = _emitted(case)
    received = _received_types(case["subscription_pattern"], events)
    expected = case["expected"]["received_event_types"]

    if expected:
        assert received == expected, (
            f"[{case_id}] subscription {case['subscription_pattern']!r} received {received}, "
            f"expected {expected} (emitted: {[e.event_type for e in events]})"
        )
    else:
        assert received == [], (
            f"[{case_id}] subscription {case['subscription_pattern']!r} must not match "
            f"another subsystem's events, but received {received}"
        )


def test_every_fixture_case_has_a_driver() -> None:
    driven = {
        "canonical_module_registered",
        "canonical_module_unregistered",
        "legacy_names_are_not_emitted",
        "health_threshold_canonical",
        "glob_subscription_registry",
        "glob_subscription_health",
        "glob_does_not_match_other_subsystem",
    }
    assert set(CASES) == driven, (
        f"event_naming.json cases without a driver: {sorted(set(CASES) - driven)}; "
        f"drivers with no matching case: {sorted(driven - set(CASES))}"
    )


def test_emitter_rejects_sync_subscriber() -> None:
    """Guards the delivery model this driver relies on: subscribers are async."""
    with pytest.raises(TypeError):
        EventEmitter().subscribe(type("Sync", (), {"on_event": lambda self, e: None})())
