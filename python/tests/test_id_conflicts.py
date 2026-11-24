"""Tests for Algorithm A03: ID conflict detection."""

from __future__ import annotations

from typing import Any

import pytest
from pydantic import BaseModel

from apcore.errors import InvalidInputError
from apcore.registry.conflicts import ConflictResult, detect_id_conflicts
from apcore.registry.registry import RESERVED_WORDS, Registry


# ---------------------------------------------------------------------------
# Helper module classes
# ---------------------------------------------------------------------------


class _TestInput(BaseModel):
    value: str


class _TestOutput(BaseModel):
    result: str


class _ValidModule:
    """A valid duck-typed test module."""

    input_schema = _TestInput
    output_schema = _TestOutput
    description = "A valid test module"

    def execute(self, inputs: dict[str, Any], _context: Any = None) -> dict[str, Any]:
        return {"result": inputs["value"]}


# ===========================================================================
# Unit tests for detect_id_conflicts()
# ===========================================================================


class TestDetectIdConflictsNoConflict:
    def test_no_conflict_returns_none(self) -> None:
        """New ID with no matches returns None."""
        result = detect_id_conflicts(
            new_id="my.module",
            existing_ids={"other.module", "another.module"},
            reserved_words=RESERVED_WORDS,
        )
        assert result is None

    def test_empty_existing_ids_returns_none(self) -> None:
        """New ID with empty existing set returns None."""
        result = detect_id_conflicts(
            new_id="my.module",
            existing_ids=set(),
            reserved_words=RESERVED_WORDS,
        )
        assert result is None


class TestDetectIdConflictsDuplicate:
    def test_exact_duplicate_returns_error(self) -> None:
        """Exact match returns ConflictResult with type='duplicate_id' and severity='error'."""
        result = detect_id_conflicts(
            new_id="my.module",
            existing_ids={"my.module", "other.module"},
            reserved_words=RESERVED_WORDS,
        )
        assert result is not None
        assert result.type == "duplicate_id"
        assert result.severity == "error"
        assert "already registered" in result.message

    def test_duplicate_id_takes_priority_over_reserved(self) -> None:
        """When ID is both duplicate and contains reserved word, duplicate is reported first."""
        result = detect_id_conflicts(
            new_id="system",
            existing_ids={"system"},
            reserved_words=RESERVED_WORDS,
        )
        assert result is not None
        assert result.type == "duplicate_id"


class TestDetectIdConflictsReservedWord:
    def test_reserved_word_single_segment(self) -> None:
        """ID that is a reserved word returns error."""
        result = detect_id_conflicts(
            new_id="system",
            existing_ids=set(),
            reserved_words=RESERVED_WORDS,
        )
        assert result is not None
        assert result.type == "reserved_word"
        assert result.severity == "error"
        assert "reserved word" in result.message
        assert "'system'" in result.message

    def test_reserved_word_in_first_segment(self) -> None:
        """ID starting with a reserved word returns error."""
        result = detect_id_conflicts(
            new_id="internal.my.handler",
            existing_ids=set(),
            reserved_words=RESERVED_WORDS,
        )
        assert result is not None
        assert result.type == "reserved_word"
        assert result.severity == "error"
        assert "'internal'" in result.message

    def test_reserved_word_in_middle_segment_is_legal(self) -> None:
        """ID containing a reserved word in middle segments is legal."""
        result = detect_id_conflicts(
            new_id="my.internal.handler",
            existing_ids=set(),
            reserved_words=RESERVED_WORDS,
        )
        assert result is None

    def test_all_reserved_words_detected(self) -> None:
        """Each reserved word is detected when used as a segment."""
        for word in RESERVED_WORDS:
            result = detect_id_conflicts(
                new_id=word,
                existing_ids=set(),
                reserved_words=RESERVED_WORDS,
            )
            assert result is not None, f"Reserved word '{word}' was not detected"
            assert result.type == "reserved_word"


class TestDetectIdConflictsCaseCollision:
    def test_case_collision_without_map(self) -> None:
        """Case collision detected via linear scan when no lowercase_map provided."""
        result = detect_id_conflicts(
            new_id="Api.Handler",
            existing_ids={"api.handler", "other.module"},
            reserved_words=frozenset(),
        )
        assert result is not None
        assert result.type == "case_collision"
        assert result.severity == "warning"
        assert "case collision" in result.message
        assert "'api.handler'" in result.message

    def test_case_collision_with_lowercase_map(self) -> None:
        """Case collision detected via O(1) lowercase_map lookup."""
        lowercase_map = {"api.handler": "api.handler"}
        result = detect_id_conflicts(
            new_id="Api.Handler",
            existing_ids={"api.handler"},
            reserved_words=frozenset(),
            lowercase_map=lowercase_map,
        )
        assert result is not None
        assert result.type == "case_collision"
        assert result.severity == "warning"
        assert "'api.handler'" in result.message

    def test_no_case_collision_for_same_id(self) -> None:
        """Same-cased ID in lowercase_map does not trigger collision (it would be a duplicate instead)."""
        lowercase_map = {"my.module": "my.module"}
        result = detect_id_conflicts(
            new_id="my.module",
            existing_ids={"my.module"},
            reserved_words=frozenset(),
            lowercase_map=lowercase_map,
        )
        # This should be a duplicate_id, not a case_collision
        assert result is not None
        assert result.type == "duplicate_id"

    def test_no_false_positive_case_collision(self) -> None:
        """IDs with different lowercase forms do not trigger case collision."""
        result = detect_id_conflicts(
            new_id="foo.bar",
            existing_ids={"baz.qux"},
            reserved_words=frozenset(),
        )
        assert result is None

    def test_case_collision_with_empty_lowercase_map(self) -> None:
        """Empty lowercase_map means no case collision found."""
        result = detect_id_conflicts(
            new_id="Api.Handler",
            existing_ids=set(),
            reserved_words=frozenset(),
            lowercase_map={},
        )
        assert result is None


class TestConflictResultDataclass:
    def test_frozen(self) -> None:
        """ConflictResult is immutable (frozen dataclass)."""
        result = ConflictResult(type="duplicate_id", severity="error", message="test")
        with pytest.raises(AttributeError):
            result.type = "other"  # type: ignore[misc]

    def test_equality(self) -> None:
        """Two ConflictResult with same fields are equal."""
        a = ConflictResult(type="duplicate_id", severity="error", message="test")
        b = ConflictResult(type="duplicate_id", severity="error", message="test")
        assert a == b


# ===========================================================================
# Integration tests with Registry
# ===========================================================================


class TestRegistryIntegrationDuplicate:
    def test_register_raises_on_duplicate(self) -> None:
        """Registry.register() raises InvalidInputError on duplicate ID."""
        reg = Registry()
        reg.register("test.module", _ValidModule())
        with pytest.raises(InvalidInputError, match="already registered"):
            reg.register("test.module", _ValidModule())

    def test_register_raises_on_reserved_word(self) -> None:
        """Registry.register() raises InvalidInputError on reserved word."""
        reg = Registry()
        with pytest.raises(InvalidInputError, match="reserved word"):
            reg.register("system", _ValidModule())

    def test_register_raises_on_reserved_word_segment(self) -> None:
        """Registry.register() raises InvalidInputError when first segment is a reserved word."""
        reg = Registry()
        with pytest.raises(InvalidInputError, match="reserved word"):
            reg.register("internal.my.module", _ValidModule())


class TestRegistryIntegrationCaseCollision:
    def test_register_warns_on_case_collision(self, caplog: pytest.LogCaptureFixture) -> None:
        """Registry.register() logs a warning on case collision.

        Note: MODULE_ID_PATTERN enforces lowercase-only, so case collision
        cannot happen through register() in practice. We test the function
        directly instead.
        """
        # Directly test detect_id_conflicts since register() enforces lowercase
        result = detect_id_conflicts(
            new_id="Api.Handler",
            existing_ids={"api.handler"},
            reserved_words=RESERVED_WORDS,
            lowercase_map={"api.handler": "api.handler"},
        )
        assert result is not None
        assert result.type == "case_collision"
        assert result.severity == "warning"


class TestRegistryIntegrationLowercaseMap:
    def test_unregister_cleans_lowercase_map(self) -> None:
        """After register + unregister, _lowercase_map is empty."""
        reg = Registry()
        reg.register("test.module", _ValidModule())
        assert "test.module" in reg._lowercase_map
        reg.unregister("test.module")
        assert len(reg._lowercase_map) == 0

    def test_register_populates_lowercase_map(self) -> None:
        """register() populates _lowercase_map with correct mapping."""
        reg = Registry()
        reg.register("test.module", _ValidModule())
        assert reg._lowercase_map["test.module"] == "test.module"

    def test_multiple_register_unregister_consistency(self) -> None:
        """Multiple register/unregister cycles keep _lowercase_map consistent."""
        reg = Registry()
        reg.register("mod.a", _ValidModule())
        reg.register("mod.b", _ValidModule())
        assert len(reg._lowercase_map) == 2

        reg.unregister("mod.a")
        assert len(reg._lowercase_map) == 1
        assert "mod.b" in reg._lowercase_map
        assert "mod.a" not in reg._lowercase_map

        reg.unregister("mod.b")
        assert len(reg._lowercase_map) == 0

    def test_safe_unregister_cleans_lowercase_map(self) -> None:
        """safe_unregister() also cleans _lowercase_map via unregister()."""
        reg = Registry()
        reg.register("test.module", _ValidModule())
        assert "test.module" in reg._lowercase_map
        reg.safe_unregister("test.module")
        assert len(reg._lowercase_map) == 0


class TestRegistryDiscoverBatchConflict:
    def test_discover_skips_reserved_word_modules(self, tmp_path: pytest.TempPathFactory) -> None:
        """discover() skips modules whose canonical ID contains reserved words."""
        ext = tmp_path / "extensions"  # type: ignore[operator]
        ext.mkdir()

        # Create a module file whose canonical ID (stem) is "system" -- a reserved word
        (ext / "system.py").write_text(
            """from pydantic import BaseModel

class TestInput(BaseModel):
    value: str

class TestOutput(BaseModel):
    result: str

class SystemModule:
    input_schema = TestInput
    output_schema = TestOutput
    description = "System module"

    def execute(self, inputs, context=None):
        return {"result": inputs["value"]}
"""
        )
        # Also create a normal module
        (ext / "normal.py").write_text(
            """from pydantic import BaseModel

class TestInput(BaseModel):
    value: str

class TestOutput(BaseModel):
    result: str

class NormalModule:
    input_schema = TestInput
    output_schema = TestOutput
    description = "Normal module"

    def execute(self, inputs, context=None):
        return {"result": inputs["value"]}
"""
        )

        reg = Registry(extensions_dir=str(ext))
        count = reg.discover()
        # "system" should be skipped due to reserved word
        assert not reg.has("system")
        assert reg.has("normal")
        assert count == 1
