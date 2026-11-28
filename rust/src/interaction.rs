//! Optional application-provided interaction for executing modules.

use std::future::Future;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::time::{sleep_until, Instant};

use crate::cancel::{deadline_remaining, wait_for_cancellation, CancelToken};
use crate::{Context, ModuleError};

/// Progress reported by a running module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProgressUpdate {
    pub progress: f64,
    pub total: Option<f64>,
    pub message: Option<String>,
}

/// A prompt and the JSON Schema for the requested response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ElicitationRequest {
    pub message: String,
    pub requested_schema: Option<Value>,
}

/// A user response to an elicitation request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "lowercase")]
pub enum ElicitationResult {
    Accept { content: Map<String, Value> },
    Decline,
    Cancel,
}

/// A callback that receives a progress update.
pub type ProgressReporter =
    Arc<dyn Fn(ProgressUpdate) -> BoxFuture<'static, Result<(), ModuleError>> + Send + Sync>;

/// A callback that handles an elicitation request.
pub type ElicitationHandler = Arc<
    dyn Fn(ElicitationRequest) -> BoxFuture<'static, Result<ElicitationResult, ModuleError>>
        + Send
        + Sync,
>;

impl<T: Clone> Context<T> {
    /// Derive a context with the given progress callback.
    #[must_use]
    pub fn with_progress_reporter(&self, callback: Option<ProgressReporter>) -> Self {
        let mut derived = self.clone();
        derived.runtime.progress_reporter = callback;
        derived
    }

    /// Derive a context with the given elicitation callback.
    #[must_use]
    pub fn with_elicitation_handler(&self, callback: Option<ElicitationHandler>) -> Self {
        let mut derived = self.clone();
        derived.runtime.elicitation_handler = callback;
        derived
    }
}

impl<T> Context<T> {
    /// Report progress through the host callback, if installed.
    pub async fn report_progress(
        &self,
        progress: f64,
        total: Option<f64>,
        message: Option<&str>,
    ) -> Result<(), ModuleError> {
        let Some(callback) = self.runtime.progress_reporter.as_ref() else {
            return Ok(());
        };
        if !progress.is_finite() || total.is_some_and(|value| !value.is_finite()) {
            return Err(ModuleError::invalid_input(
                "Progress and total must be finite",
            ));
        }
        await_interaction(self, || {
            callback(ProgressUpdate {
                progress,
                total,
                message: message.map(str::to_owned),
            })
        })
        .await
    }

    /// Request a caller response through the host callback, if installed.
    pub async fn elicit(
        &self,
        message: &str,
        requested_schema: impl Into<Option<Value>>,
    ) -> Result<Option<ElicitationResult>, ModuleError> {
        let Some(callback) = self.runtime.elicitation_handler.as_ref() else {
            return Ok(None);
        };
        let requested_schema = requested_schema.into();
        if requested_schema
            .as_ref()
            .is_some_and(|schema| !schema.is_object())
        {
            return Err(ModuleError::invalid_input(
                "Elicitation requires a schema object",
            ));
        }
        await_interaction(self, || {
            callback(ElicitationRequest {
                message: message.to_owned(),
                requested_schema,
            })
        })
        .await
        .map(Some)
    }
}

async fn await_interaction<T, R, F: Future<Output = Result<R, ModuleError>>>(
    context: &Context<T>,
    callback: impl FnOnce() -> F,
) -> Result<R, ModuleError> {
    let module_id = context.call_chain.last().map(String::as_str);
    let token = context.cancel_token.as_ref();
    let expiration = context
        .global_deadline
        .and_then(deadline_remaining)
        .and_then(|remaining| Instant::now().checked_add(remaining));
    let deadline_error = || {
        let mut error = ModuleError::module_timeout(module_id.unwrap_or(""), 0);
        "Interaction exceeded its execution deadline".clone_into(&mut error.message);
        error.details.remove("timeout_ms");
        if module_id.is_none() {
            error.details.remove("module_id");
        }
        error
    };
    let check_interruption = || {
        check_interaction_cancellation(token, module_id)?;
        if expiration.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err(deadline_error());
        }
        Ok(())
    };

    check_interruption()?;
    let timeout = async {
        match expiration {
            Some(expiration) => sleep_until(expiration).await,
            None => std::future::pending().await,
        }
    };
    let result = tokio::select! {
        biased;
        () = wait_for_cancellation(token) => return Err(cancellation_error(token, module_id)),
        () = timeout => return Err(deadline_error()),
        reply = async { callback().await } => reply,
    };
    check_interruption()?;
    result
}

fn check_interaction_cancellation(
    token: Option<&CancelToken>,
    module_id: Option<&str>,
) -> Result<(), ModuleError> {
    if token.is_some_and(CancelToken::is_cancelled) {
        return Err(cancellation_error(token, module_id));
    }
    Ok(())
}

fn cancellation_error(token: Option<&CancelToken>, module_id: Option<&str>) -> ModuleError {
    token
        .and_then(CancelToken::timeout_error)
        .cloned()
        .unwrap_or_else(|| {
            use crate::cancel::ExecutionCancelledError;
            match module_id {
                Some(module_id) => {
                    ExecutionCancelledError::new(module_id, "Execution was cancelled")
                }
                None => ExecutionCancelledError::without_module("Execution was cancelled"),
            }
            .into()
        })
}
