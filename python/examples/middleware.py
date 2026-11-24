"""User-facing before/after middleware — end-to-end demo.

apcore is a standalone product: this example shows the middleware hooks an SDK
*user* writes — `client.use_before(...)` and `client.use_after(...)` — which wrap
the whole module call (distinct from the internal pipeline *step* middleware
shown in `pipeline_demo.py`). It

  1. registers a real `executor.math.add` tool,
  2. installs a before-hook that augments inputs (injects a default `b`) and an
     after-hook that transforms output (adds a `verified` flag), with both hooks
     appending to a shared ordered trace,
  3. calls the tool through `client.call(...)`, and
  4. prints the original vs. middleware-transformed result and the trace.

The before-hook returns a replacement inputs dict (or None to pass through); the
after-hook returns a replacement output dict (same contract). See the feature
doc `docs/features/middleware-system.md` (§"Contract: Middleware.before/after").

Each expectation is asserted, so this script doubles as a smoke test: it exits
non-zero if the hook order or the transformed output drifts.

Run (from the apcore-python repo root):
    python examples/middleware.py
"""

from __future__ import annotations

import sys
from typing import Any

from apcore import APCore, Context

# A single ordered trace the tool and both hooks append to. Proving it ends as
# ["before", "execute", "after"] is the whole point: the before-hook ran before
# module execution and the after-hook ran after it.
trace: list[str] = []

# ---------------------------------------------------------------------------
# 1. Register a real tool.
# ---------------------------------------------------------------------------
client = APCore()


@client.module(id="executor.math.add", description="Add two integers")
def add(a: int, b: int) -> dict:
    trace.append("execute")
    return {"result": a + b}


# ---------------------------------------------------------------------------
# 2. User-facing before/after middleware.
# ---------------------------------------------------------------------------
DEFAULT_B = 22


def record_before(module_id: str, inputs: dict[str, Any], _ctx: Context) -> dict[str, Any]:
    """Observe + augment inputs: inject a default `b` when the caller omits it."""
    trace.append("before")
    if "b" not in inputs:
        return {**inputs, "b": DEFAULT_B}  # replacement inputs
    return inputs


def record_after(
    _module_id: str, _inputs: dict[str, Any], output: dict[str, Any], _ctx: Context
) -> dict[str, Any]:
    """Observe + transform output: stamp it as verified by middleware."""
    trace.append("after")
    return {**output, "verified": True}  # replacement output


client.use_before(record_before)
client.use_after(record_after)


# ---------------------------------------------------------------------------
# 3. Call the tool through the pipeline and report.
# ---------------------------------------------------------------------------
def main() -> int:
    print("User-facing before/after middleware — end-to-end demo")
    print("Tool: executor.math.add — before injects default b, after stamps verified")
    print("=" * 72)

    # Call with `b` omitted so the before-hook's injection is observable.
    result = client.call("executor.math.add", {"a": 20})

    print("  inputs (as called)   : {'a': 20}  (b omitted)")
    print(f"  before injected      : b = {DEFAULT_B}")
    print(f"  raw add result       : {{'result': {20 + DEFAULT_B}}}")
    print(f"  after-transformed    : {result}")
    print(f"  hook/execute trace   : {trace}")
    print("=" * 72)

    expected_trace = ["before", "execute", "after"]
    expected_output = {"result": 42, "verified": True}

    checks = [
        ("trace order is before -> execute -> after", trace == expected_trace),
        ("before-hook injected default b", result["result"] == 42),
        ("after-hook stamped verified=True", result.get("verified") is True),
        ("transformed output matches", result == expected_output),
    ]

    failures = 0
    for label, ok in checks:
        failures += not ok
        print(f"  {'PASS' if ok else 'FAIL'}  {label}")
        if not ok:
            print(f"        ^ trace={trace} output={result}")

    print("=" * 72)
    total = len(checks)
    print(f"{total - failures}/{total} checks passed.")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
