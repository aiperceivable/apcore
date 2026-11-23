"""Drive `overrides_store.json` — pluggable override persistence (#45.1, D-40/D-47).

``tests/sys_modules/test_overrides_store.py`` covers this area by hand and does
not read the fixture, so a case added upstream would not reach Python. This
driver reads the canonical file.

API-shape note (reported, not worked around silently)
-----------------------------------------------------
The fixture's operations assume a **per-key** store surface — ``save(key,
value)``, ``get(key)``, ``get_all()``, ``delete(key)`` — matching the SHOULD-level
sentence in system-modules.md ("methods: `set(key, value)`, `get(key)`,
`get_all()`, `delete(key)`"). The surface all three SDKs actually shipped under
D-47 is whole-map: ``load()`` and ``save(overrides)``. apcore-python,
apcore-typescript, and apcore-rust agree with each other and disagree with the
fixture, so this is a fixture/spec-text problem, not a Python gap.

Consequently:

* :func:`test_case` replays each fixture operation through the shipped
  ``load()`` / ``save()`` surface using a thin read-modify-write adapter, so
  every behavioural claim (durability across reopen, instance isolation,
  missing-path tolerance, idempotent delete) is really asserted.
The fixture used to describe a per-key surface — ``save(key, value)`` /
``get(key)`` / ``get_all()`` / ``delete(key)`` — that no SDK implements, and a
strict xfail here recorded the gap. The spec repo resolved it in favour of the
shipped **D-47** whole-map surface (``load()`` / ``save(mapping)``): three
independent implementations agreeing against one SHOULD sentence is the
sentence being wrong. The xfail is gone because the premise is gone.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

import pytest
import yaml

from apcore.config import Config
from apcore.executor import Executor
from apcore.registry.registry import Registry
from apcore.sys_modules.overrides import (
    FileOverridesStore,
    InMemoryOverridesStore,
    OverridesStore,
)
from apcore.sys_modules.registration import register_sys_modules

from .canonical_fixtures import load_fixture

FIXTURE = load_fixture("overrides_store.json")
CASES: dict[str, dict[str, Any]] = {tc["id"]: tc for tc in FIXTURE["test_cases"]}

# Case ids driven by the operation replayer below. `startup_loads_overrides_
# after_base_config` has no `operations` list — it is driven separately.
_OPERATION_CASES = [
    "save_persists_override",
    "inmemory_store_for_tests",
    "missing_path_first_run_ok",
    "delete_removes_override",
]


# ---------------------------------------------------------------------------
# Per-key adapter over the shipped whole-map surface
# ---------------------------------------------------------------------------


def _put(store: OverridesStore, key: str, value: Any) -> None:
    current = store.load()
    current[key] = value
    store.save(current)


def _drop(store: OverridesStore, key: str) -> None:
    current = store.load()
    current.pop(key, None)
    store.save(current)


class _Replay:
    """Captured results of replaying one fixture case's operations."""

    def __init__(self) -> None:
        # `loads` is the whole mapping each load() returned; `gets` projects it
        # onto the most recently written key so the fixture's single-value
        # expectations stay readable.
        self.loads: list[dict[str, Any]] = []
        self.gets: list[Any] = []
        self.error: Exception | None = None
        self.construction_error: Exception | None = None
        self.last_key: str | None = None


def _replay(case: dict[str, Any], tmp_path: Path) -> tuple[_Replay, Path]:
    """Replay a fixture case against the store type it names."""
    store_type = case["input"]["store_type"]
    path = tmp_path / "overrides.yaml"
    run = _Replay()

    def _new_store() -> OverridesStore:
        if store_type == "FileOverridesStore":
            return FileOverridesStore(path)
        if store_type == "InMemoryOverridesStore":
            return InMemoryOverridesStore()
        pytest.fail(f"[{case['id']}] unknown store_type {store_type!r}")

    if case["input"].get("path_exists_at_construction") is False:
        assert not path.exists(), "fixture requires the overrides path to be absent"

    try:
        store = _new_store()
    except Exception as exc:  # noqa: BLE001 - the fixture asserts construction does not raise
        run.construction_error = exc
        return run, path

    for op in case["input"]["operations"]:
        kind = op["op"]
        try:
            if kind == "construct":
                store = _new_store()  # already constructed above; re-run for fidelity
            elif kind == "load_modify_save":
                # D-47: a single-key change is a read-modify-write over the whole
                # map. That is the only surface the SDKs expose, and it is what
                # the system.control.* code paths do.
                for key, value in op.get("set", {}).items():
                    _put(store, key, value)
                    run.last_key = key
                for key in op.get("remove", []):
                    _drop(store, key)
                    run.last_key = key
            elif kind == "load":
                current = store.load()
                run.loads.append(current)
                run.gets.append(current.get(run.last_key) if run.last_key else None)
            elif kind in ("reopen_store", "new_store_instance"):
                store = _new_store()
            else:
                pytest.fail(f"[{case['id']}] unhandled overrides_store fixture op {kind!r}")
        except Exception as exc:  # noqa: BLE001 - the fixture decides whether an op may raise
            run.error = exc
            break
    return run, path


@pytest.mark.parametrize("case_id", _OPERATION_CASES)
def test_case(case_id: str, tmp_path: Path) -> None:
    case = CASES[case_id]
    expected: dict[str, Any] = case["expected"]
    run, path = _replay(case, tmp_path)

    # Bind the fixture's expectation to the OBSERVED construction outcome. This
    # used to read `assert expected["construction_raised_error"] is False`,
    # which restates the fixture and cannot fail on SDK behaviour
    # (apcore-python#32 / aiperceivable/apcore#81).
    construction_raised = run.construction_error is not None
    if "construction_raised_error" in expected:
        assert construction_raised is expected["construction_raised_error"], (
            f"[{case_id}] construction_raised_error mismatch: got {construction_raised} "
            f"({run.construction_error!r})"
        )
    else:
        assert not construction_raised, f"[{case_id}] constructing the store raised: {run.construction_error!r}"

    if "raised_error" in expected:
        raised = run.error is not None
        assert raised is expected["raised_error"], f"[{case_id}] raised_error mismatch: got {raised} ({run.error!r})"
    else:
        assert run.error is None, f"[{case_id}] operation raised unexpectedly: {run.error!r}"

    if "value_after_reopen" in expected:
        assert run.gets, f"[{case_id}] fixture expects value_after_reopen but no load() ran"
        assert run.gets[-1] == expected["value_after_reopen"], (
            f"[{case_id}] a reopened store did not observe the saved override: "
            f"got {run.gets[-1]!r}, expected {expected['value_after_reopen']!r}"
        )

    if "first_load_value" in expected:
        assert len(run.gets) >= 1, f"[{case_id}] fixture expects first_get_value but no load() ran"
        assert (
            run.gets[0] == expected["first_load_value"]
        ), f"[{case_id}] first load() returned {run.gets[0]!r}, expected {expected['first_load_value']!r}"

    if "second_instance_load_value" in expected:
        assert len(run.gets) >= 2, f"[{case_id}] fixture expects a second load() this driver did not run"
        assert run.gets[1] == expected["second_instance_load_value"], (
            f"[{case_id}] two InMemoryOverridesStore instances shared state: second instance "
            f"returned {run.gets[1]!r}, expected {expected['second_instance_load_value']!r}"
        )

    if "disk_writes" in expected:
        # Observe the disk artefact and compare it to the fixture's count; the
        # store's only possible write is the overrides file itself.
        observed_writes = 1 if path.exists() else 0
        assert observed_writes == expected["disk_writes"], (
            f"[{case_id}] expected {expected['disk_writes']} disk write(s), observed "
            f"{observed_writes}: InMemoryOverridesStore must perform no disk I/O, but {path} exists"
        )

    if "get_all_before_save" in expected:
        assert run.loads, f"[{case_id}] fixture expects get_all_before_save but no load() ran"
        assert (
            run.loads[0] == expected["get_all_before_save"]
        ), f"[{case_id}] a store over a missing path must read as empty, got {run.loads[0]!r}"

    if "path_exists_after_save" in expected:
        assert (
            path.exists() is expected["path_exists_after_save"]
        ), f"[{case_id}] path_exists_after_save mismatch for {path}"

    if "get_all_keys" in expected:
        assert run.loads, f"[{case_id}] fixture expects get_all_keys but no load() ran"
        got_keys = sorted(run.loads[-1])
        assert got_keys == sorted(expected["get_all_keys"]), (
            f"[{case_id}] load() keys {got_keys} != expected {sorted(expected['get_all_keys'])} — "
            f"delete() did not persist the removal"
        )


def test_startup_loads_overrides_after_base_config(tmp_path: Path) -> None:
    """Overrides load after the base config and win for the same key."""
    case = CASES["startup_loads_overrides_after_base_config"]
    base: dict[str, Any] = case["input"]["base_config"]
    overrides: dict[str, Any] = case["input"]["overrides_file"]
    expected: dict[str, Any] = case["expected"]

    base_path = tmp_path / "apcore.yaml"
    # `version` / `project.name` are required by apcore.yaml validation and are
    # not part of what this fixture asserts; they are scaffolding only.
    document = _nest({"version": "1.0", "project.name": "conformance", **base})
    base_path.write_text(yaml.safe_dump(document, sort_keys=True), encoding="utf-8")
    base_bytes_before = base_path.read_bytes()

    overrides_path = tmp_path / "overrides.yaml"
    overrides_path.write_text(yaml.safe_dump(overrides, sort_keys=True), encoding="utf-8")

    config = Config.load(str(base_path))
    for key, value in base.items():
        assert config.get(key) == value, (
            f"[{case['id']}] precondition: base config key {key} did not load "
            f"(got {config.get(key)!r}, expected {value!r})"
        )

    registry = Registry()
    register_sys_modules(
        registry=registry,
        executor=Executor(registry=registry),
        config=config,
        overrides_store=FileOverridesStore(overrides_path),
    )

    for key, value in expected["effective_config"].items():
        assert config.get(key) == value, (
            f"[{case['id']}] effective config for {key}: got {config.get(key)!r}, expected {value!r} — "
            f"overrides must be applied on top of the base config at startup"
        )

    base_file_modified = base_path.read_bytes() != base_bytes_before
    assert (
        base_file_modified is expected["base_file_modified"]
    ), f"[{case['id']}] the base config file MUST NOT be rewritten when overrides are applied"


def _nest(flat: dict[str, Any]) -> dict[str, Any]:
    """Expand dot-path keys into the nested mapping `apcore.yaml` uses."""
    root: dict[str, Any] = {}
    for dotted, value in flat.items():
        node = root
        parts = dotted.split(".")
        for part in parts[:-1]:
            node = node.setdefault(part, {})
        node[parts[-1]] = value
    return root


def test_every_fixture_case_has_a_driver() -> None:
    driven = set(_OPERATION_CASES) | {"startup_loads_overrides_after_base_config"}
    assert set(CASES) == driven, (
        f"overrides_store.json cases without a driver: {sorted(set(CASES) - driven)}; "
        f"drivers with no matching case: {sorted(driven - set(CASES))}"
    )
