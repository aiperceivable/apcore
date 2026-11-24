// APCore Protocol — Extension point system
// Spec reference: Registration, query, and wiring of custom extensions
// (discoverers, middleware, ACL providers, span exporters, module validators,
// and approval handlers) into the apcore runtime.

use std::collections::HashMap;
use std::sync::Arc;

use crate::acl::ACL;
use crate::approval::ApprovalHandler;
use crate::errors::{ErrorCode, ModuleError};
use crate::executor::Executor;
use crate::middleware::base::Middleware;
use crate::observability::span::SpanExporter;
use crate::observability::tracing_middleware::TracingMiddleware;
use crate::registry::registry::{Discoverer, ModuleValidator, Registry};

// ---------------------------------------------------------------------------
// ExtensionPoint — describes a named slot where extensions can be registered
// ---------------------------------------------------------------------------

/// Describes a named slot where extensions can be registered.
#[derive(Debug, Clone)]
pub struct ExtensionPoint {
    /// Name of the extension point (e.g. "middleware").
    pub name: String,
    /// Human-readable description of what this point accepts.
    pub description: String,
    /// Whether multiple extensions can be registered at this point.
    pub multiple: bool,
}

// ---------------------------------------------------------------------------
// ExtensionKind — type-safe enum of extension instances
// ---------------------------------------------------------------------------

/// A type-safe wrapper for the different kinds of extensions that can be
/// registered. This replaces the Python/TypeScript `Any` approach with Rust
/// enums so the type system enforces correctness at compile time.
///
/// Each variant holds an `Arc`, not a `Box`, because
/// [`ExtensionManager::apply`] must retain its registrations (D-78): it wires a
/// *clone* of the handle into the registry/executor and keeps its own. The same
/// property makes [`ExtensionManager::unregister`]'s identity comparison
/// expressible from outside the manager (D-91) — hold the `Arc`, register a
/// clone of it, and hand back another clone to remove it.
pub enum ExtensionKind {
    /// A custom module discovery strategy.
    Discoverer(Arc<dyn Discoverer>),
    /// Execution middleware.
    Middleware(Arc<dyn Middleware>),
    /// Access control provider.
    Acl(Arc<ACL>),
    /// Tracing span exporter.
    SpanExporter(Arc<dyn SpanExporter>),
    /// Custom module validation.
    ModuleValidator(Arc<dyn ModuleValidator>),
    /// Approval handler for Step 4.5 gate.
    ApprovalHandler(Arc<dyn ApprovalHandler>),
}

impl std::fmt::Debug for ExtensionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExtensionKind::Discoverer(_) => write!(f, "ExtensionKind::Discoverer(...)"),
            ExtensionKind::Middleware(m) => write!(f, "ExtensionKind::Middleware({m:?})"),
            ExtensionKind::Acl(_) => write!(f, "ExtensionKind::Acl(...)"),
            ExtensionKind::SpanExporter(e) => {
                write!(f, "ExtensionKind::SpanExporter({e:?})")
            }
            ExtensionKind::ModuleValidator(_) => {
                write!(f, "ExtensionKind::ModuleValidator(...)")
            }
            ExtensionKind::ApprovalHandler(h) => {
                write!(f, "ExtensionKind::ApprovalHandler({h:?})")
            }
        }
    }
}

impl ExtensionKind {
    /// Address of the extension object this variant holds.
    ///
    /// The identity [`ExtensionManager::unregister`] compares on — the address
    /// of the extension itself, not of the enum wrapping it, so two
    /// `ExtensionKind` values built around the same object compare equal.
    /// Because the variants hold `Arc`s, two clones of one handle also compare
    /// equal, which is what makes an outside-the-manager removal expressible
    /// (D-91).
    fn object_address(&self) -> *const () {
        match self {
            ExtensionKind::Discoverer(d) => std::ptr::from_ref(&**d).cast::<()>(),
            ExtensionKind::Middleware(m) => std::ptr::from_ref(&**m).cast::<()>(),
            ExtensionKind::Acl(acl) => std::ptr::from_ref(&**acl).cast::<()>(),
            ExtensionKind::SpanExporter(e) => std::ptr::from_ref(&**e).cast::<()>(),
            ExtensionKind::ModuleValidator(v) => std::ptr::from_ref(&**v).cast::<()>(),
            ExtensionKind::ApprovalHandler(h) => std::ptr::from_ref(&**h).cast::<()>(),
        }
    }

    /// Return the extension point name this kind corresponds to.
    fn point_name(&self) -> &str {
        match self {
            ExtensionKind::Discoverer(_) => "discoverer",
            ExtensionKind::Middleware(_) => "middleware",
            ExtensionKind::Acl(_) => "acl",
            ExtensionKind::SpanExporter(_) => "span_exporter",
            ExtensionKind::ModuleValidator(_) => "module_validator",
            ExtensionKind::ApprovalHandler(_) => "approval_handler",
        }
    }
}

// ---------------------------------------------------------------------------
// Built-in extension points
// ---------------------------------------------------------------------------

fn built_in_points() -> HashMap<String, ExtensionPoint> {
    let mut points = HashMap::new();
    points.insert(
        "discoverer".to_string(),
        ExtensionPoint {
            name: "discoverer".to_string(),
            description: "Custom module discovery strategy".to_string(),
            multiple: false,
        },
    );
    points.insert(
        "middleware".to_string(),
        ExtensionPoint {
            name: "middleware".to_string(),
            description: "Execution middleware".to_string(),
            multiple: true,
        },
    );
    points.insert(
        "acl".to_string(),
        ExtensionPoint {
            name: "acl".to_string(),
            description: "Access control provider".to_string(),
            multiple: false,
        },
    );
    points.insert(
        "span_exporter".to_string(),
        ExtensionPoint {
            name: "span_exporter".to_string(),
            description: "Tracing span exporter".to_string(),
            multiple: true,
        },
    );
    points.insert(
        "module_validator".to_string(),
        ExtensionPoint {
            name: "module_validator".to_string(),
            description: "Custom module validation".to_string(),
            multiple: false,
        },
    );
    points.insert(
        "approval_handler".to_string(),
        ExtensionPoint {
            name: "approval_handler".to_string(),
            description: "Approval handler for Step 4.5 gate".to_string(),
            multiple: false,
        },
    );
    points
}

// ---------------------------------------------------------------------------
// ExtensionManager
// ---------------------------------------------------------------------------

/// Manages extension points and their registered implementations.
///
/// Pre-registers six built-in extension points: discoverer, middleware,
/// acl, `span_exporter`, `module_validator`, and `approval_handler`.
///
/// Extensions are registered as [`ExtensionKind`] variants, ensuring type
/// safety at compile time rather than relying on runtime `isinstance` checks
/// as in Python/TypeScript.
pub struct ExtensionManager {
    points: HashMap<String, ExtensionPoint>,
    extensions: HashMap<String, Vec<ExtensionKind>>,
    /// Parallel to `extensions`: the handle issued for each registration, so
    /// [`ExtensionManager::unregister_handle`] can remove exactly one without
    /// the caller having to reconstruct an [`ExtensionKind`] (D-91).
    handles: HashMap<String, Vec<ExtensionHandle>>,
    next_handle: u64,
}

/// Opaque token identifying one extension registration, returned by
/// [`ExtensionManager::register`] and consumed by
/// [`ExtensionManager::unregister_handle`].
///
/// The cross-language contract removes by IDENTITY: apcore-python and
/// apcore-typescript take the extension object back and compare with `is` /
/// `===`. Rust's manager owns its registrations, so this handle is the identity
/// a caller can hold independently of the object. Mirrors
/// [`MiddlewareHandle`](crate::middleware::MiddlewareHandle), which exists for
/// the same reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExtensionHandle(u64);

impl std::fmt::Debug for ExtensionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtensionManager")
            .field("next_handle", &self.next_handle)
            .field("handles", &self.handles)
            .field("points", &self.points.keys().collect::<Vec<_>>())
            .field(
                "extensions",
                &self
                    .extensions
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.len()))
                    .collect::<HashMap<_, _>>(),
            )
            .finish()
    }
}

impl Default for ExtensionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ExtensionManager {
    /// Create a new extension manager with the built-in extension points.
    #[must_use]
    pub fn new() -> Self {
        let points = built_in_points();
        let extensions: HashMap<String, Vec<ExtensionKind>> =
            points.keys().map(|k| (k.clone(), Vec::new())).collect();
        let handles: HashMap<String, Vec<ExtensionHandle>> =
            points.keys().map(|k| (k.clone(), Vec::new())).collect();
        Self {
            points,
            extensions,
            handles,
            next_handle: 0,
        }
    }

    /// Register an extension for the given extension point.
    ///
    /// The `extension` must be an [`ExtensionKind`] variant whose internal
    /// point name matches `point_name`.
    ///
    /// Returns the [`ExtensionHandle`] for this registration. Keep it to remove
    /// exactly this extension later via [`Self::unregister_handle`] (D-91);
    /// discarding it with `?;` or `.unwrap();` stays valid.
    ///
    /// # Errors
    ///
    /// Returns [`ModuleError`] if `point_name` is unknown or if the
    /// `ExtensionKind` variant does not match the requested point.
    pub fn register(
        &mut self,
        point_name: &str,
        extension: ExtensionKind,
    ) -> Result<ExtensionHandle, ModuleError> {
        if !self.points.contains_key(point_name) {
            let mut available: Vec<&str> = self
                .points
                .keys()
                .map(std::string::String::as_str)
                .collect();
            available.sort_unstable();
            return Err(ModuleError::new(
                ErrorCode::GeneralInvalidInput,
                format!(
                    "Unknown extension point: '{}'. Available: {}",
                    point_name,
                    available.join(", ")
                ),
            ));
        }

        // Verify the ExtensionKind variant matches the requested point.
        if extension.point_name() != point_name {
            return Err(ModuleError::new(
                ErrorCode::GeneralInvalidInput,
                format!(
                    "Extension for '{}' must be an ExtensionKind::{} variant, got {:?}",
                    point_name,
                    Self::variant_name_for_point(point_name),
                    extension.point_name()
                ),
            ));
        }

        let handle = ExtensionHandle(self.next_handle);
        self.next_handle += 1;

        let point = &self.points[point_name];
        if point.multiple {
            // INVARIANT: new() pre-populates extensions[point_name] for every point in self.points;
            // the register() guard above ensures point_name is in self.points before reaching here.
            self.extensions.get_mut(point_name).unwrap().push(extension);
            self.handles
                .entry(point_name.to_string())
                .or_default()
                .push(handle);
        } else {
            self.extensions
                .insert(point_name.to_string(), vec![extension]);
            self.handles.insert(point_name.to_string(), vec![handle]);
        }

        Ok(handle)
    }
    /// The extensions at `point_name`, or `GeneralInvalidInput` if unknown.
    ///
    /// One place decides what "unknown" means, so `get`, `get_all` and
    /// `unregister` cannot drift apart on it (D-108). The message names the
    /// registered points, because the failure this exists to catch is a TYPO
    /// and the fix is visible in the list.
    fn require_point(&self, point_name: &str) -> Result<&Vec<ExtensionKind>, ModuleError> {
        self.extensions.get(point_name).ok_or_else(|| {
            let mut available: Vec<&str> = self.points.keys().map(String::as_str).collect();
            available.sort_unstable();
            ModuleError::new(
                ErrorCode::GeneralInvalidInput,
                format!(
                    "Unknown extension point: '{point_name}'. Available: {}",
                    available.join(", ")
                ),
            )
        })
    }

    /// Return the first extension registered at `point_name`, or `None`.
    ///
    /// The reader half of the spec's Contract block
    /// (`docs/features/extension-system.md` "## Contract: ExtensionManager.get"):
    /// this manager could register and count extensions but never read one
    /// back. Returns `None` for an unknown point rather than erroring, matching
    /// the contract's "No errors raised".
    ///
    /// For a single-cardinality point (`acl`, `module_validator`,
    /// `approval_handler`, `discoverer`) this is *the* registered extension.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::GeneralInvalidInput`] when `point_name` names no registered
    /// extension point (D-108). An UNKNOWN point is an error; an EMPTY one is
    /// not — this used to return a silent empty answer for both, so a typo
    /// became a wiring bug that first surfaced at `apply()`, far from the
    /// `get("middlewares")` that caused it, with nothing naming the mistake.
    /// Unchecked lookup of a BUILT-IN point, for [`Self::apply`] only.
    ///
    /// [`Self::get`] reports an unknown point name as an error (D-108). The
    /// names used by `apply` are compile-time constants that [`Self::new`]
    /// always registers, so there is no unknown-point case to report and no
    /// `Result` to thread through the wiring code.
    fn first_builtin(&self, point_name: &str) -> Option<&ExtensionKind> {
        self.extensions
            .get(point_name)
            .and_then(|exts| exts.first())
    }

    /// Unchecked bulk lookup of a BUILT-IN point. See [`Self::first_builtin`].
    fn all_builtin(&self, point_name: &str) -> &[ExtensionKind] {
        self.extensions.get(point_name).map_or(&[], Vec::as_slice)
    }

    pub fn get(&self, point_name: &str) -> Result<Option<&ExtensionKind>, ModuleError> {
        let exts = self.require_point(point_name)?;
        Ok(exts.first())
    }

    /// Return every extension registered at `point_name`, in registration
    /// order.
    ///
    /// Empty for a point with nothing registered; an ERROR for an unknown one
    /// (D-108). The Contract block's "no errors raised" row was written about
    /// the empty case, and this SDK read it as covering the unknown case too.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::GeneralInvalidInput`] when `point_name` names no registered
    /// extension point (D-108). An UNKNOWN point is an error; an EMPTY one is
    /// not — this used to return a silent empty answer for both, so a typo
    /// became a wiring bug that first surfaced at `apply()`, far from the
    /// `get("middlewares")` that caused it, with nothing naming the mistake.
    pub fn get_all(&self, point_name: &str) -> Result<&[ExtensionKind], ModuleError> {
        Ok(self.require_point(point_name)?.as_slice())
    }

    /// Remove one specific extension from `point_name`.
    ///
    /// Identity comparison, per the spec's Contract block: the extension whose
    /// underlying object IS `extension` is removed, not one that merely looks
    /// like it. Returns `true` when a match was found and removed, `false` for
    /// an unknown point or no match — the contract makes a miss a silent no-op.
    ///
    /// This is NOT [`Self::clear`], which drops every extension at the point.
    ///
    /// Rust caveat (D-91): the manager OWNS each registered extension, so the
    /// reference passed here cannot be one borrowed from this same manager (the
    /// borrow checker refuses the immutable borrow across the `&mut self`
    /// call). Since [`ExtensionKind`] holds `Arc`s, the way to express a
    /// positive removal from outside is to keep the `Arc`, register a clone of
    /// it, and pass another clone here:
    ///
    /// ```ignore
    /// let mw: Arc<dyn Middleware> = Arc::new(MyMiddleware);
    /// mgr.register("middleware", ExtensionKind::Middleware(Arc::clone(&mw)))?;
    /// assert!(mgr.unregister("middleware", &ExtensionKind::Middleware(mw)));
    /// ```
    ///
    /// [`Self::unregister_handle`] is the same removal keyed on the token
    /// `register` hands back, for callers that would rather not keep the
    /// object. [`Self::clear`] drops everything at the point.
    pub fn unregister(
        &mut self,
        point_name: &str,
        extension: &ExtensionKind,
    ) -> Result<bool, ModuleError> {
        // D-108: an UNKNOWN point is an error; an extension that is simply not
        // there is a silent `false`. Checked before the mutable borrow so the
        // error path and `get`/`get_all` agree on what "unknown" means.
        if !self.extensions.contains_key(point_name) {
            self.require_point(point_name)?;
        }
        let target = extension.object_address();
        let Some(exts) = self.extensions.get_mut(point_name) else {
            return Ok(false);
        };
        let Some(index) = exts.iter().position(|e| e.object_address() == target) else {
            return Ok(false);
        };
        exts.remove(index);
        if let Some(handles) = self.handles.get_mut(point_name) {
            if index < handles.len() {
                handles.remove(index);
            }
        }
        Ok(true)
    }

    /// Remove exactly the extension [`Self::register`] returned `handle` for.
    ///
    /// Returns `false` if it is no longer registered — a miss is a silent
    /// no-op, as with [`Self::unregister`].
    ///
    /// This is the removal path D-91 requires: the manager owns its
    /// extensions, so the token is what a host can hold independently of the
    /// object it registered.
    pub fn unregister_handle(&mut self, handle: ExtensionHandle) -> bool {
        for (point_name, handles) in &mut self.handles {
            let Some(index) = handles.iter().position(|h| *h == handle) else {
                continue;
            };
            handles.remove(index);
            if let Some(exts) = self.extensions.get_mut(point_name) {
                if index < exts.len() {
                    exts.remove(index);
                }
            }
            return true;
        }
        false
    }

    /// Return the count of extensions registered at the given point, or
    /// `None` if the point is unknown.
    pub fn count(&self, point_name: &str) -> Option<usize> {
        self.extensions.get(point_name).map(std::vec::Vec::len)
    }

    /// Return whether the given extension point has any registered extensions.
    ///
    /// # Errors
    ///
    /// Returns [`ModuleError`] if `point_name` is unknown.
    pub fn has(&self, point_name: &str) -> Result<bool, ModuleError> {
        match self.extensions.get(point_name) {
            Some(exts) => Ok(!exts.is_empty()),
            None => Err(ModuleError::new(
                ErrorCode::GeneralInvalidInput,
                format!("Unknown extension point: '{point_name}'"),
            )),
        }
    }

    /// Return all registered extension points.
    #[must_use]
    pub fn list_points(&self) -> Vec<ExtensionPoint> {
        self.points.values().cloned().collect()
    }

    /// Clear all extensions for the given point.
    ///
    /// # Errors
    ///
    /// Returns [`ModuleError`] if `point_name` is unknown.
    pub fn clear(&mut self, point_name: &str) -> Result<(), ModuleError> {
        match self.extensions.get_mut(point_name) {
            Some(exts) => {
                exts.clear();
                if let Some(handles) = self.handles.get_mut(point_name) {
                    handles.clear();
                }
                Ok(())
            }
            None => Err(ModuleError::new(
                ErrorCode::GeneralInvalidInput,
                format!("Unknown extension point: '{point_name}'"),
            )),
        }
    }

    /// Clear all extensions across all points.
    pub fn clear_all(&mut self) {
        for exts in self.extensions.values_mut() {
            exts.clear();
        }
        for handles in self.handles.values_mut() {
            handles.clear();
        }
    }

    /// Wire all registered extensions into the given registry and executor.
    ///
    /// Connections:
    /// - discoverer -> `registry.set_discoverer()`
    /// - `module_validator` -> `registry.set_validator()`
    /// - acl -> `executor.set_acl()`
    /// - `approval_handler` -> `executor.set_approval_handler()`
    /// - middleware -> `executor.use_middleware()` for each
    /// - `span_exporter` -> sets the exporter on the EXISTING
    ///   `TracingMiddleware` in the executor's middleware chain (via
    ///   `TracingMiddleware::set_exporter`); logs a warning and applies nothing
    ///   if no `TracingMiddleware` is present. A new `TracingMiddleware` is
    ///   never appended here.
    ///
    /// # Postconditions (D-78)
    ///
    /// The store is INTACT afterwards: every registration is still readable
    /// through [`Self::get`] / [`Self::get_all`] and still counted by
    /// [`Self::count`]. Applying the same manager to a second
    /// registry/executor pair therefore wires the same set again, and applying
    /// it twice to the same executor stacks the middleware — which is what the
    /// contract's `idempotent: false` row has always promised. Each extension
    /// is wired as a shared `Arc` clone, so the manager and the runtime hold
    /// the same object rather than copies of it.
    pub fn apply(
        &mut self,
        registry: &Registry,
        executor: &mut Executor,
    ) -> Result<(), ModuleError> {
        // Discoverer
        if let Some(ExtensionKind::Discoverer(d)) = self.first_builtin("discoverer") {
            registry.set_discoverer_shared(Arc::clone(d));
        }

        // Module validator
        if let Some(ExtensionKind::ModuleValidator(v)) = self.first_builtin("module_validator") {
            registry.set_validator_shared(Arc::clone(v));
        }

        // ACL
        if let Some(ExtensionKind::Acl(acl)) = self.first_builtin("acl") {
            executor.set_acl_shared(Arc::clone(acl));
        }

        // Approval handler
        if let Some(ExtensionKind::ApprovalHandler(h)) = self.first_builtin("approval_handler") {
            executor.set_approval_handler_shared(Arc::clone(h));
        }

        // Middleware — wire every entry, keeping them registered.
        let middlewares: Vec<Arc<dyn Middleware>> = self
            .all_builtin("middleware")
            .iter()
            .filter_map(|ext| match ext {
                ExtensionKind::Middleware(mw) => Some(Arc::clone(mw)),
                _ => None,
            })
            .collect();
        for mw in middlewares {
            executor.use_middleware_shared(mw)?;
        }

        // Span exporters: locate the EXISTING TracingMiddleware in the
        // executor's middleware chain and set its exporter. We do NOT append a
        // new TracingMiddleware — if none exists, warn and skip. Mirrors
        // apcore-python `_find_tracing_middleware` + `set_exporter` else warn
        // (extensions.py:226) and apcore-typescript (extensions.ts:261). Sync
        // finding A-D-18.
        let exporters: Vec<Arc<dyn SpanExporter>> = self
            .all_builtin("span_exporter")
            .iter()
            .filter_map(|ext| {
                if let ExtensionKind::SpanExporter(e) = ext {
                    Some(Arc::clone(e))
                } else {
                    None
                }
            })
            .collect();

        if !exporters.is_empty() {
            // Sync EXT-003: when N >= 2 exporters are registered, wrap them in
            // a CompositeExporter so each span fans out to every exporter with
            // per-exporter error isolation. Mirrors apcore-python's
            // `_CompositeExporter` (extensions.py:27-38).
            let combined: Arc<dyn SpanExporter> = if exporters.len() == 1 {
                exporters.into_iter().next().unwrap()
            } else {
                Arc::new(crate::observability::CompositeExporter::from_shared(
                    exporters,
                ))
            };

            let tracing_mw = executor.find_middleware("tracing").and_then(|mw| {
                // SAFETY of downcast: only TracingMiddleware overrides as_any to
                // return Some(self); other middleware named "tracing" would not.
                mw.as_any()
                    .and_then(|any| any.downcast_ref::<TracingMiddleware>())
                    .map(|tm| {
                        tm.set_exporter_shared(combined);
                    })
            });
            if tracing_mw.is_none() {
                tracing::warn!(
                    "span_exporter extensions registered but no TracingMiddleware \
                     found in the executor middleware chain; exporter not applied"
                );
            }
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Map a point name to the expected `ExtensionKind` variant name.
    fn variant_name_for_point(point_name: &str) -> &'static str {
        match point_name {
            "discoverer" => "Discoverer",
            "middleware" => "Middleware",
            "acl" => "Acl",
            "span_exporter" => "SpanExporter",
            "module_validator" => "ModuleValidator",
            "approval_handler" => "ApprovalHandler",
            _ => "Unknown",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_has_built_in_points() {
        let mgr = ExtensionManager::new();
        let points = mgr.list_points();
        assert_eq!(points.len(), 6);

        let names: Vec<String> = points.iter().map(|p| p.name.clone()).collect();
        assert!(names.contains(&"discoverer".to_string()));
        assert!(names.contains(&"middleware".to_string()));
        assert!(names.contains(&"acl".to_string()));
        assert!(names.contains(&"span_exporter".to_string()));
        assert!(names.contains(&"module_validator".to_string()));
        assert!(names.contains(&"approval_handler".to_string()));
    }

    #[test]
    fn test_register_unknown_point_errors() {
        let mut mgr = ExtensionManager::new();
        let result = mgr.register(
            "nonexistent",
            ExtensionKind::Acl(Arc::new(ACL::new(vec![], "deny", None))),
        );
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Unknown extension point"));
    }

    #[test]
    fn test_register_mismatched_kind_errors() {
        let mut mgr = ExtensionManager::new();
        // Try to register an ACL at the "middleware" point.
        let result = mgr.register(
            "middleware",
            ExtensionKind::Acl(Arc::new(ACL::new(vec![], "deny", None))),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_register_acl_replaces_previous() {
        let mut mgr = ExtensionManager::new();
        let acl1 = Arc::new(ACL::new(vec![], "deny", None));
        let acl2 = Arc::new(ACL::new(vec![], "deny", None));
        mgr.register("acl", ExtensionKind::Acl(acl1)).unwrap();
        assert_eq!(mgr.count("acl"), Some(1));
        mgr.register("acl", ExtensionKind::Acl(acl2)).unwrap();
        // Non-multiple: replaces previous.
        assert_eq!(mgr.count("acl"), Some(1));
    }

    #[test]
    fn test_has_and_clear() {
        let mut mgr = ExtensionManager::new();
        assert!(!mgr.has("acl").unwrap());
        mgr.register(
            "acl",
            ExtensionKind::Acl(Arc::new(ACL::new(vec![], "deny", None))),
        )
        .unwrap();
        assert!(mgr.has("acl").unwrap());
        mgr.clear("acl").unwrap();
        assert!(!mgr.has("acl").unwrap());
    }

    #[test]
    fn test_clear_all() {
        let mut mgr = ExtensionManager::new();
        mgr.register(
            "acl",
            ExtensionKind::Acl(Arc::new(ACL::new(vec![], "deny", None))),
        )
        .unwrap();
        mgr.clear_all();
        assert!(!mgr.has("acl").unwrap());
    }

    // ── get / get_all / unregister (spec Contract blocks) ────────────

    #[derive(Debug)]
    struct TestMiddleware(&'static str);

    #[async_trait::async_trait]
    impl crate::middleware::base::Middleware for TestMiddleware {
        fn name(&self) -> &str {
            self.0
        }
        async fn before(
            &self,
            _module_id: &str,
            _inputs: serde_json::Value,
            _ctx: &crate::context::Context<serde_json::Value>,
        ) -> Result<Option<serde_json::Value>, ModuleError> {
            Ok(None)
        }
        async fn after(
            &self,
            _module_id: &str,
            _inputs: serde_json::Value,
            _output: serde_json::Value,
            _ctx: &crate::context::Context<serde_json::Value>,
        ) -> Result<Option<serde_json::Value>, ModuleError> {
            Ok(None)
        }
        async fn on_error(
            &self,
            _module_id: &str,
            _inputs: serde_json::Value,
            _error: &ModuleError,
            _ctx: &crate::context::Context<serde_json::Value>,
        ) -> Result<Option<serde_json::Value>, ModuleError> {
            Ok(None)
        }
    }

    #[test]
    fn test_get_returns_the_registered_extension() {
        // The manager could count and clear extensions but never read one back.
        let mut mgr = ExtensionManager::new();
        assert!(mgr.get("acl").unwrap().is_none());

        mgr.register(
            "acl",
            ExtensionKind::Acl(Arc::new(ACL::new(vec![], "deny", None))),
        )
        .unwrap();
        assert!(matches!(
            mgr.get("acl").unwrap(),
            Some(ExtensionKind::Acl(_))
        ));

        // D-108: a point that EXISTS but holds nothing answers `None`; a point
        // that was never registered is an error, not an empty answer.
        let err = mgr.get("nonexistent").unwrap_err();
        assert_eq!(err.code, ErrorCode::GeneralInvalidInput);
    }

    #[test]
    fn test_get_all_returns_registration_order() {
        let mut mgr = ExtensionManager::new();
        assert!(mgr.get_all("middleware").unwrap().is_empty());

        for name in ["first", "second"] {
            mgr.register(
                "middleware",
                ExtensionKind::Middleware(Arc::new(TestMiddleware(name))),
            )
            .unwrap();
        }

        let all = mgr.get_all("middleware").unwrap();
        assert_eq!(all.len(), 2);
        let names: Vec<&str> = all
            .iter()
            .map(|e| match e {
                ExtensionKind::Middleware(m) => m.name(),
                _ => panic!("expected middleware"),
            })
            .collect();
        assert_eq!(names, vec!["first", "second"]);

        // D-108: unknown point, not an empty point.
        let err = mgr.get_all("nonexistent").unwrap_err();
        assert_eq!(err.code, ErrorCode::GeneralInvalidInput);
    }

    #[test]
    fn test_object_address_identifies_the_extension_across_moves() {
        // The identity `unregister` compares on is the extension object's own
        // address, not the enum's, so it survives the enum being moved into the
        // manager's storage.
        let extension = ExtensionKind::Middleware(Arc::new(TestMiddleware("mw")));
        let address = extension.object_address();

        let moved = extension;
        assert_eq!(moved.object_address(), address);
        let stored = [moved];
        assert_eq!(stored[0].object_address(), address);

        // A separately-constructed, equal-looking extension is NOT the same one.
        let lookalike = ExtensionKind::Middleware(Arc::new(TestMiddleware("mw")));
        assert_ne!(lookalike.object_address(), address);
    }

    #[test]
    fn test_unregister_removes_only_the_matching_extension() {
        let mut mgr = ExtensionManager::new();
        mgr.register(
            "middleware",
            ExtensionKind::Middleware(Arc::new(TestMiddleware("keep"))),
        )
        .unwrap();

        // A different object is not removed, and the miss is a silent `false`
        // rather than an error (spec Contract: "No error if the extension is
        // not found").
        let other = ExtensionKind::Middleware(Arc::new(TestMiddleware("keep")));
        assert!(!mgr.unregister("middleware", &other).unwrap());
        assert_eq!(mgr.count("middleware"), Some(1));

        // D-108: the silent `false` is for an extension the point does not
        // hold. An unregistered POINT is a different question and is an error.
        let err = mgr.unregister("nonexistent", &other).unwrap_err();
        assert_eq!(err.code, ErrorCode::GeneralInvalidInput);

        // `unregister` is identity-scoped; `clear` is the point-wide removal.
        mgr.clear("middleware").unwrap();
        assert_eq!(mgr.count("middleware"), Some(0));
    }

    #[test]
    fn test_unregister_handle_removes_exactly_one_registration() {
        // D-91: the manager owns its extensions, so the handle `register`
        // returns is the identity a host can hold independently. A positive
        // removal from outside the manager must be expressible.
        let mut mgr = ExtensionManager::new();
        let keep = mgr
            .register(
                "middleware",
                ExtensionKind::Middleware(Arc::new(TestMiddleware("keep"))),
            )
            .unwrap();
        let drop_me = mgr
            .register(
                "middleware",
                ExtensionKind::Middleware(Arc::new(TestMiddleware("drop"))),
            )
            .unwrap();
        assert_eq!(mgr.count("middleware"), Some(2));

        assert!(mgr.unregister_handle(drop_me));
        assert_eq!(mgr.count("middleware"), Some(1));
        let remaining: Vec<&str> = mgr
            .get_all("middleware")
            .unwrap()
            .iter()
            .map(|e| match e {
                ExtensionKind::Middleware(m) => m.name(),
                _ => panic!("expected middleware"),
            })
            .collect();
        assert_eq!(remaining, vec!["keep"]);

        // A second removal of the same handle is a silent `false`.
        assert!(!mgr.unregister_handle(drop_me));

        assert!(mgr.unregister_handle(keep));
        assert_eq!(mgr.count("middleware"), Some(0));
    }

    #[test]
    fn test_unregister_by_identity_is_expressible_from_outside() {
        // D-91, identity form: `ExtensionKind` holds `Arc`s, so a caller can
        // keep the handle, register a clone, and hand another clone back.
        let mut mgr = ExtensionManager::new();
        let mw: Arc<dyn crate::middleware::base::Middleware> = Arc::new(TestMiddleware("mine"));
        mgr.register("middleware", ExtensionKind::Middleware(Arc::clone(&mw)))
            .unwrap();
        assert_eq!(mgr.count("middleware"), Some(1));

        assert!(mgr
            .unregister("middleware", &ExtensionKind::Middleware(mw))
            .unwrap());
        assert_eq!(mgr.count("middleware"), Some(0));
    }

    #[test]
    fn test_default_impl() {
        let mgr = ExtensionManager::default();
        assert_eq!(mgr.list_points().len(), 6);
    }

    #[test]
    fn test_debug_impl() {
        let mgr = ExtensionManager::new();
        let debug_str = format!("{mgr:?}");
        assert!(debug_str.contains("ExtensionManager"));
    }
}
