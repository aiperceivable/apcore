"""Drive `allow_unknown_namespaces.json` — §9.6.3 `_config.allow_unknown` (#118 D-69).

Both halves of the `strict: false` row were inert. `allow_unknown: false` is
documented as "silently ignored (not stored)" and the namespace was stored
anyway; `allow_unknown: true` is documented as "stored, accessible, **WARN
logged**" and nothing logged. Fixing one without the other leaves the row half
true, so the fixture drives both — and the legacy-mode boundary besides, so the
namespace-only scoping is a decision rather than an omission.
"""

from __future__ import annotations

import logging
import tempfile
from pathlib import Path
from typing import Any

import pytest
import yaml

from apcore.config import Config
from apcore.errors import ConfigError

from .canonical_fixtures import case_ids, load_fixture

FIXTURE = load_fixture("allow_unknown_namespaces.json")
CASES: dict[str, dict[str, Any]] = {tc["id"]: tc for tc in FIXTURE["test_cases"]}

_BASE = {"version": "1.0", "project": {"name": "allow-unknown-probe"}}


def _document(spec: dict[str, Any]) -> dict[str, Any]:
    doc: dict[str, Any] = {"apcore": dict(_BASE)} if spec["mode"] == "namespace" else dict(_BASE)
    if spec.get("config") is not None:
        doc["_config"] = dict(spec["config"])
    # A D-117 case declares a REGISTERED namespace rather than a document one:
    # the whole point is that the key is absent from the file and can only come
    # from the registration.
    if spec.get("namespace") is not None:
        doc[spec["namespace"]] = {"x": 1}
    return doc


def _drive_registered_namespace_default(case_id: str, spec: dict[str, Any], expected: dict[str, Any]) -> None:
    """D-117: a registered namespace's defaults answer only in namespace mode.

    A legacy document has no namespaces, so a declaration ABOUT a namespace has
    nothing to say about one. The key is absent from the file by construction —
    if it were present, the document would be answering, not the registration.
    """
    registration = spec["registered_namespace"]
    # Namespace registration is process-wide and permanent (§9.6.3 point 5), so
    # each case registers under its own name rather than racing the other.
    name = f"{registration['name']}_{case_id[:12]}"
    try:
        Config.register_namespace(name, defaults=dict(registration["defaults"]))
    except Exception:  # noqa: BLE001 - already registered by a previous run
        pass

    doc: dict[str, Any] = {"apcore": dict(_BASE)} if spec["mode"] == "namespace" else dict(_BASE)
    path = Path(tempfile.mkdtemp()) / "apcore.yaml"
    path.write_text(yaml.safe_dump(doc), encoding="utf-8")

    config = Config.load(str(path))
    key = spec["key"].replace(registration["name"], name, 1)
    value = config.get(key)

    assert (value is not None) is expected["value_readable"], (
        f"[{case_id}] {key!r} -> {value!r}; the registration must answer in "
        f"namespace mode and stay silent for a legacy document"
    )
    if "value" in expected:
        assert value == expected["value"]


@pytest.mark.parametrize("case_id", list(CASES))
def test_allow_unknown_namespaces(case_id: str, caplog: pytest.LogCaptureFixture) -> None:
    case = CASES[case_id]
    spec, expected = case["input"], case["expected"]

    if "registered_namespace" in spec:
        _drive_registered_namespace_default(case_id, spec, expected)
        return

    path = Path(tempfile.mkdtemp()) / "apcore.yaml"
    path.write_text(yaml.safe_dump(_document(spec)), encoding="utf-8")

    caplog.clear()
    with caplog.at_level(logging.DEBUG):
        if expected.get("loads") is False:
            with pytest.raises(ConfigError) as excinfo:
                Config.load(str(path))
            assert excinfo.value.code == expected["error_code"]
            assert expected["error_message_contains"] in str(excinfo.value)
            return
        config = Config.load(str(path))

    if "value_readable" in expected:
        value = config.get(f"{spec['namespace']}.x")
        assert (value is not None) is expected["value_readable"], value

    messages = [r.getMessage() for r in caplog.records]
    if "warns_naming" in expected:
        hits = [m for m in messages if expected["warns_naming"] in m and "registered" in m]
        assert len(hits) == 1, messages
    if "warns_absent" in expected:
        assert not any(expected["warns_absent"] in m for m in messages), messages


def test_every_fixture_case_is_driven() -> None:
    assert set(case_ids("allow_unknown_namespaces.json")) == set(CASES)
