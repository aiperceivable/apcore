// APCore Protocol — Client
// Spec reference: APCore client entry point

use std::sync::Arc;

use serde_json::Value;

use crate::config::Config;
use crate::context::Context;
use crate::decorator::FunctionModule;
use crate::errors::ModuleError;
use crate::events::emitter::EventEmitter;
use crate::events::subscribers::EventSubscriber;
use crate::executor::Executor;
use crate::middleware::adapters::{AfterMiddleware, BeforeMiddleware};
use crate::middleware::base::Middleware;
use crate::middleware::manager::MiddlewareHandle;
use crate::module::ModuleAnnotations;
use crate::observability::metrics::MetricsCollector;
use crate::registry::registry::{ModuleDescriptor, Registry, DEFAULT_MODULE_VERSION};
use crate::sys_modules::{SysModulesContext, ToggleState};

/// Main entry point for interacting with the `APCore` system.
pub struct APCore {
    pub config: Config,
    /// Shared so a module this client runs can reach it for a nested call
    /// through `ctx.executor()`.
    executor: Arc<Executor>,
    /// Single shared `Arc<Registry>` used by the executor, the pipeline, and
    /// all sys modules. Interior mutability on `Registry` removes the need
    /// for `Arc::get_mut` or an external `Mutex`.
    registry: Arc<Registry>,
    // The event bus (D1-011). When sys_modules is enabled this is the SAME
    // `Arc<EventEmitter>` the sys modules + registry callbacks emit to, so
    // `on()`/`events()` receive system events. When sys_modules is disabled it
    // is `None` until `on()` lazily creates a standalone local bus (matching the
    // Python/TS contract that the emitter is surfaced only once events exist).
    // EventEmitter is interior-mutable, so subscribe/emit both work through the
    // shared `Arc` without an external lock.
    event_emitter: Option<Arc<EventEmitter>>,
    metrics_collector: Option<MetricsCollector>,
    sys_modules_context: Option<SysModulesContext>,
    /// Per-instance toggle store (issue #71). Each `APCore` owns one
    /// `Arc<ToggleState>`, injected into BOTH the pipeline `module_lookup` step
    /// (read path, via `Executor::set_toggle_state`) and `ToggleFeatureModule`
    /// (write path, via `register_sys_modules`). Disabling a module on one
    /// instance MUST NOT affect another instance in the same process.
    toggle_state: Arc<ToggleState>,
}

impl std::fmt::Debug for APCore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("APCore")
            .field("config", &self.config)
            .field("executor", &"<Executor>")
            .field("registry", &self.registry)
            .field("event_emitter", &self.event_emitter)
            .field("metrics_collector", &self.metrics_collector)
            .field(
                "sys_modules_registered",
                &self.sys_modules_context.is_some(),
            )
            .field("toggle_state", &self.toggle_state)
            .finish()
    }
}

impl Default for APCore {
    fn default() -> Self {
        Self::new()
    }
}

impl APCore {
    /// Create a new `APCore` client with default configuration.
    pub fn new() -> Self {
        Self::with_options(None, None, None, None)
    }

    /// Create a new `APCore` client with all optional parameters.
    ///
    /// A single `Arc<Registry>` is shared between the executor, the pipeline,
    /// and every built-in sys module — there is exactly one registry
    /// instance per `APCore` client.
    ///
    /// When `sys_modules.enabled` is true in the config (default: false; system
    /// modules are opt-in), built-in system modules are automatically
    /// registered into the executor pipeline.
    /// This registration is now fully synchronous and runtime-agnostic because
    /// `Registry` uses `parking_lot::RwLock` for interior mutability.
    ///
    /// **Cross-language note:** Python and TypeScript accept 4 options
    /// (registry, executor, config, metricsCollector) and use `Config.load(path)`
    /// separately — Rust's `from_path` constructor is a Rust-only convenience
    /// that does not correspond to a 5th constructor parameter in other SDKs.
    pub fn with_options(
        registry: Option<Registry>,
        executor: Option<Executor>,
        config: Option<Config>,
        metrics_collector: Option<MetricsCollector>,
    ) -> Self {
        let config = config.unwrap_or_default();

        // Resolve the shared registry: use the executor's if provided,
        // otherwise the explicit `registry` arg, otherwise a fresh default.
        let registry: Arc<Registry> = match executor.as_ref() {
            Some(e) => Arc::clone(&e.registry),
            None => Arc::new(registry.unwrap_or_default()),
        };

        // Config-driven extension roots. `extension_roots` initialises empty and
        // was previously written only by `set_extension_roots`, so a client built
        // from a `Config` carrying `extensions.root` handed an empty slice to the
        // discoverer: `discover()` returned Ok(0), silently, indistinguishable
        // from an empty directory. Python and TypeScript both resolve
        // explicit > config > default in their Registry constructors; this is the
        // Rust equivalent, placed here because `Registry::new()` takes no Config.
        // Roots already set explicitly are left alone (see the method's docs).
        registry.set_extension_roots_from_config(&config);

        // Track whether the caller supplied their own Executor: if so, respect
        // its ACL wiring as-is and skip config-driven ACL discovery below.
        let executor_supplied = executor.is_some();
        let mut executor = executor
            .unwrap_or_else(|| Executor::new(Arc::clone(&registry), Arc::new(config.clone())));

        // Issue #71: this instance owns exactly one toggle store. Bind it to the
        // executor so the pipeline `module_lookup` step (read path) consults it,
        // and hand the same Arc to `register_sys_modules` so
        // `ToggleFeatureModule` (write path) mutates the same store. Disabling a
        // module on this instance must not leak into another `APCore`.
        let toggle_state = Arc::new(ToggleState::new());
        executor.set_toggle_state(Arc::clone(&toggle_state));

        // Config-driven ACL discovery (D-64, Recommendation A). Resolve
        // `acl.root` and attach an ACL only when the path exists; a missing path
        // is a no-op that preserves today's no-enforcement default (it must NOT
        // synthesize an empty default-deny ACL). Wired before sys_modules
        // registration so system modules execute under the discovered policy.
        // A caller-supplied Executor is respected as-is and never overwritten.
        if !executor_supplied {
            match crate::acl::ACL::discover(&config) {
                Ok(Some(acl)) => executor.set_acl(acl),
                Ok(None) => {}
                Err(e) => {
                    tracing::error!(error = %e, "ACL discovery failed; continuing without ACL");
                }
            }
        }

        // Config-driven tracing (PROTOCOL_SPEC §10.1.1). `observability.tracing.*`
        // was five declared keys that reached nothing: no SDK had ever built a
        // TracingMiddleware from configuration, so `enabled: true` installed
        // nothing and the other four configured a middleware that did not exist.
        //
        // Skipped when the caller supplied their own Executor — an Executor the
        // caller built is respected as-is, tracing included, exactly as
        // config-driven ACL discovery above treats it. That is also what makes
        // §10.1.1 requirement 6 hold without a second check: the Executor built
        // here has an empty middleware chain, so configuration can never be the
        // thing that adds a SECOND tracing middleware.
        //
        // A self-contradictory configuration (requirement 3) is logged rather
        // than propagated, for the same reason `ACL::discover` above is:
        // `APCore::with_options` returns `Self` and cannot carry an error. The
        // middleware is not installed, so nothing runs under a configuration
        // this constructor could not honour.
        if !executor_supplied {
            match crate::observability::tracing_config::build_tracing_middleware(&config) {
                Ok(Some(mw)) => {
                    if let Err(e) = executor.use_middleware(Box::new(mw)) {
                        tracing::error!(error = %e, "Failed to install the configured tracing middleware");
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::error!(error = %e, "observability.tracing.* is invalid; no tracing middleware was installed");
                }
            }
        }

        let sys_modules_context = if Self::sys_modules_enabled(&config) {
            match crate::sys_modules::register_sys_modules_with_options(
                Arc::clone(&registry),
                &executor,
                &config,
                metrics_collector.clone(),
                crate::sys_modules::SysModulesOptions {
                    toggle_state: Some(Arc::clone(&toggle_state)),
                    ..Default::default()
                },
            ) {
                Ok(ctx) => Some(ctx),
                Err(e) => {
                    // Lenient default: log and continue. Callers wanting
                    // strict failure should use `register_sys_modules_with_options`
                    // with `fail_on_error: true` directly.
                    tracing::error!(error = %e, "System modules registration failed");
                    None
                }
            }
        } else {
            None
        };

        // Share ONE event bus across the client, registry callbacks, and sys
        // modules (D1-011). When sys_modules is enabled, reuse its emitter so
        // `on()`/`events()` receive module-registration and toggle events; when
        // disabled, leave it `None` (a standalone bus is created lazily on `on()`).
        let event_emitter = sys_modules_context.as_ref().map(|c| Arc::clone(&c.emitter));

        // Thread the shared event bus into the auto-created executor so the
        // pipeline publishes governance events (apcore#77:
        // `apcore.approval.decision` / `apcore.policy.override` /
        // `apcore.acl.denied`) to this instance's `on()` subscribers. A
        // caller-supplied executor keeps its own wiring (parity with ACL
        // discovery above).
        if !executor_supplied {
            if let Some(ref emitter) = event_emitter {
                executor.set_event_emitter(Some(Arc::clone(emitter)));
            }
        }

        Self {
            config,
            executor: executor.into_shared(),
            registry,
            event_emitter,
            metrics_collector,
            sys_modules_context,
            toggle_state,
        }
    }

    /// Shared handle to this instance's per-instance toggle store (issue #71).
    ///
    /// The returned `Arc<ToggleState>` is the SAME store consulted by the
    /// pipeline `module_lookup` read path and mutated by the
    /// `ToggleFeatureModule` write path for this `APCore`. Reading
    /// `toggle_state().is_disabled(id)` reflects this instance's view, isolated
    /// from other `APCore` instances in the same process.
    #[must_use]
    pub fn toggle_state(&self) -> Arc<ToggleState> {
        Arc::clone(&self.toggle_state)
    }

    /// Return whether the `sys_modules` auto-registration is enabled
    /// according to the given config. Defaults to `false` (opt-in) when the
    /// key is absent, matching the spec and the Python/TS peers
    /// (registration.py:335 / registration.ts:250). Kept consistent with
    /// `register_sys_modules` (sync finding A-D-11).
    fn sys_modules_enabled(config: &Config) -> bool {
        config
            .get("sys_modules.enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    /// Create a new `APCore` client with the given configuration.
    pub fn with_config(config: Config) -> Self {
        Self::with_options(None, None, Some(config), None)
    }

    /// Create a new `APCore` client from a pre-built Registry and Executor.
    ///
    /// Builds an `Executor` from the given `registry` and a default `Config`.
    /// To supply a custom config, use [`Self::with_options`] instead.
    pub fn with_components(registry: Registry, config: Config) -> Self {
        Self::with_options(Some(registry), None, Some(config), None)
    }

    /// Create a new `APCore` client from a configuration file path.
    pub fn from_path(path: impl AsRef<std::path::Path>) -> Result<Self, ModuleError> {
        let config = Config::load(path.as_ref())?;
        Ok(Self::with_config(config))
    }

    /// Register a function as a module — convenience wrapper that creates a
    /// [`FunctionModule`] and registers it in a single call.
    ///
    /// This mirrors the `module()` helper available in the Python and TypeScript
    /// SDKs. The handler closure must be an async function that takes
    /// `(serde_json::Value, &Context<serde_json::Value>)` and returns
    /// `Result<serde_json::Value, ModuleError>`.
    ///
    /// # Example
    ///
    /// ```ignore
    /// client.module(
    ///     "math.add",
    ///     "Add two numbers",
    ///     serde_json::json!({"type": "object"}),
    ///     serde_json::json!({"type": "object"}),
    ///     None,  // documentation
    ///     vec![], // tags
    ///     None,  // version
    ///     None,  // metadata
    ///     vec![], // examples
    ///     |inputs, _ctx| Box::pin(async move { Ok(inputs) }),
    /// )?;
    /// ```
    #[allow(clippy::too_many_arguments)]
    pub fn module<F>(
        &mut self,
        module_id: &str,
        description: &str,
        input_schema: serde_json::Value,
        output_schema: serde_json::Value,
        documentation: Option<String>,
        tags: Vec<String>,
        version: Option<&str>,
        metadata: Option<std::collections::HashMap<String, serde_json::Value>>,
        examples: Vec<crate::module::ModuleExample>,
        display: Option<&serde_json::Value>,
        handler: F,
    ) -> Result<&mut Self, ModuleError>
    where
        F: for<'a> Fn(
                serde_json::Value,
                &'a Context<serde_json::Value>,
            ) -> std::pin::Pin<
                Box<
                    dyn std::future::Future<Output = Result<serde_json::Value, ModuleError>>
                        + Send
                        + 'a,
                >,
            > + Send
            + Sync
            + 'static,
    {
        let resolved_version = version.unwrap_or(DEFAULT_MODULE_VERSION).to_string();
        let resolved_metadata = metadata.unwrap_or_default();
        let descriptor = ModuleDescriptor {
            module_id: module_id.to_string(),
            name: None,
            description: description.to_string(),
            documentation: documentation.clone(),
            input_schema: input_schema.clone(),
            output_schema: output_schema.clone(),
            version: resolved_version.clone(),
            tags: tags.clone(),
            annotations: Some(ModuleAnnotations::default()),
            examples: examples.clone(),
            metadata: resolved_metadata.clone(),
            display: display.cloned(),
            sunset_date: None,
            dependencies: vec![],
            enabled: true,
        };
        let func_module = FunctionModule::with_description(
            ModuleAnnotations::default(),
            input_schema,
            output_schema,
            description,
            documentation,
            tags,
            &resolved_version,
            resolved_metadata,
            examples,
            handler,
        );
        self.registry
            .register(module_id, Box::new(func_module), descriptor)?;
        Ok(self)
    }

    /// Call (execute) a module by ID with the given inputs.
    ///
    /// `inputs` accepts either a `serde_json::Value` directly or `None`.
    /// When `None` is passed, an empty JSON object `{}` is used, matching
    /// the Python and TypeScript SDKs where inputs can be `None`/`undefined`.
    ///
    /// In Rust, `call()` is already async — there is no separate sync variant.
    /// This method is the equivalent of both `call()` and `call_async()` in the
    /// Python and TypeScript SDKs.
    #[doc(alias = "call_async")]
    pub async fn call(
        &self,
        module_id: &str,
        inputs: impl Into<Option<serde_json::Value>>,
        ctx: Option<&Context<serde_json::Value>>,
        version_hint: Option<&str>,
    ) -> Result<serde_json::Value, ModuleError> {
        let resolved_inputs = inputs.into().unwrap_or_else(|| serde_json::json!({}));
        self.executor
            .call(module_id, resolved_inputs, ctx, version_hint)
            .await
    }

    /// Validate module inputs without executing (spec §12.3).
    ///
    /// `inputs` is optional -- pass `None` to validate with an empty `{}`
    /// object, matching the Python and TypeScript SDKs where inputs can be
    /// `None`/`undefined`. A `&serde_json::Value` reference is also accepted
    /// directly for backward compatibility.
    ///
    /// Returns a `PreflightResult` with per-check status. `ctx` enables
    /// call-chain checks against real caller state; pass `None` to validate
    /// from an anonymous external context.
    pub async fn validate<'v>(
        &self,
        module_id: &str,
        inputs: impl Into<Option<&'v serde_json::Value>>,
        ctx: Option<&crate::context::Context<serde_json::Value>>,
    ) -> Result<crate::module::PreflightResult, ModuleError> {
        let empty = serde_json::json!({});
        let resolved_inputs = inputs.into().unwrap_or(&empty);
        self.executor
            .validate(module_id, resolved_inputs, ctx)
            .await
    }

    /// Register a module with the given `module_id`.
    pub fn register(
        &self,
        module_id: &str,
        module: Box<dyn crate::module::Module>,
    ) -> Result<(), ModuleError> {
        let descriptor = ModuleDescriptor {
            module_id: module_id.to_string(),
            name: None,
            description: module.description().to_string(),
            documentation: None,
            input_schema: module.input_schema(),
            output_schema: module.output_schema(),
            version: "1.0.0".to_string(),
            tags: vec![],
            annotations: Some(ModuleAnnotations::default()),
            examples: vec![],
            metadata: std::collections::HashMap::new(),
            display: None,
            sunset_date: None,
            dependencies: vec![],
            enabled: true,
        };
        self.registry.register(module_id, module, descriptor)
    }

    /// Trigger module discovery from configured extension directories.
    ///
    /// Returns the number of newly discovered modules, or 0 if no discoverer
    /// is configured on the registry.
    pub async fn discover(&self) -> Result<usize, ModuleError> {
        match self.registry.discover_internal().await {
            Ok(count) => Ok(count),
            Err(e) if e.code == crate::errors::ErrorCode::NoDiscovererConfigured => {
                // No discoverer configured — not an error, just nothing to discover.
                Ok(0)
            }
            Err(e) => Err(e),
        }
    }

    /// List all registered modules, optionally filtered by tags and/or prefix.
    pub fn list_modules(&self, tags: Option<&[&str]>, prefix: Option<&str>) -> Vec<String> {
        self.registry.list(tags, prefix, None)
    }

    /// Register a middleware in the execution pipeline.
    ///
    /// Returns an error if the middleware's priority exceeds the allowed range.
    /// Returns `&Self` for chaining. Takes `&self` — the underlying
    /// `MiddlewareManager` uses interior mutability, so middleware can be
    /// added after `APCore` has been shared behind an `Arc` or a shared
    /// reference.
    ///
    /// **Cross-language note:** The Python and TypeScript SDKs expose this
    /// method as `use()`. Rust names it `use_middleware` because `use` is a
    /// reserved keyword.
    #[track_caller]
    pub fn use_middleware(
        &self,
        middleware: Box<dyn Middleware>,
    ) -> Result<&Self, crate::errors::ModuleError> {
        self.executor.use_middleware(middleware)?;
        Ok(self)
    }

    /// Add a middleware and keep a token that names exactly this registration.
    ///
    /// The chaining form above returns `&Self`, which is what most call sites
    /// want; this one returns the [`MiddlewareHandle`] instead, for a caller
    /// that intends to remove this specific middleware later via
    /// [`remove_handle`](Self::remove_handle).
    ///
    /// `Contract: APCore.remove` removes by IDENTITY — apcore-python and
    /// apcore-typescript take the middleware object back and compare it with
    /// `is` / `===`. Rust cannot: `use_middleware` consumes the `Box`, so the
    /// caller has no object left to hand back. The handle is that identity.
    /// It matters because duplicate registration only WARNS — it never fails —
    /// so two instances answering the same `name()` is a reachable state, and
    /// [`remove`](Self::remove) drops whichever comes first in pipeline order
    /// rather than the one the caller meant (sync finding A-C-001).
    #[track_caller]
    pub fn use_middleware_handle(
        &self,
        middleware: Box<dyn Middleware>,
    ) -> Result<MiddlewareHandle, crate::errors::ModuleError> {
        self.executor.use_middleware(middleware)
    }

    /// Remove exactly the middleware that
    /// [`use_middleware_handle`](Self::use_middleware_handle) returned `handle`
    /// for. Returns `false` if it is no longer registered — idempotent, like
    /// the two peer SDKs' `remove`.
    pub fn remove_handle(&self, handle: MiddlewareHandle) -> bool {
        self.executor.remove_handle(handle)
    }

    /// Remove a middleware by its name string.
    ///
    /// This is a Rust-specific convenience that accepts a `&str` directly.
    /// For an API closer to Python/TypeScript (which accept the middleware
    /// object), see [`remove_middleware`](Self::remove_middleware).
    ///
    /// **Deprecation notice (D1-006):** the Python and TypeScript SDKs expose
    /// `remove(middleware)` accepting the middleware object — not a name string.
    /// Prefer [`remove_middleware`](Self::remove_middleware) for cross-language
    /// consistency. The name-based form will be removed in a future major release.
    pub fn remove(&self, name: &str) -> bool {
        self.executor.remove(name)
    }

    /// Remove a middleware by reference, extracting its name via
    /// [`Middleware::name()`].
    ///
    /// This mirrors the shape of the Python and TypeScript `remove(middleware)`
    /// API, which accept the middleware object directly — but not its
    /// semantics: the middleware is matched by its
    /// [`name()`](Middleware::name), so with two instances answering the same
    /// name it removes whichever comes first in pipeline order, not the one
    /// passed in.
    ///
    /// The reason is NOT that trait objects cannot be compared by identity —
    /// `Arc::ptr_eq` has ignored vtable metadata since Rust 1.76, below this
    /// crate's MSRV. It is that [`use_middleware`](Self::use_middleware)
    /// consumes the `Box`, so by the time a caller wants to remove one it holds
    /// no pointer to compare against. For identity-exact removal, register with
    /// [`use_middleware_handle`](Self::use_middleware_handle) and remove with
    /// [`remove_handle`](Self::remove_handle) (sync finding A-C-001).
    pub fn remove_middleware(&self, middleware: &dyn Middleware) -> bool {
        self.executor.remove(middleware.name())
    }

    /// Get a reference to the registry.
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Get the shared registry as an `Arc<Registry>` (for handing out to
    /// components that need to share ownership).
    pub fn registry_arc(&self) -> Arc<Registry> {
        Arc::clone(&self.registry)
    }

    /// Get a reference to the executor.
    pub fn executor(&self) -> &Executor {
        &self.executor
    }

    /// Get a shared handle to the executor, e.g. for an
    /// [`AsyncTaskManager`](crate::AsyncTaskManager).
    #[must_use]
    pub fn executor_arc(&self) -> Arc<Executor> {
        Arc::clone(&self.executor)
    }

    /// Disable a module by routing through the executor pipeline.
    ///
    /// This calls `system.control.toggle_feature` through the executor,
    /// matching the Python and TypeScript SDK behavior (events, ACL, and
    /// middleware are applied).
    ///
    /// **Cross-language note:** Python and TypeScript SDKs accept a `reason`
    /// string with a default value (e.g. `reason=""` or `reason=None`). In Rust,
    /// `reason` is `Option<&str>` — pass `None` to use the default message, or
    /// `Some("my reason")` to provide a custom one.
    pub async fn disable(
        &self,
        module_id: &str,
        reason: Option<&str>,
    ) -> Result<Value, ModuleError> {
        if self.sys_modules_context.is_none() {
            return Err(ModuleError::new(
                crate::errors::ErrorCode::SysModulesDisabled,
                "disable() requires sys_modules to be enabled. \
                 Pass a Config with sys_modules.enabled=true to APCore::new().",
            ));
        }
        self.executor
            .call(
                "system.control.toggle_feature",
                serde_json::json!({
                    "module_id": module_id,
                    "enabled": false,
                    "reason": reason.unwrap_or("Disabled via APCore client")
                }),
                None,
                None,
            )
            .await
    }

    /// Re-enable a previously disabled module by routing through the executor pipeline.
    ///
    /// This calls `system.control.toggle_feature` through the executor,
    /// matching the Python and TypeScript SDK behavior (events, ACL, and
    /// middleware are applied).
    ///
    /// **Cross-language note:** Python and TypeScript SDKs accept a `reason`
    /// string with a default value (e.g. `reason=""` or `reason=None`). In Rust,
    /// `reason` is `Option<&str>` — pass `None` to use the default message, or
    /// `Some("my reason")` to provide a custom one.
    pub async fn enable(
        &self,
        module_id: &str,
        reason: Option<&str>,
    ) -> Result<Value, ModuleError> {
        if self.sys_modules_context.is_none() {
            return Err(ModuleError::new(
                crate::errors::ErrorCode::SysModulesDisabled,
                "enable() requires sys_modules to be enabled. \
                 Pass a Config with sys_modules.enabled=true to APCore::new().",
            ));
        }
        self.executor
            .call(
                "system.control.toggle_feature",
                serde_json::json!({
                    "module_id": module_id,
                    "enabled": true,
                    "reason": reason.unwrap_or("Enabled via APCore client")
                }),
                None,
                None,
            )
            .await
    }

    /// Subscribe to an event type using a closure. Returns the subscriber ID string.
    ///
    /// Matches the Python and TypeScript `on(event_type, callback)` API. The
    /// closure receives each matching [`ApCoreEvent`](crate::events::emitter::ApCoreEvent)
    /// by reference. The returned ID can be passed to [`off()`](Self::off) to
    /// unsubscribe the specific handler, or use [`off_by_type()`](Self::off_by_type)
    /// to remove all handlers for an event type at once.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::SysModulesDisabled`](crate::errors::ErrorCode::SysModulesDisabled)
    /// when no event bus is configured. This mirrors apcore-python and
    /// apcore-typescript, both of which raise `SysModulesDisabledError` from
    /// `on()` when events are not enabled, and the `APCore.on` contract in
    /// `docs/features/apcore-client.md`. Earlier versions lazily created a
    /// standalone local bus here instead, which made a subscription against a
    /// misconfigured client look like it had succeeded while no framework event
    /// would ever reach it.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let id = client.on("apcore.*", |event| {
    ///     println!("Got event: {}", event.event_type);
    /// });
    /// client.off(&id); // unsubscribe by ID
    /// client.off_by_type("apcore.*"); // or unsubscribe all for this type
    /// ```
    pub fn on(
        &mut self,
        event_type: &str,
        callback: impl Fn(&crate::events::emitter::ApCoreEvent) + Send + Sync + 'static,
    ) -> Result<String, ModuleError> {
        let subscriber = Box::new(ClosureSubscriber {
            id: uuid::Uuid::new_v4().to_string(),
            callback: Box::new(callback),
        });
        self.on_subscriber(event_type, subscriber)
    }

    /// Subscribe to an event type using a boxed [`EventSubscriber`]. Returns the subscriber ID.
    ///
    /// Use this when you need a custom [`EventSubscriber`] implementation. For
    /// closure-based subscriptions (the common case), prefer [`on()`](Self::on)
    /// which matches the Python/TypeScript API shape.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::SysModulesDisabled`](crate::errors::ErrorCode::SysModulesDisabled)
    /// when no event bus is configured — same condition as [`on()`](Self::on).
    pub fn on_subscriber(
        &mut self,
        event_type: &str,
        subscriber: Box<dyn EventSubscriber>,
    ) -> Result<String, ModuleError> {
        let emitter = Self::require_event_emitter(self.event_emitter.as_ref(), "on()")?;
        let wrapped = Box::new(EventTypeSubscriber {
            event_type: event_type.to_string(),
            inner: subscriber,
        });
        let id = wrapped.subscriber_id().to_string();
        emitter.subscribe(wrapped);
        Ok(id)
    }

    /// Shared guard for the event-subscription surface.
    ///
    /// Returns the configured emitter, or `SysModulesDisabled` when there is
    /// none. Mirrors the `sys_modules_context.is_none()` guard that
    /// [`disable()`](Self::disable) / [`enable()`](Self::enable) already apply,
    /// and the `self.events is None` guard in apcore-python's `on`/`off`.
    fn require_event_emitter<'a>(
        emitter: Option<&'a Arc<EventEmitter>>,
        method: &str,
    ) -> Result<&'a Arc<EventEmitter>, ModuleError> {
        emitter.ok_or_else(|| {
            ModuleError::new(
                crate::errors::ErrorCode::SysModulesDisabled,
                format!(
                    "{method} requires events to be enabled. Pass a Config with \
                     sys_modules.enabled=true and sys_modules.events.enabled=true to APCore."
                ),
            )
        })
    }

    /// Deprecated: use [`on()`](Self::on) instead.
    ///
    /// `on_fn(event_type, callback)` was renamed to `on(event_type, callback)` in 0.19.0
    /// to align with the Python and TypeScript SDK naming convention.
    #[deprecated(
        since = "0.19.0",
        note = "Renamed to `on()`. Use `client.on(event_type, callback)` instead."
    )]
    pub fn on_fn(
        &mut self,
        event_type: &str,
        callback: impl Fn(&crate::events::emitter::ApCoreEvent) + Send + Sync + 'static,
    ) -> Result<String, ModuleError> {
        self.on(event_type, callback)
    }

    /// Unsubscribe a single handler by its subscriber ID.
    ///
    /// The ID is the string returned by [`on()`](Self::on). Returns `true` if
    /// a matching subscriber was found and removed.
    ///
    /// To remove all handlers bound to a given event type, use
    /// [`off_by_type()`](Self::off_by_type).
    ///
    /// # Errors
    ///
    /// [`ErrorCode::SysModulesDisabled`](crate::errors::ErrorCode::SysModulesDisabled)
    /// when no event bus is configured, matching apcore-python and
    /// apcore-typescript, which raise `SysModulesDisabledError` from `off()`.
    /// The `bool` payload is retained (Python/TypeScript return void): "no such
    /// subscriber" and "events are off" are different answers, and collapsing
    /// the first into the second is what let a misconfigured client look
    /// merely empty.
    pub fn off(&mut self, subscriber_id: &str) -> Result<bool, ModuleError> {
        let emitter = Self::require_event_emitter(self.event_emitter.as_ref(), "off()")?;
        Ok(emitter.unsubscribe_by_id(subscriber_id))
    }

    /// Unsubscribe all handlers registered for `event_type`.
    ///
    /// Matches Python/TypeScript `off(event_type)` semantics — passing an event
    /// type string removes every handler bound to that type in a single call.
    /// Returns the number of handlers removed.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::SysModulesDisabled`](crate::errors::ErrorCode::SysModulesDisabled)
    /// when no event bus is configured — same guard as [`off()`](Self::off),
    /// applied here so the whole subscription surface answers consistently
    /// rather than leaving one entry point silently returning `0`.
    pub fn off_by_type(&mut self, event_type: &str) -> Result<usize, ModuleError> {
        let emitter = Self::require_event_emitter(self.event_emitter.as_ref(), "off_by_type()")?;
        Ok(emitter.unsubscribe_by_event_type(event_type))
    }

    /// Get this client's event bus, if one exists.
    ///
    /// D1-011 resolved: when system modules are enabled, this returns the SAME
    /// bus that emits `apcore.module.toggled`, `apcore.registry.module_registered`,
    /// and the other system events — so a subscriber added via [`on()`](Self::on)
    /// receives them, matching the Python and TypeScript SDKs which surface the
    /// sys-modules emitter here. With sys_modules disabled it is `None` until the
    /// first [`on()`](Self::on) call creates a standalone local bus.
    pub fn events(&self) -> Option<&EventEmitter> {
        self.event_emitter.as_deref()
    }

    /// Reload configuration from disk.
    ///
    /// Reloads the `Config` object from its source path. Module re-discovery
    /// is not triggered; call [`APCore::discover`] explicitly after `reload`
    /// if you need fresh module discovery.
    ///
    /// **Rust-only convenience** — Python and TypeScript expose config reload
    /// only on the Config object itself, not on the client facade.
    pub fn reload(&mut self) -> Result<(), ModuleError> {
        self.config.reload()?;
        Ok(())
    }

    /// Stream execution of a module.
    ///
    /// `inputs` accepts either a `serde_json::Value` directly or `None`.
    /// When `None` is passed, an empty JSON object `{}` is used, matching
    /// the Python and TypeScript SDKs where inputs can be `None`/`undefined`.
    ///
    /// Returns an async `Stream` of chunks. Each chunk is delivered to the
    /// caller as soon as it is produced by the underlying module -- true
    /// incremental streaming, no buffering. Phase 3 validation runs after
    /// the inner stream is exhausted; the chunks are already delivered, so a
    /// failure is not yielded but published as
    /// `apcore.stream.post_validation_failed`. See [`Executor::stream`].
    pub fn stream<'a>(
        &'a self,
        module_id: &str,
        inputs: impl Into<Option<Value>>,
        ctx: Option<&Context<Value>>,
        version_hint: Option<&str>,
    ) -> std::pin::Pin<Box<dyn futures_core::Stream<Item = Result<Value, ModuleError>> + Send + 'a>>
    {
        let resolved_inputs = inputs.into().unwrap_or_else(|| serde_json::json!({}));
        self.executor
            .stream(module_id, resolved_inputs, ctx, version_hint)
    }

    /// Describe a module by ID.
    ///
    /// Delegates to [`Registry::describe`](crate::registry::registry::Registry::describe),
    /// which builds the cross-SDK Markdown envelope. Per the spec contract,
    /// `describe` MUST raise `ModuleNotFoundError` for a missing module rather
    /// than returning a sentinel string — matching apcore-python /
    /// apcore-typescript, whose `APCore.describe` is the same thin delegation.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::ModuleNotFound`](crate::errors::ErrorCode::ModuleNotFound)
    /// when `module_id` is not registered.
    pub fn describe(&self, module_id: &str) -> Result<String, ModuleError> {
        self.registry.describe(module_id)
    }

    /// Add a before callback middleware. Returns `&Self` for chaining.
    pub fn use_before(
        &self,
        middleware: Box<dyn BeforeMiddleware>,
    ) -> Result<&Self, crate::errors::ModuleError> {
        self.executor.use_before(middleware)?;
        Ok(self)
    }

    /// Add an after callback middleware. Returns `&Self` for chaining.
    pub fn use_after(
        &self,
        middleware: Box<dyn AfterMiddleware>,
    ) -> Result<&Self, crate::errors::ModuleError> {
        self.executor.use_after(middleware)?;
        Ok(self)
    }
}

/// Internal subscriber that wraps a plain closure for [`APCore::on_fn()`].
struct ClosureSubscriber {
    id: String,
    callback: Box<dyn Fn(&crate::events::emitter::ApCoreEvent) + Send + Sync>,
}

impl std::fmt::Debug for ClosureSubscriber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClosureSubscriber")
            .field("id", &self.id)
            .field("callback", &"<closure>")
            .finish()
    }
}

#[async_trait::async_trait]
impl EventSubscriber for ClosureSubscriber {
    fn subscriber_id(&self) -> &str {
        &self.id
    }

    fn event_pattern(&self) -> &'static str {
        "*"
    }

    async fn on_event(
        &self,
        event: &crate::events::emitter::ApCoreEvent,
    ) -> Result<(), ModuleError> {
        (self.callback)(event);
        Ok(())
    }
}

/// Wrapper that binds an `event_type` pattern to an inner subscriber,
/// overriding its `event_pattern()` so the emitter only delivers matching events.
#[derive(Debug)]
struct EventTypeSubscriber {
    event_type: String,
    inner: Box<dyn EventSubscriber>,
}

#[async_trait::async_trait]
impl EventSubscriber for EventTypeSubscriber {
    fn subscriber_id(&self) -> &str {
        self.inner.subscriber_id()
    }

    fn event_pattern(&self) -> &str {
        &self.event_type
    }

    fn event_type_filter(&self) -> Option<&str> {
        Some(&self.event_type)
    }

    async fn on_event(
        &self,
        event: &crate::events::emitter::ApCoreEvent,
    ) -> Result<(), ModuleError> {
        self.inner.on_event(event).await
    }
}
