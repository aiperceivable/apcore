# Spec-traceable contract tests — each test maps to a clause in apcore/docs/features/
"""
Spec-traceable contract tests for apcore-python.

Each test function maps to a formal clause ID in the apcore protocol spec.
Clause IDs follow the pattern: <component>.<operation>.<category>.<CODE>

Test naming convention: test_{clause_id_snake}_{brief_description}
"""

from __future__ import annotations

from typing import Any

import pytest
from pydantic import BaseModel

from apcore.acl import ACL, ACLRule
from apcore.cancel import CancelToken, ExecutionCancelledError
from apcore.config import Config, _GLOBAL_NS_REGISTRY, _GLOBAL_NS_REGISTRY_LOCK
from apcore.errors import (
    CallDepthExceededError,
    CallFrequencyExceededError,
    CircularCallError,
    ConfigNamespaceDuplicateError,
    ConfigNamespaceReservedError,
    ErrorCodes,
    InvalidInputError,
    ModuleError,
    ModuleNotFoundError,
    SchemaValidationError,
)
from apcore.executor import Executor
from apcore.registry import Registry
from apcore.schema.validator import SchemaValidator
from apcore.utils.call_chain import guard_call_chain


# ---------------------------------------------------------------------------
# Minimal module stubs used throughout the test suite
# ---------------------------------------------------------------------------


class _MinimalModule:
    """No schema — accepts any input, returns a fixed dict."""

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        del inputs, context
        return {"ok": True}


class _FailingOnLoadModule:
    """Module whose on_load() always raises to exercise rollback paths."""

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        del inputs, context
        return {}

    def on_load(self) -> None:
        raise RuntimeError("deliberate on_load failure")


class _SimpleInput(BaseModel):
    name: str


class _SimpleOutput(BaseModel):
    greeting: str


class _TypedModule:
    input_schema = _SimpleInput
    output_schema = _SimpleOutput

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        del context
        return {"greeting": f"Hello, {inputs['name']}!"}


# ---------------------------------------------------------------------------
# Helper: isolate Config namespace registry mutations
# ---------------------------------------------------------------------------


def _cleanup_namespace(name: str) -> None:
    """Remove a namespace from the global registry — used in test teardowns."""
    with _GLOBAL_NS_REGISTRY_LOCK:
        _GLOBAL_NS_REGISTRY.pop(name, None)


# ===========================================================================
# Registry.register — error contracts
# ===========================================================================


class TestRegistryRegisterErrors:
    """Contract tests for Registry.register error cases."""

    def test_registry_register_error_invalid_module_id_empty(self) -> None:
        """Tests registry.register.error.INVALID_MODULE_ID

        An empty string must raise InvalidInputError with code INVALID_MODULE_ID.
        """
        registry = Registry()
        with pytest.raises(InvalidInputError) as exc_info:
            registry.register("", _MinimalModule())
        assert exc_info.value.code == ErrorCodes.INVALID_MODULE_ID

    def test_registry_register_error_invalid_module_id_uppercase(self) -> None:
        """Tests registry.register.error.INVALID_MODULE_ID

        A module_id containing uppercase letters violates the EBNF pattern and
        must raise InvalidInputError with code INVALID_MODULE_ID.
        """
        registry = Registry()
        with pytest.raises(InvalidInputError) as exc_info:
            registry.register("Bad.Module", _MinimalModule())
        assert exc_info.value.code == ErrorCodes.INVALID_MODULE_ID

    def test_registry_register_error_invalid_module_id_hyphen(self) -> None:
        """Tests registry.register.error.INVALID_MODULE_ID

        Hyphens are not allowed in module IDs per PROTOCOL_SPEC §2.7.
        Must raise InvalidInputError with code INVALID_MODULE_ID.
        """
        registry = Registry()
        with pytest.raises(InvalidInputError) as exc_info:
            registry.register("bad-module", _MinimalModule())
        assert exc_info.value.code == ErrorCodes.INVALID_MODULE_ID

    def test_registry_register_error_duplicate_module_id_raises(self) -> None:
        """Tests registry.register.error.DUPLICATE_MODULE_ID

        Registering the same module_id twice (without a version qualifier) must
        raise InvalidInputError with code DUPLICATE_MODULE_ID.
        """
        registry = Registry()
        registry.register("test.duplicate", _MinimalModule())
        with pytest.raises(InvalidInputError) as exc_info:
            registry.register("test.duplicate", _MinimalModule())
        assert exc_info.value.code == ErrorCodes.DUPLICATE_MODULE_ID

    def test_registry_register_property_idempotent_false(self) -> None:
        """Tests registry.register.property.idempotent

        Registry.register() is NOT idempotent — a second call with the same
        module_id must raise rather than silently succeed (no-op).
        Confirms the contract that callers must not rely on repeat-call safety.
        """
        registry = Registry()
        registry.register("test.not_idempotent", _MinimalModule())
        # The second call must raise; if it silently returns the test fails.
        raised = False
        try:
            registry.register("test.not_idempotent", _MinimalModule())
        except InvalidInputError:
            raised = True
        assert raised, "duplicate register() must raise, not silently succeed"


# ===========================================================================
# Registry.register — side-effect contracts (on_load rollback)
# ===========================================================================


class TestRegistryRegisterSideEffects:
    """Contract tests for side-effect ordering in Registry.register."""

    def test_registry_register_side_effect_3_check_duplicate_on_load_failure_rollback(
        self,
    ) -> None:
        """Tests registry.register.side_effect.3.check_duplicate

        When on_load() raises, the registry must roll back the registration:
        the module must not be queryable and the internal lowercase map must
        not retain the entry (mirrors Algorithm A21 rollback contract).
        """
        registry = Registry()
        with pytest.raises(RuntimeError):
            registry.register("test.rollback_onload", _FailingOnLoadModule())
        # After rollback: module is not registered
        assert registry.get("test.rollback_onload") is None
        assert not registry.has("test.rollback_onload")
        # Lowercase map must not retain a stale entry
        assert "test.rollback_onload" not in registry._lowercase_map

    def test_registry_register_side_effect_rollback_prevents_duplicate_check_interference(
        self,
    ) -> None:
        """Tests registry.register.side_effect.3.check_duplicate

        After a rolled-back registration, the same module_id can be
        re-registered successfully. If rollback were incomplete, the
        subsequent register() would incorrectly raise DUPLICATE.
        """
        registry = Registry()
        # First attempt fails on_load — must roll back cleanly
        with pytest.raises(RuntimeError):
            registry.register("test.retry_after_rollback", _FailingOnLoadModule())
        # Second attempt with a different (working) module on same ID must succeed
        registry.register("test.retry_after_rollback", _MinimalModule())
        assert registry.has("test.retry_after_rollback")


# ===========================================================================
# guard_call_chain — error contracts
# ===========================================================================


class TestGuardCallChainErrors:
    """Contract tests for guard_call_chain error codes."""

    def test_guard_call_chain_error_call_depth_exceeded(self) -> None:
        """Tests guard_call_chain.error.CALL_DEPTH_EXCEEDED

        When the call chain length exceeds max_call_depth, guard_call_chain must
        raise CallDepthExceededError with code CALL_DEPTH_EXCEEDED.
        """
        max_depth = 3
        # Build a chain of length max_depth + 1 so the depth check fires
        chain = ["a.mod", "b.mod", "c.mod", "d.mod"]
        with pytest.raises(CallDepthExceededError) as exc_info:
            guard_call_chain("d.mod", chain, max_call_depth=max_depth)
        assert exc_info.value.code == ErrorCodes.CALL_DEPTH_EXCEEDED
        # Verify the depth detail is accurate
        assert exc_info.value.current_depth == len(chain)

    def test_guard_call_chain_error_circular_call(self) -> None:
        """Tests guard_call_chain.error.CIRCULAR_CALL

        When module_id already appears earlier in the call chain with
        intermediate steps (A→B→A pattern), guard_call_chain must raise
        CircularCallError with code CIRCULAR_CALL.
        """
        # chain: root -> a.mod -> b.mod -> a.mod  (A→B→A cycle)
        chain = ["root.mod", "a.mod", "b.mod", "a.mod"]
        with pytest.raises(CircularCallError) as exc_info:
            guard_call_chain("a.mod", chain, max_call_depth=32)
        assert exc_info.value.code == ErrorCodes.CIRCULAR_CALL
        assert exc_info.value.module_id == "a.mod"

    def test_guard_call_chain_error_frequency_exceeded(self) -> None:
        """Tests guard_call_chain.error.FREQUENCY_EXCEEDED

        When a module appears more than max_module_repeat times, guard_call_chain
        must raise CallFrequencyExceededError with code CALL_FREQUENCY_EXCEEDED.
        """
        max_repeat = 2
        # chain contains "repeat.mod" three times (> max_repeat=2)
        chain = ["root.mod", "repeat.mod", "other.mod", "repeat.mod", "repeat.mod"]
        with pytest.raises(CallFrequencyExceededError) as exc_info:
            guard_call_chain("repeat.mod", chain, max_call_depth=32, max_module_repeat=max_repeat)
        assert exc_info.value.code == ErrorCodes.CALL_FREQUENCY_EXCEEDED
        assert exc_info.value.module_id == "repeat.mod"
        assert exc_info.value.count == chain.count("repeat.mod")


class TestGuardCallChainProperties:
    """Property contract tests for guard_call_chain."""

    def test_guard_call_chain_property_pure_does_not_mutate_chain(self) -> None:
        """Tests guard_call_chain.property.pure

        guard_call_chain() must not mutate the caller's call_chain list.
        The function is a pure read-only guard: its only side effects are
        the errors it may raise. The original list must be identical after
        the call.
        """
        chain: list[str] = ["root.mod", "a.mod", "b.mod"]
        original_chain = list(chain)
        # This call should not raise (within depth/repeat limits)
        guard_call_chain("c.mod", chain + ["c.mod"], max_call_depth=32, max_module_repeat=3)
        # chain passed by the caller is unchanged
        assert chain == original_chain

    def test_guard_call_chain_property_pure_does_not_mutate_on_error(self) -> None:
        """Tests guard_call_chain.property.pure

        guard_call_chain() must not mutate the caller's chain even when it
        raises. The pure contract holds regardless of success or failure path.
        """
        chain: list[str] = ["root.mod", "a.mod", "b.mod", "c.mod"]
        original_chain = list(chain)
        with pytest.raises(CallDepthExceededError):
            guard_call_chain("c.mod", chain, max_call_depth=2)
        assert chain == original_chain


# ===========================================================================
# SchemaValidator.validate — error and property contracts
# ===========================================================================


class TestSchemaValidateErrors:
    """Contract tests for SchemaValidator.validate error codes."""

    def test_schema_validate_error_schema_validation_failed_raises_correct_code(
        self,
    ) -> None:
        """Tests schema.validate.error.SCHEMA_VALIDATION_FAILED

        When validate_input() receives data that fails Pydantic validation,
        SchemaValidationError must be raised with code SCHEMA_VALIDATION_ERROR.
        """
        validator = SchemaValidator()
        with pytest.raises(SchemaValidationError) as exc_info:
            validator.validate_input({"name": 123}, _SimpleInput)
        # SchemaValidationError always carries SCHEMA_VALIDATION_ERROR
        assert exc_info.value.code == ErrorCodes.SCHEMA_VALIDATION_ERROR

    def test_schema_validate_error_missing_required_field(self) -> None:
        """Tests schema.validate.error.SCHEMA_VALIDATION_FAILED

        Missing a required field must raise SchemaValidationError with
        code SCHEMA_VALIDATION_ERROR and non-empty error details.
        """
        validator = SchemaValidator()
        with pytest.raises(SchemaValidationError) as exc_info:
            validator.validate_input({}, _SimpleInput)
        err = exc_info.value
        assert err.code == ErrorCodes.SCHEMA_VALIDATION_ERROR
        # Error details must be non-empty (the validator reports which fields failed)
        assert len(err.details.get("errors", [])) > 0


class TestSchemaValidateProperties:
    """Property contract tests for SchemaValidator.validate."""

    def test_schema_validate_property_idempotent_valid_data(self) -> None:
        """Tests schema.validate.property.idempotent

        Calling validate() twice with identical valid data+schema must produce
        identical results (both valid=True with no errors).
        """
        validator = SchemaValidator()
        data = {"name": "Alice"}
        result_1 = validator.validate(data, _SimpleInput)
        result_2 = validator.validate(data, _SimpleInput)
        assert result_1.valid is True
        assert result_2.valid is True
        assert result_1.errors == result_2.errors

    def test_schema_validate_property_idempotent_invalid_data(self) -> None:
        """Tests schema.validate.property.idempotent

        Calling validate() twice with identical invalid data+schema must produce
        identical results (both valid=False with the same error paths).
        """
        validator = SchemaValidator()
        data: dict[str, Any] = {}  # missing required 'name'
        result_1 = validator.validate(data, _SimpleInput)
        result_2 = validator.validate(data, _SimpleInput)
        assert result_1.valid is False
        assert result_2.valid is False
        paths_1 = [e.path for e in result_1.errors]
        paths_2 = [e.path for e in result_2.errors]
        assert paths_1 == paths_2


# ===========================================================================
# CancelToken — error and property contracts
# ===========================================================================


class TestCancelTokenErrors:
    """Contract tests for CancelToken error codes."""

    def test_cancel_token_raise_if_cancelled_error_execution_cancelled(self) -> None:
        """Tests cancel_token.raise_if_cancelled.error.EXECUTION_CANCELLED

        After cancel() is called, check() must raise ExecutionCancelledError
        with code EXECUTION_CANCELLED.
        """
        token = CancelToken()
        token.cancel()
        with pytest.raises(ExecutionCancelledError) as exc_info:
            token.check()
        assert exc_info.value.code == ErrorCodes.EXECUTION_CANCELLED

    def test_cancel_token_cancel_property_idempotent(self) -> None:
        """Tests cancel_token.cancel.property.idempotent

        Calling cancel() multiple times must be safe (no exception raised).
        The token remains cancelled after subsequent calls.
        """
        token = CancelToken()
        token.cancel()
        token.cancel()  # second call must not raise
        token.cancel()  # third call must not raise
        assert token.is_cancelled is True

    def test_cancel_token_check_not_cancelled_does_not_raise(self) -> None:
        """Tests cancel_token.raise_if_cancelled.error.EXECUTION_CANCELLED (negative)

        Before cancel() is called, check() must not raise.
        """
        token = CancelToken()
        # Should not raise — no exception expected
        token.check()
        assert token.is_cancelled is False

    def test_cancel_token_reset_clears_cancelled_state(self) -> None:
        """Tests cancel_token.cancel.property.idempotent (reset variant)

        After reset(), a previously-cancelled token must not raise on check().
        """
        token = CancelToken()
        token.cancel()
        token.reset()
        # After reset, check() must not raise
        token.check()
        assert token.is_cancelled is False


# ===========================================================================
# ACL.check — property contracts
# ===========================================================================


class TestACLCheckProperties:
    """Property contract tests for ACL.check."""

    def test_acl_check_property_no_raise_for_deny(self) -> None:
        """Tests acl.check.property.no_raise_for_deny

        ACL.check() must return False for a denied call, NOT raise ACLDeniedError.
        Raises are the executor's responsibility, not the ACL layer.
        """
        acl = ACL(rules=[], default_effect="deny")
        result = acl.check(caller_id="some.caller", target_id="some.target")
        # Must return False — not raise
        assert result is False

    def test_acl_check_property_return_bool_allow(self) -> None:
        """Tests acl.check.property.return_bool

        ACL.check() must return a plain Python bool (True) for allowed calls.
        """
        acl = ACL(
            rules=[ACLRule(callers=["test.*"], targets=["*"], effect="allow")],
            default_effect="deny",
        )
        result = acl.check(caller_id="test.module", target_id="other.module")
        assert isinstance(result, bool)
        assert result is True

    def test_acl_check_property_return_bool_deny(self) -> None:
        """Tests acl.check.property.return_bool

        ACL.check() must return a plain Python bool (False) for denied calls.
        """
        acl = ACL(rules=[], default_effect="deny")
        result = acl.check(caller_id="test.module", target_id="other.module")
        assert isinstance(result, bool)
        assert result is False

    def test_acl_check_property_idempotent_allow(self) -> None:
        """Tests acl.check.property.idempotent

        Identical inputs to ACL.check() must always produce the same decision.
        Called three times to confirm stability.
        """
        acl = ACL(
            rules=[ACLRule(callers=["api.*"], targets=["data.*"], effect="allow")],
            default_effect="deny",
        )
        caller = "api.handler"
        target = "data.reader"
        results = [acl.check(caller_id=caller, target_id=target) for _ in range(3)]
        # All results must be the same
        assert len(set(results)) == 1
        assert results[0] is True

    def test_acl_check_property_idempotent_deny(self) -> None:
        """Tests acl.check.property.idempotent

        Identical denied inputs must always produce False on repeated calls.
        """
        acl = ACL(rules=[], default_effect="deny")
        caller = "unknown.caller"
        target = "protected.resource"
        results = [acl.check(caller_id=caller, target_id=target) for _ in range(3)]
        assert len(set(results)) == 1
        assert results[0] is False


# ===========================================================================
# Config.register_namespace — error contracts
# ===========================================================================


class TestConfigRegisterNamespaceErrors:
    """Contract tests for Config.register_namespace error codes."""

    def test_config_register_namespace_error_reserved_apcore(self) -> None:
        """Tests config.register_namespace.error.CONFIG_NAMESPACE_RESERVED

        The name 'apcore' is a reserved namespace. Attempting to register it
        must raise ConfigNamespaceReservedError with code CONFIG_NAMESPACE_RESERVED.
        """
        with pytest.raises(ConfigNamespaceReservedError) as exc_info:
            Config.register_namespace("apcore")
        assert exc_info.value.code == ErrorCodes.CONFIG_NAMESPACE_RESERVED

    def test_config_register_namespace_error_duplicate(self) -> None:
        """Tests config.register_namespace.error.CONFIG_NAMESPACE_DUPLICATE

        Registering the same namespace name twice must raise
        ConfigNamespaceDuplicateError with code CONFIG_NAMESPACE_DUPLICATE.
        """
        ns_name = "test_spec_dup_ns_unique_xyzzy"
        _cleanup_namespace(ns_name)
        try:
            Config.register_namespace(ns_name)
            with pytest.raises(ConfigNamespaceDuplicateError) as exc_info:
                Config.register_namespace(ns_name)
            assert exc_info.value.code == ErrorCodes.CONFIG_NAMESPACE_DUPLICATE
        finally:
            _cleanup_namespace(ns_name)


# ===========================================================================
# Executor.call — error contracts
# ===========================================================================


class TestExecutorCallErrors:
    """Contract tests for Executor.call error codes."""

    def _make_executor(self) -> Executor:
        registry = Registry()
        registry.register("test.valid_module", _TypedModule())
        return Executor(registry=registry)

    def test_executor_call_error_invalid_module_id_empty(self) -> None:
        """Tests executor.call.error.INVALID_MODULE_ID

        An empty module_id must raise InvalidInputError with code
        INVALID_MODULE_ID before attempting any module lookup.
        """
        executor = self._make_executor()
        with pytest.raises(InvalidInputError) as exc_info:
            executor.call("")
        assert exc_info.value.code == ErrorCodes.INVALID_MODULE_ID

    def test_executor_call_error_invalid_module_id_malformed(self) -> None:
        """Tests executor.call.error.INVALID_MODULE_ID

        A module_id with invalid characters (e.g. hyphens) must raise
        InvalidInputError with code INVALID_MODULE_ID.
        """
        executor = self._make_executor()
        with pytest.raises(InvalidInputError) as exc_info:
            executor.call("bad-module-id!")
        assert exc_info.value.code == ErrorCodes.INVALID_MODULE_ID

    def test_executor_call_error_module_not_found(self) -> None:
        """Tests executor.call.error.MODULE_NOT_FOUND

        A valid but unregistered module_id must raise ModuleNotFoundError
        with code MODULE_NOT_FOUND.
        """
        executor = self._make_executor()
        with pytest.raises(ModuleNotFoundError) as exc_info:
            executor.call("unknown.module.does.not.exist")
        assert exc_info.value.code == ErrorCodes.MODULE_NOT_FOUND


# ===========================================================================
# ModuleError.to_dict — pure and invariant contracts
# ===========================================================================


class TestModuleErrorToDict:
    """Contract tests for ModuleError.to_dict property and invariants."""

    def test_module_error_to_dict_property_pure_same_result_twice(self) -> None:
        """Tests module_error.to_dict.property.pure

        to_dict() is pure: calling it twice on the same instance must produce
        equal dicts. No mutation of internal state occurs between calls.
        """
        err = ModuleError(
            code="TEST_CODE",
            message="test message",
            ai_guidance="Some AI guidance",
        )
        result_1 = err.to_dict()
        result_2 = err.to_dict()
        assert result_1 == result_2
        # Also assert we didn't just get empty dicts
        assert result_1["code"] == "TEST_CODE"
        assert result_1["message"] == "test message"

    def test_module_error_to_dict_property_pure_does_not_mutate_instance(self) -> None:
        """Tests module_error.to_dict.property.pure

        to_dict() must not mutate the exception instance. Fields observed
        before and after the call must be identical.
        """
        err = InvalidInputError(message="bad input")
        code_before = err.code
        message_before = err.message
        _ = err.to_dict()
        assert err.code == code_before
        assert err.message == message_before

    def test_module_error_to_dict_property_sparse_null_fields_omitted(self) -> None:
        """Tests module_error.to_dict.property.sparse_output

        Null optional fields (cause, trace_id, retryable, ai_guidance,
        user_fixable, suggestion) must NOT appear in the output dict when
        they were not set.
        """
        err = ModuleError(code="SPARSE_CODE", message="sparse test")
        d = err.to_dict()
        # Required fields must be present
        assert "code" in d
        assert "message" in d
        assert "timestamp" in d
        # Null optional fields must be absent (sparse output contract)
        assert "cause" not in d
        assert "trace_id" not in d
        assert "ai_guidance" not in d
        assert "user_fixable" not in d
        assert "suggestion" not in d

    def test_module_error_to_dict_property_sparse_set_fields_present(self) -> None:
        """Tests module_error.to_dict.property.sparse_output

        Optional fields that are explicitly set must appear in the output.
        """
        err = ModuleError(
            code="FULL_CODE",
            message="full test",
            trace_id="trace-abc-123",
            ai_guidance="Try again with valid input.",
            user_fixable=True,
            suggestion="Fix the input field.",
            retryable=False,
        )
        d = err.to_dict()
        assert d["trace_id"] == "trace-abc-123"
        assert d["ai_guidance"] == "Try again with valid input."
        assert d["user_fixable"] is True
        assert d["suggestion"] == "Fix the input field."
        assert d["retryable"] is False

    def test_module_error_invariant_code_non_empty(self) -> None:
        """Tests module_error.invariant.code_non_empty

        The 'code' field of every ModuleError (and subclass) must be a
        non-empty string. This is the cross-SDK contract invariant.
        """
        err = ModuleError(code="NON_EMPTY_CODE", message="test")
        d = err.to_dict()
        assert isinstance(d["code"], str)
        assert len(d["code"]) > 0

    def test_module_error_invariant_code_non_empty_subclasses(self) -> None:
        """Tests module_error.invariant.code_non_empty

        All standard error subclasses must carry non-empty codes.
        Spot-checks a representative sample.
        """
        errors: list[ModuleError] = [
            InvalidInputError(message="bad"),
            ModuleNotFoundError(module_id="test.missing"),
            SchemaValidationError(message="bad schema"),
            CallDepthExceededError(depth=5, max_depth=4, call_chain=["a", "b", "a"]),
        ]
        for err in errors:
            assert isinstance(err.code, str) and len(err.code) > 0, f"{type(err).__name__}.code must be non-empty"

    def test_module_error_invariant_ai_guidance_non_empty_when_set(self) -> None:
        """Tests module_error.invariant.ai_guidance_non_empty

        When ai_guidance is set on a ModuleError subclass (either via kwarg
        or the default setdefault in the constructor), it must be a non-empty
        string.
        """
        # ModuleNotFoundError auto-sets ai_guidance via setdefault
        err = ModuleNotFoundError(module_id="missing.module")
        d = err.to_dict()
        assert "ai_guidance" in d
        assert isinstance(d["ai_guidance"], str)
        assert len(d["ai_guidance"]) > 0

    def test_module_error_to_dict_details_not_doubled(self) -> None:
        """Tests module_error.to_dict.property.pure

        Calling to_dict() twice must not grow the 'details' field on the
        instance (guards against any accidental mutable-default accumulation).
        """
        err = ModuleError(
            code="DETAILS_CODE",
            message="check details",
            details={"key": "value"},
        )
        d1 = err.to_dict()
        d2 = err.to_dict()
        # Both calls see the same details
        assert d1.get("details") == d2.get("details")


# ===========================================================================
# ErrorCodes constants — invariants
# ===========================================================================


class TestErrorCodesInvariants:
    """Sanity tests that ErrorCodes constants match the codes used in exceptions."""

    def test_error_codes_match_exception_codes(self) -> None:
        """Tests that ErrorCodes constants match the codes produced by each exception.

        Every ErrorCodes constant referenced by a spec clause must equal
        the .code attribute of the corresponding exception instance.
        """
        cases: list[tuple[str, ModuleError]] = [
            (
                ErrorCodes.GENERAL_INVALID_INPUT,
                InvalidInputError(message="x"),
            ),
            (
                ErrorCodes.MODULE_NOT_FOUND,
                ModuleNotFoundError(module_id="x"),
            ),
            (
                ErrorCodes.SCHEMA_VALIDATION_ERROR,
                SchemaValidationError(message="x"),
            ),
            (
                ErrorCodes.CALL_DEPTH_EXCEEDED,
                CallDepthExceededError(depth=5, max_depth=4, call_chain=["a"]),
            ),
            (
                ErrorCodes.CIRCULAR_CALL,
                CircularCallError(module_id="a", call_chain=["a", "b", "a"]),
            ),
            (
                ErrorCodes.CALL_FREQUENCY_EXCEEDED,
                CallFrequencyExceededError(module_id="a", count=4, max_repeat=3, call_chain=["a"]),
            ),
            (
                ErrorCodes.EXECUTION_CANCELLED,
                ExecutionCancelledError(),
            ),
            (
                ErrorCodes.CONFIG_NAMESPACE_RESERVED,
                ConfigNamespaceReservedError(name="apcore"),
            ),
            (
                ErrorCodes.CONFIG_NAMESPACE_DUPLICATE,
                ConfigNamespaceDuplicateError(name="x"),
            ),
        ]
        for expected_code, exc in cases:
            assert exc.code == expected_code, (
                f"{type(exc).__name__}.code={exc.code!r} " f"does not match ErrorCodes constant {expected_code!r}"
            )


# ===========================================================================
# T-W-004: Error code drift regression tests
# registry.register must produce INVALID_MODULE_ID / DUPLICATE_MODULE_ID codes
# ===========================================================================


class TestErrorCodeDrift:
    """Regression tests for T-W-004: spec-declared error codes must match implementation.

    Before the fix, both errors raised InvalidInputError(code=GENERAL_INVALID_INPUT).
    After the fix, they raise with distinct spec-declared codes.
    """

    def test_error_codes_has_invalid_module_id_constant(self) -> None:
        """Tests registry.register.error.INVALID_MODULE_ID

        ErrorCodes must expose INVALID_MODULE_ID as a distinct constant that
        does not equal GENERAL_INVALID_INPUT.
        """
        assert hasattr(ErrorCodes, "INVALID_MODULE_ID"), "ErrorCodes.INVALID_MODULE_ID constant is missing"
        assert (
            ErrorCodes.INVALID_MODULE_ID != ErrorCodes.GENERAL_INVALID_INPUT
        ), "INVALID_MODULE_ID must be distinct from GENERAL_INVALID_INPUT"

    def test_error_codes_has_duplicate_module_id_constant(self) -> None:
        """Tests registry.register.error.DUPLICATE_MODULE_ID

        ErrorCodes must expose DUPLICATE_MODULE_ID as a distinct constant that
        does not equal GENERAL_INVALID_INPUT.
        """
        assert hasattr(ErrorCodes, "DUPLICATE_MODULE_ID"), "ErrorCodes.DUPLICATE_MODULE_ID constant is missing"
        assert (
            ErrorCodes.DUPLICATE_MODULE_ID != ErrorCodes.GENERAL_INVALID_INPUT
        ), "DUPLICATE_MODULE_ID must be distinct from GENERAL_INVALID_INPUT"

    def test_registry_register_malformed_id_uses_invalid_module_id_code(self) -> None:
        """Tests registry.register.error.INVALID_MODULE_ID

        A malformed module_id must raise InvalidInputError with code
        INVALID_MODULE_ID (not GENERAL_INVALID_INPUT).
        """
        registry = Registry()
        with pytest.raises(InvalidInputError) as exc_info:
            registry.register("bad-module-id", _MinimalModule())
        assert exc_info.value.code == ErrorCodes.INVALID_MODULE_ID

    def test_registry_register_empty_id_uses_invalid_module_id_code(self) -> None:
        """Tests registry.register.error.INVALID_MODULE_ID

        An empty module_id must raise InvalidInputError with code
        INVALID_MODULE_ID (not GENERAL_INVALID_INPUT).
        """
        registry = Registry()
        with pytest.raises(InvalidInputError) as exc_info:
            registry.register("", _MinimalModule())
        assert exc_info.value.code == ErrorCodes.INVALID_MODULE_ID

    def test_registry_register_duplicate_uses_duplicate_module_id_code(self) -> None:
        """Tests registry.register.error.DUPLICATE_MODULE_ID

        Registering the same module_id twice must raise InvalidInputError with
        code DUPLICATE_MODULE_ID (not GENERAL_INVALID_INPUT).
        """
        registry = Registry()
        registry.register("test.drift_dup", _MinimalModule())
        with pytest.raises(InvalidInputError) as exc_info:
            registry.register("test.drift_dup", _MinimalModule())
        assert exc_info.value.code == ErrorCodes.DUPLICATE_MODULE_ID
