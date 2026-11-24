"""Regression tests for registry sync-audit findings against v0.22.0.

- A-D-REG-002 — ``register_internal`` rejects both ``"ephemeral"`` (bare)
  and ``"ephemeral.*"`` IDs via the shared ``_is_ephemeral`` helper.
  (The previous ``.startswith("ephemeral.")`` check missed bare
  ``"ephemeral"``, leaving a one-character carveout that contradicted
  the canonical classifier.)

- A-D-REG-003 / Issue #65 — Discover-path registration uses the same
  deferred-publish protocol as ``register()``: modules MUST NOT appear
  in the visible store until ``on_load`` completes successfully.

- A-D-REG-004 / Issue #65 — ``register_internal`` uses deferred-publish
  too — the same invariant holds for sys-modules. ``on_load`` failures
  emit ``apcore.registry.module_load_failed`` and leave the registry
  exactly as before the registration attempt.

- A-D-REG-005 — Discover-path ``on_load`` failures emit
  ``apcore.registry.module_load_failed``, matching the public path so
  subscribers have a single hook for partial-init detection.
"""

from __future__ import annotations

from typing import Any
from unittest.mock import MagicMock

import pytest

from apcore.errors import InvalidInputError
from apcore.registry.registry import Registry


class _NoopModule:
    description = "stub"
    input_schema = {"type": "object"}
    output_schema = {"type": "object"}

    async def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        return {}


class _ProbeOnLoadModule(_NoopModule):
    """on_load that asserts the module is NOT visible while the callback runs."""

    def __init__(self, registry: Registry, mod_id: str) -> None:
        self._registry = registry
        self._mod_id = mod_id
        self.visible_during_on_load: bool | None = None

    def on_load(self) -> None:
        self.visible_during_on_load = self._registry.get(self._mod_id) is not None


class _FailingOnLoadModule(_NoopModule):
    """on_load that always raises."""

    def on_load(self) -> None:
        raise RuntimeError("intentional on_load failure")


# ---------------------------------------------------------------------------
# A-D-REG-002 — register_internal rejects bare "ephemeral" too
# ---------------------------------------------------------------------------


class TestRegisterInternalRejectsBareEphemeralId:
    def test_bare_ephemeral_id_rejected(self) -> None:
        reg = Registry()
        with pytest.raises(InvalidInputError, match="ephemeral"):
            reg.register_internal("ephemeral", _NoopModule())

    def test_namespaced_ephemeral_id_rejected(self) -> None:
        reg = Registry()
        with pytest.raises(InvalidInputError, match="ephemeral"):
            reg.register_internal("ephemeral.foo", _NoopModule())


# ---------------------------------------------------------------------------
# A-D-REG-003 — Discover-path uses deferred-publish (no early visibility)
# ---------------------------------------------------------------------------


class TestDiscoverPathDeferredPublish:
    def test_module_invisible_during_on_load(self) -> None:
        reg = Registry()
        module = _ProbeOnLoadModule(reg, "system.probe")
        # Drive _register_in_order directly via the same plumbing discover()
        # would use — the only thing being tested here is the deferred-publish
        # ordering, not the upstream stages of the 8-step pipeline.
        reg._register_in_order(
            load_order=["system.probe"],
            valid_classes={
                "system.probe": type(
                    "_ProbeFactory",
                    (),
                    {
                        "__init__": lambda self_, m=module: setattr(self_, "_proxy", m) or None,
                        "__getattr__": lambda self_, n: getattr(self_._proxy, n),
                    },
                )
            },
            raw_metadata={},
        )
        # Probe-style assertion: the on_load callback's snapshot of
        # ``registry.get(...)`` must have been ``None`` — i.e. the
        # module was NOT yet published when the callback ran.
        assert (
            module.visible_during_on_load is False
        ), "Deferred-publish invariant violated: module was visible during on_load"

    def test_on_load_failure_does_not_publish(self) -> None:
        reg = Registry()
        cls = type("_FailingFactory", (_FailingOnLoadModule,), {})
        count = reg._register_in_order(
            load_order=["system.fail"],
            valid_classes={"system.fail": cls},
            raw_metadata={},
        )
        assert count == 0
        # Module must NOT be visible after on_load failure.
        assert reg.get("system.fail") is None
        assert "system.fail" not in reg.list()
        # And the in-flight slot must be released.
        assert "system.fail" not in reg._in_flight


# ---------------------------------------------------------------------------
# A-D-REG-004 — register_internal uses deferred-publish + DLQ event
# ---------------------------------------------------------------------------


class TestRegisterInternalDeferredPublish:
    def test_register_internal_invisible_during_on_load(self) -> None:
        reg = Registry()
        module = _ProbeOnLoadModule(reg, "system.probe")
        reg.register_internal("system.probe", module)
        assert module.visible_during_on_load is False
        # And after on_load completed, the module IS visible.
        assert reg.get("system.probe") is module

    def test_register_internal_on_load_failure_leaves_registry_unchanged(self) -> None:
        reg = Registry()
        before_ids = set(reg.list())
        with pytest.raises(RuntimeError, match="intentional on_load failure"):
            reg.register_internal("system.fail", _FailingOnLoadModule())
        # Registry must be exactly as before — no partial state leak.
        assert reg.get("system.fail") is None
        assert set(reg.list()) == before_ids
        assert "system.fail" not in reg._in_flight


# ---------------------------------------------------------------------------
# A-D-REG-005 — Discover-path emits module_load_failed on on_load failure
# ---------------------------------------------------------------------------


class TestDiscoverPathEmitsModuleLoadFailed:
    def test_discover_path_emits_module_load_failed(self) -> None:
        reg = Registry()
        emitter = MagicMock()
        reg.set_event_emitter(emitter)

        cls = type("_FailingFactory", (_FailingOnLoadModule,), {})
        reg._register_in_order(
            load_order=["system.fail"],
            valid_classes={"system.fail": cls},
            raw_metadata={},
        )

        # The emitter MUST have received an apcore.registry.module_load_failed
        # event for the failing module — the discover path is no longer
        # silent about on_load failures.
        emitted_types = [call.args[0].event_type for call in emitter.emit.call_args_list]
        assert (
            "apcore.registry.module_load_failed" in emitted_types
        ), f"Expected module_load_failed event from discover path, got: {emitted_types}"

    def test_register_internal_emits_module_load_failed(self) -> None:
        # Sanity: register_internal's load-failed event was already covered
        # by Issue #65 for the public register() path; confirm that route
        # holds for register_internal too after A-D-REG-004.
        reg = Registry()
        emitter = MagicMock()
        reg.set_event_emitter(emitter)

        with pytest.raises(RuntimeError):
            reg.register_internal("system.fail", _FailingOnLoadModule())

        emitted_types = [call.args[0].event_type for call in emitter.emit.call_args_list]
        assert "apcore.registry.module_load_failed" in emitted_types
