//! Shared drain budgets and hook ownership for lifecycle operations.

use std::sync::Arc;

use futures_util::FutureExt;
use serde_json::{json, Value};
use tokio::time::Instant;

use crate::cancel::CancelToken;
use crate::errors::{ErrorCode, ModuleError};
use crate::events::emitter::ApCoreEvent;
use crate::lifecycle::{
    DrainAbortedContext, DrainContext, DrainReasonKind, DrainTimeoutPolicy, LifecycleHook,
    LifecycleOperation, LifecyclePhase, ShutdownOptions, DEFAULT_CANCEL_GRACE_MS,
    LIFECYCLE_CALLBACK, REGISTRY_CALLBACK,
};
use crate::module::Module;

use super::{deadline_after, DrainGuard, RemovalResources};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DrainAction {
    ContinueWaiting,
    #[cfg(any(feature = "discovery", feature = "sys-modules"))]
    AbortReload,
    CancelThenForce(u32),
    LegacyForce,
}

impl DrainAction {
    pub(super) fn from_options(policy: Option<DrainTimeoutPolicy>, grace: Option<u32>) -> Self {
        match policy.unwrap_or_default() {
            DrainTimeoutPolicy::ContinueWaiting => Self::ContinueWaiting,
            DrainTimeoutPolicy::CancelThenForce => {
                Self::CancelThenForce(grace.unwrap_or(DEFAULT_CANCEL_GRACE_MS))
            }
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::ContinueWaiting => "continue_waiting",
            #[cfg(any(feature = "discovery", feature = "sys-modules"))]
            Self::AbortReload => "abort_reload",
            Self::CancelThenForce(_) => "cancel_then_force",
            Self::LegacyForce => "legacy_force",
        }
    }

    pub(super) fn conflicts_with(self, options: ShutdownOptions) -> bool {
        match options.on_drain_timeout {
            None => false,
            Some(DrainTimeoutPolicy::ContinueWaiting) => self != Self::ContinueWaiting,
            Some(DrainTimeoutPolicy::CancelThenForce) => match self {
                Self::CancelThenForce(grace) => {
                    options.cancel_grace_ms.is_some_and(|ms| ms != grace)
                }
                _ => true,
            },
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct DrainPlan {
    pub(super) timeout_ms: u32,
    pub(super) deadline: Instant,
    pub(super) reason: DrainReasonKind,
    pub(super) action: DrainAction,
}

pub(super) struct DrainResult {
    pub(super) hook_error: Option<ModuleError>,
    #[cfg(any(feature = "discovery", feature = "sys-modules"))]
    pub(super) error_preceded_deadline: bool,
    #[cfg(any(feature = "discovery", feature = "sys-modules"))]
    pub(super) timed_out: bool,
    pub(super) force: bool,
}

impl RemovalResources {
    pub(super) async fn drain(
        &self,
        operation: &LifecycleOperation,
        index: usize,
        guard: &DrainGuard<'_>,
        plan: DrainPlan,
    ) -> DrainResult {
        operation.update(|report| report.modules[index].phase = LifecyclePhase::Draining);
        self.emit_module_event(
            operation,
            guard.name,
            "module_drain_started",
            "info",
            json!({
                "reason_kind": plan.reason, "drain_timeout_ms": plan.timeout_ms
            }),
        );
        let context = DrainContext {
            operation_id: operation.operation_id(),
            module_id: guard.name.to_owned(),
            reason_kind: plan.reason,
            cancel_token: CancelToken::new(),
            deadline: plan.deadline,
        };
        let modules = guard.modules();
        let hooks = self.run_hook_pair(operation, index, &modules, &context, None);
        tokio::pin!(hooks);
        let (hook_error, grace_deadline) = tokio::select! {
            biased;
            result = &mut hooks => {
                let error = result.err();
                if error.is_some() && plan.reason == DrainReasonKind::Reload {
                    return DrainResult {
                        hook_error: error,
                        #[cfg(any(feature = "discovery", feature = "sys-modules"))]
                        error_preceded_deadline: true,
                        #[cfg(any(feature = "discovery", feature = "sys-modules"))]
                        timed_out: false,
                        force: false,
                    };
                }
                if guard.references.wait_for_drain(Some(plan.deadline)).await {
                    return DrainResult {
                        hook_error: error,
                        #[cfg(any(feature = "discovery", feature = "sys-modules"))]
                        error_preceded_deadline: false,
                        #[cfg(any(feature = "discovery", feature = "sys-modules"))]
                        timed_out: false,
                        force: false,
                    };
                }
                (error, self.drain_timeout(operation, index, guard, plan))
            },
            () = async {
                if Instant::now() < plan.deadline {
                    tokio::time::sleep_until(plan.deadline).await;
                }
            } => {
                let grace = self.drain_timeout(operation, index, guard, plan);
                context.cancel_token.cancel();
                (hooks.await.err(), grace)
            }
        };
        let force = match plan.action {
            DrainAction::ContinueWaiting
                if plan.reason != DrainReasonKind::Reload || hook_error.is_none() =>
            {
                guard.references.wait_for_drain(None).await;
                false
            }
            DrainAction::ContinueWaiting => false,
            #[cfg(any(feature = "discovery", feature = "sys-modules"))]
            DrainAction::AbortReload => false,
            DrainAction::LegacyForce => true,
            DrainAction::CancelThenForce(_) => {
                !guard.references.wait_for_drain(grace_deadline).await
            }
        };
        DrainResult {
            hook_error,
            #[cfg(any(feature = "discovery", feature = "sys-modules"))]
            error_preceded_deadline: false,
            #[cfg(any(feature = "discovery", feature = "sys-modules"))]
            timed_out: true,
            force,
        }
    }

    fn drain_timeout(
        &self,
        operation: &LifecycleOperation,
        index: usize,
        guard: &DrainGuard<'_>,
        plan: DrainPlan,
    ) -> Option<Instant> {
        operation.update(|report| report.modules[index].drain_timed_out = true);
        let active = guard.references.state.lock().count;
        self.emit_module_event(
            operation,
            guard.name,
            "module_drain_timed_out",
            "warn",
            json!({
                "reason_kind": plan.reason, "action": plan.action.name(), "active_count": active,
            }),
        );
        if let DrainAction::CancelThenForce(grace) = plan.action {
            guard.references.cancel_executions();
            Some(deadline_after(grace))
        } else {
            None
        }
    }

    pub(super) fn emit_module_event(
        &self,
        operation: &LifecycleOperation,
        name: &str,
        event: &str,
        severity: &str,
        mut data: Value,
    ) {
        if let Some(emitter) = self.emitter.read().clone() {
            data["operation_id"] = json!(operation.operation_id());
            emitter.emit_delivery_semantics(ApCoreEvent::with_module(
                format!("apcore.registry.{event}"),
                data,
                name,
                severity,
            ));
        }
    }

    pub(super) async fn run_hook_pair(
        &self,
        operation: &LifecycleOperation,
        index: usize,
        modules: &[Arc<dyn Module>],
        context: &DrainContext,
        abort: Option<&DrainAbortedContext>,
    ) -> Result<(), ModuleError> {
        let mut first = Ok(());
        for module in modules {
            let Some(drainable) = module.as_drainable() else {
                continue;
            };
            let (hook, future) = abort.map_or_else(
                || (LifecycleHook::OnDrain, drainable.on_drain(context)),
                |context| {
                    (
                        LifecycleHook::OnDrainAborted,
                        drainable.on_drain_aborted(context),
                    )
                },
            );
            operation.update(|report| report.modules[index].active_hook = Some(hook));
            let result = LIFECYCLE_CALLBACK
                .scope(
                    operation.operation_id(),
                    REGISTRY_CALLBACK.scope(
                        Arc::as_ptr(&self.core) as usize,
                        std::panic::AssertUnwindSafe(future).catch_unwind(),
                    ),
                )
                .await
                .unwrap_or_else(|_| Err(hook_panicked()));
            operation.update(|report| report.modules[index].active_hook = None);
            first = first.and(result);
        }
        first
    }
}

pub(super) fn hook_panicked() -> ModuleError {
    ModuleError::new(ErrorCode::GeneralInternalError, "Lifecycle hook panicked")
}
