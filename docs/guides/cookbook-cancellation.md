# Cookbook — Cooperative Cancellation

> **Type:** User cookbook. **Normative spec:** [PROTOCOL_SPEC](../../PROTOCOL_SPEC.md) §5 Module Specification (cancellation hooks). Feature reference: [features/cancellation.md](../features/cancellation.md).

End-to-end example: cancel a long-running module mid-flight from the caller side, with proper cleanup. The example uses the same `demo.slow_task` module across all three SDKs (Python / TypeScript / Rust) so behaviour is directly comparable. All snippets are derived from the SDK example files (`examples/cancel_token.{py,ts,rs}`) — they run as-is.

## When to use this pattern

- A module performs work in a loop or in chunks and you want to interrupt it from an external trigger (timeout, user action, SIGINT, downstream failure).
- You need cleanup to run before the call returns — `ExecutionCancelledError` propagates through middleware `on_error` hooks just like any other error.
- You want the executor to **not** call subsequent middleware `before` hooks once cancellation fires.

## When NOT to use this pattern

- For wallclock timeouts on every call: use `policy.timeout_ms` in `apcore.yaml`. The framework enforces it without per-module code.
- For aborting work that is blocked in synchronous I/O without a way to poll a token: cancellation is **cooperative**. A module that never checks `context.cancel_token` cannot be cancelled.

---

## 1. The Module (cancellation-aware)

=== "Python"
    ```python
    from apcore import APCore
    from apcore.context import Context

    client = APCore()

    @client.module(id="demo.slow_task", description="Simulates a long-running task")
    def slow_task(steps: int, context: Context) -> dict:
        completed = 0
        for i in range(steps):
            # Check before each iteration. CancelToken.check() raises
            # ExecutionCancelledError if the token has been cancelled.
            if context.cancel_token:
                context.cancel_token.check()
            time.sleep(0.05)  # simulate work
            completed += 1
        return {"completed": completed}
    ```

=== "TypeScript"
    ```typescript
    import { Type } from '@sinclair/typebox';
    import { APCore, Context } from 'apcore-js';

    const client = new APCore();

    client.module({
      id: 'demo.slow_task',
      description: 'Simulates a long-running task',
      inputSchema: Type.Object({ steps: Type.Number() }),
      outputSchema: Type.Object({ completed: Type.Number() }),
      execute: async (inputs, context: Context) => {
        const steps = inputs.steps as number;
        let completed = 0;
        for (let i = 0; i < steps; i++) {
          context.cancelToken?.check();          // throws on cancel
          await new Promise((r) => setTimeout(r, 50));
          completed++;
        }
        return { completed };
      },
    });
    ```

=== "Rust"
    ```rust
    use apcore::{APCore, Context};
    use serde_json::{json, Value};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::time::sleep;

    // client.module() takes positional arguments: the closure is the handler.
    // Cancellation is exposed as the field `ctx.cancel_token: Option<CancelToken>`
    // (NOT a method); call .check() on the inner token to raise on cancel.
    let mut client = APCore::new();
    client.module(
        "demo.slow_task",
        "Simulates a long-running task",
        json!({"type":"object","properties":{"steps":{"type":"integer"}},"required":["steps"]}),
        json!({"type":"object","properties":{"completed":{"type":"integer"}}}),
        None,             // documentation
        vec![],           // tags
        None,             // version
        None,             // metadata
        vec![],           // examples
        None,             // display
        |inputs: Value, ctx: &Context<Value>| {
            let cancel_token = ctx.cancel_token.clone();
            Box::pin(async move {
                let steps = inputs["steps"].as_i64().unwrap();
                let mut completed = 0;
                for _ in 0..steps {
                    if let Some(ref t) = cancel_token { t.check()?; }
                    sleep(Duration::from_millis(50)).await;
                    completed += 1;
                }
                Ok(json!({"completed": completed}))
            })
        },
    )?;
    ```

## 2. The Caller (firing cancellation)

=== "Python"
    ```python
    import threading
    from apcore.cancel import CancelToken, ExecutionCancelledError
    from apcore.context import Context

    token = CancelToken()
    ctx = Context.create()
    ctx.cancel_token = token

    # Fire cancellation from a background thread after 80ms
    timer = threading.Timer(0.08, token.cancel)
    timer.start()

    try:
        client.call("demo.slow_task", {"steps": 10}, ctx)
    except ExecutionCancelledError as e:
        print(f"Cancelled: {e}")          # expected on time-out
    finally:
        timer.cancel()                    # clean up if completion beat the timer
    ```

=== "TypeScript"
    ```typescript
    import { CancelToken, Context, ExecutionCancelledError } from 'apcore-js';
    import { v4 as uuidv4 } from 'uuid';

    const token = new CancelToken();
    const ctx = new Context(
      uuidv4().replace(/-/g, ''), null, [], null, null, null, {}, token
    );

    setTimeout(() => token.cancel(), 80);

    try {
      await client.call('demo.slow_task', { steps: 10 }, ctx);
    } catch (e) {
      if (e instanceof ExecutionCancelledError) console.log(`Cancelled: ${e.message}`);
      else throw e;
    }
    ```

=== "Rust"
    ```rust
    use apcore::cancel::CancelToken;
    use apcore::context::{Context, Identity};
    use apcore::errors::ErrorCode;
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::time::Duration;

    let token = CancelToken::new();   // CancelToken: Clone — share by clone(), no Arc needed.

    let identity = Identity::new("user".into(), "user".into(), vec![], HashMap::new());
    let mut ctx: Context<Value> = Context::new(identity);
    ctx.cancel_token = Some(token.clone());   // public field — no setter

    let token_for_timer = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(80)).await;
        token_for_timer.cancel();
    });

    match client.call("demo.slow_task", json!({"steps": 10}), Some(ctx)).await {
        Err(e) if e.code == ErrorCode::ExecutionCancelled => println!("Cancelled: {e}"),
        other => { other?; }
    };
    ```

## 3. Pitfalls

| Pitfall | Symptom | Fix |
|---------|---------|-----|
| Module never checks the token | Cancel signal fires but `execute()` runs to completion | Add `context.cancel_token?.check()` (or equivalent) in every loop iteration |
| Token cancelled outside the call's lifetime | Subsequent calls with the same token fail immediately | Build a fresh `CancelToken` per call, or call `token.reset()` between calls |
| `try / finally` cleanup races the timer | `RuntimeError: cannot cancel a finished timer` | `timer.cancel()` in `finally` is idempotent in Python; in TS use `clearTimeout` only if you stored the timer id |
| Catching `Exception` instead of `ExecutionCancelledError` | Cancellation looks like a generic failure to upstream callers | Catch the specific error class and re-raise / re-throw if the caller cares |
| Combining cancellation with retries | Retries can re-trigger the cancelled work | Have your retry middleware bail out on `ExecutionCancelledError` (or set `retryable=False` on it — already the default) |

## 4. Verifying behaviour

The conformance fixtures don't have a dedicated cancellation suite (cancellation is a host-runtime concern, not a wire-protocol one), but the [pipeline_step_middleware](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/pipeline_step_middleware.json) fixture covers the error-propagation paths that cancellation rides on.

For a concrete cross-SDK behaviour check:

```bash
# Run the example in each SDK and compare timing
python apcore-python/examples/cancel_token.py
node --loader=tsx apcore-typescript/examples/cancel-token.ts
cargo run --example cancel_token --manifest-path apcore-rust/Cargo.toml
```

All three should print `completed: 3` for Run 1 and a `Cancelled` message after roughly 80ms ± 30ms for Run 2.

---

## See also

- [features/cancellation.md](../features/cancellation.md) — the feature reference
- [features/middleware-system.md](../features/middleware-system.md) — how `on_error` interacts with cancellation
- [PROTOCOL_SPEC §8.7](../../PROTOCOL_SPEC.md#8-error-handling-specification) — `EXECUTION_CANCELLED` error code
