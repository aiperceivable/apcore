# Cookbook — Streaming Modules

> **Type:** User cookbook. **Normative spec:** [PROTOCOL_SPEC](../../PROTOCOL_SPEC.md) §5 Module Specification (streaming hooks). Feature reference: [features/streaming.md](../features/streaming.md).

End-to-end example: a module that emits **partial output chunks** as it works, with the executor performing recursive deep-merge to assemble the final result. Demonstrates both the producer side (`stream()` generator) and the consumer side (`client.stream()` async iterator).

## When to use this pattern

- A module produces output incrementally — LLM tokens, file uploads, search hits — and you want callers to display progress as it arrives.
- Final output is the **deep-merge** of every chunk; intermediate chunks are not stored separately.
- You want middleware (`before` / `after`) and schema validation to apply **once around the stream as a whole**, not per chunk.

## When NOT to use this pattern

- For a fire-and-forget event stream: emit framework events ([features/event-system.md](../features/event-system.md)) instead.
- When chunks are unrelated items (a list, not a merge): emit a single `call()` result containing the array.
- When the consumer needs back-pressure: streaming has no built-in flow control — the producer runs at its own pace and chunks queue in memory.

## Pipeline coverage

`client.stream()` runs Steps 1–7 of the pipeline (identical to `call()` up through Input Validation), then for each chunk yielded by the module the executor:

1. Accumulates via recursive deep-merge (depth cap 32).
2. Emits a per-chunk event for observability.
3. After the generator exhausts, runs Output Validation (Step 9) on the final accumulated dict, then Middleware After (Step 10).

If the module raises mid-stream, `on_error` middleware runs as usual.

---

## 1. The Module (chunk producer)

=== "Python"
    ```python
    from typing import AsyncIterator, Any
    from apcore import APCore
    from apcore.context import Context

    client = APCore()

    @client.module(
        id="demo.search_stream",
        description="Search a corpus and stream hits as they're found",
        input_schema={"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]},
        output_schema={
            "type": "object",
            "properties": {
                "hits": {"type": "array", "items": {"type": "string"}},
                "total": {"type": "integer"},
            },
        },
    )
    async def search_stream(query: str, context: Context) -> AsyncIterator[dict[str, Any]]:
        # First chunk: announce we started
        yield {"hits": [], "total": 0}

        for i, hit in enumerate(await fake_corpus_search(query)):
            # Cooperative cancellation — see cookbook-cancellation
            if context.cancel_token: context.cancel_token.check()

            # Each chunk overlays onto the accumulator. Arrays REPLACE,
            # so we re-emit the running total list each time.
            running = getattr(context.data, "_demo_hits", [])
            running.append(hit)
            context.data["_demo_hits"] = running

            yield {"hits": list(running), "total": len(running)}
    ```

=== "TypeScript"
    ```typescript
    import { Type } from '@sinclair/typebox';
    import { APCore, Context } from 'apcore-js';

    const client = new APCore();

    client.module({
      id: 'demo.search_stream',
      description: 'Search a corpus and stream hits as they\'re found',
      inputSchema: Type.Object({ query: Type.String() }),
      outputSchema: Type.Object({
        hits: Type.Array(Type.String()),
        total: Type.Integer(),
      }),
      execute: async function* (inputs, context: Context) {
        yield { hits: [], total: 0 };

        const running: string[] = [];
        for (const hit of await fakeCorpusSearch(inputs.query as string)) {
          context.cancelToken?.check();
          running.push(hit);
          yield { hits: [...running], total: running.length };
        }
      },
    });
    ```

=== "Rust"
    ```rust
    use apcore::{APCore, Context, StreamModule};
    use async_stream::try_stream;
    use serde_json::json;

    let mut client = APCore::new();
    client.module()
        .id("demo.search_stream")
        .description("Search a corpus and stream hits as they're found")
        .input_schema(json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}))
        .output_schema(json!({"type":"object","properties":{
            "hits":{"type":"array","items":{"type":"string"}},
            "total":{"type":"integer"}
        }}))
        .stream(|inputs, ctx: Context| {
            try_stream! {
                yield json!({"hits": [], "total": 0});
                let mut running: Vec<String> = vec![];
                for hit in fake_corpus_search(inputs["query"].as_str().unwrap()).await? {
                    if let Some(t) = ctx.cancel_token() { t.check()?; }
                    running.push(hit);
                    yield json!({"hits": running, "total": running.len()});
                }
            }
        })
        .register();
    ```

## 2. The Caller (chunk consumer)

=== "Python"
    ```python
    async for chunk in client.stream("demo.search_stream", {"query": "apcore"}):
        print(f"hit count: {chunk['total']}, last 3: {chunk['hits'][-3:]}")
    ```

=== "TypeScript"
    ```typescript
    for await (const chunk of client.stream('demo.search_stream', { query: 'apcore' })) {
      console.log(`hit count: ${chunk.total}, last 3:`, chunk.hits.slice(-3));
    }
    ```

=== "Rust"
    ```rust
    use futures_util::StreamExt;

    let mut s = client.stream("demo.search_stream", json!({"query": "apcore"}), None).await?;
    while let Some(chunk) = s.next().await {
        let chunk = chunk?;
        println!("hit count: {}, last 3: {:?}",
            chunk["total"], &chunk["hits"].as_array().unwrap().iter().rev().take(3).collect::<Vec<_>>());
    }
    ```

## 3. Deep-merge semantics — what gets retained

The accumulator starts as `{}` and each chunk is **recursively merged** (depth cap 32):

| Chunk shape | Behaviour |
|-------------|-----------|
| Object key set to a primitive | Overwrites the previous value |
| Object key set to an object | Recurses into the existing object and merges keys |
| Object key set to an array | **Replaces** the previous array (arrays are NOT concatenated) |
| Object key set to `null` | Overwrites with `null` (does NOT delete the key) |
| Nested depth > 32 | Truncates at 32 — log warning, don't rely on deeper structure |

Verified by [conformance fixture `stream_aggregation`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/stream_aggregation.json) (9 cases).

**Practical consequence:** to accumulate a growing list, re-emit the full list in each chunk (as the example does), or place each item under a unique key.

## 4. Pitfalls

| Pitfall | Symptom | Fix |
|---------|---------|-----|
| Yielding only deltas in arrays | Final result has only the LAST chunk's array | Re-emit the full running list each chunk |
| Output validation fails on the partial chunks | `SCHEMA_VALIDATION_ERROR` after stream completes | The schema validates the **final accumulated** object — mark required-only-at-end fields with `default` or have the last chunk supply them |
| Middleware `before` runs once but tries to mutate per-chunk state | State leaks across chunks | Move per-chunk logic into the module body; `before` is for one-time setup |
| Caller iterates partially then breaks | Module generator runs to GC | Producer should be cancellation-aware; pass a CancelToken and `cancel()` on early exit |
| Forgetting `async for` / `for await` / `.next()` | Stream returns but no chunks consumed | Streaming returns an iterator, not a value; you must drive it |

## 5. Wiring it in `apcore.yaml`

Streaming requires no special configuration — the same module file system applies. To enable per-chunk OTel spans, configure observability:

```yaml
# apcore.yaml — extract
obs:
  otel:
    enabled: true
    service_name: my-app
    # Each emitted chunk creates a span "module.stream.chunk"
    # under the parent "module.call" span.
```

---

## See also

- [features/streaming.md](../features/streaming.md) — feature reference
- [cookbook-cancellation.md](./cookbook-cancellation.md) — cooperative cancellation, especially relevant for early-exit consumers
- [conformance fixture `stream_aggregation`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/stream_aggregation.json) — 9 deep-merge edge cases
- [PROTOCOL_SPEC §5](../../PROTOCOL_SPEC.md#5-module-specification) — module specification including streaming
