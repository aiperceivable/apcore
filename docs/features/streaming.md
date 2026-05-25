# Streaming Support

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §5 Module Specification (streaming hooks).


## Overview

The Streaming System enables modules to produce output incrementally as a sequence of chunks rather than a single complete response. This is essential for modules that wrap LLM APIs, process large datasets, or perform real-time data transformations. The executor implements a three-phase streaming pipeline that separates chunk emission from post-execution validation and middleware processing, ensuring that consumers receive chunks in real-time while maintaining the integrity of the full pipeline.

## Streaming Module Interface (Issue #62)

Earlier versions of this document described streaming support in prose: a module "should implement a `stream()` method". SDKs detected support via duck-typing (`hasattr(module, 'stream')`). This section defines an explicit, language-idiomatic interface that streaming modules **MUST** satisfy and that adapter / bridge code (e.g., `apcore-a2a`, `apcore-mcp`) can detect statically.

!!! warning "Discovered during apcore-a2a upgrade"
    Bridge layers could not distinguish streaming-capable modules from plain ones — all looked alike under the base `Module` type. A module that implemented `stream()` with a wrong signature only failed at the first call, producing cryptic errors far from the source. The contract below replaces duck-typing with a stable, statically-detectable interface.

### The `StreamingModule` Interface

=== "Python"
    ```python
    from typing import AsyncIterator, Protocol, runtime_checkable
    from apcore.context import Context

    @runtime_checkable
    class StreamingModule(Protocol):
        """Modules that produce output incrementally MUST implement this Protocol.

        SDKs MUST export `StreamingModule` from the top-level package so
        bridge / adapter code can call `isinstance(module, StreamingModule)`.
        """

        async def stream(
            self,
            inputs: dict,
            context: Context,
        ) -> AsyncIterator[dict]:
            ...
    ```

    Detection:

    ```python
    from apcore import StreamingModule

    if isinstance(module, StreamingModule):
        async for chunk in module.stream(inputs, context):
            ...
    ```

=== "TypeScript"
    TypeScript interfaces are structural, so a Symbol-based marker provides reliable runtime detection. SDKs **MUST** export the interface, the marker, and a type-narrowing helper.

    ```typescript
    import type { Module, Context } from "apcore-js";

    export const STREAMING_MARKER = Symbol.for("apcore.streaming");

    export interface StreamingModule extends Module {
        readonly [STREAMING_MARKER]: true;
        stream(
            inputs: Record<string, unknown>,
            context: Context,
        ): AsyncIterable<Record<string, unknown>>;
    }

    // Transitional detection: prefers the marker; falls back to method presence.
    // The fallback branch is deprecated and emits a one-shot warning per module
    // (see Migration from Duck-Typing below).
    export function isStreamingModule(m: Module): m is StreamingModule {
        if ((m as Record<symbol, unknown>)[STREAMING_MARKER] === true &&
            typeof (m as { stream?: unknown }).stream === "function") {
            return true;
        }
        if (typeof (m as { stream?: unknown }).stream === "function") {
            warnLegacyStreamingOnce(m);
            return true;
        }
        return false;
    }
    ```

    Detection:

    ```typescript
    import { isStreamingModule } from "apcore-js";

    if (isStreamingModule(module)) {
        for await (const chunk of module.stream(inputs, context)) {
            // ...
        }
    }
    ```

=== "Rust"
    The base `Module` trait already declares `fn stream(&self, ...) -> Option<ChunkStream>` (where `None` means "no streaming, fall back to `execute()`"). The new `StreamingModule` trait is **additive**: it gives a stable type-level handle for adapter / bridge code that needs to interact with the chunk stream directly (e.g., to attach per-chunk middleware) instead of going through the type-erased `Option<ChunkStream>` return.

    Two detection paths coexist:

    - **`module.stream(inputs, ctx)`** — the existing call-site detection. Returns `Some(stream)` if streaming is supported, `None` otherwise. This remains the canonical path for executor / pipeline code.
    - **`module.as_streaming()`** — a new trait-object accessor on `Module` (default returns `None`; streaming modules override). Returns `Option<&dyn StreamingModule>`, letting adapter code obtain a typed reference for static dispatch when needed.

    ```rust
    use apcore::context::Context;
    use apcore::module::{ChunkStream, Module};
    use serde_json::Value;

    pub trait StreamingModule: Module {
        fn stream(&self, inputs: Value, context: &Context<Value>) -> ChunkStream;
    }

    // Default in the base Module trait (already present):
    //
    // pub trait Module: Send + Sync {
    //     // ... existing methods ...
    //     fn stream(&self, _inputs: Value, _ctx: &Context<Value>) -> Option<ChunkStream> { None }
    //     fn as_streaming(&self) -> Option<&dyn StreamingModule> { None }
    // }

    pub struct ChatModule;

    impl Module for ChatModule {
        // ... existing methods ...
        fn stream(&self, inputs: Value, ctx: &Context<Value>) -> Option<ChunkStream> {
            Some(<Self as StreamingModule>::stream(self, inputs, ctx))
        }
        fn as_streaming(&self) -> Option<&dyn StreamingModule> { Some(self) }
    }

    impl StreamingModule for ChatModule {
        fn stream(&self, _inputs: Value, _ctx: &Context<Value>) -> ChunkStream {
            Box::pin(async_stream::stream! {
                // yield chunks
            })
        }
    }
    ```

    Detection by adapter code that needs the typed handle:

    ```rust
    if let Some(streaming) = module.as_streaming() {
        let stream = streaming.stream(inputs, &context);
        // consume chunks with full StreamingModule context
    }
    ```

    Detection by executor / pipeline code that only needs the chunk stream:

    ```rust
    match module.stream(inputs, &context) {
        Some(stream) => /* drive chunk loop */,
        None => /* fall back to execute() */,
    }
    ```

    Implementations **MUST** keep the two paths consistent: a module that returns `Some(_)` from `as_streaming()` MUST return `Some(_)` from `Module::stream()`, and vice versa.

### Normative Rules

- **MUST** — SDKs MUST export the streaming interface (`StreamingModule` / `isStreamingModule` / `Module::as_streaming`) as part of their public API so third-party adapter and bridge code can rely on stable detection.
- **MUST** — Adapter and bridge code MUST detect streaming support via the language's standard mechanism above (`isinstance` / `isStreamingModule` / `Module::as_streaming`), not via direct `hasattr` / `typeof` checks on the literal method name.
- **SHOULD (transitional)** — In TypeScript, new streaming modules SHOULD declare the `[STREAMING_MARKER]: true` property. SDKs MUST accept method-presence-only modules during one minor-version deprecation window (`isStreamingModule` falls back to method-presence detection and emits a one-shot `DeprecationWarning`-equivalent log naming the module ID and class name). The marker becomes a **MUST** at the next major SDK version. This window gives existing TS code a non-breaking migration path.
- **SHOULD** — When a module declares streaming support (via decorator, annotation, or marker) but its `stream()` method does not satisfy the interface (wrong arity, wrong return type, missing `async`, etc.), SDKs SHOULD raise a structured error at **module-load time**, not at first call. Error type: `StreamingInterfaceError` with the following fields:

    | Field | Type | Meaning |
    |-------|------|---------|
    | `module_id` | string | The module ID under which registration was attempted. |
    | `expected_signature` | string | Human-readable expected signature (language-specific). |
    | `actual_signature` | string | Human-readable observed signature. |
    | `mismatch_reason` | string | Short tag: `wrong_arity` / `not_async` / `wrong_return_type` / `missing_marker`. |

- **MAY** — Non-streaming modules MAY define a method literally named `stream` for unrelated purposes (e.g., wrapping a third-party API). SDKs **MUST NOT** treat such modules as streaming unless they satisfy the full interface contract above (Protocol/marker/trait).

### Migration from Duck-Typing

Existing modules that previously relied on a bare `stream()` method should migrate as follows:

- **Python** — Their class already satisfies `StreamingModule` (Protocol) structurally if the signature matches. `@runtime_checkable` is on the Protocol declaration in the SDK, not on the module. **No code change is required** for correctly-signed modules.
- **TypeScript** — During the deprecation window, existing modules continue to work via method-presence fallback in `isStreamingModule`; new modules SHOULD add `[STREAMING_MARKER]: true`. At the next major SDK version the marker becomes mandatory. Module authors are encouraged to add the marker now to silence the one-shot deprecation log.
- **Rust** — Existing modules that implement `Module::stream() -> Option<ChunkStream>` continue to work. Modules that want adapter / bridge code to obtain a typed `&dyn StreamingModule` handle SHOULD additionally override `Module::as_streaming` and implement the `StreamingModule` trait. SDKs MAY ship a default-method consistency check that warns when only one of the two is overridden.

## Requirements

- Modules that support streaming **MUST** satisfy the [`StreamingModule` interface](#streaming-module-interface-issue-62) for their target language (Python Protocol, TypeScript interface + marker, Rust trait). The interface includes a `stream()` method returning an async iterator/generator of output chunks.
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
    Streaming modules in TypeScript MUST satisfy the [`StreamingModule` interface](#streaming-module-interface-issue-62) (including the `STREAMING_MARKER` Symbol property).
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
- On success: `AsyncIterator[dict]`/`AsyncIterable<Record<string,unknown>>`/`Stream<Value>` — lazy sequence of partial output objects.
- **Normative rule (D-19):** Every chunk MUST be an object (JSON object / Python dict / TS Record). SDKs MUST validate each chunk's shape *before delivering it* and, on a non-object chunk (array, string, number, boolean, null), MUST reject it — the invalid chunk MUST NOT be yielded to the consumer. The error raised is `InvalidInputError` with `code=GENERAL_INVALID_INPUT` and `details.code = STREAM_CHUNK_NOT_OBJECT` (plus `details.chunk_index` and `details.actual_type`, the latter being the JSON type name of the offending chunk).

### Properties
- async: true (streaming MUST be async in all SDK languages)
- thread_safe: false (a stream instance MUST NOT be shared across concurrent consumers)
- pure: false (may hold open connections or file handles)
- idempotent: false
