"""Spec-traced contract tests for the APCore unified client.

Generated from /apcore/docs/features/apcore-client.md ('## Contract:' blocks).
Each test carries its verbatim clause id (formatted
``apcore_client.<method>.<kind>.<detail>``) in a leading docstring so that
cross-language test rows line up.

TESTS ONLY — production source is never modified here.

Cross-language note on error TYPES
----------------------------------
The feature spec's "Error Behavior" table lists ``RuntimeError`` (Python) for
``on()``/``off()``/``disable()``/``enable()`` when sys_modules/events are not
enabled. The authoritative Python SDK source has since upgraded this to the
typed ``SysModulesDisabledError`` (``code == "SYS_MODULES_DISABLED"``) — see
``src/apcore/errors.py`` and the per-method docstrings in ``src/apcore/client.py``
which explicitly state it "Replaces bare ``RuntimeError``". These contract
tests assert the REAL type + code the SDK raises (the source of truth), which
remains dispatchable cross-language via the stable ``SYS_MODULES_DISABLED``
code.
"""

from __future__ import annotations

import asyncio
import inspect
from typing import Any

import pytest

from apcore.client import APCore, _CallbackSubscriber
from apcore.config import Config
from apcore.context import Context
from apcore.errors import (
    InvalidInputError,
    ModuleExecuteError,
    ModuleNotFoundError,
    SysModulesDisabledError,
)
from apcore.events.emitter import ApCoreEvent
from apcore.middleware.base import Middleware


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _sys_config() -> Config:
    """Config with sys_modules + events enabled (production-like client)."""
    return Config(
        {
            "sys_modules": {
                "enabled": True,
                "events": {"enabled": True},
            },
        }
    )


def _client_with_module(module_id: str = "math.add") -> APCore:
    """Zero-config client with a single deterministic add module registered."""
    client = APCore()

    @client.module(id=module_id, description="Add two numbers")
    def add(a: int = 0, b: int = 0) -> dict:
        return {"sum": a + b}

    return client


class _NoopMiddleware(Middleware):
    """Minimal class-based middleware with a configurable priority."""

    def __init__(self, name: str = "noop", priority: int = 0) -> None:
        self.name = name
        self.priority = priority

    async def before(self, context: Context) -> None:  # pragma: no cover - trivial
        return None

    async def after(self, context: Context, result: dict) -> dict:  # pragma: no cover
        return result


# ===========================================================================
# Contract: ApCoreClient.call
# ===========================================================================


def test_call_input_module_id_invalid_pattern() -> None:
    """apcore_client.call.input.module_id.invalid_pattern

    Malformed module_id -> InvalidInputError(code=INVALID_MODULE_ID).
    """
    client = _client_with_module()
    with pytest.raises(InvalidInputError) as exc:
        client.call("!!not a valid id!!", {"a": 1, "b": 2})
    assert exc.value.code == "INVALID_MODULE_ID"


def test_call_input_module_id_empty() -> None:
    """apcore_client.call.input.module_id.empty

    Empty module_id -> InvalidInputError(code=INVALID_MODULE_ID).
    """
    client = _client_with_module()
    with pytest.raises(InvalidInputError) as exc:
        client.call("", {"a": 1, "b": 2})
    assert exc.value.code == "INVALID_MODULE_ID"


def test_call_error_invalid_module_id() -> None:
    """apcore_client.call.error.INVALID_MODULE_ID"""
    client = _client_with_module()
    with pytest.raises(InvalidInputError) as exc:
        client.call("UPPER.Reserved Bad", {})
    assert exc.value.code == "INVALID_MODULE_ID"


def test_call_error_module_not_found() -> None:
    """apcore_client.call.error.MODULE_NOT_FOUND"""
    client = _client_with_module()
    with pytest.raises(ModuleNotFoundError) as exc:
        client.call("missing.module", {"a": 1})
    assert exc.value.code == "MODULE_NOT_FOUND"


def test_call_error_schema_validation_error() -> None:
    """apcore_client.call.error.SCHEMA_VALIDATION_ERROR

    Inputs that violate the module's input_schema -> SCHEMA_VALIDATION_ERROR.
    """
    client = APCore()

    @client.module(id="math.strict", description="Strict typed add")
    def strict_add(a: int, b: int) -> dict:
        return {"sum": a + b}

    from apcore.errors import SchemaValidationError

    with pytest.raises(SchemaValidationError) as exc:
        client.call("math.strict", {"a": "not-an-int", "b": 2})
    assert exc.value.code == "SCHEMA_VALIDATION_ERROR"


def test_call_error_handler_propagates_unchanged() -> None:
    """apcore_client.call.error.handler_propagates

    A raw (non-ModuleError) exception raised by the module's execute handler is
    standardized by error propagation (Algorithm A11) into a ModuleExecuteError
    (code MODULE_EXECUTE_ERROR) whose original cause is preserved unchanged on
    ``__cause__`` / ``.cause`` and echoed in the message.
    """
    client = APCore()
    sentinel = RuntimeError("boom-from-handler")

    @client.module(id="util.raiser", description="Always raises")
    def raiser() -> dict:
        raise sentinel

    with pytest.raises(ModuleExecuteError) as exc:
        client.call("util.raiser", {})
    assert exc.value.code == "MODULE_EXECUTE_ERROR"
    assert "boom-from-handler" in str(exc.value)
    # Original handler exception preserved unchanged as the cause.
    assert exc.value.__cause__ is sentinel
    assert exc.value.cause is sentinel


async def test_call_property_async() -> None:
    """apcore_client.call.property.async

    Python exposes an async surface call_async() that is awaitable and resolves.
    """
    client = _client_with_module()
    coro = client.call_async("math.add", {"a": 10, "b": 5})
    assert inspect.isawaitable(coro)
    result = await coro
    assert result == {"sum": 15}


async def test_call_property_thread_safe() -> None:
    """apcore_client.call.property.thread_safe

    >=8 concurrent call_async() with distinct inputs -> no exception,
    each result consistent with its own input.
    """
    client = _client_with_module()
    pairs = [(i, i * 2) for i in range(10)]
    results = await asyncio.gather(*[client.call_async("math.add", {"a": a, "b": b}) for a, b in pairs])
    assert [r["sum"] for r in results] == [a + b for a, b in pairs]


def test_call_property_idempotent_false() -> None:
    """apcore_client.call.property.idempotent_false

    call is declared NOT idempotent: a stateful counter module returns
    different observable output on the second identical call.
    """
    client = APCore()
    state = {"n": 0}

    @client.module(id="util.counter", description="Increments a counter")
    def counter() -> dict:
        state["n"] += 1
        return {"n": state["n"]}

    first = client.call("util.counter", {})
    second = client.call("util.counter", {})
    assert first != second
    assert (first["n"], second["n"]) == (1, 2)


# ===========================================================================
# Contract: ApCoreClient.start   (MISSING SYMBOL — contract gap)
# ===========================================================================


@pytest.mark.skip(reason="missing symbol: APCore.start (contract gap)")
def test_start_error_config_invalid() -> None:
    """apcore_client.start.error.CONFIG_INVALID

    Spec declares APCore.start raising ConfigError(code=CONFIG_INVALID) on
    startup validation failure, but the Python APCore class exposes no
    start() method (lifecycle is implicit at construction). Recorded as a
    cross-language gap skip.
    """
    APCore().start()  # type: ignore[attr-defined]


@pytest.mark.skip(reason="missing symbol: APCore.start (contract gap)")
def test_start_property_idempotent_false() -> None:
    """apcore_client.start.property.idempotent_false

    No start() method exists on the Python APCore client.
    """
    APCore().start()  # type: ignore[attr-defined]


# ===========================================================================
# Contract: ApCoreClient.stop   (MISSING SYMBOL — contract gap)
# ===========================================================================


@pytest.mark.skip(reason="missing symbol: APCore.stop (contract gap)")
def test_stop_property_idempotent_true() -> None:
    """apcore_client.stop.property.idempotent_true

    Spec declares APCore.stop is idempotent (multiple stops safe), but the
    Python APCore client exposes no stop() method. The nearest analogue is
    close(), but the contract names stop() specifically. Recorded as a gap.
    """
    client = APCore()
    client.stop()  # type: ignore[attr-defined]
    client.stop()  # type: ignore[attr-defined]


# ===========================================================================
# Contract: APCoreClient.on
# ===========================================================================


def test_on_input_event_type_empty_filters_no_match() -> None:
    """apcore_client.on.input.event_type.non_empty

    event_type is matched by exact equality inside the subscriber; an empty
    event_type subscription never fires for a real, non-empty event type.
    """
    client = APCore(config=_sys_config())
    received: list[ApCoreEvent] = []
    client.on("", lambda e: received.append(e))
    assert client.events is not None
    client.events.emit(
        ApCoreEvent(
            event_type="apcore.registry.module_registered",
            module_id=None,
            timestamp="2026-01-01T00:00:00Z",
            severity="info",
            data={},
        )
    )
    client.events.flush()
    assert received == []


def test_on_error_runtime_without_events() -> None:
    """apcore_client.on.error.SYS_MODULES_DISABLED

    on() without events enabled raises the typed SysModulesDisabledError
    (code SYS_MODULES_DISABLED). This is the SDK's upgrade of the spec's
    nominal RuntimeError; the stable code keeps it cross-language dispatchable.
    """
    client = APCore()
    with pytest.raises(SysModulesDisabledError) as exc:
        client.on("apcore.health.error_threshold_exceeded", lambda e: None)
    assert exc.value.code == "SYS_MODULES_DISABLED"
    assert "Events are not enabled" in str(exc.value)


def test_on_property_thread_safe() -> None:
    """apcore_client.on.property.thread_safe

    >=8 concurrent subscriptions with distinct handlers -> all registered,
    all fire for the matching event, no exception.
    """
    client = APCore(config=_sys_config())
    counters = [0] * 10

    def make(idx: int):
        def handler(_e: ApCoreEvent) -> None:
            counters[idx] += 1

        return handler

    async def subscribe_all() -> None:
        await asyncio.gather(*[asyncio.to_thread(client.on, "evt.shared", make(i)) for i in range(10)])

    asyncio.run(subscribe_all())
    assert client.events is not None
    client.events.emit(
        ApCoreEvent(
            event_type="evt.shared",
            module_id=None,
            timestamp="2026-01-01T00:00:00Z",
            severity="info",
            data={},
        )
    )
    client.events.flush()
    assert counters == [1] * 10


def test_on_property_idempotent_false() -> None:
    """apcore_client.on.property.idempotent_false

    Registering the same handler twice creates two independent subscriptions
    that each fire (handler invoked twice per event).
    """
    client = APCore(config=_sys_config())
    calls: list[int] = []
    handler = lambda _e: calls.append(1)  # noqa: E731
    client.on("evt.dup", handler)
    client.on("evt.dup", handler)
    assert client.events is not None
    client.events.emit(
        ApCoreEvent(
            event_type="evt.dup",
            module_id=None,
            timestamp="2026-01-01T00:00:00Z",
            severity="info",
            data={},
        )
    )
    client.events.flush()
    assert len(calls) == 2


def test_on_returns_event_subscriber() -> None:
    """apcore_client.on.returns.subscriber

    on() returns the created subscriber object usable with off().
    """
    client = APCore(config=_sys_config())
    sub = client.on("evt.x", lambda _e: None)
    assert isinstance(sub, _CallbackSubscriber)
    assert sub.event_type == "evt.x"


# ===========================================================================
# Contract: APCoreClient.off
# ===========================================================================


def test_off_error_runtime_without_events() -> None:
    """apcore_client.off.error.SYS_MODULES_DISABLED

    off() without events enabled raises SysModulesDisabledError (same guard as
    on()). Spec nominal type RuntimeError -> typed in source; stable code.
    """
    client = APCore()
    with pytest.raises(SysModulesDisabledError) as exc:
        client.off(object())  # type: ignore[arg-type]
    assert exc.value.code == "SYS_MODULES_DISABLED"


def test_off_property_idempotent_true() -> None:
    """apcore_client.off.property.idempotent_true

    Unsubscribing the same subscriber twice is a no-op the second time
    (no error, handler no longer fires).
    """
    client = APCore(config=_sys_config())
    calls: list[int] = []
    sub = client.on("evt.off", lambda _e: calls.append(1))
    client.off(sub)
    client.off(sub)  # second off() must not raise
    assert client.events is not None
    client.events.emit(
        ApCoreEvent(
            event_type="evt.off",
            module_id=None,
            timestamp="2026-01-01T00:00:00Z",
            severity="info",
            data={},
        )
    )
    client.events.flush()
    assert calls == []


def test_off_property_thread_safe() -> None:
    """apcore_client.off.property.thread_safe

    >=8 concurrent unsubscribes of distinct subscribers -> no exception, all
    removed (no handler fires afterwards).
    """
    client = APCore(config=_sys_config())
    calls = [0] * 10
    subs = []
    for i in range(10):

        def make(idx: int):
            return lambda _e: calls.__setitem__(idx, calls[idx] + 1)

        subs.append(client.on("evt.toff", make(i)))

    async def unsubscribe_all() -> None:
        await asyncio.gather(*[asyncio.to_thread(client.off, s) for s in subs])

    asyncio.run(unsubscribe_all())
    assert client.events is not None
    client.events.emit(
        ApCoreEvent(
            event_type="evt.toff",
            module_id=None,
            timestamp="2026-01-01T00:00:00Z",
            severity="info",
            data={},
        )
    )
    client.events.flush()
    assert calls == [0] * 10


# ===========================================================================
# Contract: APCoreClient.stream
# ===========================================================================


def test_stream_input_module_id_invalid() -> None:
    """apcore_client.stream.input.module_id.invalid_pattern

    Malformed module_id -> InvalidInputError(code=INVALID_MODULE_ID) raised
    before the pipeline starts (awaiting the generator).
    """
    client = _client_with_module()

    async def drive() -> None:
        async for _ in client.stream("!!bad!!", {}):
            pass

    with pytest.raises(InvalidInputError) as exc:
        asyncio.run(drive())
    assert exc.value.code == "INVALID_MODULE_ID"


def test_stream_error_module_not_found() -> None:
    """apcore_client.stream.error.MODULE_NOT_FOUND"""
    client = _client_with_module()

    async def drive() -> None:
        async for _ in client.stream("not.registered", {}):
            pass

    with pytest.raises(ModuleNotFoundError) as exc:
        asyncio.run(drive())
    assert exc.value.code == "MODULE_NOT_FOUND"


async def test_stream_error_invalid_module_id_empty() -> None:
    """apcore_client.stream.error.INVALID_MODULE_ID"""
    client = _client_with_module()
    with pytest.raises(InvalidInputError) as exc:
        async for _ in client.stream("", {}):
            pass
    assert exc.value.code == "INVALID_MODULE_ID"


async def test_stream_property_async() -> None:
    """apcore_client.stream.property.async

    stream() is an async generator; iterating yields dict chunks (fallback to
    a single execute() chunk for non-streaming modules).
    """
    client = _client_with_module()
    chunks = [chunk async for chunk in client.stream("math.add", {"a": 3, "b": 4})]
    assert len(chunks) >= 1
    assert all(isinstance(c, dict) for c in chunks)
    assert chunks[-1] == {"sum": 7}


async def test_stream_property_thread_safe() -> None:
    """apcore_client.stream.property.thread_safe

    >=8 concurrent streams with distinct inputs -> no exception, each final
    chunk consistent with its own input.
    """
    client = _client_with_module()

    async def collect(a: int, b: int) -> dict:
        last: dict = {}
        async for chunk in client.stream("math.add", {"a": a, "b": b}):
            last = chunk
        return last

    pairs = [(i, i + 1) for i in range(8)]
    results = await asyncio.gather(*[collect(a, b) for a, b in pairs])
    assert [r["sum"] for r in results] == [a + b for a, b in pairs]


# ===========================================================================
# Contract: APCoreClient.validate
# ===========================================================================


def test_validate_input_module_id_invalid() -> None:
    """apcore_client.validate.input.module_id.invalid_pattern

    validate() is non-throwing (A-D-008): a malformed module_id does NOT raise.
    The module_id format check fails as the first check, the overall result is
    valid=False, and the failed check carries the INVALID_MODULE_ID error.
    """
    client = _client_with_module()
    result = client.validate("!!bad id!!", {})
    assert result.valid is False
    module_id_check = result.checks[0]
    assert module_id_check.check == "module_id"
    assert module_id_check.passed is False
    assert module_id_check.error is not None
    assert module_id_check.error["code"] == "INVALID_MODULE_ID"


def test_validate_error_invalid_module_id_empty() -> None:
    """apcore_client.validate.error.INVALID_MODULE_ID

    An empty module_id is captured as a failed module_id check (non-throwing),
    surfacing INVALID_MODULE_ID via PreflightResult.errors rather than raising.
    """
    client = _client_with_module()
    result = client.validate("", {})
    assert result.valid is False
    assert any(e.get("code") == "INVALID_MODULE_ID" for e in result.errors)


def test_validate_no_raise_on_validation_failure() -> None:
    """apcore_client.validate.error.no_raise_on_failure

    Validation failures are captured in PreflightResult, not raised. A missing
    module yields valid=False with a recorded check rather than an exception.
    """
    client = _client_with_module()
    result = client.validate("absent.module", {})
    assert result.valid is False
    assert any(not c.passed for c in result.checks)


def test_validate_returns_preflight_result_shape() -> None:
    """apcore_client.validate.returns.preflight_result

    PreflightResult exposes valid(bool), checks(list, 6 evaluable steps for a fully
    passing dry-run: module_id, context, call_chain, module_lookup, acl,
    schema), requires_approval(bool),
    errors(list).
    """
    client = _client_with_module()
    result = client.validate("math.add", {"a": 1, "b": 2})
    assert isinstance(result.valid, bool)
    assert isinstance(result.checks, list)
    assert [check.check for check in result.checks] == [
        "module_id",
        "context",
        "call_chain",
        "module_lookup",
        "acl",
        "schema",
    ]
    assert isinstance(result.requires_approval, bool)
    assert isinstance(result.errors, list)


def test_validate_property_pure_no_mutation() -> None:
    """apcore_client.validate.property.pure

    validate() does not mutate observable client state: the module list is
    unchanged across calls (no execution, no registration side effects).
    """
    client = _client_with_module()
    before = client.list_modules()
    client.validate("math.add", {"a": 1, "b": 2})
    after = client.list_modules()
    assert before == after


def test_validate_property_idempotent_true() -> None:
    """apcore_client.validate.property.idempotent_true

    Repeated calls with identical arguments return equivalent results.
    """
    client = _client_with_module()
    r1 = client.validate("math.add", {"a": 1, "b": 2})
    r2 = client.validate("math.add", {"a": 1, "b": 2})
    assert r1.valid == r2.valid
    assert len(r1.checks) == len(r2.checks)


async def test_validate_property_thread_safe() -> None:
    """apcore_client.validate.property.thread_safe

    >=8 concurrent validate() calls -> no exception, all report valid=True for
    a valid module/input pair.
    """
    client = _client_with_module()
    results = await asyncio.gather(
        *[asyncio.to_thread(client.validate, "math.add", {"a": i, "b": i}) for i in range(8)]
    )
    assert all(r.valid for r in results)


# ===========================================================================
# Contract: APCoreClient.disable
# ===========================================================================


def test_disable_error_runtime_without_sys_modules() -> None:
    """apcore_client.disable.error.SYS_MODULES_DISABLED

    disable() without sys_modules raises SysModulesDisabledError
    (code SYS_MODULES_DISABLED). Spec nominal RuntimeError -> typed in source.
    """
    client = APCore()
    with pytest.raises(SysModulesDisabledError) as exc:
        client.disable("some.module")
    assert exc.value.code == "SYS_MODULES_DISABLED"
    assert "disable() requires sys_modules" in str(exc.value)


def test_disable_input_reason_default() -> None:
    """apcore_client.disable.input.reason.default

    With sys_modules enabled, disable() returns a result dict reporting the
    module disabled (enabled=False). reason defaults when omitted.
    """
    client = APCore(config=_sys_config())

    @client.module(id="risky.module", description="A risky module")
    def risky() -> dict:
        return {}

    result = client.disable("risky.module")
    assert isinstance(result, dict)
    assert result.get("module_id") == "risky.module"
    assert result.get("enabled") is False


def test_disable_property_idempotent_true() -> None:
    """apcore_client.disable.property.idempotent_true

    Disabling an already-disabled module succeeds without error and remains
    disabled (enabled stays False).
    """
    client = APCore(config=_sys_config())

    @client.module(id="risky.dup", description="risky")
    def risky() -> dict:
        return {}

    first = client.disable("risky.dup")
    second = client.disable("risky.dup")
    assert first.get("enabled") is False
    assert second.get("enabled") is False


# ===========================================================================
# Contract: APCoreClient.enable
# ===========================================================================


def test_enable_error_runtime_without_sys_modules() -> None:
    """apcore_client.enable.error.SYS_MODULES_DISABLED

    enable() without sys_modules raises SysModulesDisabledError
    (code SYS_MODULES_DISABLED).
    """
    client = APCore()
    with pytest.raises(SysModulesDisabledError) as exc:
        client.enable("some.module")
    assert exc.value.code == "SYS_MODULES_DISABLED"
    assert "enable() requires sys_modules" in str(exc.value)


def test_enable_input_reason_default() -> None:
    """apcore_client.enable.input.reason.default

    enable() returns a result dict reporting the module enabled (enabled=True).
    """
    client = APCore(config=_sys_config())

    @client.module(id="risky.enable", description="risky")
    def risky() -> dict:
        return {}

    client.disable("risky.enable")
    result = client.enable("risky.enable")
    assert isinstance(result, dict)
    assert result.get("module_id") == "risky.enable"
    assert result.get("enabled") is True


def test_enable_property_idempotent_true() -> None:
    """apcore_client.enable.property.idempotent_true

    Enabling an already-enabled module succeeds without error (enabled stays
    True).
    """
    client = APCore(config=_sys_config())

    @client.module(id="risky.reenable", description="risky")
    def risky() -> dict:
        return {}

    first = client.enable("risky.reenable")
    second = client.enable("risky.reenable")
    assert first.get("enabled") is True
    assert second.get("enabled") is True


# ===========================================================================
# Contract: APCore.__init__
# ===========================================================================


def test_init_zero_config_creates_registry_and_executor() -> None:
    """apcore_client.__init__.input.zero_config

    Zero-config APCore() auto-creates a functional Registry and Executor and
    runs with no system modules (events is None).
    """
    client = APCore()
    assert client.registry is not None
    assert client.executor is not None
    assert client.events is None


def test_init_no_error_on_sys_modules_failure() -> None:
    """apcore_client.__init__.error.no_raise

    __init__ never raises for sys_modules registration failure; construction
    completes and the instance is usable.
    """
    # A config with sys_modules disabled constructs cleanly with no context.
    client = APCore(config=Config({"sys_modules": {"enabled": False}}))
    assert isinstance(client, APCore)
    assert client.events is None


def test_init_property_async_false() -> None:
    """apcore_client.__init__.property.async_false

    Construction is synchronous: __init__ is an ordinary (non-coroutine)
    function and APCore() returns an instance directly.
    """
    assert not inspect.iscoroutinefunction(APCore.__init__)
    assert isinstance(APCore(), APCore)


def test_init_property_pure_false_registers_sys_modules() -> None:
    """apcore_client.__init__.property.pure_false

    With sys_modules enabled, construction has side effects: system modules are
    registered and the events emitter becomes available.
    """
    client = APCore(config=_sys_config())
    assert client.events is not None
    assert any(mid.startswith("system.") for mid in client.list_modules())


# ===========================================================================
# Contract: APCore.module
# ===========================================================================


def test_module_input_id_invalid_pattern() -> None:
    """apcore_client.module.input.id.invalid_pattern

    Providing a malformed id -> InvalidInputError(code=INVALID_MODULE_ID).
    """
    client = APCore()
    with pytest.raises(InvalidInputError) as exc:

        @client.module(id="Bad ID With Spaces")
        def f() -> dict:
            return {}

    assert exc.value.code == "INVALID_MODULE_ID"


def test_module_error_duplicate_registration() -> None:
    """apcore_client.module.error.duplicate

    Registering the same id twice raises InvalidInputError on the second.
    """
    client = APCore()

    @client.module(id="dup.module")
    def f1() -> dict:
        return {}

    with pytest.raises(InvalidInputError):

        @client.module(id="dup.module")
        def f2() -> dict:
            return {}


def test_module_returns_original_function() -> None:
    """apcore_client.module.returns.original_function

    The decorator returns the original callable, unchanged and still callable.
    """
    client = APCore()

    @client.module(id="math.ret")
    def add(a: int, b: int) -> int:
        return a + b

    assert callable(add)
    assert add(2, 3) == 5
    assert client.registry.has("math.ret")


def test_module_property_idempotent_false() -> None:
    """apcore_client.module.property.idempotent_false

    Applying the decorator twice registers two entries / raises on the second;
    it is not idempotent. First registration succeeds, count increments by one.
    """
    client = APCore()
    before = len(client.list_modules())

    @client.module(id="once.module")
    def f() -> dict:
        return {}

    after = len(client.list_modules())
    assert after == before + 1


# ===========================================================================
# Contract: APCore.register
# ===========================================================================


def test_register_input_module_id_invalid() -> None:
    """apcore_client.register.input.module_id.invalid_pattern

    Malformed module_id -> InvalidInputError(code=INVALID_MODULE_ID).
    """
    client = APCore()
    other = _client_with_module("math.add")
    module_obj = other.registry.get("math.add")
    with pytest.raises(InvalidInputError) as exc:
        client.register("Bad Id!!", module_obj)
    assert exc.value.code == "INVALID_MODULE_ID"


def test_register_error_duplicate_id() -> None:
    """apcore_client.register.error.INVALID_MODULE_ID

    Registering twice under the same id raises InvalidInputError on the second.
    The duplicate is reported with the dedicated DUPLICATE_MODULE_ID code (the
    generic INVALID_MODULE_ID code is reserved for malformed ids).
    """
    src = _client_with_module("math.add")
    module_obj = src.registry.get("math.add")

    client = APCore()
    client.register("math.copy", module_obj)
    with pytest.raises(InvalidInputError) as exc:
        client.register("math.copy", module_obj)
    assert exc.value.code == "DUPLICATE_MODULE_ID"


def test_register_returns_none_and_registers() -> None:
    """apcore_client.register.returns.none

    register() returns None and the module becomes present in the registry.
    """
    src = _client_with_module("math.add")
    module_obj = src.registry.get("math.add")

    client = APCore()
    ret = client.register("math.target", module_obj)
    assert ret is None
    assert client.registry.has("math.target")


def test_register_property_thread_safe() -> None:
    """apcore_client.register.property.thread_safe

    >=8 concurrent registrations of distinct module ids -> no exception, all
    present in the final registry.
    """
    src = _client_with_module("math.add")
    module_obj = src.registry.get("math.add")
    client = APCore()
    ids = [f"svc.mod{i}" for i in range(10)]

    async def register_all() -> None:
        await asyncio.gather(*[asyncio.to_thread(client.register, mid, module_obj) for mid in ids])

    asyncio.run(register_all())
    for mid in ids:
        assert client.registry.has(mid)


# ===========================================================================
# Contract: APCore.discover
# ===========================================================================


def test_discover_error_config_not_found(tmp_path: Any) -> None:
    """apcore_client.discover.error.ConfigNotFoundError

    A configured extension root that does not exist -> ConfigNotFoundError.
    """
    from apcore.errors import ConfigNotFoundError

    missing = tmp_path / "does_not_exist"
    client = APCore(config=Config({"extensions": {"root": str(missing)}}))
    with pytest.raises(ConfigNotFoundError):
        client.discover()


def test_discover_returns_int_count(tmp_path: Any) -> None:
    """apcore_client.discover.returns.int_count

    discover() returns an int count of modules registered (0 for an empty but
    existing root).

    The extension root is wired through a Registry constructed with the config
    (the supported path); APCore(config=...) directs config to the executor and
    leaves the registry on its default root, so the registry is configured
    explicitly and injected via ``registry=``.
    """
    from apcore.registry import Registry

    root = tmp_path / "extensions"
    root.mkdir()
    registry = Registry(config=Config({"extensions": {"root": str(root)}}))
    client = APCore(registry=registry)
    count = client.discover()
    assert isinstance(count, int)
    assert count == 0


# ===========================================================================
# Contract: APCore.list_modules
# ===========================================================================


def test_list_modules_returns_sorted() -> None:
    """apcore_client.list_modules.returns.sorted

    Returns an alphabetically sorted list of module ids.
    """
    client = APCore()
    for mid in ["c.three", "a.one", "b.two"]:

        @client.module(id=mid)
        def f() -> dict:  # noqa: B023
            return {}

    result = client.list_modules()
    assert result == sorted(result)
    assert {"a.one", "b.two", "c.three"} <= set(result)


def test_list_modules_prefix_filter() -> None:
    """apcore_client.list_modules.input.prefix

    prefix filtering returns only ids starting with the prefix.
    """
    client = APCore()
    for mid in ["math.add", "math.sub", "text.upper"]:

        @client.module(id=mid)
        def f() -> dict:  # noqa: B023
            return {}

    result = client.list_modules(prefix="math.")
    assert set(result) == {"math.add", "math.sub"}


def test_list_modules_property_pure() -> None:
    """apcore_client.list_modules.property.pure

    Read-only: repeated calls do not mutate state and return equal results;
    each call returns a fresh list (mutating it does not affect the next call).
    """
    client = _client_with_module()
    r1 = client.list_modules()
    r1.append("injected.fake")
    r2 = client.list_modules()
    assert "injected.fake" not in r2


def test_list_modules_property_idempotent() -> None:
    """apcore_client.list_modules.property.idempotent

    Two consecutive calls return equal results.
    """
    client = _client_with_module()
    assert client.list_modules() == client.list_modules()


# ===========================================================================
# Contract: APCore.describe
# ===========================================================================


def test_describe_error_module_not_found() -> None:
    """apcore_client.describe.error.ModuleNotFoundError

    describe() on an unregistered module raises ModuleNotFoundError.
    """
    client = APCore()
    with pytest.raises(ModuleNotFoundError) as exc:
        client.describe("not.registered")
    assert exc.value.code == "MODULE_NOT_FOUND"


def test_describe_returns_markdown_string() -> None:
    """apcore_client.describe.returns.string

    Returns a non-empty Markdown description string for a registered module.
    """
    client = _client_with_module()
    text = client.describe("math.add")
    assert isinstance(text, str)
    assert len(text) > 0
    assert "math.add" in text or "Add two numbers" in text


def test_describe_property_pure() -> None:
    """apcore_client.describe.property.pure

    Read-only: describing a module does not alter the module list.
    """
    client = _client_with_module()
    before = client.list_modules()
    client.describe("math.add")
    assert client.list_modules() == before


def test_describe_property_idempotent() -> None:
    """apcore_client.describe.property.idempotent

    Two consecutive describe() calls return identical strings.
    """
    client = _client_with_module()
    assert client.describe("math.add") == client.describe("math.add")


# ===========================================================================
# Contract: APCore.use / APCore.use_middleware
# ===========================================================================


def test_use_error_priority_exceeds_max() -> None:
    """apcore_client.use.error.priority_exceeds_1000

    Middleware with priority > 1000 -> ValueError.
    """
    client = APCore()
    with pytest.raises(ValueError):
        client.use(_NoopMiddleware(name="too-high", priority=1001))


def test_use_returns_self_for_chaining() -> None:
    """apcore_client.use.returns.self

    use() returns the client instance so calls chain.
    """
    client = APCore()
    returned = client.use(_NoopMiddleware(name="a")).use(_NoopMiddleware(name="b"))
    assert returned is client


def test_use_property_idempotent_false() -> None:
    """apcore_client.use.property.idempotent_false

    Adding the same middleware instance twice inserts it twice; remove() then
    only strips one identity occurrence, so it remains findable once more.
    """
    client = APCore()
    mw = _NoopMiddleware(name="dup-mw")
    client.use(mw)
    client.use(mw)
    # First removal succeeds (one of the two insertions).
    assert client.remove(mw) is True
    # Second removal still finds the remaining duplicate insertion.
    assert client.remove(mw) is True


# ===========================================================================
# Contract: APCore.use_before
# ===========================================================================


def test_use_before_returns_self() -> None:
    """apcore_client.use_before.returns.self

    use_before() returns the client for chaining.
    """
    client = APCore()
    returned = client.use_before(lambda ctx: None)
    assert returned is client


async def test_use_before_invoked_before_execute() -> None:
    """apcore_client.use_before.side_effect.1.before_execute

    The registered before-callback runs before module execute (observed via an
    ordered log captured during a real call).
    """
    client = APCore()
    order: list[str] = []

    @client.module(id="ord.mod", description="ordered")
    def mod() -> dict:
        order.append("execute")
        return {}

    # Before-callbacks receive (module_id, inputs, context); returning None
    # leaves the inputs unchanged.
    client.use_before(lambda module_id, inputs, context: order.append("before"))
    client.call("ord.mod", {})
    assert order == ["before", "execute"]


def test_use_before_property_idempotent_false() -> None:
    """apcore_client.use_before.property.idempotent_false

    Registering the same callback twice inserts two wrappers; it fires twice
    per execution.
    """
    client = APCore()
    fired: list[int] = []

    @client.module(id="ord.dup", description="ordered")
    def mod() -> dict:
        return {}

    cb = lambda module_id, inputs, context: fired.append(1)  # noqa: E731
    client.use_before(cb)
    client.use_before(cb)
    client.call("ord.dup", {})
    assert len(fired) == 2


# ===========================================================================
# Contract: APCore.use_after
# ===========================================================================


def test_use_after_returns_self() -> None:
    """apcore_client.use_after.returns.self

    use_after() returns the client for chaining.
    """
    client = APCore()
    returned = client.use_after(lambda module_id, inputs, output, context: None)
    assert returned is client


async def test_use_after_invoked_after_execute() -> None:
    """apcore_client.use_after.side_effect.1.after_execute

    The registered after-callback runs after module execute (ordered log).
    """
    client = APCore()
    order: list[str] = []

    @client.module(id="ord.after", description="ordered")
    def mod() -> dict:
        order.append("execute")
        return {}

    # After-callbacks receive (module_id, inputs, output, context); returning
    # None leaves the output unchanged.
    client.use_after(lambda module_id, inputs, output, context: order.append("after"))
    client.call("ord.after", {})
    assert order == ["execute", "after"]


def test_use_after_property_idempotent_false() -> None:
    """apcore_client.use_after.property.idempotent_false

    Registering the same callback twice inserts two wrappers; it fires twice
    per execution.
    """
    client = APCore()
    fired: list[int] = []

    @client.module(id="ord.afterdup", description="ordered")
    def mod() -> dict:
        return {}

    cb = lambda module_id, inputs, output, context: fired.append(1)  # noqa: E731
    client.use_after(cb)
    client.use_after(cb)
    client.call("ord.afterdup", {})
    assert len(fired) == 2


# ===========================================================================
# Contract: APCore.remove
# ===========================================================================


def test_remove_returns_true_when_present() -> None:
    """apcore_client.remove.returns.true_when_present

    remove() returns True when the middleware is found by identity and removed.
    """
    client = APCore()
    mw = _NoopMiddleware(name="removable")
    client.use(mw)
    assert client.remove(mw) is True


def test_remove_returns_false_when_absent() -> None:
    """apcore_client.remove.returns.false_when_absent

    remove() returns False for a middleware never added.
    """
    client = APCore()
    assert client.remove(_NoopMiddleware(name="never-added")) is False


def test_remove_property_idempotent_true() -> None:
    """apcore_client.remove.property.idempotent_true

    Removing again after a successful removal returns False without error.
    """
    client = APCore()
    mw = _NoopMiddleware(name="idem")
    client.use(mw)
    assert client.remove(mw) is True
    assert client.remove(mw) is False
