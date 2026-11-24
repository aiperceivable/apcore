"""Conformance tests for registry on_load ordering invariants (apcore #65) fixture."""

from __future__ import annotations

import threading
import time
from concurrent.futures import ThreadPoolExecutor
from typing import Any

import pytest

from apcore.errors import InvalidInputError, ModuleError
from apcore.events.emitter import ApCoreEvent, EventEmitter
from apcore.registry import Registry
from conformance.canonical_fixtures import load_fixture


def _load_fixture() -> dict:
    """Load the canonical fixture from the apcore spec repo."""
    return load_fixture("registry_load_ordering.json")


@pytest.fixture
def fixture_data() -> dict:
    return _load_fixture()


def test_fixture_visibility_after_successful_on_load(fixture_data: dict) -> None:
    """Case: visibility_after_successful_on_load."""
    case = next(c for c in fixture_data["test_cases"] if c["id"] == "visibility_after_successful_on_load")
    setup = case["setup"]["module"]
    expected = case["expected"]

    delay_ms = setup.get("on_load_delay_ms", 0)
    mod_id = setup["id"]
    reg = Registry()

    # Capture visibility at 25ms (mid on_load)
    concurrent_list_result: list[bool] = []
    concurrent_get_result: list[Any] = []

    class _SpyModule:
        data: dict[str, Any] = {}

        def on_load(self) -> None:
            time.sleep(delay_ms / 2 / 1000)
            concurrent_list_result.append(mod_id in reg.list())
            concurrent_get_result.append(reg.get(mod_id))
            time.sleep(delay_ms / 2 / 1000)
            _SpyModule.data["_test.warmed"] = True

    _SpyModule.data.clear()
    module = _SpyModule()
    reg.register(mod_id, module)

    # After register() returns, module IS visible
    assert expected["registration_succeeds"] is True
    if expected["post_register_visible"]:
        assert mod_id in reg.list()
        assert reg.get(mod_id) is not None

    # During on_load, module was NOT visible
    if not expected["concurrent_check_visible"]:
        assert concurrent_list_result == [False]
        # Bound to the fixture rather than to a local literal. The key used to
        # read `concurrent_check_get_raises: "MODULE_NOT_FOUND"`, a behaviour no
        # SDK implements — Registry.get() returns the empty value for an id that
        # is not visible, per features/registry-system.md "On success (not
        # found)". The fixture now says `concurrent_check_get_returns: null`,
        # which is the observable thing, so the assertion can be real.
        assert concurrent_get_result == [expected["concurrent_check_get_returns"]]

    if expected.get("on_load_observed_data"):
        for key, val in expected["on_load_observed_data"].items():
            assert _SpyModule.data.get(key) == val


def test_fixture_callback_failure_blocks_visibility(fixture_data: dict) -> None:
    """Case: callback_failure_blocks_visibility."""
    case = next(c for c in fixture_data["test_cases"] if c["id"] == "callback_failure_blocks_visibility")
    setup = case["setup"]["module"]
    expected = case["expected"]

    mod_id = setup["id"]
    error_cfg = setup["on_load_raises"]

    emitter = EventEmitter()
    reg = Registry()
    reg.set_event_emitter(emitter)

    load_failed_events: list[ApCoreEvent] = []

    class _DLQRecorder:
        async def on_event(self, event: ApCoreEvent) -> None:
            if event.event_type == "apcore.registry.module_load_failed":
                load_failed_events.append(event)

    emitter.subscribe(_DLQRecorder())

    class _FailingModule:
        def on_load(self) -> None:
            raise ConnectionError(error_cfg["message"])

    raised: BaseException | None = None
    try:
        reg.register(mod_id, _FailingModule())
    except BaseException as exc:  # noqa: BLE001 - the type is the thing under test
        raised = exc

    emitter.flush(timeout=5.0)
    emitter.shutdown()

    # `registration_raises` names the HOST exception the callback threw, not an
    # apcore wire code (see the fixture's driver_contract): register() must let
    # the callback's own exception out unchanged rather than wrapping it in a
    # ModuleError, which would cost the caller the original type and traceback.
    unwrapped = type(raised).__name__ == error_cfg["type"] and not isinstance(raised, ModuleError)
    assert unwrapped, f"expected {expected['registration_raises']}, got {raised!r}"
    assert str(raised) == error_cfg["message"]

    assert (mod_id in reg.list()) is expected["post_register_list_contains"]
    # Same correction as above: the key declared a raise that no SDK performs
    # and is now `post_register_get_returns: null`.
    assert reg.get(mod_id) == expected["post_register_get_returns"]

    if expected.get("load_failed_event_emitted"):
        assert len(load_failed_events) == 1
        evt = load_failed_events[0]
        expected_evt = expected["load_failed_event"]
        assert evt.event_type == expected_evt["event_type"]
        data_contains = expected_evt["data_contains"]
        assert evt.data["module_id"] == data_contains["module_id"]
        assert evt.data["error_type"] == data_contains["error_type"]
        assert data_contains["error_message"] in evt.data["error_message"]
        for key in expected_evt["data_required_keys"]:
            assert key in evt.data, f"load_failed event missing: {key}"


def test_fixture_concurrent_same_id_rejects_duplicate(fixture_data: dict) -> None:
    """Case: concurrent_same_id_rejects_duplicate."""
    case = next(c for c in fixture_data["test_cases"] if c["id"] == "concurrent_same_id_rejects_duplicate")
    setup = case["setup"]
    expected = case["expected"]

    reg = Registry()
    results: dict[str, Any] = {"success": 0, "errors": []}
    lock = threading.Lock()
    delay_ms = setup["module_a"].get("on_load_delay_ms", 0)

    class _SlowModule:
        def on_load(self) -> None:
            time.sleep(delay_ms / 1000)

    def do_register() -> None:
        try:
            mod_id = setup["module_a"]["id"]
            reg.register(mod_id, _SlowModule())
            with lock:
                results["success"] += 1
        except InvalidInputError as e:
            with lock:
                results["errors"].append(e)

    t1 = threading.Thread(target=do_register)
    t2 = threading.Thread(target=do_register)
    t1.start()
    t2.start()
    t1.join(timeout=5.0)
    t2.join(timeout=5.0)

    assert (results["success"] == 1) is expected["one_succeeds"]
    assert len(results["errors"]) == 1
    rejected = results["errors"][0]
    assert rejected.code == expected["raised_error_code"]
    # `one_raises` names the error FAMILY by its canonical wire code: the
    # rejection must come out as the SDK's invalid-input error (whose default
    # code is GENERAL_INVALID_INPUT) carrying the specific DUPLICATE_MODULE_ID
    # code above — not, say, a bare RuntimeError or a ModuleIdConflictError.
    assert isinstance(rejected, InvalidInputError)
    # The fixture's `one_raises: GENERAL_INVALID_INPUT` is gone. It contradicted
    # `raised_error_code: DUPLICATE_MODULE_ID` in the same case, and the only way
    # to satisfy both was to construct an InvalidInputError this case never
    # raises and read its DEFAULT code — true, but about a different object than
    # the one under test. apcore-typescript and apcore-rust have no
    # invalid-input family at all, so the key was a Python implementation detail
    # promoted to a cross-language expectation.

    mod_id = setup["module_a"]["id"]
    if expected["post_register_visible"]:
        assert reg.get(mod_id) is not None
    assert len(reg.list()) == expected["post_register_count"]


@pytest.mark.parametrize("startup_delay", [0.0, 0.15], ids=["normal_startup", "slow_startup"])
def test_fixture_concurrent_distinct_ids_run_in_parallel(fixture_data: dict[str, Any], startup_delay: float) -> None:
    """Both callbacks overlap even when worker startup is slow."""
    case = next(c for c in fixture_data["test_cases"] if c["id"] == "concurrent_distinct_ids_run_in_parallel")
    setup = case["setup"]
    expected = case["expected"]
    reg = Registry()
    gate = threading.Barrier(2, timeout=5.0)
    callbacks_overlap = threading.Event()

    class _OverlapModule:
        def __init__(self, delay_ms: int) -> None:
            self.delay_ms = delay_ms

        def on_load(self) -> None:
            gate.wait()
            callbacks_overlap.set()
            time.sleep(self.delay_ms / 1000)

    def register(key: str) -> None:
        time.sleep(startup_delay)
        module = setup[key]
        reg.register(module["id"], _OverlapModule(module["on_load_delay_ms"]))

    with ThreadPoolExecutor(max_workers=2) as pool:
        futures = [pool.submit(register, key) for key in ("module_x", "module_y")]
        for future in futures:
            future.result(timeout=10.0)

    assert all(future.done() and future.exception() is None for future in futures) == expected["both_succeed"]
    assert callbacks_overlap.is_set() == expected["callbacks_overlap"]
    assert reg.get(setup["module_x"]["id"]) is not None
    assert reg.get(setup["module_y"]["id"]) is not None
    assert len(reg.list()) == expected["post_register_count"]
