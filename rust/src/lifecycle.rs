//! Owned registry lifecycle operations and their immutable progress reports.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;
use tokio::sync::Notify;
use tokio::time::Instant;

use crate::cancel::CancelToken;
use crate::errors::{ErrorCode, ModuleError};
use crate::events::emitter::{ApCoreEvent, EventEmitter};
use crate::runtime::background_runtime;

pub(crate) const DEFAULT_DRAIN_TIMEOUT_MS: u32 = 5_000;
pub(crate) const DEFAULT_CANCEL_GRACE_MS: u32 = 1_000;
const MAX_TIMEOUT_MS: u32 = 2_147_483_647;

tokio::task_local! {
    pub(crate) static LIFECYCLE_CALLBACK: String;
    pub(crate) static REGISTRY_CALLBACK: usize;
}

/// The action taken when an in-flight invocation outlives its drain deadline.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DrainTimeoutPolicy {
    /// Record the overdue drain and wait for the invocation to finish.
    #[default]
    ContinueWaiting,
    /// Cooperatively cancel the retiring generation, then allow a forced
    /// unload after the configured grace if leases still remain.
    CancelThenForce,
}

/// The action taken when a governed reload exhausts the drain budget.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReloadTimeoutPolicy {
    #[default]
    AbortReload,
    ContinueWaiting,
}

/// Drain settings for managed reloads and a trusted Registry watcher.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReloadOptions {
    pub drain_timeout_ms: Option<u32>,
    pub on_drain_timeout: Option<ReloadTimeoutPolicy>,
}

impl ReloadOptions {
    #[cfg(feature = "sys-modules")]
    pub(crate) fn from_wire(value: &serde_json::Value) -> Result<Self, ModuleError> {
        let mut wire = value.clone();
        if let Some(timeout) = wire.get_mut("drain_timeout_ms") {
            if timeout.is_f64() {
                let number = timeout.as_f64().unwrap_or(f64::NAN);
                if (0.0..=f64::from(MAX_TIMEOUT_MS)).contains(&number) && number.fract() == 0.0 {
                    // The range and integrality checks make the conversion exact.
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let integral = number as u32;
                    *timeout = serde_json::json!(integral);
                }
            }
        }
        serde_json::from_value::<Self>(wire)
            .map_err(|_| invalid_lifecycle_input("Invalid reload lifecycle options"))?
            .validate()
    }

    #[cfg(any(feature = "discovery", feature = "sys-modules"))]
    pub(crate) fn validate(self) -> Result<Self, ModuleError> {
        validate_duration(self.drain_timeout_ms, "drain_timeout_ms")?;
        Ok(self)
    }
}

/// Policies for Registry shutdown and this client's event delivery.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShutdownOptions {
    pub drain_timeout_ms: Option<u32>,
    pub on_drain_timeout: Option<DrainTimeoutPolicy>,
    pub event_flush_timeout_ms: Option<u32>,
    pub cancel_grace_ms: Option<u32>,
}

impl ShutdownOptions {
    pub(crate) fn validate(self) -> Result<(), ModuleError> {
        validate_duration(self.drain_timeout_ms, "drain_timeout_ms")?;
        validate_duration(self.event_flush_timeout_ms, "event_flush_timeout_ms")?;
        validate_grace(self.cancel_grace_ms, self.on_drain_timeout)
    }
}

/// Policies for an ordered, generation-aware removal batch.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnregisterOptions {
    pub drain_timeout_ms: Option<u32>,
    pub on_drain_timeout: Option<DrainTimeoutPolicy>,
    pub operation_timeout_ms: Option<u32>,
    pub cancel_grace_ms: Option<u32>,
}

impl UnregisterOptions {
    pub(crate) fn validate(self) -> Result<(), ModuleError> {
        validate_duration(self.drain_timeout_ms, "drain_timeout_ms")?;
        validate_duration(self.operation_timeout_ms, "operation_timeout_ms")?;
        validate_grace(self.cancel_grace_ms, self.on_drain_timeout)
    }
}

fn validate_grace(
    grace: Option<u32>,
    policy: Option<DrainTimeoutPolicy>,
) -> Result<(), ModuleError> {
    validate_duration(grace, "cancel_grace_ms")?;
    if grace.is_some() && policy != Some(DrainTimeoutPolicy::CancelThenForce) {
        return Err(invalid_lifecycle_input(
            "cancel_grace_ms requires the cancel_then_force policy",
        ));
    }
    Ok(())
}

fn validate_duration(duration_ms: Option<u32>, field: &str) -> Result<(), ModuleError> {
    if duration_ms.is_some_and(|value| value > MAX_TIMEOUT_MS) {
        return Err(invalid_lifecycle_input(format!(
            "'{field}' must be between 0 and {MAX_TIMEOUT_MS} milliseconds"
        )));
    }
    Ok(())
}

pub(crate) fn invalid_lifecycle_input(message: impl Into<String>) -> ModuleError {
    ModuleError::new(ErrorCode::GeneralInvalidInput, message)
}

pub(crate) fn lifecycle_runtime() -> Result<Handle, ModuleError> {
    Handle::try_current()
        .map_err(|_| invalid_lifecycle_input("Starting lifecycle work requires a Tokio runtime"))?;
    lifecycle_worker()
}

pub(crate) fn lifecycle_worker() -> Result<Handle, ModuleError> {
    background_runtime().map_err(|error| {
        invalid_lifecycle_input(format!("Lifecycle scheduler unavailable: {error}"))
    })
}

/// Identifies which type of work the coordinator owns.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleKind {
    Shutdown,
    UnregisterMany,
    Reload,
}

/// Progress of an entire operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleStatus {
    Pending,
    Completed,
    Failed,
}

/// Progress of a module within an operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LifecyclePhase {
    Pending,
    Draining,
    Recovering,
    Unloading,
    Loading,
    Resuming,
    Done,
}

/// Terminal disposition of a module in a removal operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleOutcome {
    Removed,
    ForcedRemoved,
    Absent,
    AlreadyRemoving,
    NotStarted,
    RetainedReady,
    RetainedUnavailable,
    Reloaded,
    Failed,
}

/// Lifecycle hook currently running for a report row.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleHook {
    OnDrain,
    OnDrainAborted,
    OnSuspend,
    OnUnload,
    OnLoad,
    OnResume,
}

/// Reason for retiring a published registration.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DrainReasonKind {
    Shutdown,
    Unregister,
    Reload,
}

/// Reason that a reload needs to recover the original module.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DrainAbortReason {
    DrainTimeout,
    DrainError,
}

/// Whether a failed replacement restored the previous registration.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleCompensation {
    NotRequired,
    Restored,
    Failed,
}

/// A drain hook's operation, deadline and cooperative signal.
#[derive(Clone, Debug)]
pub struct DrainContext {
    pub operation_id: String,
    pub module_id: String,
    pub reason_kind: DrainReasonKind,
    pub cancel_token: CancelToken,
    pub(crate) deadline: Instant,
}

impl DrainContext {
    /// Remaining whole milliseconds on the monotonic drain clock.
    #[must_use]
    pub fn remaining_ms(&self) -> u64 {
        u64::try_from(
            self.deadline
                .saturating_duration_since(Instant::now())
                .as_millis(),
        )
        .unwrap_or(u64::MAX)
    }
}

/// A recovery hook's operation and fresh cooperative signal.
#[derive(Clone, Debug)]
pub struct DrainAbortedContext {
    pub operation_id: String,
    pub module_id: String,
    pub abort_reason: DrainAbortReason,
    pub cancel_token: CancelToken,
}

/// The reason an operation could not dispatch all of its entries.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleStopReason {
    OperationDeadline,
}

/// Outcome of this client's event-delivery stage.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleEventFlush {
    NotApplicable,
    Pending,
    Completed,
    Timeout,
}

/// Failure of a lifecycle stage.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LifecycleFailure {
    pub code: String,
    pub phase: LifecyclePhase,
}

/// A snapshot of one input position or Registry generation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LifecycleModuleReport {
    pub module_id: String,
    pub phase: LifecyclePhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<LifecycleOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_hook: Option<LifecycleHook>,
    pub drain_timed_out: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abort_reason: Option<DrainAbortReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compensation: Option<LifecycleCompensation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_at_unload: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cleanup_pending: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<LifecycleFailure>,
}

impl LifecycleModuleReport {
    fn new(module_id: String) -> Self {
        Self {
            module_id,
            phase: LifecyclePhase::Pending,
            outcome: None,
            active_hook: None,
            drain_timed_out: false,
            abort_reason: None,
            compensation: None,
            active_at_unload: None,
            cleanup_pending: None,
            error: None,
        }
    }

    pub(crate) fn finish(&mut self, outcome: LifecycleOutcome) {
        self.phase = LifecyclePhase::Done;
        self.outcome = Some(outcome);
    }
}

/// An immutable copy of operation progress, in dispatch order.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LifecycleReport {
    pub operation_id: String,
    pub kind: LifecycleKind,
    pub status: LifecycleStatus,
    pub modules: Vec<LifecycleModuleReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<LifecycleStopReason>,
    pub event_flush: LifecycleEventFlush,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<LifecycleFailure>,
}

#[derive(Debug)]
struct OperationProgress {
    report: RwLock<LifecycleReport>,
    finished: Notify,
    registry_scope: Option<usize>,
}

#[derive(Debug)]
struct EventStage {
    progress: RwLock<LifecycleEventFlush>,
    finished: Notify,
    emitter_scope: usize,
}

/// A cloneable handle to work that continues independently of its waiters.
#[derive(Clone, Debug)]
pub struct LifecycleOperation {
    progress: Arc<OperationProgress>,
    event_stage: Option<Arc<EventStage>>,
}

impl LifecycleOperation {
    pub(crate) fn new(
        kind: LifecycleKind,
        ids: impl IntoIterator<Item = String>,
        registry_scope: Option<usize>,
    ) -> Self {
        Self {
            progress: Arc::new(OperationProgress {
                report: RwLock::new(LifecycleReport {
                    operation_id: uuid::Uuid::new_v4().to_string(),
                    kind,
                    status: LifecycleStatus::Pending,
                    modules: ids.into_iter().map(LifecycleModuleReport::new).collect(),
                    stop_reason: None,
                    event_flush: LifecycleEventFlush::NotApplicable,
                    error: None,
                }),
                finished: Notify::new(),
                registry_scope,
            }),
            event_stage: None,
        }
    }

    /// Copy the current report without blocking or changing the operation.
    #[must_use]
    pub fn snapshot(&self) -> LifecycleReport {
        let event_flush = self
            .event_stage
            .as_ref()
            .map(|stage| *stage.progress.read());
        let mut report = self.progress.report.read().clone();
        if let Some(flush) = event_flush {
            report.event_flush = flush;
            if report.event_flush == LifecycleEventFlush::Pending {
                report.status = LifecycleStatus::Pending;
            }
        }
        report
    }

    /// Wait for finality or return a progress snapshot when the waiter expires.
    /// A zero timeout samples immediately. Dropping this future leaves work active.
    pub async fn wait(&self, wait_timeout_ms: Option<u32>) -> Result<LifecycleReport, ModuleError> {
        validate_duration(wait_timeout_ms, "wait_timeout_ms")?;
        if wait_timeout_ms == Some(0) {
            return Ok(self.snapshot());
        }
        if !self.is_active() {
            return Ok(self.snapshot());
        }
        if self.waits_for_current_callback() {
            return Err(invalid_lifecycle_input(
                "A lifecycle callback cannot wait for its own operation",
            ));
        }
        let settled = self.wait_until_finished();
        if let Some(timeout_ms) = wait_timeout_ms {
            let runtime = lifecycle_runtime()?;
            let timeout = {
                let _clock = runtime.enter();
                tokio::time::timeout(Duration::from_millis(u64::from(timeout_ms)), settled)
            };
            let _ = timeout.await;
        } else {
            settled.await;
        }
        Ok(self.snapshot())
    }

    fn is_active(&self) -> bool {
        self.event_stage.as_ref().map_or_else(
            || self.progress.report.read().status == LifecycleStatus::Pending,
            |stage| *stage.progress.read() == LifecycleEventFlush::Pending,
        )
    }

    fn waits_for_current_callback(&self) -> bool {
        let own_callback = LIFECYCLE_CALLBACK
            .try_with(|id| id == &self.progress.report.read().operation_id)
            .unwrap_or(false);
        let registry_callback = self.progress.registry_scope.is_some_and(|scope| {
            REGISTRY_CALLBACK
                .try_with(|id| *id == scope)
                .unwrap_or(false)
                && self.progress.report.read().status == LifecycleStatus::Pending
        });
        let emitter_callback = self
            .event_stage
            .as_ref()
            .is_some_and(|stage| EventEmitter::in_delivery_callback(stage.emitter_scope));
        own_callback || registry_callback || emitter_callback
    }

    async fn wait_until_finished(&self) {
        let finished = self
            .event_stage
            .as_ref()
            .map_or(&self.progress.finished, |stage| &stage.finished);
        let notified = finished.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if self.is_active() {
            notified.await;
        }
    }

    pub(crate) fn operation_id(&self) -> String {
        self.progress.report.read().operation_id.clone()
    }

    pub(crate) fn update(&self, change: impl FnOnce(&mut LifecycleReport)) {
        change(&mut self.progress.report.write());
    }

    pub(crate) fn finish(&self) {
        self.update(|report| {
            report.error = report.modules.iter().find_map(|row| row.error.clone());
            report.status = if report.stop_reason.is_some()
                || report.error.is_some()
                || report.modules.iter().any(|row| row.abort_reason.is_some())
            {
                LifecycleStatus::Failed
            } else {
                LifecycleStatus::Completed
            };
        });
        self.progress.finished.notify_waiters();
    }

    pub(crate) fn with_event_delivery(
        self,
        emitter: Arc<EventEmitter>,
        timeout_ms: u32,
        runtime: &Handle,
    ) -> Self {
        let event_stage = Arc::new(EventStage {
            progress: RwLock::new(LifecycleEventFlush::Pending),
            finished: Notify::new(),
            emitter_scope: emitter.delivery_scope(),
        });
        runtime.spawn({
            let resources = self.clone();
            let stage = Arc::clone(&event_stage);
            async move {
                resources.wait_until_finished().await;
                let report = resources.snapshot();
                let forced_count = report
                    .modules
                    .iter()
                    .filter(|row| row.active_at_unload.is_some())
                    .count();
                let failure_count = report
                    .modules
                    .iter()
                    .filter(|row| row.error.is_some())
                    .count();
                let mut event = ApCoreEvent::new(
                    "apcore.client.shutdown_resources_completed",
                    serde_json::json!({
                        "operation_id": report.operation_id,
                        "registry_outcome": report.status,
                        "forced_count": forced_count,
                        "failure_count": failure_count,
                    }),
                );
                if report.status == LifecycleStatus::Failed {
                    "error".clone_into(&mut event.severity);
                }
                emitter.emit_delivery_semantics(event);
                let completed = emitter
                    .shutdown_lifecycle(Duration::from_millis(u64::from(timeout_ms)))
                    .await;
                *stage.progress.write() = if completed {
                    LifecycleEventFlush::Completed
                } else {
                    LifecycleEventFlush::Timeout
                };
                stage.finished.notify_waiters();
            }
        });
        Self {
            event_stage: Some(event_stage),
            ..self
        }
    }
}
