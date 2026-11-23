"""#123: a file the watcher sees for the first time gets the ID ``discover()`` would give it.

``_handle_file_change`` fell back to the file's bare basename whenever no
registered ID ended with it, so a file created at ``<root>/billing/refund.py``
after startup was registered as ``refund`` rather than ``billing.refund``.
"""

from __future__ import annotations

import textwrap
from pathlib import Path
from typing import Any

from pydantic import BaseModel

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
