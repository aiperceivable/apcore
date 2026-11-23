"""Tests for Registry.discover_multi_class instance method (D-15).

Verifies that the new ``Registry.discover_multi_class(file_path)`` method
delegates to the underlying free function and returns the same results.
"""

from __future__ import annotations

import textwrap
from pathlib import Path

from apcore.registry import Registry
from apcore.registry.multi_class import discover_multi_class

_STUB = textwrap.dedent(
    """\
    from apcore.registry.multi_class import multi_class

    class _FakeSchema:
        @classmethod
        def model_json_schema(cls):
            return {}

    @multi_class
    class Addition:
        input_schema = _FakeSchema
        output_schema = _FakeSchema
        description = "test"
        def execute(self, inputs, context): return {}

    @multi_class
    class Subtraction:
        input_schema = _FakeSchema
        output_schema = _FakeSchema
        description = "test"
        def execute(self, inputs, context): return {}
    """
)


class TestRegistryDiscoverMultiClassMethod:
    def test_method_exists(self) -> None:
        reg = Registry()
        assert callable(getattr(reg, "discover_multi_class", None))

    def test_method_returns_same_as_free_function(self, tmp_path: Path) -> None:
        ext_root = tmp_path / "extensions" / "math"
        ext_root.mkdir(parents=True)
        fp = ext_root / "ops.py"
        fp.write_text(_STUB, encoding="utf-8")

        reg = Registry()
        method_result = reg.discover_multi_class(fp)
        free_result = discover_multi_class(fp)

        # Compare by (id, class.__name__) since class identity may differ
        # between two distinct file imports.
        method_pairs = sorted((mid, cls.__name__) for mid, cls in method_result)
        free_pairs = sorted((mid, cls.__name__) for mid, cls in free_result)
        assert method_pairs == free_pairs

    def test_method_returns_two_segmented_ids(self, tmp_path: Path) -> None:
        ext_root = tmp_path / "extensions" / "math"
        ext_root.mkdir(parents=True)
        fp = ext_root / "ops.py"
        fp.write_text(_STUB, encoding="utf-8")

        reg = Registry()
        result = reg.discover_multi_class(fp)
        ids = sorted(mid for mid, _ in result)
        assert ids == ["math.ops.addition", "math.ops.subtraction"]
