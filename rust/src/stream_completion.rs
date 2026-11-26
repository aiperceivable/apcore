//! Completion metadata for a single, lazily driven streaming execution.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures_core::Stream;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::oneshot;

use crate::cancel::CancelToken;
use crate::errors::{ErrorCode, ModuleError};
use crate::pipeline::StepTrace;

/// Why a streaming execution ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StreamEndReason {
    /// The producer was drained and the post-stream steps finished.
    Completed,
    /// A module or execution step failed.
    Failed,
    /// The invocation reported cancellation.
    Cancelled,
    /// The invocation reported a timeout.
    TimedOut,
    /// The consumer explicitly closed or dropped the stream.
    ConsumerClosed,
}

/// How execution treated a failed post-stream step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StreamFailureDisposition {
    /// No recovery policy handled the failure.
    Unhandled,
    /// The step was configured to ignore failures.
    Ignored,
    /// A step middleware supplied recovery output.
    Recovered,
}

/// Why an output step did not run to an outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StreamSkipReason {
    /// The execution strategy omits the step.
    NotConfigured,
    /// Execution ended before the step produced an outcome.
    NotReached,
    /// The module ID did not match the step's selector.
    NoMatch,
    /// The execution strategy explicitly skipped the step.
    StrategySkipped,
}

/// Outcome of a configured output-validation or after-middleware step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum StreamStepOutcome {
    /// The step completed successfully.
    Passed,
    /// The step failed, even if the strategy continued.
    Failed {
        /// How the strategy treated the failure.
        disposition: StreamFailureDisposition,
    },
    /// The step was skipped or never reached.
    Skipped {
        /// Why the step did not run to an outcome.
        reason: StreamSkipReason,
    },
}

impl StreamStepOutcome {
    fn pending(configured: bool) -> Self {
        Self::Skipped {
            reason: if configured {
                StreamSkipReason::NotReached
            } else {
                StreamSkipReason::NotConfigured
            },
        }
    }

    fn from_trace(step: &StepTrace) -> Self {
        let disposition = match step.skip_reason.as_deref() {
            Some("error_ignored") => StreamFailureDisposition::Ignored,
            Some("error_recovered") => StreamFailureDisposition::Recovered,
            _ if step.skipped => {
                return Self::Skipped {
                    reason: if step.skip_reason.as_deref() == Some("no_match") {
                        StreamSkipReason::NoMatch
                    } else {
                        StreamSkipReason::StrategySkipped
                    },
                };
            }
            _ if step.result.action == "abort" => StreamFailureDisposition::Unhandled,
            _ => return Self::Passed,
        };
        Self::Failed { disposition }
    }
}

/// The terminal result of one tracked streaming execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct StreamCompletion {
    /// First terminal outcome reported for the invocation.
    pub reason: StreamEndReason,
    /// Detached snapshot before output validation or after-middleware.
    /// Apply the caller's redaction policy before exposing it to clients.
    pub output: Option<Value>,
    /// Outcome of the configured output-validation step.
    pub output_validation: StreamStepOutcome,
    /// Outcome of the configured after-middleware step.
    pub middleware_after: StreamStepOutcome,
    /// Canonical error code if execution ended with an error.
    pub error_code: Option<String>,
}

impl StreamCompletion {
    fn pending(validation_configured: bool, after_configured: bool) -> Self {
        Self {
            reason: StreamEndReason::Completed,
            output: None,
            output_validation: StreamStepOutcome::pending(validation_configured),
            middleware_after: StreamStepOutcome::pending(after_configured),
            error_code: None,
        }
    }
}

/// The completion side of a tracked stream. Drain or close the owned chunks
/// before awaiting this future, since the chunk stream drives the invocation.
pub struct StreamCompletionHandle(oneshot::Receiver<StreamCompletion>);

impl Future for StreamCompletionHandle {
    type Output = StreamCompletion;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.0).poll(cx) {
            Poll::Ready(Ok(result)) => Poll::Ready(result),
            Poll::Ready(Err(_)) => {
                // The chunks owner normally sends before dropping its recorder.
                let mut result = StreamCompletion::pending(false, false);
                result.reason = StreamEndReason::ConsumerClosed;
                Poll::Ready(result)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

type ChunkSource<'a> = Pin<Box<dyn Stream<Item = Result<Value, ModuleError>> + Send + 'a>>;

/// The owned, pull-driven chunk stream for a tracked execution.
/// Dropping it before exhaustion closes the invocation.
pub struct StreamChunks<'a> {
    source: Option<ChunkSource<'a>>,
    recorder: StreamRecorder,
    token: CancelToken,
}

impl<'a> StreamChunks<'a> {
    pub(crate) fn new(
        source: ChunkSource<'a>,
        recorder: StreamRecorder,
        token: CancelToken,
    ) -> Self {
        Self {
            source: Some(source),
            recorder,
            token,
        }
    }

    /// End consumption early. The completion remains available from the
    /// separate handle, and closing an already finished stream has no effect.
    pub fn close(&mut self) {
        if self.source.is_some() {
            self.recorder.finish(StreamEndReason::ConsumerClosed, None);
            self.token.cancel();
            self.source = None;
        }
    }
}

impl Stream for StreamChunks<'_> {
    type Item = Result<Value, ModuleError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let Some(source) = self.source.as_mut() else {
            return Poll::Ready(None);
        };
        let polled = source.as_mut().poll_next(cx);
        match &polled {
            Poll::Ready(None) => {
                self.recorder.finish(StreamEndReason::Completed, None);
                self.source = None;
            }
            Poll::Ready(Some(Err(error))) => {
                self.recorder.finish(Self::error_reason(error), Some(error));
                self.source = None;
            }
            _ => {}
        }
        polled
    }
}

impl StreamChunks<'_> {
    fn error_reason(error: &ModuleError) -> StreamEndReason {
        match error.code {
            ErrorCode::ExecutionCancelled => StreamEndReason::Cancelled,
            ErrorCode::ModuleTimeout => StreamEndReason::TimedOut,
            _ => StreamEndReason::Failed,
        }
    }
}

impl Drop for StreamChunks<'_> {
    fn drop(&mut self) {
        self.close();
    }
}

struct CompletionState {
    sender: Option<oneshot::Sender<StreamCompletion>>,
    result: StreamCompletion,
    boundary_captured: bool,
}

impl CompletionState {
    fn finish(&mut self, reason: StreamEndReason, error: Option<&ModuleError>) {
        if let Some(sender) = self.sender.take() {
            self.result.reason = reason;
            self.result.error_code = error.map(|error| error.wire_code().into_owned());
            let result =
                std::mem::replace(&mut self.result, StreamCompletion::pending(false, false));
            let _ = sender.send(result);
        }
    }
}

/// Optional recorder for the ordinary streaming execution path.
#[derive(Clone)]
pub(crate) struct StreamRecorder(Arc<Mutex<CompletionState>>);

impl StreamRecorder {
    pub(crate) fn new(
        validation_configured: bool,
        after_configured: bool,
    ) -> (Self, StreamCompletionHandle) {
        let (sender, receiver) = oneshot::channel();
        let state = CompletionState {
            sender: Some(sender),
            result: StreamCompletion::pending(validation_configured, after_configured),
            boundary_captured: false,
        };
        (
            Self(Arc::new(Mutex::new(state))),
            StreamCompletionHandle(receiver),
        )
    }

    pub(crate) fn observe_step_input(&self, name: &str, output: Option<&Value>) {
        if !matches!(
            name,
            "output_validation" | "middleware_after" | "return_result"
        ) {
            return;
        }
        let mut state = self.0.lock();
        if name == "output_validation" || !state.boundary_captured {
            state.result.output = output.filter(|value| value.is_object()).cloned();
            state.boundary_captured = true;
        }
    }

    pub(crate) fn capture_if_missing(&self, output: &Value) {
        let mut state = self.0.lock();
        if !state.boundary_captured && state.result.output.is_none() && output.is_object() {
            state.result.output = Some(output.clone());
        }
    }

    pub(crate) fn record_steps(&self, steps: &[StepTrace]) {
        let mut state = self.0.lock();
        for step in steps {
            match step.name.as_str() {
                "output_validation" => {
                    state.result.output_validation = StreamStepOutcome::from_trace(step);
                }
                "middleware_after" => {
                    state.result.middleware_after = StreamStepOutcome::from_trace(step);
                }
                _ => {}
            }
        }
    }

    pub(crate) fn record_failure(&self, error: &ModuleError) {
        self.finish(StreamEndReason::Failed, Some(error));
    }

    fn finish(&self, reason: StreamEndReason, error: Option<&ModuleError>) {
        self.0.lock().finish(reason, error);
    }
}
