"""D-83 (spec v1.49.0) — the published `guard_call_chain` signature is normative.

The decision was written about apcore-rust, whose shipped guard took a
`Context<T>` and a depth: the spec's own Rust tab published code that did not
compile, and a host whose services type was anything else could not call the
guard at all, so nested calls ran with no depth, cycle or frequency enforcement.
`DEFAULT_MAX_CALL_DEPTH` was a `pub const` the guard never consulted.

apcore-python is one of the two SDKs that already published the chain-taking
form, which is why the decision names the spec and reachability as its authority
rather than any implementation — and why this SDK had no test for it.

These pin the two things the decision is actually about:

  * REACHABILITY — the guard and both limit constants are importable from the
    PACKAGE ROOT, the way a host reaches them. A deep import
    (``apcore.utils.call_chain``) would pass while ``from apcore import ...``
    failed, which is the shape D-83 forbids.
  * The published PARAMETERS are consulted — the chain, the depth limit and the
    repeat limit each change the outcome on their own, and the documented
    defaults are the enforced ones.
"""

from __future__ import annotations

import pytest


def test_the_published_signature_is_reachable_from_the_package_root() -> None:
    import apcore

    assert "guard_call_chain" in apcore.__all__
    assert "DEFAULT_MAX_CALL_DEPTH" in apcore.__all__
    assert "DEFAULT_MAX_MODULE_REPEAT" in apcore.__all__

    from apcore import guard_call_chain

    # Keyword-callable with exactly the published parameter names: a host
    # copying the spec's signature must not have to rename anything.
    guard_call_chain(
        module_id="c",
        call_chain=["a", "b"],
        max_call_depth=32,
        max_module_repeat=3,
    )


def test_the_call_chain_argument_is_genuinely_consulted() -> None:
    """Two chains, one module_id, two outcomes."""
    from apcore import DEFAULT_MAX_CALL_DEPTH, DEFAULT_MAX_MODULE_REPEAT, guard_call_chain
    from apcore.errors import CircularCallError

    guard_call_chain(
        "c",
        ["a", "b", "c"],
        max_call_depth=DEFAULT_MAX_CALL_DEPTH,
        max_module_repeat=DEFAULT_MAX_MODULE_REPEAT,
    )

    with pytest.raises(CircularCallError):
        guard_call_chain(
            "c",
            ["c", "b", "c"],
            max_call_depth=DEFAULT_MAX_CALL_DEPTH,
            max_module_repeat=DEFAULT_MAX_MODULE_REPEAT,
        )


def test_the_documented_defaults_are_the_enforced_ones() -> None:
    """The limit a host reads MUST be the limit the guard applies.

    apcore-rust's ``DEFAULT_MAX_CALL_DEPTH`` was a public constant nothing
    consulted — a declared surface reaching no mechanism, the same shape as an
    inert configuration key.
    """
    from apcore import DEFAULT_MAX_CALL_DEPTH, guard_call_chain
    from apcore.errors import CallDepthExceededError

    assert DEFAULT_MAX_CALL_DEPTH == 32

    # A20 rejects on `len(chain) > max_call_depth`, so the limit itself is
    # allowed and the first rejected length is limit + 1. Both sides are
    # asserted: a guard that was off by one, or that ignored the constant
    # entirely, fails one of them.
    at_limit = [f"mod.{i}" for i in range(DEFAULT_MAX_CALL_DEPTH)]
    guard_call_chain("mod.last", at_limit)

    over_limit = [f"mod.{i}" for i in range(DEFAULT_MAX_CALL_DEPTH + 1)]
    with pytest.raises(CallDepthExceededError):
        guard_call_chain("mod.last", over_limit)


def test_the_depth_and_repeat_limits_are_separate_parameters() -> None:
    """Each published limit must change the outcome on its own.

    Without this, a guard that ignored ``max_module_repeat`` entirely — or
    conflated it with the depth — passes every test above.
    """
    from apcore import guard_call_chain
    from apcore.errors import CallDepthExceededError, CallFrequencyExceededError

    # A chain that repeats a module without being deep AND without forming a
    # cycle: A20 checks depth, then circularity, then frequency, so the
    # repeated module must sit at the END of the prior chain or the circular
    # check fires first and the frequency limit is never reached. Three
    # entries against a depth limit of 32 leaves only the repeat limit able to
    # reject it.
    repeating = ["a", "a", "a"]
    guard_call_chain("a", repeating, max_call_depth=32, max_module_repeat=3)
    with pytest.raises(CallFrequencyExceededError):
        guard_call_chain("a", repeating, max_call_depth=32, max_module_repeat=2)

    # A long, non-repeating chain: the repeat limit is generous, so only the
    # depth limit can reject it.
    deep = [f"mod.{i}" for i in range(10)]
    guard_call_chain("mod.next", deep, max_call_depth=32, max_module_repeat=3)
    with pytest.raises(CallDepthExceededError):
        guard_call_chain("mod.next", deep, max_call_depth=5, max_module_repeat=3)
