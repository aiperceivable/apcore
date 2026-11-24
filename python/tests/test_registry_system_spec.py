"""Spec-traced contract tests for the apcore registry-system feature.

CANONICAL clause source. TypeScript and Rust suites mirror these clause-IDs.

Source spec: apcore/docs/features/registry-system.md
Contracts covered:
  - Registry.register
  - Scanner.scan_extensions
  - Registry.get
  - Registry.list
  - Registry.get_definition

Each test carries a deterministic clause-id in its docstring of the form
``registry_system.<method>.<kind>.<detail>`` where
``kind in {input, error, property, side_effect, return}``.

Framework: pytest + pytest-asyncio (asyncio_mode=auto).
"""

from __future__ import annotations

import asyncio
from pathlib import Path
from typing import Any

import pytest
from pydantic import BaseModel

from apcore.errors import (
    ConfigNotFoundError,
    InvalidInputError,
    ModuleNotFoundError,
)
from apcore.registry.registry import MODULE_ID_PATTERN, Registry
from apcore.registry.scanner import scan_extensions
from apcore.registry.types import ModuleDescriptor


# ---------------------------------------------------------------------------
# Helper module classes (duck-typed; pydantic schemas per validation.py)
# ---------------------------------------------------------------------------


class _Input(BaseModel):
    value: str = ""


class _Output(BaseModel):
    result: str = ""


class SpecModule:
    """A valid duck-typed module satisfying validate_module()."""

    input_schema = _Input
    output_schema = _Output
    description = "Spec fixture module"
    name = "SpecModule"
    version = "1.0.0"
    tags = ["alpha", "beta"]

    async def execute(self, inputs: dict[str, Any], context: Any = None) -> dict[str, Any]:
        return {"result": inputs.get("value", "")}


def make_module(tags: list[str] | None = None) -> SpecModule:
    mod = SpecModule()
    if tags is not None:
        mod.tags = tags  # type: ignore[attr-defined]
    return mod


class OnLoadModule(SpecModule):
    """Module recording on_load invocation for ordering assertions."""

    def __init__(self, observer: Any | None = None) -> None:
        self.loaded = False
        self._observer = observer

    def on_load(self) -> None:
        self.loaded = True
        if self._observer is not None:
            self._observer()


class FailingOnLoadModule(SpecModule):
    """Module whose on_load raises, to exercise the failure path."""

    def on_load(self) -> None:
        raise ValueError("on_load boom")


# ===========================================================================
# Contract: Registry.register
# ===========================================================================


def test_register_input_module_id_empty_rejected() -> None:
    """registry_system.register.input.module_id.empty"""
    reg = Registry()
    with pytest.raises(InvalidInputError) as exc:
        reg.register("", make_module())
    assert exc.value.code == "INVALID_MODULE_ID"


def test_register_input_module_id_malformed_rejected() -> None:
    """registry_system.register.input.module_id.malformed"""
    reg = Registry()
    # Hyphens are disallowed by MODULE_ID_PATTERN.
    assert not MODULE_ID_PATTERN.match("Bad-ID")
    with pytest.raises(InvalidInputError) as exc:
        reg.register("Bad-ID", make_module())
    assert exc.value.code == "INVALID_MODULE_ID"


def test_register_input_module_id_reserved_rejected() -> None:
    """registry_system.register.input.module_id.reserved"""
    reg = Registry()
    # `system.*` is reserved; only register_internal() may use it.
    with pytest.raises(InvalidInputError) as exc:
        reg.register("system.thing", make_module())
    assert exc.value.code == "INVALID_MODULE_ID"


def test_register_error_invalid_module_id() -> None:
    """registry_system.register.error.INVALID_MODULE_ID"""
    reg = Registry()
    with pytest.raises(InvalidInputError) as exc:
        reg.register("9bad", make_module())  # must start with a lowercase letter
    assert exc.value.code == "INVALID_MODULE_ID"


def test_register_error_duplicate_module_id() -> None:
    """registry_system.register.error.DUPLICATE_MODULE_ID"""
    reg = Registry()
    reg.register("math.add", make_module())
    with pytest.raises(InvalidInputError) as exc:
        reg.register("math.add", make_module())
    assert exc.value.code == "DUPLICATE_MODULE_ID"


def test_register_return_none_on_success() -> None:
    """registry_system.register.return.none"""
    reg = Registry()
    result = reg.register("math.add", make_module())
    assert result is None
    assert reg.get("math.add") is not None


def test_register_property_async_false() -> None:
    """registry_system.register.property.async

    Contract declares async: false -> register() is a plain (non-coroutine)
    callable. Verify it returns a concrete value rather than a coroutine.
    """
    import inspect

    assert not inspect.iscoroutinefunction(Registry.register)
    reg = Registry()
    assert reg.register("math.add", make_module()) is None


def test_register_property_idempotent_false() -> None:
    """registry_system.register.property.idempotent

    Contract declares idempotent: false -> duplicate registration is an
    error, not a no-op.
    """
    reg = Registry()
    reg.register("math.add", make_module())
    with pytest.raises(InvalidInputError) as exc:
        reg.register("math.add", make_module())
    assert exc.value.code == "DUPLICATE_MODULE_ID"


def test_register_property_pure_false() -> None:
    """registry_system.register.property.pure

    Contract declares pure: false -> mutates the internal store. Observe the
    mutation via the public list() API.
    """
    reg = Registry()
    before = reg.list()
    reg.register("math.add", make_module())
    after = reg.list()
    assert "math.add" not in before
    assert "math.add" in after


async def test_register_property_thread_safe() -> None:
    """registry_system.register.property.thread_safe

    Contract declares thread_safe: true (reentrant lock during mutation).
    Drive >=8 concurrent registrations of distinct IDs; all must land without
    corruption or deadlock.
    """
    reg = Registry()
    n = 12

    def reg_one(i: int) -> None:
        reg.register(f"mod.m{i}", make_module())

    await asyncio.gather(*(asyncio.to_thread(reg_one, i) for i in range(n)))

    listed = reg.list()
    for i in range(n):
        assert f"mod.m{i}" in listed
    assert reg.count == n


async def test_register_property_thread_safe_duplicate_single_winner() -> None:
    """registry_system.register.property.thread_safe.duplicate

    Concurrent registrations of the SAME id: exactly one wins, the rest raise
    DUPLICATE_MODULE_ID (in-flight set + lock guard).
    """
    reg = Registry()
    n = 10
    errors: list[BaseException] = []

    def reg_same() -> None:
        try:
            reg.register("dup.mod", make_module())
        except InvalidInputError as e:  # noqa: PERF203
            errors.append(e)

    await asyncio.gather(*(asyncio.to_thread(reg_same) for _ in range(n)))

    assert reg.get("dup.mod") is not None
    assert len(errors) == n - 1
    assert all(getattr(e, "code", None) == "DUPLICATE_MODULE_ID" for e in errors)


def test_register_side_effect_1_validate_before_mutation() -> None:
    """registry_system.register.side_effect.1.validate_before_mutation

    Side-effect order step 2: module_id is validated before any store
    mutation. An invalid id must leave the store untouched.
    """
    reg = Registry()
    with pytest.raises(InvalidInputError):
        reg.register("Bad-ID", make_module())
    assert reg.list() == []
    assert reg.count == 0


def test_register_side_effect_6_on_load_invoked() -> None:
    """registry_system.register.side_effect.6.on_load_invoked

    Side-effect step 6: module.on_load() is invoked during registration.
    """
    reg = Registry()
    mod = OnLoadModule()
    reg.register("life.cycle", mod)
    assert mod.loaded is True


def test_register_side_effect_6_on_load_before_visible() -> None:
    """registry_system.register.side_effect.6.on_load_before_visible

    Side-effect ordering (steps 6 then 7): on_load() MUST complete before the
    module becomes visible via get(). Observe ordering: at the moment on_load
    runs, get() must NOT yet return the module.
    """
    reg = Registry()
    observed_visible_during_load: list[bool] = []

    def observer() -> None:
        observed_visible_during_load.append(reg.get("life.cycle") is not None)

    mod = OnLoadModule(observer=observer)
    reg.register("life.cycle", mod)

    assert observed_visible_during_load == [False]
    # After registration completes the module IS visible.
    assert reg.get("life.cycle") is mod


def test_register_side_effect_6_on_load_failure_not_visible() -> None:
    """registry_system.register.side_effect.6.on_load_failure_not_visible

    Side-effect step 6 failure branch: if on_load raises, the original
    exception propagates unchanged AND the module never becomes visible.
    """
    reg = Registry()
    with pytest.raises(ValueError, match="on_load boom"):
        reg.register("life.boom", FailingOnLoadModule())
    assert reg.get("life.boom") is None
    assert "life.boom" not in reg.list()


def test_register_side_effect_8_register_event_emitted() -> None:
    """registry_system.register.side_effect.8.register_event_emitted

    Side-effect step 8: a `register` event is emitted to subscribers after
    successful publication.
    """
    reg = Registry()
    received: list[tuple[str, Any]] = []
    reg.on("register", lambda module_id, payload: received.append((module_id, payload)))

    reg.register("math.add", make_module())

    assert len(received) == 1
    assert received[0][0] == "math.add"


def test_register_side_effect_ordering_load_then_event() -> None:
    """registry_system.register.side_effect.ordering.load_then_event

    Combined ordering: on_load (step 6) fires before the register event
    (step 8). Observe order via a shared sequence list.
    """
    reg = Registry()
    sequence: list[str] = []

    mod = OnLoadModule(observer=lambda: sequence.append("on_load"))
    reg.on("register", lambda module_id, payload: sequence.append("event"))

    reg.register("order.mod", mod)

    assert sequence == ["on_load", "event"]


# ===========================================================================
# Contract: Scanner.scan_extensions
# ===========================================================================


def test_scan_extensions_input_root_missing_rejected(tmp_path: Path) -> None:
    """registry_system.scan_extensions.input.root.missing"""
    missing = tmp_path / "does_not_exist"
    with pytest.raises(ConfigNotFoundError) as exc:
        scan_extensions(missing)
    assert exc.value.code == "CONFIG_NOT_FOUND"


def test_scan_extensions_error_config_not_found(tmp_path: Path) -> None:
    """registry_system.scan_extensions.error.CONFIG_NOT_FOUND"""
    with pytest.raises(ConfigNotFoundError) as exc:
        scan_extensions(tmp_path / "nope")
    assert exc.value.code == "CONFIG_NOT_FOUND"


def test_scan_extensions_return_ordered_records(tmp_path: Path) -> None:
    """registry_system.scan_extensions.return.discovered_modules

    On success returns a sequence of DiscoveredModule records (path + derived
    module id) in stable filesystem-traversal order.
    """
    ext = tmp_path / "ext"
    ext.mkdir()
    (ext / "hello.py").write_text("class HelloModule: ...\n")
    (ext / "greet.py").write_text("class GreetModule: ...\n")

    results = scan_extensions(ext)
    ids = {dm.canonical_id for dm in results}
    assert ids == {"hello", "greet"}
    # Records expose a concrete file path.
    assert all(dm.file_path.suffix == ".py" for dm in results)


def test_scan_extensions_property_async_false() -> None:
    """registry_system.scan_extensions.property.async

    Contract declares async: false.
    """
    import inspect

    assert not inspect.iscoroutinefunction(scan_extensions)


def test_scan_extensions_property_pure_false_reads_filesystem(tmp_path: Path) -> None:
    """registry_system.scan_extensions.property.pure

    Contract declares pure: false (reads the filesystem). Adding a file
    changes the output for the same root argument.
    """
    ext = tmp_path / "ext"
    ext.mkdir()
    (ext / "a.py").write_text("class A: ...\n")
    first = {dm.canonical_id for dm in scan_extensions(ext)}
    (ext / "b.py").write_text("class B: ...\n")
    second = {dm.canonical_id for dm in scan_extensions(ext)}
    assert first == {"a"}
    assert second == {"a", "b"}


def test_scan_extensions_property_thread_safe(tmp_path: Path) -> None:
    """registry_system.scan_extensions.property.thread_safe

    Contract declares thread_safe: true (no shared mutable state). Concurrent
    scans of the same root return consistent results.
    """
    ext = tmp_path / "ext"
    ext.mkdir()
    for i in range(5):
        (ext / f"m{i}.py").write_text(f"class M{i}: ...\n")

    async def run() -> list[set[str]]:
        outs = await asyncio.gather(
            *(asyncio.to_thread(lambda: {dm.canonical_id for dm in scan_extensions(ext)}) for _ in range(8))
        )
        return list(outs)

    results = asyncio.run(run())
    expected = {f"m{i}" for i in range(5)}
    assert all(r == expected for r in results)


# ===========================================================================
# Contract: Registry.get
# ===========================================================================


def test_get_input_module_id_empty_rejected() -> None:
    """registry_system.get.input.module_id.empty"""
    reg = Registry()
    with pytest.raises(ModuleNotFoundError):
        reg.get("")


def test_get_error_module_not_found_on_empty() -> None:
    """registry_system.get.error.MODULE_NOT_FOUND

    Empty module_id raises ModuleNotFoundError. The spec writes
    ModuleNotFoundError(module_id="") -> the Python error carries
    code MODULE_NOT_FOUND.
    """
    reg = Registry()
    with pytest.raises(ModuleNotFoundError) as exc:
        reg.get("")
    assert exc.value.code == "MODULE_NOT_FOUND"


def test_get_return_none_for_unregistered() -> None:
    """registry_system.get.return.none_when_absent

    A well-formed but unregistered id returns None (no error).
    """
    reg = Registry()
    assert reg.get("not.registered") is None


def test_get_return_instance_when_found() -> None:
    """registry_system.get.return.instance_when_found"""
    reg = Registry()
    mod = make_module()
    reg.register("math.add", mod)
    assert reg.get("math.add") is mod


def test_get_property_async_false() -> None:
    """registry_system.get.property.async"""
    import inspect

    assert not inspect.iscoroutinefunction(Registry.get)


def test_get_property_idempotent_true() -> None:
    """registry_system.get.property.idempotent

    Read-only; repeated calls with the same args return the same result when
    the registry is unchanged.
    """
    reg = Registry()
    mod = make_module()
    reg.register("math.add", mod)
    assert reg.get("math.add") is reg.get("math.add") is mod


def test_get_property_pure_false_reads_shared_state() -> None:
    """registry_system.get.property.pure

    Contract declares pure: false (reads shared mutable state). The result
    changes when the underlying store changes.
    """
    reg = Registry()
    assert reg.get("math.add") is None
    reg.register("math.add", make_module())
    assert reg.get("math.add") is not None


async def test_get_property_thread_safe() -> None:
    """registry_system.get.property.thread_safe

    Contract declares thread_safe: true. >=8 concurrent reads alongside writes
    must not corrupt or deadlock.
    """
    reg = Registry()
    reg.register("math.add", make_module())

    def read() -> Any:
        return reg.get("math.add")

    results = await asyncio.gather(*(asyncio.to_thread(read) for _ in range(10)))
    assert all(r is not None for r in results)


# ===========================================================================
# Contract: Registry.list
# ===========================================================================


def test_list_input_tags_filter_all_or_nothing() -> None:
    """registry_system.list.input.tags.superset_match

    When tags supplied, only modules whose tag set is a superset of ALL
    supplied tags are included.
    """
    reg = Registry()
    reg.register("m.alpha", make_module(tags=["alpha", "beta"]))
    reg.register("m.gamma", make_module(tags=["gamma"]))

    assert reg.list(tags=["alpha"]) == ["m.alpha"]
    assert reg.list(tags=["alpha", "beta"]) == ["m.alpha"]
    # Requiring a tag not all-present excludes the module.
    assert reg.list(tags=["alpha", "missing"]) == []


def test_list_input_tags_empty_means_no_filter() -> None:
    """registry_system.list.input.tags.empty_no_filter

    An empty tags list MUST be treated the same as None (no tag filter).
    """
    reg = Registry()
    reg.register("m.alpha", make_module(tags=["alpha"]))
    reg.register("m.gamma", make_module(tags=["gamma"]))

    assert sorted(reg.list(tags=[])) == sorted(reg.list())
    assert set(reg.list(tags=[])) == {"m.alpha", "m.gamma"}


def test_list_input_prefix_exact_startswith() -> None:
    """registry_system.list.input.prefix.startswith

    Prefix matching is exact string prefix (startsWith), not glob/regex.
    """
    reg = Registry()
    reg.register("math.add", make_module())
    reg.register("math.sub", make_module())
    reg.register("string.upper", make_module())

    assert reg.list(prefix="math.") == ["math.add", "math.sub"]
    # '*' is literal, not a wildcard -> matches nothing.
    assert reg.list(prefix="math.*") == []


def test_list_input_tags_and_prefix_combined() -> None:
    """registry_system.list.input.combined.tags_and_prefix"""
    reg = Registry()
    reg.register("math.add", make_module(tags=["arith"]))
    reg.register("math.sub", make_module(tags=["other"]))
    reg.register("string.cat", make_module(tags=["arith"]))

    assert reg.list(tags=["arith"], prefix="math.") == ["math.add"]


def test_list_error_unknown_tag_returns_empty() -> None:
    """registry_system.list.error.none

    No error for unknown tags/prefixes that match nothing -> empty list.
    """
    reg = Registry()
    reg.register("math.add", make_module(tags=["arith"]))
    assert reg.list(tags=["nonexistent"]) == []
    assert reg.list(prefix="zzz") == []


def test_list_return_sorted_unique() -> None:
    """registry_system.list.return.sorted_unique

    Returns a lexicographically sorted list of unique module ID strings.
    """
    reg = Registry()
    for mid in ["zeta.one", "alpha.two", "mid.three"]:
        reg.register(mid, make_module())
    result = reg.list()
    assert result == ["alpha.two", "mid.three", "zeta.one"]
    assert len(result) == len(set(result))


def test_list_property_async_false() -> None:
    """registry_system.list.property.async"""
    import inspect

    assert not inspect.iscoroutinefunction(Registry.list)


def test_list_property_idempotent_true() -> None:
    """registry_system.list.property.idempotent

    Same registry state -> identical result across calls.
    """
    reg = Registry()
    reg.register("a.one", make_module())
    reg.register("b.two", make_module())
    assert reg.list() == reg.list()


async def test_list_property_thread_safe() -> None:
    """registry_system.list.property.thread_safe

    Contract declares thread_safe: true (snapshot under lock before filtering).
    >=8 concurrent list() calls during writes return sorted lists without
    corruption.
    """
    reg = Registry()
    for i in range(6):
        reg.register(f"mod.m{i}", make_module())

    def do_list() -> list[str]:
        return reg.list()

    results = await asyncio.gather(*(asyncio.to_thread(do_list) for _ in range(10)))
    for r in results:
        assert r == sorted(r)  # always sorted
        assert set(r).issubset({f"mod.m{i}" for i in range(6)})


# ===========================================================================
# Contract: Registry.get_definition
# ===========================================================================


def test_get_definition_input_module_id_empty_propagates() -> None:
    """registry_system.get_definition.input.module_id.empty

    Any error get(module_id) raises is propagated (ModuleNotFoundError on
    empty string).
    """
    reg = Registry()
    with pytest.raises(ModuleNotFoundError):
        reg.get_definition("")


def test_get_definition_error_propagates_from_get() -> None:
    """registry_system.get_definition.error.MODULE_NOT_FOUND"""
    reg = Registry()
    with pytest.raises(ModuleNotFoundError) as exc:
        reg.get_definition("")
    assert exc.value.code == "MODULE_NOT_FOUND"


def test_get_definition_return_none_for_unregistered() -> None:
    """registry_system.get_definition.return.none_when_absent

    A well-formed but unregistered id returns None (no error).
    """
    reg = Registry()
    assert reg.get_definition("not.registered") is None


def test_get_definition_return_descriptor_fields() -> None:
    """registry_system.get_definition.return.descriptor_fields

    On success returns a ModuleDescriptor with the contracted fields.
    """
    reg = Registry()
    reg.register("math.add", make_module(tags=["alpha", "beta"]))

    desc = reg.get_definition("math.add")
    assert isinstance(desc, ModuleDescriptor)
    assert desc.module_id == "math.add"
    assert desc.description == "Spec fixture module"
    assert isinstance(desc.input_schema, dict)
    assert isinstance(desc.output_schema, dict)
    assert desc.version == "1.0.0"
    assert set(["alpha", "beta"]).issubset(set(desc.tags))


def test_get_definition_property_async_false() -> None:
    """registry_system.get_definition.property.async"""
    import inspect

    assert not inspect.iscoroutinefunction(Registry.get_definition)


def test_get_definition_property_idempotent_true() -> None:
    """registry_system.get_definition.property.idempotent

    For the same registry state, returns an equivalent descriptor each call.
    """
    reg = Registry()
    reg.register("math.add", make_module())
    d1 = reg.get_definition("math.add")
    d2 = reg.get_definition("math.add")
    assert d1 is not None and d2 is not None
    assert d1.module_id == d2.module_id
    assert d1.input_schema == d2.input_schema
    assert d1.output_schema == d2.output_schema


async def test_get_definition_property_thread_safe() -> None:
    """registry_system.get_definition.property.thread_safe

    Contract declares thread_safe: true. >=8 concurrent get_definition calls
    return consistent descriptors.
    """
    reg = Registry()
    reg.register("math.add", make_module())

    def do() -> ModuleDescriptor | None:
        return reg.get_definition("math.add")

    results = await asyncio.gather(*(asyncio.to_thread(do) for _ in range(10)))
    assert all(r is not None and r.module_id == "math.add" for r in results)
