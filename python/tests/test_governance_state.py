"""Executor.governance_state() -- configured vs. actually wired.

PROTOCOL_SPEC 6.6.5. The accessor exists because ``acl is not None`` is not the
answer to "what is gating this registry": the gates are pipeline steps, and the
``internal`` / ``testing`` / ``minimal`` presets remove them.
"""

from __future__ import annotations

from apcore import (
    ACL,
    APCore,
    Config,
    ExecutionPolicy,
    Executor,
    GovernanceState,
    ModuleAnnotations,
)
from apcore.pipeline import ExecutionStrategy

SYS_ON = {"sys_modules": {"enabled": True, "events": {"enabled": True}}}
READ_ONLY = {"sys_modules": {"enabled": True, "events": {"enabled": False}}}
SYS_OFF = {"sys_modules": {"enabled": False}}


def _client(cfg: dict) -> APCore:
    return APCore(config=Config(cfg))


def _executor(client: APCore, strategy=None) -> Executor:
    if strategy is None:
        return client.executor
    return Executor(client.registry, config=client.config, strategy=strategy)


class TestRegistrationObservations:
    def test_no_system_modules(self) -> None:
        state = _client(SYS_OFF).executor.governance_state()
        assert state.control_modules_registered is False
        assert state.read_modules_registered is False
        assert state.unprotected_control_surface is False

    def test_read_modules_only_is_not_a_control_surface(self) -> None:
        """Six read-only modules and no ACL is an information-disclosure
        question, not a control-plane one. The flag must not fire on a
        configuration with no write surface at all."""
        state = _client(READ_ONLY).executor.governance_state()
        assert state.read_modules_registered is True
        assert state.control_modules_registered is False
        assert state.unprotected_control_surface is False

    def test_control_modules_present_with_no_gates(self) -> None:
        state = _client(SYS_ON).executor.governance_state()
        assert state.control_modules_registered is True
        assert state.read_modules_registered is True
        assert state.unprotected_control_surface is True


class TestConfiguredIsNotEnforced:
    """The case the accessor exists for."""

    def test_acl_configured_and_wired_on_standard(self) -> None:
        client = _client(SYS_ON)
        executor = client.executor
        executor.set_acl(ACL(default_effect="deny"))

        state = executor.governance_state()
        assert state.acl_configured is True
        assert state.builtin_acl_gate_wired is True
        assert state.unprotected_control_surface is False

    def test_acl_configured_but_internal_strategy_has_no_acl_step(self) -> None:
        client = _client(SYS_ON)
        executor = _executor(client, "internal")
        executor.set_acl(ACL(default_effect="deny"))

        state = executor.governance_state()
        assert state.acl_configured is True
        assert state.builtin_acl_gate_wired is False
        # `acl is not None` would report this as protected. It is not.
        assert state.unprotected_control_surface is True


class TestGateDetectionIsByType:
    def test_custom_step_named_acl_check_is_not_the_builtin(self) -> None:
        """PROTOCOL_SPEC 6.6.5.2. A name test would set the flag here, and a
        false ``builtin_acl_gate_wired`` is the one direction that must never
        happen -- it reports a gate that is not there."""

        class LookalikeACLCheck:
            name = "acl_check"
            requires: list[str] = []
            provides: list[str] = []

            def execute(self, ctx):  # pragma: no cover - never run
                return None

        client = _client(SYS_ON)
        strategy = ExecutionStrategy("lookalike", [LookalikeACLCheck()], validate_dependencies=False)
        executor = _executor(client, strategy)
        executor.set_acl(ACL(default_effect="deny"))

        state = executor.governance_state()
        assert state.acl_configured is True
        assert state.builtin_acl_gate_wired is False
        assert state.unprotected_control_surface is True


class TestApprovalGateIsPerModuleConditional:
    """PROTOCOL_SPEC 6.6.5.1.1 -- the two gates are not symmetric.

    ``acl_check`` evaluates every call. ``approval_gate`` returns before
    consulting the handler when the module does not need approval, so a wired
    gate plus a handler gates nothing for an unannotated control module.
    """

    @staticmethod
    def _register_control_module(client: APCore, module_id: str, *, requires_approval: bool):
        def handler(inputs, ctx):  # pragma: no cover - never executed
            return {}

        handler.annotations = ModuleAnnotations(requires_approval=requires_approval)
        handler.description = "test control module"
        handler.input_schema = {"type": "object"}
        handler.output_schema = {"type": "object"}
        client.registry.register_internal(module_id, handler)

    def test_sdk_control_modules_declare_requires_approval(self) -> None:
        state = _client(SYS_ON).executor.governance_state()
        assert state.all_control_modules_require_approval is True

    def test_unannotated_control_module_makes_the_surface_unprotected(self) -> None:
        client = _client(SYS_ON)
        self._register_control_module(client, "system.control.custom_thing", requires_approval=False)
        executor = client.executor
        executor.set_approval_handler(_AlwaysApprove())

        state = executor.governance_state()
        assert state.approval_handler_configured is True
        assert state.builtin_approval_gate_wired is True
        assert state.all_control_modules_require_approval is False
        # The v1.15.0 formula answered False here -- a gate that is not there.
        assert state.unprotected_control_surface is True

    def test_strict_policy_does_not_gate_an_unannotated_module(self) -> None:
        client = _client(SYS_ON)
        self._register_control_module(client, "system.control.custom_thing", requires_approval=False)
        executor = client.executor
        executor.set_policy(ExecutionPolicy(strict=True))

        state = executor.governance_state()
        assert state.policy_strict is True
        assert state.all_control_modules_require_approval is False
        assert state.unprotected_control_surface is True

    def test_all_annotated_with_handler_is_gated(self) -> None:
        client = _client(SYS_ON)
        self._register_control_module(client, "system.control.custom_thing", requires_approval=True)
        executor = client.executor
        executor.set_approval_handler(_AlwaysApprove())

        state = executor.governance_state()
        assert state.all_control_modules_require_approval is True
        assert state.unprotected_control_surface is False


class TestAccessorContract:
    def test_is_a_pure_read(self) -> None:
        """6.6.5.3: never enforces, warns, throws or mutates."""
        executor = _client(SYS_ON).executor
        first = executor.governance_state()
        second = executor.governance_state()
        assert first == second

    def test_is_live_not_cached(self) -> None:
        client = _client(SYS_ON)
        executor = client.executor
        before = executor.governance_state()
        executor.set_acl(ACL(default_effect="deny"))
        after = executor.governance_state()

        assert before.acl_configured is False
        assert after.acl_configured is True

    def test_returns_booleans_only(self) -> None:
        """6.6.5.3 constraint 4: no ACL object, handler or policy leaks out."""
        state = _client(SYS_ON).executor.governance_state()
        assert isinstance(state, GovernanceState)
        for field, value in vars(state).items():
            assert isinstance(value, bool), f"{field} is {type(value).__name__}, not bool"


class _AlwaysApprove:
    def request_approval(self, request):  # pragma: no cover - never invoked here
        raise AssertionError("governance_state must not invoke the handler")

    def check_approval(self, token):  # pragma: no cover
        raise AssertionError("governance_state must not invoke the handler")
