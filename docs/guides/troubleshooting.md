---
description: "FAQ-style troubleshooting for common apcore issues — modules not discovered, unexpected ACL_DENIED, validation and streaming surprises — with an error-code table."
---

# Troubleshooting Guide

> **Type:** User guide. **Normative spec:** [PROTOCOL_SPEC §8](../spec/protocol-spec.md#8-error-handling-specification) Error Handling Specification.

The questions developers hit most often while building on apcore with the Python, TypeScript and Rust SDKs.

For the full error class hierarchy see [features/error-system.md](../features/error-system.md). For the conformance fixture index see [spec/conformance.md §8](../spec/conformance.md#8-conformance-test-fixtures).

---

## 1. Frequently Asked Questions

### 1.1 My module file exists but `client.list_modules()` doesn't show it. Why?

Check, in order:

1. **Did discovery run, and over the right directory?** Discovery scans `extensions.root` (or each entry of `extensions.roots`) from `apcore.yaml`, default `./extensions`, and only when you call `client.discover()`. It returns the number of modules registered; a root directory that does not exist raises `CONFIG_NOT_FOUND`.
2. **Is the file hidden or excluded?** Files and directories starting with `_` or `.` are skipped ([PROTOCOL_SPEC §3.5](../spec/protocol-spec.md#3-directory-specification)), as is anything matching `extensions.ignore_patterns`, and anything deeper than `extensions.max_depth` (default 8).
3. **Does the file look like a module to your SDK?**
    - **Python** — the file must define exactly one class that has `input_schema` and `output_schema` attributes that are Pydantic `BaseModel` subclasses, plus a callable `execute`. Dict schemas and decorated functions are not picked up by discovery (register those with `client.register()` / `@client.module`). No match, or more than one, is a `MODULE_LOAD_ERROR`.
    - **TypeScript** — a `.ts` or `.js` file (not `.d.ts`, `.test.*`, `.spec.*`) whose default export — or its single matching named export — is an **object instance** with `inputSchema`, `outputSchema`, a string `description` and an `execute` function. Export `new MyModule()`, not the class.
    - **Rust** — modules are compiled into your binary, so nothing is loaded from disk by default and `discover()` returns 0. Register modules with `client.register(...)` / `client.module(...)`, or attach a `DefaultDiscoverer` whose factory maps discovered files to module instances.
4. **Is the ID already taken?** A second module with the same Canonical ID is rejected with `DUPLICATE_MODULE_ID`.

### 1.2 Why does a call return `ACL_DENIED` even when no rules match?

Because no rule matched: the ACL file's `default_effect: deny` then decides, which is the intended production behaviour. Add an explicit `allow` rule for that caller/target pair — see [ACL Configuration Guide § Troubleshooting](./acl-configuration.md#10-troubleshooting). Do not switch the default to `allow` to get past a denial; that turns every missing rule into a silent grant.

### 1.3 `validate()` says `valid: true` but `call()` then fails with `SCHEMA_VALIDATION_ERROR` — how?

`validate()` runs pipeline steps 1–5 and 7 (context, call-chain guard, lookup, ACL, approval, input validation) and the module's optional `preflight()`. It does not run middleware. Most often:

- **A middleware `before()` hook changed the inputs** so they no longer match `input_schema`.
- **Different inputs were passed** to `validate()` and `call()`.
- **The output failed `output_schema`** — `validate()` never executes the module, so it cannot check the output.

### 1.4 Streaming module emits chunks but the merged result is wrong / fields disappear.

Chunks are combined with a recursive deep merge ([Cookbook — Streaming Modules § How chunks merge](./cookbook-streaming.md#3-how-chunks-merge)):

- **Arrays are replaced, not concatenated.** Re-emit the full list in each chunk, or give each item its own key.
- **`null` overwrites**; it does not delete the key.
- **Deep nesting.** Past `stream.max_merge_depth` (default 32) the later chunk's value replaces the earlier one wholesale instead of being merged.
- **Output validation failures after a stream do not raise** — the SDK logs a warning and emits `apcore.stream.post_validation_failed`.

### 1.5 Resuming an `APPROVAL_PENDING` call with `_approval_token` repeats ACL audit entries. Is that expected?

Yes. A resume is a fresh call: it starts again at step 1 and runs context creation, the call-chain guard, module lookup, the ACL check (which audits again) and the approval gate, which removes `_approval_token` and asks the handler's `check_approval()`. Middleware `before` hooks come after the gate, so they run only on the call that gets through. Nothing from the pending call is carried over ([PROTOCOL_SPEC §7](../spec/protocol-spec.md#7-approval-system)).

### 1.6 Hot reload says it succeeded but my module's behavior didn't change.

1. **Was the change on disk before the reload?** Some editors delay saving.
2. **Did the loader see new code?** TypeScript loads the compiled `.js` — rebuild first. Rust modules are compiled into the binary; reloading cannot pick up source changes.
3. **Check the result of `system.control.reload_module`.** `RELOAD_FAILED` means the new code did not load (`details.reason` says why). `MODULE_RELOAD_CONFLICT` means both `module_id` and `path_filter` were passed; give exactly one.

### 1.7 My traces show spans but no `target_id`.

The built-in tracing middleware names each span `apcore.module.execute` and records the target as `module_id`, alongside `caller_id` (absent for top-level calls), `method`, `duration_ms`, `success` and `error_code`. There is no separate `target_id` attribute. If calls from one request show up as unrelated root traces, the inbound `traceparent` header was not passed into `Context.create` — see [Integrating Existing Projects](./integrating-existing-projects.md).

### 1.8 `x-sensitive: true` field appears in plain text in my error message.

Redaction applies to **fields** — log record fields and the context's `redacted_inputs` / `redacted_output` — not to the text of an error message. If you build an exception message from a sensitive value, the value travels with the error to the caller and into any log line that prints the message. Build error messages without sensitive data; put identifiers you need for debugging into `details` under non-sensitive names.

### 1.9 My TypeScript SDK uses camelCase but my Python SDK uses snake_case in the same context — what's authoritative?

Both. Wire-format identifiers (Canonical IDs, schema property names, fixture fields) are language-agnostic `snake_case`. Each SDK's **method and property names** follow the host language: `useBefore()` in TypeScript, `use_before()` in Python and Rust. See [APCore Client — Language-Specific Adaptations](../features/apcore-client.md#language-specific-adaptations).

### 1.10 Where do I look first when something breaks?

1. Look the error code up in section 2.
2. Run `client.validate(module_id, inputs)` to separate pipeline failures from execution failures.
3. Add `ObsLoggingMiddleware` to log every call's inputs, outputs and errors (redacted) — see [Cookbook — Tracing and Redacted Logs](./cookbook-observability.md).
4. Inspect `context.call_chain` to see where in the call tree the error came from.
5. Compare the behaviour against the relevant [conformance fixture](../spec/conformance.md#8-conformance-test-fixtures).

---

## 2. Error Code → Cause → Fix

Common codes. For the full list see [features/error-system.md](../features/error-system.md).

| Code | Likely cause | Fix |
|------|-------------|-----|
| `MODULE_NOT_FOUND` | Canonical ID typo, or the module was never registered or discovered | `client.list_modules()` lists registered IDs; see §1.1 |
| `MODULE_DISABLED` | The module was switched off at runtime (`client.disable()` / `system.control.toggle_feature`) | `client.enable(module_id)` — requires `sys_modules.enabled: true` |
| `MODULE_TIMEOUT` | `execute()` ran longer than `executor.default_timeout` (ms, default 30000) or the call tree exceeded `executor.global_timeout` | Shorten the work or raise the limit in `apcore.yaml` |
| `MODULE_LOAD_ERROR` | The file failed to import/compile, or discovery found no (or several) module classes in it | `details` carries the reason; see §1.1 for what discovery accepts |
| `MODULE_EXECUTE_ERROR` | A non-apcore exception escaped `execute()`, a middleware hook or an approval handler | The original exception is the error's cause; fix it at the source |
| `DUPLICATE_MODULE_ID` | Two registrations or discovered files resolve to the same Canonical ID | Rename or move one of them |
| `MODULE_ID_CONFLICT` | Two classes in one multi-class file produce the same ID segment | Rename one class |
| `SCHEMA_VALIDATION_ERROR` | Inputs or output did not match `input_schema` / `output_schema` | The error's details list each failing path; fix the data or the schema |
| `SCHEMA_NOT_FOUND` | No schema file for the module under `schema.root`, or a `$ref` points at a missing file or outside the schemas directory | Check the path relative to `schema.root` |
| `SCHEMA_PARSE_ERROR` | Schema YAML/JSON is malformed | Validate the file; use JSON Schema Draft 2020-12 |
| `SCHEMA_CIRCULAR_REF` | A `$ref` → `$ref` chain re-enters itself without reaching a schema body ([PROTOCOL_SPEC §4.15](../spec/protocol-spec.md#4-schema-specification)) | Put a schema body (`properties` / `items`) in the chain, or remove the indirection |
| `SCHEMA_MAX_DEPTH_EXCEEDED` | `$ref` resolution went deeper than `schema.max_ref_depth` (default 32) | Flatten the reference chain, or raise `schema.max_ref_depth` (max 100) |
| `BINDING_FILE_INVALID` | A binding file is malformed, or its `schema_ref` file does not exist | `schema_ref` resolves relative to the binding file's directory |
| `BINDING_SCHEMA_INFERENCE_FAILED` | A binding with `auto_schema` targets a callable without enough type information | Add type hints to the callable, or give `input_schema` / `output_schema` / `schema_ref` explicitly |
| `ACL_DENIED` | No `allow` rule matched and the ACL file's `default_effect` is `deny`, or a `deny` rule matched | Add an `allow` rule — see §1.2 |
| `ACL_RULE_ERROR` | The ACL file is invalid: missing `callers`/`targets`/`effect`, unknown rule key, bad `effect` value or pattern array | Compare with [ACL Configuration Guide § Configuration Format](./acl-configuration.md#3-configuration-format) |
| `APPROVAL_DENIED` | The handler returned `rejected`, or no handler is attached under `ExecutionPolicy(strict=True)` | Inspect the handler; attach one if strict mode is on |
| `APPROVAL_TIMEOUT` | The handler returned `timeout` | Back off before retrying; the same handler answers again |
| `APPROVAL_PENDING` | Phase B: the handler returned `pending` | Keep `approval_id`; call again later with `_approval_token` in the inputs ([Cookbook — Approval-Gated Modules](./cookbook-approval-flow.md)) |
| `CALL_DEPTH_EXCEEDED` | The call chain is deeper than `executor.max_call_depth` (default 32) | Flatten the call graph, or raise the limit if the depth is intended |
| `CIRCULAR_CALL` | A module called itself, directly or through other modules | Break the cycle; move shared logic into a separate module |
| `CALL_FREQUENCY_EXCEEDED` | One module appears more than `executor.max_module_repeat` times (default 3) in a call chain | Restructure the calls, or raise the limit |
| `EXECUTION_CANCELLED` | The context's `CancelToken` was cancelled | Expected after `cancel()`; retry only with a new token ([Cookbook — Cooperative Cancellation](./cookbook-cancellation.md)) |
| `MIDDLEWARE_CHAIN_ERROR` | Internal wrapper for a failing `before()` hook — callers normally never see it | The executor unwraps it and re-raises the hook's own error (a non-apcore exception arrives as `MODULE_EXECUTE_ERROR`) |
| `PIPELINE_DEPENDENCY_ERROR` | A pipeline step `requires` something no earlier step `provides` | Move the step later (`after:` / `before:` in `pipeline.steps`), or check that `pipeline.remove` did not drop the provider |

For codes not listed here:

- The error class in [features/error-system.md](../features/error-system.md) usually names the cause.
- Search the SDK source for the code string to find where it is raised.
- Many codes have a conformance fixture under `conformance/fixtures/` showing the expected behaviour.

---

## 3. Diagnostic Snippets

Enumerate registered modules:

=== "Python"

    ```python
    from apcore import APCore, Config

    client = APCore(config=Config.load("apcore.yaml"))
    print(client.discover(), "modules discovered")
    for module_id in client.list_modules():
        print(module_id)
    ```

=== "TypeScript"

    ```typescript
    import { APCore, Config } from 'apcore-js';

    const client = new APCore({ config: Config.load('apcore.yaml') });
    console.log(await client.discover(), 'modules discovered');
    console.log(client.listModules());
    ```

=== "Rust"

    ```rust
    use apcore::{APCore, ModuleError};

    #[tokio::main]
    async fn main() -> Result<(), ModuleError> {
        let client = APCore::from_path("apcore.yaml")?;
        // Returns 0 unless a discoverer is attached (see §1.1).
        println!("{} modules discovered", client.discover().await?);
        // list_modules takes an optional tag filter and prefix filter; None means "all".
        for module_id in client.list_modules(None, None) {
            println!("{module_id}");
        }
        Ok(())
    }
    ```

Load `apcore.yaml` and print the effective configuration (loading validates it and raises on errors):

=== "Python"

    ```python
    from apcore import Config

    config = Config.load("apcore.yaml")
    print(config.data)
    ```

=== "TypeScript"

    ```typescript
    import { Config } from 'apcore-js';

    const config = Config.load('apcore.yaml');
    console.log(config.data);
    ```

=== "Rust"

    ```rust
    use apcore::{Config, ModuleError};
    use std::path::Path;

    fn main() -> Result<(), ModuleError> {
        let config = Config::load(Path::new("apcore.yaml"))?;
        println!("{}", config.data());
        Ok(())
    }
    ```

---

## 4. When to file an issue

Open a GitHub issue at `aiperceivable/apcore` if:

- Your SDK's behaviour contradicts the linked conformance fixture and the fix is not obvious.
- A normative requirement in PROTOCOL_SPEC is unclear or appears self-contradictory.
- You hit an error code with no documented cause and the SDK source isn't decisive.

File SDK-specific bugs (Python-only, TypeScript-only, Rust-only) in the **respective SDK repo**.

---

## See also

- [features/error-system.md](../features/error-system.md) — error class hierarchy and codes
- [spec/conformance.md](../spec/conformance.md) — conformance levels and fixture catalog
- [features/acl-system.md](../features/acl-system.md) — ACL rule evaluation, including `$or` / `$not`
- [PROTOCOL_SPEC §8](../spec/protocol-spec.md#8-error-handling-specification) — normative error specification
