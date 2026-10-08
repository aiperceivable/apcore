---
description: "Canonicalize external ASCII names into module-ID segments with structured diagnostics, or convert language-local module IDs without repairing invalid identifiers."
---

# ID Normalization

## Overview

The SDKs expose two independent utilities. Bare-name canonicalization converts an external tool, operation, or command name into one canonical segment. Module-ID normalization converts an already valid language-local module ID into a dotted canonical ID without repairing punctuation.

For example, `canonicalize_name("cat-file")` returns `cat_file`, while Algorithm A02 rejects `cat-file`. A02 keeps namespace boundaries; bare-name canonicalization treats dots and colons as separators inside one name.

The normative contract is [Protocol Specification §2.2.1](../spec/protocol-spec.md#221-bare-name-canonicalization). Module-ID conversion is defined by [Algorithm A02](../spec/algorithms.md#a02-normalize_to_canonical_id-cross-language-id-normalization).

## Contract: canonicalize_name

The Python and Rust function is `canonicalize_name`; the TypeScript function is `canonicalizeName`. Each accepts one string and returns `CanonicalNameResult`, without raising an exception or panicking for string input.

| Result field (Python / Rust) | TypeScript field | Meaning |
|---|---|---|
| `original_name` | `originalName` | The exact input, retained on success and failure |
| `canonical_name` | `canonicalName` | The canonical segment on success; null on failure |
| `error` | `error` | Null on success; a `CanonicalNameError` diagnostic on failure |

Python uses `None` and Rust uses `Option` for nullable fields. Rust serializes all three fields, including null values, and supports deserialization and JSON Schema generation. TypeScript exports the function and result/error types from both the package root and browser entry point.

### Conversion rules

The conversion order is deterministic:

1. Reject non-ASCII input before trimming or case conversion. Unicode is neither removed nor transliterated.
2. Trim ASCII edge characters outside `[A-Za-z0-9_]`. Existing underscores are not trimmed.
3. Apply A02's ASCII camelCase/PascalCase/acronym boundaries and lowercase conversion.
4. Replace each maximal internal run outside `[A-Za-z0-9_]` with one underscore. Existing underscore runs remain unchanged.
5. Reject an empty candidate, then a candidate not starting with `[a-z]`, then a candidate longer than 192 ASCII characters.

No prefix is invented and no name is truncated. A successful segment matches `[a-z][a-z0-9_]*`. The operation is pure and idempotent on successful canonical names.

### Diagnostics

| Diagnostic value | Condition |
|---|---|
| `non_ascii` | Any original input character is non-ASCII |
| `empty_name` | Nothing remains after trimming ASCII edge separators |
| `invalid_start` | The candidate starts with a digit or underscore |
| `name_too_long` | The otherwise valid candidate exceeds 192 characters |

Diagnostic precedence follows the conversion order. For example, `!!!Ü!!!` yields `non_ascii`, not `empty_name`; an overlong name starting with a digit yields `invalid_start`, not `name_too_long`.

### Registration boundary

This utility produces a segment, not a registered module ID. `system` is a valid bare segment; namespace reservations, full-ID length validation, and collision detection belong to registration or scanning. Distinct inputs such as `cat-file` and `cat.file` can produce the same segment. Retain `original_name` and handle such collisions before registration.

## Usage

=== "Python"

    ```python
    from apcore import CanonicalNameError, canonicalize_name, normalize_to_canonical_id

    result = canonicalize_name("cat-file")
    assert result.original_name == "cat-file"
    assert result.canonical_name == "cat_file"
    assert result.error is None

    rejected = canonicalize_name("7z")
    assert rejected.original_name == "7z"
    assert rejected.canonical_name is None
    assert rejected.error is CanonicalNameError.INVALID_START

    assert normalize_to_canonical_id("api.GetUser", "python") == "api.get_user"
    ```

=== "TypeScript"

    ```typescript
    import { strict as assert } from 'node:assert';
    import { canonicalizeName, normalizeToCanonicalId } from 'apcore-js';

    const result = canonicalizeName('cat-file');
    assert.equal(result.originalName, 'cat-file');
    assert.equal(result.canonicalName, 'cat_file');
    assert.equal(result.error, null);

    const rejected = canonicalizeName('7z');
    assert.equal(rejected.originalName, '7z');
    assert.equal(rejected.canonicalName, null);
    assert.equal(rejected.error, 'invalid_start');

    assert.equal(normalizeToCanonicalId('api.GetUser', 'typescript'), 'api.get_user');
    ```

=== "Rust"

    ```rust
    use apcore::{canonicalize_name, normalize_to_canonical_id, CanonicalNameError};

    fn main() {
        let result = canonicalize_name("cat-file");
        assert_eq!(result.original_name, "cat-file");
        assert_eq!(result.canonical_name.as_deref(), Some("cat_file"));
        assert_eq!(result.error, None);

        let rejected = canonicalize_name("7z");
        assert_eq!(rejected.original_name, "7z");
        assert_eq!(rejected.canonical_name, None);
        assert_eq!(rejected.error, Some(CanonicalNameError::InvalidStart));

        assert_eq!(
            normalize_to_canonical_id("api::GetUser", "rust").expect("valid module ID"),
            "api.get_user"
        );
    }
    ```

## Testing Strategy

All three SDKs drive the 32 cases in `conformance/fixtures/canonicalize_name.json` through their public package-root API. The fixture covers separator repair, existing underscores, acronym boundaries, Unicode rejection, diagnostic precedence, and length growth during case conversion. SDK-specific tests also verify public exports, result serialization, idempotence, and isolation from A02's non-repairing module-ID normalization.
