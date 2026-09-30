---
description: "Incremental chunk-based output via the statically-detectable StreamingModule interface (stream() returns an async iterator); executor's three-phase pipeline splits emit from validation."
---

# Streaming Support

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §5 Module Specification (streaming hooks).


## Overview

A streaming module produces its output as a sequence of chunks instead of one response — useful for modules that wrap LLM APIs, process large datasets, or transform data in real time. The executor's `stream()` runs a three-phase pipeline that delivers chunks to the caller as they are produced, while still running output validation and after-middleware on the combined result.

## Streaming Module Interface

Each SDK exposes a language-idiomatic `StreamingModule` interface so that adapter and bridge code (e.g. `apcore-mcp`, `apcore-a2a`) can detect streaming support without guessing from method names.

=== "Python"
    `apcore.StreamingModule` is a `@runtime_checkable` Protocol: an async-generator `stream(inputs, context)` yielding `dict` chunks.

    ```python
    from collections.abc import AsyncIterator

    from pydantic import BaseModel

    from apcore import Context, ModuleAnnotations, StreamingModule


    class ChatInput(BaseModel):
        prompt: str


    class ChatChunk(BaseModel):
        content: str
        done: bool


    class ChatModule:
        input_schema = ChatInput
        output_schema = ChatChunk
        description = "Stream chat completions"
        annotations = ModuleAnnotations(streaming=True)

        async def execute(self, inputs: dict, context: Context) -> dict:
            # Used when a caller does not stream.
            return {"content": f"echo:{inputs['prompt']}", "done": True}

        async def stream(self, inputs: dict, context: Context) -> AsyncIterator[dict]:
            for token in ("echo:", inputs["prompt"]):
                yield {"content": token, "done": False}
            yield {"content": "", "done": True}


    assert isinstance(ChatModule(), StreamingModule)
    ```

    `isinstance` checks only that `stream` exists. When a module declares `annotations.streaming = True`, registration also checks the signature (arity, async generator) and raises `StreamingInterfaceError` (`STREAMING_INTERFACE_MISMATCH`) if it does not match. The `@module` decorator wraps plain functions only; a streaming module is a class.

=== "TypeScript"
    TypeScript interfaces are structural, so a streaming module carries the `STREAMING_MARKER` symbol property; `isStreamingModule()` narrows the type.

    ```typescript
    import { Type } from "@sinclair/typebox";
    import { STREAMING_MARKER, isStreamingModule } from "apcore-js";
    import type { Context, Module, StreamingModule } from "apcore-js";

    class ChatModule implements StreamingModule {
      readonly [STREAMING_MARKER] = true as const;
      inputSchema = Type.Object({ prompt: Type.String() });
      outputSchema = Type.Object({ content: Type.String(), done: Type.Boolean() });
      description = "Stream chat completions";
      annotations = { streaming: true };

      async execute(inputs: Record<string, unknown>, _context: Context): Promise<Record<string, unknown>> {
        // Used when a caller does not stream.
        return { content: `echo:${String(inputs.prompt)}`, done: true };
      }

      async *stream(inputs: Record<string, unknown>, _context: Context): AsyncGenerator<Record<string, unknown>> {
        for (const token of ["echo:", String(inputs.prompt)]) {
          yield { content: token, done: false };
        }
        yield { content: "", done: true };
      }
    }

    const mod: Module = new ChatModule();
    console.log(isStreamingModule(mod)); // true
    ```

    A module that has a `stream()` method but no marker is still detected, with a one-time deprecation warning per module; add the marker to silence it.

=== "Rust"
    The base `Module` trait's `stream()` returns `Option<ChunkStream>` (`None` means "not streaming — fall back to `execute()`"). A streaming module also implements the `StreamingModule` trait (`stream_typed`) and returns itself from `Module::as_streaming()`, which gives adapter code a typed handle.

    ```rust
    use apcore::{ChunkStream, Context, Module, ModuleAnnotations, ModuleError, StreamingModule};
    use async_trait::async_trait;
    use serde_json::{json, Value};

    struct ChatModule;

    impl ChatModule {
        fn chunks(prompt: String) -> ChunkStream {
            Box::pin(async_stream::stream! {
                for token in ["echo:".to_string(), prompt] {
                    yield Ok(json!({ "content": token, "done": false }));
                }
                yield Ok(json!({ "content": "", "done": true }));
            })
        }
    }

    #[async_trait]
    impl Module for ChatModule {
        fn description(&self) -> &str { "Stream chat completions" }
        fn input_schema(&self) -> Value {
            json!({ "type": "object", "properties": { "prompt": { "type": "string" } }, "required": ["prompt"] })
        }
        fn output_schema(&self) -> Value {
            json!({ "type": "object", "properties": { "content": { "type": "string" }, "done": { "type": "boolean" } } })
        }
        fn annotations(&self) -> ModuleAnnotations {
            ModuleAnnotations { streaming: true, ..ModuleAnnotations::default() }
        }

        async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
            // Used when a caller does not stream.
            Ok(json!({ "content": format!("echo:{}", inputs["prompt"].as_str().unwrap_or("")), "done": true }))
        }

        fn stream(&self, inputs: Value, _ctx: &Context<Value>) -> Option<ChunkStream> {
            Some(Self::chunks(inputs["prompt"].as_str().unwrap_or("").to_string()))
        }

        fn as_streaming(&self) -> Option<&dyn StreamingModule> {
            Some(self)
        }
    }

    impl StreamingModule for ChatModule {
        fn stream_typed(&self, inputs: Value, _context: &Context<Value>) -> ChunkStream {
            Self::chunks(inputs["prompt"].as_str().unwrap_or("").to_string())
        }
    }
    ```

    `as_streaming()` and `Module::stream()` must agree: both `Some` or both `None`. Registering a module whose `annotations.streaming` is `true` while `as_streaming()` returns `None` fails with `STREAMING_INTERFACE_MISMATCH`.

### Rules

- SDKs export the streaming interface (`StreamingModule` / `STREAMING_MARKER` + `isStreamingModule` / `Module::as_streaming`) so adapter code can rely on it.
- Adapter and bridge code detects streaming through that interface (`isinstance`, `isStreamingModule`, `as_streaming`), not through `hasattr` / `typeof` checks on a method named `stream`.
- A module that declares `annotations.streaming = true` but whose `stream()` does not satisfy the interface is rejected at registration with `StreamingInterfaceError` / `STREAMING_INTERFACE_MISMATCH` (fields: `module_id`, `expected_signature`, `actual_signature`, `mismatch_reason`), not at the first call.
- A non-streaming module may have an unrelated method named `stream`; it is treated as streaming only if it satisfies the interface.

## Requirements

- Streaming modules satisfy the [Streaming Module Interface](#streaming-module-interface) for their language.
- The executor's `stream()` returns an async iterator that yields chunks as the module produces them.
- Every chunk is an object; chunks are combined with a depth-capped deep merge into the output that is validated.
- Output validation and after-middleware run on the combined output after the last chunk, not on individual chunks.
- A module without streaming support is called through `execute()` and its result is yielded as a single chunk.
- Streaming modules should set the `streaming` annotation so agents can discover them.

## Three-phase streaming pipeline

**Phase 1 — pipeline setup.** Steps 1–7 of the standard pipeline run as for `call()`: context creation, call-chain guard, module lookup, ACL, approval gate, middleware before, input validation. At step 8 the executor calls the module's `stream()` instead of `execute()`.

**Phase 2 — chunk emission.** The executor iterates the module's stream and yields each chunk to the caller immediately, while deep-merging it into the accumulated output. A chunk that is not an object is rejected before delivery (see [Contract: Module.stream](#contract-modulestream)). The execution timeout is checked between chunks.

**Phase 3 — post-processing.** After the last chunk, output validation (step 9) and middleware after (step 10) run on the accumulated output. A failure here cannot recall chunks already delivered: it is logged and reported as an event instead of being raised to the caller.

## Consuming streams

=== "Python"
    ```python
    import asyncio

    from apcore import APCore


    async def main() -> None:
        client = APCore()
        client.register("llm.chat", ChatModule())  # ChatModule from the example above
        async for chunk in client.stream("llm.chat", {"prompt": "Hello"}):
            print(chunk["content"], end="", flush=True)


    asyncio.run(main())
    ```
=== "TypeScript"
    ```typescript
    import { APCore } from "apcore-js";

    const client = new APCore();
    client.register("llm.chat", new ChatModule()); // ChatModule from the example above

    for await (const chunk of client.stream("llm.chat", { prompt: "Hello" })) {
      process.stdout.write(String(chunk.content));
    }
    ```
=== "Rust"
    ```rust
    use apcore::{APCore, ModuleError};
    use futures::StreamExt;
    use serde_json::json;

    #[tokio::main]
    async fn main() -> Result<(), ModuleError> {
        let client = APCore::new();
        client.register("llm.chat", Box::new(ChatModule))?; // ChatModule from the example above

        // stream() returns a Stream of Result<Value, ModuleError>; it is not awaited.
        let mut chunks = client.stream("llm.chat", json!({ "prompt": "Hello" }), None, None);
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk?;
            print!("{}", chunk["content"].as_str().unwrap_or(""));
        }
        Ok(())
    }
    ```

## Deep merge

Chunks are combined with a recursive deep merge (Algorithm A24):

| Left value | Right value | Result |
|-----------|-------------|--------|
| object | object | merged recursively |
| anything else | anything else | right value wins — arrays are replaced, not concatenated |

Recursion stops at `stream.max_merge_depth` levels (default 32); at the cap the right value replaces the left without further merging.

```text
Chunk 1: {"content": "Hello", "metadata": {"tokens": 1}}
Chunk 2: {"content": " world", "metadata": {"tokens": 1, "model": "gpt-4"}}
Merged:  {"content": " world", "metadata": {"tokens": 1, "model": "gpt-4"}}
```

!!! note
    Strings are not concatenated. A module that wants the validated output to contain the full text should yield the growing string, or put the complete text in its final chunk.

## Fallback behaviour

When the executor's `stream()` is called for a module without streaming support, it runs `execute()` and yields the result as a single chunk; no merge is needed.

## Streaming annotation

```yaml
annotations:
  streaming: true
```

The annotation lets AI agents discover streamable modules and lets schema export advertise the capability. It also opts the module into the registration-time interface check above.

## Dependencies

- **Core Executor** — runs the three-phase pipeline.
- **Middleware System** — before-middleware runs in phase 1, after-middleware in phase 3.
- **Schema System** — output validation runs on the accumulated output in phase 3.
- **Cancellation** — a module can check its cancel token between chunks for cooperative cancellation.

??? info "Python SDK reference"
    Not a protocol requirement — the relevant `apcore-python` source files.

    | File | Purpose |
    |------|---------|
    | `src/apcore/streaming.py` | `StreamingModule` Protocol |
    | `src/apcore/executor.py` | `Executor.stream()` three-phase pipeline and the depth-capped deep merge |

## Testing strategy

- **Basic streaming** — chunks arrive in order; the accumulated output is correct.
- **Deep merge** — recursive object merge, array replacement, scalar replacement, depth cap (`conformance/fixtures/stream_aggregation.json`).
- **Fallback** — a non-streaming module produces a single-chunk stream.
- **Pipeline integration** — before-middleware runs before the first chunk; after-middleware runs after the last.
- **Validation** — output validation runs on the accumulated output; a phase-3 failure does not affect chunks already delivered.
- **Non-object chunk** — rejected with `STREAM_CHUNK_NOT_OBJECT` and never delivered.
- **Cancellation** — cancelling mid-stream stops emission and raises `ExecutionCancelledError`.

## Contract: Module.stream

### Inputs
- `inputs` (dict/object/Value, required) — already validated against the module's `input_schema`
- `context` (Context, required) — execution context

### Errors
- An error raised by the module mid-stream ends the stream and is propagated to the consumer.
- A non-object chunk (array, string, number, boolean, null) is rejected before it is delivered, with `InvalidInputError` (`GENERAL_INVALID_INPUT`) whose `details.code` is `STREAM_CHUNK_NOT_OBJECT`, plus `details.chunk_index` and `details.actual_type` (the JSON type name) (D-58).

### Returns
- `AsyncIterator[dict]` / `AsyncGenerator<Record<string, unknown>>` / `ChunkStream` (a `Stream` of `Result<Value, ModuleError>`) — a lazy sequence of partial output objects.

### Properties
- async: true
- thread_safe: false (a stream instance is not shared across concurrent consumers)
- pure: false (may hold open connections or file handles)
- idempotent: false
