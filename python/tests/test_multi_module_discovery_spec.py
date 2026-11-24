"""Spec-traced contract tests for the multi-module-discovery feature.

CANONICAL clause source for the apcore Python SDK. TypeScript and Rust suites
mirror the clause-IDs defined here verbatim. Each test carries a deterministic
clause-id in its docstring of the form:

    multi_module_discovery.<method>.<kind>.<detail>

where <kind> in {input, error, property, side_effect, return}.

Contract under test: ``Registry.discover_multi_class`` (and its underlying
free-function helper ``apcore.registry.multi_class.discover_multi_class``).
Feature spec: apcore/docs/features/multi-module-discovery.md

Framework: pytest + pytest-asyncio (asyncio_mode=auto). Tests-only — src/ is
never modified.
"""

from __future__ import annotations

import asyncio
import re
import textwrap
from pathlib import Path

import pytest

from apcore.errors import (
    IdTooLongError,
    InvalidSegmentError,
    ModuleIdConflictError,
    ModuleLoadError,
)
from apcore.registry import Registry
from apcore.registry.multi_class import (
    MAX_MODULE_ID_LEN,
    class_name_to_segment,
    discover_multi_class,
)

# ---------------------------------------------------------------------------
# Canonical ID grammar (PROTOCOL_SPEC §2.7), mirrored from the feature spec.
# ---------------------------------------------------------------------------
_CANONICAL_ID_RE = re.compile(r"^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$")


# ---------------------------------------------------------------------------
# Fixture-file helpers. discover_multi_class imports the file at scan time, so
# tests materialise real .py files under tmp_path/extensions/... to exercise
# both the file-system read and Algorithm-A01 base-ID derivation.
# ---------------------------------------------------------------------------
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


def _write_multi_class_file(file_path: Path, class_names: list[str]) -> Path:
    """Write a .py file with @multi_class-decorated Module stub classes."""
    body = "\n\n".join(_CLASS_TEMPLATE.format(name=n) for n in class_names)
    file_path.parent.mkdir(parents=True, exist_ok=True)
    file_path.write_text(_MODULE_STUB.format(body=body), encoding="utf-8")
    return file_path


def _ext_file(tmp_path: Path, *segments: str) -> Path:
    """Build a path under an ``extensions`` root so Algorithm A01 has context."""
    return tmp_path.joinpath("extensions", *segments)


# ===========================================================================
# RETURN / single-class identity guarantee
# ===========================================================================
def test_discover_multi_class_return_single_class_identity(tmp_path: Path) -> None:
    """multi_module_discovery.discover_multi_class.return.single_class_identity"""
    f = _write_multi_class_file(_ext_file(tmp_path, "math", "math_ops.py"), ["Addition"])
    result = discover_multi_class(f, extensions_root="extensions")
    assert len(result) == 1
    module_id, cls = result[0]
    # Single class -> bare base_id, no class segment appended.
    assert module_id == "math.math_ops"
    assert cls.__name__ == "Addition"


def test_discover_multi_class_return_two_class_distinct_ids(tmp_path: Path) -> None:
    """multi_module_discovery.discover_multi_class.return.two_class_distinct_ids"""
    f = _write_multi_class_file(_ext_file(tmp_path, "math", "math_ops.py"), ["Addition", "Subtraction"])
    result = discover_multi_class(f, extensions_root="extensions")
    ids = sorted(mid for mid, _ in result)
    assert ids == ["math.math_ops.addition", "math.math_ops.subtraction"]


def test_discover_multi_class_return_pairs_shape(tmp_path: Path) -> None:
    """multi_module_discovery.discover_multi_class.return.pairs_shape"""
    f = _write_multi_class_file(_ext_file(tmp_path, "math", "math_ops.py"), ["Addition", "Subtraction"])
    result = discover_multi_class(f, extensions_root="extensions")
    for entry in result:
        assert isinstance(entry, tuple) and len(entry) == 2
        module_id, cls = entry
        assert isinstance(module_id, str)
        assert isinstance(cls, type)


# ===========================================================================
# INPUT contracts
# ===========================================================================
def test_discover_multi_class_input_file_path_missing(tmp_path: Path) -> None:
    """multi_module_discovery.discover_multi_class.input.file_path.nonexistent"""
    # A file_path that does not exist on disk cannot be imported -> ModuleLoadError.
    missing = _ext_file(tmp_path, "math", "does_not_exist.py")
    with pytest.raises(ModuleLoadError) as exc:
        discover_multi_class(missing, extensions_root="extensions")
    assert exc.value.code == "MODULE_LOAD_ERROR"


def test_discover_multi_class_input_extensions_root_default(tmp_path: Path) -> None:
    """multi_module_discovery.discover_multi_class.input.extensions_root.default"""
    # Default extensions_root="extensions": directory context beneath it is kept.
    f = _write_multi_class_file(_ext_file(tmp_path, "math", "math_ops.py"), ["Addition", "Subtraction"])
    result = discover_multi_class(f)  # rely on default extensions_root
    ids = sorted(mid for mid, _ in result)
    assert ids == ["math.math_ops.addition", "math.math_ops.subtraction"]


def test_discover_multi_class_input_extensions_root_custom(tmp_path: Path) -> None:
    """multi_module_discovery.discover_multi_class.input.extensions_root.custom"""
    # Custom root name drives Algorithm A01; path beneath "plugins" is used.
    f = tmp_path.joinpath("plugins", "math", "math_ops.py")
    _write_multi_class_file(f, ["Addition", "Subtraction"])
    result = discover_multi_class(f, extensions_root="plugins")
    ids = sorted(mid for mid, _ in result)
    assert ids == ["math.math_ops.addition", "math.math_ops.subtraction"]


def test_discover_multi_class_input_pre_approval_hook_rejects(tmp_path: Path) -> None:
    """multi_module_discovery.discover_multi_class.input.pre_approval_hook.reject"""
    # Python-only safety hook: raising from the hook aborts the import.
    f = _write_multi_class_file(_ext_file(tmp_path, "math", "math_ops.py"), ["Addition"])

    def deny(path: Path) -> None:
        raise PermissionError(f"blocked {path}")

    with pytest.raises(ModuleLoadError) as exc:
        discover_multi_class(f, extensions_root="extensions", pre_approval_hook=deny)
    assert exc.value.code == "MODULE_LOAD_ERROR"


def test_discover_multi_class_input_pre_approval_hook_allows(tmp_path: Path) -> None:
    """multi_module_discovery.discover_multi_class.input.pre_approval_hook.allow"""
    f = _write_multi_class_file(_ext_file(tmp_path, "math", "math_ops.py"), ["Addition"])
    seen: list[Path] = []

    def allow(path: Path) -> None:
        seen.append(Path(path))

    result = discover_multi_class(f, extensions_root="extensions", pre_approval_hook=allow)
    assert len(result) == 1
    # Hook observed the file before import.
    assert len(seen) == 1


# ===========================================================================
# ERROR contracts (assert .code exactly)
# ===========================================================================
def test_discover_multi_class_error_module_id_conflict(tmp_path: Path) -> None:
    """multi_module_discovery.discover_multi_class.error.MODULE_ID_CONFLICT"""
    # MyModule and My_Module both produce segment "my_module".
    f = _write_multi_class_file(_ext_file(tmp_path, "pkg", "dup.py"), ["MyModule", "My_Module"])
    with pytest.raises(ModuleIdConflictError) as exc:
        discover_multi_class(f, extensions_root="extensions")
    assert exc.value.code == "MODULE_ID_CONFLICT"
    # Details carry file_path, class_names, conflicting_segment per spec.
    details = exc.value.details
    assert details["conflicting_segment"] == "my_module"
    assert set(details["class_names"]) == {"MyModule", "My_Module"}
    assert "file_path" in details


def test_discover_multi_class_error_invalid_segment(tmp_path: Path) -> None:
    """multi_module_discovery.discover_multi_class.error.INVALID_SEGMENT"""
    # A class whose snake_case segment starts with a digit violates the grammar.
    # Need >=2 classes to enter the multi-class validation path.
    f = _write_multi_class_file(_ext_file(tmp_path, "pkg", "bad.py"), ["Addition", "_3D"])
    with pytest.raises(InvalidSegmentError) as exc:
        discover_multi_class(f, extensions_root="extensions")
    assert exc.value.code == "INVALID_SEGMENT"


def test_discover_multi_class_error_id_too_long(tmp_path: Path) -> None:
    """multi_module_discovery.discover_multi_class.error.ID_TOO_LONG"""
    # Force the full module_id over MAX_MODULE_ID_LEN (192) via a very long
    # class name. Two classes are needed to enter the multi-class path.
    long_name = "A" + "b" * (MAX_MODULE_ID_LEN + 10)
    f = _write_multi_class_file(_ext_file(tmp_path, "pkg", "long.py"), [long_name, "Subtraction"])
    with pytest.raises(IdTooLongError) as exc:
        discover_multi_class(f, extensions_root="extensions")
    assert exc.value.code == "ID_TOO_LONG"


# ===========================================================================
# PROPERTY: snake_case derivation correctness (pure helper)
# ===========================================================================
@pytest.mark.parametrize(
    ("class_name", "expected"),
    [
        ("Addition", "addition"),
        ("MathOps", "math_ops"),
        ("HTTPSender", "http_sender"),
        ("MyModule_V2", "my_module_v2"),
    ],
)
def test_discover_multi_class_property_snake_case_conversion(class_name: str, expected: str) -> None:
    """multi_module_discovery.discover_multi_class.property.snake_case_conversion"""
    assert class_name_to_segment(class_name) == expected


def test_discover_multi_class_property_grammar_conformance(tmp_path: Path) -> None:
    """multi_module_discovery.discover_multi_class.property.grammar_conformance"""
    f = _write_multi_class_file(
        _ext_file(tmp_path, "math", "math_ops.py"),
        ["Addition", "Subtraction", "Multiplication"],
    )
    result = discover_multi_class(f, extensions_root="extensions")
    assert len(result) == 3
    for module_id, _ in result:
        assert _CANONICAL_ID_RE.match(module_id), module_id


def test_discover_multi_class_property_pure_false_reads_fs(tmp_path: Path) -> None:
    """multi_module_discovery.discover_multi_class.property.pure"""
    # pure: false — the result depends on file-system state. Deleting the file
    # changes behaviour (import fails), proving the function is not pure.
    f = _write_multi_class_file(_ext_file(tmp_path, "math", "math_ops.py"), ["Addition"])
    first = discover_multi_class(f, extensions_root="extensions")
    assert len(first) == 1
    f.unlink()
    with pytest.raises(ModuleLoadError):
        discover_multi_class(f, extensions_root="extensions")


def test_discover_multi_class_property_idempotent(tmp_path: Path) -> None:
    """multi_module_discovery.discover_multi_class.property.idempotent"""
    # idempotent: true — repeated calls with the same file yield the same IDs.
    f = _write_multi_class_file(_ext_file(tmp_path, "math", "math_ops.py"), ["Addition", "Subtraction"])
    ids1 = sorted(mid for mid, _ in discover_multi_class(f, extensions_root="extensions"))
    ids2 = sorted(mid for mid, _ in discover_multi_class(f, extensions_root="extensions"))
    ids3 = sorted(mid for mid, _ in discover_multi_class(f, extensions_root="extensions"))
    assert ids1 == ids2 == ids3 == ["math.math_ops.addition", "math.math_ops.subtraction"]


async def test_discover_multi_class_property_async_false(tmp_path: Path) -> None:
    """multi_module_discovery.discover_multi_class.property.async"""
    # async: false — the method is a plain (non-coroutine) callable.
    import inspect

    f = _write_multi_class_file(_ext_file(tmp_path, "math", "math_ops.py"), ["Addition"])
    registry = Registry()
    result = registry.discover_multi_class(f, extensions_root="extensions")
    assert not inspect.iscoroutine(result)
    assert not asyncio.iscoroutinefunction(Registry.discover_multi_class)
    assert len(result) == 1


async def test_discover_multi_class_property_thread_safe(tmp_path: Path) -> None:
    """multi_module_discovery.discover_multi_class.property.thread_safe"""
    # thread_safe: true — >=8 concurrent discoveries must all agree and not
    # corrupt one another (each scans an independent file).
    files = []
    for i in range(8):
        f = _write_multi_class_file(_ext_file(tmp_path, "math", f"ops_{i}.py"), ["Addition", "Subtraction"])
        files.append(f)

    async def scan(path: Path) -> list[str]:
        return sorted(mid for mid, _ in await asyncio.to_thread(discover_multi_class, path, "extensions"))

    results = await asyncio.gather(*(scan(f) for f in files))
    for i, ids in enumerate(results):
        assert ids == [f"math.ops_{i}.addition", f"math.ops_{i}.subtraction"]


# ===========================================================================
# SIDE EFFECTS — observable via public API
# ===========================================================================
def test_discover_multi_class_side_effect_1_no_registration(tmp_path: Path) -> None:
    """multi_module_discovery.discover_multi_class.side_effect.1.discovery_does_not_register"""
    # discover_multi_class returns candidate pairs but MUST NOT itself register
    # modules into the registry (registration is a separate caller step).
    f = _write_multi_class_file(_ext_file(tmp_path, "math", "math_ops.py"), ["Addition", "Subtraction"])
    registry = Registry()
    before = set(registry.list_module_ids()) if hasattr(registry, "list_module_ids") else set(registry._modules.keys())  # type: ignore[attr-defined]
    registry.discover_multi_class(f, extensions_root="extensions")
    after = set(registry.list_module_ids()) if hasattr(registry, "list_module_ids") else set(registry._modules.keys())  # type: ignore[attr-defined]
    assert after == before


def test_discover_multi_class_side_effect_2_conflict_no_partial(tmp_path: Path) -> None:
    """multi_module_discovery.discover_multi_class.side_effect.2.conflict_aborts_whole_file"""
    # On conflict the whole file is aborted: no pairs are returned (the
    # exception propagates before any results escape).
    f = _write_multi_class_file(_ext_file(tmp_path, "pkg", "dup.py"), ["Addition", "MyModule", "My_Module"])
    captured: list = []
    with pytest.raises(ModuleIdConflictError):
        captured.extend(discover_multi_class(f, extensions_root="extensions"))
    # Nothing leaked out before the abort.
    assert captured == []
