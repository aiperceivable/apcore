// APCore Protocol — Registry module
// Spec reference: Module registry, discovery, and validation

#[allow(clippy::module_inception)]
pub mod registry;

pub mod conflicts;
pub mod default_discoverer;
pub mod dependencies;
pub mod entry_point;
pub mod metadata;
pub mod multi_class;
pub mod scanner;
pub mod types;
pub mod validation;
pub mod version;

pub use conflicts::{detect_id_conflicts, ConflictResult, ConflictSeverity, ConflictType};
pub use default_discoverer::{DefaultDiscoverer, ModuleFactory};
pub use dependencies::resolve_dependencies;
pub use entry_point::{infer_struct_name, resolve_entry_point_name, snake_to_pascal};
pub use metadata::{load_id_map, load_metadata, merge_module_metadata, parse_dependencies};
pub use multi_class::{
    class_name_to_segment, compute_base_id, derive_module_ids, DiscoveredClass, DiscoveryConfig,
    MultiClassEntry, MAX_MODULE_ID_LEN,
};
pub use registry::{
    module_id_pattern, registry_events, DependencyInfo, DiscoveredModule, Discoverer,
    ModuleDescriptor, ModuleValidator, Registry, RegistryEvents, DEFAULT_MODULE_VERSION,
    MAX_MODULE_ID_LENGTH, MODULE_ID_PATTERN, REGISTRY_EVENTS, RESERVED_WORDS,
};
pub use scanner::{scan_extensions, scan_multi_root};
pub use types::{DepInfo, DiscoveredFile};
pub use validation::validate_descriptor;
pub use version::{
    matches_version_hint, parse_semver, select_best_version, try_matches_version_hint,
    try_select_best_version, VersionedStore,
};
