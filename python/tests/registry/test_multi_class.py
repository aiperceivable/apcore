"""Tests for multi-class module discovery (PROTOCOL_SPEC §2.1.1).

Covers: snake_case segment derivation, single-class identity guarantee,
multi-class ID assignment, conflict detection, grammar validation,
ID length limit, and the @multi_class decorator.
"""

from __future__ import annotations

import re
import textwrap
from pathlib import Path

import pytest

from apcore.errors import IdTooLongError, InvalidSegmentError, ModuleIdConflictError
from apcore.registry.multi_class import (
    MAX_MODULE_ID_LEN,
    class_name_to_segment,
    discover_multi_class,
    multi_class,
)

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

_CANONICAL_ID_RE = re.compile(r"^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$")

_MODULE_STUB = textwrap.dedent(
    """\
    from apcore.registry.multi_class import multi_class

    class _FakeSchema:
        @classmethod
        def model_json_schema(cls):
            return {{}}

    {body}
"""
)

_CLASS_TEMPLATE = textwrap.dedent(
    """\
    @multi_class
    class {name}:
        input_schema = _FakeSchema
        output_schema = _FakeSchema
        description = "test"

        def execute(self, inputs, context):
            return {{}}
"""
)


def _write_module_file(tmp_path: Path, filename: str, class_names: list[str]) -> Path:
    """Write a Python file with @multi_class-decorated stub classes."""
    body = "\n".join(_CLASS_TEMPLATE.format(name=n) for n in class_names)
    content = _MODULE_STUB.format(body=body)
    p = tmp_path / filename
    p.write_text(content, encoding="utf-8")
    return p


def _write_undecorated_file(tmp_path: Path, filename: str, class_names: list[str]) -> Path:
    """Write a Python file with Module classes that are NOT decorated with @multi_class."""
    lines = ["class _FakeSchema:\n    @classmethod\n    def model_json_schema(cls): return {}\n"]
    for name in class_names:
        lines.append(
            f"class {name}:\n"
            f"    input_schema = _FakeSchema\n"
            f"    output_schema = _FakeSchema\n"
            f"    description = 'test'\n"
            f"    def execute(self, inputs, context): return {{}}\n"
        )
    p = tmp_path / filename
    p.write_text("\n".join(lines), encoding="utf-8")
    return p


# ---------------------------------------------------------------------------
# class_name_to_segment: spec-defined conversion table
# ---------------------------------------------------------------------------


class TestClassNameToSegment:
    def test_addition(self) -> None:
        assert class_name_to_segment("Addition") == "addition"

    def test_math_ops(self) -> None:
        assert class_name_to_segment("MathOps") == "math_ops"

    def test_http_sender(self) -> None:
        assert class_name_to_segment("HTTPSender") == "http_sender"

    def test_my_module_v2(self) -> None:
        assert class_name_to_segment("MyModule_V2") == "my_module_v2"

    def test_all_lowercase(self) -> None:
        assert class_name_to_segment("simple") == "simple"

    def test_consecutive_non_alphanumeric_collapsed(self) -> None:
        # "My__Module" → underscores collapsed to one
        assert class_name_to_segment("My__Module") == "my_module"

    def test_leading_non_alphanumeric_stripped(self) -> None:
        # "_MyModule" → leading _ stripped after conversion
        assert class_name_to_segment("_MyModule") == "my_module"

    def test_subtraction(self) -> None:
        assert class_name_to_segment("Subtraction") == "subtraction"

    def test_single_char(self) -> None:
        assert class_name_to_segment("A") == "a"

    def test_conflict_pair_my_module_and_my_underscore_module(self) -> None:
        # Both map to the same segment — used by conflict detection tests
        assert class_name_to_segment("MyModule") == class_name_to_segment("My_Module")

    def test_conflict_pair_http_client(self) -> None:
        assert class_name_to_segment("HTTPClient") == class_name_to_segment("Http_Client")


# ---------------------------------------------------------------------------
# discover_multi_class: single-class identity guarantee (fixture: single_class_id_unchanged)
# ---------------------------------------------------------------------------


class TestSingleClassIdentity:
    def test_single_class_returns_base_id(self, tmp_path: Path) -> None:
        """A file with one @multi_class class returns bare base_id, not base_id.segment."""
        ext_root = tmp_path / "extensions" / "math"
        ext_root.mkdir(parents=True)
        fp = _write_module_file(ext_root, "math_ops.py", ["MathOps"])

        result = discover_multi_class(fp, extensions_root="extensions")

        assert len(result) == 1
        module_id, cls = result[0]
        assert module_id == "math.math_ops"
        assert cls.__name__ == "MathOps"

    def test_single_class_id_matches_scanner_convention(self, tmp_path: Path) -> None:
        """Single-class result ID matches the path-derived canonical_id."""
        ext_root = tmp_path / "extensions" / "executor" / "math"
        ext_root.mkdir(parents=True)
        fp = _write_module_file(ext_root, "arithmetic.py", ["Addition"])

        result = discover_multi_class(fp, extensions_root="extensions")

        assert result[0][0] == "executor.math.arithmetic"


# ---------------------------------------------------------------------------
# discover_multi_class: two classes → distinct IDs (fixture: two_classes_distinct_ids)
# ---------------------------------------------------------------------------


class TestTwoClassesDistinctIds:
    def test_two_classes_get_separate_ids(self, tmp_path: Path) -> None:
        ext_root = tmp_path / "extensions" / "math"
        ext_root.mkdir(parents=True)
        fp = _write_module_file(ext_root, "math_ops.py", ["Addition", "Subtraction"])

        result = discover_multi_class(fp, extensions_root="extensions")

        ids = {mid for mid, _ in result}
        assert ids == {"math.math_ops.addition", "math.math_ops.subtraction"}

    def test_two_classes_class_refs_are_correct(self, tmp_path: Path) -> None:
        ext_root = tmp_path / "extensions" / "math"
        ext_root.mkdir(parents=True)
        fp = _write_module_file(ext_root, "math_ops.py", ["Addition", "Subtraction"])

        result = discover_multi_class(fp, extensions_root="extensions")

        class_names = {cls.__name__ for _, cls in result}
        assert class_names == {"Addition", "Subtraction"}


# ---------------------------------------------------------------------------
# discover_multi_class: grammar conformance (fixture: full_id_grammar_valid)
# ---------------------------------------------------------------------------


class TestGrammarConformance:
    def test_derived_ids_match_canonical_grammar(self, tmp_path: Path) -> None:
        ext_root = tmp_path / "extensions" / "executor" / "math"
        ext_root.mkdir(parents=True)
        fp = _write_module_file(ext_root, "arithmetic.py", ["Addition", "Subtraction"])

        result = discover_multi_class(fp, extensions_root="extensions")

        for module_id, _ in result:
            assert _CANONICAL_ID_RE.match(module_id), f"'{module_id}' does not match canonical ID grammar"


# ---------------------------------------------------------------------------
# discover_multi_class: conflict detection (fixture: conflict_same_segment)
# ---------------------------------------------------------------------------


class TestConflictDetection:
    def test_same_segment_raises_module_id_conflict(self, tmp_path: Path) -> None:
        """MyModule and My_Module both produce 'my_module' → MODULE_ID_CONFLICT."""
        ext_root = tmp_path / "extensions" / "math"
        ext_root.mkdir(parents=True)
        fp = _write_module_file(ext_root, "math_ops.py", ["MyModule", "My_Module"])

        with pytest.raises(ModuleIdConflictError) as exc_info:
            discover_multi_class(fp, extensions_root="extensions")

        err = exc_info.value
        assert err.code == "MODULE_ID_CONFLICT"
        assert err.details["conflicting_segment"] == "my_module"
        assert set(err.details["class_names"]) == {"MyModule", "My_Module"}
        assert str(fp) in err.details["file_path"]

    def test_conflict_registers_no_modules(self, tmp_path: Path) -> None:
        """On conflict, discover_multi_class must raise before returning any results."""
        ext_root = tmp_path / "extensions" / "math"
        ext_root.mkdir(parents=True)
        fp = _write_module_file(ext_root, "math_ops.py", ["MyModule", "My_Module"])

        with pytest.raises(ModuleIdConflictError):
            discover_multi_class(fp, extensions_root="extensions")
        # The raise itself proves no partial list was returned


# ---------------------------------------------------------------------------
# discover_multi_class: disabled by default (fixture: disabled_by_default)
# ---------------------------------------------------------------------------


class TestDisabledByDefault:
    def test_undecorated_classes_are_ignored(self, tmp_path: Path) -> None:
        """Classes without @multi_class are not returned, even if they look like Modules."""
        ext_root = tmp_path / "extensions" / "math"
        ext_root.mkdir(parents=True)
        fp = _write_undecorated_file(ext_root, "math_ops.py", ["Addition", "Subtraction"])

        result = discover_multi_class(fp, extensions_root="extensions")

        assert result == []

    def test_mixed_decorated_and_undecorated(self, tmp_path: Path) -> None:
        """Only the @multi_class-decorated class is returned, under its segment (D-147)."""
        content = textwrap.dedent(
            """\
            from apcore.registry.multi_class import multi_class

            class _FakeSchema:
                @classmethod
                def model_json_schema(cls): return {}

            @multi_class
            class Addition:
                input_schema = _FakeSchema
                output_schema = _FakeSchema
                description = "test"
                def execute(self, inputs, context): return {}  # noqa: ARG002

            class Subtraction:
                input_schema = _FakeSchema
                output_schema = _FakeSchema
                description = "test"
                def execute(self, inputs, context): return {}  # noqa: ARG002
        """
        )
        ext_root = tmp_path / "extensions" / "math"
        ext_root.mkdir(parents=True)
        fp = ext_root / "math_ops.py"
        fp.write_text(content, encoding="utf-8")

        result = discover_multi_class(fp, extensions_root="extensions")

        assert len(result) == 1
        # Two Module classes in the file: the single-class identity rule does
        # not apply, so the marked one keeps its segment.
        assert result[0][0] == "math.math_ops.addition"
        assert result[0][1].__name__ == "Addition"


# ---------------------------------------------------------------------------
# discover_multi_class: INVALID_SEGMENT error
# ---------------------------------------------------------------------------


class TestInvalidSegment:
    def test_empty_segment_is_caught_by_segment_re(self) -> None:
        """class_name_to_segment('_') produces '' which fails the segment grammar guard."""
        from apcore.registry.multi_class import _SEGMENT_RE

        segment = class_name_to_segment("_")
        assert segment == ""
        assert not _SEGMENT_RE.match(segment)

    def test_invalid_segment_error_attributes(self) -> None:
        """InvalidSegmentError carries the expected code and details."""
        err = InvalidSegmentError(file_path="/ext/mod.py", class_name="MyClass", segment="")
        assert err.code == "INVALID_SEGMENT"
        assert err.details["file_path"] == "/ext/mod.py"
        assert err.details["class_name"] == "MyClass"
        assert err.details["segment"] == ""


# ---------------------------------------------------------------------------
# discover_multi_class: ID_TOO_LONG error
# ---------------------------------------------------------------------------


class TestIdTooLong:
    def test_id_exceeding_192_chars_raises_id_too_long(self, tmp_path: Path) -> None:
        # Construct a very deep path so that base_id + segment > 192 chars
        # base_id will be: "a" * 95 repeated through path segments
        deep_path = tmp_path / "extensions"
        segment_name = "a" * 30
        for _ in range(5):
            deep_path = deep_path / segment_name
        deep_path.mkdir(parents=True)

        long_stem = "x" * 50
        fp = _write_module_file(deep_path, f"{long_stem}.py", ["Addition", "Subtraction"])

        with pytest.raises(IdTooLongError) as exc_info:
            discover_multi_class(fp, extensions_root="extensions")

        err = exc_info.value
        assert err.code == "ID_TOO_LONG"
        assert err.details["length"] > MAX_MODULE_ID_LEN


# ---------------------------------------------------------------------------
# @multi_class decorator
# ---------------------------------------------------------------------------


class TestMultiClassDecorator:
    def test_decorator_sets_marker_attribute(self) -> None:
        @multi_class
        class MyMod:
            pass

        assert getattr(MyMod, "_apcore_multi_class", False) is True

    def test_decorator_returns_original_class(self) -> None:
        class Original:
            pass

        result = multi_class(Original)
        assert result is Original

    def test_decorator_is_idempotent(self) -> None:
        @multi_class
        @multi_class
        class MyMod:
            pass

        assert getattr(MyMod, "_apcore_multi_class", False) is True


# ---------------------------------------------------------------------------
# discover_multi_class: pre_approval_hook
# ---------------------------------------------------------------------------


class TestPreApprovalHook:
    def test_hook_is_called_with_file_path(self, tmp_path: Path) -> None:
        """pre_approval_hook receives the file path before import."""
        ext_root = tmp_path / "extensions" / "math"
        ext_root.mkdir(parents=True)
        fp = _write_module_file(ext_root, "math_ops.py", ["Addition"])

        seen: list[Path] = []
        discover_multi_class(fp, extensions_root="extensions", pre_approval_hook=seen.append)

        assert seen == [fp]

    def test_hook_rejection_raises_module_load_error(self, tmp_path: Path) -> None:
        """Raising from the hook wraps as ModuleLoadError."""
        from apcore.errors import ModuleLoadError

        ext_root = tmp_path / "extensions" / "math"
        ext_root.mkdir(parents=True)
        fp = _write_module_file(ext_root, "math_ops.py", ["Addition"])

        def rejecting_hook(_: Path) -> None:
            raise PermissionError("not allowed")

        with pytest.raises(ModuleLoadError):
            discover_multi_class(fp, extensions_root="extensions", pre_approval_hook=rejecting_hook)

    def test_no_hook_works_by_default(self, tmp_path: Path) -> None:
        """Omitting pre_approval_hook does not change existing behavior."""
        ext_root = tmp_path / "extensions" / "math"
        ext_root.mkdir(parents=True)
        fp = _write_module_file(ext_root, "math_ops.py", ["Addition"])

        result = discover_multi_class(fp, extensions_root="extensions")

        assert len(result) == 1


# ---------------------------------------------------------------------------
# discover_multi_class: empty file
# ---------------------------------------------------------------------------


class TestEmptyFile:
    def test_no_qualifying_classes_returns_empty(self, tmp_path: Path) -> None:
        fp = tmp_path / "empty_mod.py"
        fp.write_text("# no classes\n", encoding="utf-8")

        result = discover_multi_class(fp, extensions_root="extensions")

        assert result == []
