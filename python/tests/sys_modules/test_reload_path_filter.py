"""Issue #45 §4 — granular reload via ``path_filter``.

The reload module's bulk path matches existing ``module_ids``; the underlying
``Registry.discover()`` should also accept an optional ``path_filter`` so that
a hot-reload only re-walks files matching the glob.  Modules outside the
filter must remain registered with their original instance — i.e. they MUST
NOT be unregistered or replaced.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from apcore.registry.registry import Registry


_MODULE_TEMPLATE = """\
from pydantic import BaseModel

class _In(BaseModel):
    value: str

class _Out(BaseModel):
    result: str

class {class_name}:
    input_schema = _In
    output_schema = _Out
    description = "{description}"
    tags = ["filter-test"]

    def execute(self, inputs, context=None):
        return {{"result": inputs["value"]}}
"""


@pytest.fixture
def ext_dir(tmp_path: Path) -> Path:
    ext = tmp_path / "extensions"
    ext.mkdir()
    (ext / "alpha.py").write_text(_MODULE_TEMPLATE.format(class_name="AlphaModule", description="alpha"))
    (ext / "beta.py").write_text(_MODULE_TEMPLATE.format(class_name="BetaModule", description="beta"))
    (ext / "gamma.py").write_text(_MODULE_TEMPLATE.format(class_name="GammaModule", description="gamma"))
    return ext


def test_discover_with_path_filter_only_walks_matching_files(ext_dir: Path) -> None:
    """``discover(path_filter='*alpha*')`` should only register Alpha."""
    registry = Registry(extensions_dir=str(ext_dir))
    n = registry.discover(path_filter="*alpha*")
    assert n == 1
    ids = registry.module_ids
    assert any("alpha" in mid for mid in ids), f"Expected alpha module, got {ids}"
    assert not any("beta" in mid or "gamma" in mid for mid in ids), f"Filter leaked: {ids}"


def test_discover_no_filter_walks_everything(ext_dir: Path) -> None:
    """``discover()`` (no filter) preserves the original behavior — all 3 found."""
    registry = Registry(extensions_dir=str(ext_dir))
    n = registry.discover()
    assert n == 3


def test_discover_path_filter_preserves_existing_unmatched_modules(ext_dir: Path) -> None:
    """A second discover() with a filter must keep previously-registered
    modules that don't match the filter (Issue #45.4 — unaffected modules
    stay loaded).
    """
    registry = Registry(extensions_dir=str(ext_dir))
    registry.discover()
    before_ids = set(registry.module_ids)
    assert len(before_ids) == 3

    # Touch alpha file — simulate a hot-reload of just alpha.
    alpha_path = ext_dir / "alpha.py"
    alpha_path.write_text(_MODULE_TEMPLATE.format(class_name="AlphaModule", description="alpha v2"))
    registry.unregister(next(mid for mid in before_ids if "alpha" in mid))
    registry.discover(path_filter="*alpha*")

    after_ids = set(registry.module_ids)
    # All three IDs are still registered.
    assert after_ids == before_ids


def test_discover_path_filter_list_form(ext_dir: Path) -> None:
    """``path_filter`` accepts a list of patterns (union)."""
    registry = Registry(extensions_dir=str(ext_dir))
    n = registry.discover(path_filter=["*alpha*", "*beta*"])
    assert n == 2
    ids = registry.module_ids
    assert not any("gamma" in mid for mid in ids), f"Gamma should not be discovered: {ids}"
