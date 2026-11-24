"""Tests for dependency resolution via Kahn's topological sort."""

from __future__ import annotations

import logging

import pytest

from apcore.errors import (
    CircularDependencyError,
    DependencyNotFoundError,
    DependencyVersionMismatchError,
)
from apcore.registry.dependencies import resolve_dependencies
from apcore.registry.types import DependencyInfo


class TestNoDependencies:
    def test_no_deps_returns_all(self) -> None:
        """Modules with no dependencies all appear in output."""
        result = resolve_dependencies([("A", []), ("B", []), ("C", [])])
        assert set(result) == {"A", "B", "C"}


class TestSimpleOrdering:
    def test_a_depends_on_b(self) -> None:
        """A depends on B -> B before A."""
        result = resolve_dependencies(
            [
                ("A", [DependencyInfo(module_id="B")]),
                ("B", []),
            ]
        )
        assert result.index("B") < result.index("A")

    def test_chain(self) -> None:
        """Chain A -> B -> C -> order is C, B, A."""
        result = resolve_dependencies(
            [
                ("A", [DependencyInfo(module_id="B")]),
                ("B", [DependencyInfo(module_id="C")]),
                ("C", []),
            ]
        )
        assert result == ["C", "B", "A"]

    def test_diamond(self) -> None:
        """Diamond: A -> B,C; B,C -> D; D first, A last."""
        result = resolve_dependencies(
            [
                ("A", [DependencyInfo(module_id="B"), DependencyInfo(module_id="C")]),
                ("B", [DependencyInfo(module_id="D")]),
                ("C", [DependencyInfo(module_id="D")]),
                ("D", []),
            ]
        )
        assert result[0] == "D"
        assert result[-1] == "A"
        assert result.index("B") < result.index("A")
        assert result.index("C") < result.index("A")


class TestCircularDetection:
    def test_simple_cycle(self) -> None:
        """A -> B -> A raises CircularDependencyError."""
        with pytest.raises(CircularDependencyError) as exc_info:
            resolve_dependencies(
                [
                    ("A", [DependencyInfo(module_id="B")]),
                    ("B", [DependencyInfo(module_id="A")]),
                ]
            )
        assert "A" in exc_info.value.details["cycle_path"]
        assert "B" in exc_info.value.details["cycle_path"]

    def test_three_node_cycle(self) -> None:
        """A -> B -> C -> A raises CircularDependencyError."""
        with pytest.raises(CircularDependencyError) as exc_info:
            resolve_dependencies(
                [
                    ("A", [DependencyInfo(module_id="B")]),
                    ("B", [DependencyInfo(module_id="C")]),
                    ("C", [DependencyInfo(module_id="A")]),
                ]
            )
        path = exc_info.value.details["cycle_path"]
        assert "A" in path and "B" in path and "C" in path

    def test_partial_cycle_with_independent(self) -> None:
        """Partial cycle B <-> C with independent D raises CircularDependencyError."""
        with pytest.raises(CircularDependencyError) as exc_info:
            resolve_dependencies(
                [
                    ("A", [DependencyInfo(module_id="B")]),
                    ("B", [DependencyInfo(module_id="C")]),
                    ("C", [DependencyInfo(module_id="B")]),
                    ("D", []),
                ]
            )
        path = exc_info.value.details["cycle_path"]
        assert "B" in path and "C" in path

    def test_cycle_path_is_actual_cycle(self) -> None:
        """cycle_path must only contain nodes that form a real cycle.

        Regression: when a module blocked on an external (known-but-not-in-batch)
        dependency sits alongside a true cycle, the extractor must not lump the
        blocked module into the reported cycle path.
        """
        with pytest.raises(CircularDependencyError) as exc_info:
            resolve_dependencies(
                modules=[
                    ("A", [DependencyInfo(module_id="B")]),
                    ("B", [DependencyInfo(module_id="A")]),
                    ("C", [DependencyInfo(module_id="external")]),
                ],
                known_ids={"A", "B", "C", "external"},
            )
        path = exc_info.value.details["cycle_path"]
        assert path[0] == path[-1], "cycle path must start and end at same node"
        assert set(path[:-1]) == {"A", "B"}, f"expected cycle over {{A, B}}, got {path}"


class TestOptionalDependencies:
    def test_missing_optional_dep_warns(self, caplog: pytest.LogCaptureFixture) -> None:
        """Missing optional dep logs warning, still resolves."""
        with caplog.at_level(logging.WARNING):
            result = resolve_dependencies(
                [
                    ("A", [DependencyInfo(module_id="missing_dep", optional=True)]),
                ]
            )
        assert result == ["A"]
        assert "missing_dep" in caplog.text

    def test_missing_required_dep_raises(self) -> None:
        """Missing required dep raises DependencyNotFoundError with DEPENDENCY_NOT_FOUND code."""
        with pytest.raises(DependencyNotFoundError) as exc_info:
            resolve_dependencies(
                [
                    ("A", [DependencyInfo(module_id="missing_dep", optional=False)]),
                ]
            )
        assert exc_info.value.code == "DEPENDENCY_NOT_FOUND"
        assert exc_info.value.details["module_id"] == "A"
        assert exc_info.value.details["dependency_id"] == "missing_dep"

    def test_optional_dep_present_included(self) -> None:
        """Optional dep present is included in ordering."""
        result = resolve_dependencies(
            [
                ("A", [DependencyInfo(module_id="B", optional=True)]),
                ("B", []),
            ]
        )
        assert result == ["B", "A"]


class TestEdgeCases:
    def test_empty_input(self) -> None:
        """Empty input returns empty list."""
        assert resolve_dependencies([]) == []

    def test_single_module(self) -> None:
        """Single module with no deps returns [module_id]."""
        assert resolve_dependencies([("A", [])]) == ["A"]


class TestVersionConstraints:
    def test_version_constraint_satisfied(self) -> None:
        result = resolve_dependencies(
            modules=[
                ("A", [DependencyInfo(module_id="B", version=">=1.0.0")]),
                ("B", []),
            ],
            module_versions={"A": "1.0.0", "B": "1.2.3"},
        )
        assert result == ["B", "A"]

    def test_version_constraint_violated_raises(self) -> None:
        with pytest.raises(DependencyVersionMismatchError) as exc_info:
            resolve_dependencies(
                modules=[
                    ("A", [DependencyInfo(module_id="B", version=">=2.0.0")]),
                    ("B", []),
                ],
                module_versions={"A": "1.0.0", "B": "1.2.3"},
            )
        details = exc_info.value.details
        assert details["module_id"] == "A"
        assert details["dependency_id"] == "B"
        assert details["required"] == ">=2.0.0"
        assert details["actual"] == "1.2.3"

    def test_caret_constraint(self) -> None:
        # ^1.2.3 = >=1.2.3, <2.0.0
        resolve_dependencies(
            modules=[
                ("A", [DependencyInfo(module_id="B", version="^1.2.3")]),
                ("B", []),
            ],
            module_versions={"A": "1.0.0", "B": "1.9.0"},
        )
        with pytest.raises(DependencyVersionMismatchError):
            resolve_dependencies(
                modules=[
                    ("A", [DependencyInfo(module_id="B", version="^1.2.3")]),
                    ("B", []),
                ],
                module_versions={"A": "1.0.0", "B": "2.0.0"},
            )

    def test_tilde_constraint(self) -> None:
        # ~1.2.3 = >=1.2.3, <1.3.0
        resolve_dependencies(
            modules=[
                ("A", [DependencyInfo(module_id="B", version="~1.2.3")]),
                ("B", []),
            ],
            module_versions={"A": "1.0.0", "B": "1.2.9"},
        )
        with pytest.raises(DependencyVersionMismatchError):
            resolve_dependencies(
                modules=[
                    ("A", [DependencyInfo(module_id="B", version="~1.2.3")]),
                    ("B", []),
                ],
                module_versions={"A": "1.0.0", "B": "1.3.0"},
            )

    def test_range_constraint(self) -> None:
        # >=1.0.0,<2.0.0
        resolve_dependencies(
            modules=[
                ("A", [DependencyInfo(module_id="B", version=">=1.0.0,<2.0.0")]),
                ("B", []),
            ],
            module_versions={"A": "1.0.0", "B": "1.5.0"},
        )
        with pytest.raises(DependencyVersionMismatchError):
            resolve_dependencies(
                modules=[
                    ("A", [DependencyInfo(module_id="B", version=">=1.0.0,<2.0.0")]),
                    ("B", []),
                ],
                module_versions={"A": "1.0.0", "B": "2.0.1"},
            )

    def test_constraint_without_module_versions_is_skipped(self) -> None:
        # When module_versions is not provided, version field is ignored.
        result = resolve_dependencies(
            modules=[
                ("A", [DependencyInfo(module_id="B", version=">=99.0.0")]),
                ("B", []),
            ],
        )
        assert result == ["B", "A"]

    def test_optional_version_mismatch_skipped_with_warning(self, caplog: pytest.LogCaptureFixture) -> None:
        with caplog.at_level(logging.WARNING):
            result = resolve_dependencies(
                modules=[
                    (
                        "A",
                        [DependencyInfo(module_id="B", version=">=2.0.0", optional=True)],
                    ),
                    ("B", []),
                ],
                module_versions={"A": "1.0.0", "B": "1.0.0"},
            )
        # Optional + version mismatch -> B is skipped (A has no hard dep), so A loads alone
        assert "A" in result
        assert "does not satisfy" in caplog.text

    def test_no_version_declared_accepts_any(self) -> None:
        # dep.version is None -> no constraint checking regardless of module_versions.
        result = resolve_dependencies(
            modules=[
                ("A", [DependencyInfo(module_id="B")]),
                ("B", []),
            ],
            module_versions={"A": "1.0.0", "B": "99.0.0"},
        )
        assert result == ["B", "A"]


class TestNonCycleBlockage:
    """Regression: non-cycle blockages must surface as ModuleLoadError, not CircularDependencyError."""

    def test_non_cycle_blockage_raises_module_load_error(self) -> None:
        """Artificial scenario where Kahn's sort stalls without a true cycle.

        Construct a graph where two modules mutually depend via an
        optional-then-required edge that only one side sees, leaving both
        stuck at non-zero in-degree with no back-edge. The resolver must not
        raise CircularDependencyError with a phantom path — it must raise
        ModuleLoadError naming the blocked modules.
        """
        from apcore.errors import ModuleLoadError

        # Use manually constructed in-degree state to simulate a blockage
        # without a true cycle. In practice this is reached when an optional
        # dep filter drops half a cycle on one side only.
        # Simpler approach: a chain A -> B where B's in-degree is
        # corrupted by a missing optional dep path is not reachable via the
        # public API; instead we verify the error type by constructing a
        # real cycle and confirming the DIFFERENT code path that raises
        # ModuleLoadError when _find_back_edge_cycle returns None. Since
        # that path is unreachable via the public surface without injecting
        # state, assert via mocking.
        import apcore.registry.dependencies as deps_mod

        original = deps_mod._find_back_edge_cycle
        try:
            deps_mod._find_back_edge_cycle = lambda *args, **kwargs: None  # type: ignore[assignment]
            with pytest.raises(ModuleLoadError) as exc_info:
                resolve_dependencies(
                    modules=[
                        ("A", [DependencyInfo(module_id="B")]),
                        ("B", [DependencyInfo(module_id="A")]),
                    ],
                )
            assert exc_info.value.code == "MODULE_LOAD_ERROR"
            assert "A" in exc_info.value.details["module_id"]
            assert "B" in exc_info.value.details["module_id"]
        finally:
            deps_mod._find_back_edge_cycle = original
