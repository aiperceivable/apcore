//! Async hook dispatch and registration with the Registry's publication rules.

use std::collections::HashMap;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use futures_util::FutureExt;
use serde_json::{json, Value};

use crate::errors::{ErrorCode, ModuleError};
use crate::lifecycle::{
    lifecycle_worker, LifecycleHook, LifecycleOperation, LifecyclePhase, LIFECYCLE_CALLBACK,
    REGISTRY_CALLBACK,
};
use crate::module::Module;

use super::{
    DiscoveredModule, Discoverer, ModuleDescriptor, ModuleReferences, RegistrationOptions,
    Registry, RemovalCompletion, RemovedRegistration,
};

#[derive(Clone)]
pub(super) struct HookScope {
    pub(super) registry: usize,
    pub(super) operation: Option<String>,
}

fn hook_panicked() -> ModuleError {
    ModuleError::new(ErrorCode::GeneralInternalError, "Lifecycle hook panicked")
}

pub(super) fn requires_async_registration(name: &str) -> ModuleError {
    let mut error = ModuleError::new(
        ErrorCode::ModuleLoadError,
        format!("Module '{name}' requires asynchronous registration"),
    );
    error.details.insert("phase".to_owned(), json!("loading"));
    error
}

impl HookScope {
    async fn execute<T>(
        &self,
        hook: impl Future<Output = Result<T, ModuleError>>,
    ) -> Result<T, ModuleError> {
        let scoped = REGISTRY_CALLBACK.scope(self.registry, AssertUnwindSafe(hook).catch_unwind());
        match &self.operation {
            Some(id) => LIFECYCLE_CALLBACK.scope(id.clone(), scoped).await,
            None => scoped.await,
        }
        .unwrap_or_else(|_| Err(hook_panicked()))
    }

    async fn blocking<T: Send + 'static>(
        &self,
        hook: impl FnOnce() -> Result<T, ModuleError> + Send + 'static,
    ) -> Result<T, ModuleError> {
        let scope = self.clone();
        tokio::task::spawn_blocking(move || {
            let hook = || REGISTRY_CALLBACK.sync_scope(scope.registry, hook);
            match scope.operation {
                Some(id) => LIFECYCLE_CALLBACK.sync_scope(id, hook),
                None => hook(),
            }
        })
        .await
        .unwrap_or_else(|_| Err(hook_panicked()))
    }

    async fn load(&self, module: &Arc<dyn Module>) -> Result<(), ModuleError> {
        if let Some(adapter) = module.as_async_lifecycle() {
            self.execute(adapter.on_load_async()).await
        } else {
            let module = Arc::clone(module);
            self.blocking(move || module.on_load()).await
        }
    }

    async fn unload(&self, module: &Arc<dyn Module>) -> Result<(), ModuleError> {
        if let Some(adapter) = module.as_async_lifecycle() {
            self.execute(adapter.on_unload_async()).await
        } else {
            let module = Arc::clone(module);
            self.blocking(move || {
                module.on_unload();
                Ok(())
            })
            .await
        }
    }

    pub(super) async fn suspend(&self, module: &Arc<dyn Module>) -> Option<Value> {
        let result = if let Some(adapter) = module.as_async_lifecycle() {
            self.execute(adapter.on_suspend_async()).await
        } else {
            let module = Arc::clone(module);
            self.blocking(move || Ok(module.on_suspend())).await
        };
        result.unwrap_or_else(|error| {
            tracing::warn!(code = %error.wire_code(), "Module suspension failed");
            None
        })
    }

    async fn resume(&self, module: &Arc<dyn Module>, state: Value) {
        let result = if let Some(adapter) = module.as_async_lifecycle() {
            self.execute(adapter.on_resume_async(state)).await
        } else {
            let module = Arc::clone(module);
            self.blocking(move || {
                module.on_resume(state);
                Ok(())
            })
            .await
        };
        if let Err(error) = result {
            tracing::warn!(code = %error.wire_code(), "Module state restoration failed");
        }
    }

    pub(super) async fn unload_registration(
        &self,
        removed: &RemovedRegistration,
    ) -> Result<(), ModuleError> {
        let mut first = self.unload(&removed.primary).await;
        for version in &removed.older_versions {
            first = first.and(self.unload(&version.module).await);
        }
        first
    }

    pub(super) async fn run_callbacks(&self, callback: impl FnOnce() + Send + 'static) {
        let _ = self
            .blocking(move || {
                callback();
                Ok(())
            })
            .await;
    }
}

pub(super) type HookProgress<'a> = Option<(&'a LifecycleOperation, usize)>;

fn stage(progress: HookProgress<'_>, phase: LifecyclePhase, hook: Option<LifecycleHook>) {
    if let Some((operation, index)) = progress {
        operation.update(|report| {
            report.modules[index].phase = phase;
            report.modules[index].active_hook = hook;
        });
    }
}

impl Registry {
    pub(super) fn hook_scope(&self) -> HookScope {
        HookScope {
            registry: Arc::as_ptr(&self.core) as usize,
            operation: None,
        }
    }

    pub(super) async fn finish_detached_removal(
        &self,
        removed: RemovedRegistration,
        completion: RemovalCompletion,
        suspend: bool,
        runtime: &tokio::runtime::Handle,
    ) -> Result<(RemovedRegistration, Option<Value>), ModuleError> {
        let name = completion.name.clone();
        // A detached registration and its pending-removal marker now belong
        // to the owner, even when the API caller drops its wait future.
        let scope = self.hook_scope();
        let callbacks = Arc::clone(&self.callbacks);
        let emitter = Arc::clone(&self.event_emitter);
        runtime.spawn(async move {
            let _completion = completion;
            let state = if suspend { scope.suspend(&removed.primary).await } else { None };
            if let Err(error) = scope.unload_registration(&removed).await {
                tracing::error!(module_id = name, code = %error.wire_code(), "Module unload failed");
            }
            let primary = Arc::clone(&removed.primary);
            scope.run_callbacks(move || {
                Self::notify_removal_callbacks(&callbacks, &emitter, &name, primary.as_ref());
            }).await;
            (removed, state)
        }).await.map_err(|_| {
            ModuleError::new(ErrorCode::GeneralInternalError, "Detached module cleanup did not complete")
        })
    }

    /// Discover modules and allow programs using typed async hooks to load.
    pub async fn discover_async(
        self: &Arc<Self>,
        discoverer: &dyn Discoverer,
    ) -> Result<usize, ModuleError> {
        if self.core.read().closed {
            return Err(Self::closed_error());
        }
        let roots = self.extension_roots.read().clone();
        let candidates = discoverer.discover(&roots).await?;
        self.register_discovered_owned(candidates, HashMap::new())
            .await
    }

    /// Discover from the configured source and select typed async hooks.
    pub async fn discover_internal_async(self: &Arc<Self>) -> Result<usize, ModuleError> {
        self.discover_with_resume(&HashMap::new()).await
    }

    pub(super) async fn register_discovered_owned(
        self: &Arc<Self>,
        discovered: Vec<DiscoveredModule>,
        states: HashMap<String, Value>,
    ) -> Result<usize, ModuleError> {
        let ids = discovered.iter().map(|entry| entry.name.clone()).collect();
        if !discovered
            .iter()
            .any(|entry| entry.module.as_async_lifecycle().is_some())
        {
            return Ok(self
                .register_discovered_batch(discovered, &ids, &states, false)
                .await);
        }
        let registry = Arc::clone(self);
        let runtime = crate::lifecycle::lifecycle_runtime()?;
        runtime
            .spawn(async move {
                registry
                    .register_discovered_batch(discovered, &ids, &states, true)
                    .await
            })
            .await
            .map_err(|_| {
                ModuleError::new(
                    ErrorCode::ModuleLoadError,
                    "Discovery owner did not complete",
                )
            })
    }

    /// Register a module using its typed async hooks when exposed.
    ///
    /// The Registry must be shared so the load can finish if the caller leaves.
    pub async fn register_async(
        self: &Arc<Self>,
        name: &str,
        module: Box<dyn Module>,
    ) -> Result<(), ModuleError> {
        let registry = Arc::clone(self);
        let module: Arc<dyn Module> = module.into();
        let descriptor = Self::descriptor_from_module(name, module.as_ref());
        let name = name.to_owned();
        let runtime = crate::lifecycle::lifecycle_runtime()?;
        runtime
            .spawn(async move {
                registry
                    .load_with_hooks(
                        module,
                        descriptor,
                        None,
                        RegistrationOptions {
                            allow_reserved: false,
                            run_validator: true,
                            context: None,
                            resume: None,
                        },
                        None,
                    )
                    .await
            })
            .await
            .unwrap_or_else(|_| {
                Err(ModuleError::new(
                    ErrorCode::ModuleLoadError,
                    format!("Registration owner for '{name}' did not complete"),
                ))
            })
    }

    /// Register modules in order, allowing typed async lifecycle hooks.
    pub async fn register_many_async<I, S>(self: &Arc<Self>, modules: I) -> Result<(), ModuleError>
    where
        I: IntoIterator<Item = (S, Box<dyn Module>)>,
        S: AsRef<str>,
    {
        for (name, module) in modules {
            self.register_async(name.as_ref(), module).await?;
        }
        Ok(())
    }

    pub(super) async fn load_discovered_with_hooks(
        &self,
        candidate: DiscoveredModule,
        state: Option<Value>,
        progress: HookProgress<'_>,
    ) -> Result<(), ModuleError> {
        if !Self::descriptor_schema_shape_is_valid(&candidate.descriptor) {
            return Err(ModuleError::new(
                ErrorCode::ModuleLoadError,
                "Discovered schemas must be objects",
            ));
        }
        if candidate.name != candidate.descriptor.module_id
            || super::is_ephemeral_module_id(&candidate.name)
        {
            return Err(ModuleError::new(
                ErrorCode::ModuleLoadError,
                "Invalid discovered module ID",
            ));
        }
        Self::check_discovered_dependencies(&self.core.read(), &candidate)?;
        self.load_with_hooks(
            candidate.module,
            candidate.descriptor,
            Some(candidate.source),
            RegistrationOptions {
                allow_reserved: false,
                run_validator: true,
                context: None,
                resume: state,
            },
            progress,
        )
        .await
    }

    pub(super) async fn load_with_hooks(
        &self,
        module: Arc<dyn Module>,
        mut descriptor: ModuleDescriptor,
        source: Option<String>,
        options: RegistrationOptions<'_>,
        progress: HookProgress<'_>,
    ) -> Result<(), ModuleError> {
        let name = descriptor.module_id.clone();
        let async_runtime = module
            .as_async_lifecycle()
            .map(|_| lifecycle_worker())
            .transpose()?;
        self.check_registration(
            &name,
            &module,
            &mut descriptor,
            options.allow_reserved,
            options.run_validator,
        )?;
        let scope = HookScope {
            registry: Arc::as_ptr(&self.core) as usize,
            operation: progress.map(|(operation, _)| operation.operation_id()),
        };
        let reservation =
            self.reserve_registration(&name, options.allow_reserved, scope.operation.as_deref())?;
        stage(
            progress,
            LifecyclePhase::Loading,
            Some(LifecycleHook::OnLoad),
        );
        if let Err(error) = scope.load(&module).await {
            stage(progress, LifecyclePhase::Loading, None);
            drop(reservation);
            if progress.is_none() {
                self.notify_discovered_load_failed(&name, &error);
            }
            return Err(error);
        }
        if let Some(state) = options.resume {
            stage(
                progress,
                LifecyclePhase::Resuming,
                Some(LifecycleHook::OnResume),
            );
            scope.resume(&module, state).await;
        }

        let result = self.publish_loaded(
            &name,
            &module,
            descriptor,
            source,
            scope.operation.as_deref(),
            async_runtime,
        );
        if let Err(error) = result {
            stage(
                progress,
                LifecyclePhase::Unloading,
                Some(LifecycleHook::OnUnload),
            );
            let _ = scope.unload(&module).await;
            stage(progress, LifecyclePhase::Unloading, None);
            return Err(error);
        }
        stage(progress, LifecyclePhase::Loading, None);
        drop(reservation);
        self.notify_callbacks("register", &name, module.as_ref());
        self.emit_ephemeral_event("apcore.registry.module_registered", &name, options.context);
        Ok(())
    }

    fn publish_loaded(
        &self,
        name: &str,
        module: &Arc<dyn Module>,
        descriptor: ModuleDescriptor,
        source: Option<String>,
        reload_owner: Option<&str>,
        async_runtime: Option<tokio::runtime::Handle>,
    ) -> Result<(), ModuleError> {
        let mut core = self.core.write();
        if core.closed {
            return Err(Self::closed_error());
        }
        core.check_reload_owner(name, reload_owner)?;
        if core.modules.contains_key(name) {
            return Err(ModuleError::duplicate_module_id(name));
        }
        if let Some(source) = source {
            let discovery = DiscoveredModule {
                name: name.to_owned(),
                source: source.clone(),
                descriptor: descriptor.clone(),
                module: Arc::clone(module),
            };
            Self::check_discovered_dependencies(&core, &discovery)?;
            core.sources.insert(name.to_owned(), source);
        }
        core.schema_cache.insert(
            name.to_owned(),
            json!({
                "input": descriptor.input_schema, "output": descriptor.output_schema,
            }),
        );
        core.lowercase_map
            .insert(name.to_lowercase(), name.to_owned());
        core.modules.insert(name.to_owned(), Arc::clone(module));
        core.descriptors.insert(name.to_owned(), descriptor);
        if let Some(runtime) = async_runtime {
            core.ref_counts.insert(
                name.to_owned(),
                Arc::new(ModuleReferences {
                    async_runtime: Some(runtime),
                    ..ModuleReferences::default()
                }),
            );
        }
        Ok(())
    }
}
