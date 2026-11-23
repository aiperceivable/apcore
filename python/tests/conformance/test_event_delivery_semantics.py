"""Conformance tests for event delivery semantics (apcore #61) fixture."""

from __future__ import annotations

import asyncio
import logging
import re
import sys
from typing import Any

import pytest

import apcore.events.emitter as emitter_module
from apcore.events import create_subscriber_from_config
from apcore.events.emitter import ApCoreEvent, EventEmitter, _DLQ_EVENT_TYPE
from apcore.events.retry import EventRetryConfig
from conformance.canonical_fixtures import case_ids, load_fixture

FIXTURE = "event_delivery_semantics.json"


class _SleepRecorder:
    """Stand-in for the emitter module's ``asyncio`` global that records sleeps.

    ``backoff_delays_ms`` is asserted against the delays the emitter actually
    hands to ``asyncio.sleep`` between retry attempts. Timing the gaps between
    deliveries on the wall clock cannot separate a 10 ms backoff from scheduler
    jitter without either going flaky or accepting a tolerance so wide that
    dropping the backoff entirely would still pass.

    Every other attribute proxies through to the real :mod:`asyncio`, so the
    emitter's ``gather`` / ``new_event_loop`` calls are untouched.
    """

    def __init__(self) -> None:
        self.delays_ms: list[int] = []

    def __getattr__(self, name: str) -> Any:
        return getattr(asyncio, name)

    async def sleep(self, delay: float, *args: Any, **kwargs: Any) -> Any:
        self.delays_ms.append(round(delay * 1000))
        return await asyncio.sleep(0)


class _BrokenStdout:
    """A stdout whose writes fail, so a real ``StdoutSubscriber`` fails delivery.

    The fixture's ``fail_attempts: "all"`` applies to a subscriber of type
    ``stdout``; breaking the sink keeps the subscriber under test the SDK's own
    ``StdoutSubscriber`` (including its generated ``subscriber_id``) instead of
    swapping in a stub that would prove nothing about it.
    """

    def write(self, _data: str) -> int:
        raise OSError("stdout is broken")

    def flush(self) -> None:
        raise OSError("stdout is broken")


def _load_fixture() -> dict:
    """Load the canonical fixture from the apcore spec repo."""
    return load_fixture(FIXTURE)


def _make_event(name: str, payload: dict) -> ApCoreEvent:
    return ApCoreEvent(
        event_type=name,
        module_id=None,
        timestamp="2026-01-01T00:00:00Z",
        severity="info",
        data=payload,
    )


class _CountingSubscriber:
    def __init__(
        self,
        subscriber_id: str,
        fail_attempts,
        retry: EventRetryConfig,
        event_pattern: str = "*",
        subscriber_type: str = "counting",
    ) -> None:
        self.subscriber_id = subscriber_id
        self.subscriber_type = subscriber_type
        self.retry = retry
        self.event_pattern = event_pattern
        self._fail_attempts = fail_attempts  # "all" or list of 1-based attempt numbers
        self._call_count = 0

    async def on_event(self, event: ApCoreEvent) -> None:
        self._call_count += 1
        if self._fail_attempts == "all":
            raise RuntimeError(f"Simulated permanent failure (attempt {self._call_count})")
        if isinstance(self._fail_attempts, list) and self._call_count in self._fail_attempts:
            raise RuntimeError(f"Simulated transient failure (attempt {self._call_count})")

    @property
    def call_count(self) -> int:
        return self._call_count


class _DLQRecorder:
    def __init__(self) -> None:
        self.subscriber_id = "dlq-conformance-recorder"
        self.event_pattern = _DLQ_EVENT_TYPE
        self.retry = EventRetryConfig()
        self.received: list[ApCoreEvent] = []

    async def on_event(self, event: ApCoreEvent) -> None:
        self.received.append(event)


@pytest.fixture
def fixture_data() -> dict:
    return _load_fixture()


def _build_retry(retry_cfg: dict) -> EventRetryConfig:
    return EventRetryConfig(
        max_attempts=retry_cfg.get("max_attempts", 3),
        initial_backoff_ms=retry_cfg.get("initial_backoff_ms", 100),
        max_backoff_ms=retry_cfg.get("max_backoff_ms", 30_000),
        backoff_multiplier=retry_cfg.get("backoff_multiplier", 2.0),
    )


def test_fixture_retry_succeeds_before_exhaustion(fixture_data: dict, monkeypatch: pytest.MonkeyPatch) -> None:
    """Case: retry_succeeds_before_exhaustion."""
    case = next(c for c in fixture_data["test_cases"] if c["id"] == "retry_succeeds_before_exhaustion")
    setup = case["setup"]["subscriber"]
    expected = case["expected"]

    retry = _build_retry(setup["retry"])
    fail_attempts = setup["fail_attempts"]

    emitter = EventEmitter()
    dlq = _DLQRecorder()
    emitter.subscribe(dlq)

    sub = _CountingSubscriber(
        subscriber_id=setup["id"],
        fail_attempts=fail_attempts,
        retry=retry,
    )
    emitter.subscribe(sub)

    sleeps = _SleepRecorder()
    monkeypatch.setattr(emitter_module, "asyncio", sleeps)

    trigger = case["trigger"]["event"]
    emitter.emit(_make_event(trigger["name"], trigger.get("payload", {})))
    emitter.flush(timeout=10.0)
    emitter.shutdown()

    assert sub.call_count == expected["attempt_count"]
    assert (len(dlq.received) > 0) == expected["dlq_event_emitted"]
    # One backoff per retry, exponential per the subscriber's retry config:
    # initial_backoff_ms=10, backoff_multiplier=2.0 → [10, 20].
    assert sleeps.delays_ms == expected["backoff_delays_ms"], (
        f"backoff delays: emitter slept {sleeps.delays_ms!r}ms, fixture requires "
        f"{expected['backoff_delays_ms']!r}ms for retry config {setup['retry']!r}"
    )


def test_fixture_permanent_failure_emits_dlq_event(fixture_data: dict) -> None:
    """Case: permanent_failure_emits_dlq_event."""
    case = next(c for c in fixture_data["test_cases"] if c["id"] == "permanent_failure_emits_dlq_event")
    setup = case["setup"]["subscriber"]
    expected = case["expected"]

    retry = _build_retry(setup["retry"])

    emitter = EventEmitter()
    dlq = _DLQRecorder()
    emitter.subscribe(dlq)

    sub = _CountingSubscriber(
        subscriber_id=setup["id"],
        fail_attempts=setup["fail_attempts"],
        retry=retry,
        subscriber_type=setup.get("type", "counting"),
    )
    emitter.subscribe(sub)

    trigger = case["trigger"]["event"]
    emitter.emit(_make_event(trigger["name"], trigger.get("payload", {})))
    emitter.flush(timeout=10.0)
    emitter.shutdown()

    assert sub.call_count == expected["attempt_count"]
    assert (len(dlq.received) > 0) == expected["dlq_event_emitted"]

    if expected["dlq_event_emitted"]:
        dlq_event = dlq.received[0]
        expected_dlq = expected["dlq_event"]
        assert dlq_event.event_type == expected_dlq["event_type"]
        data_contains = expected_dlq["data_contains"]
        assert dlq_event.data["subscriber_type"] == data_contains["subscriber_type"]
        assert dlq_event.data["subscriber_id"] == data_contains["subscriber_id"]
        assert dlq_event.data["attempt_count"] == data_contains["attempt_count"]
        assert dlq_event.data["original_event"]["name"] == data_contains["original_event"]["name"]
        for key in expected_dlq["data_required_keys"]:
            assert key in dlq_event.data, f"DLQ event missing required key: {key}"


def test_fixture_dlq_event_subscriber_failure_not_retried(fixture_data: dict, caplog: pytest.LogCaptureFixture) -> None:
    """Case: dlq_event_subscriber_failure_is_not_retried."""

    case = next(c for c in fixture_data["test_cases"] if c["id"] == "dlq_event_subscriber_failure_is_not_retried")
    setup = case["setup"]
    expected = case["expected"]

    primary_cfg = setup["primary_subscriber"]
    dlq_cfg = setup["dlq_subscriber"]

    emitter = EventEmitter()
    dlq_recorder = _DLQRecorder()
    emitter.subscribe(dlq_recorder)

    primary = _CountingSubscriber(
        subscriber_id=primary_cfg["id"],
        fail_attempts=primary_cfg["fail_attempts"],
        retry=_build_retry(primary_cfg["retry"]),
    )
    emitter.subscribe(primary)

    dlq_sub_calls: list[int] = []

    class _BreakingDLQSub:
        subscriber_id = dlq_cfg["id"]
        event_pattern = dlq_cfg["event_pattern"]
        retry = _build_retry(dlq_cfg["retry"])

        async def on_event(self, evt: ApCoreEvent) -> None:
            dlq_sub_calls.append(1)
            raise RuntimeError("dlq subscriber also broken")

    emitter.subscribe(_BreakingDLQSub())

    trigger = case["trigger"]["event"]
    with caplog.at_level(logging.ERROR, logger=emitter_module.__name__):
        emitter.emit(_make_event(trigger["name"], trigger.get("payload", {})))
        emitter.flush(timeout=10.0)
        emitter.shutdown()

    assert primary.call_count == expected["primary_attempt_count"]
    assert (len(dlq_recorder.received) > 0) == expected["dlq_event_emitted"]
    assert len(dlq_sub_calls) == expected["dlq_subscriber_attempt_count"]
    # No second-order DLQ event: the DLQ subscriber's own failure must not
    # produce another apcore.event.delivery_failed naming it.
    second_order = [e for e in dlq_recorder.received if e.data.get("subscriber_id") == dlq_cfg["id"]]
    assert (len(second_order) > 0) is expected["second_order_dlq_event_emitted"], f"{second_order!r}"
    # The failed DLQ delivery MUST be logged at ERROR and discarded — exactly
    # once, since it is neither retried nor re-DLQ'd.
    error_logs = [
        record
        for record in caplog.records
        if record.levelno == logging.ERROR and record.name == emitter_module.__name__
    ]
    assert len(error_logs) == expected["error_log_count"], f"{[r.getMessage() for r in error_logs]!r}"


def test_fixture_subscriber_id_generated_when_omitted(fixture_data: dict, monkeypatch: pytest.MonkeyPatch) -> None:
    """Case: subscriber_id_sdk_generated_when_omitted.

    The generated id has to be observed *on the DLQ events*, which is where the
    fixture says it must be used consistently. Asserting only on the two
    subscriber objects left ``dlq_events_emitted`` unread — an emitter that
    dropped DLQ emission for id-less subscribers passed that version of this
    test.
    """
    from apcore.events.subscribers import StdoutSubscriber

    case = next(c for c in fixture_data["test_cases"] if c["id"] == "subscriber_id_sdk_generated_when_omitted")
    expected = case["expected"]
    subscriber_configs = case["setup"]["subscribers"]

    emitter = EventEmitter()
    dlq = _DLQRecorder()
    emitter.subscribe(dlq)

    subscribers = [StdoutSubscriber(retry=_build_retry(cfg["retry"])) for cfg in subscriber_configs]
    for subscriber in subscribers:
        emitter.subscribe(subscriber)

    trigger = case["trigger"]["event"]
    monkeypatch.setattr(sys, "stdout", _BrokenStdout())
    try:
        emitter.emit(_make_event(trigger["name"], trigger.get("payload", {})))
        emitter.flush(timeout=10.0)
        emitter.shutdown()
    finally:
        monkeypatch.undo()

    assert len(dlq.received) == expected["dlq_events_emitted"], (
        f"expected one DLQ event per exhausted subscriber, got "
        f"{[e.data.get('subscriber_id') for e in dlq.received]!r}"
    )

    dlq_ids = [event.data["subscriber_id"] for event in dlq.received]
    assert (len(set(dlq_ids)) == len(dlq_ids)) is expected[
        "subscriber_ids_distinct"
    ], f"DLQ events must carry distinct generated ids, got {dlq_ids!r}"
    assert set(dlq_ids) == {
        s.subscriber_id for s in subscribers
    }, "the id on the DLQ event must be the same id the subscriber carries"

    pattern = expected["subscriber_ids_pattern"]
    for subscriber_id in dlq_ids:
        assert re.match(pattern, subscriber_id), f"{subscriber_id!r} doesn't match {pattern}"


# ---------------------------------------------------------------------------
# apcore#85 — the declared ``retry:`` block must reach the delivery policy
# ---------------------------------------------------------------------------

_RETRY_FIELDS = ("max_attempts", "initial_backoff_ms", "max_backoff_ms", "backoff_multiplier")

# The connection fields each built-in type needs to instantiate at all. The
# fixture declares only ``type`` and ``retry`` — everything below is inert
# plumbing, chosen so the factory gets far enough to parse the retry block.
# Nothing here is ever delivered to: the assertion is on the resolved policy,


def _resolved_delivery_policy(subscriber: Any) -> EventRetryConfig:
    """Return the policy the emitter's delivery loop would apply to *subscriber*.

    Deliberately not ``subscriber.retry`` and deliberately not the config dict:
    the contract is about what the DELIVERY PATH resolves. ``_deliver_with_retry``
    reads the policy through ``_get_retry_config``, so that is the function that
    has to be asserted — a field a factory writes but delivery never consults
    would otherwise read as green (which is exactly how apcore-rust's
    ``retry_count`` survived).
    """
    return emitter_module._get_retry_config(subscriber)


def _subscriber_from_declared_config(entry: dict[str, Any]) -> Any:
    """Instantiate a subscriber through the public config factory.

    Goes through ``create_subscriber_from_config`` rather than a constructor so
    the thing under test is the config-parsing path an application's
    ``sys_modules.events.subscribers`` block actually takes.
    """
    # The fixture carries every field a type requires to construct, so the entry
    # goes to the factory verbatim — see its `a_case_must_carry_its_own_inputs`
    # contract. This driver used to hold its own table of `url` / `path` /
    # `delegate_type` values; three drivers each inventing one is not a
    # cross-language contract, it is three private guesses.
    return create_subscriber_from_config(dict(entry))


def test_fixture_declared_retry_policy_is_read_for_every_subscriber_type(fixture_data: dict) -> None:
    """Case: declared_retry_policy_is_read_for_every_subscriber_type."""
    case = next(
        c for c in fixture_data["test_cases"] if c["id"] == "declared_retry_policy_is_read_for_every_subscriber_type"
    )
    expected = case["expected"]
    per_type = expected["resolved_policy_per_type"]

    # The fixture's own precondition, asserted rather than trusted. A declared
    # value that happens to equal the shipped default passes whether or not the
    # config was ever read — the reason this defect survived in three SDKs at
    # once. If a future fixture edit softens one of these back to a default,
    # this fails here instead of silently going vacuous.
    default = EventRetryConfig()
    assert expected["every_field_differs_from_the_default"] is True, (
        "the fixture no longer claims its policy differs from the default on every field; "
        "a default-valued field cannot distinguish 'config was read' from 'config was ignored'"
    )
    for type_name, policy in per_type.items():
        for field in _RETRY_FIELDS:
            assert policy[field] != getattr(default, field), (
                f"resolved_policy_per_type[{type_name}].{field} == the shipped default "
                f"({getattr(default, field)!r}) — that assertion cannot fail on an unread config"
            )

    # Every built-in type or the case is half done: `webhook` parsed a
    # different flat spelling while file/stdout/filter had no field at all, so
    # a webhook-only assertion was green in every SDK.
    declared = [entry["type"] for entry in case["subscribers"]]
    assert sorted(declared) == sorted(per_type), (
        f"fixture declares subscribers {sorted(declared)} but expects policies for "
        f"{sorted(per_type)} — every declared type must have an expectation"
    )

    for entry in case["subscribers"]:
        subscriber_type = entry["type"]
        resolved = _resolved_delivery_policy(_subscriber_from_declared_config(entry))
        actual = {field: getattr(resolved, field) for field in _RETRY_FIELDS}
        want = {field: per_type[subscriber_type][field] for field in _RETRY_FIELDS}
        assert actual == want, (
            f"{subscriber_type}: the emitter would deliver with {actual!r}, but the subscriber "
            f"declared {entry['retry']!r} — the config never reached the delivery policy"
        )


def test_fixture_nested_retry_block_wins_over_legacy_retry_count(fixture_data: dict) -> None:
    """Case: nested_retry_block_wins_over_legacy_retry_count."""
    case = next(c for c in fixture_data["test_cases"] if c["id"] == "nested_retry_block_wins_over_legacy_retry_count")
    expected = case["expected"]

    # Picked structurally, not by index: which entry is which is the point of
    # the case, so reading it off list order would assert nothing.
    both = next(e for e in case["subscribers"] if "retry" in e and "retry_count" in e)
    legacy_only = next(e for e in case["subscribers"] if "retry" not in e and "retry_count" in e)

    nested = _resolved_delivery_policy(_subscriber_from_declared_config(both))
    assert nested.max_attempts == expected["nested_wins"]["max_attempts"], (
        f"both spellings present: delivery resolved max_attempts={nested.max_attempts}, "
        f"but the nested block declared {both['retry']['max_attempts']} and MUST win"
    )
    # The nested block wins whole, not only on the one field the flat spelling
    # can express — otherwise `retry_count` would silently reset the backoff.
    for field in _RETRY_FIELDS:
        assert getattr(nested, field) == both["retry"][field], f"nested block lost its {field}"

    legacy = _resolved_delivery_policy(_subscriber_from_declared_config(legacy_only))
    assert legacy.max_attempts == expected["legacy_alone_still_translates"]["max_attempts"], (
        f"retry_count={legacy_only['retry_count']} MUST translate to "
        f"max_attempts={legacy_only['retry_count'] + 1}, got {legacy.max_attempts}"
    )


# ---------------------------------------------------------------------------
# Case-inventory guard
# ---------------------------------------------------------------------------

# Maps every canonical case id to the test function that drives it. A case
# added to the fixture with no driver here fails the guard below instead of
# quietly widening the gap between the spec and this SDK.
COVERED: dict[str, str] = {
    "retry_succeeds_before_exhaustion": "test_fixture_retry_succeeds_before_exhaustion",
    "permanent_failure_emits_dlq_event": "test_fixture_permanent_failure_emits_dlq_event",
    "dlq_event_subscriber_failure_is_not_retried": "test_fixture_dlq_event_subscriber_failure_not_retried",
    "subscriber_id_sdk_generated_when_omitted": "test_fixture_subscriber_id_generated_when_omitted",
    "declared_retry_policy_is_read_for_every_subscriber_type": (
        "test_fixture_declared_retry_policy_is_read_for_every_subscriber_type"
    ),
    "nested_retry_block_wins_over_legacy_retry_count": ("test_fixture_nested_retry_block_wins_over_legacy_retry_count"),
}


def test_every_canonical_case_is_driven() -> None:
    canonical = set(case_ids(FIXTURE))
    claimed = set(COVERED)
    assert canonical - claimed == set(), f"canonical fixture {FIXTURE} gained case(s) with no driver here"
    assert claimed - canonical == set(), f"this file claims case(s) {FIXTURE} no longer defines"


def test_every_claimed_driver_exists() -> None:
    module = sys.modules[__name__]
    missing = [name for name in COVERED.values() if not hasattr(module, name)]
    assert missing == [], f"claimed driver function(s) not defined: {missing}"
