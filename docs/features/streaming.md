# Streaming Support

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../../PROTOCOL_SPEC.md) §5 Module Specification (streaming hooks).


## Overview

The Streaming System enables modules to produce output incrementally as a sequence of chunks rather than a single complete response. This is essential for modules that wrap LLM APIs, process large datasets, or perform real-time data transformations. The executor implements a three-phase streaming pipeline that separates chunk emission from post-execution validation and middleware processing, ensuring that consumers receive chunks in real-time while maintaining the integrity of the full pipeline.

## Requirements

- Modules that support streaming **MUST** implement a `stream()` method (in addition to or instead of `execute()`) that returns an async iterator/generator of output chunks.
- The executor **MUST** support a `stream()` method that returns an async iterator yielding chunks as they are produced.
- Chunks **MUST** be accumulated using a deep merge algorithm to produce a final combined output for validation.
- Output validation and after-middleware **MUST** run on the accumulated output after all chunks are emitted, not on individual chunks.
- If a module does not implement `stream()`, the executor's `stream()` method **MUST** fall back to calling `execute()` and yielding the complete result as a single chunk.
- The `streaming` annotation **SHOULD** be set on modules that support streaming, enabling AI agents to discover streamable modules.
- Deep merge **MUST** be depth-capped (default 32) to prevent stack overflow on deeply nested structures.

## Technical Design

### Three-Phase Streaming Pipeline

The executor's `stream()` method operates in three phases:

**Phase 1 — Pipeline Setup:**
The standard execution pipeline runs through Steps 1–7 (Context Creation, Call Chain Guard, Module Lookup, ACL, Approval Gate, Middleware Before, Input Validation). The context is flagged with `stream=true`, and the module's `stream()` method is invoked instead of `execute()`.

**Phase 2 — Chunk Emission:**
The executor iterates over the module's async iterator, yielding each chunk to the caller immediately. Each chunk is also accumulated into a running merged output using the deep merge algorithm.

**Phase 3 — Post-Validation:**
After all chunks have been emitted, the accumulated output is passed through Output Validation (Step 9) and Middleware After Chain (Step 10). Validation errors at this stage are logged but do not retroactively invalidate chunks already yielded to the consumer.

### Module Interface

=== "Python"
    ```python
    from apcore.decorator import module
    from apcore.context import Context
    from typing import AsyncIterator

    @module(
        id="llm.chat",
        description="Stream chat completions",
        annotations={"streaming": True},
    )
    async def chat(prompt: str, context: Context) -> AsyncIterator[dict]:
        async for token in llm_client.stream(prompt):
            yield {"content": token, "done": False}
        yield {"content": "", "done": True}
    ```
=== "TypeScript"
    Streaming modules in TypeScript must implement the `Module` interface with a `stream()` method.
    The `client.module()` shorthand creates a `FunctionModule` which only supports `execute()`.

    ```typescript
    import { APCore, Registry, Executor } from "apcore-js";
    import type { Context, Module, TSchema } from "apcore-js";

    class ChatModule implements Module {
        inputSchema: TSchema = { type: "object", properties: { prompt: { type: "string" } } };
        outputSchema: TSchema = { type: "object", properties: { content: { type: "string" }, done: { type: "boolean" } } };
        description = "Stream chat completions";

        async execute(inputs: Record<string, unknown>, context: Context): Promise<Record<string, unknown>> {
            // Fallback for non-streaming callers
            const result = await collectStream(this.stream(inputs, context));
            return result;
        }

        async *stream(inputs: Record<string, unknown>, context: Context): AsyncGenerator<Record<string, unknown>> {
            const prompt = inputs.prompt as string;
            for await (const token of llmClient.stream(prompt)) {
                yield { content: token, done: false };
            }
            yield { content: "", done: true };
        }
    }

    const client = new APCore();
    client.register("llm.chat", new ChatModule());
    ```
=== "Rust"
    ```rust
    use apcore::module::Module;
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use async_trait::async_trait;
    use serde_json::{json, Value};
    use tokio::sync::mpsc;

    struct ChatModule;

    #[async_trait]
    impl Module for ChatModule {
        fn description(&self) -> &str { "Stream chat completions" }
        fn input_schema(&self) -> Value { json!({"type": "object", "properties": {"prompt": {"type": "string"}}}) }
        fn output_schema(&self) -> Value { json!({"type": "object", "properties": {"content": {"type": "string"}}}) }

        // Module trait returns Option<ChunkStream> = Option<Pin<Box<dyn Stream<Item=Result<Value, ModuleError>> + Send>>>.
        // None signals "no streaming support — fall back to execute() wrapped as a single chunk" (per spec).
        fn stream(&self, inputs: Value, _ctx: &Context<Value>) -> Option<ChunkStream> {
            let prompt = inputs["prompt"].as_str()?.to_string();
            Some(Box::pin(async_stream::stream! {
                // Produce chunks incrementally
                for token in llm_client_stream(&prompt).await? {
                    yield Ok(json!({"content": token, "done": false}));
                }
                yield Ok(json!({"content": "", "done": true}));
            }))
        }
    }
    ```

### Consuming Streams

=== "Python"
    ```python
    from apcore import APCore

    client = APCore()

    async for chunk in client.stream("llm.chat", {"prompt": "Hello, world!"}):
        print(chunk["content"], end="", flush=True)
    ```
=== "TypeScript"
    ```typescript
    import { APCore } from "apcore-js";

    const client = new APCore();

    for await (const chunk of client.stream("llm.chat", { prompt: "Hello, world!" })) {
        process.stdout.write(chunk.content);
    }
    ```
=== "Rust"
    ```rust
    use apcore::APCore;

    let client = APCore::new();
    let chunks = client.stream(
        "llm.chat",
        serde_json::json!({"prompt": "Hello, world!"}),
        None,
        None,
    ).await?;
    for chunk in &chunks {
        print!("{}", chunk["content"].as_str().unwrap_or(""));
    }
    ```

### Deep Merge Algorithm

Chunks are accumulated using a recursive deep merge. When two chunks contain the same key:

| Left Value | Right Value | Result |
|-----------|-------------|--------|
| dict/object | dict/object | Recursively merge |
| any | any | Right value wins (including arrays) |

**Depth cap:** Merge recursion is capped at 32 levels. If nesting exceeds this limit, the right value replaces the left at that level without further recursion.

**Example:**
```
Chunk 1: {"content": "Hello", "metadata": {"tokens": 1}}
Chunk 2: {"content": " world", "metadata": {"tokens": 1, "model": "gpt-4"}}
Merged:  {"content": " world", "metadata": {"tokens": 1, "model": "gpt-4"}}
```

!!! note
    String concatenation is **not** performed automatically — the deep merge uses right-value-wins for scalar types. Modules that need concatenated strings should accumulate them internally and yield the growing string in each chunk.

### Fallback Behavior

When the executor's `stream()` is called on a module that does not implement a `stream()` method:

1. The executor calls `execute()` normally.
2. The single result is yielded as one chunk.
3. No deep merge is needed since there is only one chunk.

### Streaming Annotation

Modules that support streaming **SHOULD** declare the `streaming` annotation:

```yaml
annotations:
  streaming: true
```

This annotation enables:
- AI agents to discover which modules support streaming.
- The executor to log a warning if `stream()` is called on a non-streaming module.
- Schema export to include streaming capability in tool definitions.

## Dependencies

- **Core Executor** — Implements the three-phase streaming pipeline.
- **Middleware System** — Before-middleware runs in Phase 1; after-middleware runs in Phase 3.
- **Schema System** — Output validation runs on the accumulated output in Phase 3.
- **Cancellation System** — `CancelToken.check()` can be called between chunks for cooperative cancellation.

??? info "Python SDK reference"
    The following information is **not a protocol requirement** — it documents the Python SDK's implementation for reference.

    **Source files:**

    | File | Purpose |
    |------|---------|
    | `src/apcore/executor.py` | `Executor.stream()` — three-phase streaming pipeline |
    | `src/apcore/utils/merge.py` | Deep merge utility with depth cap |

## Testing Strategy

- **Basic streaming tests** verify that chunks are yielded in order and the accumulated output matches expectations.
- **Deep merge tests** verify recursive merging of dicts, array concatenation, scalar replacement, and depth cap enforcement.
- **Fallback tests** verify that non-streaming modules produce a single-chunk stream.
- **Pipeline integration tests** verify that before-middleware runs before streaming starts and after-middleware runs after all chunks are emitted.
- **Validation tests** verify that output validation runs on the accumulated output and that validation failures in Phase 3 do not invalidate already-yielded chunks.
- **Cancellation tests** verify that cancelling during streaming stops chunk emission and raises `ExecutionCancelledError`.

## Contract: Module.stream

### Inputs
- `inputs` (dict/object/Value, required) — validated against the module's `input_schema`
- `context` (Context, required) — execution context

### Errors
- `SchemaValidationError(code=SCHEMA_VALIDATION_FAILED)` — `inputs` fails validation
- Any error raised mid-stream is surfaced as the final item in the async iterator (iterator terminates after the error item)

### Returns
- On success: `AsyncIterator[dict]`/`AsyncIterable<Record<string,unknown>>`/`Stream<Value>` — lazy sequence of partial output dicts; the iterator MUST be exhausted or explicitly closed to release resources

### Properties
- async: true (streaming MUST be async in all SDK languages)
- thread_safe: false (a stream instance MUST NOT be shared across concurrent consumers)
- pure: false (may hold open connections or file handles)
- idempotent: false
