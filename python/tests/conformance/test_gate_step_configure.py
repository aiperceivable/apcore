"""Drive ``gate_step_configure.json`` — ``configure`` cannot weaken a governance gate.

PROTOCOL_SPEC §5.16.1 (D-130). On the built-in gate steps ``acl_check`` and
``approval_gate``, ``ignore_errors: true``, any ``match_modules`` and
``pure: true`` MUST be rejected with ``PIPELINE_CONFIGURATION_ERROR`` while the
configuration is turned into a strategy.

``driver_contract.path``: the strategy is built from ``input.yaml`` through the
same public config path ``pipeline_failfast_config.json`` uses
(``build_strategy_from_config``); nothing is executed.
``driver_contract.assert_the_wire_code``: the error CODE is asserted, not the
class name.
"""

from __future__ import annotations

from typing import Any

import pytest

from apcore.errors import ModuleError
from apcore.pipeline_config import build_strategy_from_config
from conformance.canonical_fixtures import case_ids, load_fixture

FIXTURE = load_fixture("gate_step_configure.json")
CASES = FIXTURE["test_cases"]


class _StubRegistry:
    """``build_standard_strategy`` only needs an object with these two methods."""

    def get(self, *args: Any, **kwargs: Any) -> None:
        return None

    def discover(self, *args: Any, **kwargs: Any) -> list[Any]:
        return []


@pytest.mark.parametrize("case", CASES, ids=case_ids("gate_step_configure.json"))
def test_gate_step_configure(case: dict[str, Any]) -> None:
    pipeline_section = case["input"]["yaml"]["pipeline"]
    expected = case["expected"]

    error: ModuleError | None = None
    strategy: Any = None
    try:
        strategy = build_strategy_from_config(pipeline_section, registry=_StubRegistry())
    except ModuleError as exc:
        error = exc

    assert (error is not None) is expected["raises"], (
        f"[{case['id']}] expected raises={expected['raises']}, got "
        f"{'no error' if error is None else f'{error.code}: {error}'}"
    )
    if error is None:
        assert strategy is not None
        return

    assert error.code == expected["error_code"], (
        f"[{case['id']}] the WIRE CODE is the contract, not the class name: "
        f"got {error.code!r}, expected {expected['error_code']!r}"
    )
    for needle in expected.get("error_message_contains", []):
        assert needle in str(error), f"[{case['id']}] error message must name {needle!r}; got: {error}"


def test_every_fixture_case_is_driven() -> None:
    known_expected = {"raises", "error_code", "error_message_contains"}
    for case in CASES:
        unread = set(case["expected"]) - known_expected
        assert not unread, f"[{case['id']}] expected keys this driver does not assert: {sorted(unread)}"
