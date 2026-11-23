"""Spec-traced contract tests for the Context Object feature.

Source spec: apcore/docs/features/context-object.md
Contract under test: ``ContextKey[T]`` (the only ``## Contract:`` block in the spec).

Each test's docstring carries the verbatim clause id formatted as
``context_object.<method>.<kind>.<detail>`` so cross-language diffs line up.

framework: pytest + pytest-asyncio (asyncio_mode=auto). Property tests that the
spec marks ``async: false`` stay synchronous; the thread_safe clause exercises
concurrency via ``asyncio.gather`` per the shared contract-spec rules.

TESTS ONLY — no production source is modified here.
"""

from __future__ import annotations

import asyncio

import pytest

from apcore.context import Context
from apcore.context_key import ContextKey


def _make_ctx() -> Context:
    """Create a minimal Context exposing a ``data`` map."""
    return Context.create()


# ---------------------------------------------------------------------------
# Inputs — build inputs that exercise the declared parameter rules.
#
# The contract declares NO ``reject_with`` rules and NO errors: "set, get,
# exists, delete, and scoped do not raise." So an "invalid"-style input must
# NOT raise; instead we assert the declared graceful fallback behavior.
# ---------------------------------------------------------------------------


def test_input_get_missing_key_returns_default_no_raise() -> None:
    """context_object.get.input.default.absent_key_returns_default_not_raise."""
    key: ContextKey[int] = ContextKey("ext.spec.absent")
    ctx = _make_ctx()
    # Spec Errors: get reports a missing key by returning the default rather
    # than raising. Default supplied -> returns the default exactly.
    assert key.get(ctx, default=7) == 7
    # No default supplied -> returns None (not a raise).
    assert key.get(ctx) is None


def test_input_delete_absent_key_is_noop_no_raise() -> None:
    """context_object.delete.input.absent.delete_absent_is_noop_no_raise."""
    key: ContextKey[int] = ContextKey("ext.spec.never_set")
    ctx = _make_ctx()
    # Spec Errors: "delete is a no-op on an absent key." Must not raise.
    key.delete(ctx)
    assert key.exists(ctx) is False


def test_input_scoped_suffix_appends_dotted_segment() -> None:
    """context_object.scoped.input.suffix.appends_dotted_segment."""
    base: ContextKey[int] = ContextKey("ext.spec.retry")
    scoped = base.scoped("mod.a")
    # Spec Returns: scoped(suffix) -> a new key named "{name}.{suffix}".
    assert scoped.name == "ext.spec.retry.mod.a"
    assert scoped is not base


# ---------------------------------------------------------------------------
# Errors — the contract declares NONE. We assert the absence of raises across
# the full surface so a future regression that adds a raise is caught.
# ---------------------------------------------------------------------------


def test_error_none_no_method_raises_on_normal_use() -> None:
    """context_object.contextkey.error.none.no_method_raises."""
    key: ContextKey[str] = ContextKey("ext.spec.noerr")
    ctx = _make_ctx()
    # set / get / exists / delete / scoped must all complete without raising.
    key.set(ctx, "v")
    assert key.get(ctx) == "v"
    assert key.exists(ctx) is True
    assert key.scoped("s").name == "ext.spec.noerr.s"
    key.delete(ctx)
    assert key.exists(ctx) is False
    # Second delete (now-absent) still must not raise.
    key.delete(ctx)


# ---------------------------------------------------------------------------
# Returns — assert the exact declared return shapes.
# ---------------------------------------------------------------------------


def test_returns_set_returns_none() -> None:
    """context_object.set.returns.none.set_yields_no_value."""
    key: ContextKey[int] = ContextKey("ext.spec.ret_set")
    ctx = _make_ctx()
    assert key.set(ctx, 1) is None


def test_returns_get_present_yields_stored_value() -> None:
    """context_object.get.returns.value.present_returns_stored."""
    key: ContextKey[int] = ContextKey("ext.spec.ret_get")
    ctx = _make_ctx()
    key.set(ctx, 123)
    assert key.get(ctx) == 123


def test_returns_get_default_narrows_to_default_when_absent() -> None:
    """context_object.get.returns.default.absent_returns_default."""
    key: ContextKey[int] = ContextKey("ext.spec.ret_get_def")
    ctx = _make_ctx()
    assert key.get(ctx, default=0) == 0


def test_returns_exists_is_bool() -> None:
    """context_object.exists.returns.bool.true_iff_present."""
    key: ContextKey[int] = ContextKey("ext.spec.ret_exists")
    ctx = _make_ctx()
    assert key.exists(ctx) is False
    key.set(ctx, 9)
    assert key.exists(ctx) is True


def test_returns_delete_removes_and_returns_none() -> None:
    """context_object.delete.returns.none.removes_name_from_data."""
    key: ContextKey[int] = ContextKey("ext.spec.ret_delete")
    ctx = _make_ctx()
    key.set(ctx, 5)
    assert key.delete(ctx) is None
    assert "ext.spec.ret_delete" not in ctx.data


def test_returns_scoped_yields_new_contextkey() -> None:
    """context_object.scoped.returns.key.new_contextkey_named_name_dot_suffix."""
    base: ContextKey[int] = ContextKey("ext.spec.ret_scoped")
    child = base.scoped("x")
    assert isinstance(child, ContextKey)
    assert child.name == "ext.spec.ret_scoped.x"


# ---------------------------------------------------------------------------
# Properties.
# ---------------------------------------------------------------------------


def test_property_async_false_methods_are_synchronous() -> None:
    """context_object.contextkey.property.async.all_methods_synchronous."""
    key: ContextKey[int] = ContextKey("ext.spec.async")
    ctx = _make_ctx()
    # Spec Properties: "async: false — all methods are synchronous."
    # Results are plain values, never awaitables/coroutines.
    assert not asyncio.iscoroutine(key.set(ctx, 1))
    got = key.get(ctx)
    assert not asyncio.iscoroutine(got)
    assert got == 1
    assert not asyncio.iscoroutine(key.exists(ctx))
    assert not asyncio.iscoroutine(key.scoped("s"))


async def test_property_thread_safe_concurrent_distinct_keys() -> None:
    """context_object.contextkey.property.thread_safe.concurrent_distinct_keys."""
    ctx = _make_ctx()
    n = 16

    async def writer(i: int) -> int:
        key: ContextKey[int] = ContextKey(f"ext.spec.concurrent.{i}")
        key.set(ctx, i)
        return key.get(ctx, default=-1)

    results = await asyncio.gather(*(writer(i) for i in range(n)))
    # No exception raised, and every distinct key landed its own value.
    assert results == list(range(n))
    for i in range(n):
        key: ContextKey[int] = ContextKey(f"ext.spec.concurrent.{i}")
        assert key.get(ctx) == i


def test_property_idempotent_set_same_value() -> None:
    """context_object.set.property.idempotent.repeat_same_value_same_state."""
    key: ContextKey[int] = ContextKey("ext.spec.idem_set")
    ctx = _make_ctx()
    key.set(ctx, 42)
    first = dict(ctx.data)
    key.set(ctx, 42)
    second = dict(ctx.data)
    assert first == second
    assert key.get(ctx) == 42


def test_property_idempotent_delete() -> None:
    """context_object.delete.property.idempotent.repeat_delete_same_state."""
    key: ContextKey[int] = ContextKey("ext.spec.idem_delete")
    ctx = _make_ctx()
    key.set(ctx, 1)
    key.delete(ctx)
    after_first = dict(ctx.data)
    key.delete(ctx)
    after_second = dict(ctx.data)
    assert after_first == after_second
    assert key.exists(ctx) is False


def test_property_idempotent_exists_and_get_are_read_only() -> None:
    """context_object.exists.property.idempotent.repeat_query_same_state."""
    key: ContextKey[int] = ContextKey("ext.spec.idem_query")
    ctx = _make_ctx()
    key.set(ctx, 5)
    snapshot = dict(ctx.data)
    assert key.exists(ctx) is True
    assert key.exists(ctx) is True
    assert key.get(ctx) == 5
    assert key.get(ctx) == 5
    # Repeated queries observed no state change.
    assert dict(ctx.data) == snapshot


def test_property_pure_get_does_not_mutate_data() -> None:
    """context_object.get.property.pure.read_only_no_self_mutation."""
    key: ContextKey[int] = ContextKey("ext.spec.pure_get")
    ctx = _make_ctx()
    key.set(ctx, 3)
    snapshot = dict(ctx.data)
    _ = key.get(ctx)
    _ = key.get(ctx, default=99)
    # Spec Properties: "get and exists are side-effect-free (read-only)."
    assert dict(ctx.data) == snapshot


def test_property_pure_scoped_does_not_mutate_receiver() -> None:
    """context_object.scoped.property.pure.allocates_new_key_no_mutation."""
    base: ContextKey[int] = ContextKey("ext.spec.pure_scoped")
    base_name_before = base.name
    child = base.scoped("leaf")
    # Spec Properties: "scoped is pure and allocates a new key" / "never
    # mutates the receiver."
    assert base.name == base_name_before
    assert child.name == "ext.spec.pure_scoped.leaf"
    assert child is not base


def test_property_immutable_key_is_frozen() -> None:
    """context_object.contextkey.property.immutable_key.name_is_readonly."""
    key: ContextKey[int] = ContextKey("ext.spec.frozen")
    # Spec: "A key is immutable — Python uses a frozen dataclass."
    with pytest.raises(AttributeError):
        key.name = "changed"  # type: ignore[misc]


# ---------------------------------------------------------------------------
# Side Effects — set / delete mutate context.data in place; observe ordering
# via the public data map. (The contract has no numbered "### Side Effects"
# block, but Properties declare set/delete as the only mutating operations, so
# we trace their observable in-place effect and ordering here.)
# ---------------------------------------------------------------------------


def test_side_effect_set_then_delete_observed_in_order() -> None:
    """context_object.contextkey.side_effect.1.set_then_delete_ordered."""
    key: ContextKey[str] = ContextKey("ext.spec.effect")
    ctx = _make_ctx()
    observed: list[bool] = []
    observed.append(key.exists(ctx))  # absent before set
    key.set(ctx, "v")
    observed.append(key.exists(ctx))  # present after set
    key.delete(ctx)
    observed.append(key.exists(ctx))  # absent after delete
    assert observed == [False, True, False]


# ---------------------------------------------------------------------------
# Namespace Convention (Normative) — the contract's MUST rules. These are
# observable structurally: identifier strings must round-trip verbatim into
# context.data (one shared namespace with raw string keys), so a raw read sees
# what a ContextKey wrote and vice versa.
# ---------------------------------------------------------------------------


def test_namespace_ext_key_shares_one_namespace_with_raw_keys() -> None:
    """context_object.contextkey.namespace.ext.shared_with_raw_string_keys."""
    key: ContextKey[int] = ContextKey("ext.my_company.retry.count")
    ctx = _make_ctx()
    key.set(ctx, 4)
    # Two views of one dict: raw access sees the ContextKey-written value.
    assert ctx.data["ext.my_company.retry.count"] == 4
    # And a raw write is visible through the typed key.
    ctx.data["ext.my_company.retry.count"] = 9
    assert key.get(ctx) == 9


def test_namespace_apcore_prefix_collides_two_views_one_dict() -> None:
    """context_object.contextkey.namespace.apcore_prefix.collides_with_raw."""
    # Spec: ContextKey("_apcore.foo") and context.data["_apcore.foo"] collide.
    key: ContextKey[int] = ContextKey("_apcore.foo")
    ctx = _make_ctx()
    ctx.data["_apcore.foo"] = 1
    assert key.exists(ctx) is True
    assert key.get(ctx) == 1
