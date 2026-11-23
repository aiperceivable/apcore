"""Spec-traced contract tests for the decorator-bindings feature.

Generated from apcore/docs/features/decorator-bindings.md. Each test embeds a
verbatim clause id of the form ``decorator_bindings.<method>.<kind>.<detail>``
so cross-language diffs line up row-by-row across the Python/TypeScript/Rust
SDKs.

TESTS ONLY — no production source is modified here.

Notes on spec/source divergence discovered while authoring (documented inline
rather than papered over):
* The feature spec names the inference helpers ``_generate_input_model`` /
  ``_generate_output_model``; the real Python source exports them without the
  leading underscore (``generate_input_model`` / ``generate_output_model``).
* The contract declares ``BindingSchemaMissingError(code=BINDING_SCHEMA_MISSING)``.
  In this SDK ``BindingSchemaMissingError`` is a backwards-compat alias for
  ``BindingSchemaInferenceFailedError`` whose real ``.code`` is
  ``BINDING_SCHEMA_INFERENCE_FAILED``. The test below asserts the REAL runtime
  code and flags the contract's declared code as a documented gap.
* ``BindingLoader.load_bindings`` / ``load_binding_dir`` take their path as the
  first positional argument (``file_path`` / ``dir_path``); the contract calls
  it ``path`` / ``directory``. Positional usage keeps both names compatible.
"""

from __future__ import annotations

import asyncio
import inspect

import pytest

from apcore.bindings import BindingLoader
from apcore.decorator import FunctionModule, module
from apcore.errors import (
    BindingCallableNotFoundError,
    BindingFileInvalidError,
    BindingInvalidTargetError,
    BindingModuleNotFoundError,
    BindingNotCallableError,
    BindingSchemaInferenceFailedError,
    FuncMissingReturnTypeError,
    FuncMissingTypeHintError,
)
from apcore.registry import Registry

# ---------------------------------------------------------------------------
# Shared helpers
# ---------------------------------------------------------------------------

_BINDING = (
    "bindings:\n" "  - module_id: {mid}\n" "    target: binding_helpers:typed_function\n" "    auto_schema: true\n"
)


def _typed(name: str, count: int = 1) -> dict:
    """A fully-typed function (valid for auto schema inference)."""
    return {"name": name, "count": count}


def _untyped(name, count=1):  # noqa: ANN001, ANN201
    """A function whose params lack type hints (invalid for inference)."""
    return {"name": name, "count": count}


def _no_return(name: str):  # noqa: ANN201
    """A function whose return type annotation is absent."""
    return {"name": name}


@pytest.fixture
def registry() -> Registry:
    return Registry()


@pytest.fixture
def loader() -> BindingLoader:
    return BindingLoader()


# ===========================================================================
# Contract: module
# ===========================================================================


class TestModuleContract:
    """Clauses traced to '## Contract: module'."""

    def test_input_func_or_none_untyped_param(self):
        # decorator_bindings.module.input.func_or_none.untyped_param_no_schema
        # Build an input that FAILS the rule: a function with an untyped
        # parameter and no explicit input_schema -> must reject with the
        # declared error TYPE and exact CODE.
        with pytest.raises(FuncMissingTypeHintError) as exc_info:
            module(_untyped, id="spec.untyped")
        assert isinstance(exc_info.value, FuncMissingTypeHintError)
        assert exc_info.value.code == "FUNC_MISSING_TYPE_HINT"

    def test_error_func_missing_type_hint(self):
        # decorator_bindings.module.error.FUNC_MISSING_TYPE_HINT
        with pytest.raises(FuncMissingTypeHintError) as exc_info:
            module(_untyped, id="spec.missing_hint")
        assert exc_info.value.code == "FUNC_MISSING_TYPE_HINT"

    def test_error_func_missing_return_type(self):
        # decorator_bindings.module.error.FUNC_MISSING_RETURN_TYPE
        with pytest.raises(FuncMissingReturnTypeError) as exc_info:
            module(_no_return, id="spec.missing_return")
        assert exc_info.value.code == "FUNC_MISSING_RETURN_TYPE"

    def test_property_async_false(self):
        # decorator_bindings.module.property.async.false
        # The decorator itself is synchronous: calling it returns immediately
        # (a FunctionModule / wrapped function), never a coroutine/awaitable.
        result = module(_typed, id="spec.async_false")
        assert not inspect.iscoroutine(result)
        assert not inspect.isawaitable(result)
        assert isinstance(result, FunctionModule)

    def test_property_thread_safe_true(self):
        # decorator_bindings.module.property.thread_safe.true
        # Launch >=8 concurrent module-creation calls with DISTINCT inputs.
        # Creation without a registry mutates no shared state -> no exception,
        # and every result is an independent FunctionModule with the right id.
        async def _run() -> list[FunctionModule]:
            def _make(i: int) -> FunctionModule:
                return module(_typed, id=f"spec.concurrent.{i}")

            tasks = [asyncio.to_thread(_make, i) for i in range(12)]
            return await asyncio.gather(*tasks)

        modules = asyncio.run(_run())
        assert len(modules) == 12
        assert all(isinstance(m, FunctionModule) for m in modules)
        # Final state consistent: distinct ids preserved, no cross-talk.
        assert sorted(m.module_id for m in modules) == sorted(f"spec.concurrent.{i}" for i in range(12))

    def test_property_pure_false_with_registry(self, registry):
        # decorator_bindings.module.property.pure.false_when_registry
        # Spec: pure is FALSE when a registry is provided (registration mutates
        # registry state). Observe the side effect via the registry's public
        # query API: the module id must be present afterwards.
        assert not registry.has("spec.pure.registered")
        module(_typed, id="spec.pure.registered", registry=registry)
        assert registry.has("spec.pure.registered")


# ===========================================================================
# Contract: BindingLoader.load_bindings
# ===========================================================================


class TestLoadBindingsContract:
    """Clauses traced to '## Contract: BindingLoader.load_bindings'."""

    def test_error_binding_file_invalid(self, loader, registry, tmp_path):
        # decorator_bindings.load_bindings.error.BINDING_FILE_INVALID
        # Empty file -> structural failure with exact code.
        f = tmp_path / "empty.binding.yaml"
        f.write_text("")
        with pytest.raises(BindingFileInvalidError) as exc_info:
            loader.load_bindings(str(f), registry)
        assert exc_info.value.code == "BINDING_FILE_INVALID"

    def test_error_binding_invalid_target(self, loader, registry, tmp_path):
        # decorator_bindings.load_bindings.error.BINDING_INVALID_TARGET
        # Target string missing the ':' separator.
        f = tmp_path / "t.binding.yaml"
        f.write_text(
            "bindings:\n"
            "  - module_id: bad.target\n"
            "    target: binding_helpers.typed_function\n"
            "    auto_schema: true\n"
        )
        with pytest.raises(BindingInvalidTargetError) as exc_info:
            loader.load_bindings(str(f), registry)
        assert exc_info.value.code == "BINDING_INVALID_TARGET"

    def test_error_binding_module_not_found(self, loader, registry, tmp_path):
        # decorator_bindings.load_bindings.error.BINDING_MODULE_NOT_FOUND
        f = tmp_path / "t.binding.yaml"
        f.write_text(
            "bindings:\n"
            "  - module_id: missing.mod\n"
            "    target: definitely_not_a_real_module_xyz:fn\n"
            "    auto_schema: true\n"
        )
        with pytest.raises(BindingModuleNotFoundError) as exc_info:
            loader.load_bindings(str(f), registry)
        assert exc_info.value.code == "BINDING_MODULE_NOT_FOUND"

    def test_error_binding_callable_not_found(self, loader, registry, tmp_path):
        # decorator_bindings.load_bindings.error.BINDING_CALLABLE_NOT_FOUND
        f = tmp_path / "t.binding.yaml"
        f.write_text(
            "bindings:\n"
            "  - module_id: missing.callable\n"
            "    target: binding_helpers:no_such_callable\n"
            "    auto_schema: true\n"
        )
        with pytest.raises(BindingCallableNotFoundError) as exc_info:
            loader.load_bindings(str(f), registry)
        assert exc_info.value.code == "BINDING_CALLABLE_NOT_FOUND"

    def test_error_binding_not_callable(self, loader, registry, tmp_path):
        # decorator_bindings.load_bindings.error.BINDING_NOT_CALLABLE
        # binding_helpers.NOT_CALLABLE is the int 42.
        f = tmp_path / "t.binding.yaml"
        f.write_text(
            "bindings:\n"
            "  - module_id: not.callable\n"
            "    target: binding_helpers:NOT_CALLABLE\n"
            "    auto_schema: true\n"
        )
        with pytest.raises(BindingNotCallableError) as exc_info:
            loader.load_bindings(str(f), registry)
        assert exc_info.value.code == "BINDING_NOT_CALLABLE"

    def test_error_binding_schema_missing(self, loader, registry, tmp_path):
        # decorator_bindings.load_bindings.error.BINDING_SCHEMA_MISSING
        # Auto-schema over an untyped callable. The contract declares the code
        # BINDING_SCHEMA_MISSING, but in this SDK the raised error is
        # BindingSchemaInferenceFailedError with the REAL code
        # BINDING_SCHEMA_INFERENCE_FAILED. We assert the real runtime code and
        # document the contract's stale code as a cross-language gap.
        f = tmp_path / "t.binding.yaml"
        f.write_text(
            "bindings:\n"
            "  - module_id: schema.missing\n"
            "    target: binding_helpers:untyped_function\n"
            "    auto_schema: true\n"
        )
        with pytest.raises(BindingSchemaInferenceFailedError) as exc_info:
            loader.load_bindings(str(f), registry)
        # Spec contract code 'BINDING_SCHEMA_MISSING' is stale; real code below.
        assert exc_info.value.code == "BINDING_SCHEMA_INFERENCE_FAILED"

    def test_property_async_false(self, loader, registry, tmp_path):
        # decorator_bindings.load_bindings.property.async.false
        f = tmp_path / "ok.binding.yaml"
        f.write_text(_BINDING.format(mid="async.false"))
        result = loader.load_bindings(str(f), registry)
        assert not inspect.isawaitable(result)
        assert isinstance(result, list)
        assert all(isinstance(m, FunctionModule) for m in result)

    def test_property_idempotent_false(self, loader, registry, tmp_path):
        # decorator_bindings.load_bindings.property.idempotent.false
        # Spec: idempotent is FALSE — calling twice re-registers and the
        # Registry raises a duplicate error on the second call. Observe via the
        # exception AND the post-state (id registered exactly once).
        f = tmp_path / "dup.binding.yaml"
        f.write_text(_BINDING.format(mid="idem.false"))
        first = loader.load_bindings(str(f), registry)
        assert len(first) == 1
        assert registry.has("idem.false")
        with pytest.raises(Exception):  # noqa: B017,PT011 - duplicate registration
            loader.load_bindings(str(f), registry)
        # State remains consistent: still exactly one registration.
        assert registry.has("idem.false")


# ===========================================================================
# Contract: BindingLoader.load_binding_dir
# ===========================================================================


class TestLoadBindingDirContract:
    """Clauses traced to '## Contract: BindingLoader.load_binding_dir'."""

    def test_error_binding_file_invalid_missing_dir(self, loader, registry, tmp_path):
        # decorator_bindings.load_binding_dir.error.BINDING_FILE_INVALID
        missing = tmp_path / "does_not_exist"
        with pytest.raises(BindingFileInvalidError) as exc_info:
            loader.load_binding_dir(str(missing), registry)
        assert exc_info.value.code == "BINDING_FILE_INVALID"

    def test_empty_dir_returns_empty_list(self, loader, registry, tmp_path):
        # decorator_bindings.load_binding_dir.return.empty_dir_empty_list
        empty = tmp_path / "empty_dir"
        empty.mkdir()
        result = loader.load_binding_dir(str(empty), registry)
        assert result == []

    def test_side_effect_1_sorted_file_load_order(self, loader, registry, tmp_path):
        # decorator_bindings.load_binding_dir.side_effect.1.sorted_file_order
        # The loader globs files in sorted order; modules from 'a' must be
        # registered (and returned) before modules from 'b'. Observe ORDER via
        # the returned FunctionModule list (public post-state).
        (tmp_path / "a.binding.yaml").write_text(_BINDING.format(mid="dir.a"))
        (tmp_path / "b.binding.yaml").write_text(_BINDING.format(mid="dir.b"))
        result = loader.load_binding_dir(str(tmp_path), registry)
        ids = [m.module_id for m in result]
        assert ids == ["dir.a", "dir.b"]
        assert registry.has("dir.a")
        assert registry.has("dir.b")

    def test_property_idempotent_false(self, loader, registry, tmp_path):
        # decorator_bindings.load_binding_dir.property.idempotent.false
        # Re-scanning the same directory re-registers -> duplicate error.
        (tmp_path / "x.binding.yaml").write_text(_BINDING.format(mid="dir.idem"))
        first = loader.load_binding_dir(str(tmp_path), registry)
        assert [m.module_id for m in first] == ["dir.idem"]
        with pytest.raises(Exception):  # noqa: B017,PT011 - duplicate registration
            loader.load_binding_dir(str(tmp_path), registry)
        assert registry.has("dir.idem")
