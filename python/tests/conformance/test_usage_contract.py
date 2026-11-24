"""Conformance driver for ``usage_contract.json`` (PROTOCOL_SPEC 6.7.1).

The two value semantics no JSON Schema can assert. A full-history ``call_count``
and an off-by-one ``p99_latency_ms`` are both well-typed numbers in the right
field, so they need fixed inputs and expected outputs.

Drives the real ``system.usage.*`` modules against a real ``UsageCollector``,
not the collector's accessors directly: every divergence this fixture pins lived
in the sys-module layer's choice of accessor.
"""

from __future__ import annotations

import json
import re
import warnings
from datetime import datetime, timedelta, timezone
from typing import Any

from jsonschema import Draft202012Validator

import pytest

from apcore import APCore, ModuleAnnotations, Registry
from apcore.config import Config
from apcore.errors import ModuleError
from apcore.observability.usage import UsageCollector, UsageMiddleware
from apcore.sys_modules.usage import UsageModuleModule, UsageSummaryModule
from conformance.canonical_fixtures import case_ids, load_fixture, schemas_dir

FIXTURE = load_fixture("usage_contract.json")
CASES = FIXTURE["test_cases"]

# driver_contract.output_validates_against_the_canonical_schema: the same files
# the spec repo ships, not a copy. `additionalProperties: false` is the point —
# a field one SDK emits and the others do not fails here.
# §8.2.1 rule 4: resolve `schemas/` on its own rather than climbing out of
# the fixtures root — under CONFORMANCE_FIXTURES there is no repo above it.
_SCHEMAS = schemas_dir()
_SCHEMA_FOR = {
    "system.usage.summary": "sys-usage-summary.schema.json",
    "system.usage.module": "sys-usage-module.schema.json",
}


def _validate_against_canonical_schema(module: str, output: dict[str, Any]) -> None:
    schema = json.loads((_SCHEMAS / _SCHEMA_FOR[module]).read_text())
    errors = sorted(Draft202012Validator(schema).iter_errors(output), key=lambda e: list(e.path))
    assert not errors, "\n".join(f"{list(e.path)}: {e.message}" for e in errors)


_OFFSET = re.compile(r"^-(\d+)([hd])$")


def _at(offset: str) -> str:
    match = _OFFSET.match(offset)
    assert match, f"unsupported at_offset {offset!r}"
    amount, unit = int(match.group(1)), match.group(2)
    delta = timedelta(hours=amount) if unit == "h" else timedelta(days=amount)
    return (datetime.now(timezone.utc) - delta).isoformat()


def _registry_with(module_id: str) -> Registry:
    registry = Registry()

    def handler(inputs: dict[str, Any], ctx: Any) -> dict[str, Any]:  # pragma: no cover
        return {}

    handler.annotations = ModuleAnnotations()
    handler.description = "conformance target"
    handler.input_schema = {"type": "object"}
    handler.output_schema = {"type": "object"}
    registry.register(module_id, handler)
    return registry


def _collector(case: dict[str, Any], module_id: str) -> UsageCollector:
    collector = UsageCollector()
    for latency in case.get("latencies_ms", []):
        collector.record(module_id, "caller-a", float(latency), True)
    for record in case.get("records", []):
        if record["caller_id"] is None:
            # driver_contract.unattributed_caller: a call with NO caller
            # identity must go through this SDK's usage-recording path, not
            # straight into the collector -- apcore-python substitutes
            # "unknown" in UsageMiddleware, apcore-rust in the breakdown.
            _record_through_middleware(collector, module_id, bool(record["success"]))
            continue
        collector.record(
            module_id,
            record["caller_id"],
            float(record["latency_ms"]),
            bool(record["success"]),
            timestamp=_at(record["at_offset"]),
        )
    return collector


class _NoCallerContext:
    """A Context carrying no caller identity."""

    caller_id = None
    data: dict[str, Any] = {}


def _record_through_middleware(collector: UsageCollector, module_id: str, success: bool) -> None:
    middleware = UsageMiddleware(collector)
    context = _NoCallerContext()
    middleware.before(module_id, {}, context)
    if success:
        middleware.after(module_id, {}, {}, context)
    else:
        middleware.on_error(module_id, {}, RuntimeError("conformance"), context)


def _run(case: dict[str, Any]) -> dict[str, Any]:
    module_id = case.get("module_id", "math.add")
    registry = _registry_with(module_id)
    collector = _collector(case, module_id)
    inputs = dict(case.get("inputs") or {})

    if case["module"] == "system.usage.summary":
        module = UsageSummaryModule(collector)
        return module.execute(inputs, None)

    module = UsageModuleModule(registry=registry, usage_collector=collector)
    inputs.setdefault("module_id", module_id)
    return module.execute(inputs, None)


@pytest.mark.parametrize("case", CASES, ids=case_ids("usage_contract.json"))
def test_usage_contract(case: dict[str, Any]) -> None:
    expected = case["expected"]

    # Rejection cases assert the WIRE CODE. The grammar is declared as a
    # `pattern` in input_schema so the rejection happens at input validation
    # (6.7.1.1), not inside a private parser raising a language-native error.
    if "error_code" in expected:
        schema = UsageSummaryModule.input_schema
        pattern = schema["properties"]["period"]["pattern"]
        assert pattern == "^[1-9][0-9]*[hd]$", "the grammar must be declared in input_schema"
        assert not re.match(pattern, case["inputs"]["period"]), (
            f"{case['id']}: fixture expects {case['inputs']['period']!r} to be rejected, "
            f"but the declared pattern accepts it"
        )

        # ...and then assert what the case actually SAYS. The two checks above
        # are about the schema; neither reads `expected["error_code"]`, so this
        # branch used to pass whatever wire code the fixture declared — the case
        # was run and its expectation asserted nothing. Found by
        # `check_case_pinning.py`: mutating the code left every driver green.
        #
        # The rejection happens at input validation (§6.7.1.1), which is the
        # pipeline's job rather than `execute`'s, so the assertion has to go
        # through a real client.
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", DeprecationWarning)
            client = APCore(
                config=Config(
                    {
                        "version": "1.0",
                        "project": {"name": "usage-contract"},
                        "sys_modules": {"enabled": True, "usage": {"enabled": True}},
                    }
                )
            )
        with pytest.raises(ModuleError) as raised:
            client.call(case["module"], dict(case["inputs"]))
        assert raised.value.code == expected["error_code"], (
            f"{case['id']}: {case['inputs']['period']!r} was rejected with "
            f"{raised.value.code!r}, and the fixture declares "
            f"{expected['error_code']!r} as the wire code an operator sees"
        )
        return

    result = _run(case)
    _validate_against_canonical_schema(case["module"], result)

    if "caller_ids" in expected:
        assert [c["caller_id"] for c in result["callers"]] == expected["caller_ids"], (
            f"{case['id']}: caller ids are "
            f"{[c['caller_id'] for c in result['callers']]!r}, fixture expects "
            f"{expected['caller_ids']!r}\n  {case['note']}"
        )

    for field, want in expected.items():
        if field.startswith("hourly_distribution_") or field == "caller_ids":
            continue
        assert (
            result[field] == pytest.approx(want) if isinstance(want, (int, float)) else (result[field] == want)
        ), f"{case['id']}: {field} is {result[field]!r}, fixture expects {want!r}\n  {case['note']}"

    if "hourly_distribution_length" in expected:
        hourly = result["hourly_distribution"]
        assert len(hourly) == expected["hourly_distribution_length"]
        key_re = re.compile(expected["hourly_distribution_key_format"])
        for entry in hourly:
            assert key_re.match(entry["hour"]), f"{case['id']}: hour key {entry['hour']!r} is not YYYY-MM-DDTHH"
        assert sum(e["call_count"] for e in hourly) == expected["hourly_distribution_total_calls"]
        if expected["hourly_distribution_sorted_ascending"]:
            hours = [e["hour"] for e in hourly]
            assert hours == sorted(hours)
