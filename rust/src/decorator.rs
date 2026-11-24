// APCore Protocol — Decorator / FunctionModule
// Spec reference: Function-based module creation via attribute macros
//
// In Rust, the #[module] attribute macro concept would be implemented
// as a proc-macro in a separate crate. This file provides the runtime
// support types for function-based modules.

use async_trait::async_trait;

use std::collections::HashMap;

use crate::context::Context;
use crate::errors::ModuleError;
use crate::module::{Module, ModuleAnnotations, ModuleExample};

/// Boxed async handler type for `FunctionModule`.
type HandlerFn = Box<
    dyn for<'a> Fn(
            serde_json::Value,
            &'a Context<serde_json::Value>,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<serde_json::Value, ModuleError>>
                    + Send
                    + 'a,
            >,
        > + Send
        + Sync,
>;

/// A module implemented as a wrapped async function.
pub struct FunctionModule {
    pub annotations: ModuleAnnotations,
    pub input_schema: serde_json::Value,
    pub output_schema: serde_json::Value,
    description: String,
    pub documentation: Option<String>,
    pub tags: Vec<String>,
    pub version: String,
    pub metadata: HashMap<String, serde_json::Value>,
    pub examples: Vec<ModuleExample>,
    handler: HandlerFn,
}

impl std::fmt::Debug for FunctionModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FunctionModule")
            .field("annotations", &self.annotations)
            .finish_non_exhaustive() // handler (fn pointer) and schemas are not Debug
    }
}

impl FunctionModule {
    /// Create a new `FunctionModule` wrapping an async handler.
    pub fn new<F, Fut>(
        annotations: ModuleAnnotations,
        input_schema: serde_json::Value,
        output_schema: serde_json::Value,
        handler: F,
    ) -> Self
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
        Self {
            annotations,
            input_schema,
            output_schema,
            description: String::new(),
            documentation: None,
            tags: vec![],
            version: "0.1.0".to_string(),
            metadata: HashMap::new(),
            examples: vec![],
            handler: Box::new(handler),
        }
    }

    /// Create a new `FunctionModule` with an explicit description and optional metadata.
    ///
    /// This is used by [`APCore::module()`](crate::client::APCore::module) to
    /// propagate the caller-supplied description and metadata into the module.
    #[allow(clippy::too_many_arguments)]
    pub fn with_description<F>(
        annotations: ModuleAnnotations,
        input_schema: serde_json::Value,
        output_schema: serde_json::Value,
        description: impl Into<String>,
        documentation: Option<String>,
        tags: Vec<String>,
        version: impl Into<String>,
        metadata: HashMap<String, serde_json::Value>,
        examples: Vec<ModuleExample>,
        handler: F,
    ) -> Self
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
        Self {
            annotations,
            input_schema,
            output_schema,
            description: description.into(),
            documentation,
            tags,
            version: version.into(),
            metadata,
            examples,
            handler: Box::new(handler),
        }
    }
}

#[async_trait]
impl Module for FunctionModule {
    fn input_schema(&self) -> serde_json::Value {
        self.input_schema.clone()
    }

    fn output_schema(&self) -> serde_json::Value {
        self.output_schema.clone()
    }

    fn description(&self) -> &str {
        &self.description
    }

    // A `FunctionModule` is built from a declaration — `APCore::module()` or a
    // `*.binding.yaml` entry — and those declarations are stored on the struct
    // below. Without these four accessors the trait defaults answered instead
    // (`ModuleAnnotations::default()`, an empty tag list, `None`, an empty
    // map), so `Registry::register_module`, `BuiltinApprovalGate` and every
    // other `dyn Module` reader saw a module that declared nothing — and a
    // binding's `annotations: {requires_approval: true}` was discarded at
    // registration. Cross-language parity with apcore-python (`decorator.py`
    // attaches the declared values to the wrapped function) and
    // apcore-typescript (`bindings.ts` attaches them to the module object).
    fn annotations(&self) -> ModuleAnnotations {
        self.annotations.clone()
    }

    fn tags(&self) -> Vec<String> {
        self.tags.clone()
    }

    fn documentation(&self) -> Option<&str> {
        self.documentation.as_deref()
    }

    fn metadata(&self) -> HashMap<String, serde_json::Value> {
        self.metadata.clone()
    }

    fn version(&self) -> Option<&str> {
        Some(self.version.as_str())
    }

    fn examples(&self) -> Vec<ModuleExample> {
        self.examples.clone()
    }

    fn dependencies(&self) -> Vec<crate::registry::registry::DependencyInfo> {
        crate::registry::registry::dependencies_from_metadata(&self.metadata)
    }

    async fn execute(
        &self,
        inputs: serde_json::Value,
        ctx: &Context<serde_json::Value>,
    ) -> Result<serde_json::Value, ModuleError> {
        (self.handler)(inputs, ctx).await
    }
}
