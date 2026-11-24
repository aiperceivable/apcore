"""Tests for apcore error hierarchy and AI error guidance fields."""

from __future__ import annotations

import pytest

from apcore.errors import (
    ACLDeniedError,
    ACLRuleError,
    ApprovalDeniedError,
    ApprovalError,
    ApprovalPendingError,
    ApprovalTimeoutError,
    BindingCallableNotFoundError,
    BindingFileInvalidError,
    BindingInvalidTargetError,
    BindingModuleNotFoundError,
    BindingNotCallableError,
    BindingSchemaMissingError,
    CallDepthExceededError,
    CallFrequencyExceededError,
    CircularCallError,
    CircularDependencyError,
    ConfigError,
    ConfigNotFoundError,
    FuncMissingReturnTypeError,
    FuncMissingTypeHintError,
    InternalError,
    InvalidInputError,
    ModuleDisabledError,
    ModuleError,
    ModuleExecuteError,
    ModuleLoadError,
    ModuleNotFoundError,
    ModuleTimeoutError,
    SchemaCircularRefError,
    SchemaMaxDepthExceededError,
    SchemaNotFoundError,
    SchemaParseError,
    SchemaValidationError,
)


class TestModuleErrorBase:
    """Tests for the ModuleError base class."""

    def test_default_ai_fields_are_none(self) -> None:
        err = ModuleError(code="TEST", message="test")
        assert err.retryable is None
        assert err.ai_guidance is None
        assert err.user_fixable is None
        assert err.suggestion is None

    def test_explicit_ai_fields(self) -> None:
        err = ModuleError(
            code="TEST",
            message="test",
            retryable=True,
            ai_guidance="retry after delay",
            user_fixable=False,
            suggestion="Wait and try again",
        )
        assert err.retryable is True
        assert err.ai_guidance == "retry after delay"
        assert err.user_fixable is False
        assert err.suggestion == "Wait and try again"

    def test_retryable_can_be_set_to_false(self) -> None:
        err = ModuleError(code="TEST", message="test", retryable=False)
        assert err.retryable is False

    def test_retryable_can_be_set_to_none(self) -> None:
        err = ModuleError(code="TEST", message="test", retryable=None)
        assert err.retryable is None

    def test_to_dict_sparse_serialization(self) -> None:
        err = ModuleError(code="TEST", message="test")
        d = err.to_dict()
        assert d["code"] == "TEST"
        assert d["message"] == "test"
        assert "timestamp" in d
        assert "retryable" not in d
        assert "ai_guidance" not in d
        assert "user_fixable" not in d
        assert "suggestion" not in d
        assert "details" not in d
        assert "cause" not in d
        assert "trace_id" not in d

    def test_to_dict_includes_non_null_ai_fields(self) -> None:
        err = ModuleError(
            code="TEST",
            message="test",
            retryable=False,
            ai_guidance="do not retry",
            user_fixable=True,
            suggestion="Fix input",
        )
        d = err.to_dict()
        assert d["retryable"] is False
        assert d["ai_guidance"] == "do not retry"
        assert d["user_fixable"] is True
        assert d["suggestion"] == "Fix input"

    def test_to_dict_includes_details_when_present(self) -> None:
        err = ModuleError(code="TEST", message="test", details={"key": "val"})
        d = err.to_dict()
        assert d["details"] == {"key": "val"}

    def test_to_dict_includes_cause_as_string(self) -> None:
        cause = ValueError("root")
        err = ModuleError(code="TEST", message="test", cause=cause)
        d = err.to_dict()
        assert d["cause"] == "root"

    def test_to_dict_includes_trace_id(self) -> None:
        err = ModuleError(code="TEST", message="test", trace_id="abc-123")
        d = err.to_dict()
        assert d["trace_id"] == "abc-123"

    def test_str_format(self) -> None:
        err = ModuleError(code="MY_CODE", message="my message")
        assert str(err) == "[MY_CODE] my message"

    def test_is_instance_of_exception(self) -> None:
        err = ModuleError(code="X", message="msg")
        assert isinstance(err, Exception)


class TestDefaultRetryable:
    """Tests that each subclass has the correct _default_retryable value."""

    @pytest.mark.parametrize(
        ("cls", "kwargs"),
        [
            (ModuleTimeoutError, {"module_id": "m", "timeout_ms": 1000}),
            (InternalError, {"message": "err"}),
            (ApprovalTimeoutError, {"result": None, "module_id": "m"}),
        ],
    )
    def test_retryable_true(self, cls: type[ModuleError], kwargs: dict) -> None:
        err = cls(**kwargs)
        assert err.retryable is True

    def test_retryable_none_for_module_execute_error(self) -> None:
        err = ModuleExecuteError(module_id="m", message="fail")
        assert err.retryable is None

    @pytest.mark.parametrize(
        ("cls", "kwargs"),
        [
            (ConfigNotFoundError, {"config_path": "/etc/app.yaml"}),
            (ConfigError, {"message": "bad"}),
            (ACLRuleError, {"message": "bad rule"}),
            (ACLDeniedError, {"caller_id": "a", "target_id": "b"}),
            (ApprovalDeniedError, {"result": None, "module_id": "m"}),
            (ApprovalPendingError, {"result": None, "module_id": "m"}),
            (ModuleNotFoundError, {"module_id": "m"}),
            (SchemaValidationError, {}),
            (SchemaNotFoundError, {"schema_id": "s"}),
            (SchemaParseError, {"message": "bad"}),
            (SchemaCircularRefError, {"ref_path": "#/a"}),
            (SchemaMaxDepthExceededError, {"ref_path": "#/a"}),
            (CallDepthExceededError, {"depth": 5, "max_depth": 4, "call_chain": ["a"]}),
            (CircularCallError, {"module_id": "m", "call_chain": ["m"]}),
            (CallFrequencyExceededError, {"module_id": "m", "count": 4, "max_repeat": 3, "call_chain": ["m"]}),
            (InvalidInputError, {}),
            (FuncMissingTypeHintError, {"function_name": "f", "parameter_name": "p"}),
            (FuncMissingReturnTypeError, {"function_name": "f"}),
            (BindingInvalidTargetError, {"target": "t"}),
            (BindingModuleNotFoundError, {"module_path": "m"}),
            (BindingCallableNotFoundError, {"callable_name": "c", "module_path": "m"}),
            (BindingNotCallableError, {"target": "t"}),
            (BindingSchemaMissingError, {"target": "t"}),
            (BindingFileInvalidError, {"file_path": "/f", "reason": "bad"}),
            (CircularDependencyError, {"cycle_path": ["a", "b", "a"]}),
            (ModuleLoadError, {"module_id": "m", "reason": "fail"}),
        ],
    )
    def test_retryable_false(self, cls: type[ModuleError], kwargs: dict) -> None:
        err = cls(**kwargs)
        assert err.retryable is False


class TestSubclassKwargsOverride:
    """Tests that subclasses can override AI fields via kwargs."""

    def test_override_retryable_on_non_retryable_subclass(self) -> None:
        err = ConfigNotFoundError(config_path="/app.yaml", retryable=True)
        assert err.retryable is True

    def test_override_retryable_to_false_on_retryable_subclass(self) -> None:
        err = ModuleTimeoutError(module_id="m", timeout_ms=1000, retryable=False)
        assert err.retryable is False

    def test_pass_ai_guidance_via_kwargs(self) -> None:
        err = SchemaValidationError(
            ai_guidance="check schema definition",
            suggestion="Fix the input field",
        )
        assert err.ai_guidance == "check schema definition"
        assert err.suggestion == "Fix the input field"

    def test_pass_user_fixable_via_kwargs(self) -> None:
        err = ACLDeniedError(caller_id="a", target_id="b", user_fixable=True)
        assert err.user_fixable is True

    def test_approval_error_with_ai_fields(self) -> None:
        err = ApprovalDeniedError(
            result=None,
            module_id="m",
            retryable=True,
            ai_guidance="request with different user",
            user_fixable=True,
            suggestion="Ask admin for permission",
        )
        assert err.retryable is True
        assert err.ai_guidance == "request with different user"
        assert err.user_fixable is True
        assert err.suggestion == "Ask admin for permission"


class TestBackwardCompatibility:
    """Tests that existing constructor calls still work unchanged."""

    def test_module_error_positional(self) -> None:
        err = ModuleError("CODE", "msg", {"k": "v"}, ValueError("cause"), "trace-1")
        assert err.code == "CODE"
        assert err.details == {"k": "v"}
        assert err.trace_id == "trace-1"
        assert err.retryable is None

    def test_config_not_found_error(self) -> None:
        err = ConfigNotFoundError("/path")
        assert err.code == "CONFIG_NOT_FOUND"
        assert err.retryable is False

    def test_module_timeout_with_cause(self) -> None:
        cause = RuntimeError("network")
        err = ModuleTimeoutError(module_id="m", timeout_ms=5000, cause=cause)
        assert err.cause is cause
        assert err.retryable is True

    def test_approval_denied_error(self) -> None:
        err = ApprovalDeniedError(result=None, module_id="test")
        assert err.code == "APPROVAL_DENIED"
        assert err.retryable is False

    def test_approval_pending_error(self) -> None:
        err = ApprovalPendingError(result=None, module_id="test")
        assert err.code == "APPROVAL_PENDING"
        assert err.retryable is False


class TestApprovalErrorBase:
    """Tests for ApprovalError base class and AI field overrides on approval subclasses."""

    def test_approval_error_base_class(self) -> None:
        err = ApprovalError(code="APPROVAL_DENIED", message="denied", result=None, module_id="mod.x")
        assert err.code == "APPROVAL_DENIED"
        assert err.module_id == "mod.x"
        assert err.retryable is False

    def test_approval_timeout_with_ai_fields(self) -> None:
        err = ApprovalTimeoutError(
            result=None,
            module_id="m",
            retryable=False,
            ai_guidance="do not retry automatically",
            user_fixable=True,
            suggestion="Contact the approver directly",
        )
        assert err.retryable is False
        assert err.ai_guidance == "do not retry automatically"
        assert err.user_fixable is True
        assert err.suggestion == "Contact the approver directly"

    def test_approval_pending_with_ai_fields(self) -> None:
        err = ApprovalPendingError(
            result=None,
            module_id="m",
            ai_guidance="poll for approval status",
            suggestion="Wait for approval or contact approver",
        )
        assert err.retryable is False
        assert err.ai_guidance == "poll for approval status"
        assert err.suggestion == "Wait for approval or contact approver"


class TestAiGuidanceDefaults:
    """Tests that error classes with setdefault('ai_guidance', ...) provide a non-empty default."""

    @pytest.mark.parametrize(
        ("cls", "kwargs"),
        [
            (ACLDeniedError, {"caller_id": "agent", "target_id": "secret.module"}),
            (ModuleNotFoundError, {"module_id": "missing.mod"}),
            (ModuleDisabledError, {"module_id": "disabled.mod"}),
            (ModuleTimeoutError, {"module_id": "slow.mod", "timeout_ms": 5000}),
            (SchemaValidationError, {}),
            (CallDepthExceededError, {"depth": 10, "max_depth": 8, "call_chain": ["a", "b"]}),
            (CircularCallError, {"module_id": "loop.mod", "call_chain": ["a", "b", "a"]}),
        ],
    )
    def test_default_ai_guidance_is_non_empty(self, cls: type[ModuleError], kwargs: dict) -> None:
        err = cls(**kwargs)
        assert err.ai_guidance is not None
        assert len(err.ai_guidance) > 0

    @pytest.mark.parametrize(
        ("cls", "kwargs"),
        [
            (ACLDeniedError, {"caller_id": "agent", "target_id": "secret.module"}),
            (ModuleNotFoundError, {"module_id": "missing.mod"}),
            (ModuleDisabledError, {"module_id": "disabled.mod"}),
            (ModuleTimeoutError, {"module_id": "slow.mod", "timeout_ms": 5000}),
            (SchemaValidationError, {}),
            (CallDepthExceededError, {"depth": 10, "max_depth": 8, "call_chain": ["a", "b"]}),
            (CircularCallError, {"module_id": "loop.mod", "call_chain": ["a", "b", "a"]}),
        ],
    )
    def test_explicit_ai_guidance_overrides_default(self, cls: type[ModuleError], kwargs: dict) -> None:
        custom_guidance = "custom AI guidance message"
        err = cls(**kwargs, ai_guidance=custom_guidance)
        assert err.ai_guidance == custom_guidance


class TestCircuitBreakerOpenError:
    """Cross-language alignment (sync A-001): canonical name CircuitBreakerOpenError, code CIRCUIT_BREAKER_OPEN."""

    def test_circuit_breaker_open_error_canonical_name_and_code(self) -> None:
        from apcore.errors import CircuitBreakerOpenError, CircuitOpenError, ErrorCodes

        # Canonical class exists and exposes the new code constant
        assert hasattr(ErrorCodes, "CIRCUIT_BREAKER_OPEN")
        assert ErrorCodes.CIRCUIT_BREAKER_OPEN == "CIRCUIT_BREAKER_OPEN"

        err = CircuitBreakerOpenError(module_id="my.module")
        assert err.code == "CIRCUIT_BREAKER_OPEN"
        assert err.retryable is True
        assert err.module_id == "my.module"

        # Backwards-compatible alias: existing user code that catches
        # CircuitOpenError must still work — it is a subclass of the new
        # canonical class so isinstance checks pass both ways.
        legacy = CircuitOpenError(module_id="legacy.module")
        assert isinstance(legacy, CircuitBreakerOpenError)
        assert isinstance(CircuitBreakerOpenError(module_id="x"), CircuitBreakerOpenError)

    def test_circuit_breaker_open_error_exported_from_apcore(self) -> None:
        import apcore

        assert hasattr(apcore, "CircuitBreakerOpenError")
        assert apcore.CircuitBreakerOpenError is not None
        # Legacy alias still exported
        assert hasattr(apcore, "CircuitOpenError")
        assert issubclass(apcore.CircuitOpenError, apcore.CircuitBreakerOpenError)


class TestBindingSchemaErrorCodesRegistered:
    """The binding auto-schema error codes must be ErrorCodes constants and
    therefore present in the framework-codes collision set (TS/Rust parity)."""

    def test_codes_exist_and_match_subclass_emitted_codes(self) -> None:
        from apcore.errors import (
            BindingSchemaInferenceFailedError,
            BindingSchemaModeConflictError,
            BindingStrictSchemaIncompatibleError,
            ErrorCodes,
        )

        inference = BindingSchemaInferenceFailedError(target="mod.fn")
        assert ErrorCodes.BINDING_SCHEMA_INFERENCE_FAILED == "BINDING_SCHEMA_INFERENCE_FAILED"
        assert ErrorCodes.BINDING_SCHEMA_INFERENCE_FAILED == inference.code

        mode_conflict = BindingSchemaModeConflictError(module_id="m", modes_listed=["a", "b"])
        assert ErrorCodes.BINDING_SCHEMA_MODE_CONFLICT == "BINDING_SCHEMA_MODE_CONFLICT"
        assert ErrorCodes.BINDING_SCHEMA_MODE_CONFLICT == mode_conflict.code

        strict = BindingStrictSchemaIncompatibleError(module_id="m", features_listed=["x"])
        assert ErrorCodes.BINDING_STRICT_SCHEMA_INCOMPATIBLE == "BINDING_STRICT_SCHEMA_INCOMPATIBLE"
        assert ErrorCodes.BINDING_STRICT_SCHEMA_INCOMPATIBLE == strict.code

    def test_codes_in_framework_collision_set(self) -> None:
        from apcore.errors import _FRAMEWORK_CODES

        assert "BINDING_SCHEMA_INFERENCE_FAILED" in _FRAMEWORK_CODES
        assert "BINDING_SCHEMA_MODE_CONFLICT" in _FRAMEWORK_CODES
        assert "BINDING_STRICT_SCHEMA_INCOMPATIBLE" in _FRAMEWORK_CODES
