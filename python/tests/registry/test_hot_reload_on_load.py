"""A hot-reloaded instance MUST run ``on_load()`` before it becomes visible (D-123).

``Registry._handle_file_change`` used to write the freshly imported instance
straight into ``_modules`` / ``_versioned_modules`` / ``_module_meta`` without
ever calling its ``on_load()``. Every other registration path in this SDK runs
that hook first and publishes only on success — ``discover()`` does it in its
Phase 2/3 split, and ``register()`` is bound by ``Contract: Registry.register``
Side Effects step 8. That contract is scoped to ``register``, and the watch path
writes the internal maps directly, so it broke no stated rule while publishing a
module that was visible, callable, and never initialised.

That is strictly worse than a module that is simply absent: an uninitialised
instance fails at call time, inside whatever the hook was supposed to have set
up, with a stack trace that points nowhere near the reload that caused it.

D-123 applies D-112 rules 2-4 to this path:

1. the new instance runs ``on_load()`` BEFORE it is published;
2. if that fails, the previous instance is restored and its ``on_load()`` is
   re-run (Phase 3 already ran its ``on_unload``, so republishing it without a
   fresh load would leave a visible-but-torn-down module -- the exact state
   rule 2 exists to prevent);
3. if the restoring load fails too, the module stays unavailable.
"""

from __future__ import annotations

import textwrap
from pathlib import Path
from typing import Any

from pydantic import BaseModel

from apcore.registry.registry import Registry

MODULE_ID = "executor.reload_probe"

_PROBE_SOURCE = textwrap.dedent(
    '''
    """Probe module used by the D-123 hot-reload on_load regression test."""

    from pathlib import Path
    from typing import Any

    from pydantic import BaseModel

    _MARKER = Path({marker!r})
    _VERSION = "{version}"


    class _Input(BaseModel):
        value: int = 0


    class _Output(BaseModel):
        value: int = 0


    class ReloadProbeModule:
        input_schema = _Input
        output_schema = _Output
        description = "on_load probe {version}"
        version = "{version}"

        def on_load(self) -> None:
            with _MARKER.open("a", encoding="utf-8") as fh:
                fh.write("on_load:" + _VERSION + "\\n")
            {on_load_tail}

        def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
            return {{"value": inputs.get("value", 0)}}
    '''
)


def _write_probe(path: Path, *, marker: Path, version: str, fails: bool) -> None:
    """Write a probe revision whose ``on_load`` records itself, then maybe raises."""
    tail = 'raise RuntimeError("on_load refused revision " + _VERSION)' if fails else "return None"
    path.write_text(
        _PROBE_SOURCE.format(marker=str(marker), version=version, on_load_tail=tail),
        encoding="utf-8",
    )


def _calls(marker: Path) -> list[str]:
    if not marker.exists():
        return []
    return marker.read_text(encoding="utf-8").split()


class _Input(BaseModel):
    value: int = 0


class _Output(BaseModel):
    value: int = 0


class OldInstance:
    """The already-registered module a reload replaces.

    Defined here rather than imported from the probe file so the test controls
    exactly which of its ``on_load`` calls succeed: ``fail_from_call=2`` lets the
    initial ``register()`` succeed and the D-112 rule 3 restore fail.
    """

    input_schema = _Input
    output_schema = _Output
    description = "old instance"
    version = "1.5.0"

    def __init__(self, marker: Path, *, fail_from_call: int | None = None) -> None:
        self._marker = marker
        self._fail_from_call = fail_from_call
        self.load_calls = 0

    def on_load(self) -> None:
        self.load_calls += 1
        with self._marker.open("a", encoding="utf-8") as fh:
            fh.write(f"on_load:old#{self.load_calls}\n")
        if self._fail_from_call is not None and self.load_calls >= self._fail_from_call:
            raise RuntimeError("old instance refused to re-load")

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        return {"value": inputs.get("value", 0)}


class _CapturingEmitter:
    def __init__(self) -> None:
        self.events: list[Any] = []

    def emit(self, event: Any) -> None:
        self.events.append(event)


class TestHotReloadRunsOnLoad:
    def test_reloaded_instance_runs_on_load(self, tmp_path: Path) -> None:
        """The successful path: the new revision's ``on_load`` actually runs.

        Before D-123 the marker file stayed empty for the reloaded revision --
        the instance was published without its hook ever being entered.
        """
        marker = tmp_path / "calls.log"
        probe_path = tmp_path / "reload_probe.py"
        _write_probe(probe_path, marker=marker, version="2.0.0", fails=False)

        registry = Registry()
        old = OldInstance(marker)
        registry.register(MODULE_ID, old)
        assert _calls(marker) == ["on_load:old#1"]

        registry._handle_file_change(str(probe_path))

        assert _calls(marker) == ["on_load:old#1", "on_load:2.0.0"]
        definition = registry.get_definition(MODULE_ID)
        assert definition is not None
        assert definition.version == "2.0.0"

    def test_failed_on_load_keeps_the_new_instance_invisible(self, tmp_path: Path) -> None:
        """D-123 rule 1: a revision whose ``on_load`` raises is never published.

        This is the ordering proof. ``on_load`` running *before* the publish is
        not directly observable, but its consequence is: a failing load leaves
        the broken revision entirely absent from every store. Before the fix the
        2.0.0 descriptor was already visible by the time anything noticed.
        """
        marker = tmp_path / "calls.log"
        probe_path = tmp_path / "reload_probe.py"
        _write_probe(probe_path, marker=marker, version="2.0.0", fails=True)

        registry = Registry()
        old = OldInstance(marker)
        registry.register(MODULE_ID, old)

        registry._handle_file_change(str(probe_path))

        definition = registry.get_definition(MODULE_ID)
        assert definition is not None, "rule 2: the previous instance must be restored"
        assert definition.version == "1.5.0", "the broken 2.0.0 revision must not be visible"
        assert registry.get(MODULE_ID) is old

    def test_failed_on_load_reloads_the_restored_instance(self, tmp_path: Path) -> None:
        """D-123 rule 2: the restored instance gets a fresh ``on_load``.

        Phase 3 already ran the old instance's ``on_unload`` before the new
        revision was even imported, so putting it back without re-loading it
        would republish a torn-down module.
        """
        marker = tmp_path / "calls.log"
        probe_path = tmp_path / "reload_probe.py"
        _write_probe(probe_path, marker=marker, version="2.0.0", fails=True)

        registry = Registry()
        old = OldInstance(marker)
        registry.register(MODULE_ID, old)

        registry._handle_file_change(str(probe_path))

        assert old.load_calls == 2
        # The failing revision records its entry before raising, so the order
        # also pins that the restore happens after the failed load, not instead
        # of attempting it.
        assert _calls(marker) == ["on_load:old#1", "on_load:2.0.0", "on_load:old#2"]

    def test_module_stays_unavailable_when_the_restore_also_fails(self, tmp_path: Path) -> None:
        """D-123 rule 3: with no good state left, the module stays gone.

        Publishing a module whose load hook failed is the defect the whole
        deferred-publish rule exists to prevent; it applies to the old instance
        just as much as to the new one.
        """
        marker = tmp_path / "calls.log"
        probe_path = tmp_path / "reload_probe.py"
        _write_probe(probe_path, marker=marker, version="2.0.0", fails=True)

        registry = Registry()
        old = OldInstance(marker, fail_from_call=2)
        registry.register(MODULE_ID, old)

        registry._handle_file_change(str(probe_path))

        assert old.load_calls == 2, "the restore must still be attempted"
        assert registry.has(MODULE_ID) is False
        assert registry.get(MODULE_ID) is None
        assert registry.get_definition(MODULE_ID) is None

    def test_failed_on_load_emits_module_load_failed(self, tmp_path: Path) -> None:
        """The watch path emits the same DLQ-style signal as every other path.

        ``_invoke_on_load`` owns this, so routing the watch path through it is
        what gives subscribers one hook for partial-init detection regardless of
        which registration path failed (A-D-REG-005).
        """
        marker = tmp_path / "calls.log"
        probe_path = tmp_path / "reload_probe.py"
        _write_probe(probe_path, marker=marker, version="2.0.0", fails=True)

        registry = Registry()
        emitter = _CapturingEmitter()
        registry.set_event_emitter(emitter)
        registry.register(MODULE_ID, OldInstance(marker))

        registry._handle_file_change(str(probe_path))

        failures = [e for e in emitter.events if e.event_type == "apcore.registry.module_load_failed"]
        assert len(failures) == 1
        assert failures[0].module_id == MODULE_ID
        assert failures[0].data["callback_name"] == "on_load"
        assert failures[0].data["error_type"] == "RuntimeError"
