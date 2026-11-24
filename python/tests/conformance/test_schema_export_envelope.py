"""Drive `schema_export_envelope.json` — the `Registry.export_schema` envelope.

Four keys, no more. The three SDKs each emitted a different shape until this was
pinned: Python added an always-empty ``definitions`` (a ``SchemaDefinition``
field a descriptor does not have — ``$defs`` were already inside
``input_schema`` where JSON Schema puts them), TypeScript added ``name`` /
``version`` / ``tags`` / ``annotations`` / ``examples``, making its export a
partial, non-conforming duplicate of ``system.manifest.module``.
"""

from __future__ import annotations

from typing import Any

import pytest

from apcore import Registry

from .canonical_fixtures import load_fixture

FIXTURE = load_fixture("schema_export_envelope.json")
ENVELOPE_KEYS: list[str] = FIXTURE["envelope_keys"]
CASES: list[dict[str, Any]] = FIXTURE["test_cases"]


def _make_module(spec: dict[str, Any]) -> Any:
    """Build a duck-typed module carrying whatever the fixture declares."""

    class _M:
        input_schema = spec["input_schema"]
        output_schema = spec["output_schema"]
        description = spec["description"]

        def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
            return {}

    mod = _M()
    # Descriptor metadata the fixture may declare — present on the module so the
    # test proves the exporter drops it, rather than proving it was never there.
    for attr in ("version", "tags", "annotations", "examples", "name"):
        if attr in spec:
            setattr(mod, attr, spec[attr])
    return mod


def _export(case: dict[str, Any]) -> Any:
    registry = Registry()
    if "module" in case:
        spec = case["module"]
        registry.register(spec["module_id"], _make_module(spec))
        module_id = spec["module_id"]
    else:
        module_id = case["module_id"]
    return registry.export_schema(module_id, strict=case["strict"])


@pytest.mark.parametrize("case", CASES, ids=[c["id"] for c in CASES])
def test_export_schema_envelope(case: dict[str, Any]) -> None:
    result = _export(case)
    expected = case["expected"]

    if expected is None:
        assert result is None, "an unregistered module must export None"
        return

    # EXACT key set — a subset check would not catch the extra keys this pins.
    assert sorted(result) == sorted(ENVELOPE_KEYS), (
        f"{case['id']}: envelope keys are {sorted(result)}, " f"canonical is {sorted(ENVELOPE_KEYS)}"
    )
    assert result == expected


def test_no_sibling_definitions_key() -> None:
    """`$defs` live inside `input_schema`; a top-level `definitions` was always
    empty on this path and gave callers a second place to look."""
    case = next(c for c in CASES if c["id"] == "defs_stay_inside_input_schema_no_sibling_definitions_key")
    result = _export(case)
    assert "definitions" not in result
    assert "$defs" in result["input_schema"]
