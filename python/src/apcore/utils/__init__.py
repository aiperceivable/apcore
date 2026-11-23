"""Utility functions for apcore."""

from apcore.utils.call_chain import guard_call_chain
from apcore.utils.error_propagation import propagate_error
from apcore.utils.normalize import CanonicalNameError, CanonicalNameResult, canonicalize_name, normalize_to_canonical_id
from apcore.utils.pattern import match_pattern, calculate_specificity

__all__ = [
    "guard_call_chain",
    "match_pattern",
    "CanonicalNameError",
    "CanonicalNameResult",
    "canonicalize_name",
    "normalize_to_canonical_id",
    "propagate_error",
    "calculate_specificity",
]
