//! Registry-owned drain coordination for explicit batches and host shutdown.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use tokio::time::Instant;

use crate::errors::ModuleError;
use crate::events::emitter::EventEmitter;
use crate::lifecycle::{
    invalid_lifecycle_input, lifecycle_runtime, DrainReasonKind, LifecycleFailure, LifecycleHook,
    LifecycleKind, LifecycleOperation, LifecycleOutcome, LifecyclePhase, LifecycleStopReason,
    ShutdownOptions, UnregisterOptions, DEFAULT_DRAIN_TIMEOUT_MS,
};

use super::async_lifecycle::HookScope;

use super::{
    validate_module_id, CallbackMap, DrainGuard, ModuleReferences, Registry, RegistryCore,
    RemovedRegistration,
};

#[path = "managed_drain.rs"]
mod managed_drain;
#[path = "managed_reload.rs"]
mod managed_reload;

use managed_drain::{DrainAction, DrainPlan};

pub(super) struct RegistryShutdown {
    operation: LifecycleOperation,
    drain_timeout_ms: u32,
    action: DrainAction,
}

#[derive(Clone)]
struct RemovalResources {
    core: Arc<RwLock<RegistryCore>>,
    callbacks: Arc<RwLock<CallbackMap>>,
    emitter: Arc<RwLock<Option<Arc<EventEmitter>>>>,
}

struct BatchTarget {
    name: String,
    generation: Option<Arc<ModuleReferences>>,
}

enum ShutdownTarget {
    Registered(String, Arc<ModuleReferences>),
    Detached(String, Arc<ModuleReferences>),
}

impl ShutdownTarget {
    fn name(&self) -> &str {
        match self {
            Self::Registered(name, _) | Self::Detached(name, _) => name,
        }
    }
}

enum RemovalClaim<'a> {
    Deadline,
    Absent,
    Draining,
    Detached,
    Owned(DrainGuard<'a>),
}

impl Registry {
    fn removal_resources(&self) -> RemovalResources {
        RemovalResources {
            core: Arc::clone(&self.core),
            callbacks: Arc::clone(&self.callbacks),
            emitter: Arc::clone(&self.event_emitter),
        }
    }

    /// Start an ordered batch using the generation registered at each input position.
    /// The returned operation owns cleanup even when the caller leaves its wait.
    pub fn start_unregister_many<I, S>(
        &self,
        module_ids: I,
        options: Option<UnregisterOptions>,
    ) -> Result<LifecycleOperation, ModuleError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let options = options.unwrap_or_default();
        options.validate()?;
        let ids = module_ids
            .into_iter()
            .map(|value| {
                let name = value.as_ref();
                validate_module_id(name, true)
                    .map_err(|error| invalid_lifecycle_input(error.message))?;
                Ok(name.to_owned())
            })
            .collect::<Result<Vec<_>, ModuleError>>()?;
        let runtime = (!ids.is_empty()).then(lifecycle_runtime).transpose()?;
        let resources = self.removal_resources();
        let deadline = options.operation_timeout_ms.map(deadline_after);
        let targets = resources.snapshot_batch(&ids)?;
        let operation = LifecycleOperation::new(LifecycleKind::UnregisterMany, ids, None);
        if let Some(runtime) = runtime {
            let owned = operation.clone();
            runtime.spawn(async move {
                resources
                    .run_batch(&owned, targets, options, deadline)
                    .await;
                owned.finish();
            });
        } else {
            operation.finish();
        }
        Ok(operation)
    }

    pub(crate) fn start_owned_shutdown(
        self: &Arc<Self>,
        options: ShutdownOptions,
        legacy_force: bool,
    ) -> Result<LifecycleOperation, ModuleError> {
        let mut shutdown = self.shutdown_operation.lock();
        if let Some(owner) = shutdown.as_ref() {
            if options
                .drain_timeout_ms
                .is_some_and(|ms| ms != owner.drain_timeout_ms)
                || owner.action.conflicts_with(options)
            {
                return Err(invalid_lifecycle_input(
                    "Conflicting Registry shutdown drain policy",
                ));
            }
            return Ok(owner.operation.clone());
        }

        let runtime = lifecycle_runtime()?;
        let resources = self.removal_resources();
        let targets = resources.close_and_snapshot();
        let operation = LifecycleOperation::new(
            LifecycleKind::Shutdown,
            targets.iter().map(|target| target.name().to_owned()),
            Some(Arc::as_ptr(&self.core) as usize),
        );
        let drain_timeout_ms = options.drain_timeout_ms.unwrap_or(DEFAULT_DRAIN_TIMEOUT_MS);
        let action = if legacy_force {
            DrainAction::LegacyForce
        } else {
            DrainAction::from_options(options.on_drain_timeout, options.cancel_grace_ms)
        };
        *shutdown = Some(RegistryShutdown {
            operation: operation.clone(),
            drain_timeout_ms,
            action,
        });
        let registry = Arc::clone(self);
        let owned = operation.clone();
        let watcher = self.stop_watching();
        runtime.spawn(async move {
            if let Some(watcher) = watcher {
                let _ = watcher.await;
            }
            registry.in_flight.wait_until_empty().await;
            resources
                .run_shutdown(&owned, targets, drain_timeout_ms, action)
                .await;
            owned.finish();
        });
        Ok(operation)
    }
}

impl RemovalResources {
    fn snapshot_batch(&self, ids: &[String]) -> Result<Vec<BatchTarget>, ModuleError> {
        let mut core = self.core.write();
        if core.closed {
            return Err(Registry::closed_error());
        }
        Ok(ids
            .iter()
            .map(|name| BatchTarget {
                name: name.clone(),
                generation: if core.modules.contains_key(name) {
                    Some(Arc::clone(core.ref_counts.entry(name.clone()).or_default()))
                } else {
                    core.pending_removals
                        .get(name)
                        .and_then(|items| items.last())
                        .cloned()
                },
            })
            .collect())
    }

    fn close_and_snapshot(&self) -> Vec<ShutdownTarget> {
        let mut core = self.core.write();
        core.closed = true;
        let mut ids: Vec<_> = core.modules.keys().cloned().collect();
        ids.sort_unstable();
        let mut targets: Vec<_> = ids
            .into_iter()
            .map(|name| {
                let references = Arc::clone(core.ref_counts.entry(name.clone()).or_default());
                ShutdownTarget::Registered(name, references)
            })
            .collect();
        let mut detached: Vec<_> = core.pending_removals.iter().collect();
        detached.sort_unstable_by(|a, b| a.0.cmp(b.0));
        targets.extend(detached.into_iter().flat_map(|(name, references)| {
            references
                .iter()
                .map(|generation| ShutdownTarget::Detached(name.clone(), Arc::clone(generation)))
        }));
        targets
    }

    fn claim<'a>(
        &'a self,
        name: &'a str,
        generation: Option<&Arc<ModuleReferences>>,
        closing: bool,
        operation_deadline: Option<Instant>,
    ) -> RemovalClaim<'a> {
        let mut core = self.core.write();
        if operation_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return RemovalClaim::Deadline;
        }
        let Some(generation) = generation else {
            return RemovalClaim::Absent;
        };
        if core
            .ref_counts
            .get(name)
            .is_some_and(|current| Arc::ptr_eq(current, generation))
        {
            if (!closing && core.closed)
                || core.reload_owners.contains_key(name)
                || !core.draining.insert(name.to_owned())
            {
                return RemovalClaim::Draining;
            }
            return RemovalClaim::Owned(DrainGuard {
                core: &self.core,
                name,
                references: Arc::clone(generation),
            });
        }
        if core.pending_removals.get(name).is_some_and(|pending| {
            pending
                .iter()
                .any(|current| Arc::ptr_eq(current, generation))
        }) {
            RemovalClaim::Detached
        } else {
            RemovalClaim::Absent
        }
    }

    async fn run_batch(
        &self,
        operation: &LifecycleOperation,
        targets: Vec<BatchTarget>,
        options: UnregisterOptions,
        deadline: Option<Instant>,
    ) {
        let drain_timeout_ms = options.drain_timeout_ms.unwrap_or(DEFAULT_DRAIN_TIMEOUT_MS);
        let action = DrainAction::from_options(options.on_drain_timeout, options.cancel_grace_ms);
        for (index, target) in targets.into_iter().enumerate() {
            match self.claim(&target.name, target.generation.as_ref(), false, deadline) {
                RemovalClaim::Deadline => {
                    operation.update(|report| {
                        report.stop_reason = Some(LifecycleStopReason::OperationDeadline);
                        for row in &mut report.modules[index..] {
                            row.finish(LifecycleOutcome::NotStarted);
                        }
                    });
                    break;
                }
                RemovalClaim::Absent => {
                    finish_module(operation, index, LifecycleOutcome::Absent);
                }
                RemovalClaim::Draining | RemovalClaim::Detached => {
                    finish_module(operation, index, LifecycleOutcome::AlreadyRemoving);
                }
                RemovalClaim::Owned(guard) => {
                    let drain_deadline = deadline.map_or_else(
                        || deadline_after(drain_timeout_ms),
                        |deadline| deadline.min(deadline_after(drain_timeout_ms)),
                    );
                    self.remove_owned(
                        operation,
                        index,
                        guard,
                        DrainPlan {
                            timeout_ms: drain_timeout_ms,
                            deadline: drain_deadline,
                            reason: DrainReasonKind::Unregister,
                            action,
                        },
                    )
                    .await;
                }
            }
        }
    }

    async fn run_shutdown(
        &self,
        operation: &LifecycleOperation,
        targets: Vec<ShutdownTarget>,
        drain_timeout_ms: u32,
        action: DrainAction,
    ) {
        let mut detached_deadline = None;
        for (index, target) in targets.into_iter().enumerate() {
            match target {
                ShutdownTarget::Registered(name, references) => {
                    self.join_or_remove(
                        operation,
                        index,
                        &name,
                        &references,
                        drain_timeout_ms,
                        action,
                    )
                    .await;
                }
                ShutdownTarget::Detached(_, references) => {
                    let deadline = (action == DrainAction::LegacyForce).then(|| {
                        *detached_deadline.get_or_insert_with(|| deadline_after(drain_timeout_ms))
                    });
                    join_detached(operation, index, &references, deadline).await;
                }
            }
        }
    }

    async fn join_or_remove(
        &self,
        operation: &LifecycleOperation,
        index: usize,
        name: &str,
        references: &Arc<ModuleReferences>,
        drain_timeout_ms: u32,
        action: DrainAction,
    ) {
        loop {
            let changed = references.drained.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            match self.claim(name, Some(references), true, None) {
                RemovalClaim::Draining => {
                    operation
                        .update(|report| report.modules[index].phase = LifecyclePhase::Draining);
                    changed.await;
                }
                RemovalClaim::Detached => {
                    join_detached(operation, index, references, None).await;
                    return;
                }
                RemovalClaim::Owned(guard) => {
                    self.remove_owned(
                        operation,
                        index,
                        guard,
                        DrainPlan {
                            timeout_ms: drain_timeout_ms,
                            deadline: deadline_after(drain_timeout_ms),
                            reason: DrainReasonKind::Shutdown,
                            action,
                        },
                    )
                    .await;
                    return;
                }
                RemovalClaim::Absent | RemovalClaim::Deadline => {
                    let outcome = if references.state.lock().completed {
                        LifecycleOutcome::AlreadyRemoving
                    } else {
                        LifecycleOutcome::Absent
                    };
                    finish_module(operation, index, outcome);
                    return;
                }
            }
        }
    }

    async fn remove_owned(
        &self,
        operation: &LifecycleOperation,
        index: usize,
        guard: DrainGuard<'_>,
        plan: DrainPlan,
    ) {
        let drained = self.drain(operation, index, &guard, plan).await;
        let Some((removed, _completion)) = guard.detach() else {
            finish_module(operation, index, LifecycleOutcome::Absent);
            return;
        };
        let active_at_unload = guard.references.state.lock().count;
        let forced = drained.force && active_at_unload > 0;
        operation.update(|report| {
            let row = &mut report.modules[index];
            row.phase = LifecyclePhase::Unloading;
            row.active_hook = Some(LifecycleHook::OnUnload);
            if forced {
                row.active_at_unload = Some(active_at_unload);
                row.cleanup_pending = Some(true);
            }
        });
        let (_, removal) = self
            .unload_for_operation(operation, index, guard.name, removed)
            .await;
        operation.update(|report| {
            let row = &mut report.modules[index];
            if let Some(error) = &drained.hook_error {
                row.error = Some(LifecycleFailure {
                    code: error.wire_code().into_owned(),
                    phase: LifecyclePhase::Draining,
                });
            } else if let Err(error) = &removal {
                row.error = Some(LifecycleFailure {
                    code: error.wire_code().into_owned(),
                    phase: LifecyclePhase::Unloading,
                });
            }
            row.active_hook = None;
            row.finish(if row.error.is_some() {
                LifecycleOutcome::Failed
            } else if forced {
                LifecycleOutcome::ForcedRemoved
            } else {
                LifecycleOutcome::Removed
            });
        });
        if forced && plan.action != DrainAction::LegacyForce {
            guard.references.wait_for_drain(None).await;
            operation.update(|report| report.modules[index].cleanup_pending = Some(false));
        }
    }

    async fn unload_for_operation(
        &self,
        operation: &LifecycleOperation,
        index: usize,
        name: &str,
        removed: RemovedRegistration,
    ) -> (RemovedRegistration, Result<(), ModuleError>) {
        let scope = HookScope {
            registry: Arc::as_ptr(&self.core) as usize,
            operation: Some(operation.operation_id()),
        };
        let result = scope.unload_registration(&removed).await;
        operation.update(|report| report.modules[index].active_hook = None);
        let resources = self.clone();
        let name = name.to_owned();
        let primary = Arc::clone(&removed.primary);
        scope
            .run_callbacks(move || {
                Registry::notify_removal_callbacks(
                    &resources.callbacks,
                    &resources.emitter,
                    &name,
                    primary.as_ref(),
                );
            })
            .await;
        (removed, result)
    }
}

fn deadline_after(ms: u32) -> Instant {
    Instant::now() + Duration::from_millis(u64::from(ms))
}

fn finish_module(operation: &LifecycleOperation, index: usize, outcome: LifecycleOutcome) {
    operation.update(|report| report.modules[index].finish(outcome));
}

async fn join_detached(
    operation: &LifecycleOperation,
    index: usize,
    references: &Arc<ModuleReferences>,
    deadline: Option<Instant>,
) {
    operation.update(|report| report.modules[index].phase = LifecyclePhase::Draining);
    references.wait_for_removal(deadline).await;
    finish_module(operation, index, LifecycleOutcome::AlreadyRemoving);
}
