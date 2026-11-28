"""Tests for the apcore public API surface.

Verifies that all expected names are importable from the top-level
``apcore`` package and that ``__all__`` is comprehensive.
"""

import builtins
import re

import apcore


class TestPublicAPIImports:
    """Every public component must be importable from ``import apcore``."""

    # -- Core --

    def test_context_importable(self):
        from apcore import Context

        assert Context is not None

    def test_identity_importable(self):
        from apcore import Identity

        assert Identity is not None

    def test_registry_importable(self):
        from apcore import Registry

        assert Registry is not None

    def test_executor_importable(self):
        from apcore import Executor

        assert Executor is not None

    # -- Module types --

    def test_module_importable(self):
        from apcore import Module

        assert Module is not None

    def test_module_annotations_importable(self):
        from apcore import ModuleAnnotations

        assert ModuleAnnotations is not None

    def test_module_example_importable(self):
        from apcore import ModuleExample

        assert ModuleExample is not None

    def test_validation_result_importable(self):
        from apcore import ValidationResult

        assert ValidationResult is not None

    def test_change_importable(self):
        from apcore import Change

        assert Change is not None

    def test_preview_result_importable(self):
        from apcore import PreviewResult

        assert PreviewResult is not None

    # -- Registry types --

    def test_module_descriptor_importable(self):
        from apcore import ModuleDescriptor

        assert ModuleDescriptor is not None

    # -- Config --

    def test_config_importable(self):
        from apcore import Config

        assert Config is not None

    # -- Errors --

    def test_module_error_importable(self):
        from apcore import ModuleError

        assert ModuleError is not None

    def test_schema_validation_error_importable(self):
        from apcore import SchemaValidationError

        assert SchemaValidationError is not None

    def test_acl_denied_error_importable(self):
        from apcore import ACLDeniedError

        assert ACLDeniedError is not None

    def test_module_not_found_error_importable(self):
        from apcore import ModuleNotFoundError

        assert ModuleNotFoundError is not None

    def test_config_error_importable(self):
        from apcore import ConfigError

        assert ConfigError is not None

    def test_circular_dependency_error_importable(self):
        from apcore import CircularDependencyError

        assert CircularDependencyError is not None

    def test_invalid_input_error_importable(self):
        from apcore import InvalidInputError

        assert InvalidInputError is not None

    def test_module_timeout_error_importable(self):
        from apcore import ModuleTimeoutError

        assert ModuleTimeoutError is not None

    def test_call_depth_exceeded_error_importable(self):
        from apcore import CallDepthExceededError

        assert CallDepthExceededError is not None

    def test_circular_call_error_importable(self):
        from apcore import CircularCallError

        assert CircularCallError is not None

    def test_module_id_errors_importable(self):
        """A-001: ModuleIdConflictError, InvalidSegmentError, and
        IdTooLongError are exported by the TS/Rust SDKs and the errors
        submodule; they must also be importable from the top-level apcore
        package."""
        from apcore import IdTooLongError, InvalidSegmentError, ModuleIdConflictError

        assert ModuleIdConflictError is not None
        assert InvalidSegmentError is not None
        assert IdTooLongError is not None

    def test_call_frequency_exceeded_error_importable(self):
        from apcore import CallFrequencyExceededError

        assert CallFrequencyExceededError is not None

    # -- ACL --

    def test_acl_importable(self):
        from apcore import ACL

        assert ACL is not None

    def test_acl_rule_importable(self):
        from apcore import ACLRule

        assert ACLRule is not None

    def test_condition_outcome_importable(self):
        from apcore import ConditionOutcome

        assert {o.name for o in ConditionOutcome} == {"SATISFIED", "UNSATISFIED", "UNEVALUABLE"}

    def test_rule_validation_finding_importable(self):
        from apcore import RuleValidationFinding

        assert RuleValidationFinding is not None

    # -- Middleware --

    def test_middleware_importable(self):
        from apcore import Middleware

        assert Middleware is not None

    def test_middleware_manager_importable(self):
        from apcore import MiddlewareManager

        assert MiddlewareManager is not None

    def test_before_middleware_importable(self):
        from apcore import BeforeMiddleware

        assert BeforeMiddleware is not None

    def test_after_middleware_importable(self):
        from apcore import AfterMiddleware

        assert AfterMiddleware is not None

    def test_logging_middleware_importable(self):
        from apcore import LoggingMiddleware

        assert LoggingMiddleware is not None

    # -- Decorators --

    def test_module_decorator_importable(self):
        from apcore import module

        assert module is not None

    def test_function_module_importable(self):
        from apcore import FunctionModule

        assert FunctionModule is not None

    # -- Extensions --

    def test_extension_manager_importable(self):
        from apcore import ExtensionManager

        assert ExtensionManager is not None

    def test_extension_point_importable(self):
        from apcore import ExtensionPoint

        assert ExtensionPoint is not None

    # -- Async tasks --

    def test_async_task_manager_importable(self):
        from apcore import AsyncTaskManager

        assert AsyncTaskManager is not None

    def test_task_status_importable(self):
        from apcore import TaskStatus

        assert TaskStatus is not None

    def test_task_info_importable(self):
        from apcore import TaskInfo

        assert TaskInfo is not None

    # -- Trace Context --

    def test_trace_context_importable(self):
        from apcore import TraceContext

        assert TraceContext is not None

    def test_trace_parent_importable(self):
        from apcore import TraceParent

        assert TraceParent is not None

    # -- Bindings --

    def test_binding_loader_importable(self):
        from apcore import BindingLoader

        assert BindingLoader is not None

    # -- Utilities --

    def test_redact_sensitive_importable(self):
        from apcore import redact_sensitive

        assert redact_sensitive is not None

    def test_redacted_value_importable(self):
        from apcore import REDACTED_VALUE

        assert REDACTED_VALUE == "***REDACTED***"

    # -- Observability --

    def test_tracing_middleware_importable(self):
        from apcore import TracingMiddleware

        assert TracingMiddleware is not None

    def test_context_logger_importable(self):
        from apcore import ContextLogger

        assert ContextLogger is not None

    def test_obs_logging_middleware_importable(self):
        from apcore import ObsLoggingMiddleware

        assert ObsLoggingMiddleware is not None

    def test_metrics_middleware_importable(self):
        from apcore import MetricsMiddleware

        assert MetricsMiddleware is not None

    def test_metrics_collector_importable(self):
        from apcore import MetricsCollector

        assert MetricsCollector is not None

    def test_span_importable(self):
        from apcore import Span

        assert Span is not None

    def test_stdout_exporter_importable(self):
        from apcore import StdoutExporter

        assert StdoutExporter is not None

    def test_in_memory_exporter_importable(self):
        from apcore import InMemoryExporter

        assert InMemoryExporter is not None

    # -- Registry protocols --

    def test_discoverer_importable(self):
        from apcore import Discoverer

        assert Discoverer is not None

    def test_module_validator_importable(self):
        from apcore import ModuleValidator

        assert ModuleValidator is not None

    # -- Multi-class discovery (cross-language root parity: TS/Rust export at root) --

    def test_multi_class_discovery_importable_from_root(self):
        from apcore import class_name_to_segment, discover_multi_class

        assert class_name_to_segment is not None
        assert discover_multi_class is not None

    # NOTE (audit D1-003): the events-layer CircuitBreakerWrapper / CircuitState
    # are intentionally NOT exposed at the apcore top level (see
    # test_circuit_breaker_middleware.py::test_old_middleware_circuit_state_name_is_gone).
    # The TS/Rust root export of CircuitState is an accepted cross-language
    # divergence — Python avoids the ambiguous top-level CircuitState name.

    # -- Shadowing safety --

    def test_module_not_found_error_is_not_builtin(self):
        assert apcore.ModuleNotFoundError is not builtins.ModuleNotFoundError
        assert issubclass(apcore.ModuleNotFoundError, apcore.ModuleError)

    # -- SpanExporter is now exported --

    def test_span_exporter_is_available_from_observability(self):
        from apcore.observability import SpanExporter

        assert "SpanExporter" not in apcore.__all__
        assert apcore.SpanExporter is SpanExporter

    # -- Version --

    def test_version_is_set(self):
        assert hasattr(apcore, "__version__")
        assert isinstance(apcore.__version__, str)
        assert re.match(r"^\d+\.\d+\.\d+", apcore.__version__)

    # -- Pipeline preset builders (parity with apcore-typescript / apcore-rust) --
    # Regression for sync finding A-006: Python previously only exported
    # build_strategy_from_config; the 5 named-preset builders existed in
    # apcore.builtin_steps but were not re-exported to the package root.

    def test_pipeline_preset_builders_importable_from_package_root(self):
        from apcore import (
            build_internal_strategy,
            build_minimal_strategy,
            build_performance_strategy,
            build_standard_strategy,
            build_testing_strategy,
        )

        for fn in (
            build_standard_strategy,
            build_internal_strategy,
            build_testing_strategy,
            build_performance_strategy,
            build_minimal_strategy,
        ):
            assert callable(fn)

    # -- ID conflict detection (D1-004) --
    # Regression: detect_id_conflicts, ConflictResult were defined in
    # apcore.registry.conflicts but not re-exported from the package root.

    def test_detect_id_conflicts_importable_from_package_root(self):
        from apcore import ConflictResult, detect_id_conflicts

        assert callable(detect_id_conflicts)
        assert ConflictResult is not None

    # -- Registry module-id constants (issue #30) --
    # Regression: MAX_MODULE_ID_LENGTH, RESERVED_WORDS, REGISTRY_EVENTS,
    # EPHEMERAL_NAMESPACE_PREFIX, DEFAULT_MODULE_VERSION and MODULE_ID_PATTERN
    # are root-public in apcore-typescript / apcore-rust but were reachable in
    # Python only via the internal module apcore.registry.registry. They must be
    # importable from both the apcore.registry package and the top-level apcore.

    _REGISTRY_CONSTANTS = (
        "MAX_MODULE_ID_LENGTH",
        "RESERVED_WORDS",
        "REGISTRY_EVENTS",
        "EPHEMERAL_NAMESPACE_PREFIX",
        "DEFAULT_MODULE_VERSION",
        "MODULE_ID_PATTERN",
    )

    def test_registry_constants_importable_from_package_root(self):
        import apcore as apcore_mod

        for name in self._REGISTRY_CONSTANTS:
            assert hasattr(apcore_mod, name), f"{name} not importable from apcore"

    def test_registry_constants_importable_from_registry_package(self):
        import apcore.registry as reg

        for name in self._REGISTRY_CONSTANTS:
            assert hasattr(reg, name), f"{name} not importable from apcore.registry"
            assert name in reg.__all__, f"{name} missing from apcore.registry.__all__"

    def test_registry_constant_values_are_consistent(self):
        """The root, the registry package, and the internal module must all
        resolve to the same object (no shadowing / divergent copies)."""
        import apcore as apcore_mod
        import apcore.registry as reg
        from apcore.registry import registry as deep

        for name in self._REGISTRY_CONSTANTS:
            assert getattr(apcore_mod, name) is getattr(deep, name)
            assert getattr(reg, name) is getattr(deep, name)

    def test_canonical_length_name_matches_multi_class_alias(self):
        """Tier 2: MAX_MODULE_ID_LEN is a back-compat alias; the canonical
        cross-SDK name MAX_MODULE_ID_LENGTH must now be public and equal."""
        import apcore as apcore_mod
        from apcore.registry import MAX_MODULE_ID_LEN

        assert apcore_mod.MAX_MODULE_ID_LENGTH == MAX_MODULE_ID_LEN


class TestPublicAPIAll:
    """Verify __all__ is comprehensive and matches actual exports."""

    EXPECTED_NAMES = {
        "ACL",
        "ACLRule",
        "APCore",
        "AsyncBoundModule",
        "ApprovalDeniedError",
        "ApprovalHandler",
        "ApprovalPendingError",
        "ApprovalRequest",
        "ApprovalResult",
        "AutoApproveHandler",
        "BoundModule",
        "CancelToken",
        "AccessHint",
        "CallableDefinition",
        "CapabilityPage",
        "CapabilitySearchProvider",
        "ModuleSummary",
        "VisibilityDecision",
        "Context",
        "ErrorCodes",
        "ExecutionPolicy",
        "PolicyDeniedError",
        "PolicyEvaluationError",
        "PolicyEvaluationRequest",
        "PolicyEvaluator",
        "PolicyVerdict",
        "ExecutionStrategy",
        "Executor",
        "Identity",
        "Middleware",
        "Module",
        "ModuleAnnotations",
        "ModuleError",
        "PipelineEngine",
        "PreflightResult",
        "PreviewResult",
        "Registry",
        "ReloadOptions",
        "SchemaExporter",
        "SchemaValidator",
        "ScopedApprovalHandler",
        "ShutdownOptions",
        "StreamCompletion",
        "StreamExecution",
        "StreamingModule",
        "TraceContext",
        "UnregisterOptions",
        "call",
        "call_async",
        "close",
        "module",
        "redact_sensitive",
        "register",
        "stream",
        "to_strict_schema",
        "validate",
    }

    def test_all_contains_all_expected_names(self):
        actual = set(apcore.__all__)
        missing = self.EXPECTED_NAMES - actual
        assert not missing, f"Missing from __all__: {missing}"

    def test_all_has_no_unexpected_extras(self):
        actual = set(apcore.__all__)
        extra = actual - self.EXPECTED_NAMES
        assert not extra, f"Unexpected names in __all__: {extra}"

    def test_all_names_are_importable(self):
        _MISSING = object()
        for name in apcore.__all__:
            obj = getattr(apcore, name, _MISSING)
            assert obj is not _MISSING, f"Name '{name}' listed in __all__ but not found on module"

    def test_default_client_is_lazy_and_close_resets(self):
        """import apcore must not eagerly construct APCore(); close() releases + allows fresh create."""
        import apcore as apcore_mod

        # Force a clean slate — clear any client allocated earlier in the session.
        apcore_mod.close()
        # Private state check: the backing slot is now None.
        assert apcore_mod._default_client is not None  # triggers lazy init via __getattr__
        # Because access via __getattr__ materialises the client, we close again and assert
        # the backing module-level name is bound to the freshly-created instance.
        first = apcore_mod._default_client
        apcore_mod.close()
        second = apcore_mod._default_client
        # After close(), a new instance is created on next access — not the same object.
        assert first is not second

    def test_legacy_global_subscriber_apis_removed(self):
        """Subscriber registration belongs to the host registry."""
        deprecated = (
            "register_subscriber_type",
            "unregister_subscriber_type",
            "reset_subscriber_registry",
        )
        for name in deprecated:
            assert name not in apcore.__all__, f"Deprecated {name} should not be advertised in __all__"
            assert not hasattr(apcore, name)
