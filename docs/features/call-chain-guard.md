---
description: "Executor Step 2 guard with three ordered checks on the Context call_chain: max call depth, circular call detection (length >= 2), per-module frequency throttling vs runaway recursion."
---

# Call Chain Guard

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §5.7 Context Object (`call_chain` field), §9.1.1 (limits); algorithm [A20](../spec/algorithms.md#a20-guard_call_chain-call-chain-safety-check). This page is the canonical explanation of the guard.


## Overview

The Call Chain Guard is a safety mechanism that prevents runaway, circular, and abusive module call patterns. It is invoked at Step 2 of the execution pipeline — before module lookup — and performs three sequential checks: call depth limiting, circular call detection, and frequency throttling. These checks protect the system from unbounded recursion, tight-loop abuse, and stack overflow scenarios.

## Requirements

- Evaluate three safety checks in strict order: depth → circular → frequency.
- Reject calls that exceed the configured maximum nesting depth.
- Detect and reject circular call patterns of length ≥ 2 (e.g., A→B→A).
- Track and reject calls where a single module appears more than the configured maximum repeat count in the call chain. Because any repeat with another module in between is already a cycle, this check only ever fires for direct self-recursion (A→A→A→A).
- All three checks operate on the call chain recorded in the `Context` object.
- Both limits have defaults and are set through the Config Bus keys `executor.max_call_depth` and `executor.max_module_repeat` (see [Configuration](#configuration)). There is no per-call override.

## Technical Design

### Constants

| Constant | Default | Description |
|----------|---------|-------------|
| `DEFAULT_MAX_CALL_DEPTH` | 32 | Maximum allowed call chain length |
| `DEFAULT_MAX_MODULE_REPEAT` | 3 | Maximum times a module can appear in one call chain |

### Algorithm (A20)

The `guard_call_chain` function performs three checks in order. If any check fails, the corresponding error is raised immediately and subsequent checks are skipped.

!!! note
    The `call_chain` passed to this function already includes `module_id` at the end, as set by `Context.child()`. The guard validates the chain in its current state.

**Step 0 — Limit floors:**
If `max_call_depth < 1` or `max_module_repeat < 1`, raise `InvalidInputError` with code `GENERAL_INVALID_INPUT` before inspecting the chain (D-84).

**Step 1 — Depth Limit:**
Check that `len(call_chain) <= max_call_depth`. If the chain exceeds the limit, raise `CallDepthExceededError`. The comparison is strict: a chain of exactly `max_call_depth` entries passes.

**Step 2 — Circular Detection:**
Extract the prior chain (everything except the last element). Find the last occurrence of `module_id` in the prior chain. If found and there are any entries after that occurrence (i.e., other modules were called in between), a cycle is detected. Raise `CircularCallError` with the offending `module_id` and the full `call_chain`.

A repeat with nothing in between is a direct self-call, not a cycle: `[A, A]` passes this step, `[A, B, A]` does not.

For example, given chain `[A, B, C, A]` (where the last `A` is the current call): the prior chain is `[A, B, C]`, `A` is found at index 0 with subsequent entries `[B, C]`, so a circular pattern A→B→C→A is detected.

**Step 3 — Frequency Throttle:**
Count occurrences of `module_id` in the full call chain. If `count > max_module_repeat`, raise `CallFrequencyExceededError` with `module_id`, `count`, and `max_repeat`. Since Step 2 already rejects any repeat separated by another module, the calls that reach this check with `count > 1` are direct self-recursion; this step bounds how deep a module may recurse into itself.

### Function Signature

=== "Python"
    ```python
    from apcore.utils.call_chain import guard_call_chain

    def guard_call_chain(
        module_id: str,
        call_chain: list[str] | tuple[str, ...],
        *,
        max_call_depth: int = 32,
        max_module_repeat: int = 3,
    ) -> None:
        """
        Validate call chain safety (Algorithm A20).

        Raises:
            InvalidInputError: A limit is below 1 (GENERAL_INVALID_INPUT)
            CallDepthExceededError: Chain exceeds max_call_depth
            CircularCallError: Circular call pattern detected
            CallFrequencyExceededError: Module appears too many times
        """
        ...
    ```
=== "TypeScript"
    ```typescript
    import { guardCallChain } from "apcore-js";

    function guardCallChain(
        moduleId: string,
        callChain: readonly string[],
        maxCallDepth: number = 32,
        maxModuleRepeat: number = 3,
    ): void;
    // Throws: InvalidInputError, CallDepthExceededError, CircularCallError, CallFrequencyExceededError
    ```
=== "Rust"
    ```rust
    use apcore::guard_call_chain;

    pub fn guard_call_chain(
        module_id: &str,
        call_chain: &[String],
        max_call_depth: usize,      // default: 32
        max_module_repeat: usize,   // default: 3
    ) -> Result<(), ModuleError>;
    ```

    `guard_call_chain_for_context(&ctx, module_id)` (defaults) and
    `guard_call_chain_with_repeat(&ctx, module_id, max_depth, max_module_repeat)` are
    thin wrappers that take a `Context<T>` of any services type; the executor's
    `call_chain_guard` step uses the latter with the configured limits.

### Error Types

| Error | Code | `details` keys | Description |
|-------|------|----------------|-------------|
| `CallDepthExceededError` | `CALL_DEPTH_EXCEEDED` | `depth`, `max_depth`, `call_chain` | Chain exceeds maximum depth |
| `CircularCallError` | `CIRCULAR_CALL` | `module_id`, `call_chain` | Circular invocation detected |
| `CallFrequencyExceededError` | `CALL_FREQUENCY_EXCEEDED` | `module_id`, `count`, `max_repeat`, `call_chain` | Module called too many times |
| `InvalidInputError` | `GENERAL_INVALID_INPUT` | — | `max_call_depth` or `max_module_repeat` is below 1 |

In Rust all four are a `ModuleError` carrying the corresponding `ErrorCode`.

### Examples

**Depth limit scenario:**
```text
Call chain: [mod.0, mod.1, ..., mod.32]  (length 33, max_call_depth=32)
→ CallDepthExceededError (depth=33, max_depth=32)

Call chain: [mod.0, mod.1, ..., mod.31]  (length 32, max_call_depth=32)
→ passes (the comparison is strict)
```

**Circular detection scenario:**
```text
Call chain: [A, B, C, B]  (B is the current call, added by Context.child())
Prior chain: [A, B, C]
B found at index 1, subsequent entries: [C]
→ CircularCallError (B→C→B cycle detected)

Call chain: [A, B, A, B, A, B]  (B is the current call)
Prior chain: [A, B, A, B, A]; last B at index 3, subsequent entries: [A]
→ CircularCallError (the cycle is caught before frequency is ever counted)
```

**Frequency throttle scenario (direct self-recursion):**
```text
Call chain: [A, B, B, B]  (B is the current call; B calls itself)
Prior chain: [A, B, B]; last B at index 2, no entries after it → not circular
B appears 3 times, max_module_repeat = 3 → passes (≤ 3)

Call chain: [A, B, B, B, B]  (B is the current call)
Not circular (same reasoning); B appears 4 times > 3
→ CallFrequencyExceededError (module_id=B, count=4, max_repeat=3)
```

### Configuration

Both limits are Config Bus keys. The executor's `call_chain_guard` step reads them from the `Config` it was built with and falls back to the defaults above:

```yaml
# apcore.yaml
executor:
  max_call_depth: 16
  max_module_repeat: 2
```

The same keys can be set programmatically before the client is built:

=== "Python"
    ```python
    from apcore import APCore, Config

    config = Config.from_defaults()
    config.set("executor.max_call_depth", 16)
    config.set("executor.max_module_repeat", 2)

    client = APCore(config=config)
    ```
=== "TypeScript"
    ```typescript
    import { APCore, Config } from "apcore-js";

    const config = Config.fromDefaults();
    config.set("executor.max_call_depth", 16);
    config.set("executor.max_module_repeat", 2);

    const client = new APCore({ config });
    ```
=== "Rust"
    ```rust
    use apcore::{APCore, Config};
    use serde_json::json;

    fn main() {
        let mut config = Config::from_defaults();
        config.set("executor.max_call_depth", json!(16));
        config.set("executor.max_module_repeat", json!(2));

        let _client = APCore::with_config(config);
    }
    ```

An `Executor` constructed directly takes the same `Config` (`Executor(registry, config=config)`, `new Executor({ registry, config })`, `Executor::new(registry, config)`). Valid ranges are `1..1000` for `max_call_depth` and `1..100` for `max_module_repeat` ([protocol-spec §9.1.1](../spec/protocol-spec.md)).

## Integration

The Call Chain Guard is invoked automatically at **Step 2** of the [Core Execution Engine](./core-executor.md) pipeline. It reads the `call_chain` from the current `Context` and validates the target module against the configured limits.

Modules that perform nested calls (calling other modules within their execution) will naturally build up the call chain through `Context.child()`, which appends the target module ID to the chain.

## Dependencies

- **Context** — Reads `call_chain` from the execution context.
- **Error System** — Raises `CallDepthExceededError`, `CircularCallError`, `CallFrequencyExceededError`, `InvalidInputError`.
- **Config Bus** — Supplies `executor.max_call_depth` and `executor.max_module_repeat`.

??? info "Python SDK reference"
    The following table is **not a protocol requirement** — it documents the Python SDK's source layout for implementers/users of `apcore-python`.

    **Source files:**

    | File | Purpose |
    |------|---------|
    | `src/apcore/utils/call_chain.py` | `guard_call_chain()`, constants |

## Testing Strategy

- **Depth tests** verify rejection at exactly `max_call_depth + 1` and acceptance at `max_call_depth`.
- **Circular tests** verify detection of 2-node cycles (A→B→A), 3-node cycles (A→B→C→A), and that direct self-calls (A→A) are not treated as cycles.
- **Frequency tests** verify, on self-recursive chains, rejection at exactly `max_module_repeat + 1` and acceptance at `max_module_repeat`.
- **Floor tests** verify that a limit below 1 raises `GENERAL_INVALID_INPUT`.
- **Order tests** verify that depth is checked before circular, and circular before frequency (a chain that fails both depth and circular should raise `CallDepthExceededError`).
- **Configuration tests** verify that `executor.max_call_depth` / `executor.max_module_repeat` set on the `Config` override the defaults.

Cross-language cases live in `conformance/fixtures/call_chain.json`.

## Contract: guard_call_chain

### Inputs
- `module_id` (str/string/&str, required) — the module about to be called
- `call_chain` (sequence of str, required) — the current call chain; already includes `module_id` at the end, as set by `Context.child()`
- `max_call_depth` (int, optional, default=`DEFAULT_MAX_CALL_DEPTH`) — maximum allowed call-chain depth
- `max_module_repeat` (int, optional, default=`DEFAULT_MAX_MODULE_REPEAT`) — maximum allowed repeat invocations of the same module in the current chain

### Errors
- `InvalidInputError(code=GENERAL_INVALID_INPUT)` — `max_call_depth` or `max_module_repeat` is less than 1 (checked first, D-84)
- `CallDepthExceededError(code=CALL_DEPTH_EXCEEDED)` — `len(call_chain) > max_call_depth`
- `CircularCallError(code=CIRCULAR_CALL)` — `module_id` occurs earlier in the chain with at least one other module after that occurrence (a cycle of length ≥ 2); direct self-recursion is not circular
- `CallFrequencyExceededError(code=CALL_FREQUENCY_EXCEEDED)` — `module_id` occurs more than `max_module_repeat` times in the chain

### Returns
- On success (guard passes): void/None/() — no return value; raises on violation

### Properties
- async: false
- thread_safe: true
- pure: true (reads the `call_chain` argument, does not mutate)
