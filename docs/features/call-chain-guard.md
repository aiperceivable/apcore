# Call Chain Guard

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §5.7 Context Object (`call_chain` field).


## Overview

The Call Chain Guard is a safety mechanism that prevents runaway, circular, and abusive module call patterns. It is invoked at Step 2 of the execution pipeline — before module lookup — and performs three sequential checks: call depth limiting, circular call detection, and frequency throttling. These checks protect the system from unbounded recursion, tight-loop abuse, and stack overflow scenarios.

## Requirements

- Evaluate three safety checks in strict order: depth → circular → frequency.
- Reject calls that exceed the configured maximum nesting depth.
- Detect and reject circular call patterns of length ≥ 2 (e.g., A→B→A).
- Track and reject calls where a single module appears more than the configured maximum repeat count in the call chain.
- All three checks operate on the call chain recorded in the `Context` object.
- Configurable limits **MUST** have sensible defaults but be overridable per executor or per call.

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

**Step 1 — Depth Limit:**
Check that `len(call_chain) <= max_call_depth`. If the chain exceeds the limit, raise `CallDepthExceededError` with `current_depth` and `max_depth`.

**Step 2 — Circular Detection:**
Extract the prior chain (everything except the last element). Find the last occurrence of `module_id` in the prior chain. If found and there are any entries after that occurrence (i.e., other modules were called in between), a cycle is detected. Raise `CircularCallError` with the offending `module_id` and the full `call_chain`.

For example, given chain `[A, B, C, A]` (where the last `A` is the current call): the prior chain is `[A, B, C]`, `A` is found at index 0 with subsequent entries `[B, C]`, so a circular pattern A→B→C→A is detected.

**Step 3 — Frequency Throttle:**
Count occurrences of `module_id` in the full call chain. If `count > max_module_repeat`, raise `CallFrequencyExceededError` with `module_id`, `count`, and `max_repeat`.

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
            CallDepthExceededError: Chain exceeds max_call_depth
            CircularCallError: Circular call pattern detected
            CallFrequencyExceededError: Module appears too many times
        """
        ...
    ```
=== "TypeScript"
    ```typescript
    import { guardCallChain } from "apcore/utils/call-chain";

    function guardCallChain(
        moduleId: string,
        callChain: readonly string[],
        maxCallDepth: number = 32,
        maxModuleRepeat: number = 3,
    ): void;
    // Throws: CallDepthExceededError, CircularCallError, CallFrequencyExceededError
    ```
=== "Rust"
    ```rust
    use apcore::utils::call_chain::guard_call_chain;

    pub fn guard_call_chain(
        module_id: &str,
        call_chain: &[String],
        max_call_depth: usize,      // default: 32
        max_module_repeat: usize,   // default: 3
    ) -> Result<(), ModuleError>;
    ```

### Error Types

| Error | Code | Key Properties | Description |
|-------|------|----------------|-------------|
| `CallDepthExceededError` | `CALL_DEPTH_EXCEEDED` | `current_depth`, `max_depth`, `call_chain` | Chain exceeds maximum depth |
| `CircularCallError` | `CIRCULAR_CALL` | `module_id`, `call_chain` | Circular invocation detected |
| `CallFrequencyExceededError` | `CALL_FREQUENCY_EXCEEDED` | `module_id`, `count`, `max_repeat` | Module called too many times |

### Examples

**Depth limit scenario:**
```
Call chain: [A, B, C, ..., Z] (length 33, max_call_depth=32)
→ CallDepthExceededError (current_depth=33, max_depth=32)
```

**Circular detection scenario:**
```
Call chain: [A, B, C, B]  (B is the current call, added by Context.child())
Prior chain: [A, B, C]
B found at index 1, subsequent entries: [C]
→ CircularCallError (B→C→B cycle detected)
```

**Frequency throttle scenario:**
```
Call chain: [A, B, A, B, A, B]  (B is the current call)
max_module_repeat = 3
B appears 3 times, which equals max → no error (≤ 3)

Call chain: [A, B, A, B, A, B, A]  (A is the current call)
A appears 4 times, exceeds max_module_repeat=3
→ CallFrequencyExceededError (module_id=A, count=4, max_repeat=3)
```

### Configuration

Limits are configurable through the Config Bus:

```yaml
executor:
  max_call_depth: 32
  max_module_repeat: 3
```

Or overridden per executor instance:

=== "Python"
    ```python
    from apcore import Executor, Registry

    executor = Executor(
        registry=Registry(),
        max_call_depth=16,
        max_module_repeat=2,
    )
    ```
=== "TypeScript"
    ```typescript
    import { Executor, Registry } from "apcore-js";

    const executor = new Executor({
        registry: new Registry(),
        maxCallDepth: 16,
        maxModuleRepeat: 2,
    });
    ```
=== "Rust"
    ```rust
    use apcore::{Executor, Registry};

    let executor = Executor::builder(registry)
        .max_call_depth(16)
        .max_module_repeat(2)
        .build()?;
    ```

## Integration

The Call Chain Guard is invoked automatically at **Step 2** of the [Core Execution Engine](./core-executor.md) pipeline. It reads the `call_chain` from the current `Context` and validates the target module against the configured limits.

Modules that perform nested calls (calling other modules within their execution) will naturally build up the call chain through `Context.child()`, which appends the target module ID to the chain.

## Dependencies

- **Context** — Reads `call_chain` from the execution context.
- **Error System** — Raises `CallDepthExceededError`, `CircularCallError`, `CallFrequencyExceededError`.

??? info "Python SDK reference"
    The following table is **not a protocol requirement** — it documents the Python SDK's source layout for implementers/users of `apcore-python`.

    **Source files:**

    | File | Purpose |
    |------|---------|
    | `src/apcore/utils/call_chain.py` | `guard_call_chain()`, constants |

## Testing Strategy

- **Depth tests** verify rejection at exactly `max_call_depth + 1` and acceptance at `max_call_depth`.
- **Circular tests** verify detection of 2-node cycles (A→B→A), 3-node cycles (A→B→C→A), and non-circular chains that share module names without forming cycles.
- **Frequency tests** verify rejection at exactly `max_module_repeat + 1` and acceptance at `max_module_repeat`.
- **Order tests** verify that depth is checked before circular, and circular before frequency (a chain that fails both depth and circular should raise `CallDepthExceededError`).
- **Configuration tests** verify that custom limits override defaults.

## Contract: guard_call_chain

### Inputs
- `context` (Context, required) — current execution context containing the call chain
- `module_id` (str/string/&str, required) — the module about to be called
- `max_depth` (int, optional, default=`DEFAULT_MAX_CALL_DEPTH`) — maximum allowed call-chain depth
- `max_repeat` (int, optional, default=`DEFAULT_MAX_REPEAT`) — maximum allowed repeat invocations of the same module in the current chain

### Errors
- `CallDepthExceededError(code=CALL_DEPTH_EXCEEDED)` — call chain depth exceeds `max_depth`
- `CircularCallError(code=CIRCULAR_CALL)` — `module_id` already appears in the current call chain (cycle detected)
- `CallFrequencyExceededError(code=CALL_FREQUENCY_EXCEEDED)` — `module_id` has been invoked more than `max_repeat` times in this chain

### Returns
- On success (guard passes): void/None/() — no return value; raises on violation

### Properties
- async: false
- thread_safe: true
- pure: true (reads context state, does not mutate)
