//! Generation-preserving governed replacement and D-112 compensation.

use std::sync::Arc;

use parking_lot::RwLock;
use serde_json::{json, Value};

use crate::cancel::CancelToken;
use crate::errors::{ErrorCode, ModuleError};
use crate::lifecycle::{
    lifecycle_runtime, DrainAbortReason, DrainAbortedContext, DrainContext, DrainReasonKind,
    LifecycleCompensation, LifecycleFailure, LifecycleHook, LifecycleKind, LifecycleOperation,
    LifecycleOutcome, LifecyclePhase, ReloadOptions, ReloadTimeoutPolicy, DEFAULT_DRAIN_TIMEOUT_MS,
};

use super::super::async_lifecycle::HookScope;
use super::super::{DiscoveredModule, RegistrationOptions, RegistryCore, RemovedRegistration};
use super::managed_drain::{DrainAction, DrainPlan};
use super::{deadline_after, BatchTarget, DrainGuard, Registry, RemovalClaim, RemovalResources};

struct ReloadSlot<'a> {
    core: &'a RwLock<RegistryCore>,
    name: &'a str,
}

impl<'a> ReloadSlot<'a> {
    fn own(core: &'a RwLock<RegistryCore>, name: &'a str, operation: &LifecycleOperation) -> Self {
        // The caller owns this generation's drain marker. No other operation
        // can remove or acquire the registration before the slot is installed.
        let previous = core
            .write()
            .reload_owners
            .insert(name.to_owned(), operation.operation_id());
        debug_assert!(previous.is_none());
        Self { core, name }
    }
}

impl Drop for ReloadSlot<'_> {
    fn drop(&mut self) {
        let mut core = self.core.write();
        core.reload_owners.remove(self.name);
        if let Some(references) = core.ref_counts.get(self.name) {
            references.drained.notify_waiters();
        }
    }
}

impl Registry {
    #[cfg(feature = "sys-modules")]
    pub(crate) fn start_managed_reload(
        self: &Arc<Self>,
        ids: Vec<String>,
        options: ReloadOptions,
    ) -> Result<LifecycleOperation, ModuleError> {
        self.start_managed_reload_with_candidate(ids, options, None)
    }

    pub(crate) fn start_managed_reload_with_candidate(
        self: &Arc<Self>,
        ids: Vec<String>,
        options: ReloadOptions,
        candidate: Option<DiscoveredModule>,
    ) -> Result<LifecycleOperation, ModuleError> {
        let options = options.validate()?;
        let resources = self.removal_resources();
        let targets = resources.snapshot_batch(&ids)?;
        let operation = LifecycleOperation::new(LifecycleKind::Reload, ids, None);
        if targets.is_empty() {
            operation.finish();
            return Ok(operation);
        }
        let runtime = lifecycle_runtime()?;
        let operation_owner = operation.clone();
        let registry = Arc::clone(self);
        runtime.spawn(async move {
            resources
                .run_reload(&registry, &operation_owner, targets, options, candidate)
                .await;
            operation_owner.finish();
        });
        Ok(operation)
    }

    async fn replace_one(
        &self,
        name: &str,
        state: Option<Value>,
        operation: &LifecycleOperation,
        index: usize,
        candidate: Option<DiscoveredModule>,
    ) -> Result<(), ModuleError> {
        let candidate = if let Some(candidate) = candidate {
            candidate
        } else {
            self.run_discoverer()
                .await?
                .into_iter()
                .find(|candidate| candidate.name == name)
                .ok_or_else(|| {
                    ModuleError::new(ErrorCode::ReloadFailed, "Replacement was not discovered")
                })?
        };
        self.load_discovered_with_hooks(candidate, state, Some((operation, index)))
            .await
    }

    async fn restore_one(
        &self,
        previous: RemovedRegistration,
        state: Option<Value>,
        operation: &LifecycleOperation,
        index: usize,
    ) -> Result<(), ModuleError> {
        let RemovedRegistration {
            primary,
            descriptor,
            older_versions,
            source,
        } = previous;
        let name = descriptor.module_id.clone();
        let restored = Arc::clone(&primary);
        self.load_with_hooks(
            primary,
            descriptor,
            None,
            RegistrationOptions {
                allow_reserved: true,
                run_validator: false,
                context: None,
                resume: state,
            },
            Some((operation, index)),
        )
        .await?;
        if let Some(source) = source {
            let mut core = self.core.write();
            if core
                .modules
                .get(&name)
                .is_some_and(|module| Arc::ptr_eq(module, &restored))
            {
                core.sources.insert(name.clone(), source);
            }
        }
        for version in older_versions {
            self.add_version(
                &name,
                &version.module,
                version.descriptor,
                Some(&operation.operation_id()),
            )?;
        }
        Ok(())
    }
}

impl RemovalResources {
    async fn run_reload(
        &self,
        registry: &Registry,
        operation: &LifecycleOperation,
        targets: Vec<BatchTarget>,
        options: ReloadOptions,
        mut candidate: Option<DiscoveredModule>,
    ) {
        let timeout_ms = options.drain_timeout_ms.unwrap_or(DEFAULT_DRAIN_TIMEOUT_MS);
        let action = match options.on_drain_timeout.unwrap_or_default() {
            ReloadTimeoutPolicy::AbortReload => DrainAction::AbortReload,
            ReloadTimeoutPolicy::ContinueWaiting => DrainAction::ContinueWaiting,
        };
        for (index, target) in targets.into_iter().enumerate() {
            let RemovalClaim::Owned(guard) =
                self.claim(&target.name, target.generation.as_ref(), false, None)
            else {
                let error = ModuleError::new(
                    ErrorCode::ModuleReloadConflict,
                    "Selected generation changed or is being removed",
                );
                self.reload_failed(
                    operation,
                    index,
                    &target.name,
                    LifecyclePhase::Draining,
                    &error,
                    LifecycleCompensation::NotRequired,
                );
                continue;
            };
            let _slot = ReloadSlot::own(&self.core, &target.name, operation);
            let plan = DrainPlan {
                timeout_ms,
                deadline: deadline_after(timeout_ms),
                reason: DrainReasonKind::Reload,
                action,
            };
            let drained = self.drain(operation, index, &guard, plan).await;
            let abort = if drained.error_preceded_deadline {
                Some(DrainAbortReason::DrainError)
            } else if drained.timed_out && action == DrainAction::AbortReload {
                Some(DrainAbortReason::DrainTimeout)
            } else {
                drained
                    .hook_error
                    .as_ref()
                    .map(|_| DrainAbortReason::DrainError)
            };
            if let Some(reason) = abort {
                self.recover(operation, index, guard, reason, plan, drained.hook_error)
                    .await;
            } else {
                self.replace(registry, operation, index, guard, candidate.take())
                    .await;
            }
        }
    }

    async fn recover(
        &self,
        operation: &LifecycleOperation,
        index: usize,
        guard: DrainGuard<'_>,
        reason: DrainAbortReason,
        plan: DrainPlan,
        drain_error: Option<ModuleError>,
    ) {
        operation.update(|report| {
            report.modules[index].abort_reason = Some(reason);
            report.modules[index].phase = LifecyclePhase::Recovering;
        });
        let context = DrainAbortedContext {
            operation_id: operation.operation_id(),
            module_id: guard.name.to_owned(),
            abort_reason: reason,
            cancel_token: CancelToken::new(),
        };
        let drain_context = DrainContext {
            operation_id: context.operation_id.clone(),
            module_id: context.module_id.clone(),
            reason_kind: DrainReasonKind::Reload,
            cancel_token: CancelToken::new(),
            deadline: plan.deadline,
        };
        let recovery = self
            .run_hook_pair(
                operation,
                index,
                &guard.modules(),
                &drain_context,
                Some(&context),
            )
            .await;
        let ready = guard.apply_recovery(recovery.is_ok());
        drop(guard);
        let outcome = if ready {
            LifecycleOutcome::RetainedReady
        } else {
            LifecycleOutcome::RetainedUnavailable
        };
        operation.update(|report| {
            let row = &mut report.modules[index];
            row.error = match recovery {
                Err(error) => Some(LifecycleFailure {
                    code: error.wire_code().into_owned(),
                    phase: LifecyclePhase::Recovering,
                }),
                Ok(()) => drain_error.map(|error| LifecycleFailure {
                    code: error.wire_code().into_owned(),
                    phase: LifecyclePhase::Draining,
                }),
            };
            row.finish(outcome);
        });
        self.emit_module_event(
            operation,
            &context.module_id,
            "module_drain_recovered",
            if ready { "info" } else { "error" },
            json!({"abort_reason": reason, "outcome": outcome}),
        );
    }

    async fn replace(
        &self,
        registry: &Registry,
        operation: &LifecycleOperation,
        index: usize,
        guard: DrainGuard<'_>,
        candidate: Option<DiscoveredModule>,
    ) {
        let name = guard.name;
        let Some((previous, _completion)) = guard.detach() else {
            self.reload_failed(
                operation,
                index,
                name,
                LifecyclePhase::Draining,
                &ModuleError::module_not_found(name),
                LifecycleCompensation::NotRequired,
            );
            return;
        };
        let scope = HookScope {
            registry: Arc::as_ptr(&self.core) as usize,
            operation: Some(operation.operation_id()),
        };
        operation
            .update(|report| report.modules[index].active_hook = Some(LifecycleHook::OnSuspend));
        let state = scope.suspend(&previous.primary).await;
        operation.update(|report| {
            report.modules[index].phase = LifecyclePhase::Unloading;
            report.modules[index].active_hook = Some(LifecycleHook::OnUnload);
        });
        let (previous, unloaded) = self
            .unload_for_operation(operation, index, name, previous)
            .await;
        if let Err(error) = unloaded {
            self.compensate(
                registry,
                operation,
                index,
                previous,
                state,
                LifecyclePhase::Unloading,
                &error,
            )
            .await;
            return;
        }
        operation.update(|report| report.modules[index].phase = LifecyclePhase::Loading);
        match registry
            .replace_one(name, state.clone(), operation, index, candidate)
            .await
        {
            Ok(()) => {
                operation.update(|report| report.modules[index].finish(LifecycleOutcome::Reloaded));
            }
            Err(error) => {
                self.compensate(
                    registry,
                    operation,
                    index,
                    previous,
                    state,
                    LifecyclePhase::Loading,
                    &error,
                )
                .await;
            }
        }
    }

    #[allow(clippy::too_many_arguments)] // one immutable report coordinate and one replacement failure
    async fn compensate(
        &self,
        registry: &Registry,
        operation: &LifecycleOperation,
        index: usize,
        previous: RemovedRegistration,
        state: Option<Value>,
        phase: LifecyclePhase,
        error: &ModuleError,
    ) {
        let name = previous.descriptor.module_id.clone();
        let restored = registry
            .restore_one(previous, state, operation, index)
            .await
            .is_ok();
        let compensation = if restored {
            LifecycleCompensation::Restored
        } else {
            LifecycleCompensation::Failed
        };
        self.reload_failed(operation, index, &name, phase, error, compensation);
    }

    fn reload_failed(
        &self,
        operation: &LifecycleOperation,
        index: usize,
        name: &str,
        phase: LifecyclePhase,
        error: &ModuleError,
        compensation: LifecycleCompensation,
    ) {
        let code = error.wire_code();
        operation.update(|report| {
            let row = &mut report.modules[index];
            row.active_hook = None;
            row.error = Some(LifecycleFailure {
                code: code.to_string(),
                phase,
            });
            row.compensation = Some(compensation);
            row.finish(LifecycleOutcome::Failed);
        });
        self.emit_module_event(
            operation,
            name,
            "module_reload_failed",
            "error",
            json!({
                "phase": phase, "error_code": code, "compensation": compensation,
            }),
        );
    }
}
