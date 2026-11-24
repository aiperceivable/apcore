"""Conformance tests for Event Management Hardening (Issue #36).

Covers all 10 cases in:
  apcore/conformance/fixtures/event_management_hardening.json
"""

from __future__ import annotations

import sys

import re
from datetime import datetime, timedelta, timezone
from typing import Any
from unittest.mock import AsyncMock, MagicMock

import pytest

from apcore.events.circuit_breaker import CircuitBreakerWrapper, CircuitState
from apcore.events.emitter import ApCoreEvent, EventEmitter
from conformance.canonical_fixtures import (
    case_ids,
    dispatch_or_fail,
    load_fixture,
    reject_unknown_expectations,
)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


FIXTURE = "event_management_hardening.json"

#: canonical case id -> case body. apcore#93: every one of this fixture's ten
#: cases used to be transcribed into the driver as literals — the subscriber
#: configs, the events, the circuit-breaker thresholds, the expected states —
#: so mutating any declared value in the canonical JSON left the whole file
#: green. Each case body below now takes its inputs AND its expectations from
#: here.
_CASES: dict[str, Any] = {case["id"]: case for case in load_fixture(FIXTURE)["test_cases"]}


def _case(case_id: str) -> dict[str, Any]:
    case = _CASES[case_id]
    reject_unknown_expectations(FIXTURE, case, {"expected"})
    return case


#: fixture ``circuit_state`` (a wire-level state NAME) -> the SDK enum member.
#: ``dispatch_or_fail`` hard-fails on an unrecognised name rather than letting
#: it skip the assertion.
_CIRCUIT_STATE_BY_NAME: dict[str, CircuitState] = {s.value: s for s in CircuitState}


def _make_event(**overrides: Any) -> ApCoreEvent:
    defaults: dict[str, Any] = {
        "event_type": "apcore.module.toggled",
        "module_id": "executor.email.send_email",
        "timestamp": "2026-04-28T00:00:00Z",
        "severity": "info",
        "data": {"module_id": "executor.email.send_email", "enabled": False},
    }
    defaults.update(overrides)
    return ApCoreEvent(**defaults)


def _elapsed_seconds(start: str, end: str) -> float:
    """Seconds between two ISO-8601 instants the fixture declares."""
    fmt = lambda s: datetime.fromisoformat(s.replace("Z", "+00:00"))  # noqa: E731
    return (fmt(end) - fmt(start)).total_seconds()


def _event_from(spec: dict[str, Any]) -> ApCoreEvent:
    """Build the event a case declares, verbatim."""
    return ApCoreEvent(
        event_type=spec["event_type"],
        module_id=spec["module_id"],
        timestamp=spec["timestamp"],
        severity=spec["severity"],
        data=dict(spec["data"]),
    )


def _make_mock_emitter() -> MagicMock:
    emitter = MagicMock(spec=EventEmitter)
    emitter.emit = MagicMock()
    return emitter


# ---------------------------------------------------------------------------
# Case 1: subscriber_factory_registered_type
# ---------------------------------------------------------------------------


class TestSubscriberFactoryRegisteredType:
    """A custom subscriber type registered via register_subscriber_type is
    instantiated when referenced by name in configuration."""

    @pytest.fixture(autouse=True)
    def _reset_registry(self) -> Any:
        from apcore.sys_modules.registration import reset_subscriber_registry

        reset_subscriber_registry()
        yield
        reset_subscriber_registry()

    def test_custom_slack_type_instantiated(self) -> None:
        from apcore.sys_modules.registration import (
            _create_subscriber,
            register_subscriber_type,
        )

        case = _case("subscriber_factory_registered_type")
        params, expected = case["input"], case["expected"]

        # One factory per declared registered type, so the assertion below can
        # say WHICH factory ran rather than "a subscriber came back".
        subscribers: dict[str, MagicMock] = {}
        factories: dict[str, MagicMock] = {}
        for type_name in params["registered_types"]:
            sub = MagicMock()
            sub.on_event = AsyncMock()
            subscribers[type_name] = sub
            factories[type_name] = MagicMock(return_value=sub)
            register_subscriber_type(type_name, factories[type_name])

        sub_cfg: dict[str, Any] = dict(params["subscriber_config"])
        subscriber = _create_subscriber(sub_cfg)

        assert (subscriber is not None) is expected["subscriber_created"], (
            f"[{case['id']}] fixture declares subscriber_created="
            f"{expected['subscriber_created']}, got {subscriber!r}"
        )
        factory = dispatch_or_fail(FIXTURE, case["id"], expected["subscriber_type"], factories, "subscriber type")
        factory.assert_called_once_with(sub_cfg)
        assert subscriber is subscribers[expected["subscriber_type"]], (
            f"[{case['id']}] the config named type {sub_cfg['type']!r}; the fixture declares "
            f"the {expected['subscriber_type']!r} factory must be the one that built it"
        )


# ---------------------------------------------------------------------------
# Cases 2/3: builtin_stdout_type, builtin_file_type
# ---------------------------------------------------------------------------


def _assert_builtin_subscriber_case(case_id: str) -> None:
    """Drive one ``requires_registration: false`` built-in case off the fixture.

    ``subscriber_created`` and ``requires_registration`` used to reach no
    assertion at all — the driver checked ``isinstance`` against a class it had
    chosen itself, which is true of an SDK whose built-in registry is empty for
    every OTHER type the fixture might declare.
    """
    from apcore.events.subscribers import FileSubscriber, StdoutSubscriber
    from apcore.sys_modules.registration import (
        _BUILTIN_FACTORIES,
        _create_subscriber,
        reset_subscriber_registry,
    )

    classes: dict[str, type] = {"stdout": StdoutSubscriber, "file": FileSubscriber}

    case = _case(case_id)
    params, expected = case["input"], case["expected"]
    sub_cfg: dict[str, Any] = dict(params["subscriber_config"])
    expected_class = dispatch_or_fail(FIXTURE, case_id, expected["subscriber_type"], classes, "subscriber type")

    # No register_subscriber_type call anywhere in this test: the registry is
    # reset to built-ins only, which is what "requires_registration: false"
    # asserts.
    reset_subscriber_registry()
    requires_registration = sub_cfg["type"] not in _BUILTIN_FACTORIES
    assert requires_registration is expected["requires_registration"], (
        f"[{case_id}] fixture declares requires_registration="
        f"{expected['requires_registration']}; {sub_cfg['type']!r} "
        f"{'is not' if requires_registration else 'is'} a built-in type"
    )

    subscriber = _create_subscriber(sub_cfg)
    assert (subscriber is not None) is expected["subscriber_created"], (
        f"[{case_id}] fixture declares subscriber_created=" f"{expected['subscriber_created']}, got {subscriber!r}"
    )
    assert isinstance(subscriber, expected_class), (
        f"[{case_id}] fixture declares subscriber_type "
        f"{expected['subscriber_type']!r}; got {type(subscriber).__name__}"
    )


# ---------------------------------------------------------------------------
# Case 2: builtin_stdout_type
# ---------------------------------------------------------------------------


class TestBuiltinStdoutType:
    """The stdout subscriber type is available as a built-in without registration."""

    def test_stdout_subscriber_created_without_registration(self) -> None:
        _assert_builtin_subscriber_case("builtin_stdout_type")

    def test_stdout_subscriber_conforms_to_protocol(self) -> None:
        from apcore.events.emitter import EventSubscriber
        from apcore.events.subscribers import StdoutSubscriber

        sub = StdoutSubscriber(output_format="json")
        assert isinstance(sub, EventSubscriber)


# ---------------------------------------------------------------------------
# Case 3: builtin_file_type
# ---------------------------------------------------------------------------


class TestBuiltinFileType:
    """The file subscriber type is available as a built-in without registration."""

    def test_file_subscriber_created_without_registration(self) -> None:
        _assert_builtin_subscriber_case("builtin_file_type")

    def test_file_subscriber_conforms_to_protocol(self) -> None:
        from apcore.events.emitter import EventSubscriber
        from apcore.events.subscribers import FileSubscriber

        sub = FileSubscriber(path="/tmp/test.log")
        assert isinstance(sub, EventSubscriber)


# ---------------------------------------------------------------------------
# Cases 4/5: builtin_filter_passes_matching, builtin_filter_discards_nonmatching
# ---------------------------------------------------------------------------


async def _assert_filter_case(case_id: str) -> None:
    """Drive one filter case off the fixture's own config and event.

    ``delivery_attempted`` and ``discarded`` are the declared contract and are
    now compared as booleans, so the pass case and the discard case cannot be
    satisfied by the same implementation — which ``assert_called_once_with`` /
    ``assert_not_called`` against driver-authored events could not tell apart
    once the fixture changed.
    """
    from apcore.events.subscribers import FilterSubscriber

    case = _case(case_id)
    params, expected = case["input"], case["expected"]
    cfg = params["subscriber_config"]

    delegate = MagicMock()
    delegate.on_event = AsyncMock()
    filter_sub = FilterSubscriber(
        delegate=delegate,
        include_events=list(cfg.get("include_events") or []) or None,
        exclude_events=list(cfg.get("exclude_events") or []) or None,
    )

    event = _event_from(params["event"])
    await filter_sub.on_event(event)

    delivery_attempted = delegate.on_event.call_count > 0
    assert delivery_attempted is expected["delivery_attempted"], (
        f"[{case_id}] fixture declares delivery_attempted="
        f"{expected['delivery_attempted']}; the delegate was called "
        f"{delegate.on_event.call_count}x for {event.event_type!r}"
    )
    discarded = not delivery_attempted
    assert (
        discarded is expected["discarded"]
    ), f"[{case_id}] fixture declares discarded={expected['discarded']}, got {discarded}"
    if delivery_attempted:
        # Forwarded verbatim: a filter that rebuilds the event is not a filter.
        delegate.on_event.assert_called_once_with(event)


# ---------------------------------------------------------------------------
# Case 4: builtin_filter_passes_matching
# ---------------------------------------------------------------------------


class TestBuiltinFilterPassesMatching:
    """A filter subscriber forwards an event whose type matches include_events."""

    @pytest.mark.asyncio
    async def test_filter_forwards_matching_event(self) -> None:
        await _assert_filter_case("builtin_filter_passes_matching")


# ---------------------------------------------------------------------------
# Case 5: builtin_filter_discards_nonmatching
# ---------------------------------------------------------------------------


class TestBuiltinFilterDiscardsNonmatching:
    """A filter subscriber silently discards an event not matching include_events."""

    @pytest.mark.asyncio
    async def test_filter_discards_nonmatching_event(self) -> None:
        await _assert_filter_case("builtin_filter_discards_nonmatching")


# ---------------------------------------------------------------------------
# The event-pattern DIALECT — PROTOCOL_SPEC 9.16.3, Algorithm A25 (#117)
# ---------------------------------------------------------------------------


class TestFilterPatternDialect:
    """`event_pattern` / `include_events` / `exclude_events` are matched with A25.

    The direction matters and is why these cases are written on
    ``exclude_events`` wherever possible: **exclude_events fails open.** A
    pattern that does not match means the event is DELIVERED, so a matcher
    understanding fewer metacharacters than the operator wrote does not narrow
    the filter, it opens it. ``include_events`` fails the safe way round under
    the identical divergence, which is exactly why nobody noticed.
    """

    @pytest.mark.asyncio
    async def test_exclude_question_mark_is_a_wildcard(self) -> None:
        await _assert_filter_case("filter_exclude_question_mark_is_a_wildcard")

    @pytest.mark.asyncio
    async def test_exclude_character_class_does_not_expand(self) -> None:
        await _assert_filter_case("filter_exclude_character_class_does_not_expand")

    @pytest.mark.asyncio
    async def test_include_question_mark_is_a_wildcard(self) -> None:
        await _assert_filter_case("filter_include_question_mark_is_a_wildcard")


# ---------------------------------------------------------------------------
# Case 6: circuit_open_after_threshold
# ---------------------------------------------------------------------------


class TestCircuitOpenAfterThreshold:
    """After consecutive_failures reaches open_threshold, circuit transitions to OPEN."""

    @pytest.mark.asyncio
    async def test_circuit_opens_after_three_failures(self) -> None:
        case = _case("circuit_open_after_threshold")
        params, expected = case["input"], case["expected"]
        cfg = params["circuit_breaker_config"]

        failing_sub = MagicMock()
        failing_sub.on_event = AsyncMock(side_effect=RuntimeError("downstream error"))

        emitted_events: list[ApCoreEvent] = []
        mock_emitter = _make_mock_emitter()
        mock_emitter.emit.side_effect = emitted_events.append

        cb = CircuitBreakerWrapper(
            subscriber=failing_sub,
            emitter=mock_emitter,
            timeout_ms=cfg["timeout_ms"],
            open_threshold=cfg["open_threshold"],
            recovery_window_ms=cfg["recovery_window_ms"],
        )

        event = _make_event()
        for attempt in params["failure_sequence"]:
            assert attempt["outcome"] == "failure", (
                f"[{case['id']}] this driver models a failure-only sequence; "
                f"attempt {attempt['attempt']} declares {attempt['outcome']!r}"
            )
            await cb.on_event(event)

        expected_state = dispatch_or_fail(
            FIXTURE, case["id"], expected["circuit_state"], _CIRCUIT_STATE_BY_NAME, "circuit state"
        )
        assert cb.state is expected_state, (
            f"[{case['id']}] fixture declares circuit_state={expected['circuit_state']!r}, " f"got {cb.state.value!r}"
        )
        assert cb.consecutive_failures == expected["consecutive_failures"], (
            f"[{case['id']}] fixture declares consecutive_failures="
            f"{expected['consecutive_failures']}, got {cb.consecutive_failures}"
        )

        emitted = [e for e in emitted_events if e.event_type == expected["event_emitted"]]
        assert len(emitted) == 1, (
            f"[{case['id']}] fixture declares event_emitted={expected['event_emitted']!r}; "
            f"emitted were {[e.event_type for e in emitted_events]}"
        )
        assert emitted[0].data["consecutive_failures"] == expected["consecutive_failures"]


# ---------------------------------------------------------------------------
# Case 7: circuit_discards_in_open_state
# ---------------------------------------------------------------------------


class TestCircuitDiscardsInOpenState:
    """In OPEN state, deliver() is not called — the event is silently discarded."""

    @pytest.mark.asyncio
    async def test_delivery_not_attempted_when_open(self) -> None:
        case = _case("circuit_discards_in_open_state")
        params, expected = case["input"], case["expected"]

        inner_sub = MagicMock()
        inner_sub.on_event = AsyncMock()

        mock_emitter = _make_mock_emitter()

        cb = CircuitBreakerWrapper(
            subscriber=inner_sub,
            emitter=mock_emitter,
            open_threshold=3,
        )
        # Start in the state the fixture declares, inside the recovery window
        # so nothing transitions on its own.
        cb._state = _CIRCUIT_STATE_BY_NAME[params["circuit_state"]]
        cb._last_failure_at = datetime.now(timezone.utc)

        await cb.on_event(_event_from(params["event"]))

        delivery_attempted = inner_sub.on_event.call_count > 0
        assert delivery_attempted is expected["delivery_attempted"], (
            f"[{case['id']}] fixture declares delivery_attempted="
            f"{expected['delivery_attempted']}; the wrapped subscriber was called "
            f"{inner_sub.on_event.call_count}x"
        )
        expected_state = dispatch_or_fail(
            FIXTURE, case["id"], expected["circuit_state"], _CIRCUIT_STATE_BY_NAME, "circuit state"
        )
        assert cb.state is expected_state, (
            f"[{case['id']}] fixture declares circuit_state={expected['circuit_state']!r}, " f"got {cb.state.value!r}"
        )


# ---------------------------------------------------------------------------
# Case 8: circuit_half_open_after_window
# ---------------------------------------------------------------------------


class TestCircuitHalfOpenAfterWindow:
    """After recovery_window_ms has elapsed, OPEN transitions to HALF_OPEN."""

    @pytest.mark.asyncio
    async def test_transitions_to_half_open_after_window(self) -> None:
        """The state the delivery attempt RUNS IN is the declared one.

        apcore#93: this used to assert ``cb.state != CircuitState.OPEN`` after
        ``on_event`` — a catch-all that CLOSED, HALF_OPEN and any future state
        all satisfy, and that read the fixture's declared ``circuit_state``
        not at all. The wrapped subscriber now records the state it was called
        in, which is exactly the transition the case describes; the post-
        delivery state is CLOSED and is asserted by ``circuit_closes_on_success``.
        """
        case = _case("circuit_half_open_after_window")
        params, expected = case["input"], case["expected"]
        cfg = params["circuit_breaker_config"]
        elapsed = _elapsed_seconds(params["last_failure_at"], params["current_time"])
        assert (
            elapsed * 1000 > cfg["recovery_window_ms"]
        ), f"[{case['id']}] the declared current_time must be beyond the recovery window"

        observed_states: list[CircuitState] = []
        inner_sub = MagicMock()

        async def _record(_event: ApCoreEvent) -> None:
            observed_states.append(cb.state)

        inner_sub.on_event = AsyncMock(side_effect=_record)
        mock_emitter = _make_mock_emitter()

        cb = CircuitBreakerWrapper(
            subscriber=inner_sub,
            emitter=mock_emitter,
            timeout_ms=cfg["timeout_ms"],
            open_threshold=cfg["open_threshold"],
            recovery_window_ms=cfg["recovery_window_ms"],
        )
        cb._state = _CIRCUIT_STATE_BY_NAME[params["circuit_state"]]
        cb._last_failure_at = datetime.now(timezone.utc) - timedelta(seconds=elapsed)

        await cb.on_event(_make_event())

        expected_state = dispatch_or_fail(
            FIXTURE, case["id"], expected["circuit_state"], _CIRCUIT_STATE_BY_NAME, "circuit state"
        )
        assert observed_states == [expected_state], (
            f"[{case['id']}] fixture declares the circuit transitions to "
            f"{expected['circuit_state']!r} once the recovery window elapses; the "
            f"delivery ran in {[s.value for s in observed_states]}"
        )

    def test_check_recovery_transitions_open_to_half_open(self) -> None:
        case = _case("circuit_half_open_after_window")
        params, expected = case["input"], case["expected"]
        cfg = params["circuit_breaker_config"]
        elapsed = _elapsed_seconds(params["last_failure_at"], params["current_time"])

        mock_emitter = _make_mock_emitter()
        inner_sub = MagicMock()
        inner_sub.on_event = AsyncMock()

        cb = CircuitBreakerWrapper(
            subscriber=inner_sub,
            emitter=mock_emitter,
            recovery_window_ms=cfg["recovery_window_ms"],
        )
        cb._state = _CIRCUIT_STATE_BY_NAME[params["circuit_state"]]
        cb._last_failure_at = datetime.now(timezone.utc) - timedelta(seconds=elapsed)

        cb._check_recovery()

        expected_state = dispatch_or_fail(
            FIXTURE, case["id"], expected["circuit_state"], _CIRCUIT_STATE_BY_NAME, "circuit state"
        )
        assert cb._state is expected_state, (
            f"[{case['id']}] fixture declares circuit_state={expected['circuit_state']!r}, " f"got {cb._state.value!r}"
        )

    def test_recovery_does_not_fire_inside_the_window(self) -> None:
        """The observable that makes the case above mean something.

        A wrapper that transitions to HALF_OPEN unconditionally would satisfy
        every assertion above; the window has to actually gate it.
        """
        case = _case("circuit_half_open_after_window")
        params = case["input"]
        cfg = params["circuit_breaker_config"]

        mock_emitter = _make_mock_emitter()
        inner_sub = MagicMock()
        inner_sub.on_event = AsyncMock()

        cb = CircuitBreakerWrapper(
            subscriber=inner_sub,
            emitter=mock_emitter,
            recovery_window_ms=cfg["recovery_window_ms"],
        )
        cb._state = _CIRCUIT_STATE_BY_NAME[params["circuit_state"]]
        cb._last_failure_at = datetime.now(timezone.utc)

        cb._check_recovery()

        assert cb._state is _CIRCUIT_STATE_BY_NAME[params["circuit_state"]], (
            f"[{case['id']}] the circuit left {params['circuit_state']!r} before "
            f"recovery_window_ms={cfg['recovery_window_ms']} had elapsed"
        )


# ---------------------------------------------------------------------------
# Case 9: circuit_closes_on_success
# ---------------------------------------------------------------------------


class TestCircuitClosesOnSuccess:
    """A successful delivery in HALF_OPEN transitions to CLOSED and emits circuit_closed."""

    @pytest.mark.asyncio
    async def test_half_open_success_closes_circuit(self) -> None:
        case = _case("circuit_closes_on_success")
        params, expected = case["input"], case["expected"]
        assert params["delivery_outcome"] == "success", f"[{case['id']}] this driver models a succeeding delivery"

        inner_sub = MagicMock()
        inner_sub.on_event = AsyncMock(return_value=None)

        emitted_events: list[ApCoreEvent] = []
        mock_emitter = _make_mock_emitter()
        mock_emitter.emit.side_effect = emitted_events.append

        cb = CircuitBreakerWrapper(
            subscriber=inner_sub,
            emitter=mock_emitter,
        )
        cb._state = _CIRCUIT_STATE_BY_NAME[params["circuit_state"]]
        cb._consecutive_failures = 3

        await cb.on_event(_make_event())

        expected_state = dispatch_or_fail(
            FIXTURE, case["id"], expected["circuit_state"], _CIRCUIT_STATE_BY_NAME, "circuit state"
        )
        assert cb.state is expected_state, (
            f"[{case['id']}] fixture declares circuit_state={expected['circuit_state']!r}, " f"got {cb.state.value!r}"
        )
        assert cb.consecutive_failures == expected["consecutive_failures"], (
            f"[{case['id']}] fixture declares consecutive_failures="
            f"{expected['consecutive_failures']}, got {cb.consecutive_failures}"
        )

        emitted = [e for e in emitted_events if e.event_type == expected["event_emitted"]]
        assert len(emitted) == 1, (
            f"[{case['id']}] fixture declares event_emitted={expected['event_emitted']!r}; "
            f"emitted were {[e.event_type for e in emitted_events]}"
        )
        assert emitted[0].severity == "info"


# ---------------------------------------------------------------------------
# Additional coverage: HALF_OPEN → failure → OPEN
# ---------------------------------------------------------------------------


class TestCircuitHalfOpenFailureReopens:
    """A failed delivery in HALF_OPEN transitions back to OPEN and re-emits circuit_opened."""

    @pytest.mark.asyncio
    async def test_half_open_failure_reopens_circuit(self) -> None:
        failing_sub = MagicMock()
        failing_sub.on_event = AsyncMock(side_effect=RuntimeError("still down"))

        emitted_events: list[ApCoreEvent] = []
        mock_emitter = _make_mock_emitter()
        mock_emitter.emit.side_effect = emitted_events.append

        cb = CircuitBreakerWrapper(
            subscriber=failing_sub,
            emitter=mock_emitter,
        )
        cb._state = CircuitState.HALF_OPEN
        cb._consecutive_failures = 3

        await cb.on_event(_make_event())

        assert cb.state == CircuitState.OPEN
        circuit_opened = [e for e in emitted_events if e.event_type == "apcore.subscriber.circuit_opened"]
        assert len(circuit_opened) == 1


class TestCircuitClosedSuccessResetsCounter:
    """A successful delivery in CLOSED state resets consecutive_failures to 0."""

    @pytest.mark.asyncio
    async def test_closed_success_resets_failures(self) -> None:
        inner_sub = MagicMock()
        inner_sub.on_event = AsyncMock(return_value=None)
        mock_emitter = _make_mock_emitter()

        cb = CircuitBreakerWrapper(subscriber=inner_sub, emitter=mock_emitter)
        cb._consecutive_failures = 2  # some prior failures that didn't reach threshold

        await cb.on_event(_make_event())

        assert cb.consecutive_failures == 0
        assert cb.state == CircuitState.CLOSED
        mock_emitter.emit.assert_not_called()


class TestFileSubscriberWritesJson:
    """FileSubscriber writes a JSON line to the target file."""

    @pytest.mark.asyncio
    async def test_writes_json_line(self, tmp_path: Any) -> None:
        from apcore.events.subscribers import FileSubscriber

        log_file = tmp_path / "events.jsonl"
        sub = FileSubscriber(path=str(log_file), output_format="json")
        event = _make_event(event_type="apcore.module.toggled")

        await sub.on_event(event)

        content = log_file.read_text()
        import json

        record = json.loads(content.strip())
        assert record["event_type"] == "apcore.module.toggled"

    @pytest.mark.asyncio
    async def test_writes_text_line(self, tmp_path: Any) -> None:
        from apcore.events.subscribers import FileSubscriber

        log_file = tmp_path / "events.log"
        sub = FileSubscriber(path=str(log_file), output_format="text")
        event = _make_event(event_type="apcore.module.toggled", severity="info")

        await sub.on_event(event)

        content = log_file.read_text()
        assert "apcore.module.toggled" in content
        assert "INFO" in content


class TestStdoutSubscriberOutput:
    """StdoutSubscriber prints events, respecting level_filter."""

    @pytest.mark.asyncio
    async def test_prints_json_to_stdout(self, capsys: Any) -> None:
        from apcore.events.subscribers import StdoutSubscriber

        sub = StdoutSubscriber(output_format="json")
        await sub.on_event(_make_event(event_type="apcore.config.updated", severity="info"))

        captured = capsys.readouterr()
        import json

        record = json.loads(captured.out.strip())
        assert record["event_type"] == "apcore.config.updated"

    @pytest.mark.asyncio
    async def test_level_filter_suppresses_below_threshold(self, capsys: Any) -> None:
        from apcore.events.subscribers import StdoutSubscriber

        sub = StdoutSubscriber(level_filter="error")
        await sub.on_event(_make_event(severity="info"))
        await sub.on_event(_make_event(severity="warn"))

        captured = capsys.readouterr()
        assert captured.out == ""

    @pytest.mark.asyncio
    async def test_level_filter_passes_at_or_above_threshold(self, capsys: Any) -> None:
        from apcore.events.subscribers import StdoutSubscriber

        sub = StdoutSubscriber(level_filter="error")
        await sub.on_event(_make_event(severity="error"))

        captured = capsys.readouterr()
        assert "apcore.module.toggled" in captured.out


class TestFilterSubscriberExcludeEvents:
    """FilterSubscriber discards events matching exclude_events."""

    @pytest.mark.asyncio
    async def test_exclude_events_discards_matching(self) -> None:
        from apcore.events.subscribers import FilterSubscriber

        delegate = MagicMock()
        delegate.on_event = AsyncMock()

        filter_sub = FilterSubscriber(
            delegate=delegate,
            exclude_events=["apcore.config.*"],
        )

        await filter_sub.on_event(_make_event(event_type="apcore.config.updated"))
        delegate.on_event.assert_not_called()

    @pytest.mark.asyncio
    async def test_exclude_events_passes_nonmatching(self) -> None:
        from apcore.events.subscribers import FilterSubscriber

        delegate = MagicMock()
        delegate.on_event = AsyncMock()

        filter_sub = FilterSubscriber(
            delegate=delegate,
            exclude_events=["apcore.config.*"],
        )

        event = _make_event(event_type="apcore.module.toggled")
        await filter_sub.on_event(event)
        delegate.on_event.assert_called_once_with(event)


# ---------------------------------------------------------------------------
# Case 10: event_naming_canonical
# ---------------------------------------------------------------------------


async def _emitted_subscriber_event_names() -> list[str]:
    """Event names apcore-python ACTUALLY emits, captured from a live circuit.

    Not a transcription: the circuit is driven open and then closed, and the
    ``event_type`` strings the SDK produces are returned. This is what makes
    ``event_naming_canonical`` an assertion about the implementation rather
    than about a list the driver wrote for itself.
    """
    emitted: list[ApCoreEvent] = []
    mock_emitter = _make_mock_emitter()
    mock_emitter.emit.side_effect = emitted.append

    failing = MagicMock()
    failing.on_event = AsyncMock(side_effect=RuntimeError("downstream error"))
    cb = CircuitBreakerWrapper(subscriber=failing, emitter=mock_emitter, open_threshold=1, recovery_window_ms=30000)
    await cb.on_event(_make_event())  # → OPEN, emits circuit_opened

    cb._state = CircuitState.HALF_OPEN
    failing.on_event = AsyncMock(return_value=None)
    cb._subscriber = failing
    await cb.on_event(_make_event())  # → CLOSED, emits circuit_closed

    return [e.event_type for e in emitted]


class TestEventNamingCanonical:
    """All built-in framework events follow the apcore.<subsystem>.<event> pattern.

    ``test_circuit_opened_event_has_canonical_name`` and its ``_closed`` twin
    used to live here, each matching the driver's own regex against the
    driver's own string literal — a tautology that could not fail on SDK
    behaviour (shape 5 in ``conformance.canonical_fixtures``). They are not
    deleted but CORRECTED into
    :meth:`test_declared_pattern_accepts_the_names_the_sdk_emits`, which checks
    the same two names against the same contract with both sides sourced from
    outside the driver: the pattern from the fixture, the names from a live
    circuit breaker.
    """

    def test_declared_pattern_accepts_every_declared_event(self) -> None:
        """``events_to_check`` judged by the fixture's own declared pattern.

        apcore#93: the list AND the regex used to be class attributes, so
        neither ``expected.pattern`` nor ``expected.all_match_pattern`` reached
        an assertion.
        """
        case = _case("event_naming_canonical")
        params, expected = case["input"], case["expected"]
        pattern = re.compile(expected["pattern"])
        mismatches = [e for e in params["events_to_check"] if not pattern.match(e)]
        all_match = not mismatches
        assert all_match is expected["all_match_pattern"], (
            f"[{case['id']}] fixture declares all_match_pattern="
            f"{expected['all_match_pattern']}; {mismatches} do not match "
            f"{expected['pattern']!r}"
        )

    @pytest.mark.asyncio
    async def test_declared_pattern_accepts_the_names_the_sdk_emits(self) -> None:
        """The declared pattern must accept apcore-python's real event names.

        This is the assertion that makes the case pin anything. The comparison
        above is symmetric — mutate ``pattern`` to something unmatchable and
        ``all_match_pattern`` to false in the same edit and it still passes —
        so the pattern is additionally held against event names taken from the
        SDK rather than from the fixture. ``events_to_check`` must name them,
        too, or the fixture has drifted from what the SDK emits.
        """
        case = _case("event_naming_canonical")
        params, expected = case["input"], case["expected"]
        pattern = re.compile(expected["pattern"])

        emitted = await _emitted_subscriber_event_names()
        assert emitted, "the circuit breaker emitted nothing to check"

        unmatched = [name for name in emitted if not pattern.match(name)]
        assert unmatched == [], (
            f"[{case['id']}] the declared pattern {expected['pattern']!r} rejects event "
            f"name(s) apcore-python actually emits: {unmatched}"
        )
        undeclared = [name for name in emitted if name not in params["events_to_check"]]
        assert undeclared == [], (
            f"[{case['id']}] apcore-python emits {undeclared}, which events_to_check "
            f"does not list: {params['events_to_check']}"
        )


# ---------------------------------------------------------------------------
# D-116: the circuit event reports the DECLARED subscriber type
# ---------------------------------------------------------------------------


class _DeclaredSubscriber:
    """A subscriber whose declared kind is NOT a prefix of its id."""

    event_pattern = "*"

    def __init__(self, subscriber_id: str, subscriber_type: str | None) -> None:
        self.subscriber_id = subscriber_id
        if subscriber_type is not None:
            self.subscriber_type = subscriber_type

    async def on_event(self, event: ApCoreEvent) -> None:
        raise RuntimeError("downstream error")


async def _trip_open(case: dict[str, Any]) -> ApCoreEvent:
    """Drive the breaker to OPEN and return the emitted circuit event."""
    params = case["input"]
    cfg = params["circuit_breaker_config"]
    sub_spec = params["subscriber"]

    emitted: list[ApCoreEvent] = []
    mock_emitter = _make_mock_emitter()
    mock_emitter.emit.side_effect = emitted.append

    cb = CircuitBreakerWrapper(
        subscriber=_DeclaredSubscriber(sub_spec["subscriber_id"], sub_spec.get("subscriber_type")),
        emitter=mock_emitter,
        timeout_ms=cfg["timeout_ms"],
        open_threshold=cfg["open_threshold"],
        recovery_window_ms=cfg["recovery_window_ms"],
    )
    for _ in params["failure_sequence"]:
        await cb.on_event(_make_event())

    opened = [e for e in emitted if e.event_type == case["expected"]["event_emitted"]]
    assert len(opened) == 1, f"[{case['id']}] expected one circuit event, got {[e.event_type for e in emitted]}"
    return opened[0]


class TestCircuitEventReportsTheDeclaredSubscriberType:
    """D-116. The id contains a hyphen and the declared kind is not its prefix.

    That is the whole discriminator: this SDK reported the wrapped subscriber's
    CLASS NAME, and apcore-rust split the id on the first hyphen so
    ``health-alert`` became ``health``. An id without a hyphen, or one whose
    prefix happens to equal the declared kind, is answered identically by every
    wrong implementation and by the right one.
    """

    @pytest.mark.asyncio
    async def test_the_declared_kind_is_reported(self) -> None:
        case = _case("circuit_event_reports_the_declared_subscriber_type")
        event = await _trip_open(case)
        assert event.data["subscriber_type"] == case["expected"]["event_subscriber_type"]


class TestCircuitEventUsesTheDlqDefaultForAnUndeclaredSubscriber:
    """D-116 control: no second default is invented for this surface.

    Asserted as EQUAL TO what the DLQ path reports for the same subscriber
    rather than as a literal — the default is a language-shaped derivation, and
    what is normative is that the two surfaces AGREE. All three SDKs disagreed
    with their own DLQ value, which is the finding.
    """

    @pytest.mark.asyncio
    async def test_it_matches_what_the_dlq_path_reports(self) -> None:
        from apcore.events.emitter import _get_subscriber_type

        case = _case("circuit_event_uses_the_dlq_default_for_an_undeclared_subscriber")
        assert case["expected"]["event_subscriber_type_equals_dlq_subscriber_type"] is True

        sub_spec = case["input"]["subscriber"]
        subscriber = _DeclaredSubscriber(sub_spec["subscriber_id"], sub_spec.get("subscriber_type"))
        event = await _trip_open(case)

        assert event.data["subscriber_type"] == _get_subscriber_type(subscriber)


class TestCircuitEventsCarrySubscriberId:
    """D-145: opening and closing events identify the affected subscriber."""

    @pytest.mark.asyncio
    async def test_opened_event_carries_the_subscriber_id(self) -> None:
        case = _case("circuit_opened_event_carries_the_subscriber_id")
        event = await _trip_open(case)
        assert event.data["subscriber_id"] == case["expected"]["event_subscriber_id"]
        assert event.data["subscriber_type"] == case["expected"]["event_subscriber_type"]

    @pytest.mark.asyncio
    async def test_closed_event_carries_the_subscriber_id(self) -> None:
        case = _case("circuit_closed_event_carries_the_subscriber_id")
        params = case["input"]
        subscriber = MagicMock()
        subscriber.subscriber_id = params["subscriber"]["subscriber_id"]
        subscriber.subscriber_type = params["subscriber"]["subscriber_type"]
        subscriber.event_pattern = "*"
        subscriber.on_event = AsyncMock(return_value=None)
        emitted: list[ApCoreEvent] = []
        emitter = _make_mock_emitter()
        emitter.emit.side_effect = emitted.append
        breaker = CircuitBreakerWrapper(subscriber=subscriber, emitter=emitter)
        breaker._state = _CIRCUIT_STATE_BY_NAME[params["circuit_state"]]

        await breaker.on_event(_make_event())

        closed = [event for event in emitted if event.event_type == case["expected"]["event_emitted"]]
        assert len(closed) == 1
        assert closed[0].data["subscriber_id"] == case["expected"]["event_subscriber_id"]


# ---------------------------------------------------------------------------
# Fixture coverage guard
# ---------------------------------------------------------------------------


class TestFixtureCoverage:
    """Every case in the canonical fixture has a driver class in this file.

    The assertions above are hand-written rather than generated from the
    fixture. That is fine, but the fixture used to be named only in the module
    docstring, so a case added on the spec side left no trace here. This guard
    closes that gap: a new canonical case fails until someone writes the class.
    """

    FIXTURE = "event_management_hardening.json"

    #: canonical case id -> the class in this module that asserts it.
    COVERED: dict[str, str] = {
        "subscriber_factory_registered_type": "TestSubscriberFactoryRegisteredType",
        "builtin_stdout_type": "TestBuiltinStdoutType",
        "builtin_file_type": "TestBuiltinFileType",
        "builtin_filter_passes_matching": "TestBuiltinFilterPassesMatching",
        "builtin_filter_discards_nonmatching": "TestBuiltinFilterDiscardsNonmatching",
        "filter_exclude_question_mark_is_a_wildcard": "TestFilterPatternDialect",
        "filter_exclude_character_class_does_not_expand": "TestFilterPatternDialect",
        "filter_include_question_mark_is_a_wildcard": "TestFilterPatternDialect",
        "circuit_open_after_threshold": "TestCircuitOpenAfterThreshold",
        "circuit_event_reports_the_declared_subscriber_type": "TestCircuitEventReportsTheDeclaredSubscriberType",
        "circuit_event_uses_the_dlq_default_for_an_undeclared_subscriber": "TestCircuitEventUsesTheDlqDefaultForAnUndeclaredSubscriber",
        "circuit_opened_event_carries_the_subscriber_id": "TestCircuitEventsCarrySubscriberId",
        "circuit_closed_event_carries_the_subscriber_id": "TestCircuitEventsCarrySubscriberId",
        "circuit_discards_in_open_state": "TestCircuitDiscardsInOpenState",
        "circuit_half_open_after_window": "TestCircuitHalfOpenAfterWindow",
        "circuit_closes_on_success": "TestCircuitClosesOnSuccess",
        "event_naming_canonical": "TestEventNamingCanonical",
    }

    def test_every_canonical_case_is_claimed(self) -> None:
        canonical = set(case_ids(self.FIXTURE))
        claimed = set(self.COVERED)
        assert canonical - claimed == set(), f"canonical fixture {self.FIXTURE} gained case(s) with no driver here"
        assert claimed - canonical == set(), f"this file claims case(s) {self.FIXTURE} no longer defines"

    def test_every_claimed_class_exists(self) -> None:
        module = sys.modules[__name__]
        missing = [cls for cls in self.COVERED.values() if not hasattr(module, cls)]
        assert missing == [], f"claimed driver class(es) not defined: {missing}"
