"""Tests for the extension point framework."""

from __future__ import annotations

from typing import Any
from unittest.mock import MagicMock

import pytest

from apcore.acl import ACL
from apcore.errors import InvalidInputError
from apcore.extensions import ExtensionManager, ExtensionPoint
from apcore.middleware import Middleware
from apcore.observability.tracing import InMemoryExporter, TracingMiddleware


# ---------------------------------------------------------------------------
# Helpers: concrete implementations that satisfy the protocols
# ---------------------------------------------------------------------------


class StubDiscoverer:
    """Concrete discoverer for testing."""

    def discover(self, roots: list[str]) -> list[dict[str, Any]]:
        return []


class StubValidator:
    """Concrete validator for testing."""

    def validate(self, module: Any) -> list[str]:
        return []


class StubMiddleware(Middleware):
    """Concrete middleware for testing."""


class StubExporter:
    """Concrete span exporter for testing."""

    def export(self, span: Any) -> None:
        pass


# ---------------------------------------------------------------------------
# Tests: ExtensionManager basics
# ---------------------------------------------------------------------------


class TestExtensionManagerInit:
    def test_has_five_built_in_points(self) -> None:
        mgr = ExtensionManager()
        points = mgr.list_points()
        assert len(points) == 6

    def test_built_in_point_names(self) -> None:
        mgr = ExtensionManager()
        names = {p.name for p in mgr.list_points()}
        assert names == {"discoverer", "middleware", "acl", "span_exporter", "module_validator", "approval_handler"}

    def test_list_points_returns_extension_point_instances(self) -> None:
        mgr = ExtensionManager()
        for p in mgr.list_points():
            assert isinstance(p, ExtensionPoint)


# ---------------------------------------------------------------------------
# Tests: register / get / get_all / unregister
# ---------------------------------------------------------------------------


class TestDiscovererExtension:
    def test_register_and_get(self) -> None:
        mgr = ExtensionManager()
        disc = StubDiscoverer()
        mgr.register("discoverer", disc)
        assert mgr.get("discoverer") is disc

    def test_register_replaces_single(self) -> None:
        mgr = ExtensionManager()
        disc1 = StubDiscoverer()
        disc2 = StubDiscoverer()
        mgr.register("discoverer", disc1)
        mgr.register("discoverer", disc2)
        assert mgr.get("discoverer") is disc2

    def test_unregister_returns_true(self) -> None:
        mgr = ExtensionManager()
        disc = StubDiscoverer()
        mgr.register("discoverer", disc)
        assert mgr.unregister("discoverer", disc) is True
        assert mgr.get("discoverer") is None

    def test_unregister_returns_false_when_missing(self) -> None:
        mgr = ExtensionManager()
        disc = StubDiscoverer()
        assert mgr.unregister("discoverer", disc) is False

    def test_get_returns_none_when_empty(self) -> None:
        mgr = ExtensionManager()
        assert mgr.get("discoverer") is None


class TestMiddlewareExtension:
    def test_register_and_get_all(self) -> None:
        mgr = ExtensionManager()
        mw1 = StubMiddleware()
        mw2 = StubMiddleware()
        mgr.register("middleware", mw1)
        mgr.register("middleware", mw2)
        assert mgr.get_all("middleware") == [mw1, mw2]

    def test_unregister_specific(self) -> None:
        mgr = ExtensionManager()
        mw1 = StubMiddleware()
        mw2 = StubMiddleware()
        mgr.register("middleware", mw1)
        mgr.register("middleware", mw2)
        mgr.unregister("middleware", mw1)
        assert mgr.get_all("middleware") == [mw2]


class TestAclExtension:
    def test_register_and_get(self) -> None:
        mgr = ExtensionManager()
        acl = ACL(rules=[])
        mgr.register("acl", acl)
        assert mgr.get("acl") is acl


class TestSpanExporterExtension:
    def test_register_multiple(self) -> None:
        mgr = ExtensionManager()
        exp1 = StubExporter()
        exp2 = StubExporter()
        mgr.register("span_exporter", exp1)
        mgr.register("span_exporter", exp2)
        assert mgr.get_all("span_exporter") == [exp1, exp2]


class TestModuleValidatorExtension:
    def test_register_and_get(self) -> None:
        mgr = ExtensionManager()
        val = StubValidator()
        mgr.register("module_validator", val)
        assert mgr.get("module_validator") is val


# ---------------------------------------------------------------------------
# Tests: validation errors
# ---------------------------------------------------------------------------


class TestValidation:
    def test_unknown_point_raises_invalid_input(self) -> None:
        mgr = ExtensionManager()
        with pytest.raises(InvalidInputError, match="Unknown extension point") as exc_info:
            mgr.register("nonexistent", object())
        assert exc_info.value.code == "GENERAL_INVALID_INPUT"

    def test_wrong_type_raises_type_error(self) -> None:
        mgr = ExtensionManager()
        with pytest.raises(TypeError, match="must be an instance of"):
            mgr.register("middleware", "not_a_middleware")

    def test_get_unknown_point_raises_invalid_input(self) -> None:
        # D-108: the code is the assertion. A bare KeyError says "something
        # went wrong with a key" and is indistinguishable from a dict miss
        # anywhere else in the call; GENERAL_INVALID_INPUT names the mistake.
        mgr = ExtensionManager()
        with pytest.raises(InvalidInputError) as exc_info:
            mgr.get("nonexistent")
        assert exc_info.value.code == "GENERAL_INVALID_INPUT"

    def test_get_all_unknown_point_raises_invalid_input(self) -> None:
        mgr = ExtensionManager()
        with pytest.raises(InvalidInputError) as exc_info:
            mgr.get_all("nonexistent")
        assert exc_info.value.code == "GENERAL_INVALID_INPUT"

    def test_unregister_unknown_point_raises_invalid_input(self) -> None:
        mgr = ExtensionManager()
        with pytest.raises(InvalidInputError) as exc_info:
            mgr.unregister("nonexistent", object())
        assert exc_info.value.code == "GENERAL_INVALID_INPUT"

    def test_empty_registered_point_does_not_raise(self) -> None:
        # D-108's other half, and the control for the three above: if the
        # rejection were "raise for anything not holding an extension", these
        # would be red. A registered point holding nothing answers, quietly.
        mgr = ExtensionManager()
        assert mgr.get("acl") is None
        assert mgr.get_all("middleware") == []
        assert mgr.unregister("middleware", object()) is False

    def test_discoverer_rejects_wrong_type(self) -> None:
        mgr = ExtensionManager()
        with pytest.raises(TypeError):
            mgr.register("discoverer", StubMiddleware())

    def test_acl_rejects_wrong_type(self) -> None:
        mgr = ExtensionManager()
        with pytest.raises(TypeError):
            mgr.register("acl", StubMiddleware())


# ---------------------------------------------------------------------------
# Tests: apply()
# ---------------------------------------------------------------------------


class TestApply:
    def test_apply_discoverer_to_registry(self) -> None:
        mgr = ExtensionManager()
        disc = StubDiscoverer()
        mgr.register("discoverer", disc)

        registry = MagicMock()
        executor = MagicMock()
        executor.middlewares = []
        mgr.apply(registry, executor)

        registry.set_discoverer.assert_called_once_with(disc)

    def test_apply_validator_to_registry(self) -> None:
        mgr = ExtensionManager()
        val = StubValidator()
        mgr.register("module_validator", val)

        registry = MagicMock()
        executor = MagicMock()
        executor.middlewares = []
        mgr.apply(registry, executor)

        registry.set_validator.assert_called_once_with(val)

    def test_apply_acl_to_executor(self) -> None:
        mgr = ExtensionManager()
        acl = ACL(rules=[])
        mgr.register("acl", acl)

        registry = MagicMock()
        executor = MagicMock()
        executor.middlewares = []
        mgr.apply(registry, executor)

        executor.set_acl.assert_called_once_with(acl)

    def test_apply_middleware_to_executor(self) -> None:
        mgr = ExtensionManager()
        mw = StubMiddleware()
        mgr.register("middleware", mw)

        registry = MagicMock()
        executor = MagicMock()
        executor.middlewares = []
        mgr.apply(registry, executor)

        executor.use.assert_called_once_with(mw)

    def test_apply_single_span_exporter_to_tracing_middleware(self) -> None:
        mgr = ExtensionManager()
        new_exp = StubExporter()
        mgr.register("span_exporter", new_exp)

        in_mem = InMemoryExporter()
        tracing_mw = TracingMiddleware(exporter=in_mem)

        registry = MagicMock()
        executor = MagicMock()
        executor.middlewares = [tracing_mw]
        mgr.apply(registry, executor)

        # set_exporter is called with the single exporter directly
        assert tracing_mw._exporter is new_exp

    def test_apply_multiple_span_exporters_uses_composite(self) -> None:
        mgr = ExtensionManager()
        exp1 = StubExporter()
        exp2 = StubExporter()
        mgr.register("span_exporter", exp1)
        mgr.register("span_exporter", exp2)

        in_mem = InMemoryExporter()
        tracing_mw = TracingMiddleware(exporter=in_mem)

        registry = MagicMock()
        executor = MagicMock()
        executor.middlewares = [tracing_mw]
        mgr.apply(registry, executor)

        # A composite exporter should have been set that delegates to both
        composite = tracing_mw._exporter
        assert hasattr(composite, "_exporters")
        assert getattr(composite, "_exporters") == [exp1, exp2]

    def test_apply_with_no_extensions(self) -> None:
        """apply() should be safe with no extensions registered."""
        mgr = ExtensionManager()
        registry = MagicMock()
        executor = MagicMock()
        executor.middlewares = []
        mgr.apply(registry, executor)

        registry.set_discoverer.assert_not_called()
        registry.set_validator.assert_not_called()
        executor.use.assert_not_called()

    def test_apply_span_exporter_warns_without_tracing_middleware(self) -> None:
        mgr = ExtensionManager()
        new_exp = StubExporter()
        mgr.register("span_exporter", new_exp)

        registry = MagicMock()
        executor = MagicMock()
        executor.middlewares = []

        # Should not raise; just logs a warning
        mgr.apply(registry, executor)
