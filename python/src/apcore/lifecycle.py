"""In-process lifecycle operations and immutable progress reports."""

from __future__ import annotations

import asyncio
import logging
import threading
from time import monotonic
from collections.abc import Callable, Iterator
from contextlib import contextmanager
from contextvars import ContextVar
from dataclasses import asdict, dataclass, replace
from typing import Literal
from uuid import uuid4

from apcore.errors import ErrorCodes, InvalidInputError, ModuleError
from apcore.cancel import CancelToken

logger = logging.getLogger(__name__)

_Kind = Literal["shutdown", "unregister_many", "reload"]
_Status = Literal["pending", "completed", "failed"]
_EventFlush = Literal["not_applicable", "pending", "completed", "timeout"]
_Phase = Literal["pending", "draining", "recovering", "unloading", "loading", "resuming", "done"]
_Outcome = Literal[
    "removed",
    "forced_removed",
    "absent",
    "already_removing",
    "not_started",
    "failed",
    "retained_ready",
    "retained_unavailable",
    "reloaded",
]
_StopReason = Literal["operation_deadline"]
_HostDrainAction = Literal["continue_waiting", "cancel_then_force"]
_ReloadDrainAction = Literal["continue_waiting", "abort_reload"]
_AbortReason = Literal["drain_timeout", "drain_error"]
_SELF_WAIT = "A lifecycle callback cannot wait for its own operation"


@dataclass(frozen=True)
class ShutdownOptions:
    """Policy for starting or rejoining host shutdown. Omitted fields inherit."""

    drain_timeout_ms: int | None = None
    on_drain_timeout: _HostDrainAction | None = None
    event_flush_timeout_ms: int | None = None
    cancel_grace_ms: int | None = None


@dataclass(frozen=True)
class UnregisterOptions:
    """Per-generation drain threshold and optional ordered-batch deadline."""

    drain_timeout_ms: int | None = None
    on_drain_timeout: _HostDrainAction | None = None
    operation_timeout_ms: int | None = None
    cancel_grace_ms: int | None = None


@dataclass(frozen=True)
class ReloadOptions:
    """Managed reload policy for filesystem watchers."""

    drain_timeout_ms: int = 5000
    on_drain_timeout: _ReloadDrainAction = "abort_reload"


@dataclass(frozen=True)
class LifecycleError:
    """A public error summary with the lifecycle phase and error code."""

    code: str
    phase: str

    @classmethod
    def _from_exception(cls, error: BaseException, phase: str) -> LifecycleError:
        code = error.code if isinstance(error, ModuleError) else ErrorCodes.GENERAL_INTERNAL_ERROR
        return cls(code=code, phase=phase)


@dataclass(frozen=True)
class LifecycleModuleReport:
    """Progress and outcome for one input row or shutdown registration."""

    module_id: str
    phase: _Phase = "pending"
    outcome: _Outcome | None = None
    active_hook: Literal["on_drain", "on_drain_aborted", "on_suspend", "on_unload", "on_load", "on_resume"] | None = (
        None
    )
    drain_timed_out: bool = False
    active_at_unload: int | None = None
    cleanup_pending: bool = False
    abort_reason: _AbortReason | None = None
    compensation: Literal["not_required", "restored", "failed"] | None = None
    error: LifecycleError | None = None


@dataclass(frozen=True)
class LifecycleReport:
    """A snapshot that remains valid as the operation advances."""

    operation_id: str
    kind: _Kind
    status: _Status
    modules: tuple[LifecycleModuleReport, ...]
    stop_reason: _StopReason | None
    event_flush: _EventFlush
    error: LifecycleError | None

    def _to_wire(self) -> dict[str, object]:
        report = asdict(self)
        local_fields = {"active_hook", "active_at_unload", "cleanup_pending"}
        report["modules"] = [
            {key: value for key, value in row.items() if value is not None and key not in local_fields}
            for row in report["modules"]
        ]
        return {key: value for key, value in report.items() if value is not None}


@dataclass(frozen=True)
class DrainContext:
    """Information and cooperative signal for one module's drain hook."""

    operation_id: str
    module_id: str
    reason_kind: Literal["unregister", "reload", "shutdown"]
    cancel_token: CancelToken
    _deadline: float

    def remaining_ms(self) -> int:
        """Return remaining whole milliseconds on the monotonic drain clock."""
        return max(0, int((self._deadline - monotonic()) * 1000))


@dataclass(frozen=True)
class DrainAbortedContext:
    """Information and fresh cooperative signal for a readiness recovery hook."""

    operation_id: str
    module_id: str
    abort_reason: _AbortReason
    cancel_token: CancelToken


def _validate_duration(name: str, value: object) -> int | None:
    if value is not None and (type(value) is not int or not 0 <= value <= 2_147_483_647):
        raise InvalidInputError(message=f"{name} must be an integer between 0 and 2147483647")
    return value


def _validate_drain_options(options: ShutdownOptions | UnregisterOptions) -> None:
    _validate_duration("drain_timeout_ms", options.drain_timeout_ms)
    if options.on_drain_timeout not in (None, "continue_waiting", "cancel_then_force"):
        raise InvalidInputError(message="on_drain_timeout must be continue_waiting or cancel_then_force")
    _validate_duration("cancel_grace_ms", options.cancel_grace_ms)
    if options.cancel_grace_ms is not None and options.on_drain_timeout not in (None, "cancel_then_force"):
        raise InvalidInputError(message="cancel_grace_ms requires cancel_then_force")


def _require_lifecycle_loop() -> None:
    try:
        asyncio.get_running_loop()
    except RuntimeError as error:
        raise InvalidInputError(message="Starting a lifecycle operation requires a running asyncio loop") from error


@dataclass
class _CallbackScope:
    registry: object
    operation_id: str | None
    active: bool = True


_callback_scopes: ContextVar[tuple[_CallbackScope, ...]] = ContextVar("apcore_lifecycle_callbacks", default=())


@contextmanager
def _lifecycle_callback(registry: object, operation_id: str | None = None) -> Iterator[None]:
    scope = _CallbackScope(registry, operation_id)
    token = _callback_scopes.set((*_callback_scopes.get(), scope))
    try:
        yield
    finally:
        scope.active = False
        _callback_scopes.reset(token)


class LifecycleOperation:
    """An owned cleanup operation. Cancelling or timing out a wait leaves it running."""

    def __init__(
        self,
        kind: _Kind,
        *,
        event_flush: _EventFlush = "not_applicable",
        registry: object | None = None,
        is_delivery_thread: Callable[[], bool] | None = None,
    ) -> None:
        self._id = uuid4().hex
        self._kind: _Kind = kind
        self._registry = registry
        self._is_delivery_thread = is_delivery_thread
        self._source: LifecycleOperation | None = None
        self._status: _Status = "pending"
        self._event_flush: _EventFlush = event_flush
        self._modules: list[LifecycleModuleReport] = []
        self._stop_reason: _StopReason | None = None
        self._error: LifecycleError | None = None
        self._failure: BaseException | None = None
        self._lock = threading.Lock()
        self._finished = threading.Event()
        self._waiters: set[tuple[asyncio.AbstractEventLoop, asyncio.Future[None]]] = set()

    def snapshot(self) -> LifecycleReport:
        """Read the latest progress without waiting."""
        with self._lock:
            source = self._source
            if source is not None:
                report = source.snapshot()
                return replace(
                    report, status=self._status, event_flush=self._event_flush, error=self._error or report.error
                )
            return LifecycleReport(
                self._id,
                self._kind,
                self._status,
                tuple(self._modules),
                self._stop_reason,
                self._event_flush,
                self._error,
            )

    async def wait(self, *, wait_timeout_ms: int | None = None) -> LifecycleReport:
        """Await completion or return the current report when this wait expires."""
        timeout = _validate_duration("wait_timeout_ms", wait_timeout_ms)
        if timeout == 0 or self._finished.is_set():
            return self.snapshot()
        self._require_wait_allowed()
        loop = asyncio.get_running_loop()
        waiter: asyncio.Future[None] = loop.create_future()
        with self._lock:
            if not self._finished.is_set():
                self._waiters.add((loop, waiter))
            else:
                waiter.set_result(None)
        try:
            async with asyncio.timeout(None if timeout is None else timeout / 1000):
                await waiter
        except TimeoutError:
            pass
        finally:
            with self._lock:
                self._waiters.discard((loop, waiter))
        return self.snapshot()

    def _require_wait_allowed(self) -> None:
        if self._finished.is_set():
            return
        callback_blocks_completion = any(
            scope.active
            and (scope.operation_id == self._id or (self._kind == "shutdown" and scope.registry is self._registry))
            for scope in _callback_scopes.get()
        )
        if callback_blocks_completion or (self._is_delivery_thread is not None and self._is_delivery_thread()):
            raise InvalidInputError(message=_SELF_WAIT)

    def _join_blocking(self) -> LifecycleReport:
        self._require_wait_allowed()
        self._finished.wait()
        return self.snapshot()

    def _attach_source(self, source: LifecycleOperation) -> None:
        with self._lock:
            self._id = source._id
            self._source = source

    def _set_modules(self, module_ids: Iterator[str]) -> None:
        with self._lock:
            self._modules = [LifecycleModuleReport(module_id) for module_id in module_ids]

    def _add_module(self, module_id: str) -> int:
        with self._lock:
            index = len(self._modules)
            self._modules.append(LifecycleModuleReport(module_id))
            return index

    def _update_module(self, index: int, **updates: object) -> None:
        with self._lock:
            row = replace(self._modules[index], **updates)
            self._modules[index] = row
            self._error = self._error or row.error

    def _stop_batch(self, index: int) -> None:
        with self._lock:
            self._stop_reason = "operation_deadline"
            self._modules[index:] = [replace(row, phase="done", outcome="not_started") for row in self._modules[index:]]

    def _complete(
        self,
        *,
        error: LifecycleError | None = None,
        failure: BaseException | None = None,
        event_flush: _EventFlush | None = None,
    ) -> None:
        with self._lock:
            self._failure = failure
            self._error = self._error or error
            self._status = "failed" if self._error is not None or self._stop_reason is not None else "completed"
            if event_flush is not None:
                self._event_flush = event_flush
            self._finished.set()
            waiters = tuple(self._waiters)
            self._waiters.clear()
        for loop, waiter in waiters:
            try:
                loop.call_soon_threadsafe(_resolve_waiter, waiter)
            except RuntimeError:
                # A closed waiter loop has no effect on the owned operation.
                continue

    def _start(self, run: Callable[[], None], error_phase: str) -> None:
        def work() -> None:
            try:
                run()
            except BaseException as error:
                logger.exception("Lifecycle operation %s failed", self._id)
                self._complete(error=LifecycleError._from_exception(error, error_phase), failure=error)

        thread = threading.Thread(target=work, name=f"apcore-lifecycle-{self._kind}", daemon=True)
        try:
            thread.start()
        except RuntimeError as error:
            raise InvalidInputError(message="Could not start a lifecycle worker") from error


def _resolve_waiter(waiter: asyncio.Future[None]) -> None:
    if not waiter.done():
        waiter.set_result(None)
