"""Tests for the ``ephemeral.*`` namespace + ``discoverable`` annotation pilot.

Pilots protocol-spec §2.5.1. Covers:

* Filesystem discovery rejection of ``ephemeral.*`` IDs.
* ``Registry.register()`` happy path for ``ephemeral.*`` IDs.
* Soft warning when ``requires_approval`` is missing on registration.
* ``discoverable=False`` filtered out of ``list`` / ``module_ids`` / ``iter``.
* Audit-event emission on register / unregister with D-35 contextual payload.

The RFC is in ``Draft / RFC`` state; these tests ratchet the implementation
against the proposed semantics and will be promoted to canonical conformance
fixtures after the upstream RFC is accepted.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass, field
from typing import Any

import pytest
from pydantic import BaseModel

from apcore.errors import InvalidInputError
from apcore.events.emitter import ApCoreEvent, EventEmitter
from apcore.module import ModuleAnnotations
from apcore.registry.registry import EPHEMERAL_NAMESPACE_PREFIX, Registry


# ---------------------------------------------------------------------------
# Helper schemas + module classes
# ---------------------------------------------------------------------------


class _In(BaseModel):
    value: str = ""


class _Out(BaseModel):
    result: str = ""


class _EphemeralModuleApproved:
    """Minimal ephemeral module with the recommended ``requires_approval=True``."""

    input_schema = _In
    output_schema = _Out
    description = "Approved ephemeral module"
    annotations = ModuleAnnotations(requires_approval=True, discoverable=False)

    def execute(self, inputs: dict[str, Any], _ctx: Any = None) -> dict[str, Any]:
        return {"result": inputs.get("value", "")}


class _EphemeralModuleNoApproval:
    """Ephemeral module that omits ``requires_approval`` — should trigger warning."""

    input_schema = _In
    output_schema = _Out
    description = "Ephemeral without approval"
    annotations = ModuleAnnotations(discoverable=False)

    def execute(self, inputs: dict[str, Any], _ctx: Any = None) -> dict[str, Any]:
        return {"result": "ok"}


class _DiscoverableHidden:
    """Non-ephemeral module that opts out of enumeration."""

    input_schema = _In
    output_schema = _Out
    description = "Internal-only utility"
    annotations = ModuleAnnotations(discoverable=False)

    def execute(self, inputs: dict[str, Any], _ctx: Any = None) -> dict[str, Any]:
        return {"result": "hidden"}


class _DiscoverableVisible:
    """Standard module — discoverable=True by default."""

    input_schema = _In
    output_schema = _Out
    description = "Standard visible module"

    def execute(self, inputs: dict[str, Any], _ctx: Any = None) -> dict[str, Any]:
        return {"result": "visible"}


# ---------------------------------------------------------------------------
# Capturing subscriber for audit-event tests
# ---------------------------------------------------------------------------


@dataclass
class _CapturingSubscriber:
    events: list[ApCoreEvent] = field(default_factory=list)

    async def on_event(self, event: ApCoreEvent) -> None:
        self.events.append(event)


# ---------------------------------------------------------------------------
# Identity stub for D-35 payload tests
# ---------------------------------------------------------------------------


class _IdentityStub:
    def __init__(self, **kwargs: Any) -> None:
        self.id = kwargs.get("id")
        self.type = kwargs.get("type")
        self.roles = kwargs.get("roles", [])
        self.attrs = kwargs.get("attrs", {})


class _ContextStub:
    def __init__(self, caller_id: str | None = None, identity: Any = None) -> None:
        self.caller_id = caller_id
        self.identity = identity


# ===========================================================================
# 1. Namespace reservation — filesystem discovery
# ===========================================================================


class TestNamespaceReservation:
    def test_filesystem_discovery_rejects_ephemeral_root(self, tmp_path: Any) -> None:
        """A directory named ``ephemeral/`` produces canonical IDs ``ephemeral.<file>`` —
        the registry MUST refuse to register them via the discovery path."""
        root = tmp_path / "extensions"
        eph_dir = root / "ephemeral"
        eph_dir.mkdir(parents=True)
        # Minimal valid module file
        (eph_dir / "synth_tool.py").write_text(
            "from pydantic import BaseModel\n"
            "class _I(BaseModel):\n"
            "    value: str = ''\n"
            "class _O(BaseModel):\n"
            "    result: str = ''\n"
            "class SynthTool:\n"
            "    input_schema = _I\n"
            "    output_schema = _O\n"
            "    description = 'fs-derived'\n"
            "    def execute(self, inputs, ctx=None):\n"
            "        return {'result': 'x'}\n",
            encoding="utf-8",
        )
        registry = Registry(extensions_dir=str(root))
        with pytest.raises(InvalidInputError, match="ephemeral"):
            registry.discover()

    def test_filesystem_discovery_rejects_ephemeral_via_namespace_prefix(self, tmp_path: Any) -> None:
        """A multi-root extension with ``namespace='ephemeral'`` would also produce
        forbidden IDs and MUST be rejected."""
        root = tmp_path / "agent_tools"
        root.mkdir()
        (root / "tool.py").write_text(
            "from pydantic import BaseModel\n"
            "class _I(BaseModel):\n"
            "    value: str = ''\n"
            "class _O(BaseModel):\n"
            "    result: str = ''\n"
            "class Tool:\n"
            "    input_schema = _I\n"
            "    output_schema = _O\n"
            "    description = 'ns-prefixed'\n"
            "    def execute(self, inputs, ctx=None):\n"
            "        return {'result': 'y'}\n",
            encoding="utf-8",
        )
        registry = Registry(extensions_dirs=[{"root": str(root), "namespace": "ephemeral"}])
        with pytest.raises(InvalidInputError, match="ephemeral"):
            registry.discover()

    def test_register_accepts_ephemeral_id(self) -> None:
        """``Registry.register()`` MUST accept ``ephemeral.*`` IDs (programmatic-only path)."""
        registry = Registry()
        mod = _EphemeralModuleApproved()
        registry.register("ephemeral.synth_tool_v1", mod)
        assert registry.get("ephemeral.synth_tool_v1") is mod

    def test_unregister_accepts_ephemeral_id(self) -> None:
        """Caller-managed lifecycle: ``unregister`` removes ephemeral modules."""
        registry = Registry()
        registry.register("ephemeral.tmp", _EphemeralModuleApproved())
        assert registry.unregister("ephemeral.tmp") is True
        assert registry.get("ephemeral.tmp") is None

    def test_namespace_prefix_constant_uses_trailing_dot(self) -> None:
        """Sanity: ``ephemerals.*`` must NOT be classified as the reserved namespace."""
        assert EPHEMERAL_NAMESPACE_PREFIX == "ephemeral."


# ===========================================================================
# 2. discoverable annotation
# ===========================================================================


class TestDiscoverableAnnotation:
    def test_default_annotation_is_discoverable_true(self) -> None:
        """Backward compat: ``ModuleAnnotations()`` must default to ``discoverable=True``."""
        ann = ModuleAnnotations()
        assert ann.discoverable is True

    def test_list_excludes_hidden_modules(self) -> None:
        registry = Registry()
        registry.register("v.visible", _DiscoverableVisible())
        registry.register("h.hidden", _DiscoverableHidden())
        ids = registry.list()
        assert "v.visible" in ids
        assert "h.hidden" not in ids

    def test_list_include_hidden_returns_all(self) -> None:
        registry = Registry()
        registry.register("v.visible", _DiscoverableVisible())
        registry.register("h.hidden", _DiscoverableHidden())
        ids = registry.list(include_hidden=True)
        assert "v.visible" in ids
        assert "h.hidden" in ids

    def test_module_ids_excludes_hidden(self) -> None:
        registry = Registry()
        registry.register("v.visible", _DiscoverableVisible())
        registry.register("h.hidden", _DiscoverableHidden())
        assert "h.hidden" not in registry.module_ids
        assert "v.visible" in registry.module_ids

    def test_iter_excludes_hidden_by_default(self) -> None:
        registry = Registry()
        registry.register("v.visible", _DiscoverableVisible())
        registry.register("h.hidden", _DiscoverableHidden())
        ids = {mid for mid, _ in registry.iter()}
        assert "h.hidden" not in ids
        assert "v.visible" in ids

    def test_iter_include_hidden_returns_all(self) -> None:
        registry = Registry()
        registry.register("v.visible", _DiscoverableVisible())
        registry.register("h.hidden", _DiscoverableHidden())
        ids = {mid for mid, _ in registry.iter(include_hidden=True)}
        assert "h.hidden" in ids
        assert "v.visible" in ids

    def test_hidden_module_remains_callable_via_get(self) -> None:
        """Per the RFC, ``discoverable=False`` only hides from enumeration —
        ``Registry.get()`` MUST still resolve when the caller knows the ID."""
        registry = Registry()
        mod = _DiscoverableHidden()
        registry.register("h.hidden", mod)
        assert registry.get("h.hidden") is mod
        assert registry.has("h.hidden") is True

    def test_count_includes_hidden(self) -> None:
        """``count`` is total registrations, not the visible subset."""
        registry = Registry()
        registry.register("v.visible", _DiscoverableVisible())
        registry.register("h.hidden", _DiscoverableHidden())
        assert registry.count == 2

    def test_yaml_metadata_can_set_discoverable(self) -> None:
        """YAML override: ``annotations: {discoverable: false}`` hides a normally-visible module."""
        registry = Registry()
        registry.register(
            "v.optedout",
            _DiscoverableVisible(),
            metadata={"annotations": {"discoverable": False}},
        )
        assert "v.optedout" not in registry.list()
        assert "v.optedout" in registry.list(include_hidden=True)


# ===========================================================================
# 3. requires_approval soft warning
# ===========================================================================


class TestRequiresApprovalWarning:
    def test_warning_emitted_when_approval_missing(self, caplog: pytest.LogCaptureFixture) -> None:
        registry = Registry()
        with caplog.at_level(logging.WARNING, logger="apcore.registry.registry"):
            registry.register("ephemeral.no_approval", _EphemeralModuleNoApproval())
        assert any(
            "requires_approval" in rec.getMessage() for rec in caplog.records
        ), "expected a WARN about missing requires_approval=True"

    def test_no_warning_when_approval_present(self, caplog: pytest.LogCaptureFixture) -> None:
        registry = Registry()
        with caplog.at_level(logging.WARNING, logger="apcore.registry.registry"):
            registry.register("ephemeral.with_approval", _EphemeralModuleApproved())
        assert not any("requires_approval" in rec.getMessage() for rec in caplog.records)

    def test_no_warning_for_non_ephemeral(self, caplog: pytest.LogCaptureFixture) -> None:
        """Non-ephemeral modules without ``requires_approval`` are NOT warned (regular workflow)."""
        registry = Registry()
        with caplog.at_level(logging.WARNING, logger="apcore.registry.registry"):
            registry.register("normal.module", _DiscoverableVisible())
        assert not any("requires_approval" in rec.getMessage() for rec in caplog.records)

    def test_register_does_not_raise_when_approval_missing(self) -> None:
        """Soft warning, not error: registration MUST succeed."""
        registry = Registry()
        # Should not raise.
        registry.register("ephemeral.no_approval", _EphemeralModuleNoApproval())
        assert registry.get("ephemeral.no_approval") is not None

    def test_warning_handles_dict_annotations(self, caplog: pytest.LogCaptureFixture) -> None:
        """A module exposing ``annotations`` as a plain dict is also covered."""

        class _DictAnn:
            input_schema = _In
            output_schema = _Out
            description = "dict-style annotations, missing approval"
            annotations = {"requires_approval": False, "discoverable": False}

            def execute(self, inputs: dict[str, Any], _ctx: Any = None) -> dict[str, Any]:
                return {"result": ""}

        registry = Registry()
        with caplog.at_level(logging.WARNING, logger="apcore.registry.registry"):
            registry.register("ephemeral.dict_ann", _DictAnn())
        assert any("requires_approval" in rec.getMessage() for rec in caplog.records)


# ===========================================================================
# 4. Audit events
# ===========================================================================


class TestEphemeralAuditEvents:
    def _make_emitter(self) -> tuple[EventEmitter, _CapturingSubscriber]:
        emitter = EventEmitter()
        sub = _CapturingSubscriber()
        emitter.subscribe(sub)
        return emitter, sub

    def _drain(self, emitter: EventEmitter, sub: _CapturingSubscriber, count: int = 1) -> None:
        """Block until ``sub`` has received ``count`` events (with timeout)."""
        import time

        deadline = time.monotonic() + 1.0
        while len(sub.events) < count and time.monotonic() < deadline:
            time.sleep(0.02)

    def test_register_emits_canonical_event(self) -> None:
        emitter, sub = self._make_emitter()
        registry = Registry()
        registry.set_event_emitter(emitter)
        registry.register("ephemeral.audit_v1", _EphemeralModuleApproved())
        self._drain(emitter, sub, 1)
        emitter.shutdown()

        register_events = [e for e in sub.events if e.event_type == "apcore.registry.module_registered"]
        assert len(register_events) == 1
        ev = register_events[0]
        assert ev.module_id == "ephemeral.audit_v1"
        assert ev.severity == "info"
        # Default audit payload — no context = "@external" sentinel
        assert ev.data.get("caller_id") == "@external"
        assert ev.data["identity"] is None
        assert ev.data["namespace_class"] == "ephemeral"

    def test_unregister_emits_canonical_event(self) -> None:
        emitter, sub = self._make_emitter()
        registry = Registry()
        registry.set_event_emitter(emitter)
        registry.register("ephemeral.bye", _EphemeralModuleApproved())
        self._drain(emitter, sub, 1)
        registry.unregister("ephemeral.bye")
        self._drain(emitter, sub, 2)
        emitter.shutdown()

        unreg = [e for e in sub.events if e.event_type == "apcore.registry.module_unregistered"]
        assert len(unreg) == 1
        assert unreg[0].module_id == "ephemeral.bye"

    def test_audit_payload_includes_caller_id_and_identity(self) -> None:
        emitter, sub = self._make_emitter()
        registry = Registry()
        registry.set_event_emitter(emitter)
        identity = _IdentityStub(
            id="user-42",
            type="human",
            roles=["operator"],
            attrs={"display_name": "Tess Tester", "bearer_token": "secret-xyz"},
        )
        ctx = _ContextStub(caller_id="orchestrator.api", identity=identity)
        registry.register("ephemeral.with_ctx", _EphemeralModuleApproved(), context=ctx)
        self._drain(emitter, sub, 1)
        emitter.shutdown()

        ev = next(e for e in sub.events if e.event_type == "apcore.registry.module_registered")
        assert ev.data["caller_id"] == "orchestrator.api"
        assert ev.data["identity"]["id"] == "user-42"
        assert ev.data["identity"]["type"] == "human"
        assert ev.data["identity"]["roles"] == ["operator"]
        assert ev.data["identity"]["display_name"] == "Tess Tester"
        # Sensitive attribute MUST be redacted (matches D-35 behaviour).
        assert ev.data["identity"]["bearer_token"] == "<redacted>"

    def test_no_event_for_non_ephemeral_module(self) -> None:
        """Non-ephemeral registrations MUST NOT trigger the registry-side audit emit."""
        emitter, sub = self._make_emitter()
        registry = Registry()
        registry.set_event_emitter(emitter)
        registry.register("normal.module", _DiscoverableVisible())
        # Briefly wait — no event should ever appear.
        import time

        time.sleep(0.1)
        emitter.shutdown()
        assert all(
            e.module_id != "normal.module" for e in sub.events
        ), "non-ephemeral registrations must not produce registry-side audit events"

    def test_unregister_audit_uses_context_payload(self) -> None:
        emitter, sub = self._make_emitter()
        registry = Registry()
        registry.set_event_emitter(emitter)
        registry.register("ephemeral.unreg_ctx", _EphemeralModuleApproved())
        self._drain(emitter, sub, 1)
        ctx = _ContextStub(caller_id="cleanup.daemon")
        assert registry.unregister("ephemeral.unreg_ctx", context=ctx) is True
        self._drain(emitter, sub, 2)
        emitter.shutdown()
        unreg = next(e for e in sub.events if e.event_type == "apcore.registry.module_unregistered")
        assert unreg.data["caller_id"] == "cleanup.daemon"

    def test_no_emitter_falls_back_to_log(self, caplog: pytest.LogCaptureFixture) -> None:
        """When no emitter is wired the audit info MUST still surface (INFO log)."""
        registry = Registry()  # no set_event_emitter
        with caplog.at_level(logging.INFO, logger="apcore.registry.registry"):
            registry.register("ephemeral.logged", _EphemeralModuleApproved())
        assert any(
            "ephemeral audit event" in rec.getMessage() and "ephemeral.logged" in rec.getMessage()
            for rec in caplog.records
        ), "expected an INFO log fallback when no EventEmitter is wired"

    def test_bridge_does_not_dual_emit_for_ephemeral(self) -> None:
        """Single-emit rule (protocol-spec §2.5.1,
        "Audit events"): when both the registry-side direct
        emit (``set_event_emitter``) and the ``_bridge_registry_events``
        callback bridge are wired, an ``ephemeral.*`` registration MUST
        produce exactly ONE ``apcore.registry.module_registered`` event —
        the rich one with the contextual payload, not a second empty-payload
        copy from the bridge.
        """
        from apcore.sys_modules.registration import _bridge_registry_events

        emitter, sub = self._make_emitter()
        registry = Registry()
        registry.set_event_emitter(emitter)
        # Wire the legacy bridge as well — this is what would have produced
        # the second empty-payload emit before the single-emit guard landed.
        _bridge_registry_events(registry, emitter)

        registry.register("ephemeral.single_emit", _EphemeralModuleApproved())
        self._drain(emitter, sub, 1)
        # Give the bridge a chance to fire if the guard regresses.
        import time

        time.sleep(0.1)
        emitter.shutdown()

        register_events = [
            e
            for e in sub.events
            if e.event_type == "apcore.registry.module_registered" and e.module_id == "ephemeral.single_emit"
        ]
        assert len(register_events) == 1, (
            f"expected exactly one apcore.registry.module_registered event for "
            f"ephemeral.single_emit, got {len(register_events)}: {register_events}"
        )
        # And it MUST be the rich one (caller_id present), not the empty bridge emit.
        assert register_events[0].data.get("caller_id") == "@external"
        # The legacy bare alias `module_registered` MUST also be suppressed for
        # ephemeral.* (single-emit rule applies to both canonical and legacy).
        legacy_events = [
            e for e in sub.events if e.event_type == "module_registered" and e.module_id == "ephemeral.single_emit"
        ]
        assert (
            legacy_events == []
        ), "single-emit rule: legacy `module_registered` alias must also be suppressed for ephemeral.* IDs"


# ===========================================================================
# 5. register_internal() rejection of ephemeral.* IDs
# ===========================================================================


class TestRegisterInternalRejectsEphemeral:
    """Per protocol-spec §2.5.1: ``register_internal()`` MUST reject ``ephemeral.*`` IDs
    and direct the caller to ``Registry.register()``. Rationale: namespace →
    registration-mechanism is a 1:1 mapping; mixing blurs the audit-trail
    distinction between framework-emitted (``system.*``) and caller-emitted
    (``ephemeral.*``) modules.
    """

    def test_register_internal_rejects_ephemeral_id(self) -> None:
        registry = Registry()
        mod = _EphemeralModuleApproved()
        with pytest.raises(InvalidInputError, match="ephemeral"):
            registry.register_internal("ephemeral.test_v1", mod)
        # And the rejection MUST short-circuit before any state mutation.
        assert registry.get("ephemeral.test_v1") is None

    def test_register_internal_error_mentions_register(self) -> None:
        """The error message MUST point the caller to ``Registry.register()``.

        The error is a typed, coded ``InvalidInputError`` — the same class every
        other rejection in ``register_internal`` raises, and what
        apcore-typescript throws / apcore-rust returns. It used to be a bare
        builtin ``ValueError`` with no code, which no conformance fixture could
        cover and no caller could branch on.
        """
        registry = Registry()
        with pytest.raises(InvalidInputError) as exc_info:
            registry.register_internal("ephemeral.bad", _EphemeralModuleApproved())
        msg = str(exc_info.value)
        assert "Registry.register()" in msg
        assert "ephemeral" in msg
        assert exc_info.value.code == "INVALID_MODULE_ID"
