"""Drive `storage_backend.json` — the pluggable StorageBackend primitive (#43, D-39).

The fixture asserts the four-method contract (``save`` / ``get`` / ``list`` /
``delete``), namespace isolation, prefix filtering, overwrite, and idempotent
delete against ``InMemoryStorageBackend``.

Value-type disagreement (deliberately surfaced, not papered over)
----------------------------------------------------------------
Every fixture operation stores a **scalar** value (``"ENT-1"``, ``"1"``,
``"x"``).  apcore-python's ``StorageBackend`` protocol declares
``value: dict`` and ``InMemoryStorageBackend.save`` copies through
``dict(value)`` (``src/apcore/observability/storage.py:66``), so a scalar raises
``ValueError``.  apcore-typescript declares ``Record<string, unknown>`` too;
apcore-rust accepts any ``serde_json::Value``.

Rather than hide that, this driver does both:

* :class:`TestStorageBackendFixture` replays each fixture case with the scalar
  wrapped in a one-field record, which exercises the *behavioural* claims the
  fixture is actually about (roundtrip, prefix, isolation, overwrite,
  idempotent delete) against the value type this SDK supports.
* :class:`TestStorageBackendScalarValues` replays the same cases verbatim under
  a strict ``xfail``.  It documents the gap and turns red the day the SDKs and
  the fixture converge, so the disagreement cannot rot silently.

The fixture is canonical and is NOT edited here.  See the task report: the
open question for the spec is whether ``value`` is object-only (Python/TS) or
any JSON value (Rust + this fixture).
"""

from __future__ import annotations

from typing import Any

import pytest

from apcore.observability import InMemoryStorageBackend, StorageBackend

from .canonical_fixtures import load_fixture

FIXTURE = load_fixture("storage_backend.json")
CASES: list[dict[str, Any]] = FIXTURE["test_cases"]

# Field used to lift a scalar fixture value into the record shape this SDK's
# StorageBackend protocol declares.  Never applied to `expected` — comparisons
# below unwrap before asserting, so the fixture's expected values stand as-is.
_WRAP_FIELD = "value"

# Every `expected` key this driver knows how to check.  A fixture case carrying
# an unknown key fails instead of being quietly ignored.
_KNOWN_EXPECTATIONS = {
    "final_get_value",
    "matched_keys_sorted",
    "raised_error",
    "errors_namespace_value",
    "metrics_namespace_value",
}


class _Run:
    """Outcome of replaying one fixture case's operation list."""

    def __init__(self) -> None:
        self.gets: list[tuple[str, str, Any]] = []  # (namespace, key, value)
        self.lists: list[list[tuple[str, Any]]] = []
        self.error: Exception | None = None


def _replay(backend: StorageBackend, operations: list[dict[str, Any]], *, wrap: bool) -> _Run:
    """Replay fixture operations against *backend*, capturing get/list results."""
    run = _Run()
    for op in operations:
        kind = op["op"]
        try:
            if kind == "save":
                value = {_WRAP_FIELD: op["value"]} if wrap else op["value"]
                backend.save(op["namespace"], op["key"], value)
            elif kind == "get":
                stored = backend.get(op["namespace"], op["key"])
                run.gets.append((op["namespace"], op["key"], _unwrap(stored, wrap=wrap)))
            elif kind == "list":
                run.lists.append(list(backend.list(op["namespace"], op.get("prefix", ""))))
            elif kind == "delete":
                backend.delete(op["namespace"], op["key"])
            else:
                pytest.fail(f"unhandled storage_backend fixture op {kind!r}")
        except Exception as exc:  # noqa: BLE001 - the fixture asserts whether an op raises
            run.error = exc
            break
    return run


def _unwrap(stored: Any, *, wrap: bool) -> Any:
    """Undo :data:`_WRAP_FIELD` so results compare against the fixture verbatim."""
    if not wrap or stored is None:
        return stored
    assert isinstance(stored, dict) and _WRAP_FIELD in stored, f"unexpected stored shape: {stored!r}"
    return stored[_WRAP_FIELD]


def _assert_case(case: dict[str, Any], run: _Run) -> None:
    """Check every `expected` key the fixture case declares."""
    cid = case["id"]
    expected: dict[str, Any] = case["expected"]

    unknown = set(expected) - _KNOWN_EXPECTATIONS
    assert not unknown, (
        f"[{cid}] storage_backend.json declares expectations this driver cannot check: "
        f"{sorted(unknown)} — extend _KNOWN_EXPECTATIONS rather than ignoring them"
    )

    if "raised_error" in expected:
        raised = run.error is not None
        assert raised is expected["raised_error"], (
            f"[{cid}] raised_error mismatch: got {raised} ({run.error!r}), " f"expected {expected['raised_error']}"
        )
    elif run.error is not None:
        raise AssertionError(f"[{cid}] operation raised unexpectedly: {run.error!r}")

    if "final_get_value" in expected:
        assert run.gets, f"[{cid}] fixture expects final_get_value but no get() op ran"
        assert run.gets[-1][2] == expected["final_get_value"], (
            f"[{cid}] final get() on {run.gets[-1][0]}/{run.gets[-1][1]} returned "
            f"{run.gets[-1][2]!r}, expected {expected['final_get_value']!r}"
        )

    if "matched_keys_sorted" in expected:
        assert run.lists, f"[{cid}] fixture expects matched_keys_sorted but no list() op ran"
        got = sorted(key for key, _ in run.lists[-1])
        assert (
            got == expected["matched_keys_sorted"]
        ), f"[{cid}] list() prefix filter returned {got}, expected {expected['matched_keys_sorted']}"

    for expectation, namespace in (
        ("errors_namespace_value", "errors"),
        ("metrics_namespace_value", "metrics"),
    ):
        if expectation not in expected:
            continue
        matches = [value for ns, _, value in run.gets if ns == namespace]
        assert matches, f"[{cid}] fixture expects {expectation} but no get() ran on namespace {namespace!r}"
        assert matches[-1] == expected[expectation], (
            f"[{cid}] namespace isolation broken: get() on {namespace!r} returned {matches[-1]!r}, "
            f"expected {expected[expectation]!r}"
        )


def _drive_collector_case(case: dict[str, Any]) -> None:
    """D-113: which namespace each bundled collector writes, and the default.

    Drives all named collectors through ONE backend so the assertion is the SET
    of namespaces the surface produces, not three independent behaviours — an
    SDK writing two of three would otherwise read as a partial gap rather than
    a wrong surface.
    """
    from apcore.errors import ModuleError
    from apcore.observability.error_history import ErrorHistory
    from apcore.observability.metrics import MetricsCollector
    from apcore.observability.usage import UsageCollector

    if case.get("omit_backend"):
        # The observable is READABILITY: a record written with no backend
        # supplied is still there to read. Asserting a class name would pin this
        # SDK's spelling, which the decision does not.
        history = ErrorHistory()
        history.record("mod.x", ModuleError(code="E1", message="boom"))
        readable = len(history.stored_entries()) > 0
        assert (
            readable is case["expected"]["records_readable"]
        ), f"[{case['id']}] an omitted backend must be the in-memory one, not 'no storage'"
        return

    backend = InMemoryStorageBackend()
    for collector in case["collectors"]:
        if collector == "metrics":
            MetricsCollector(storage=backend).observe_duration("mod.x", 0.01)
        elif collector == "usage":
            UsageCollector(storage=backend).record("mod.x", "caller", 12.0, True)
        elif collector == "error_history":
            ErrorHistory(storage=backend).record("mod.x", ModuleError(code="E1", message="boom"))
        else:  # pragma: no cover - the fixture names only these three
            raise AssertionError(f"[{case['id']}] unknown collector {collector!r}")

    assert sorted(backend._data.keys()) == sorted(
        case["expected"]["namespaces_written"]
    ), f"[{case['id']}] namespaces written: {sorted(backend._data.keys())}"


class TestStorageBackendFixture:
    """storage_backend.json replayed against InMemoryStorageBackend."""

    def test_default_backend_satisfies_protocol(self) -> None:
        assert isinstance(InMemoryStorageBackend(), StorageBackend), (
            "InMemoryStorageBackend must satisfy the StorageBackend protocol "
            "(observability.md § Pluggable storage backends)"
        )

    @pytest.mark.parametrize("case", CASES, ids=lambda c: c["id"])
    def test_case(self, case: dict[str, Any]) -> None:
        if "collectors" in case:
            _drive_collector_case(case)
            return
        backend = InMemoryStorageBackend()
        run = _replay(backend, case["input"]["operations"], wrap=True)
        _assert_case(case, run)


class TestStorageBackendScalarValues:
    """The same cases with the fixture's scalar values passed through unchanged.

    Strict xfail: this SDK narrows StorageBackend values to ``dict`` and copies
    via ``dict(value)``, so a scalar raises.  When that is widened (or the
    fixture moves to record values) these turn into unexpected passes and this
    class must be revisited.
    """

    @pytest.mark.xfail(
        strict=True,
        reason=(
            "apcore-python StorageBackend declares value: dict and InMemoryStorageBackend.save "
            "copies via dict(value) (src/apcore/observability/storage.py:66), so the fixture's "
            "scalar values raise ValueError. apcore-rust accepts any serde_json::Value. "
            "Spec must decide which value width is normative."
        ),
    )
    @pytest.mark.parametrize("case", [c for c in CASES if "collectors" not in c], ids=lambda c: c["id"])
    def test_case_verbatim(self, case: dict[str, Any]) -> None:
        # The D-113 collector cases carry no `input.operations` — they exercise
        # the collectors, not the backend's value width this class is about.
        backend = InMemoryStorageBackend()
        run = _replay(backend, case["input"]["operations"], wrap=False)
        _assert_case(case, run)
