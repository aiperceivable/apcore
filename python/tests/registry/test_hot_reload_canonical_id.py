"""#123: a file the watcher sees for the first time gets the ID ``discover()`` would give it.

``_handle_file_change`` fell back to the file's bare basename whenever no
registered ID ended with it, so a file created at ``<root>/billing/refund.py``
after startup was registered as ``refund`` rather than ``billing.refund``.
"""

from __future__ import annotations

import textwrap
from pathlib import Path
from typing import Any

import pytest
from pydantic import BaseModel

from apcore.config import Config
from apcore.registry.registry import Registry

_SOURCE = textwrap.dedent(
    """
    from typing import Any

    from pydantic import BaseModel


    class _Input(BaseModel):
        value: int = 0


    class _Output(BaseModel):
        value: int = 0


    class RefundModule:
        input_schema = _Input
        output_schema = _Output
        description = "hot reload canonical id probe"

        def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
            return {"value": inputs.get("value", 0)}
    """
)


def _write(path: Path) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(_SOURCE, encoding="utf-8")
    return path


class TestHotReloadCanonicalId:
    @pytest.mark.parametrize("relative_path", ["_excluded/probe.py", "system/probe.py"])
    def test_ineligible_source_is_not_passed_to_the_loader(self, tmp_path: Path, relative_path: str) -> None:
        source = _write(tmp_path / relative_path)
        seen: list[Path] = []
        registry = Registry(extensions_dir=str(tmp_path), pre_approval_hook=seen.append)

        registry._handle_file_change(str(source))

        assert seen == []
        assert registry.module_ids == []

    def test_ignored_source_is_not_passed_to_the_loader(self, tmp_path: Path) -> None:
        source = _write(tmp_path / "ignored.py")
        seen: list[Path] = []
        config = Config({"extensions": {"ignore_patterns": ["ignored.py"]}})
        registry = Registry(config, extensions_dir=str(tmp_path), pre_approval_hook=seen.append)

        registry._handle_file_change(str(source))

        assert seen == []
        assert registry.module_ids == []

    @pytest.mark.parametrize("follow_symlinks", [False, True])
    def test_outside_symlink_is_not_passed_to_the_loader(self, tmp_path: Path, follow_symlinks: bool) -> None:
        root = tmp_path / "extensions"
        root.mkdir()
        alias = root / "probe.py"
        alias.symlink_to(_write(tmp_path / "probe.py"))
        seen: list[Path] = []
        config = Config({"extensions": {"follow_symlinks": follow_symlinks}})
        registry = Registry(config, extensions_dir=str(root), pre_approval_hook=seen.append)

        registry._handle_file_change(str(alias))

        assert seen == []
        assert registry.module_ids == []

    def test_watcher_does_not_remove_programmatically_registered_module(self, tmp_path: Path) -> None:
        registry = Registry(extensions_dir=str(tmp_path))
        registered = _Other()
        registry.register("billing.refund", registered)

        registry._handle_file_deletion(str(tmp_path / "billing" / "refund.py"))

        assert registry.get("billing.refund") is registered

    def test_new_file_registers_under_path_derived_id(self, tmp_path: Path) -> None:
        root = tmp_path / "extensions"
        probe = _write(root / "billing" / "refund.py")
        registry = Registry(extensions_dir=str(root))

        registry._handle_file_change(str(probe))

        assert registry.has("billing.refund")
        assert not registry.has("refund")

    def test_new_file_under_namespaced_root_gets_namespace_prefix(self, tmp_path: Path) -> None:
        root = tmp_path / "ext_a"
        probe = _write(root / "billing" / "refund.py")
        registry = Registry(extensions_dirs=[{"root": str(root), "namespace": "pay"}])

        registry._handle_file_change(str(probe))

        assert registry.has("pay.billing.refund")

    def test_matches_the_id_discover_assigns(self, tmp_path: Path) -> None:
        root = tmp_path / "extensions"
        probe = _write(root / "billing" / "refund.py")
        discovered = Registry(extensions_dir=str(root))
        discovered.discover()

        watched = Registry(extensions_dir=str(root))
        watched._handle_file_change(str(probe))

        assert watched.module_ids == discovered.module_ids

    def test_deleting_the_file_unregisters_the_canonical_id(self, tmp_path: Path) -> None:
        root = tmp_path / "extensions"
        probe = _write(root / "billing" / "refund.py")
        registry = Registry(extensions_dir=str(root))
        registry.discover()
        registry.register("archive.refund", _Other())
        assert registry.has("billing.refund")

        probe.unlink()
        registry._handle_file_deletion(str(probe))

        assert not registry.has("billing.refund")
        assert registry.has("archive.refund")


class _IO(BaseModel):
    value: int = 0


class _Other:
    """Unrelated module whose ID ends with the same basename."""

    input_schema = _IO
    output_schema = _IO
    description = "unrelated module sharing the basename"

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        return {"value": 0}
