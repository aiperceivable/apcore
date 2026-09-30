---
description: "Cookbook: a module that yields partial output chunks, consumed with client.stream(); the executor deep-merges the chunks into the final result."
---

# Cookbook — Streaming Modules

> **Type:** User cookbook. **Normative spec:** [PROTOCOL_SPEC §5](../spec/protocol-spec.md#5-module-specification) (streaming execution protocol). Feature reference: [features/streaming.md](../features/streaming.md).

End-to-end recipe: a module that emits **partial output chunks** as it works (producer side, a `stream()` method) and a caller that reads them as they arrive (consumer side, `client.stream()`). The executor merges the chunks into one final result.

## When to use this pattern

- A module produces output incrementally — LLM tokens, search hits, progress — and callers should see it as it arrives.
- The final result is the deep merge of all chunks.
- Middleware and output validation should apply once around the whole stream, not per chunk.

## When NOT to use this pattern

- Fire-and-forget notifications: use framework events ([features/event-system.md](../features/event-system.md)).
- Independent items with no merged result: return them as an array from a single `call()`.

## How a streamed call runs

`client.stream()` runs the same pipeline steps as `call()` up to input validation, then:

1. Pulls chunks from the module's `stream()` one at a time and hands each to the caller as soon as it is produced. The generator only advances when the caller asks for the next chunk.
2. Merges each chunk into an accumulator (recursive deep merge, see section 3).
3. After the last chunk, runs output validation and the `after` middleware on the merged result.

The chunks have already been delivered by step 3, so a failure there does **not** raise to the caller: the SDK logs a warning and, when events are enabled, emits `apcore.stream.post_validation_failed`. An error raised by the module mid-stream goes through `on_error` middleware as it does for `call()`.

---

## 1. The module (chunk producer)

A streaming module is a class (Python, TypeScript) or struct (Rust) with both `execute()` — the non-streaming fallback — and `stream()`, registered with `client.register(module_id, instance)`. The `@client.module` / `client.module({...})` shortcuts register single-output modules only.

=== "Python"
    ```python
    import asyncio
    from collections.abc import AsyncIterator
    from typing import Any

    from apcore import APCore
    from apcore.context import Context

    CORPUS = ["apcore spec", "apcore sdk", "apcore guides", "other"]


    class SearchStream:
        description = "Search a corpus and stream hits as they are found"
        input_schema = {
            "type": "object",
            "properties": {"query": {"type": "string"}},
            "required": ["query"],
        }
        output_schema = {
            "type": "object",
            "properties": {
                "hits": {"type": "array", "items": {"type": "string"}},
                "total": {"type": "integer"},
            },
        }

        def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
            hits = [doc for doc in CORPUS if inputs["query"] in doc]
            return {"hits": hits, "total": len(hits)}

        async def stream(self, inputs: dict[str, Any], context: Context) -> AsyncIterator[dict[str, Any]]:
            running: list[str] = []
            for doc in CORPUS:
                if context.cancel_token:
                    context.cancel_token.check()  # raises ExecutionCancelledError
                await asyncio.sleep(0.01)  # simulate search latency
                if inputs["query"] in doc:
                    running.append(doc)
                    # Arrays are replaced on merge, so re-emit the full list.
                    yield {"hits": list(running), "total": len(running)}


    client = APCore()
    client.register("demo.search_stream", SearchStream())
    ```

=== "TypeScript"
    ```typescript
    import { Type } from '@sinclair/typebox';
    import { APCore, Context } from 'apcore-js';

    const CORPUS = ['apcore spec', 'apcore sdk', 'apcore guides', 'other'];

    const searchStream = {
      description: 'Search a corpus and stream hits as they are found',
      inputSchema: Type.Object({ query: Type.String() }),
      outputSchema: Type.Object({
        hits: Type.Array(Type.String()),
        total: Type.Integer(),
      }),

      async execute(inputs: Record<string, unknown>, _context: Context) {
        const hits = CORPUS.filter((doc) => doc.includes(inputs.query as string));
        return { hits, total: hits.length };
      },

      async *stream(inputs: Record<string, unknown>, context: Context): AsyncGenerator<Record<string, unknown>> {
        const running: string[] = [];
        for (const doc of CORPUS) {
          context.cancelToken?.check(); // throws ExecutionCancelledError
          await new Promise((resolve) => setTimeout(resolve, 10)); // simulate search latency
          if (doc.includes(inputs.query as string)) {
            running.push(doc);
            // Arrays are replaced on merge, so re-emit the full list.
            yield { hits: [...running], total: running.length };
          }
        }
      },
    };

    const client = new APCore();
    client.register('demo.search_stream', searchStream);
    ```

=== "Rust"
    ```rust
    // Cargo.toml: apcore, async-stream, async-trait, serde_json, tokio
    use apcore::{APCore, ChunkStream, Context, Module, ModuleError};
    use async_stream::stream;
    use async_trait::async_trait;
    use serde_json::{json, Value};
    use std::time::Duration;

    const CORPUS: [&str; 4] = ["apcore spec", "apcore sdk", "apcore guides", "other"];

    struct SearchStream;

    #[async_trait]
    impl Module for SearchStream {
        fn input_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {"query": {"type": "string"}},
                "required": ["query"]
            })
        }

        fn output_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {
                    "hits": {"type": "array", "items": {"type": "string"}},
                    "total": {"type": "integer"}
                }
            })
        }

        fn description(&self) -> &str {
            "Search a corpus and stream hits as they are found"
        }

        async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
            let query = inputs["query"].as_str().unwrap_or_default();
            let hits: Vec<&str> = CORPUS.iter().copied().filter(|doc| doc.contains(query)).collect();
            Ok(json!({"hits": hits, "total": hits.len()}))
        }

        // Return Some(stream) to stream; the default None falls back to execute().
        fn stream(&self, inputs: Value, ctx: &Context<Value>) -> Option<ChunkStream> {
            let query = inputs["query"].as_str().unwrap_or_default().to_string();
            let cancel_token = ctx.cancel_token.clone();
            Some(Box::pin(stream! {
                let mut running: Vec<String> = Vec::new();
                for doc in CORPUS {
                    if let Some(token) = &cancel_token {
                        if let Err(e) = token.check() {
                            yield Err(ModuleError::from(e));
                            return;
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await; // simulate search latency
                    if doc.contains(query.as_str()) {
                        running.push(doc.to_string());
                        // Arrays are replaced on merge, so re-emit the full list.
                        yield Ok(json!({"hits": running.clone(), "total": running.len()}));
                    }
                }
            }))
        }
    }

    fn build_client() -> Result<APCore, ModuleError> {
        let client = APCore::new();
        client.register("demo.search_stream", Box::new(SearchStream))?;
        Ok(client)
    }
    ```

## 2. The caller (chunk consumer)

Continues the file from section 1.

=== "Python"
    ```python
    async def main() -> None:
        async for chunk in client.stream("demo.search_stream", {"query": "apcore"}):
            print(f"hit count: {chunk['total']}, hits: {chunk['hits']}")


    asyncio.run(main())
    ```

=== "TypeScript"
    ```typescript
    for await (const chunk of client.stream('demo.search_stream', { query: 'apcore' })) {
      console.log(`hit count: ${chunk.total}, hits: ${JSON.stringify(chunk.hits)}`);
    }
    ```

=== "Rust"
    ```rust
    use futures_util::StreamExt; // Cargo.toml: futures-util

    #[tokio::main]
    async fn main() -> Result<(), ModuleError> {
        let client = build_client()?;
        // stream() is not async; it returns the chunk stream directly.
        let mut chunks = client.stream("demo.search_stream", json!({"query": "apcore"}), None, None);
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk?;
            println!("hit count: {}, hits: {}", chunk["total"], chunk["hits"]);
        }
        Ok(())
    }
    ```

Each run prints three chunks; the third is `{"hits": ["apcore spec", "apcore sdk", "apcore guides"], "total": 3}`.

## 3. How chunks merge

The accumulator starts as `{}` and each chunk is merged into it recursively:

| Chunk sets a key to… | Result |
|----------------------|--------|
| a primitive | overwrites the previous value |
| an object, where the previous value is an object | merges key by key (recursively) |
| an array | **replaces** the previous array — arrays are never concatenated |
| `null` | overwrites with `null` (the key is not deleted) |

Recursion stops at `stream.max_merge_depth` levels (default 32, set in `apcore.yaml`). At that depth the chunk's value replaces the previous one wholesale instead of being merged further. The algorithm is [A24 in algorithms.md](../spec/algorithms.md#a24-stream-chunk-aggregation).

To accumulate a growing list, re-emit the full list in every chunk (as the example does) or give each item its own key.

## 4. Pitfalls

| Pitfall | Symptom | Fix |
|---------|---------|-----|
| Yielding only the new items of an array | Merged result holds only the last chunk's items | Re-emit the full running list each chunk |
| A chunk that is not an object | Stream fails with `GENERAL_INVALID_INPUT` (`details.code = STREAM_CHUNK_NOT_OBJECT`) | Yield objects only |
| Merged result does not match `output_schema` | No exception — a warning is logged and `apcore.stream.post_validation_failed` is emitted | Make the final merged object valid; validate chunks inside `stream()` if consumers need per-chunk guarantees |
| Consumer stops reading early | The generator is abandoned at its current `yield`; output validation and `after` middleware do not run | Put cleanup in a `finally` block in `stream()`; pass a `CancelToken` if the producer does work between yields ([Cooperative Cancellation](./cookbook-cancellation.md)) |
| Not driving the iterator | No chunks are produced | `client.stream()` returns an iterator — consume it with `async for` / `for await` / `.next().await` |

## 5. Configuration

Streaming needs no configuration. The one related key is `stream.max_merge_depth`. With tracing enabled (`observability.tracing.*`, see [Cookbook — Tracing and Redacted Logs](./cookbook-observability.md)) a streamed call produces one `apcore.module.execute` span covering the whole stream; there are no per-chunk spans.

---

## See also

- [features/streaming.md](../features/streaming.md) — streaming reference
- [Cookbook — Cooperative Cancellation](./cookbook-cancellation.md) — stopping a producer early
- [PROTOCOL_SPEC §5](../spec/protocol-spec.md#5-module-specification) — module specification, including streaming
