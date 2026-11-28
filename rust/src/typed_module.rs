use std::collections::HashMap;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;

use schemars::{gen::SchemaSettings, JsonSchema};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;

use crate::{APCore, Context, ErrorCode, FunctionModule, Module, ModuleAnnotations, ModuleError};

/// A typed reference to a registered module ID.
///
/// Calling the handle uses the same registry, middleware and governance as
/// [`APCore::call`].
pub struct TypedHandle<I, O> {
    module_id: &'static str,
    types: PhantomData<fn(I) -> O>,
}

impl<I, O> TypedHandle<I, O> {
    #[doc(hidden)]
    pub const fn new(module_id: &'static str) -> Self {
        Self {
            module_id,
            types: PhantomData,
        }
    }

    /// Return the registry ID used by this handle.
    pub const fn module_id(&self) -> &'static str {
        self.module_id
    }
}

impl<I: Serialize, O: DeserializeOwned> TypedHandle<I, O> {
    /// Serialize input, invoke the governed module, and decode its result.
    pub async fn invoke(
        &self,
        client: &APCore,
        input: I,
        context: Option<&Context<Value>>,
    ) -> Result<O, ModuleError> {
        let input = encode(&input).map_err(ModuleError::invalid_input)?;
        let output = client.call(self.module_id, input, context, None).await?;
        serde_json::from_value(output).map_err(|_| {
            ModuleError::new(
                ErrorCode::SchemaValidationError,
                "Module output cannot be decoded as the handle's output type",
            )
        })
    }
}

/// A module definition materialized by the attribute macro.
#[derive(Clone, Copy)]
pub struct ModuleDeclaration {
    module_id: &'static str,
    build: fn() -> Result<Box<dyn Module>, ModuleError>,
}

impl ModuleDeclaration {
    #[doc(hidden)]
    pub const fn new(
        module_id: &'static str,
        build: fn() -> Result<Box<dyn Module>, ModuleError>,
    ) -> Self {
        Self { module_id, build }
    }

    /// Return the ID that registration assigns to the module.
    pub const fn module_id(&self) -> &'static str {
        self.module_id
    }

    /// Materialize and register the declared module on this client.
    pub fn register(&self, client: &APCore) -> Result<(), ModuleError> {
        client.register(self.module_id, (self.build)()?)
    }
}

/// A static collection of explicitly selected module declarations.
#[derive(Clone, Copy)]
pub struct ModulePack {
    declarations: &'static [ModuleDeclaration],
}

impl ModulePack {
    /// Create a pack in registration order.
    pub const fn new(declarations: &'static [ModuleDeclaration]) -> Self {
        Self { declarations }
    }

    /// Register declarations in order. Earlier registrations remain on error.
    pub fn register(&self, client: &APCore) -> Result<(), ModuleError> {
        for declaration in self.declarations {
            declaration.register(client)?;
        }
        Ok(())
    }
}

#[doc(hidden)]
pub fn __declare_module<I, O>(
    description: &str,
    documentation: Option<&str>,
    annotations: ModuleAnnotations,
    handler: impl for<'a> Fn(
            I,
            &'a Context<Value>,
        ) -> Pin<Box<dyn Future<Output = Result<O, ModuleError>> + Send + 'a>>
        + Send
        + Sync
        + 'static,
) -> Result<Box<dyn Module>, ModuleError>
where
    I: JsonSchema + Serialize + DeserializeOwned + Send + 'static,
    O: JsonSchema + Serialize + Send + 'static,
{
    let mut module = FunctionModule::with_description(
        annotations,
        draft2020_12::<I>()?,
        draft2020_12::<O>()?,
        description,
        documentation.map(str::to_owned),
        Vec::new(),
        crate::registry::registry::DEFAULT_MODULE_VERSION,
        HashMap::new(),
        Vec::new(),
        move |inputs, context| {
            let future = decode::<I>(&inputs).and_then(|input| {
                if encode_input(&input)? != inputs {
                    return Err(ModuleError::invalid_input(
                        "Typed module arguments changed after normalization",
                    ));
                }
                Ok(handler(input, context))
            });
            Box::pin(async move {
                encode(&future?.await?)
                    .map_err(|message| ModuleError::new(ErrorCode::GeneralInternalError, message))
            })
        },
    );
    module.set_input_normalizer(|inputs| encode_input(&decode::<I>(inputs)?));
    Ok(Box::new(module))
}

fn decode<T: DeserializeOwned>(value: &Value) -> Result<T, ModuleError> {
    T::deserialize(value).map_err(|_| {
        ModuleError::invalid_input("Arguments cannot be decoded as the declared input")
    })
}

fn encode<T: Serialize>(value: &T) -> Result<Value, &'static str> {
    serde_json::to_value(value).map_err(|_| "Declared value cannot be encoded as JSON")
}

fn encode_input<T: Serialize>(value: &T) -> Result<Value, ModuleError> {
    let inputs = encode(value).map_err(ModuleError::invalid_input)?;
    if !inputs.is_object() || inputs.get(crate::policy::APPROVAL_TOKEN_KEY).is_some() {
        return Err(ModuleError::invalid_input(
            "Typed module input must encode a JSON object without reserved approval credentials",
        ));
    }
    Ok(inputs)
}

fn draft2020_12<T: JsonSchema>() -> Result<Value, ModuleError> {
    let settings = SchemaSettings::draft2019_09().with(|settings| {
        settings.meta_schema = Some("https://json-schema.org/draft/2020-12/schema".into());
        settings.definitions_path = "#/$defs/".into();
    });
    let mut schema = serde_json::to_value(settings.into_generator().into_root_schema_for::<T>())
        .map_err(|_| {
            ModuleError::new(
                ErrorCode::SchemaParseError,
                "Could not encode the declared JSON Schema",
            )
        })?;
    if let Some(root) = schema.as_object_mut() {
        if let Some(definitions) = root.remove("definitions") {
            root.insert("$defs".into(), definitions);
        }
    }
    update_tuple_keywords(&mut schema);
    Ok(schema)
}

// Schemars 0.8 emits the 2019-09 tuple encoding. Walk only schema positions
// so literal values such as a default or enum choice remain untouched.
fn update_tuple_keywords(schema: &mut Value) {
    let Some(schema) = schema.as_object_mut() else {
        return;
    };
    if schema.get("items").is_some_and(Value::is_array) {
        if let Some(items) = schema.remove("items") {
            schema.insert("prefixItems".into(), items);
        }
        if let Some(additional) = schema.remove("additionalItems") {
            schema.insert("items".into(), additional);
        }
    }
    for keyword in [
        "$defs",
        "properties",
        "patternProperties",
        "dependentSchemas",
    ] {
        if let Some(children) = schema.get_mut(keyword).and_then(Value::as_object_mut) {
            children.values_mut().for_each(update_tuple_keywords);
        }
    }
    for keyword in ["allOf", "anyOf", "oneOf", "prefixItems"] {
        if let Some(children) = schema.get_mut(keyword).and_then(Value::as_array_mut) {
            children.iter_mut().for_each(update_tuple_keywords);
        }
    }
    for keyword in [
        "additionalProperties",
        "contains",
        "else",
        "if",
        "items",
        "not",
        "propertyNames",
        "then",
        "unevaluatedItems",
        "unevaluatedProperties",
    ] {
        if let Some(child) = schema.get_mut(keyword) {
            update_tuple_keywords(child);
        }
    }
}
