"""Cross-language driver for ``id_conflict_reserved_words.json``.

PROTOCOL_SPEC §2.6 step 2, narrowed to the **first segment** in spec v1.26.0
(#99). A reserved word claims a namespace, not a token, so ``foo.system.bar``
and ``executor.schema.validate`` are legal and ``system.custom_module`` is not.

The driver exercises the **public** ``register()`` path deliberately.
``register_internal()`` bypasses the reserved-word check by design, so running
the cases through it would report agreement while testing nothing.
"""

from __future__ import annotations

from typing import Any

import pytest

from apcore.registry import Registry

from .canonical_fixtures import fixtures_dir, load_fixture

FIXTURE = "id_conflict_reserved_words.json"


def _present() -> bool:
    return (fixtures_dir() / FIXTURE).is_file()


# The fixture lands in the spec repo one push after this driver, so that
# `check_driver_coverage.py --strict` has a driver to find for it. Until then
# the module skips and says which fixture is unexercised — "not verified", not
# "passed".
pytestmark = pytest.mark.skipif(
    not _present(),
    reason=f"{FIXTURE} not in the spec repo yet (spec v1.26.0, #99)",
)


class _Module:
    """Minimal conformant module; the ID is what is under test, not the body."""

    description = "conformance fixture module"

    def input_schema(self) -> dict[str, Any]:
        return {"type": "object"}

    def output_schema(self) -> dict[str, Any]:
        return {"type": "object"}

    async def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        return {}


def _cases() -> list[dict[str, Any]]:
    # Evaluated at collection, before ``pytestmark`` can skip, so it must
    # tolerate the fixture being absent rather than failing the collection.
    return load_fixture(FIXTURE)["test_cases"] if _present() else []


def test_reserved_word_set_matches_the_fixture() -> None:
    """The canonical set lives in the fixture, not in this SDK.

    Reading it from ``apcore`` would let a divergent local list agree with
    itself: every case would be computed from the same wrong set and pass.
    """
    from apcore.registry.registry import RESERVED_WORDS

    assert set(load_fixture(FIXTURE)["reserved_words"]) == set(RESERVED_WORDS)


@pytest.mark.parametrize("case", _cases(), ids=lambda c: c["id"])
def test_id_conflict_reserved_words(case: dict[str, Any]) -> None:
    registry = Registry()
    for existing in case.get("existing_ids", []):
        registry.register(existing, _Module())

    expected = case["expected"]
    if expected is None:
        # Must register cleanly. A raise here is the pre-v1.26.0 per-segment
        # reading resurfacing.
        registry.register(case["new_id"], _Module())
        assert registry.get(case["new_id"]) is not None, case["note"]
    else:
        with pytest.raises(Exception) as excinfo:
            registry.register(case["new_id"], _Module())
        # The fixture names the conflict `type`; SDKs surface it through their
        # own error classes, so the message check identifies the offending id
        # rather than pinning a class name three languages do not share.
        assert case["new_id"].split(".")[0] in str(excinfo.value) or case["new_id"] in str(excinfo.value), case["note"]

        # ...and the WIRE CODE says WHICH conflict it was. Without this the
        # driver only asserts "refused", so `check_case_pinning.py` could mutate
        # `expected` from `reserved_word` to `duplicate_id` — a value this very
        # fixture uses — and every SDK stayed green: the distinction the fixture
        # exists to draw was asserted by nobody. The mapping lives in the
        # fixture, not here, because it is one rule and §8's codes are the
        # cross-language contract.
        codes = load_fixture(FIXTURE)["error_code_by_conflict"]
        assert getattr(excinfo.value, "code", None) == codes[expected], (
            f"{case['id']}: expected the {expected!r} conflict to surface as "
            f"{codes[expected]}, got {getattr(excinfo.value, 'code', None)!r} — {case['note']}"
        )
