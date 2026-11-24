"""Smoke tests verifying that integration fixtures are created without errors."""

from __future__ import annotations

from pathlib import Path

from apcore.context import Context
from apcore.executor import Executor
from apcore.registry import Registry


class TestIntegrationFixtures:
    """Verify all conftest fixtures create valid objects."""

    def test_extensions_dir_has_module_files(self, int_extensions_dir):
        ext_dir = Path(int_extensions_dir)
        assert (ext_dir / "greet.py").exists()
        assert (ext_dir / "failing.py").exists()
        assert (ext_dir / "async_greet.py").exists()

    def test_registry_discovers_modules(self, int_registry):
        assert isinstance(int_registry, Registry)
        assert int_registry.get("greet") is not None
        assert int_registry.get("failing") is not None
        assert int_registry.get("async_greet") is not None

    def test_executor_created(self, int_executor):
        assert isinstance(int_executor, Executor)

    def test_acl_executor_created(self, int_acl_executor):
        assert isinstance(int_acl_executor, Executor)
        assert int_acl_executor._acl is not None

    def test_context_created(self, int_context):
        assert isinstance(int_context, Context)
        assert int_context.trace_id is not None
