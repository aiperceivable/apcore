"""High-level client for apcore to simplify interaction.

Cross-language note — sync vs async divergence
-----------------------------------------------
The TypeScript and Rust SDKs expose ``call()``, ``validate()``, ``disable()``,
and ``enable()`` as *async* functions (``Promise``/``Future``).  The Python SDK
keeps these methods *synchronous* for ergonomics: internally they run the async
pipeline on a cached event loop (or a background thread when a loop is already
running — the "sync-in-thread" model).

When porting code from TypeScript/Rust to Python, remove ``await`` from these
calls.  When writing Python code that must itself be async, prefer the
``call_async()`` / ``stream()`` counterparts which are true coroutines.
"""

from __future__ import annotations

import asyncio
from collections.abc import AsyncIterator, Callable as CallableABC
from typing import Any, Callable

from apcore.config import Config
from apcore.context import Context
from apcore.errors import SysModulesDisabledError
from apcore.decorator import module as decorator_module
from apcore.events.emitter import ApCoreEvent, EventEmitter, EventSubscriber
from apcore.executor import Executor
from apcore.module import PreflightResult
from apcore.policy import ExecutionPolicy
from apcore.observability.metrics import MetricsCollector
from apcore.registry import Registry
from apcore.sys_modules.control import ToggleState


class APCore:
    """A high-level client that manages Registry and Executor.

    This class provides a unified entry point for apcore, making it easier
    for beginners to get started without manually managing multiple objects.
    """

    def __init__(
        self,
        registry: Registry | None = None,
        executor: Executor | None = None,
        config: Config | None = None,
        metrics_collector: MetricsCollector | None = None,
        policy: ExecutionPolicy | None = None,
        *,
        fail_on_error: bool = False,
    ) -> None:
        """Initialize the APCore client.

        Args:
            registry: Optional Registry instance. Creates a new one if None.
            executor: Optional Executor instance. Creates a new one if None.
            config: Optional Config instance. Use Config.load() to load from a file.
            metrics_collector: Optional MetricsCollector for observability.
                               Auto-created when sys_modules are enabled and none is provided.
            policy: Optional ExecutionPolicy with execution-time governance
                overrides (apcore#76). Applied to the auto-created Executor's
                approval gate. Ignored when the caller supplies their own
                ``executor`` — wire the policy on that Executor directly
                (parity with config-driven ACL discovery).
            fail_on_error: Raise the first system-module registration failure.
                By default, startup continues and exposes failures through
                ``sys_modules_errors``.
        """
        self.config = config

        # Per-instance ToggleState (#71). Each APCore instance owns one
        # ToggleState that is injected into BOTH the write path
        # (ToggleFeatureModule, via register_sys_modules) and the read path
        # (BuiltinModuleLookup, via the Executor's strategy). This isolates
        # toggles between instances in the same process; disabling a module on
        # one APCore does not disable it on another. The module-global
        # ``_default_toggle_state`` remains the fallback only for the free
        # ``is_module_disabled`` function and Executors built without a
        # toggle_state.
        self.toggle_state = ToggleState()

        # When the caller supplies their own Executor we respect it as-is (its
        # read path uses whatever toggle_state it was built with). The
        # auto-created Executor is wired to this instance's ToggleState.
        executor_supplied = executor is not None
        if executor_supplied:
            self.executor = executor  # type: ignore[assignment]
            # CLI-1. The client adopts the supplied Executor's Registry rather
            # than building a second one beside it. `Contract: APCore.with_options`
            # already states this — `registry` is "Ignored when executor is also
            # provided (the executor's own registry is used instead)" — and
            # apcore-rust (client.rs) implements it. Without it the client's
            # write path (`register`, `module`, `discover`, `list_modules`) and
            # the executor's read path address two different Registry objects,
            # so a module registered through the client is not callable through
            # it, `list_modules()` advertises modules that raise
            # MODULE_NOT_FOUND, and `disable()` passes the sys-modules guard
            # only to fail on `system.control.toggle_feature`.
            self.registry = self.executor.registry
        else:
            # CLI-3. The auto-created Registry is given the client's Config.
            # Built without one, `Registry._scan_params` returns hardcoded
            # defaults and the extension roots fall back to the built-in
            # `./extensions`, which makes `extensions.root`, `extensions.roots`,
            # `extensions.max_depth`, `extensions.follow_symlinks`,
            # `extensions.ignore_patterns` and `id_map.overrides` all inert
            # through the client door — the "declared key reaches no mechanism"
            # shape §9.1.3 forbids. `Contract: APCore.discover` says discovery
            # roots come from `extensions.root` in the config supplied at
            # construction.
            self.registry = registry or Registry(config=config)
            self.executor = Executor(
                registry=self.registry,
                config=config,
                policy=policy,
                toggle_state=self.toggle_state,
            )

        # Config-driven ACL discovery (D-64, Recommendation A). Resolve
        # acl.root and attach an ACL only when the path exists; a missing path
        # is a no-op that preserves today's no-enforcement default (it must NOT
        # synthesize an empty default-deny ACL). Done before sys_modules
        # registration so system modules execute under the discovered policy.
        # Skipped when the caller supplied their own Executor — discovery must
        # not clobber an ACL the caller wired explicitly (parity with the TS and
        # Rust SDKs).
        if self.config is not None and not executor_supplied:
            from apcore.acl import ACL

            discovered_acl = ACL.discover(self.config)
            if discovered_acl is not None:
                self.executor.set_acl(discovered_acl)

        # Config-driven tracing (PROTOCOL_SPEC §10.1.1). `observability.tracing.*`
        # was five declared keys that reached nothing: no SDK had ever built a
        # TracingMiddleware from configuration, so `enabled: true` installed
        # nothing and the other four configured a middleware that did not exist.
        #
        # Skipped when the caller supplied their own Executor — an Executor the
        # caller built is respected as-is, tracing included, exactly as
        # config-driven ACL discovery above treats it. That is also what makes
        # §10.1.1 requirement 6 hold without a second check here: the
        # auto-created Executor's middleware chain is empty at this point, so
        # configuration can never be the thing that adds a SECOND tracing
        # middleware. A caller who calls `use(TracingMiddleware(...))` after
        # construction is adding one deliberately, and keeps it.
        if self.config is not None and not executor_supplied:
            from apcore.observability.tracing_config import build_tracing_middleware

            tracing_mw = build_tracing_middleware(self.config)
            if tracing_mw is not None:
                self.executor.use(tracing_mw)

        # Auto-register sys.* modules and middleware from config
        self._sys_modules_context: dict[str, Any] = {}
        self._sys_modules_errors: list[Exception] = []
        if self.config is not None:
            from apcore.sys_modules.registration import register_sys_modules

            # Auto-create MetricsCollector when sys_modules are enabled
            if metrics_collector is None and self.config.get("sys_modules.enabled", False):
                metrics_collector = MetricsCollector()
            self.metrics_collector = metrics_collector

            try:
                self._sys_modules_context = register_sys_modules(
                    registry=self.registry,
                    executor=self.executor,
                    config=self.config,
                    metrics_collector=self.metrics_collector,
                    fail_on_error=fail_on_error,
                    toggle_state=self.toggle_state,
                    on_registration_error=self._sys_modules_errors.append,
                )
            except Exception as exc:
                if fail_on_error:
                    raise
                self._sys_modules_errors.append(exc)
                import logging

                logging.getLogger(__name__).warning(
                    "Failed to register sys modules",
                    exc_info=True,
                )
            emitter = self.events
            if not executor_supplied and emitter is not None:
                self.executor.set_event_emitter(emitter)
        else:
            self.metrics_collector = metrics_collector

    @property
    def sys_modules_errors(self) -> tuple[Exception, ...]:
        """Return system-module registration errors recovered during startup."""
        return tuple(self._sys_modules_errors)

    def module(
        self,
        id: str | None = None,  # noqa: A002
        description: str | None = None,
        documentation: str | None = None,
        annotations: dict[str, Any] | None = None,
        tags: list[str] | None = None,
        version: str = "1.0.0",
        metadata: dict[str, Any] | None = None,
        display: dict[str, Any] | None = None,
        examples: list[Any] | None = None,
        *,
        resources: dict[str, Any] | None = None,
    ) -> Callable:
        """Decorator to register a function as a module in this client's registry.

        ``resources["timeout"]`` sets the module's execution budget in milliseconds.

        Usage:
            @client.module(id="math.add")
            def add(a: int, b: int) -> int:
                return a + b
        """

        def decorator(func: Callable) -> Callable:
            inner = decorator_module(
                id=id,
                description=description,
                documentation=documentation,
                annotations=annotations,
                tags=tags,
                version=version,
                metadata=metadata,
                display=display,
                examples=examples,
                registry=self.registry,
                resources=resources,
            )
            return inner(func)

        return decorator

    def register(self, module_id: str, module_obj: Any) -> None:
        """Register a module object directly.

        Args:
            module_id: The ID to register the module under.
            module_obj: The module instance (Class or FunctionModule).
        """
        self.registry.register(module_id, module_obj)

    def call(
        self,
        module_id: str,
        inputs: dict[str, Any] | None = None,
        context: Context | None = None,
        version_hint: str | None = None,
        *,
        timeout_ms: int | None = None,
    ) -> dict[str, Any]:
        """Execute a module call.

        Args:
            module_id: The ID of the module to call.
            inputs: Input arguments for the module.
            context: Optional execution context.
            version_hint: Optional semver hint for version negotiation.
            timeout_ms: Optional per-call wait limit for the sync thread bridge.

        Returns:
            The module output.

        Cross-language note:
            TypeScript and Rust expose this as an async method (``await client.call(...)``).
            Python keeps it synchronous using the sync-in-thread model; do not ``await`` it.
            Use ``call_async()`` instead when calling from an async context.
        """
        if timeout_ms is None:
            return self.executor.call(module_id, inputs, context, version_hint=version_hint)
        return self.executor.call(module_id, inputs, context, version_hint=version_hint, timeout_ms=timeout_ms)

    async def call_async(
        self,
        module_id: str,
        inputs: dict[str, Any] | None = None,
        context: Context | None = None,
        version_hint: str | None = None,
    ) -> dict[str, Any]:
        """Execute an asynchronous module call.

        Args:
            module_id: The ID of the module to call.
            inputs: Input arguments for the module.
            context: Optional execution context.
            version_hint: Optional semver hint for version negotiation.

        Returns:
            The module output.
        """
        return await self.executor.call_async(module_id, inputs, context, version_hint=version_hint)

    async def stream(
        self,
        module_id: str,
        inputs: dict[str, Any] | None = None,
        context: Context | None = None,
        version_hint: str | None = None,
    ) -> AsyncIterator[dict[str, Any]]:
        """Stream module output chunk by chunk.

        Args:
            module_id: The ID of the module to stream.
            inputs: Input arguments for the module.
            context: Optional execution context.
            version_hint: Optional semver hint for version negotiation.

        Yields:
            Dict chunks from the module's stream() or a single call result.
        """
        async for chunk in self.executor.stream(module_id, inputs, context, version_hint=version_hint):
            yield chunk

    def validate(
        self,
        module_id: str,
        inputs: dict[str, Any] | None = None,
        context: Context | None = None,
        *,
        timeout_ms: int | None = None,
    ) -> PreflightResult:
        """Non-destructive preflight check without execution.

        Args:
            module_id: The module to validate against.
            inputs: Input data to validate.
            context: Optional context for call-chain checks.
            timeout_ms: Optional per-call wait limit for the sync thread bridge.

        Returns:
            PreflightResult with per-check status.

        Cross-language note:
            TypeScript and Rust expose this as an async method.
            Python keeps it synchronous; do not ``await`` it.
        """
        if timeout_ms is None:
            return self.executor.validate(module_id, inputs, context)
        return self.executor.validate(module_id, inputs, context, timeout_ms=timeout_ms)

    def describe(self, module_id: str) -> str:
        """Get module description info (for AI/LLM use).

        Args:
            module_id: The module to describe.

        Returns:
            Human-readable description of the module.
        """
        return self.registry.describe(module_id)

    def use(self, middleware: Any) -> APCore:
        """Add class-based middleware. Returns self for chaining."""
        self.executor.use(middleware)
        return self

    def use_before(self, callback: Callable) -> APCore:
        """Add before function middleware. Returns self for chaining."""
        self.executor.use_before(callback)
        return self

    def use_after(self, callback: Callable) -> APCore:
        """Add after function middleware. Returns self for chaining."""
        self.executor.use_after(callback)
        return self

    def remove(self, middleware: Any) -> bool:
        """Remove middleware by identity. Returns True if found and removed."""
        return self.executor.remove(middleware)

    def discover(self) -> int:
        """Discover and register modules from configured extension directories.

        Returns:
            Number of modules discovered.
        """
        return self.registry.discover()

    def list_modules(
        self,
        tags: list[str] | None = None,
        prefix: str | None = None,
    ) -> list[str]:
        """Return sorted list of registered module IDs, optionally filtered.

        Args:
            tags: Only include modules with all specified tags.
            prefix: Only include modules whose ID starts with this prefix.

        Returns:
            Sorted list of module ID strings.
        """
        return self.registry.list(tags=tags, prefix=prefix)

    # ------------------------------------------------------------------
    # Events
    # ------------------------------------------------------------------

    @property
    def events(self) -> EventEmitter | None:
        """Access the event emitter (available when sys_modules.events is enabled).

        Returns:
            The EventEmitter instance, or None if events are not configured.
        """
        return self._sys_modules_context.get("event_emitter")

    def on(
        self,
        event_type: str,
        handler: CallableABC[[ApCoreEvent], Any],
    ) -> EventSubscriber:
        """Subscribe to events of a specific type with a simple callback.

        Creates a lightweight subscriber that filters by event_type and
        calls the provided handler. Returns the subscriber for later
        unsubscription via ``off()``.

        Args:
            event_type: Event type to listen for (e.g. "apcore.registry.module_registered").
            handler: Sync or async callable receiving an ApCoreEvent.

        Returns:
            The created EventSubscriber (for use with ``off()``).

        Raises:
            SysModulesDisabledError: If events are not enabled (code
                ``SYS_MODULES_DISABLED``). Cross-language equivalent of Rust's
                ``ModuleError(code=SysModulesDisabled)``.
        """
        emitter = self.events
        if emitter is None:
            raise SysModulesDisabledError(
                "Events are not enabled. Set sys_modules.enabled=true and sys_modules.events.enabled=true in config."
            )
        subscriber = _CallbackSubscriber(event_type=event_type, handler=handler)
        emitter.subscribe(subscriber)
        return subscriber

    def off(self, subscriber: EventSubscriber) -> None:
        """Unsubscribe a previously registered event subscriber.

        Args:
            subscriber: The subscriber returned by ``on()``.

        Raises:
            SysModulesDisabledError: If events are not enabled (code
                ``SYS_MODULES_DISABLED``).
        """
        emitter = self.events
        if emitter is None:
            raise SysModulesDisabledError(
                "Events are not enabled. Set sys_modules.enabled=true and sys_modules.events.enabled=true in config."
            )
        emitter.unsubscribe(subscriber)

    # ------------------------------------------------------------------
    # Module toggle (enable/disable)
    # ------------------------------------------------------------------

    def disable(self, module_id: str, reason: str = "Disabled via APCore client") -> dict[str, Any]:
        """Disable a module without unloading it.

        Convenience wrapper around ``system.control.toggle_feature``.

        Args:
            module_id: The module to disable.
            reason: Audit reason for the change.

        Returns:
            Result dict with ``success``, ``module_id``, ``enabled``.

        Raises:
            SysModulesDisabledError: If sys_modules are not enabled in config
                (code ``SYS_MODULES_DISABLED``).

        Cross-language note:
            TypeScript and Rust expose this as an async method.
            Python keeps it synchronous; do not ``await`` it.
        """
        self._require_sys_modules("disable")
        return self.call(
            "system.control.toggle_feature",
            {"module_id": module_id, "enabled": False, "reason": reason},
        )

    def enable(self, module_id: str, reason: str = "Enabled via APCore client") -> dict[str, Any]:
        """Re-enable a previously disabled module.

        Convenience wrapper around ``system.control.toggle_feature``.

        Args:
            module_id: The module to enable.
            reason: Audit reason for the change.

        Returns:
            Result dict with ``success``, ``module_id``, ``enabled``.

        Raises:
            SysModulesDisabledError: If sys_modules are not enabled in config
                (code ``SYS_MODULES_DISABLED``).

        Cross-language note:
            TypeScript and Rust expose this as an async method.
            Python keeps it synchronous; do not ``await`` it.
        """
        self._require_sys_modules("enable")
        return self.call(
            "system.control.toggle_feature",
            {"module_id": module_id, "enabled": True, "reason": reason},
        )

    def _require_sys_modules(self, method_name: str) -> None:
        """Raise SysModulesDisabledError if sys_modules are not configured.

        Cross-language equivalent of Rust's ``ModuleError(code=SysModulesDisabled)``.
        """
        if not self._sys_modules_context:
            raise SysModulesDisabledError(
                f"{method_name}() requires sys_modules to be enabled. "
                "Pass a Config with sys_modules.enabled=true to APCore()."
            )

    def close(self) -> None:
        """Release resources owned by this APCore instance.

        Currently closes the cached sync event loop inside the Executor.
        Safe to call multiple times. After ``close()`` the same APCore can
        continue to be used — the Executor will lazily allocate a fresh
        sync loop on the next synchronous ``call()``.
        """
        self.executor.close()


class _CallbackSubscriber:
    """Lightweight EventSubscriber that filters by event_type and delegates to a callback."""

    def __init__(
        self,
        event_type: str,
        handler: CallableABC[[ApCoreEvent], Any],
    ) -> None:
        self.event_type = event_type
        self._handler = handler
        self._is_async = asyncio.iscoroutinefunction(handler)

    async def on_event(self, event: ApCoreEvent) -> None:
        """Deliver event to handler if event_type matches."""
        if event.event_type != self.event_type:
            return
        if self._is_async:
            await self._handler(event)  # type: ignore[misc]
        else:
            self._handler(event)
