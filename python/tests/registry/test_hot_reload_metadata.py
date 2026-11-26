"""Hot reload must restore ``_module_meta``, not just the instance maps.

``Registry._handle_file_change`` removes the module from every internal store
(Phase 2) and re-inserts the freshly imported instance (Phase 4). Phase 4 used
to write ``_modules`` / ``_lowercase_map`` / ``_versioned_modules`` /
``_versioned_meta`` but not ``_module_meta``, so a module that reported a
description, version, tags and dependencies a moment earlier came back from
``get_definition()`` with ``description=""`` / ``version="1.0.0"`` / ``tags=[]``
and from ``get_module_metadata()`` as ``{}``.

The dependency loss is the one with teeth: ``ReloadModule._topo_sort_modules``
reads ``get_module_metadata()["dependencies"]`` to order a ``path_filter``
reload, so the degraded descriptor silently turned a topological reload order
into an alphabetical one.
"""

from __future__ import annotations

import textwrap
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from time import monotonic, sleep
from typing import Any

import pytest

from apcore.registry.registry import Registry
from apcore.registry.types import DependencyInfo

_MODULE_SOURCE = textwrap.dedent(
    '''
    """Probe module used by the hot-reload metadata regression test."""

    from typing import Any

    from pydantic import BaseModel


    class _Input(BaseModel):
        value: int = 0


    class _Output(BaseModel):
        value: int = 0


    class ReloadProbeModule:
        input_schema = _Input
        output_schema = _Output
        description = "{description}"
        version = "{version}"
        tags = {tags!r}
        documentation = "{documentation}"
        dependencies = [{{"module_id": "common.util", "version": ">=1.0.0"}}]
        metadata = {{"probe": "{version}"}}

        def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
            return {{"value": inputs.get("value", 0)}}
    '''
)


def _write_probe(path: Path, *, description: str, version: str, tags: list[str], documentation: str) -> None:
    path.write_text(
        _MODULE_SOURCE.format(
            description=description,
            version=version,
            tags=tags,
            documentation=documentation,
        ),
        encoding="utf-8",
    )


class TestHotReloadPreservesMetadata:
    def test_watcher_loads_companion_metadata_for_the_new_revision(self, tmp_path: Path) -> None:
        path = tmp_path / "probe.py"
        _write_probe(path, description="code before", version="1.0.0", tags=["code"], documentation="probe")
        companion = tmp_path / "probe_meta.yaml"
        companion.write_text("description: companion before\nversion: 1.1.0\ntags: [registered]\n", encoding="utf-8")
        registry = Registry(extensions_dir=str(tmp_path))
        assert registry.discover() == 1
        original = registry.get("probe")

        _write_probe(path, description="code replacement", version="2.0.0", tags=["code"], documentation="probe")
        companion.write_text("description: companion after\nversion: 2.1.0\ntags: [registered]\n", encoding="utf-8")
        registry._handle_file_change(str(path))

        descriptor = registry.get_definition("probe")
        assert descriptor is not None
        assert descriptor.description == "companion after"
        assert descriptor.version == "2.1.0"
        assert descriptor.tags == ["registered"]
        replacement = registry.get("probe", version_hint="2.1.0")
        assert replacement is not None and replacement is not original

    @pytest.mark.parametrize("operation", ["change", "delete"])
    def test_watcher_drains_active_lease_before_unloading(self, tmp_path: Path, operation: str) -> None:
        path = tmp_path / "probe.py"
        _write_probe(path, description="code before", version="1.0.0", tags=[], documentation="probe")
        registry = Registry(extensions_dir=str(tmp_path))
        assert registry.discover() == 1
        old_hooks: list[str] = []
        with ThreadPoolExecutor(max_workers=1) as pool:
            with registry.acquire("probe") as old:
                old.on_suspend = lambda: old_hooks.append("suspend")
                old.on_unload = lambda: old_hooks.append("unload")
                if operation == "change":
                    _write_probe(path, description="replacement", version="2.0.0", tags=[], documentation="probe")
                    watched = pool.submit(registry._handle_file_change, str(path))
                else:
                    path.unlink()
                    watched = pool.submit(registry._handle_file_deletion, str(path))
                deadline = monotonic() + 1
                while not registry.is_draining("probe") and not watched.done() and monotonic() < deadline:
                    sleep(0.001)
                assert registry.is_draining("probe")
                assert old_hooks == []
            watched.result(timeout=1)
        assert old_hooks == (["suspend", "unload"] if operation == "change" else ["unload"])
        assert registry.has("probe") is (operation == "change")

    def test_watcher_preserves_previous_registration_when_custom_validator_rejects(self, tmp_path: Path) -> None:
        class Validator:
            def validate(self, module: Any) -> list[str]:
                return ["description rejected"] if module.description == "invalid" else []

        path = tmp_path / "probe.py"
        _write_probe(path, description="original", version="1.0.0", tags=[], documentation="probe")
        reg = Registry(extensions_dir=str(tmp_path))
        assert reg.discover() == 1
        original = reg.get("probe")
        reg.set_validator(Validator())

        _write_probe(path, description="invalid", version="2.0.0", tags=[], documentation="probe")
        reg._handle_file_change(str(path))

        assert reg.get("probe") is original

    def test_reloaded_module_keeps_its_descriptor_and_dependencies(self, tmp_path: Path) -> None:
        probe_path = tmp_path / "reload_probe.py"
        _write_probe(
            probe_path,
            description="probe v1",
            version="1.5.0",
            tags=["alpha"],
            documentation="v1 docs",
        )

        registry = Registry(extensions_dirs=[{"root": str(tmp_path), "namespace": "executor"}])
        assert registry.discover() == 1

        before = registry.get_definition("executor.reload_probe")
        assert before is not None
        assert before.description == "probe v1"
        assert before.dependencies == [DependencyInfo(module_id="common.util", version=">=1.0.0", optional=False)]

        # Touch the file with a new revision and let the watcher callback run.
        _write_probe(
            probe_path,
            description="probe v2",
            version="2.0.0",
            tags=["alpha", "beta"],
            documentation="v2 docs",
        )
        registry._handle_file_change(str(probe_path))

        after = registry.get_definition("executor.reload_probe")
        assert after is not None
        # Every one of these is a value the empty-meta fallback would have
        # replaced with "", "1.0.0", [] or None.
        assert after.description == "probe v2"
        assert after.version == "2.0.0"
        assert after.tags == ["alpha", "beta"]
        assert after.documentation == "v2 docs"
        assert after.dependencies == [DependencyInfo(module_id="common.util", version=">=1.0.0", optional=False)]

        meta = registry.get_module_metadata("executor.reload_probe")
        assert meta != {}
        # `_topo_sort_modules` reads exactly this key to order a path_filter reload.
        assert meta["dependencies"] == [{"module_id": "common.util", "version": ">=1.0.0"}]
        assert meta["description"] == "probe v2"
        assert meta["version"] == "2.0.0"

    def test_reloaded_module_metadata_replaces_the_previous_revision(self, tmp_path: Path) -> None:
        """The stored metadata describes the NEW instance, not the old one."""
        probe_path = tmp_path / "reload_probe.py"
        _write_probe(
            probe_path,
            description="probe v1",
            version="1.5.0",
            tags=["alpha"],
            documentation="v1 docs",
        )

        registry = Registry(extensions_dirs=[{"root": str(tmp_path), "namespace": "executor"}])
        assert registry.discover() == 1

        _write_probe(
            probe_path,
            description="probe v2",
            version="2.0.0",
            tags=["beta"],
            documentation="v2 docs",
        )
        registry._handle_file_change(str(probe_path))

        meta: dict[str, Any] = registry.get_module_metadata("executor.reload_probe")
        assert meta["metadata"] == {"probe": "2.0.0"}
        assert meta["tags"] == ["beta"]
