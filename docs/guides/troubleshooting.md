---
description: "FAQ-style troubleshooting for common apcore issues — modules not discovered, unexpected ACL_DENIED, validation failures — keyed to PROTOCOL_SPEC error handling and conformance fixtures."
---

# Troubleshooting Guide

> **Type:** User guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §8 Error Handling Specification.

A focused reference for the questions developers hit most often while building on apcore. The sections below assume you are using one of the three reference SDKs (apcore-python, apcore-typescript, apcore-rust) at version 0.20.0 or later.

For the full error class hierarchy see [features/error-system.md](../features/error-system.md). For the conformance fixture index see [spec/conformance.md §8](../spec/conformance.md#8-conformance-test-fixtures).

---

## 1. Frequently Asked Questions

### 1.1 My module file exists but `client.list_modules()` doesn't show it. Why?

Most common causes (in order of frequency):

1. **The directory is not under one of the configured `extension_dirs`.** apcore only scans paths listed in `apcore.yaml` `extensions:` (or the explicit constructor argument). Verify with `client.discover()` returning >0 and check the configured paths.
2. **The file starts with `_` or `.` or sits inside a hidden directory.** Hidden file filtering is mandatory at Level 0 ([PROTOCOL_SPEC §3.5](../spec/protocol-spec.md#3-directory-specification)).
3. **The class did not subclass `Module` (Python) / implement the module interface (TS/Rust).** Discovery only registers classes/functions matching the contract.
4. **Two modules share the same Canonical ID.** Look for `MODULE_ID_CONFLICT` in your logs — the second one is rejected.

### 1.2 Why does a call return `ACL_DENIED` even when no rules match?

`default_effect: deny` is in force. **This is correct production behavior.** Either:

- Add an explicit `allow` rule for the `(caller, target)` pair, or
- Change `default_effect: allow` only for narrow public read paths (and add explicit `deny` rules for sensitive targets — see the warning in [features/acl-system.md](../features/acl-system.md)).

Never disable ACL globally to get past a denial — that masks real authorization bugs and creates compliance risk.

### 1.3 `validate()` says `valid: true` but `call()` then fails with `SCHEMA_VALIDATION_ERROR` — how?

`validate()` runs Steps 1–5 and Step 7 of the pipeline. The actual call additionally runs the Middleware Before Chain (Step 6) and the module's `execute()` body. A mismatch most often means:

- **A middleware mutated the inputs** between `validate()` and `execute()`, producing a payload that no longer matches `input_schema`. Audit your middleware `before` hooks; ensure they return data still conforming to the schema.
- **You called `validate()` without `inputs`** (a name-only check) and then `call()` with a real payload. Always pass the same `inputs` to both.

### 1.4 Streaming module emits chunks but the merged result is wrong / fields disappear.

The Executor uses **recursive deep-merge with depth cap 32** to combine chunks ([fixture: `stream_aggregation`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/stream_aggregation.json)). Common pitfalls:

- **Arrays are replaced, not concatenated.** Each chunk's array overwrites the previous chunk's array at the same key. To accumulate items, place them under different keys per chunk or have the final chunk emit the full list.
- **Setting a key to `null` in a later chunk** does not erase it; it overwrites with `null`. Validate that downstream consumers handle that.
- **You exceeded depth 32.** Check your output schema for very deeply nested objects.

### 1.5 Why does `_approval_token` retry re-execute my logging middleware?

By design. Resuming a `pending` approval re-enters the pipeline from Step 1 with no preserved intermediate state ([PROTOCOL_SPEC §7](../spec/protocol-spec.md#7-approval-system)). Pre-approval middleware side effects re-execute. If you need at-most-once semantics across an approval gate, **inspect `_approval_token` inside your middleware** and short-circuit when present.

### 1.6 Hot reload says it succeeded but my module's behavior didn't change.

Three checks:

1. **Was the change actually on disk before reload?** Some IDEs delay autosave.
2. **Was the module loaded from a cached `.pyc` / compiled artifact?** Clear caches.
3. **Did reload return `MODULE_RELOAD_CONFLICT` (retryable)?** A call was in flight; the reload was skipped. Retry after the call completes.

### 1.7 OTel traces show the spans but `caller_id` / `target_id` are missing.

Confirm your tracing middleware is reading from `Context` (the apcore object), not from the OTel context alone. apcore propagates its trace context separately at the protocol level, then **bridges to W3C TraceContext** at the SDK boundary. See [PROTOCOL_SPEC §10.5](../spec/protocol-spec.md#10-observability-specification).

### 1.8 `x-sensitive: true` field appears in plain text in my error message.

Default redaction applies to **logs and audit events**, not to `error.message`. If you raise an error with the sensitive value in its message, that value will leak. Two fixes:

- Construct error messages **without inlining sensitive data**.
- Configure `obs.redaction.regex_patterns` with a regex matching the value — the redaction middleware will scrub it from log records.

### 1.9 My TypeScript SDK uses camelCase but my Python SDK uses snake_case in the same context — what's authoritative?

Both. Wire-format identifiers (Canonical IDs, schema property names, fixture cases) are language-agnostic and `snake_case`. Each SDK's **method names** follow the host language convention: `useBefore()` in TS, `use_before()` in Python and Rust. See [APCore Client — Language-Specific Adaptations](../features/apcore-client.md#language-specific-adaptations).

### 1.10 Where do I look first when something breaks?

1. Check the error code against the table in §2 below.
2. Run `client.validate(module_id, inputs)` to isolate validation failures from execution failures.
3. Enable the `LoggingMiddleware` with `log_inputs=True, log_outputs=True` and re-run.
4. Inspect `Context.call_chain` to see where in the call tree the error originated.
5. Compare your behavior against the relevant [conformance fixture](../spec/conformance.md#8-conformance-test-fixtures).

---

## 2. Error Code → Cause → Fix

The 20 codes you are most likely to hit. For the full list see [features/error-system.md](../features/error-system.md).

| Code | Likely cause | Fix |
|------|-------------|-----|
| `MODULE_NOT_FOUND` | Canonical ID typo, or module not under any `extension_dir` | Run `client.list_modules()` to enumerate registered IDs; check `apcore.yaml` `extensions:` |
| `MODULE_DISABLED` | Module loaded but explicitly disabled in config | Re-enable in `apcore.yaml` or via `client.enable_module(id)` |
| `MODULE_TIMEOUT` | `execute()` exceeded the configured per-module timeout | Increase `policy.timeout_ms` in apcore.yaml or shorten the work; check for blocking I/O |
| `MODULE_LOAD_ERROR` | Import-time exception (Python) / parse error (TS) / compile error (Rust) | The error's `details` field carries the original traceback — fix the underlying bug |
| `MODULE_EXECUTE_ERROR` | Unhandled exception inside `execute()` | The error's `details` field carries the original exception; treat `execute()` like any user code path |
| `MODULE_ID_CONFLICT` | Two files map to the same Canonical ID (e.g. `foo/bar.py` and `foo/bar/__init__.py`) | Rename one or restructure the directory |
| `SCHEMA_VALIDATION_ERROR` | `inputs` or `outputs` did not match `input_schema` / `output_schema` | The error's `errors` list pinpoints the failing path; tighten the schema or fix the data |
| `SCHEMA_NOT_FOUND` | `external_schema:` referenced a YAML file that doesn't exist | Verify the path is relative to the binding file's directory |
| `SCHEMA_PARSE_ERROR` | Schema YAML/JSON malformed | Validate with a JSON Schema linter; ensure Draft 2020-12 |
| `SCHEMA_CIRCULAR_REF` | `$ref` chain references back to an ancestor without a base case | Add a base case or use bounded recursion (`schema_hardening_recursive` fixture) |
| `ACL_DENIED` | No matching `allow` rule and `default_effect: deny` | Add an `allow` rule (rare: change default) — see §1.2 |
| `ACL_RULE_ERROR` | Invalid YAML in the ACL rules (missing required keys, bad pattern) | Compare against the YAML example in [features/acl-system.md](../features/acl-system.md) |
| `APPROVAL_DENIED` | Handler returned `status: rejected` | Inspect the handler's logic; there is no automatic recovery |
| `APPROVAL_TIMEOUT` | Handler returned `status: timeout` (or your handler raised) | Retry once; if persistent, audit the handler's escalation path |
| `APPROVAL_PENDING` | Phase B async approval; not really an error | Capture `approval_id`, retry the call later with `_approval_token` in `arguments` |
| `CALL_DEPTH_EXCEEDED` | Call chain exceeded `policy.max_call_depth` (default 32) | Refactor to flatten the call graph, or raise the limit if intentional |
| `CIRCULAR_CALL` | Module invoked itself transitively through the call chain | Break the cycle; consider extracting the shared logic into a helper module |
| `BINDING_SCHEMA_MISSING` | Binding has no `input_schema` and type hints can't be inferred | Add a YAML schema or annotate the function with concrete types |
| `MIDDLEWARE_CHAIN_ERROR` | A `before()` hook raised | Error's `original` carries the cause; `executed_middlewares` lists what ran — fix the offending middleware |
| `PIPELINE_DEPENDENCY_ERROR` | A custom step's `requires` field references a `provides` capability no preceding step declares | Reorder steps in `pipeline.configure[]`, or fix the step's dependency declarations |

For codes not listed here:

- The error's class name in [features/error-system.md](../features/error-system.md) usually says exactly what's wrong.
- Search the SDK source: `grep -rn 'YOUR_ERROR_CODE' src/` shows the raise sites.
- Many error codes have a matching conformance fixture under `conformance/fixtures/` demonstrating the expected behaviour.

---

## 3. Diagnostic Commands

```bash
# Enumerate registered modules with metadata
python -c "from apcore import APCore; c = APCore(); c.discover(); [print(m) for m in c.list_modules()]"

# Build the docs locally to surface broken cross-references
mkdocs build 2>&1 | grep WARNING

# Validate apcore.yaml against the schema
python -c "from apcore.config import Config; print(Config.load('apcore.yaml'))"

# Run a single conformance fixture against your SDK (example: Python)
pytest tests/conformance/test_acl_evaluation.py -v
```

```typescript
// Enumerate registered modules
import { APCore } from 'apcore-js';
const c = new APCore();
await c.discover();
console.log(await c.listModules());
```

```rust
// Enumerate registered modules
use apcore::APCore;

#[tokio::main]
async fn main() {
    let c = APCore::new();
    c.discover().await.unwrap();
    // list_modules takes optional tag-filter and prefix-filter; pass None for "all".
    for m in c.list_modules(None, None) { println!("{:?}", m); }
}
```

---

## 4. When to file an issue

Open a GitHub issue at `aiperceivable/apcore` if:

- A behavior in your SDK contradicts the linked conformance fixture and the fix is not obvious.
- A normative `MUST` from PROTOCOL_SPEC is unclear or appears self-contradictory.
- You hit an error code with no documented cause and the SDK source isn't decisive.

For SDK-specific bugs (a Python-only or TypeScript-only issue) file in the **respective SDK repo**, not in apcore.

---

## See also

- [features/error-system.md](../features/error-system.md) — full error class hierarchy and code constants
- [spec/conformance.md](../spec/conformance.md) — conformance levels and fixture catalog
- [features/acl-system.md](../features/acl-system.md) — ACL rule evaluation, including `$or`/`$not` compound operators
- [PROTOCOL_SPEC §8](../spec/protocol-spec.md#8-error-handling-specification) — normative error specification
