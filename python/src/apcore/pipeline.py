"""Execution pipeline types for configurable step-based module invocation."""

from __future__ import annotations

import asyncio
import inspect
import logging
import time
from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from enum import Enum
from collections.abc import Sequence
from typing import Any, Callable, Protocol, runtime_checkable

from apcore.errors import ModuleError, ModuleTimeoutError
from apcore.middleware.manager import MiddlewareChainError
from apcore.utils.pattern import match_pattern

_logger = logging.getLogger(__name__)


def _any_match(patterns: tuple[str, ...], module_id: str) -> bool:
    """Return True if any glob pattern matches the module_id (Algorithm A09)."""
    return any(match_pattern(p, module_id) for p in patterns)


@runtime_checkable
class Step(Protocol):
    """Protocol for pipeline steps."""

    @property
    def name(self) -> str: ...

    @property
    def description(self) -> str: ...

    @property
    def removable(self) -> bool: ...

    @property
    def replaceable(self) -> bool: ...

    async def execute(self, ctx: PipelineContext) -> StepResult: ...


class BaseStep(ABC):
    """Convenience base class for pipeline steps."""

    def __init__(
        self,
        name: str,
        description: str = "",
        *,
        removable: bool = True,
        replaceable: bool = True,
        match_modules: tuple[str, ...] | None = None,
        ignore_errors: bool = False,
        pure: bool = False,
        timeout_ms: int = 0,
        requires: tuple[str, ...] = (),
        provides: tuple[str, ...] = (),
    ) -> None:
        self.name = name
        self.description = description
        self.removable = removable
        self.replaceable = replaceable
        self.match_modules = match_modules
        self.ignore_errors = ignore_errors
        self.pure = pure
        self.timeout_ms = timeout_ms
        self.requires = requires
        self.provides = provides

    @abstractmethod
    async def execute(self, ctx: PipelineContext) -> StepResult: ...


@dataclass
class StepResult:
    """Result returned by a pipeline step execution."""

    action: str  # "continue", "skip_to", "abort"
    skip_to: str | None = None
    explanation: str | None = None
    confidence: float | None = None
    alternatives: list[str] | None = None


@dataclass
class PipelineContext:
    """Holds all state flowing through the pipeline."""

    module_id: str
    inputs: dict[str, Any]
    context: Any  # Context object
    module: Any | None = None
    validated_inputs: dict[str, Any] | None = None
    output: dict[str, Any] | None = None
    validated_output: dict[str, Any] | None = None
    stream: bool = False
    output_stream: Any | None = None
    strategy: ExecutionStrategy | None = None
    trace: PipelineTrace | None = None
    # New in v0.17
    dry_run: bool = False
    preflight_errors: dict[str, dict[str, Any]] = field(default_factory=dict)
    version_hint: str | None = None
    executed_middlewares: list[Any] = field(default_factory=list)
    # New in v0.20
    run_until: Callable[[PipelineState], bool] | None = None
    # PROTOCOL_SPEC §6.1.6 / §7.4: the second half of Step 4's result. The ACL
    # answers two independent questions, and Step 5's approval gate fires on the
    # union of this, the module annotation and gate_destructive (§6.9 rows 3-5).
    # False when no ACL is attached, when no rule matched, or when the attached
    # ACL is an older object with no structured accessor.
    acl_approval_required: bool = False
    #: The registry's declared (descriptor) annotations for this module,
    #: resolved by Step 3 (``module_lookup``) alongside ``module``.
    #:
    #: The second governance source PROTOCOL_SPEC §7.4 (D-96) unions with the
    #: live instance. It travels on the context for the same reason
    #: ``acl_approval_required`` does: the step that can compute it is not the
    #: step that needs it, and the approval gate has no registry of its own.
    #: ``None`` when the module was never merged (``register_internal``) or the
    #: lookup was bypassed.
    declared_annotations: Any | None = None
    # PROTOCOL_SPEC §6.1.8: the governance projection of THIS call's arguments.
    # Computed by Step 3 (module lookup) and handed to Step 4's ACL check.
    # CTX-2: it lives here, on the per-call pipeline state, and NOT on the
    # execution Context — that object is handed to module code and outlives the
    # check, so a projection left on it decided a later, unrelated
    # `check_access` against the previous call's argument key set. Framework-
    # computed and never accepted from caller input (rule 3).
    governance_projection: Any | None = None


@dataclass
class StepTrace:
    """Records execution details for a single step."""

    name: str
    duration_ms: float
    result: StepResult
    skipped: bool = False
    decision_point: bool = False
    skip_reason: str | None = None


@dataclass
class PipelineTrace:
    """Records execution details for the entire pipeline run."""

    module_id: str
    strategy_name: str
    steps: list[StepTrace] = field(default_factory=list)
    total_duration_ms: float = 0.0
    success: bool = False


@dataclass
class PipelineState:
    """Snapshot passed to run_until predicates and to StepMiddleware hooks.

    ``outputs`` maps step name to that step's output. In a **StepMiddleware
    hook** it contains exactly the steps that completed *before*
    ``step_name`` — the current step is never present, in any of the three
    hooks: ``before_step`` has not run it, ``on_step_error`` has no output to
    record, and in ``after_step`` the current step's output is the ``result``
    argument. One rule with one meaning, so a middleware never has to know
    which hook it is in before reading the map
    (middleware-system.md § "What ``state.outputs`` contains").

    A ``run_until`` predicate is the exception, and deliberately so: it is
    evaluated *after* the step it judges, so ``outputs`` there does include
    ``step_name``.

    ``outputs`` is a **live reference** to the engine's map, not a copy — the
    same aliasing apcore-typescript has. Read it (or copy it) inside the hook:
    a middleware that stashes the state object and inspects it after the run
    sees the final map, in which every step is present.
    """

    step_name: str
    outputs: dict[str, Any]
    context: PipelineContext


@runtime_checkable
class StepMiddleware(Protocol):
    """Protocol for pipeline step middlewares (Issue #33 §2.2).

    Step middlewares wrap individual pipeline steps the same way module
    middlewares wrap whole module calls.  All three hooks are optional and
    each method may be either synchronous or ``async``.

    Hook semantics (middleware-system.md § "Pipeline Step Middleware", pinned
    by ``conformance/fixtures/pipeline_step_middleware.json``):

    * ``before_step(step_name, state)`` — invoked just before the step runs, in
      **registration order**.  It is an *observation* hook: a Step is
      ``execute(ctx)`` and takes no ``inputs`` argument, so the return value
      carries no meaning and MUST NOT change what the step body sees.  Input
      rewriting is the module-level ``Middleware.before`` contract.
      An exception raised here is wrapped in :class:`MiddlewareChainError`,
      stops the chain (the step body does NOT run), and triggers
      ``on_step_error`` on only the middlewares whose ``before_step`` already
      executed.  **That failure is terminal** — see below.
    * ``after_step(step_name, state, result)`` — invoked after a successful
      step **and after a recovered one**, in **reverse registration order**
      (onion model).  A recovered step produced an output and the pipeline
      continued, so the onion MUST close or the recovery path leaks whatever
      ``before_step`` acquired.  It is NOT invoked when the step failed
      unrecovered, and NOT invoked after a ``before_step`` failure.
    * ``on_step_error(step_name, state, error)`` — invoked when a step raises,
      in **reverse registration order**.  The FIRST handler returning a
      non-``None`` value supplies the recovery result and short-circuits the
      remaining handlers (first-recovery-wins); the original error does not
      propagate.  Returning ``None`` from every handler lets the original
      exception propagate (subject to the step's ``ignore_errors``).
      A handler that itself raises is logged and iteration continues.

    **``state.outputs`` never contains the current step**, in any of the three
    hooks — it holds exactly the steps that completed before ``step_name``.
    ``before_step`` has not run it; ``on_step_error`` has no output to record;
    and in ``after_step`` the current step's output is the ``result``
    argument.  The alternative reading, "outputs of completed steps", would
    make ``after_step`` the one hook whose ``outputs`` differs in shape, so a
    middleware would have to know which hook it was in before reading the map.

    **A ``before_step`` failure terminates the step — it is not recoverable.**
    A ``before_step`` failure and a step-body failure are categorically
    different and MUST NOT share a recovery path.  On the ``before_step`` path
    ``on_step_error`` still runs, on the already-entered middlewares, in
    reverse order — but for *observation and cleanup only*: any value it
    returns is **discarded**, ``after_step`` does not fire, and the step's
    ``ignore_errors`` does not apply.  ``MiddlewareChainError`` propagates
    regardless.  Honouring the recovery would advance the pipeline past a step
    whose body never ran, and ``acl_check`` / ``approval_gate`` sit in the
    built-in strategy, so it is a silent authorization bypass reachable from an
    extension point that carries no authority.

    Both sync and async returns are supported: the engine awaits any return
    value that ``inspect.isawaitable()`` reports as awaitable, mirroring the
    pattern used by Issue #42's async ``on_error`` fix.
    """

    def before_step(self, step_name: str, state: "PipelineState") -> Any: ...

    def after_step(self, step_name: str, state: "PipelineState", result: "StepResult") -> Any: ...

    def on_step_error(self, step_name: str, state: "PipelineState", error: Exception) -> Any: ...


@dataclass
class StrategyInfo:
    """AI-introspectable description of an execution strategy."""

    name: str
    step_count: int
    step_names: list[str]
    description: str

    def __str__(self) -> str:
        return f"{self.step_count}-step pipeline: " + " \u2192 ".join(self.step_names)


@dataclass(frozen=True)
class GovernanceState:
    """What is actually gating an executor's registry (PROTOCOL_SPEC 6.6.5).

    Eight plain observations plus one derived flag. ``acl is not None`` is not
    the answer to "what is gating this registry": the ACL and approval gates are
    pipeline *steps*, and the ``internal``, ``testing`` and ``minimal``
    strategies all remove them -- so an executor can hold an ACL that no step
    ever consults.

    Returned by :meth:`apcore.Executor.governance_state`. Pure data: reading it
    enforces nothing and changes nothing.
    """

    control_modules_registered: bool
    """At least one ``system.control.*`` module is in the registry."""

    read_modules_registered: bool
    """At least one read-only ``system.*`` module is in the registry."""

    acl_configured: bool
    """An ACL object is attached to the executor."""

    builtin_acl_gate_wired: bool
    """The running strategy contains the built-in ACL gate, matched by TYPE."""

    approval_handler_configured: bool
    """An ``ApprovalHandler`` is attached."""

    builtin_approval_gate_wired: bool
    """The running strategy contains the built-in approval gate, matched by TYPE."""

    policy_strict: bool
    """An ``ExecutionPolicy`` with ``strict=True`` is attached."""

    all_control_modules_require_approval: bool
    """Every registered ``system.control.*`` module declares ``requires_approval``.

    Required by the derived flag because the two gates are not symmetric
    (PROTOCOL_SPEC 6.6.5.1.1): ``acl_check`` evaluates every call, but
    ``approval_gate`` resolves per module and returns before consulting the
    handler when the module does not need approval. ``False`` when no control
    module is registered.
    """

    unprotected_control_surface: bool
    """Control modules are registered and no recognised built-in gate engages.

    Reports the **absence of a gate**, never the presence of protection: a wired
    ACL that permits every call still yields ``False``. And ``True`` does not
    mean the call will succeed -- a custom step, custom middleware or an
    upstream gateway is invisible here by construction.
    """


class ExecutionStrategy:
    """An ordered sequence of steps that defines how a module is executed."""

    def __init__(
        self,
        name: str,
        steps: Sequence[Step],
        *,
        validate_dependencies: bool = True,
    ) -> None:
        """Construct an :class:`ExecutionStrategy`.

        Args:
            name: A human-readable name for the strategy (used in traces).
            steps: The ordered sequence of :class:`Step` instances.
            validate_dependencies: When ``True`` (default), a step whose
                ``requires`` keys are not provided by a preceding step's
                ``provides`` triggers a :class:`PipelineDependencyError`
                (Issue #33 §2.1). Internal callers that assemble derived
                strategies from a parent strategy already known to be
                consistent (e.g. the streaming executor splitting validation
                steps into a post-stream sub-strategy) may pass ``False`` to
                skip this check.
        """
        self.name = name
        self.steps = list(steps)
        # StepMiddlewares attached to this strategy (Issue #33 §2.2).
        # Lazily mutated via ``add_step_middleware``.
        self.step_middlewares: list[StepMiddleware] = []
        # Validate unique step names
        names = [s.name for s in self.steps]
        if len(names) != len(set(names)):
            dupes = [n for n in names if names.count(n) > 1]
            raise StepNameDuplicateError(f"Duplicate step names: {set(dupes)}")
        self._rebuild_index()
        self._validate_dependencies_enabled = validate_dependencies
        if validate_dependencies:
            self._validate_dependencies()

    def add_step_middleware(self, middleware: StepMiddleware) -> None:
        """Register a :class:`StepMiddleware` for this strategy.

        Middlewares are invoked in registration order for ``before_step``, and
        in reverse registration order for ``after_step`` and ``on_step_error``
        (onion semantics).
        """
        self.step_middlewares.append(middleware)

    def _rebuild_index(self) -> None:
        """Rebuild the O(1) name→index map. Called after any mutation."""
        self._name_to_idx: dict[str, int] = {s.name: i for i, s in enumerate(self.steps)}

    def _validate_dependencies(self) -> None:
        """Raise :class:`PipelineDependencyError` if a step's ``requires`` are
        not satisfied by any preceding step's ``provides`` (Issue #33 §2.1).

        This is fail-fast: a misconfigured strategy must not silently produce
        runtime errors deep inside step execution.
        """
        provided: set[str] = set()
        for step in self.steps:
            requires: tuple[str, ...] = getattr(step, "requires", ())
            missing = set(requires) - provided
            if missing:
                raise PipelineDependencyError(
                    step_name=step.name,
                    missing=tuple(sorted(missing)),
                )
            provides: tuple[str, ...] = getattr(step, "provides", ())
            provided.update(provides)

    def insert_after(self, anchor: str, step: Step) -> None:
        """Insert a step after the named anchor step."""
        if step.name in self._name_to_idx:
            raise StepNameDuplicateError(f"Step '{step.name}' already exists")
        anchor_idx = self._name_to_idx.get(anchor)
        if anchor_idx is None:
            raise StepNotFoundError(f"Anchor step '{anchor}' not found")
        self.steps.insert(anchor_idx + 1, step)
        self._rebuild_index()
        if self._validate_dependencies_enabled:
            self._validate_dependencies()

    def insert_before(self, anchor: str, step: Step) -> None:
        """Insert a step before the named anchor step."""
        if step.name in self._name_to_idx:
            raise StepNameDuplicateError(f"Step '{step.name}' already exists")
        anchor_idx = self._name_to_idx.get(anchor)
        if anchor_idx is None:
            raise StepNotFoundError(f"Anchor step '{anchor}' not found")
        self.steps.insert(anchor_idx, step)
        self._rebuild_index()
        if self._validate_dependencies_enabled:
            self._validate_dependencies()

    def remove(self, step_name: str) -> None:
        """Remove a step by name. Raises if the step is not removable."""
        idx = self._name_to_idx.get(step_name)
        if idx is None:
            raise StepNotFoundError(f"Step '{step_name}' not found")
        if not self.steps[idx].removable:
            raise StepNotRemovableError(f"Step '{step_name}' is not removable")
        self.steps.pop(idx)
        self._rebuild_index()

    def _validate_replacement_name(self, new_step: Step, idx: int) -> None:
        """Reject a replacement whose name is already taken by a DIFFERENT step.

        EXE-007. ``docs/features/execution-pipeline.md`` states the invariant:
        step names are unique within a strategy. The constructor checks it once
        and never runs again, and ``insert_after`` / ``insert_before`` check it
        on their own doors — but ``replace`` and ``configure_step`` assigned
        straight into ``self.steps`` and rebuilt the index, so two identically
        named steps could coexist. ``_rebuild_index`` then maps the name to the
        LATER one, silently: a subsequent ``insert_after("module_lookup", …)``
        anchors on the impostor, ``remove()`` targets the wrong entry, and a
        ``skip_to`` resolves past the real step. apcore-rust validates on both
        doors (``pipeline.rs``, ``validate_replacement_name``).

        Renaming is still allowed — including replacing a step with one of the
        same name, which is what ``configure_step``'s idempotence means; only a
        collision with a different position is rejected.
        """
        existing = self._name_to_idx.get(new_step.name)
        if existing is not None and existing != idx:
            raise StepNameDuplicateError(f"Step '{new_step.name}' already exists")

    def replace(self, step_name: str, new_step: Step) -> None:
        """Replace a step by name. Raises if the step is not replaceable."""
        idx = self._name_to_idx.get(step_name)
        if idx is None:
            raise StepNotFoundError(f"Step '{step_name}' not found")
        if not self.steps[idx].replaceable:
            raise StepNotReplaceableError(f"Step '{step_name}' is not replaceable")
        self._validate_replacement_name(new_step, idx)
        self.steps[idx] = new_step
        self._rebuild_index()

    def configure_step(self, step_name: str, new_step: Step) -> None:
        """Replace a step by name (replace semantic: idempotent, no duplicate).

        Calling configure_step twice with the same step_name always leaves
        exactly one step at that position.
        """
        idx = self._name_to_idx.get(step_name)
        if idx is None:
            raise PipelineStepNotFoundError(step_name)
        if not self.steps[idx].replaceable:
            raise StepNotReplaceableError(f"Step '{step_name}' is not replaceable")
        self._validate_replacement_name(new_step, idx)
        self.steps[idx] = new_step
        self._rebuild_index()

    def step_names(self) -> list[str]:
        """Return the ordered list of step names."""
        return [s.name for s in self.steps]

    def info(self) -> StrategyInfo:
        """Return an AI-introspectable description of this strategy."""
        return StrategyInfo(
            name=self.name,
            step_count=len(self.steps),
            step_names=self.step_names(),
            description=" \u2192 ".join(self.step_names()),
        )


async def _invoke_step_mw_hook(
    middlewares: Sequence[StepMiddleware],
    hook: str,
    *args: Any,
    reverse: bool = False,
) -> None:
    """Invoke a *best-effort* ``hook`` on each middleware that defines it.

    Used for ``after_step``, whose return value carries no meaning and whose
    failures are SDK-defined: an exception is logged and iteration continues.
    Awaitable returns are awaited so async middlewares are never dropped as
    un-awaited coroutines.
    """
    iterable = reversed(middlewares) if reverse else middlewares
    for mw in iterable:
        fn = getattr(mw, hook, None)
        if fn is None:
            continue
        try:
            value = fn(*args)
            if inspect.isawaitable(value):
                await value
        except Exception:
            _logger.exception("StepMiddleware %r raised in %s", type(mw).__name__, hook)
            continue


async def _invoke_before_step(
    middlewares: Sequence[StepMiddleware],
    step_name: str,
    state: PipelineState,
) -> None:
    """Run ``before_step`` on every middleware in REGISTRATION order.

    ``before_step`` is an observation hook — its return value is discarded
    (a Step is ``execute(ctx)``; there is no ``inputs`` parameter to replace),
    but an awaitable return is still awaited so async middlewares complete
    before the step body runs.

    An exception raised by a ``before_step`` implementation is NOT swallowed:
    it is wrapped in :class:`~apcore.middleware.manager.MiddlewareChainError`
    carrying the middlewares whose ``before_step`` had already been entered
    (including the one that raised), and the chain stops — the step body must
    not execute.  Mirrors the module-level ``MiddlewareManager.execute_before``
    contract (middleware-system.md § "Pipeline Step Middleware").
    """
    executed: list[Any] = []
    for mw in middlewares:
        executed.append(mw)
        fn = getattr(mw, "before_step", None)
        if fn is None:
            continue
        try:
            value = fn(step_name, state)
            if inspect.isawaitable(value):
                await value
        except Exception as exc:
            raise MiddlewareChainError(exc, list(executed)) from exc


async def _invoke_on_step_error(
    middlewares: Sequence[StepMiddleware],
    step_name: str,
    state: PipelineState,
    error: Exception,
) -> Any:
    """Run ``on_step_error`` in REVERSE registration order, first-recovery-wins.

    This is the STEP-BODY failure path, the only one on which a recovery is
    sought.  The first handler returning a non-``None`` value supplies the
    recovery result and short-circuits the remaining handlers.  Returning
    ``None`` from every handler yields ``None``, letting the original error
    propagate.

    Do NOT reuse this for a ``before_step`` failure — see
    :func:`_invoke_step_cleanup_hooks`.

    A handler that itself raises MUST NOT mask the original error: the
    exception is logged and iteration continues with the next middleware.
    """
    for mw in reversed(list(middlewares)):
        fn = getattr(mw, "on_step_error", None)
        if fn is None:
            continue
        try:
            value = fn(step_name, state, error)
            if inspect.isawaitable(value):
                value = await value
        except Exception:
            _logger.exception(
                "StepMiddleware %r raised in on_step_error",
                type(mw).__name__,
            )
            continue
        if value is not None:
            return value
    return None


async def _invoke_step_cleanup_hooks(
    middlewares: Sequence[Any],
    step_name: str,
    state: PipelineState,
    error: Exception,
) -> None:
    """Notify ``on_step_error`` on the ``before_step`` failure path — cleanup only.

    Deliberately a SEPARATE helper from :func:`_invoke_on_step_error` rather
    than a flag on it, because the two passes have opposite purposes and the
    recovery-seeking one is unsafe here:

    * every return value is DISCARDED — a broken middleware chain is not
      recoverable, so nothing a handler returns may become the step's output or
      let the pipeline continue; and
    * **first-recovery-wins MUST NOT apply.** Short-circuiting exists to stop
      shopping for a recovery once one is found; no recovery is being sought
      here, so **every** already-entered middleware MUST be notified.  Stopping
      at the first one that happens to return a value would strand the cleanup
      of every middleware registered behind it — the opposite of what this pass
      is for.

    Reverse registration order still holds: cleanup unwinds the onion in the
    order the middlewares were entered.  Mirrors apcore-typescript
    ``_runStepCleanupHooks``.  See middleware-system.md § "A `before_step`
    failure terminates the step — it is not recoverable".

    A handler that itself raises is logged and iteration continues, so one
    broken cleanup cannot strand the rest.
    """
    for mw in reversed(list(middlewares)):
        fn = getattr(mw, "on_step_error", None)
        if fn is None:
            continue
        try:
            value = fn(step_name, state, error)
            if inspect.isawaitable(value):
                await value
        except Exception:
            _logger.exception(
                "StepMiddleware %r raised in on_step_error",
                type(mw).__name__,
            )
            continue


class PipelineEngine:
    """Executes a pipeline strategy step by step."""

    async def run(
        self,
        strategy: ExecutionStrategy,
        ctx: PipelineContext,
        *,
        run_until: Callable[[PipelineState], bool] | None = None,
    ) -> tuple[Any, PipelineTrace]:
        """Run all steps in the strategy, returning the final output and trace."""
        trace = PipelineTrace(
            module_id=ctx.module_id,
            strategy_name=strategy.name,
        )
        # Publish the live trace onto the context so a caller that only ever
        # sees the raised exception can still read what happened. Without this,
        # ``Executor.call_async_with_trace`` had nothing to return on the
        # middleware-recovery path and fabricated an empty ``PipelineTrace``.
        # ``PipelineContext.trace`` already existed for exactly this and was
        # never assigned. Parity with apcore-typescript ``pipelineCtx.trace``
        # and apcore-rust ``pipeline_ctx.trace``.
        ctx.trace = trace
        start = time.monotonic()
        steps = strategy.steps
        step_mws = strategy.step_middlewares
        name_to_idx = strategy._name_to_idx  # O(1) step lookup
        step_outputs: dict[str, Any] = {}
        unavailable: set[str] = set()
        effective_run_until = run_until or ctx.run_until
        i = 0

        # Index-based loop (not for-each) to support skip_to.
        while i < len(steps):
            step = steps[i]

            # ── Read declarative metadata (getattr for backward compat) ──
            match_modules = getattr(step, "match_modules", None)
            ignore_errors = getattr(step, "ignore_errors", False)
            pure = getattr(step, "pure", False)
            timeout_ms = getattr(step, "timeout_ms", 0)

            # (1) match_modules filter
            if match_modules is not None and not _any_match(match_modules, ctx.module_id):
                if ctx.dry_run:
                    unavailable.update(getattr(step, "provides", ()))
                trace.steps.append(
                    StepTrace(
                        name=step.name,
                        duration_ms=0,
                        result=StepResult(action="continue"),
                        skipped=True,
                        skip_reason="no_match",
                    )
                )
                i += 1
                continue

            # (2) dry_run filter: skip impure steps
            if getattr(ctx, "dry_run", False) and not pure:
                unavailable.update(getattr(step, "provides", ()))
                trace.steps.append(
                    StepTrace(
                        name=step.name,
                        duration_ms=0,
                        result=StepResult(action="continue"),
                        skipped=True,
                        skip_reason="dry_run",
                    )
                )
                i += 1
                continue

            # Dependencies are logical capabilities, not Context attributes.
            # Only a failed or skipped provider makes a dependent check
            # unevaluable. Independent checks continue after every failure.
            if ctx.dry_run and unavailable.intersection(getattr(step, "requires", ())):
                unavailable.update(getattr(step, "provides", ()))
                trace.steps.append(
                    StepTrace(
                        name=step.name,
                        duration_ms=0,
                        result=StepResult(action="continue"),
                        skipped=True,
                        skip_reason="missing_dependency",
                    )
                )
                i += 1
                continue

            # (3) Execute with optional per-step timeout
            step_start = time.monotonic()

            # ── Step middleware: before_step (registration order) ──
            # Deliberately OUTSIDE the step-body ``try`` below. A before_step
            # failure is categorically different from a step-body failure and
            # the two MUST NOT share a recovery path (middleware-system.md
            # § "A `before_step` failure terminates the step — it is not
            # recoverable"). While they shared one, an on_step_error recovery
            # value advanced the pipeline past a step whose body never ran;
            # with ``acl_check`` and ``approval_gate`` in the built-in
            # strategy, that is a silent authorization bypass reachable from an
            # extension point that carries no authority.
            if step_mws:
                pre_state = PipelineState(
                    step_name=step.name,
                    outputs=step_outputs,
                    context=ctx,
                )
                try:
                    await _invoke_before_step(step_mws, step.name, pre_state)
                except MiddlewareChainError as chain_exc:
                    duration = (time.monotonic() - step_start) * 1000
                    # on_step_error still runs, in reverse order, on the
                    # middlewares whose before_step had already been entered —
                    # for OBSERVATION AND CLEANUP ONLY. Its return value is
                    # discarded: it MUST NOT become the step's output and MUST
                    # NOT let the pipeline continue. The cleanup-only helper is
                    # used, NOT the recovery-seeking one: first-recovery-wins
                    # MUST NOT apply here or the cleanup of every middleware
                    # behind the first one to return a value is stranded.
                    await _invoke_step_cleanup_hooks(
                        chain_exc.executed_middlewares,
                        step.name,
                        pre_state,
                        chain_exc,
                    )
                    trace.steps.append(
                        StepTrace(
                            name=step.name,
                            duration_ms=duration,
                            result=StepResult(action="abort", explanation=str(chain_exc)),
                        )
                    )
                    trace.total_duration_ms = (time.monotonic() - start) * 1000
                    # ``after_step`` MUST NOT fire — no step body ran, so there
                    # is nothing to close over — and the step's ``ignore_errors``
                    # MUST NOT apply: it declares that *this step's* failure is
                    # tolerable, and a broken middleware chain is not a step
                    # failure. MiddlewareChainError propagates regardless, in
                    # its canonical wrapper (never re-wrapped as
                    # PipelineStepError).
                    raise

            try:
                if timeout_ms > 0:
                    result = await asyncio.wait_for(
                        step.execute(ctx),
                        timeout=timeout_ms / 1000,
                    )
                else:
                    result = await step.execute(ctx)
            except Exception as exc:
                duration = (time.monotonic() - step_start) * 1000

                if ctx.dry_run:
                    unavailable.update(getattr(step, "provides", ()))
                    to_dict = getattr(exc, "to_dict", None)
                    serialized = to_dict() if callable(to_dict) else None
                    ctx.preflight_errors[step.name] = (
                        serialized
                        if isinstance(serialized, dict)
                        else {
                            "code": type(exc).__name__,
                            "message": str(exc),
                        }
                    )
                    trace.steps.append(
                        StepTrace(
                            name=step.name,
                            duration_ms=duration,
                            result=StepResult(action="abort", explanation=str(exc)),
                        )
                    )
                    i += 1
                    continue

                # EXE-006 — a per-step timeout is an ordinary step failure.
                # `timeout_ms` is documented as a plain per-step timeout
                # (core-executor.md "Step Metadata": "Per-step timeout in
                # milliseconds (0 = no limit)") with no exemption from the
                # step-error contract, and apcore-typescript and apcore-rust
                # both route it here. It used to have a dedicated `except
                # asyncio.TimeoutError` clause AHEAD of this one, which never
                # called `_invoke_on_step_error` and raised PipelineAbortError
                # directly — so `on_step_error` never observed the timeout, a
                # recovery value could not resume the pipeline, and the
                # Executor's PipelineAbortError branch re-raised without
                # `_recover_from_call_error`, skipping the module's `on_error`
                # too. Only `ignore_errors` was honoured, by that clause's own
                # copy of the logic.
                #
                # The MODULE_TIMEOUT code is preserved by carrying a typed
                # ModuleTimeoutError as the cause below, so the error a caller
                # sees is unchanged.
                is_timeout = isinstance(exc, asyncio.TimeoutError)
                error_text = f"Step timed out after {timeout_ms}ms" if is_timeout else str(exc)

                # ── Step middleware: on_step_error (reverse order) ──
                # This is the STEP-BODY failure path: every registered step
                # middleware entered ``before_step``, so every one may observe
                # the error, and the FIRST non-None return is the recovery
                # value and short-circuits the remaining handlers; None means
                # propagate. A ``before_step`` failure never reaches here — it
                # is terminal and handled above.
                #
                # A MiddlewareChainError CAN still arrive here, from the
                # ``middleware_before`` step body re-raising the MODULE-level
                # chain error. Its ``executed_middlewares`` are module
                # Middleware instances, not StepMiddlewares, so they must not
                # be substituted for ``step_mws``.
                error_mws: Sequence[StepMiddleware] = step_mws
                recovery: Any = None
                if error_mws:
                    err_state = PipelineState(
                        step_name=step.name,
                        outputs=step_outputs,
                        context=ctx,
                    )
                    recovery = await _invoke_on_step_error(error_mws, step.name, err_state, exc)

                if recovery is not None:
                    # Coerce non-StepResult recoveries (e.g. plain dicts) into
                    # a continue StepResult, matching middleware-on-error semantics.
                    # The recovery value itself becomes the step's output.
                    if not isinstance(recovery, StepResult):
                        if isinstance(recovery, dict):
                            ctx.output = recovery
                        recovery = StepResult(
                            action="continue",
                            explanation=f"Recovered from {type(exc).__name__}",
                        )
                    result = recovery
                    # Fall through to the success path below.
                else:
                    # (4) ignore_errors: log and continue
                    if ignore_errors:
                        _logger.warning("Step '%s' failed (ignored): %s", step.name, error_text)
                        trace.steps.append(
                            StepTrace(
                                name=step.name,
                                duration_ms=duration,
                                result=StepResult(
                                    action="continue",
                                    explanation=error_text,
                                ),
                                skip_reason="error_ignored",
                            )
                        )
                        i += 1
                        continue
                    # Fail-fast (execution-pipeline.md § "Errors"): wrap in PipelineStepError with step name and cause
                    trace.steps.append(
                        StepTrace(
                            name=step.name,
                            duration_ms=duration,
                            result=StepResult(action="abort", explanation=error_text),
                        )
                    )
                    trace.total_duration_ms = (time.monotonic() - start) * 1000
                    if is_timeout:
                        # The typed cause carries MODULE_TIMEOUT, so the
                        # Executor unwraps it to the same ModuleTimeoutError
                        # callers caught before — via PipelineStepError, which
                        # is the branch that runs the module's `on_error`.
                        raise PipelineStepError(
                            step_name=step.name,
                            cause=ModuleTimeoutError(module_id=ctx.module_id, timeout_ms=timeout_ms),
                            trace=trace,
                        ) from exc
                    if isinstance(exc, MiddlewareChainError):
                        # A middleware chain failure is already in its canonical
                        # wrapper; re-wrapping would hide MiddlewareChainError
                        # from callers that catch it (spec: "MUST be wrapped in
                        # MiddlewareChainError, identical to the module-level
                        # contract").
                        raise
                    raise PipelineStepError(step_name=step.name, cause=exc, trace=trace) from exc

            # (5) Record successful step trace
            step_trace = StepTrace(
                name=step.name,
                duration_ms=(time.monotonic() - step_start) * 1000,
                result=result,
                decision_point=result.confidence is not None,
            )
            trace.steps.append(step_trace)

            # ── Step middleware: after_step (reverse order, onion semantics) ──
            # Runs BEFORE the ``step_outputs`` snapshot below, deliberately.
            # ``state.outputs`` holds exactly the steps that completed BEFORE
            # the current one, in all three hooks, so a middleware never has to
            # know which hook it is in to read the map (middleware-system.md
            # § "What `state.outputs` contains"). The current step's output is
            # not missing from ``after_step`` — it is the ``result`` argument;
            # carrying the same value down two paths is how the two drift
            # apart. This ordering is what enforces the rule, and it is what
            # apcore-typescript (src/pipeline.ts) and apcore-rust already do.
            if step_mws:
                post_state = PipelineState(
                    step_name=step.name,
                    outputs=step_outputs,
                    context=ctx,
                )
                await _invoke_step_mw_hook(step_mws, "after_step", step.name, post_state, result, reverse=True)

            # (6) Snapshot output for run_until predicates (shallow copy prevents
            # later in-place mutations from altering the historical record).
            # Placed after ``after_step`` (above) and before the ``run_until``
            # predicate (below): run_until observes the step that just
            # completed, step middleware hooks do not.
            step_outputs[step.name] = dict(ctx.output) if ctx.output is not None else None

            if result.action == "abort":
                if ctx.dry_run:
                    unavailable.update(getattr(step, "provides", ()))
                    i += 1
                    continue
                trace.total_duration_ms = (time.monotonic() - start) * 1000
                raise PipelineAbortError(
                    step=step.name,
                    explanation=result.explanation,
                    alternatives=result.alternatives,
                    trace=trace,
                )
            elif result.action == "skip_to":
                target = result.skip_to
                if target is None:
                    raise StepNotFoundError("skip_to target is None")
                # O(1) lookup via pre-built index
                target_idx = name_to_idx.get(target)
                if target_idx is None or target_idx <= i:
                    raise StepNotFoundError(f"skip_to target '{target}' not found")
                # Record skipped steps in trace
                for j in range(i + 1, target_idx):
                    if ctx.dry_run:
                        unavailable.update(getattr(steps[j], "provides", ()))
                    trace.steps.append(
                        StepTrace(
                            name=steps[j].name,
                            duration_ms=0,
                            result=StepResult(action="continue"),
                            skipped=True,
                            decision_point=False,
                        )
                    )
                i = target_idx
                continue

            if ctx.dry_run:
                unavailable.difference_update(getattr(step, "provides", ()))

            # (7) run_until: evaluate predicate only on clean continue (execution-pipeline.md § "run_until")
            if effective_run_until is not None:
                state = PipelineState(
                    step_name=step.name,
                    outputs=step_outputs,
                    context=ctx,
                )
                if effective_run_until(state):
                    trace.success = True
                    trace.total_duration_ms = (time.monotonic() - start) * 1000
                    return ctx.output, trace

            i += 1

        trace.success = not any(item.result.action == "abort" for item in trace.steps if not item.skipped)
        trace.total_duration_ms = (time.monotonic() - start) * 1000
        # Use ctx.output — it's the most up-to-date (middleware_after may modify it)
        return ctx.output, trace


# ---------------------------------------------------------------------------
# Error types
# ---------------------------------------------------------------------------


class AbortReason(str, Enum):
    """Stable, typed discriminator for why a pipeline step aborted.

    Prefer this over inspecting ``PipelineAbortError.explanation`` strings:
    the explanation is user-facing text that may change between releases,
    while ``AbortReason`` is a committed contract consumed by the executor
    when translating aborts back to typed errors.
    """

    MODULE_NOT_FOUND = "module_not_found"
    ACL_DENIED = "acl_denied"
    INPUT_VALIDATION_FAILED = "input_validation_failed"
    OUTPUT_VALIDATION_FAILED = "output_validation_failed"
    MODULE_CANCELLED = "module_cancelled"
    MODULE_TIMEOUT = "module_timeout"
    OTHER = "other"


class PipelineAbortError(ModuleError):
    """Raised when a pipeline is aborted at a step."""

    def __init__(
        self,
        step: str,
        explanation: str | None = None,
        alternatives: list[str] | None = None,
        trace: PipelineTrace | None = None,
        abort_reason: AbortReason | None = None,
        **kwargs: Any,
    ) -> None:
        super().__init__(
            code="PIPELINE_ABORT",
            message=f"Pipeline aborted at step '{step}': {explanation or 'no explanation'}",
            **kwargs,
        )
        self.step = step
        self.explanation = explanation
        self.alternatives = alternatives
        self.pipeline_trace = trace
        self.abort_reason: AbortReason = abort_reason or AbortReason.OTHER

    @property
    def step_name(self) -> str:
        """Alias for ``step``, matching the naming convention of ``PipelineStepError``."""
        return self.step


class StepNotFoundError(ModuleError):
    """Raised when a referenced step does not exist."""

    def __init__(self, message: str = "", **kwargs: Any) -> None:
        super().__init__(code="STEP_NOT_FOUND", message=message, **kwargs)


class StepNotRemovableError(ModuleError):
    """Raised when attempting to remove a non-removable step."""

    def __init__(self, message: str = "", **kwargs: Any) -> None:
        super().__init__(code="STEP_NOT_REMOVABLE", message=message, **kwargs)


class StepNotReplaceableError(ModuleError):
    """Raised when attempting to replace a non-replaceable step."""

    def __init__(self, message: str = "", **kwargs: Any) -> None:
        super().__init__(code="STEP_NOT_REPLACEABLE", message=message, **kwargs)


class StepNameDuplicateError(ModuleError):
    """Raised when a step name already exists in the strategy."""

    def __init__(self, message: str = "", **kwargs: Any) -> None:
        super().__init__(code="STEP_NAME_DUPLICATE", message=message, **kwargs)


class StrategyNotFoundError(ModuleError):
    """Raised when a referenced strategy does not exist."""

    def __init__(self, message: str = "", **kwargs: Any) -> None:
        super().__init__(code="STRATEGY_NOT_FOUND", message=message, **kwargs)


class PipelineStepError(ModuleError):
    """Raised when a pipeline step fails (fail-fast; execution-pipeline.md § "Errors").

    Wraps the original exception from the step, carrying the step name for
    diagnostics. When ignore_errors is true on the step, this error is NOT
    raised; execution continues instead.
    """

    def __init__(
        self,
        step_name: str,
        cause: Exception | None = None,
        trace: PipelineTrace | None = None,
        **kwargs: Any,
    ) -> None:
        cause_msg = str(cause) if cause is not None else "unknown error"
        super().__init__(
            code="PIPELINE_STEP_ERROR",
            message=f"Pipeline step '{step_name}' failed: {cause_msg}",
            cause=cause,
            details={"step_name": step_name},
            **kwargs,
        )
        self.step_name = step_name
        self.pipeline_trace = trace


class PipelineStepNotFoundError(ModuleError):
    """Raised when configure_step targets a step name that does not exist."""

    def __init__(self, step_name: str = "", **kwargs: Any) -> None:
        super().__init__(
            code="PIPELINE_STEP_NOT_FOUND",
            message=f"Pipeline step not found: '{step_name}'",
            **kwargs,
        )
        self.step_name = step_name


class ConfigurationError(ModuleError):
    """Raised when pipeline YAML configuration is invalid (execution-pipeline.md § "Configuring the pipeline from apcore.yaml").

    Replaces the prior warning-and-continue behaviour: when YAML refers to a
    step name that does not exist (in ``remove``, ``configure``, or
    ``insert.before``/``insert.after``), or assigns an unknown field via
    ``configure``, a :class:`ConfigurationError` is raised so the misconfig
    surfaces at start-up rather than at runtime.
    """

    def __init__(self, message: str = "", **kwargs: Any) -> None:
        super().__init__(code="PIPELINE_CONFIGURATION_ERROR", message=message, **kwargs)


class PipelineDependencyError(ModuleError):
    """Raised when a step's ``requires`` are not satisfied by preceding
    ``provides`` (Issue #33 §2.1).

    The error names the offending step and the missing keys so the operator
    can correct the strategy assembly.
    """

    def __init__(
        self,
        step_name: str = "",
        missing: tuple[str, ...] = (),
        **kwargs: Any,
    ) -> None:
        msg = f"Pipeline step '{step_name}' requires {set(missing)}, but no preceding step provides them."
        super().__init__(
            code="PIPELINE_DEPENDENCY_ERROR",
            message=msg,
            details={"step_name": step_name, "missing": list(missing)},
            **kwargs,
        )
        self.step_name = step_name
        self.missing = missing
