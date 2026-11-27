//! Caller-scoped discovery of public, active capabilities.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::{Mutex, RwLock};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::context::{Context, Identity};
use crate::errors::ModuleError;
use crate::executor::Executor;
use crate::pipeline::BuiltinGate;
use crate::policy::ExecutionPolicy;

const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 100;
const CURSOR_TTL: Duration = Duration::from_secs(60);
const MAX_CURSORS: usize = 128;

/// Advisory access requirements for a visible capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccessHint {
    /// The known gates add no approval requirement.
    Allowed,
    /// The known gates require approval.
    RequiresApproval,
    /// The final requirements depend on the invocation.
    Conditional,
}

impl AccessHint {
    pub(crate) fn from_approval_outcomes(required: bool, not_required: bool) -> Self {
        match (required, not_required) {
            (false, _) => Self::Allowed,
            (true, false) => Self::RequiresApproval,
            (true, true) => Self::Conditional,
        }
    }
}

/// Filters and pagination controls for capability summaries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct CapabilityQuery {
    pub query: Option<String>,
    pub prefix: Option<String>,
    pub tags: Vec<String>,
    pub readonly: Option<bool>,
    pub limit: usize,
    pub cursor: Option<String>,
    pub include_approval_required: bool,
}

impl Default for CapabilityQuery {
    fn default() -> Self {
        Self {
            query: None,
            prefix: None,
            tags: Vec::new(),
            readonly: None,
            limit: DEFAULT_LIMIT,
            cursor: None,
            include_approval_required: true,
        }
    }
}

impl CapabilityQuery {
    fn normalize(&mut self) -> Result<(), ModuleError> {
        if !(1..=MAX_LIMIT).contains(&self.limit) {
            return Err(ModuleError::invalid_input(
                "Capability limit must be between 1 and 100",
            ));
        }
        self.query = self
            .query
            .as_deref()
            .map(str::trim)
            .filter(|query| !query.is_empty())
            .map(str::to_lowercase);
        self.prefix = self.prefix.take().filter(|prefix| !prefix.is_empty());
        self.tags.sort_unstable();
        self.tags.dedup();
        Ok(())
    }

    fn matches_filters(&self, summary: &ModuleSummary) -> bool {
        self.prefix
            .as_ref()
            .is_none_or(|prefix| summary.module_id.starts_with(prefix))
            && self
                .readonly
                .is_none_or(|readonly| summary.readonly == readonly)
            && self.tags.iter().all(|tag| summary.tags.contains(tag))
            && (self.include_approval_required
                || summary.access_hint != AccessHint::RequiresApproval)
    }

    fn matches_query(&self, summary: &ModuleSummary) -> bool {
        self.query.as_ref().is_none_or(|query| {
            summary.module_id.to_lowercase().contains(query)
                || summary.description.to_lowercase().contains(query)
                || summary
                    .tags
                    .iter()
                    .any(|tag| tag.to_lowercase().contains(query))
        })
    }
}

/// Lightweight discovery metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModuleSummary {
    pub module_id: String,
    pub version: String,
    pub description: String,
    pub tags: Vec<String>,
    pub readonly: bool,
    pub access_hint: AccessHint,
}

/// A page of caller-scoped summaries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CapabilityPage {
    pub items: Vec<ModuleSummary>,
    pub next_cursor: Option<String>,
}

/// Detached schemas for the selected callable registration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CallableDefinition {
    pub module_id: String,
    pub version: String,
    pub input_schema: Value,
    pub output_schema: Value,
}

/// A host's optional ranking or selection of currently visible candidate IDs.
pub trait CapabilitySearchProvider: Send + Sync {
    /// Return ranked IDs. The executor enforces filters and current visibility.
    fn search(&self, query: Option<&str>, candidate_ids: &[String]) -> Vec<String>;
}

#[derive(Clone, PartialEq, Eq)]
struct CursorScope {
    caller_id: Option<String>,
    call_chain: Vec<String>,
    identity: Option<Identity>,
}

impl From<&Context<Value>> for CursorScope {
    fn from(context: &Context<Value>) -> Self {
        Self {
            caller_id: context.caller_id.clone(),
            call_chain: context.call_chain.clone(),
            identity: context.identity.clone(),
        }
    }
}

#[derive(Clone)]
struct CursorPosition {
    candidates: Arc<Vec<String>>,
    offset: usize,
    custom_search: bool,
    provider_revision: u64,
}

struct CursorEntry {
    token: Uuid,
    scope: CursorScope,
    query: CapabilityQuery,
    position: CursorPosition,
    created: Instant,
}

#[derive(Clone, Default)]
struct SearchProvider {
    implementation: Option<Arc<dyn CapabilitySearchProvider>>,
    revision: u64,
}

#[derive(Default)]
pub(crate) struct DiscoveryState {
    provider: RwLock<SearchProvider>,
    cursors: Mutex<VecDeque<CursorEntry>>,
}

impl std::fmt::Debug for DiscoveryState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DiscoveryState")
            .finish_non_exhaustive()
    }
}

impl DiscoveryState {
    fn resume(
        &self,
        token: &str,
        scope: &CursorScope,
        query: &CapabilityQuery,
    ) -> Result<CursorPosition, ModuleError> {
        let invalid = || ModuleError::invalid_input("Invalid capability cursor");
        let token = Uuid::parse_str(token).map_err(|_| invalid())?;
        let provider = self.provider.read();
        let mut cursors = self.cursors.lock();
        cursors.retain(|entry| {
            entry.created.elapsed() < CURSOR_TTL
                && entry.position.provider_revision == provider.revision
        });
        cursors
            .iter()
            .find(|entry| entry.token == token && &entry.scope == scope && &entry.query == query)
            .map(|entry| entry.position.clone())
            .ok_or_else(invalid)
    }

    fn save(&self, scope: CursorScope, query: CapabilityQuery, position: CursorPosition) -> String {
        let token = Uuid::new_v4();
        let mut cursors = self.cursors.lock();
        if cursors.len() == MAX_CURSORS {
            cursors.pop_front();
        }
        cursors.push_back(CursorEntry {
            token,
            scope,
            query,
            position,
            created: Instant::now(),
        });
        token.to_string()
    }
}

impl Executor {
    /// Install or remove a provider for capability search.
    pub fn set_capability_search_provider(
        &self,
        provider: Option<Arc<dyn CapabilitySearchProvider>>,
    ) {
        let mut registered = self.capability_discovery.provider.write();
        registered.implementation = provider;
        registered.revision = registered.revision.wrapping_add(1);
        self.capability_discovery.cursors.lock().clear();
    }

    /// Return a governed page of public capability summaries. Tokens can be
    /// used only with the same executor, caller frame, and normalized filters.
    pub fn list_callable(
        &self,
        context: Option<&Context<Value>>,
        mut query: CapabilityQuery,
    ) -> Result<CapabilityPage, ModuleError> {
        query.normalize()?;
        let cursor = query.cursor.take();
        let anonymous = Context::anonymous();
        let context = context.unwrap_or(&anonymous);
        let scope = CursorScope::from(context);
        let position = match cursor {
            Some(cursor) => self.capability_discovery.resume(&cursor, &scope, &query)?,
            None => self.capability_candidates(context, &query),
        };
        let mut items = Vec::new();
        let mut next_offset = None;
        for (index, id) in position.candidates.iter().enumerate().skip(position.offset) {
            let Some(summary) = self.discover_summary(id, context) else {
                continue;
            };
            if !query.matches_filters(&summary)
                || (!position.custom_search && !query.matches_query(&summary))
            {
                continue;
            }
            if items.len() == query.limit {
                next_offset = Some(index);
                break;
            }
            items.push(summary);
        }
        let next_cursor = next_offset.map(|offset| {
            self.capability_discovery
                .save(scope, query, CursorPosition { offset, ..position })
        });
        Ok(CapabilityPage { items, next_cursor })
    }

    fn capability_candidates(
        &self,
        context: &Context<Value>,
        query: &CapabilityQuery,
    ) -> CursorPosition {
        let ids = self.registry.list(None, query.prefix.as_deref(), None);
        let provider = self.capability_discovery.provider.read().clone();
        let custom_search = provider.implementation.is_some();
        let candidates = if let Some(provider) = provider.implementation {
            let visible = ids
                .into_iter()
                .filter(|id| {
                    self.discover_summary(id, context)
                        .is_some_and(|summary| query.matches_filters(&summary))
                })
                .collect::<Vec<_>>();
            let allowed: HashSet<&str> = visible.iter().map(String::as_str).collect();
            let mut seen = HashSet::new();
            provider
                .search(query.query.as_deref(), &visible)
                .into_iter()
                .filter(|id| allowed.contains(id.as_str()) && seen.insert(id.clone()))
                .collect()
        } else {
            ids
        };
        CursorPosition {
            candidates: Arc::new(candidates),
            custom_search,
            provider_revision: provider.revision,
            offset: 0,
        }
    }

    /// Return definitions in request order, omitting duplicates and any
    /// module that cannot be disclosed to the caller.
    pub fn get_callable_definitions<S: AsRef<str>>(
        &self,
        module_ids: &[S],
        context: Option<&Context<Value>>,
    ) -> Vec<CallableDefinition> {
        let mut seen = HashSet::new();
        module_ids
            .iter()
            .map(AsRef::as_ref)
            .filter(|id| seen.insert(*id))
            .filter_map(|id| self.get_callable_definition(id, None, context))
            .collect()
    }

    /// Resolve a live, authorized definition, optionally checking an exact
    /// expected version. Adapters can use this to reject stale exported IDs.
    #[must_use]
    pub fn get_callable_definition(
        &self,
        module_id: &str,
        expected_version: Option<&str>,
        context: Option<&Context<Value>>,
    ) -> Option<CallableDefinition> {
        self.map_callable_descriptor(
            module_id,
            expected_version,
            context,
            self.strategy(),
            |descriptor| CallableDefinition {
                module_id: module_id.into(),
                version: descriptor.version.clone(),
                input_schema: descriptor.input_schema.clone(),
                output_schema: descriptor.output_schema.clone(),
            },
        )
    }

    /// Project a detached, currently authorized descriptor for a trusted adapter.
    #[must_use]
    pub fn get_callable_descriptor(
        &self,
        module_id: &str,
        expected_version: Option<&str>,
        context: Option<&Context<Value>>,
    ) -> Option<crate::registry::ModuleDescriptor> {
        self.get_callable_descriptor_for_strategy(
            module_id,
            expected_version,
            context,
            self.strategy(),
        )
    }

    /// Project a descriptor for an adapter using an explicit execution strategy.
    #[must_use]
    pub fn get_callable_descriptor_for_strategy(
        &self,
        module_id: &str,
        expected_version: Option<&str>,
        context: Option<&Context<Value>>,
        strategy: &crate::ExecutionStrategy,
    ) -> Option<crate::registry::ModuleDescriptor> {
        self.map_callable_descriptor(
            module_id,
            expected_version,
            context,
            strategy,
            |descriptor| {
                let mut live = descriptor.clone();
                module_id.clone_into(&mut live.module_id);
                live
            },
        )
    }

    fn map_callable_descriptor<T>(
        &self,
        module_id: &str,
        expected_version: Option<&str>,
        context: Option<&Context<Value>>,
        strategy: &crate::ExecutionStrategy,
        map: impl FnOnce(&crate::registry::ModuleDescriptor) -> T,
    ) -> Option<T> {
        let anonymous = Context::anonymous();
        let context = context.unwrap_or(&anonymous);
        self.visibility_hint(module_id, context, strategy)?;
        self.registry
            .with_discoverable_descriptor(module_id, |descriptor, _| {
                if expected_version.is_some_and(|version| version != descriptor.version) {
                    return None;
                }
                Some(map(descriptor))
            })?
    }

    fn visibility_hint(
        &self,
        module_id: &str,
        parent: &Context<Value>,
        strategy: &crate::ExecutionStrategy,
    ) -> Option<AccessHint> {
        if self.pipeline_load_error.is_some() || self.acl_load_error.is_some() {
            return None;
        }
        let permits_lookup = strategy
            .steps()
            .iter()
            .find_map(|step| step.builtin_module_is_enabled(module_id));
        if permits_lookup != Some(true) {
            return None;
        }
        if !strategy
            .steps()
            .iter()
            .any(|step| step.builtin_gate() == Some(BuiltinGate::Acl))
        {
            return None;
        }
        if let Some(acl) = &self.acl {
            let prospective = parent.child(module_id);
            return acl.evaluate_visibility(
                prospective.caller_id.as_deref(),
                module_id,
                Some(&prospective),
            );
        }
        Some(AccessHint::Allowed)
    }

    fn has_discovery_gate(&self, gate: BuiltinGate) -> bool {
        self.strategy()
            .steps()
            .iter()
            .any(|step| step.builtin_gate() == Some(gate))
    }

    fn discover_summary(&self, module_id: &str, context: &Context<Value>) -> Option<ModuleSummary> {
        let acl = self.visibility_hint(module_id, context, self.strategy())?;
        let (mut summary, module, declared) =
            self.registry
                .with_discoverable_descriptor(module_id, |descriptor, module| {
                    (
                        ModuleSummary {
                            module_id: module_id.into(),
                            version: descriptor.version.clone(),
                            description: descriptor.description.clone(),
                            tags: descriptor.tags.clone(),
                            readonly: descriptor.annotations.as_ref().is_some_and(|a| a.readonly),
                            access_hint: acl,
                        },
                        Arc::clone(module),
                        descriptor.annotations.clone(),
                    )
                })?;
        let annotations = module.annotations();
        summary.readonly = declared
            .as_ref()
            .map_or(annotations.readonly, |value| value.readonly);
        summary.tags.extend(module.tags());
        summary.tags.sort_unstable();
        summary.tags.dedup();
        let annotations = crate::module::ModuleAnnotations::governance_union(
            Some(&annotations),
            declared.as_ref(),
        );
        let evaluator = self.has_discovery_gate(BuiltinGate::PostValidationPolicy);
        let policy = ExecutionPolicy::effective(self.policy.as_deref());
        let approval_enforced = self.has_discovery_gate(BuiltinGate::Approval)
            && (policy.strict || self.approval_handler.is_some() || evaluator);
        if !approval_enforced {
            summary.access_hint = AccessHint::Allowed;
        } else if policy
            .resolve(module_id, annotations.as_ref())
            .needs_approval
        {
            summary.access_hint = AccessHint::RequiresApproval;
        }
        if summary.access_hint == AccessHint::Allowed && evaluator {
            summary.access_hint = AccessHint::Conditional;
        }
        self.registry
            .with_discoverable_descriptor(module_id, |descriptor, current| {
                (Arc::ptr_eq(current, &module) && descriptor.version == summary.version)
                    .then_some(summary)
            })?
    }
}
