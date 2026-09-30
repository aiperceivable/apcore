---
description: "ErrorHistory keeps a bounded, deduplicated record of recent ModuleErrors per module; entries are keyed by a cross-language error fingerprint."
---

# Error History

`ErrorHistory` remembers the recent errors of every module — deduplicated, bounded, and queryable per module. It is what the `system.health.summary` and `system.health.module` modules report as recent errors. `ErrorHistoryMiddleware` feeds it from the pipeline's `on_error` hook. When [system modules](./system-modules.md) are enabled, both are created and wired for you.

Related pages: [Observability](./observability.md) · [Metrics and Usage](./metrics-and-usage.md#pluggable-storage-backends) (storage backends) · [Error System](./error-system.md) (`ModuleError`).

## How it works

- **Only `ModuleError`s are recorded.** `ErrorHistoryMiddleware` passes each `ModuleError` raised by a call to `record(module_id, error)`; other exceptions are ignored. The middleware never recovers from an error.
- **Deduplication by fingerprint.** Each error is keyed by its [fingerprint](#error-fingerprinting). Recording an error whose fingerprint is already present increments that entry's `count` and moves its `last_occurred` forward instead of adding an entry.
- **Two bounds.** Each module keeps at most `max_entries_per_module` distinct entries (default 50); past that, the module's oldest entry is dropped. Across all modules at most `max_total_entries` (default 1000) are kept; past that, the entry with the oldest `last_occurred` is evicted, whichever module it belongs to.
- **Timestamps** are UTC with millisecond precision and a `Z` suffix — `2026-09-16T10:30:00.123Z` (D-120).
- **Persistence.** Each recorded entry is also saved to the collector's [`StorageBackend`](./metrics-and-usage.md#pluggable-storage-backends) under the namespace `error_history`, keyed by fingerprint (D-113).
- **Thread safety.** All operations are safe to call concurrently; `record` never raises, because it runs inside error handling.

All three SDKs implement the eviction with a min-heap keyed on `last_occurred` (with lazy deletion of entries refreshed by deduplication) plus a per-module index, which makes eviction O(log N). That is an implementation choice, not part of the protocol: any structure that evicts the same entries is conformant.

## Configuration

With system modules enabled, the auto-created `ErrorHistory` takes its limits from:

```yaml
sys_modules:
  enabled: true
  error_history:
    max_entries_per_module: 50
    max_total_entries: 1000
```

When you construct `ErrorHistory` yourself, pass the limits to the constructor.

## API

| Operation | Python | TypeScript | Rust |
|---|---|---|---|
| Construct | `ErrorHistory(max_entries_per_module=50, max_total_entries=1000, store=None, storage=None)` | `new ErrorHistory({ maxEntriesPerModule, maxTotalEntries, store, storage })` | `ErrorHistory::with_limits(50, 1000)`; `with_storage_backend(per_module, total, backend)`; `with_store(store)` |
| Record | `record(module_id, error)` | `record(moduleId, error)` | `record(module_id, &error)`; `record_at(module_id, &error, when)` |
| One module | `get(module_id, limit=None)` | `get(moduleId, limit?)` | `get(module_id, limit: Option<usize>)` |
| All modules | `get_all(limit=None)` | `getAll(limit?)` | `get_all(limit: Option<usize>)` |
| Clear | — | `clear()`, `clearModule(moduleId)` | `clear(None)` / `clear(Some(module_id))` |
| Size | — | — | `count()` |

- `get` returns a module's entries newest first — Python and TypeScript order by when the entry was created, Rust by `last_occurred`. An unknown module returns an empty list.
- `get_all` returns every retained entry, newest `last_occurred` first.
- Rust's `ErrorHistory::new(max_entries_per_module)` sets the total limit to 100 × the per-module limit; use `with_limits` for the standard 50 / 1000.
- `store` is an optional `ObservabilityStore` notified of every recorded entry (see [storage backends](./metrics-and-usage.md#pluggable-storage-backends)).

### ErrorEntry

| Field | Type | Meaning |
|---|---|---|
| `module_id` | string | Module the error was recorded against |
| `code` | string | Error code, e.g. `MODULE_EXECUTE_ERROR` (Rust field `error_code`, serialized as `code`) |
| `message` | string | Error message of the first occurrence |
| `ai_guidance` | string or null | `ModuleError.ai_guidance` of the first occurrence |
| `count` | integer | Occurrences folded into this entry |
| `first_occurred` | timestamp | First occurrence |
| `last_occurred` | timestamp | Most recent occurrence |
| `timestamp` | timestamp | When the entry was created (Rust: updated on every occurrence) |
| `fingerprint` | string | 64-char lowercase hex [fingerprint](#error-fingerprinting) |
| `top_frame` | string | Python only: `file:line:function` of the raising frame, a local diagnostic that is **not** part of the fingerprint |

TypeScript uses camelCase field names (`moduleId`, `aiGuidance`, `firstOccurred`, `lastOccurred`). Rust timestamps are `DateTime<Utc>`.

## Example

=== "Python"
    ```python
    from apcore import ModuleError
    from apcore.middleware import ErrorHistoryMiddleware
    from apcore.observability import ErrorHistory

    history = ErrorHistory(max_entries_per_module=50, max_total_entries=1000)

    for order_id in (48213, 48214):
        history.record(
            "executor.orders.submit",
            ModuleError("MODULE_EXECUTE_ERROR", f"Order {order_id} failed"),
        )

    [entry] = history.get("executor.orders.submit")
    print(entry.count)  # 2 — both messages normalize to "order <id> failed"
    print(entry.last_occurred)  # e.g. 2026-09-16T10:30:00.123Z

    # In a client, record automatically from every failed call:
    middleware = ErrorHistoryMiddleware(history)
    ```

=== "TypeScript"
    ```typescript
    import { ErrorHistory, ErrorHistoryMiddleware, ModuleError } from "apcore-js";

    const history = new ErrorHistory({ maxEntriesPerModule: 50, maxTotalEntries: 1000 });

    for (const orderId of [48213, 48214]) {
      history.record(
        "executor.orders.submit",
        new ModuleError("MODULE_EXECUTE_ERROR", `Order ${orderId} failed`),
      );
    }

    const [entry] = history.get("executor.orders.submit");
    console.log(entry.count); // 2 — both messages normalize to "order <id> failed"
    console.log(entry.lastOccurred); // e.g. 2026-09-16T10:30:00.123Z

    // In a client, record automatically from every failed call:
    const middleware = new ErrorHistoryMiddleware(history);
    ```

=== "Rust"
    ```rust
    use apcore::errors::{ErrorCode, ModuleError};
    use apcore::observability::{ErrorHistory, ErrorHistoryMiddleware};

    fn main() {
        let history = ErrorHistory::with_limits(50, 1000);

        for order_id in [48213, 48214] {
            history.record(
                "executor.orders.submit",
                &ModuleError::new(ErrorCode::ModuleExecuteError, format!("Order {order_id} failed")),
            );
        }

        let entries = history.get("executor.orders.submit", None);
        assert_eq!(entries[0].count, 2); // both messages normalize to "order <id> failed"

        // In a client, record automatically from every failed call:
        let middleware = ErrorHistoryMiddleware::new(history.clone());
    }
    ```

## Error fingerprinting

The fingerprint makes the same logical error collapse into one entry — across repeated occurrences and across SDKs:

```text
fingerprint = lowercase_hex( SHA-256( error_code + ":" + module_id + ":" + normalize_message(message) ) )
```

`normalize_message` is exactly these five steps, in this order:

```text
1. UUIDs (8-4-4-4-12 hex, hyphenated)                      -> <UUID>
2. ISO 8601 dates / date-times, with optional fraction
   and Z or ±HH:MM offset                                  -> <TIMESTAMP>
3. Runs of 4 or more digits between word boundaries        -> <ID>
4. Trim leading and trailing whitespace
5. Lowercase the whole string (placeholders included)
```

The order matters: timestamps are replaced before integers, because step 3 would otherwise consume the four-digit year and step 2 would no longer match. Step 3 requires a word boundary on both sides, so digits glued to letters (`30000ms`) are left as they are.

| Input | Normalized |
|---|---|
| `Order 48213 failed at 2026-09-16T10:30:00Z (request a1b2c3d4-e5f6-7890-abcd-ef1234567890)` | `order <id> failed at <timestamp> (request <uuid>)` |
| `timed out after 30000ms` | `timed out after 30000ms` |

- Errors that differ only in UUIDs, timestamps or long numbers share a fingerprint.
- Different error codes never share one, even with identical messages; neither do different modules.
- The call site (stack frame) is deliberately not an input: frame identities differ between Python, V8 and Rust, and including one would give the same error a different fingerprint in each SDK.

Every SDK exports the two functions, so you can compute a fingerprint without recording anything. With the input above and code `MODULE_EXECUTE_ERROR` in module `executor.orders.submit`, all three return `dba777e10f3022e9eeb69b9b9943b4f76eedecd489bfa57d9bd1eaa00c5f3742`.

=== "Python"
    ```python
    from apcore.observability import compute_fingerprint, normalize_message

    message = "Order 48213 failed at 2026-09-16T10:30:00Z (request a1b2c3d4-e5f6-7890-abcd-ef1234567890)"
    print(normalize_message(message))  # order <id> failed at <timestamp> (request <uuid>)
    print(compute_fingerprint("MODULE_EXECUTE_ERROR", "executor.orders.submit", message))
    ```

=== "TypeScript"
    ```typescript
    import { computeFingerprint, normalizeMessage } from "apcore-js";

    const message =
      "Order 48213 failed at 2026-09-16T10:30:00Z (request a1b2c3d4-e5f6-7890-abcd-ef1234567890)";
    console.log(normalizeMessage(message)); // order <id> failed at <timestamp> (request <uuid>)
    console.log(computeFingerprint("MODULE_EXECUTE_ERROR", "executor.orders.submit", message));
    ```

=== "Rust"
    ```rust
    use apcore::observability::{compute_fingerprint, normalize_message};

    fn main() {
        let message = "Order 48213 failed at 2026-09-16T10:30:00Z (request a1b2c3d4-e5f6-7890-abcd-ef1234567890)";
        println!("{}", normalize_message(message)); // order <id> failed at <timestamp> (request <uuid>)
        println!("{}", compute_fingerprint("MODULE_EXECUTE_ERROR", "executor.orders.submit", message));
    }
    ```
