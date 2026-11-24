//! Typed key for type-safe access to `Context.data`.

use std::borrow::Cow;
use std::marker::PhantomData;

use crate::context::Context;

/// Typed accessor for a named slot within `Context.data`.
///
/// Provides get/set/delete/exists operations with namespace isolation
/// via the `scoped()` factory.
pub struct ContextKey<T> {
    /// The string key used in the data map.
    pub name: Cow<'static, str>,
    _marker: PhantomData<T>,
}

impl<T> ContextKey<T> {
    /// Create a new key from a static string (zero-allocation).
    #[must_use]
    pub const fn new(name: &'static str) -> Self {
        Self {
            name: Cow::Borrowed(name),
            _marker: PhantomData,
        }
    }

    /// Create a sub-key with `{name}.{suffix}`.
    #[must_use]
    pub fn scoped(&self, suffix: &str) -> ContextKey<T> {
        ContextKey {
            name: Cow::Owned(format!("{}.{}", self.name, suffix)),
            _marker: PhantomData,
        }
    }
}

impl<T: serde::de::DeserializeOwned> ContextKey<T> {
    /// Return the deserialized value for this key, or `None` if absent or
    /// deserialization fails.
    pub fn get<S>(&self, ctx: &Context<S>) -> Option<T> {
        let map = ctx.data.read();
        let val = map.get(self.name.as_ref())?;
        serde_json::from_value(val.clone()).ok()
    }

    /// Return true if this key is present in context.data.
    pub fn exists<S>(&self, ctx: &Context<S>) -> bool {
        ctx.data.read().contains_key(self.name.as_ref())
    }
}

impl<T: serde::Serialize> ContextKey<T> {
    /// Store `value` under this key in context.data.
    pub fn set<S>(&self, ctx: &Context<S>, value: T) {
        let mut map = ctx.data.write();
        if let Ok(v) = serde_json::to_value(value) {
            map.insert(self.name.to_string(), v);
        }
    }

    /// Remove this key from context.data (no-op if absent).
    pub fn delete<S>(&self, ctx: &Context<S>) {
        let mut map = ctx.data.write();
        map.remove(self.name.as_ref());
    }
}
