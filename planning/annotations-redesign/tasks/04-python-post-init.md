# Task 4: Python __post_init__

Python `__post_init__` for frozen dataclass immutability enforcement.

## Implementation
- `cache_key_fields`: list -> tuple conversion via `object.__setattr__`
- `extra`: shallow copy to detach from caller's dict
- `cache_ttl`: negative value clamped to 0 with WARN log

## Also
- `from_dict()` classmethod: known/unknown key separation, unknown -> extra
- `_CANONICAL_FIELDS` set
- `DEFAULT_ANNOTATIONS` constant

## Tests
- `cache_key_fields=["a", "b"]` -> assert isinstance tuple
- Extra dict detachment: mutating original doesn't affect instance
- Negative `cache_ttl` clamped to 0
- `from_dict()` captures unknown keys into extra
