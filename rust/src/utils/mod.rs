// APCore Protocol — Utility functions
// Spec reference: Shared helpers and guard utilities

pub mod error_propagation;
pub mod helpers;
pub mod name;

pub use error_propagation::{propagate_error, propagate_module_error};
pub use helpers::{
    calculate_specificity, guard_call_chain, guard_call_chain_for_context,
    guard_call_chain_with_repeat, match_glob, match_pattern, normalize_to_canonical_id,
    DEFAULT_MAX_CALL_DEPTH, DEFAULT_MAX_MODULE_REPEAT,
};
pub use name::{canonicalize_name, CanonicalNameError, CanonicalNameResult};
