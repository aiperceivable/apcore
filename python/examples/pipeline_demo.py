"""Demonstrate the 11-step ExecutionStrategy pipeline.

This example surfaces the engine that powers every apcore call:

  1. Introspection      — print the 11 default step names.
  2. Middleware tracing — register a StepMiddleware that logs entry,
                          exit, and per-step duration.
  3. Orchestration      — insert_after() adds a custom AuditLogStep,
                          then replace() swaps it for a quieter one.

Run:
    python examples/pipeline_demo.py
"""

from __future__ import annotations

import time

from apcore import APCore
from apcore.pipeline import (
    BaseStep,
    ExecutionStrategy,
    PipelineContext,
    PipelineState,
    StepResult,
)


# The canonical 11-step pipeline defined by the apcore protocol spec.
# Anything not in this set is a user-inserted custom step.
CANONICAL_STEPS: tuple[str, ...] = (
    "context_creation",
    "call_chain_guard",
    "module_lookup",
    "acl_check",
    "approval_gate",
    "middleware_before",
    "input_validation",
    "execute",
    "output_validation",
    "middleware_after",
    "return_result",
)

# Per-step roles, used by TracingMiddleware to narrate the pipeline.
STEP_ROLES: dict[str, str] = {
    "context_creation": "create execution context, set global deadline",
    "call_chain_guard": "check call depth & repeat limits",
    "module_lookup": "resolve module from registry",
    "acl_check": "enforce access control (default-deny)",
    "approval_gate": "human approval gate (if required)",
    "middleware_before": "run before-middleware chain (in order)",
    "input_validation": "validate inputs against schema",
    "execute": "invoke the module",
    "output_validation": "validate output against schema",
    "middleware_after": "run after-middleware chain (reverse order)",
    "return_result": "finalize and return output",
}


# ── Section 2: a StepMiddleware that traces each step ────────────────────
class TracingMiddleware:
    """Narrate each pipeline step: index, role, duration, key state change."""

    def __init__(self, strategy: ExecutionStrategy) -> None:
        self._strategy = strategy  # kept for symmetry; counts come from CANONICAL_STEPS
        self._starts: dict[str, float] = {}
        self._core_idx = 0  # only counts the 11 canonical steps; resets each call

    def before_step(self, step_name: str, state: PipelineState) -> None:
        if step_name == "context_creation":
            self._core_idx = 0  # reset at the top of every new call
        self._starts[step_name] = time.perf_counter()
        if step_name in CANONICAL_STEPS:
            self._core_idx += 1
            label = f"[{self._core_idx:>2}/11]"
            role = STEP_ROLES[step_name]
        else:
            label = "[  +  ]"
            role = "CUSTOM step inserted via insert_after / replace"
        print(f"  {label} {step_name:<19} — {role}")

    def after_step(self, step_name: str, state: PipelineState, result: StepResult) -> None:
        elapsed_ms = (time.perf_counter() - self._starts.pop(step_name, time.perf_counter())) * 1000
        detail = self._summarize(step_name, state.context, result)
        print(f"          ✓ {elapsed_ms:>6.2f} ms · {detail}")

    def on_step_error(self, step_name: str, state: PipelineState, error: Exception) -> None:
        print(f"          ✗ {type(error).__name__}: {error}")
        # Returning None lets the original exception propagate.

    @staticmethod
    def _summarize(step_name: str, ctx: PipelineContext, result: StepResult) -> str:
        if step_name == "context_creation":
            cid = getattr(ctx.context, "caller_id", None) or "anonymous"
            tid = getattr(ctx.context, "trace_id", "")[:8]
            return f"caller={cid} trace_id={tid}…"
        if step_name == "module_lookup" and ctx.module is not None:
            return f"resolved module '{ctx.module_id}'"
        if step_name == "middleware_before":
            return f"inputs={ctx.inputs}"
        if step_name == "input_validation" and ctx.validated_inputs is not None:
            return f"validated_inputs={ctx.validated_inputs}"
        if step_name == "execute" and ctx.output is not None:
            return f"output={ctx.output}"
        if step_name == "output_validation" and ctx.validated_output is not None:
            return f"validated_output={ctx.validated_output}"
        if step_name == "return_result":
            final = ctx.validated_output if ctx.validated_output is not None else ctx.output
            return f"returning {final}"
        return result.explanation or result.action


# ── Section 3: a custom step inserted after output_validation ───────────
class AuditLogStep(BaseStep):
    """Append a one-line audit record to stdout for every successful call."""

    def __init__(self) -> None:
        super().__init__(
            name="audit_log",
            description="Emit an audit record after output validation.",
        )

    async def execute(self, ctx: PipelineContext) -> StepResult:
        caller_id = getattr(ctx.context, "caller_id", None) or "anonymous"
        print(f"    [audit] caller={caller_id} target={ctx.module_id} ok=true")
        return StepResult(action="continue")


class QuietAuditLogStep(BaseStep):
    """Replacement step: same name + position, no stdout side effect."""

    def __init__(self) -> None:
        super().__init__(
            name="audit_log",
            description="Quiet audit step (replacement demo).",
        )

    async def execute(self, ctx: PipelineContext) -> StepResult:
        return StepResult(action="continue", explanation="quiet audit recorded")


def banner(title: str) -> None:
    print(f"\n=== {title} ===")


def main() -> None:
    client = APCore()

    @client.module(id="math.add", description="Add two integers")
    def add(a: int, b: int) -> int:
        return a + b

    # The Executor builds the standard 11-step strategy by default.
    # Reaching into Executor internals — this is an advanced demo of the
    # pipeline surface; production code should prefer a public accessor.
    strategy = client.executor._strategy

    # ── Section 1: Introspection ────────────────────────────────────────
    banner("Section 1: Introspection — the default 11-step pipeline")
    info = strategy.info()
    print(f"strategy: {info.name}  (steps: {info.step_count})")
    for i, name in enumerate(strategy.step_names(), start=1):
        print(f"  {i:>2}. {name}")

    # ── Section 2: Middleware tracing ──────────────────────────────────
    banner("Section 2: Middleware tracing — one call through 11 steps")
    strategy.add_step_middleware(TracingMiddleware(strategy))
    result = client.call("math.add", {"a": 10, "b": 5})
    print(f"result: {result}")

    # ── Section 3: Orchestration ───────────────────────────────────────
    banner("Section 3: Orchestration — insert_after + replace")
    strategy.insert_after("output_validation", AuditLogStep())
    custom = [n for n in strategy.step_names() if n not in CANONICAL_STEPS]
    print(f"after insert_after: 11 standard + {len(custom)} custom = " f"{len(strategy.steps)} steps")
    for i, name in enumerate(strategy.step_names(), start=1):
        marker = "  ← CUSTOM (inserted)" if name not in CANONICAL_STEPS else ""
        print(f"  {i:>2}. {name}{marker}")

    print("\ncalling with the inserted audit step:")
    client.call("math.add", {"a": 2, "b": 3})

    strategy.replace("audit_log", QuietAuditLogStep())
    print(
        f"\nafter replace: {len(strategy.steps)} steps "
        f"(audit_log still at index {strategy.step_names().index('audit_log')})"
    )

    print("\ncalling with the quiet replacement:")
    client.call("math.add", {"a": 7, "b": 9})


if __name__ == "__main__":
    main()
