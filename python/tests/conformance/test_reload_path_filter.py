"""Drive `reload_path_filter.json` — granular reload on `system.control.reload_module`.

The fixture pins four behaviours (system-modules.md §14):

* a glob ``path_filter`` reloads exactly the matching module ids,
* ``module_id`` alone still performs the single-module reload,
* a filter matching nothing is a success with an empty reload set (never an error),
* ``module_id`` and ``path_filter`` together raise ``MODULE_RELOAD_CONFLICT``.

Re-discovery is the one thing stubbed: ``ReloadModule._rediscover_module`` walks
the registry's discovery paths, which a fixture cannot supply. The stub is the
seam already used by ``tests/test_suspend_resume.py`` and it records the ids it
was asked to re-discover, so the *selection* logic — the whole point of the
fixture — is asserted on real code, not on a mock's return value.

What the stub necessarily cannot see is what happens *after* re-discovery: real
discovery registers what it finds, and for a while nothing in the suite ran that
sequence unstubbed, which hid apcore-python#33 (a `DUPLICATE_MODULE_ID` on every
filesystem-backed reload). The unstubbed round-trip now lives in
``tests/sys_modules/test_control.py::TestReloadModuleRealDiscoveryPath``; keep it
in mind before widening the stub here.
"""

from __future__ import annotations

from typing import Any
from unittest.mock import patch

import pytest

from apcore.errors import ModuleReloadConflictError
from apcore.events import EventEmitter
from apcore.registry import Registry
from apcore.sys_modules.audit import InMemoryAuditStore
from apcore.sys_modules.control import ReloadModule

from .canonical_fixtures import load_fixture

FIXTURE = load_fixture("reload_path_filter.json")
CASES: list[dict[str, Any]] = FIXTURE["test_cases"]

# `expected` keys this driver checks. `_`-prefixed entries are fixture prose.
_KNOWN_EXPECTATIONS = {
    "success",
    "reloaded_modules_set",
    "error",
    "error_code",
    "audit_target_module_ids_set",
    "audit_correlation_ids_are_equal_and_non_empty",
}


class _DummyModule:
    """Minimal registrable module standing in for a discovered one."""

    version = "1.0.0"
    description = "conformance stand-in module"
    input_schema: dict[str, Any] = {"type": "object", "properties": {}}
    output_schema: dict[str, Any] = {"type": "object", "properties": {}}

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        return {}


def _registry_with(module_ids: list[str]) -> Registry:
    registry = Registry()
    for module_id in module_ids:
        registry.register_internal(module_id, _DummyModule())
    return registry


@pytest.mark.parametrize("case", CASES, ids=lambda c: c["id"])
def test_reload_path_filter_case(case: dict[str, Any]) -> None:
    cid = case["id"]
    expected: dict[str, Any] = case["expected"]
    unknown = {k for k in expected if not k.startswith("_")} - _KNOWN_EXPECTATIONS
    assert not unknown, (
        f"[{cid}] reload_path_filter.json declares expectations this driver cannot check: "
        f"{sorted(unknown)} — extend _KNOWN_EXPECTATIONS rather than ignoring them"
    )

    registry = _registry_with(case["registered_modules"])
    audit_store = InMemoryAuditStore()
    reload_module = ReloadModule(registry=registry, event_emitter=EventEmitter(), audit_store=audit_store)

    rediscovered: list[str] = []

    def _fake_rediscover(module_id: str) -> _DummyModule:
        # Returns the freshly "discovered" instance only; ReloadModule performs
        # the re-registration itself via _reregister_module.
        rediscovered.append(module_id)
        return _DummyModule()

    error: Exception | None = None
    result: dict[str, Any] = {}
    with patch.object(reload_module, "_rediscover_module", side_effect=_fake_rediscover):
        try:
            result = reload_module.execute(dict(case["input"]), context=None)
        except Exception as exc:  # noqa: BLE001 - the fixture decides which errors are expected
            error = exc

    if "error_code" in expected:
        assert (
            error is not None
        ), f"[{cid}] expected error_code {expected['error_code']!r} but execute() returned {result!r}"
        assert isinstance(
            error, ModuleReloadConflictError
        ), f"[{cid}] expected ModuleReloadConflictError, got {type(error).__name__}: {error}"
        assert (
            error.code == expected["error_code"]
        ), f"[{cid}] error code mismatch: got {error.code!r}, expected {expected['error_code']!r}"
        return

    assert error is None, f"[{cid}] execute() raised unexpectedly: {type(error).__name__}: {error}"
    if "error" in expected:
        assert expected["error"] is None, f"[{cid}] fixture declares a non-null error this driver cannot express"

    if "success" in expected:
        assert (
            result.get("success") is expected["success"]
        ), f"[{cid}] success mismatch: got {result.get('success')!r}, expected {expected['success']!r}"

    # Single reload reports the id in `module_id`; bulk reload reports the list
    # in `reloaded_modules`. Both are compared as a set against the fixture.
    if "reloaded_modules" in result:
        reported = sorted(result["reloaded_modules"])
    elif result.get("module_id") is not None:
        reported = [result["module_id"]]
    else:
        reported = []

    if "audit_target_module_ids_set" in expected:
        # D-111. Asserted as a SET of concrete ids rather than a count: an
        # aggregate entry keyed on the glob has count 1, which a count
        # assertion accepts whenever the glob matches one module, and its
        # target is one `AuditStore.query(module_id=...)` can never find.
        entries = audit_store.query()
        assert sorted(e.target_module_id for e in entries) == sorted(
            expected["audit_target_module_ids_set"]
        ), f"[{cid}] audit targets: got {[e.target_module_id for e in entries]!r}"
        for module_id in expected["audit_target_module_ids_set"]:
            found = audit_store.query(module_id=module_id)
            assert [e.target_module_id for e in found] == [
                module_id
            ], f"[{cid}] {module_id!r} is not findable by the accessor the store exists for"

    if expected.get("audit_correlation_ids_are_equal_and_non_empty"):
        # A property, not a value: the id is generated per call. All equal
        # rejects a fresh id per entry, which groups nothing; non-empty rejects
        # leaving the field unset.
        ids = {e.correlation_id for e in audit_store.query()}
        assert len(ids) == 1, f"[{cid}] entries from one bulk reload must share one id: {ids!r}"
        assert ids != {""}, f"[{cid}] the correlation id must be set"

    assert reported == sorted(expected["reloaded_modules_set"]), (
        f"[{cid}] reload set mismatch: got {reported}, expected {sorted(expected['reloaded_modules_set'])} "
        f"(path_filter={case['input'].get('path_filter')!r}, module_id={case['input'].get('module_id')!r})"
    )
    assert sorted(rediscovered) == sorted(expected["reloaded_modules_set"]), (
        f"[{cid}] re-discovery touched {sorted(rediscovered)}, expected {sorted(expected['reloaded_modules_set'])} — "
        f"the reported set and the modules actually reloaded must agree"
    )
    # Non-matching modules must be left registered and untouched.
    survivors = set(case["registered_modules"]) - set(expected["reloaded_modules_set"])
    still_registered = {mid for mid in survivors if registry.get(mid) is not None}
    assert (
        still_registered == survivors
    ), f"[{cid}] reload dropped non-matching modules: {sorted(survivors - still_registered)}"
