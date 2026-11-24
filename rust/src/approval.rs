// APCore Protocol — Approval workflow
// Spec reference: Approval requests, results, and handler trait

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use crate::context::Context;
use crate::errors::ModuleError;
use crate::module::ModuleAnnotations;

/// Approval request sent before a sensitive operation.
/// Spec §7.3.1: required fields are `module_id`, `arguments`, `context`,
/// `annotations`, `caller_id`, `action`.
///
/// Marked `#[non_exhaustive]` (issue #24) so a future spec revision can add a
/// field without a major version bump. That works by **removing struct-literal
/// construction from every crate but this one** — `..Default::default()`
/// included, since it is itself a struct expression (`error[E0639]`). From a
/// downstream crate, start from `ApprovalRequest::default()` and assign the
/// fields you need; there is no builder for this type. See
/// `api-surface-conventions.md` §9.1.
///
/// In practice the SDK builds this and hands it to your
/// [`ApprovalHandler`]; you rarely construct one outside tests. `module_id`
/// defaults to an empty string and SHOULD be set explicitly.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ApprovalRequest {
    pub module_id: String,
    pub arguments: serde_json::Value,
    /// The execution context (`trace_id`, identity, `call_chain`).
    /// Skipped during serialization as Context contains non-serializable runtime refs.
    #[serde(skip)]
    pub context: Option<Context<serde_json::Value>>,
    /// Module behavior annotations (`requires_approval` is guaranteed true).
    #[serde(default)]
    pub annotations: ModuleAnnotations,
    /// The invoking module's canonical ID, read straight off
    /// `context.caller_id` with no substitution — `None` for a top-level /
    /// external call, exactly as `Context::caller_id` is (§5.7), NOT the
    /// `"@external"` sentinel ACL evaluation substitutes internally.
    /// (Decision D-03, spec v1.32.0 §7.3.1.)
    #[serde(default)]
    pub caller_id: Option<String>,
    /// The target module's canonical ID being requested — a flat duplicate of
    /// `module_id`, not a separate free-text label, so a handler can read
    /// `request.action` without cross-referencing `request.module_id` under a
    /// second name. (Decision D-03, spec v1.32.0 §7.3.1.)
    #[serde(default)]
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Outcome of an approval request.
///
/// Marked `#[non_exhaustive]` (issue #24) so a future spec revision can add a
/// field without a major version bump. That works by **removing struct-literal
/// construction from every crate but this one** — `..Default::default()`
/// included, since it is itself a struct expression (`error[E0639]`).
///
/// Prefer the semantic constructors, which set `status` to a canonical value
/// (`api-surface-conventions.md` §9.2 rule 4): [`ApprovalResult::approved`] and
/// [`ApprovalResult::rejected`]. `Default::default()` plus field assignment
/// still works for the `"timeout"` / `"pending"` statuses, but note that
/// `status` then defaults to the **empty string**, which is not a valid
/// decision — a default-constructed value is structurally valid and
/// semantically empty, which is what the constructors exist to prevent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ApprovalResult {
    /// "approved", "rejected", "timeout", or "pending"
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approved_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

impl ApprovalResult {
    /// An `"approved"` result attributed to `by`.
    ///
    /// ```
    /// use apcore::ApprovalResult;
    ///
    /// let result = ApprovalResult::approved("slack_user");
    /// assert_eq!(result.status, "approved");
    /// assert_eq!(result.approved_by.as_deref(), Some("slack_user"));
    /// ```
    #[must_use]
    pub fn approved(by: impl Into<String>) -> Self {
        Self {
            status: "approved".to_string(),
            approved_by: Some(by.into()),
            reason: None,
            approval_id: None,
            metadata: None,
        }
    }

    /// A `"rejected"` result carrying the human-readable `reason`.
    ///
    /// ```
    /// use apcore::ApprovalResult;
    ///
    /// let result = ApprovalResult::rejected("out of budget");
    /// assert_eq!(result.status, "rejected");
    /// assert_eq!(result.reason.as_deref(), Some("out of budget"));
    /// ```
    #[must_use]
    pub fn rejected(reason: impl Into<String>) -> Self {
        Self {
            status: "rejected".to_string(),
            approved_by: None,
            reason: Some(reason.into()),
            approval_id: None,
            metadata: None,
        }
    }
}

/// Trait for handling approval requests.
#[async_trait]
pub trait ApprovalHandler: Send + Sync + std::fmt::Debug {
    /// Request approval for an operation. Returns the result.
    async fn request_approval(
        &self,
        request: &ApprovalRequest,
    ) -> Result<ApprovalResult, ModuleError>;

    /// Check the current status of a pending approval by ID.
    async fn check_approval(&self, approval_id: &str) -> Result<ApprovalResult, ModuleError>;
}

/// An approval handler that automatically approves all requests.
#[derive(Debug, Clone)]
pub struct AutoApproveHandler;

#[async_trait]
impl ApprovalHandler for AutoApproveHandler {
    async fn request_approval(
        &self,
        _request: &ApprovalRequest,
    ) -> Result<ApprovalResult, ModuleError> {
        Ok(ApprovalResult::approved("auto"))
    }

    async fn check_approval(&self, _approval_id: &str) -> Result<ApprovalResult, ModuleError> {
        Ok(ApprovalResult::approved("auto"))
    }
}

/// The stored form both `CallbackApprovalHandler` constructors erase into.
///
/// Boxing the returned future keeps [`CallbackApprovalHandler`] **non-generic**,
/// so `Box::new(handler)` still coerces to `Box<dyn ApprovalHandler>` and the
/// type can be named in a struct field without a type parameter.
type BoxedApprovalCallback = Box<
    dyn Fn(
            ApprovalRequest,
        ) -> Pin<Box<dyn Future<Output = Result<ApprovalResult, ModuleError>> + Send>>
        + Send
        + Sync,
>;

/// An approval handler that delegates to a user-provided callback.
///
/// The callback is **asynchronous and fallible** — the semantics this name
/// already carries in apcore-python (`Callable[[ApprovalRequest],
/// Coroutine[…, ApprovalResult]]`) and apcore-typescript
/// (`(request) => Promise<ApprovalResult>`). See apcore#104: until spec
/// v1.24.0 the Rust constructor took a synchronous, infallible
/// `Fn(&ApprovalRequest) -> ApprovalResult`, which could express neither an
/// approval decision that performs I/O (a Slack round-trip) nor one that
/// fails. The failure case was the worse of the two: an author whose Slack API
/// was down had to choose between panicking inside the approval gate and
/// returning a `"rejected"` result, which reads in the audit log exactly like a
/// human saying no.
///
/// Use [`Self::new_sync`] for an in-process decision that performs no I/O. It
/// also returns a `Result`, because the silent-rejection hole is independent of
/// the async question.
///
/// `check_approval` returns `"rejected"` by default, since callback handlers
/// typically do not support Phase B async resume.
///
/// # Taking the request by value
///
/// The callback receives an owned [`ApprovalRequest`]. That clone does **not**
/// break the spec's promise that `Context.data` travels by reference:
/// `Context.data`, `cancel_token` and `executor` all sit behind an `Arc`, so
/// the clone shares them. What is deep-copied is `arguments`, `identity` and
/// `call_chain` — the read-only inputs to an approval decision. An approval
/// waits on a human, so one `serde_json::Value` clone against seconds-to-hours
/// of latency is not a cost worth designing around.
///
/// # Example
///
/// ```
/// use apcore::{ApprovalResult, CallbackApprovalHandler};
///
/// let handler = CallbackApprovalHandler::new(|req| async move {
///     // `await` a webhook, a queue, a database — anything.
///     Ok(ApprovalResult::approved(format!("callback:{}", req.module_id)))
/// });
/// ```
pub struct CallbackApprovalHandler {
    callback: BoxedApprovalCallback,
}

impl CallbackApprovalHandler {
    /// Build a handler from an **async, fallible** callback.
    ///
    /// This is an ordinary closure returning an `async move` block; no async
    /// closures (and so no raised MSRV) are involved. `Fut` is boxed on the way
    /// in so the handler type stays non-generic.
    ///
    /// An `Err` propagates out of the approval gate exactly as a raised
    /// exception does in apcore-python and a rejected promise in
    /// apcore-typescript — a failing approval decision is a first-class
    /// outcome, distinct from a rejection.
    ///
    /// ```
    /// use apcore::{ApprovalResult, CallbackApprovalHandler, ErrorCode, ModuleError};
    ///
    /// let handler = CallbackApprovalHandler::new(|_req| async move {
    ///     Err::<ApprovalResult, _>(ModuleError::new(
    ///         ErrorCode::GeneralInternalError,
    ///         "approval service unreachable",
    ///     ))
    /// });
    /// ```
    pub fn new<F, Fut>(callback: F) -> Self
    where
        F: Fn(ApprovalRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<ApprovalResult, ModuleError>> + Send + 'static,
    {
        Self {
            callback: Box::new(move |request| Box::pin(callback(request))),
        }
    }

    /// Build a handler from a **synchronous, fallible** callback, for
    /// in-process decisions that perform no I/O.
    ///
    /// Returns a `Result` to match [`Self::new`]: a policy check that cannot
    /// reach its rule store must be able to say so rather than fabricate a
    /// rejection.
    ///
    /// ```
    /// use apcore::{ApprovalResult, CallbackApprovalHandler};
    ///
    /// let handler = CallbackApprovalHandler::new_sync(|req| {
    ///     Ok(if req.annotations.destructive {
    ///         ApprovalResult::rejected("destructive modules need a human")
    ///     } else {
    ///         ApprovalResult::approved("policy")
    ///     })
    /// });
    /// ```
    pub fn new_sync(
        callback: impl Fn(&ApprovalRequest) -> Result<ApprovalResult, ModuleError>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        Self {
            callback: Box::new(move |request| {
                let result = callback(&request);
                Box::pin(std::future::ready(result))
            }),
        }
    }
}

impl std::fmt::Debug for CallbackApprovalHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallbackApprovalHandler")
            .field("callback", &"<closure>")
            .finish()
    }
}

#[async_trait]
impl ApprovalHandler for CallbackApprovalHandler {
    async fn request_approval(
        &self,
        request: &ApprovalRequest,
    ) -> Result<ApprovalResult, ModuleError> {
        (self.callback)(request.clone()).await
    }

    async fn check_approval(&self, _approval_id: &str) -> Result<ApprovalResult, ModuleError> {
        Ok(ApprovalResult::rejected(
            "Phase B not supported by callback handler",
        ))
    }
}

/// An approval handler that automatically denies all requests.
#[derive(Debug, Clone)]
pub struct AlwaysDenyHandler;

#[async_trait]
impl ApprovalHandler for AlwaysDenyHandler {
    async fn request_approval(
        &self,
        _request: &ApprovalRequest,
    ) -> Result<ApprovalResult, ModuleError> {
        Ok(ApprovalResult::rejected("Always denied"))
    }

    async fn check_approval(&self, _approval_id: &str) -> Result<ApprovalResult, ModuleError> {
        Ok(ApprovalResult::rejected("Always denied"))
    }
}
