"""D-112 — a failed reload restores the previous module.

apcore-typescript re-registered the original on failure; this SDK and
apcore-rust left it unregistered, and the contract endorsed that ("callers must
handle the partial state"). For a control plane that is the wrong default: a
failed hot-fix should not make a WORKING module disappear.

The decision is stated in four parts because the mechanism cannot deliver more
than this, and each is asserted separately below:

1. Restoration is COMPENSATING, not transactional. The module is genuinely
   unregistered for a window; an implementation MUST NOT claim atomic
   replacement.
2. The restore MUST re-run the restored module's ``on_load``. Its ``on_unload``
   already ran during the unregister, so re-publishing without it yields a
   module that is visible but torn down — harder to diagnose than one that is
   absent.
3. If that restoring load ALSO fails, the module MAY remain unavailable.
4. On the bulk path, restoration is PER MODULE. The operation still fails, and
   modules that already reloaded are NOT rolled back.
"""

from __future__ import annotations

from typing import Any

import pytest

from apcore.errors import ReloadFailedError
from apcore.events import EventEmitter
from apcore.registry.registry import Registry
from apcore.sys_modules.control import ReloadModule


class _HookRecordingModule:
    """Records its lifecycle hooks so the restore path can be observed."""

    input_schema: dict[str, Any] = {"type": "object", "properties": {}}
    output_schema: dict[str, Any] = {"type": "object", "properties": {}}
    description = "records lifecycle hooks"
    version = "1.0.0"

    def __init__(self, tag: str, calls: list[str], fail_on_load: bool = False) -> None:
        self.tag = tag
        self._calls = calls
        self._fail_on_load = fail_on_load

    def on_load(self) -> None:
        self._calls.append(f"{self.tag}:on_load")
        if self._fail_on_load:
            raise RuntimeError(f"{self.tag} refuses to load")

    def on_unload(self) -> None:
        self._calls.append(f"{self.tag}:on_unload")

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        return {"tag": self.tag}


def _reload_module(registry: Registry) -> ReloadModule:
    return ReloadModule(registry=registry, event_emitter=EventEmitter())


class TestFailedReloadRestoresThePreviousModule:
    def test_the_original_instance_is_back_after_a_failed_rediscovery(self) -> None:
        calls: list[str] = []
        original = _HookRecordingModule("original", calls)
        registry = Registry()
        registry.register_internal("executor.probe", original)

        mod = _reload_module(registry)

        def _explode(module_id: str) -> Any:
            raise RuntimeError("discovery is broken")

        mod._rediscover_module = _explode  # type: ignore[method-assign]

        with pytest.raises(ReloadFailedError):
            mod.execute({"module_id": "executor.probe", "reason": "hot-fix"}, None)

        restored = registry.get("executor.probe")
        assert restored is original, "a failed hot-fix must not make a working module disappear"

    def test_the_restore_re_runs_on_load(self) -> None:
        """Rule 2. ``on_unload`` already ran during the unregister, so a restore
        that skips ``on_load`` republishes a module that is visible but torn
        down — harder to diagnose than one that is absent."""
        calls: list[str] = []
        original = _HookRecordingModule("original", calls)
        registry = Registry()
        registry.register_internal("executor.probe", original)
        calls.clear()

        mod = _reload_module(registry)
        mod._rediscover_module = lambda module_id: (_ for _ in ()).throw(RuntimeError("boom"))  # type: ignore[method-assign]

        with pytest.raises(ReloadFailedError):
            mod.execute({"module_id": "executor.probe", "reason": "hot-fix"}, None)

        assert calls == ["original:on_unload", "original:on_load"], calls

    def test_rule_3_if_the_restoring_load_also_fails_the_module_stays_unavailable(self) -> None:
        """There is no good state left to return to, and publishing a module
        whose load hook failed is the defect deferred publication prevents.

        The caller is still told about the ORIGINAL failure, not the restore's.
        """
        calls: list[str] = []
        original = _HookRecordingModule("original", calls)
        registry = Registry()
        registry.register_internal("executor.probe", original)

        # The module loads once (at registration) and refuses the second time,
        # which is exactly the "no good state left" situation rule 3 names.
        original._fail_on_load = True

        mod = _reload_module(registry)
        mod._rediscover_module = lambda module_id: (_ for _ in ()).throw(RuntimeError("boom"))  # type: ignore[method-assign]

        with pytest.raises(ReloadFailedError) as excinfo:
            mod.execute({"module_id": "executor.probe", "reason": "hot-fix"}, None)

        assert "boom" in str(excinfo.value), "the ORIGINAL failure is the one reported"
        assert registry.get("executor.probe") is None

    def test_control_a_successful_reload_does_not_restore_the_old_instance(self) -> None:
        """Without this, "the original is registered afterwards" is also
        satisfied by an implementation that never swaps anything in."""
        calls: list[str] = []
        original = _HookRecordingModule("original", calls)
        replacement = _HookRecordingModule("replacement", calls)
        registry = Registry()
        registry.register_internal("executor.probe", original)

        mod = _reload_module(registry)

        def _rediscover(module_id: str) -> Any:
            registry.register_internal(module_id, replacement)
            return replacement

        mod._rediscover_module = _rediscover  # type: ignore[method-assign]

        mod.execute({"module_id": "executor.probe", "reason": "hot-fix"}, None)

        assert registry.get("executor.probe") is replacement

    def test_rule_4_bulk_restores_per_module_and_keeps_the_ones_that_succeeded(self) -> None:
        """The operation as a whole still fails, but a module that already
        reloaded is NOT rolled back — cross-module transactionality is not a
        primitive the registry has."""
        calls: list[str] = []
        first = _HookRecordingModule("first", calls)
        second = _HookRecordingModule("second", calls)
        replacement = _HookRecordingModule("replacement", calls)
        registry = Registry()
        registry.register_internal("executor.first", first)
        registry.register_internal("executor.second", second)

        mod = _reload_module(registry)

        def _rediscover(module_id: str) -> Any:
            if module_id == "executor.second":
                raise RuntimeError("the second one is broken")
            registry.register_internal(module_id, replacement)
            return replacement

        mod._rediscover_module = _rediscover  # type: ignore[method-assign]

        with pytest.raises(ReloadFailedError):
            mod.execute({"path_filter": "executor.*", "reason": "bulk deploy"}, None)

        # The failing module is back as its ORIGINAL instance...
        assert registry.get("executor.second") is second
        # ...and the one that already reloaded keeps its NEW instance.
        assert registry.get("executor.first") is replacement
