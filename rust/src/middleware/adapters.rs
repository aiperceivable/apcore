// APCore Protocol — Middleware adapter traits
// Spec reference: Simplified before-only and after-only middleware

use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;

use super::base::Middleware;
use crate::context::Context;
use crate::errors::ModuleError;

/// Adapter for middleware that only needs a before hook.
#[async_trait]
pub trait BeforeMiddleware: Send + Sync + std::fmt::Debug {
    fn name(&self) -> &str;

    async fn before(
        &self,
        module_id: &str,
        inputs: serde_json::Value,
        ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError>;
}

/// Adapter for middleware that only needs an after hook.
#[async_trait]
pub trait AfterMiddleware: Send + Sync + std::fmt::Debug {
    fn name(&self) -> &str;

    async fn after(
        &self,
        module_id: &str,
        inputs: serde_json::Value,
        output: serde_json::Value,
        ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError>;
}

// ---------------------------------------------------------------------------
// Closure adapters — register before-only / after-only closures as Middleware
// ---------------------------------------------------------------------------

/// Closure-based [`Middleware`] that only acts on the `before` hook.
///
/// `BeforeAdapter` lets users register a plain async closure as a fully-fledged
/// middleware (registrable via [`MiddlewareManager::add`](super::manager::MiddlewareManager::add))
/// without having to define a new struct + `impl Middleware`. The `after` and
/// `on_error` hooks are no-ops (return `Ok(None)`).
///
/// The closure receives **owned** copies of `module_id`, `inputs`, and `ctx`,
/// so it can freely capture state with `move`.
///
/// Cross-language parity with apcore-python's `before_middleware()` decorator
/// and apcore-typescript's `BeforeMiddleware` helper (sync finding A-D-402).
pub struct BeforeAdapter<F> {
    name: String,
    callback: Arc<F>,
}

impl<F> std::fmt::Debug for BeforeAdapter<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BeforeAdapter")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl<F, Fut> BeforeAdapter<F>
where
    F: Fn(String, serde_json::Value, Context<serde_json::Value>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Option<serde_json::Value>, ModuleError>> + Send + 'static,
{
    /// Create a `BeforeAdapter` with the given name and async callback.
    pub fn new(name: impl Into<String>, callback: F) -> Self {
        Self {
            name: name.into(),
            callback: Arc::new(callback),
        }
    }
}

#[async_trait]
impl<F, Fut> Middleware for BeforeAdapter<F>
where
    F: Fn(String, serde_json::Value, Context<serde_json::Value>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Option<serde_json::Value>, ModuleError>> + Send + 'static,
{
    fn name(&self) -> &str {
        &self.name
    }

    async fn before(
        &self,
        module_id: &str,
        inputs: serde_json::Value,
        ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        (self.callback)(module_id.to_string(), inputs, ctx.clone()).await
    }

    async fn after(
        &self,
        _module_id: &str,
        _inputs: serde_json::Value,
        _output: serde_json::Value,
        _ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        Ok(None)
    }

    async fn on_error(
        &self,
        _module_id: &str,
        _inputs: serde_json::Value,
        _error: &ModuleError,
        _ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        Ok(None)
    }
}

/// Closure-based [`Middleware`] that only acts on the `after` hook.
///
/// See [`BeforeAdapter`] — `AfterAdapter` is the mirror image, running on
/// successful module output rather than on input. The `before` and
/// `on_error` hooks are no-ops (sync finding A-D-402).
pub struct AfterAdapter<F> {
    name: String,
    callback: Arc<F>,
}

impl<F> std::fmt::Debug for AfterAdapter<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AfterAdapter")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl<F, Fut> AfterAdapter<F>
where
    F: Fn(String, serde_json::Value, serde_json::Value, Context<serde_json::Value>) -> Fut
        + Send
        + Sync
        + 'static,
    Fut: Future<Output = Result<Option<serde_json::Value>, ModuleError>> + Send + 'static,
{
    /// Create an `AfterAdapter` with the given name and async callback.
    pub fn new(name: impl Into<String>, callback: F) -> Self {
        Self {
            name: name.into(),
            callback: Arc::new(callback),
        }
    }
}

#[async_trait]
impl<F, Fut> Middleware for AfterAdapter<F>
where
    F: Fn(String, serde_json::Value, serde_json::Value, Context<serde_json::Value>) -> Fut
        + Send
        + Sync
        + 'static,
    Fut: Future<Output = Result<Option<serde_json::Value>, ModuleError>> + Send + 'static,
{
    fn name(&self) -> &str {
        &self.name
    }

    async fn before(
        &self,
        _module_id: &str,
        _inputs: serde_json::Value,
        _ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        Ok(None)
    }

    async fn after(
        &self,
        module_id: &str,
        inputs: serde_json::Value,
        output: serde_json::Value,
        ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        (self.callback)(module_id.to_string(), inputs, output, ctx.clone()).await
    }

    async fn on_error(
        &self,
        _module_id: &str,
        _inputs: serde_json::Value,
        _error: &ModuleError,
        _ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Debug)]
    struct LoggingBeforeMiddleware;

    #[async_trait]
    impl BeforeMiddleware for LoggingBeforeMiddleware {
        fn name(&self) -> &'static str {
            "logging_before"
        }

        async fn before(
            &self,
            _module_id: &str,
            inputs: serde_json::Value,
            _ctx: &Context<serde_json::Value>,
        ) -> Result<Option<serde_json::Value>, ModuleError> {
            Ok(Some(inputs))
        }
    }

    #[derive(Debug)]
    struct LoggingAfterMiddleware;

    #[async_trait]
    impl AfterMiddleware for LoggingAfterMiddleware {
        fn name(&self) -> &'static str {
            "logging_after"
        }

        async fn after(
            &self,
            _module_id: &str,
            _inputs: serde_json::Value,
            output: serde_json::Value,
            _ctx: &Context<serde_json::Value>,
        ) -> Result<Option<serde_json::Value>, ModuleError> {
            Ok(Some(output))
        }
    }

    #[tokio::test]
    async fn test_before_middleware_name() {
        let mw = LoggingBeforeMiddleware;
        assert_eq!(mw.name(), "logging_before");
    }

    #[tokio::test]
    async fn test_before_middleware_passthrough() {
        let mw = LoggingBeforeMiddleware;
        let ctx = Context::<serde_json::Value>::anonymous();
        let inputs = json!({"key": "value"});
        let result = mw
            .before("test.module", inputs.clone(), &ctx)
            .await
            .unwrap();
        assert_eq!(result, Some(inputs));
    }

    #[tokio::test]
    async fn test_after_middleware_name() {
        let mw = LoggingAfterMiddleware;
        assert_eq!(mw.name(), "logging_after");
    }

    #[tokio::test]
    async fn test_after_middleware_passthrough() {
        let mw = LoggingAfterMiddleware;
        let ctx = Context::<serde_json::Value>::anonymous();
        let inputs = json!({"in": 1});
        let output = json!({"out": 2});
        let result = mw
            .after("test.module", inputs, output.clone(), &ctx)
            .await
            .unwrap();
        assert_eq!(result, Some(output));
    }

    #[tokio::test]
    async fn test_before_middleware_returns_none() {
        #[derive(Debug)]
        struct NoOpBefore;

        #[async_trait]
        impl BeforeMiddleware for NoOpBefore {
            fn name(&self) -> &'static str {
                "noop"
            }
            async fn before(
                &self,
                _module_id: &str,
                _inputs: serde_json::Value,
                _ctx: &Context<serde_json::Value>,
            ) -> Result<Option<serde_json::Value>, ModuleError> {
                Ok(None)
            }
        }

        let mw = NoOpBefore;
        let ctx = Context::<serde_json::Value>::anonymous();
        let result = mw.before("m", json!({}), &ctx).await.unwrap();
        assert_eq!(result, None);
    }
}
