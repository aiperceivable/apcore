"""Tests for ErrorCodeRegistry (Algorithm A17)."""

from __future__ import annotations

import threading

import pytest

from apcore.errors import ErrorCodeCollisionError, ErrorCodeRegistry, ErrorCodes


class TestErrorCodeRegistry:
    def test_register_custom_codes(self) -> None:
        reg = ErrorCodeRegistry()
        reg.register("my.module", {"MY_MODULE_CUSTOM_ERR", "MY_MODULE_OTHER"})
        assert "MY_MODULE_CUSTOM_ERR" in reg.all_codes
        assert "MY_MODULE_OTHER" in reg.all_codes

    def test_empty_codes_is_noop(self) -> None:
        reg = ErrorCodeRegistry()
        reg.register("my.module", set())
        # Only framework codes present
        assert ErrorCodes.MODULE_NOT_FOUND in reg.all_codes

    def test_collision_with_framework_code(self) -> None:
        reg = ErrorCodeRegistry()
        with pytest.raises(ErrorCodeCollisionError, match="framework"):
            reg.register("my.module", {ErrorCodes.MODULE_NOT_FOUND})

    def test_collision_with_framework_prefix(self) -> None:
        reg = ErrorCodeRegistry()
        with pytest.raises(ErrorCodeCollisionError, match="reserved prefix"):
            reg.register("my.module", {"MODULE_CUSTOM_THING"})

    def test_narrowed_prefix_allows_streaming_custom(self) -> None:
        """A-D-006: STREAMING_ is no longer a reserved prefix; a module may
        register a STREAMING_-prefixed custom code that is not an exact
        framework code."""
        reg = ErrorCodeRegistry()
        reg.register("my.module", {"STREAMING_CUSTOM"})
        assert "STREAMING_CUSTOM" in reg.all_codes

    def test_narrowed_prefix_allows_circuit_pipeline_context_custom(self) -> None:
        """A-D-006: CIRCUIT_, PIPELINE_, CONTEXT_ are no longer reserved
        prefixes for non-framework codes."""
        reg = ErrorCodeRegistry()
        reg.register("my.module", {"CIRCUIT_CUSTOM", "PIPELINE_CUSTOM", "CONTEXT_CUSTOM"})
        assert {"CIRCUIT_CUSTOM", "PIPELINE_CUSTOM", "CONTEXT_CUSTOM"} <= reg.all_codes

    def test_exact_streaming_framework_code_still_collides(self) -> None:
        """A-D-006: the specific framework code STREAMING_INTERFACE_MISMATCH
        is still protected by the exact-code check even after the prefix was
        dropped."""
        reg = ErrorCodeRegistry()
        with pytest.raises(ErrorCodeCollisionError, match="framework"):
            reg.register("my.module", {"STREAMING_INTERFACE_MISMATCH"})

    def test_exact_context_framework_code_still_collides(self) -> None:
        """A-D-006: CONTEXT_BINDING_ERROR remains protected as an exact
        framework code."""
        reg = ErrorCodeRegistry()
        with pytest.raises(ErrorCodeCollisionError, match="framework"):
            reg.register("my.module", {"CONTEXT_BINDING_ERROR"})

    def test_collision_between_modules(self) -> None:
        reg = ErrorCodeRegistry()
        reg.register("module.a", {"SHARED_CODE"})
        with pytest.raises(ErrorCodeCollisionError, match="module.a"):
            reg.register("module.b", {"SHARED_CODE"})

    def test_unregister_removes_codes(self) -> None:
        reg = ErrorCodeRegistry()
        reg.register("my.module", {"CUSTOM_X"})
        assert "CUSTOM_X" in reg.all_codes
        reg.unregister("my.module")
        assert "CUSTOM_X" not in reg.all_codes

    def test_unregister_allows_reuse(self) -> None:
        reg = ErrorCodeRegistry()
        reg.register("module.a", {"CUSTOM_X"})
        reg.unregister("module.a")
        # Now module.b can use the same code
        reg.register("module.b", {"CUSTOM_X"})
        assert "CUSTOM_X" in reg.all_codes

    def test_unregister_nonexistent_is_noop(self) -> None:
        reg = ErrorCodeRegistry()
        reg.unregister("nonexistent")  # Should not raise

    def test_framework_codes_always_present(self) -> None:
        reg = ErrorCodeRegistry()
        assert ErrorCodes.MODULE_TIMEOUT in reg.all_codes
        assert ErrorCodes.ACL_DENIED in reg.all_codes
        assert ErrorCodes.SCHEMA_VALIDATION_ERROR in reg.all_codes

    def test_all_codes_is_frozen(self) -> None:
        reg = ErrorCodeRegistry()
        codes = reg.all_codes
        assert isinstance(codes, frozenset)

    def test_thread_safety(self) -> None:
        """Concurrent registrations should not corrupt state."""
        reg = ErrorCodeRegistry()
        errors: list[Exception] = []

        def register_codes(module_id: str, codes: set[str]) -> None:
            try:
                reg.register(module_id, codes)
            except Exception as e:
                errors.append(e)

        threads = [threading.Thread(target=register_codes, args=(f"mod.{i}", {f"CODE_{i}"})) for i in range(20)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

        assert not errors
        for i in range(20):
            assert f"CODE_{i}" in reg.all_codes

    def test_multiple_codes_per_module(self) -> None:
        reg = ErrorCodeRegistry()
        reg.register("my.module", {"ERR_A", "ERR_B", "ERR_C"})
        assert {"ERR_A", "ERR_B", "ERR_C"} <= reg.all_codes

    def test_collision_error_details(self) -> None:
        reg = ErrorCodeRegistry()
        reg.register("module.a", {"SHARED"})
        with pytest.raises(ErrorCodeCollisionError) as exc_info:
            reg.register("module.b", {"SHARED"})
        err = exc_info.value
        assert err.details["error_code"] == "SHARED"
        assert err.details["module_id"] == "module.b"
        assert err.details["conflict_source"] == "module.a"


class TestFrameworkCodeInventoryIsComplete:
    """Every code the framework can emit must be guarded against user collision.

    Sync finding A-C-003. `_FRAMEWORK_CODES` is built from `vars(ErrorCodes)`,
    but eight codes are raised from `apcore.pipeline` and were never added to
    that map: PIPELINE_ABORT, PIPELINE_CONFIGURATION_ERROR,
    PIPELINE_DEPENDENCY_ERROR, STEP_NAME_DUPLICATE, STEP_NOT_FOUND,
    STEP_NOT_REMOVABLE, STEP_NOT_REPLACEABLE, STRATEGY_NOT_FOUND. None of them
    starts with one of the 14 reserved prefixes either, so neither half of the
    A17 guard covered them and `ErrorCodeRegistry.register()` accepted all
    eight from a user module.

    That mattered because the comment above FRAMEWORK_ERROR_CODE_PREFIXES
    narrows the reserved-prefix set on the stated grounds that the specific
    codes under CIRCUIT_/PIPELINE_/STREAMING_/CONTEXT_ "remain protected by the
    exact-code collision check against _FRAMEWORK_CODES". For these eight that
    compensating control was absent.

    This test derives the expectation from what the package actually exports,
    so a future error class raised from a new module cannot silently reopen the
    hole — which is how it opened in the first place.
    """

    @staticmethod
    def _emitted_codes() -> dict[str, list[str]]:
        import inspect

        import apcore
        from apcore.errors import ModuleError

        emitted: dict[str, list[str]] = {}
        for name, obj in vars(apcore).items():
            if not (inspect.isclass(obj) and issubclass(obj, ModuleError) and obj is not ModuleError):
                continue
            try:
                code = getattr(obj("probe"), "code", None)
            except Exception:
                try:
                    code = getattr(obj("CODE", "probe"), "code", None)
                except Exception:
                    code = None
            if isinstance(code, str) and code.isupper():
                emitted.setdefault(code, []).append(name)
        return emitted

    def test_every_emitted_code_is_a_known_framework_code(self) -> None:
        from apcore.errors import FRAMEWORK_ERROR_CODE_PREFIXES, _FRAMEWORK_CODES

        unguarded = {
            code: classes
            for code, classes in self._emitted_codes().items()
            if code not in _FRAMEWORK_CODES and not any(code.startswith(p) for p in FRAMEWORK_ERROR_CODE_PREFIXES)
        }
        assert not unguarded, (
            "these framework-emitted codes are protected by neither _FRAMEWORK_CODES "
            f"nor a reserved prefix, so a user module can claim them: {unguarded}"
        )

    def test_registry_rejects_every_emitted_code(self) -> None:
        from apcore.errors import ErrorCodeCollisionError, ErrorCodeRegistry

        registry = ErrorCodeRegistry()
        accepted: list[str] = []
        for index, code in enumerate(sorted(self._emitted_codes())):
            try:
                registry.register(f"user.module_{index}", {code})
            except ErrorCodeCollisionError:
                continue
            accepted.append(code)
        assert not accepted, f"user modules were allowed to claim framework codes: {accepted}"

    def test_the_eight_that_regressed_are_pinned(self) -> None:
        # Named explicitly so a change that drops them from ErrorCodes fails
        # here with the specific list, not just a generic inventory mismatch.
        from apcore.errors import _FRAMEWORK_CODES

        for code in (
            "PIPELINE_ABORT",
            "PIPELINE_CONFIGURATION_ERROR",
            "PIPELINE_DEPENDENCY_ERROR",
            "STEP_NAME_DUPLICATE",
            "STEP_NOT_FOUND",
            "STEP_NOT_REMOVABLE",
            "STEP_NOT_REPLACEABLE",
            "STRATEGY_NOT_FOUND",
        ):
            assert code in _FRAMEWORK_CODES, f"{code} must be a reserved framework code"
