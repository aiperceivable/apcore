"""The nested ``retry:`` block in subscriber config is read by every factory (apcore#85).

``docs/features/event-system.md`` documents a per-subscriber ``retry:`` block and
shows it on multiple subscriber types. Before apcore#85 no factory parsed it:
only ``webhook`` built a policy, and only from the legacy flat ``retry_count``
shorthand. An operator copying the documented example got no retry policy at all,
silently.

Every assertion below deliberately uses values that differ from
``EventRetryConfig()`` defaults (``max_attempts=3``, ``initial_backoff_ms=100``,
``max_backoff_ms=30000``, ``backoff_multiplier=2.0``) — otherwise the test would
pass whether or not the config was ever read.
"""

from __future__ import annotations

import asyncio
from typing import Any

import pytest

from apcore.events.emitter import ApCoreEvent, EventEmitter
from apcore.events.retry import EventRetryConfig
from apcore.events.subscribers import (
    A2ASubscriber,
    FileSubscriber,
    FilterSubscriber,
    StdoutSubscriber,
    WebhookSubscriber,
)
from apcore.sys_modules.registration import (
    create_subscriber_from_config,
    register_subscriber_type,
    reset_subscriber_registry,
)

# A policy in which every single field differs from EventRetryConfig() defaults.
NON_DEFAULT_RETRY: dict[str, Any] = {
    "max_attempts": 7,
    "initial_backoff_ms": 250,
    "max_backoff_ms": 10_000,
    "backoff_multiplier": 3.0,
}


def _assert_non_default_policy(retry: EventRetryConfig) -> None:
    """Assert the parsed policy is NON_DEFAULT_RETRY, field by field."""
    default = EventRetryConfig()
    assert retry.max_attempts == 7 != default.max_attempts
    assert retry.initial_backoff_ms == 250 != default.initial_backoff_ms
    assert retry.max_backoff_ms == 10_000 != default.max_backoff_ms
    assert retry.backoff_multiplier == 3.0 != default.backoff_multiplier


@pytest.fixture(autouse=True)
def _clean_registry() -> Any:
    reset_subscriber_registry()
    yield
    reset_subscriber_registry()


# ---------------------------------------------------------------------------
# One case per built-in subscriber type
# ---------------------------------------------------------------------------


class TestNestedRetryBlockPerSubscriberType:
    def test_webhook_reads_nested_retry_block(self) -> None:
        sub = create_subscriber_from_config(
            {
                "type": "webhook",
                "url": "https://example.com/hook",
                "retry": NON_DEFAULT_RETRY,
            }
        )
        assert isinstance(sub, WebhookSubscriber)
        _assert_non_default_policy(sub.retry)

    def test_a2a_reads_nested_retry_block(self) -> None:
        sub = create_subscriber_from_config(
            {
                "type": "a2a",
                "platform_url": "https://platform.example.com",
                "retry": NON_DEFAULT_RETRY,
            }
        )
        assert isinstance(sub, A2ASubscriber)
        _assert_non_default_policy(sub.retry)

    def test_file_reads_nested_retry_block(self, tmp_path: Any) -> None:
        sub = create_subscriber_from_config(
            {
                "type": "file",
                "path": str(tmp_path / "events.jsonl"),
                "retry": NON_DEFAULT_RETRY,
            }
        )
        assert isinstance(sub, FileSubscriber)
        _assert_non_default_policy(sub.retry)

    def test_stdout_reads_nested_retry_block(self) -> None:
        sub = create_subscriber_from_config(
            {
                "type": "stdout",
                "format": "json",
                "retry": NON_DEFAULT_RETRY,
            }
        )
        assert isinstance(sub, StdoutSubscriber)
        _assert_non_default_policy(sub.retry)

    def test_filter_reads_nested_retry_block(self) -> None:
        sub = create_subscriber_from_config(
            {
                "type": "filter",
                "delegate_type": "stdout",
                "delegate_config": {"format": "json"},
                "include_events": ["apcore.error.*"],
                "retry": NON_DEFAULT_RETRY,
            }
        )
        assert isinstance(sub, FilterSubscriber)
        _assert_non_default_policy(sub.retry)


# ---------------------------------------------------------------------------
# Parsing semantics
# ---------------------------------------------------------------------------


class TestNestedRetryBlockSemantics:
    def test_partial_block_merges_over_spec_defaults(self, tmp_path: Any) -> None:
        """The documented `file` example declares only two of the four keys."""
        sub = create_subscriber_from_config(
            {
                "type": "file",
                "path": str(tmp_path / "events.jsonl"),
                "retry": {"max_attempts": 2, "initial_backoff_ms": 50},
            }
        )
        default = EventRetryConfig()
        assert sub.retry.max_attempts == 2 != default.max_attempts
        assert sub.retry.initial_backoff_ms == 50 != default.initial_backoff_ms
        # Unspecified keys keep the spec defaults.
        assert sub.retry.max_backoff_ms == default.max_backoff_ms
        assert sub.retry.backoff_multiplier == default.backoff_multiplier

    def test_absent_block_keeps_spec_defaults(self) -> None:
        sub = create_subscriber_from_config({"type": "stdout"})
        assert sub.retry == EventRetryConfig()

    def test_non_mapping_retry_value_is_ignored(self) -> None:
        """A malformed `retry:` scalar must not crash subscriber construction."""
        sub = create_subscriber_from_config({"type": "stdout", "retry": "aggressive"})
        assert sub.retry == EventRetryConfig()

    def test_flat_retry_count_still_honoured_for_webhook(self) -> None:
        """Deprecated alias: retry_count counted retries AFTER the first attempt."""
        sub = create_subscriber_from_config({"type": "webhook", "url": "https://example.com/hook", "retry_count": 5})
        assert sub.retry.max_attempts == 6

    def test_nested_block_wins_over_flat_retry_count(self) -> None:
        sub = create_subscriber_from_config(
            {
                "type": "webhook",
                "url": "https://example.com/hook",
                "retry_count": 5,
                "retry": NON_DEFAULT_RETRY,
            }
        )
        # retry_count=5 would have produced max_attempts=6; the nested block wins.
        _assert_non_default_policy(sub.retry)

    def test_filter_delegate_reads_its_own_nested_block(self, tmp_path: Any) -> None:
        """A `retry:` inside delegate_config configures the delegate, not the filter."""
        sub = create_subscriber_from_config(
            {
                "type": "filter",
                "delegate_type": "file",
                "delegate_config": {
                    "path": str(tmp_path / "events.jsonl"),
                    "retry": NON_DEFAULT_RETRY,
                },
            }
        )
        assert isinstance(sub, FilterSubscriber)
        assert sub.retry == EventRetryConfig()
        _assert_non_default_policy(sub._delegate.retry)  # type: ignore[attr-defined]


# ---------------------------------------------------------------------------
# End-to-end: the declared policy actually governs delivery
# ---------------------------------------------------------------------------


class TestDeclaredPolicyTakesEffect:
    def test_emitter_honours_config_declared_max_attempts(self) -> None:
        """A config-declared max_attempts drives the real delivery attempt count."""
        attempts = 0

        async def _failing_on_event(_event: ApCoreEvent) -> None:
            nonlocal attempts
            attempts += 1
            raise RuntimeError("transient sink failure")

        sub = create_subscriber_from_config(
            {
                "type": "stdout",
                # 5 differs from the default 3, so a factory that ignored the
                # block would deliver 3 times and fail this assertion.
                "retry": {"max_attempts": 5, "initial_backoff_ms": 0},
            }
        )
        sub.on_event = _failing_on_event  # type: ignore[method-assign]

        emitter = EventEmitter()
        try:
            emitter.subscribe(sub)
            emitter.emit(
                ApCoreEvent(
                    event_type="test.event",
                    module_id="mod.a",
                    timestamp="2026-03-08T00:00:00Z",
                    severity="info",
                    data={},
                )
            )
            emitter.flush(timeout=5.0)
        finally:
            emitter.shutdown()

        assert attempts == 5

    def test_registered_custom_type_can_use_shared_parser(self) -> None:
        """Third-party factories opt in via the same nested block shape."""
        from apcore.sys_modules.registration import _parse_retry_config

        class _CustomSink:
            subscriber_type = "custom_sink"

            def __init__(self, retry: EventRetryConfig | None) -> None:
                self.subscriber_id = "custom-0"
                self.event_pattern = "*"
                self.retry = retry if retry is not None else EventRetryConfig()

            async def on_event(self, event: ApCoreEvent) -> None:
                await asyncio.sleep(0)

        register_subscriber_type("custom_sink", lambda cfg: _CustomSink(_parse_retry_config(cfg)))
        sub = create_subscriber_from_config({"type": "custom_sink", "retry": NON_DEFAULT_RETRY})
        _assert_non_default_policy(sub.retry)


class TestRetryAttributeCoercion:
    """A subscriber's ``retry`` attribute declared as a plain dict must be honoured.

    Sync finding A-D-024. ``_get_retry_config`` was
    ``retry if isinstance(retry, EventRetryConfig) else EventRetryConfig()`` —
    a dict failed the isinstance check and the declared policy was silently
    replaced by the defaults. ``docs/features/event-system.md`` shows exactly
    that dict form in its own Python example, so a subscriber copied from the
    spec retried 3x/100ms instead of the declared 5x/250ms.

    Values below differ from every default so the assertions cannot pass by
    coincidence.
    """

    def test_spec_example_dict_is_honoured(self) -> None:
        from apcore.events.emitter import _get_retry_config

        class HealthAlertSubscriber:
            event_pattern = "apcore.health.*"
            retry = {"max_attempts": 5, "initial_backoff_ms": 250}

            async def on_event(self, event: Any) -> None: ...

        cfg = _get_retry_config(HealthAlertSubscriber())
        assert cfg.max_attempts == 5
        assert cfg.initial_backoff_ms == 250

    def test_partial_dict_merges_over_defaults(self) -> None:
        from apcore.events.emitter import _get_retry_config
        from apcore.events.retry import EventRetryConfig

        class S:
            retry = {"max_attempts": 7}

            async def on_event(self, event: Any) -> None: ...

        cfg = _get_retry_config(S())
        assert cfg.max_attempts == 7
        # Unspecified keys keep their defaults, matching TypeScript resolveRetry.
        assert cfg.initial_backoff_ms == EventRetryConfig().initial_backoff_ms
        assert cfg.backoff_multiplier == EventRetryConfig().backoff_multiplier

    def test_typed_config_still_passes_through(self) -> None:
        from apcore.events.emitter import _get_retry_config
        from apcore.events.retry import EventRetryConfig

        declared = EventRetryConfig(max_attempts=9, initial_backoff_ms=500)

        class S:
            retry = declared

            async def on_event(self, event: Any) -> None: ...

        assert _get_retry_config(S()) is declared

    def test_absent_retry_uses_defaults(self) -> None:
        from apcore.events.emitter import _get_retry_config
        from apcore.events.retry import EventRetryConfig

        class S:
            async def on_event(self, event: Any) -> None: ...

        assert _get_retry_config(S()) == EventRetryConfig()

    def test_unknown_key_warns_and_is_ignored(self, caplog: Any) -> None:
        from apcore.events.emitter import _get_retry_config

        class S:
            subscriber_id = "typo-sub"
            retry = {"max_attempts": 4, "max_attemps": 99}  # typo'd second key

            async def on_event(self, event: Any) -> None: ...

        with caplog.at_level("WARNING"):
            cfg = _get_retry_config(S())
        assert cfg.max_attempts == 4
        assert "max_attemps" in caplog.text, "a typo'd retry key must not vanish silently"

    def test_wrong_type_warns_and_falls_back(self, caplog: Any) -> None:
        from apcore.events.emitter import _get_retry_config
        from apcore.events.retry import EventRetryConfig

        class S:
            subscriber_id = "bad-sub"
            retry = "5 times please"

            async def on_event(self, event: Any) -> None: ...

        with caplog.at_level("WARNING"):
            cfg = _get_retry_config(S())
        assert cfg == EventRetryConfig()
        assert "bad-sub" in caplog.text
