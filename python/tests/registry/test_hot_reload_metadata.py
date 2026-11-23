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
from pathlib import Path
from typing import Any

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


def _load_probe_class(path: Path) -> type:
    from apcore.registry.entry_point import resolve_entry_point

    return resolve_entry_point(path)


class TestHotReloadPreservesMetadata:
    def test_reloaded_module_keeps_its_descriptor_and_dependencies(self, tmp_path: Path) -> None:
        probe_path = tmp_path / "reload_probe.py"
        _write_probe(
            probe_path,
            description="probe v1",
            version="1.5.0",
            tags=["alpha"],
            documentation="v1 docs",
        )

        registry = Registry()
        registry.register("executor.reload_probe", _load_probe_class(probe_path)())

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

        registry = Registry()
        registry.register("executor.reload_probe", _load_probe_class(probe_path)())

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
