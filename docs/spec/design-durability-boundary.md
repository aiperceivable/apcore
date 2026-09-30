---
description: "apcore's boundary with durable execution: the hooks retry/replay/workflow layers may rely on, what apcore deliberately leaves to them, and how they integrate."
---

# Durability Boundary

apcore governs **single module calls**. **Durable execution** — retry across crashes, replay after restart, deduplication of logically-equivalent invocations, long-running pause/resume, persistent task queues — is a runtime concern layered above it. AI agent runtimes and workflow systems routinely need it; apcore does not implement it.

This page states:

1. **What apcore guarantees** — the stable hooks any retry/replay/workflow layer above apcore MAY rely on.
2. **What apcore deliberately does not do** — so integrators know which concerns they own.
3. **How a downstream layer integrates** — vendor-neutral patterns built only from existing apcore primitives.
4. **Gap watchlist** — decisions deferred until real usage provides evidence.

[protocol-spec.md](./protocol-spec.md) is normative and wins on any conflict.

---

## 1. Position

apcore defines how modules are described, validated, governed, observed, and invoked through an 11-step execution pipeline. It is intentionally:

- **Single-call scoped.** The pipeline operates on one module invocation at a time. apcore has no concept of a "workflow" spanning multiple module calls.
- **Stateless across invocations.** No `PipelineContext` state is preserved between calls. Each call starts a fresh pipeline.
- **Interfaces plus reference implementations.** apcore ships interfaces and in-memory reference behaviour. Production backends (persistent task stores, distributed approval handlers, replay engines) belong to applications, framework adapters, or community packages.

Workflow orchestration — sagas, fan-out/fan-in, multi-step compensation, cross-call state machines, durable pause/resume of arbitrary execution — is outside apcore's scope. See [`POSITIONING.md`](../POSITIONING.md).

---

## 2. Stable hooks (downstream layers MAY rely on these)

The following primitives are part of apcore's normative surface and are stable within a major version under [protocol-spec §13.5](./protocol-spec.md#135-backwardforward-compatibility-matrix).

### 2.1 Context serialization

`Context` serializes to a JSON-compatible object and back (`Context.serialize()` / `Context.deserialize()`); the rules are normative in [protocol-spec §5.7](./protocol-spec.md#57-context-parameter-specification). Key guarantees:

- `trace_id`, `caller_id`, `call_chain`, `identity` round-trip losslessly.
- `data` round-trips for all serializable values; non-serializable values (functions, sockets, connections) are skipped with a logged warning, and keys starting with `_` are never serialized.
- `executor`, `cancel_token`, `services` are runtime injects and **MUST NOT** be serialized.

This is what lets a Context cross a queue, a job board, or a persistent task store and be rehydrated later — the foundation of any cross-process retry/replay layer.

### 2.2 The `_approval_token` pause/resume contract (Approval Phase B)

The approval gate (pipeline Step 5, [protocol-spec §7](./protocol-spec.md#7-approval-system)) is the **only** suspension point in apcore's pipeline. Its Phase B contract:

1. The gate fires when any governance source requires approval — the module's annotations, annotations the registry holds for it (binding file, `*_meta.yaml`, `metadata`), an ACL rule with `approval: required`, or `ExecutionPolicy.gate_destructive` for a destructive module ([protocol-spec §7.4, §6.9](./protocol-spec.md#74-executor-integration-step-5)). It calls the configured `ApprovalHandler`; with no handler the gate is skipped with a warning, unless `ExecutionPolicy(strict = true)` makes it fail closed.
2. If the handler returns `pending`, the executor raises `ApprovalPendingError` carrying an `approval_id`.
3. The caller persists `approval_id` and the original inputs (apcore does not), waits for an out-of-band approval signal, and re-invokes the same module with `_approval_token = approval_id` in the arguments.
4. On resume the executor re-enters the pipeline **from Step 1**. The gate removes `_approval_token` from the arguments and calls `handler.check_approval(token)` instead of `request_approval()`.

This pause/resume cycle crosses arbitrary time gaps and process restarts, provided the application persists the `approval_id` and the original inputs. Downstream layers that need human-in-the-loop or externally-decided execution **SHOULD** model their suspension on this contract rather than introduce their own.

The pipeline keeps no state across the gate (protocol-spec §7.4, *Resume semantics*): Steps 1–4 run again on resume. Middleware `before` hooks (Step 6) and the module itself (Step 8) come after the gate, so they run only on the call that is approved.

### 2.3 The `TaskStore` interface

[`AsyncTaskManager`](../features/async-tasks.md) provides single-call durable task execution: submit a module call, get a `task_id`, look up status later, persist `TaskInfo` records across restarts. Its persistence layer is the `TaskStore` interface — five **asynchronous** methods: `save`, `get`, `list`, `delete`, `list_expired` (`listExpired` in TypeScript) (D-17).

Every SDK ships an `InMemoryTaskStore` reference implementation. **Production backends (Redis, SQL, file system, custom queues) are not part of apcore** — the interface is the contract; the storage choice is downstream. A store that cannot reach its backend raises `TaskStoreError` (`TASK_STORE_UNAVAILABLE`), and every manager method propagates it to the caller rather than reporting an empty or false result (D-81).

A workflow runtime that wants persistent task tracking implements the five methods and passes the store to the manager — `AsyncTaskManager(executor, store=MyTaskStore())` in Python, `new AsyncTaskManager({ executor, store })` in TypeScript — without touching apcore internals.

### 2.4 The six extension points

Via `ExtensionManager` ([protocol-spec §11.3](./protocol-spec.md#113-custom-extension-points)), apcore exposes six pluggable slots:

| Extension point | Multi | Typical use by retry/replay layers |
|---|---|---|
| `discoverer` | single | Custom module discovery (rarely needed for durability) |
| `middleware` | multi | **Primary integration point.** Wrap calls with retry, dedup, span recording, replay short-circuit |
| `acl` | single | Custom access control |
| `span_exporter` | multi | Persist trace data for replay reconstruction |
| `module_validator` | single | Custom module validation |
| `approval_handler` | single | **Primary suspension point.** Persist approval state externally, gate on durable decisions |

A retry/replay layer typically integrates as a custom **middleware** plus a custom **approval_handler**, plus a custom **TaskStore** when persistent task state is needed. No spec change is required for any of this.

### 2.5 Module annotations relevant to retry safety

The following annotations are inputs to retry/replay decisions (see [`schema-system.md`](../features/schema-system.md)):

- `idempotent: bool` — repeated calls with identical arguments are safe.
- `readonly: bool` — the module performs no observable mutation; trivially retry-safe.
- `destructive: bool` — the module performs irreversible mutation; warrants caution before retry.
- `requires_approval: bool` — the module requires explicit approval; engages the Phase B contract above.

Annotations are **declarative metadata**. apcore does not make retry decisions from them — that is a runtime decision — but they are part of the module's published contract, so downstream layers can rely on them.

### 2.6 `context.data` for runtime state

`context.data` is the carrier for state that threads through a call without becoming part of apcore's normative schema. Its key space is partitioned ([Context Object](../features/context-object.md#data-key-convention)):

| Prefix | Owner |
|---|---|
| `_apcore.` | The framework (reserved; never serialized) |
| `ext.<vendor>.` | Reusable third-party code — retry/replay runtimes, adapters, integrations |
| no prefix | The application's own modules and entry point; `x-correlation-id` is the well-known key for an external correlation ID ([protocol-spec §5.7](./protocol-spec.md#57-context-parameter-specification)) |

Strings, numbers, booleans, lists and JSON-serializable objects in `context.data` round-trip through `Context.serialize()` / `Context.deserialize()`. This makes it the place to carry idempotency keys, replay flags, attempt counters, deadlines, runtime correlation IDs and other non-normative per-call state. A downstream runtime keeps its own keys under `ext.<vendor>.`; apcore defines no dedup behaviour of its own.

---

## 3. Explicit non-goals

apcore deliberately does **not** ship the following. They are runtime concerns.

### 3.1 Mid-pipeline checkpointing of `PipelineContext`

The pipeline is stateless across invocations. There is no `CheckpointStore`, no per-step persistence, no resume from Step 6. The only suspension point is the approval gate (§2.2), and resumption re-runs the pipeline from Step 1.

Mid-pipeline checkpointing would break the pipeline's stateless invariant and require every SDK to carry a generic state-machine engine. Downstream layers that need it implement it externally, typically by wrapping module calls in their own state machine and using `Context.serialize()` to persist input snapshots.

### 3.2 Multi-call workflow orchestration

apcore has no notion of a "workflow", "saga", "compensation chain", "fan-out/fan-in", or "DAG". A workflow coordinates multiple module calls, which is the runtime layer above apcore; its concerns (step-level retry, compensation on failure, parallel branching, durable timers, signal handling) are out of scope here.

### 3.3 Cost governance, budget limits, rate-shaping

Modules can carry `x-cost-per-call`, `x-sla` and similar metadata as informational hints for AI planners ([protocol-spec §4.6](./protocol-spec.md#46-module-extension-metadata-metadata)). apcore does **not** enforce them — no budget tracker, no rate-limiter in the standard surface, no central policy engine for cost. Downstream layers own enforcement, typically as custom middleware or an external policy engine that reads the metadata.

### 3.4 Cross-call state machines

apcore does not model state shared across multiple module calls. Each call is independent; there is no "session", "conversation context" or "agent memory" at the protocol level. Such state lives in `context.data` (per call chain) or in a downstream runtime that owns its lifecycle.

### 3.5 First-class Context fields for application-level concerns

apcore does not promote application-level metadata to first-class `Context` fields when apcore itself does not consume it. Examples:

- **Idempotency keys** — application semantics; `context.data["x-correlation-id"]` or an `ext.<vendor>.` key.
- **Replay-mode hints** (`is_replay`, `attempt_number`) — runtime semantics; an `ext.<vendor>.` key.
- **Compensation pointers / saga step IDs** — workflow semantics; they live in the workflow runtime.
- **Budgets, retry policies** — same reasoning. (The call-chain deadline is the exception: apcore consumes it, so it is a Context field.)

Promoting any of these to a normative field would widen apcore's surface without adding apcore-level behaviour. A future revision may revisit this if real usage shows the `data` convention is insufficient.

---

## 4. Integration patterns for downstream layers

Recommended, vendor-neutral recipes for a retry/replay/workflow layer on top of apcore. They use only the stable hooks in §2, and none needs new spec surface.

### 4.1 Pattern A — Transient retry (no durability)

For retries within the lifetime of one process: a `Middleware` whose `on_error` returns a `RetrySignal` for a retryable error. Every SDK ships `RetryMiddleware`, which does exactly this (see [`middleware-system.md`](../features/middleware-system.md)).

### 4.2 Pattern B — Crash-durable single-call retry

For "submit a call, survive a process restart, retry on failure with backoff":

1. Use `AsyncTaskManager` with a custom `TaskStore` backed by your persistence layer.
2. Pass a `RetryConfig` (`max_retries`, `retry_delay_ms`, `backoff_multiplier`, `max_retry_delay_ms`) on submission.
3. On process startup, the application enumerates `pending` and `running` records from its `TaskStore` and re-submits or fails them as it sees fit. apcore does **not** resume tasks across process boundaries: the store makes the state durable, and the resumption policy is the application's.

`AsyncTaskManager` handles the lifecycle, retry scheduling and backoff within one process.

### 4.3 Pattern C — Long-running pause for external decisions

For execution that must pause indefinitely waiting on a human, an external service, or a scheduled trigger:

1. Make the call require approval — `requires_approval: true` on the module, or an ACL rule with `approval: required` for the callers that need it.
2. Implement an `ApprovalHandler` that:
   - on `request_approval()`, persists the request to your durable store and returns `pending` with an `approval_id`;
   - on `check_approval(token)`, looks up the persisted decision and returns `approved` / `rejected` / `timeout` / still-`pending`.
3. The caller catches `ApprovalPendingError`, persists the original inputs alongside `approval_id`, and re-invokes with `_approval_token = approval_id` once the decision is recorded.

This is the pause/resume contract of §2.2. It crosses crashes, restarts and arbitrary time gaps.

### 4.4 Pattern D — Cross-process invocation

For invoking modules from a worker process, a queue consumer, or a remote service:

1. The originator builds the `Context` (trace ID, identity, any application or `ext.<vendor>.` keys in `data`) and calls `Context.serialize()`.
2. The serialized Context, `module_id` and `inputs` go onto a queue, RPC channel or task store.
3. The worker calls `Context.deserialize()`, attaches its own `executor` / `services` / `cancel_token`, and invokes the module through its executor.

The round-trip is conformance-tested (`context_serialization` fixture).

### 4.5 Pattern E — Logical-call deduplication / replay short-circuit

For "the same logical call MUST NOT execute side effects twice":

1. The originator sets a stable identifier (UUID, ULID, business key) in `context.data["x-correlation-id"]` or an `ext.<vendor>.` key.
2. A middleware on the worker side reads the key in `before` and consults a dedup store for a prior completion.
3. If a prior result exists, the middleware short-circuits with the cached output (or re-raises the recorded error). Otherwise the call runs normally and the middleware records the result in `after`.

The middleware owns the dedup logic; apcore guarantees only that the key survives serialization and child-context derivation.

---

## 5. Gap watchlist

These items were considered and **deliberately deferred**. They are listed so integrators know which gaps are intentional and where to prototype today.

| Deferred item | Today's workaround | Why deferred |
|---|---|---|
| Explicit `is_replay` hint on Context | An `ext.<vendor>.is_replay` key | No concrete consumer yet; promoting it before usage evidence risks getting the semantics wrong (does it apply mid-call-chain? does it propagate to children?). |
| Explicit `attempt_number` field | An `ext.<vendor>.attempt` key | Same. Runtimes count attempts differently (per call vs. per workflow). |
| `compensatable: bool` annotation | Application-level convention | Compensation is a workflow concept; apcore has no notion of paired forward/reverse modules. |
| `deterministic: bool` annotation | `idempotent` is the closest existing primitive | Determinism (same input → same output) is stricter than idempotency and harder to verify. |
| Dedicated `idempotency_key` field | `context.data["x-correlation-id"]` or an `ext.<vendor>.` key | apcore does not consume the key, so promoting it adds surface without behaviour. |
| Mid-pipeline `CheckpointStore` extension point | Wrap calls externally with a state machine + `Context.serialize()` | Would break the executor's stateless invariant. |
| Cost / budget enforcement | Application-level middleware | Cost is multi-dimensional and runtime-specific. |

Each item can become a spec proposal once a concrete downstream runtime makes a sustained, evidence-backed case.

---

## 6. Versioning

This page is part of apcore's spec surface and follows the compatibility rules of protocol-spec §13.5.

- Stable hooks in §2 evolve only in additive, backward-compatible ways within a major version.
- Non-goals in §3 may become goals in a future major version, with community discussion and evidence.
- The gap watchlist (§5) is non-normative; entries move to §2 (adopted) or §3 (rejected) over time.

---

## 7. References

- [protocol-spec.md](./protocol-spec.md) — full normative specification
    - §4.6 — module extension metadata (`x-` keys)
    - §5.7 — Context structure and serialization
    - §7 — Approval System (including `_approval_token` Phase B)
    - §11 — Extension mechanism
    - §13.5 — Versioning and compatibility
- [`POSITIONING.md`](../POSITIONING.md) — apcore's place in the AI stack
- [Async Tasks](../features/async-tasks.md) — `AsyncTaskManager` and the `TaskStore` interface
- [Approval System](../features/approval-system.md) — approval handler protocol
- [Extension System](../features/extension-system.md) — `ExtensionManager` and extension point lifecycle
- [Middleware System](../features/middleware-system.md) — middleware contract and `RetrySignal`
- [Context Object](../features/context-object.md) — `context.data` key convention
