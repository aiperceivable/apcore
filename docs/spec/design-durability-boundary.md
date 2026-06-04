---
description: "Defines apcore's boundary with durable execution: what hooks it guarantees for retry/replay/workflow layers, what it deliberately omits as single-call-scoped, and how downstream runtimes integrate."
---

# Durability Boundary Design

This document defines apcore's boundary with respect to **durable execution** — the family of concerns that includes retry across crashes, replay after restart, deduplication of logically-equivalent invocations, long-running pause/resume, and persistent task queues. These concerns are routinely required by AI Agent runtimes and workflow systems built on top of module standards like apcore, but they are **not** part of apcore itself.

The purpose of this document is to make explicit:

1. **What apcore guarantees** — the stable hooks and contracts that any retry/replay/workflow layer above apcore MAY rely on.
2. **What apcore deliberately does NOT do** — to prevent scope creep and to give downstream integrators a clear picture of which concerns they must own.
3. **How a downstream layer should integrate** — concrete, vendor-neutral patterns using existing apcore primitives.
4. **Gap watchlist** — known areas where apcore has deliberately deferred a decision until real-world usage provides evidence.

apcore is the **module standard**. Durable execution is a **runtime concern**. Both can coexist; this document documents the seam.

---

## 1. Position

apcore defines how AI-perceivable modules are described, validated, governed, observed, and invoked through an 11-step execution pipeline. It is intentionally:

- **Single-call scoped.** The pipeline operates on one module invocation at a time. apcore has no concept of a "workflow" that spans multiple module calls.
- **Stateless across invocations.** No `PipelineContext` state is preserved between calls. Each call starts a fresh pipeline.
- **Standard, not runtime.** apcore ships interfaces and reference behaviors. Production-grade backends (persistent task stores, distributed approval handlers, replay engines) are the responsibility of applications, framework adapters, or community packages built on top.

Workflow orchestration — sagas, fan-out/fan-in, multi-step compensation, cross-call state machines, durable pause/resume of arbitrary execution — is outside apcore's scope. See [`POSITIONING.md`](../POSITIONING.md).

External retry/replay/workflow layers can and do build on apcore. This document enumerates the contracts they may rely on as stable.

---

## 2. Stable hooks (downstream layers MAY rely on these)

The following primitives are part of apcore's normative surface. Behavior described here is guaranteed within the current major version per the compatibility matrix in `protocol-spec.md` §13.5.

### 2.1 Context JSON serialization

`Context` is JSON-serializable across process and network boundaries. The exact serialization rules are normative in `protocol-spec.md` §5.7. Key guarantees:

- `trace_id`, `caller_id`, `call_chain`, `identity` round-trip losslessly.
- `data` round-trips for all serializable values; non-serializable values (functions, sockets, connections) are silently skipped with a logged warning.
- `executor`, `cancel_token`, `services` are runtime injects and **MUST NOT** be serialized.

This makes it possible to ship a Context across a queue, a job board, or a persistent task store and rehydrate it later — the foundation of any cross-process retry/replay layer.

### 2.2 The `_approval_token` pause/resume contract (Approval Phase B)

The Approval System (`protocol-spec.md` §7) is currently the **only** suspension point built into apcore's pipeline. Its Phase B contract is:

1. A module annotated `requires_approval: true` calls into the configured `ApprovalHandler`.
2. If the handler returns `pending`, the executor raises `ApprovalPendingError` carrying an `approval_id`.
3. The caller persists `approval_id` somewhere (apcore does not), waits for an out-of-band approval signal, and re-invokes the same module with `_approval_token = approval_id` injected into the arguments.
4. On resume, the executor re-enters the pipeline **from step 1** with `_approval_token` present; the approval gate calls `handler.check_approval(token)` instead of `request_approval()`.

This is a complete pause/resume cycle that already crosses arbitrary time gaps and process restarts, provided the application persists the `approval_id` and original inputs. Downstream layers wanting human-in-the-loop or external-decision-gated execution SHOULD model their suspension on this contract rather than introducing their own.

The "Resume semantics" clarification in `protocol-spec.md` §7 is normative: the executor re-runs from step 1 — pre-approval middleware side effects (logging, tracing) re-execute. Middleware that needs at-most-once semantics across an approval gate SHOULD inspect `_approval_token` itself.

### 2.3 The `TaskStore` interface

[`AsyncTaskManager`](../features/async-tasks.md) provides single-call durable task execution: submit a module call, get a `task_id`, look up status later, persist `TaskInfo` records across restarts. Its persistence layer is the `TaskStore` interface (five methods: `save`, `get`, `list`, `delete`, `list_expired`).

apcore MUST ship an `InMemoryTaskStore` reference implementation in every SDK. **Production-grade backends (Redis, SQL, file-system, custom queues, etc.) are not part of apcore** — they are the application's or community's responsibility. The interface is the contract; the storage choice is downstream.

This means a workflow runtime that wants persistent task tracking can implement a single 5-method interface and plug it in via `AsyncTaskManager(executor, store=MyTaskStore())` without touching apcore internals.

### 2.4 The six extension points

Via `ExtensionManager` (`protocol-spec.md` §11), apcore exposes six pluggable slots:

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

The following annotations are normative inputs to retry/replay decisions (see [`schema-system.md`](../features/schema-system.md)):

- `idempotent: bool` — indicates repeated calls with identical arguments are safe.
- `readonly: bool` — module performs no observable mutation; trivially retry-safe.
- `destructive: bool` — module performs irreversible mutation; warrants caution before retry.
- `requires_approval: bool` — module requires explicit approval; engages the Phase B contract above.

Annotations are **declarative metadata**. apcore the SDK does not enforce retry decisions based on them — that's a runtime decision. But because the values are part of the module's published contract, downstream layers can rely on them being stable.

### 2.6 `context.data` and the §4.6 `x-` extension mechanism

`context.data` is the canonical carrier for application-level and runtime-private state that needs to thread through a call without becoming part of apcore's normative schema. Per `protocol-spec.md` §4.6, keys prefixed with `x-` are reserved for extension metadata.

Common patterns supported today:

- `context.data["x-correlation-id"]` — caller-supplied correlation identifier (formalized in `protocol-spec.md` §5.7).
- Any other `x-*` key — reserved namespace for downstream layers.

Strings, numbers, booleans, lists, and JSON-serializable objects placed in `context.data` round-trip through `Context.serialize` / `Context.deserialize` reliably. Non-serializable values are silently skipped with a logged warning. This makes `context.data` the appropriate place to carry idempotency keys, replay flags, attempt counters, deadlines, runtime correlation IDs, and any other non-normative per-call state.

A downstream layer needing a durable identity for a logical invocation (for retry deduplication, replay short-circuit, etc.) MAY define and consume its own `x-*` key. apcore reserves no specific namespace for any particular runtime, and offers no normative dedup behavior of its own.

---

## 3. Explicit non-goals

apcore deliberately does **not** ship the following, and is unlikely to in the foreseeable future. These are runtime concerns, not module-standard concerns.

### 3.1 Mid-pipeline checkpointing of `PipelineContext`

The 11-step executor pipeline is stateless across invocations. There is no `CheckpointStore`, no per-step persistence, no resume from step 6. The only suspension point built into the pipeline is the approval gate (§2.2 above), and resumption re-runs the entire pipeline from step 1.

Mid-pipeline checkpointing — preserving partial state mid-execution and resuming after a crash — would invade the pipeline's stateless invariant and require every SDK to carry a generic state-machine engine. Downstream layers that need this MUST implement it externally, typically by wrapping module calls in their own state-machine and using `Context.serialize` to persist input snapshots.

### 3.2 Multi-call workflow orchestration

apcore has no notion of a "workflow", "saga", "compensation chain", "fan-out/fan-in", or "DAG". A workflow is a coordinated sequence of multiple module calls — that's the runtime layer above apcore. Workflow orchestrators are downstream consumers; their concerns (step-level retry, compensation on failure, parallel branching, durable timer, signal handling) are out of scope here.

### 3.3 Cost governance, budget limits, rate-shaping

Modules carry `x-cost-per-call`, `x-rate-limit`, and similar extension annotations as informational hints for AI planners (see `protocol-spec.md` §4.6). apcore does **not** enforce them — no built-in budget tracker, no rate-limiter middleware in the standard surface, no centralized policy engine. Downstream layers own enforcement, typically as custom middleware or via an external policy engine that consumes the annotations.

### 3.4 Cross-call state machines

apcore does not model state shared across multiple module calls. Each call is independent; there is no "session", no "conversation context", no "agent memory" at the protocol level. Such state, if required, lives in `context.data` (transient, per-call-chain) or in a downstream runtime that owns the state lifecycle.

### 3.5 First-class Context fields for application-level concerns

apcore deliberately does not promote application-level metadata to first-class `Context` fields when apcore the SDK does not consume that metadata itself. Examples:

- **Idempotency keys** — application semantics; lives in `context.data["x-correlation-id"]` or any other `x-*` key.
- **Replay-mode hints** (`is_replay`, `attempt_number`) — runtime semantics; lives in `context.data` under a runtime-defined namespace.
- **Compensation pointers / saga step IDs** — workflow semantics; lives in the workflow runtime, not in apcore.
- **Deadlines, budgets, retry policies** — same reasoning.

Promoting any of these to a normative field would expand apcore's surface area without adding apcore-level behavior. The §4.6 `x-` extension mechanism is the documented home for such concerns. Future spec revisions MAY revisit this if real-world usage demonstrates that a `data`-key convention is insufficient — but the bar is concrete usage evidence, not speculation.

---

## 4. Integration patterns for downstream layers

The following are recommended, vendor-neutral recipes for building a retry/replay/workflow layer on top of apcore. They use only the stable hooks listed in §2.

### 4.1 Pattern A — Transient retry (no durability)

For retries that survive the lifetime of a single process: implement a `Middleware` that catches retryable errors in `on_error` and returns a `RetrySignal`. `RetryMiddleware` is shipped in `apcore-python`'s middleware collection as a reference (see [`middleware-system.md`](../features/middleware-system.md)).

**No new spec surface needed.**

### 4.2 Pattern B — Crash-durable single-call retry

For "submit a call, survive a process restart, retry on failure with backoff":

1. Use `AsyncTaskManager` with a custom `TaskStore` implementation backed by your persistence layer (Redis, SQL, etc.).
2. Configure `RetryConfig` (max_retries, backoff) on submission.
3. On process startup, the application enumerates `pending` and `running` records from its `TaskStore` and re-submits or marks them as appropriate. apcore does **not** auto-resume tasks across process boundaries; the `TaskStore` makes the state durable, but resumption policy (re-submit vs. fail-and-restart vs. ignore) is the application's choice.

`AsyncTaskManager` handles the lifecycle, retry scheduling, and backoff math within a single process. The 5-method `TaskStore` is the only interface the application implements; cross-restart behavior is built on top of it by the application.

**No new spec surface needed.**

### 4.3 Pattern C — Long-running pause for external decisions

For execution that must pause indefinitely waiting on a human, an external service, or a scheduled trigger:

1. Mark the relevant modules with `requires_approval: true`.
2. Implement a custom `ApprovalHandler` that:
   - On `request_approval()`, persists the request to your durable store and returns `pending` with an `approval_id`.
   - On `check_approval(token)`, looks up the persisted decision and returns `approved` / `rejected` / `timeout` / still-`pending`.
3. The caller catches `ApprovalPendingError`, persists the call's original inputs alongside `approval_id`, and re-invokes with `_approval_token = approval_id` once the decision is recorded.

This is the canonical pause/resume contract (§2.2). It crosses crashes, restarts, and arbitrary time gaps.

**No new spec surface needed.**

### 4.4 Pattern D — Cross-process invocation

For invoking modules from a worker process, a queue consumer, or a remote service:

1. The originator builds the `Context` (sets `trace_id`, `identity`, any `x-*` keys in `data`) and calls `Context.serialize()` to JSON.
2. The serialized Context plus `module_id` and `inputs` are placed on a queue / RPC channel / task store.
3. The worker calls `Context.deserialize()`, attaches its own `executor`/`services`/`cancel_token` runtime injects, and invokes the module via the executor.

`Context.serialize()` and `Context.deserialize()` are normative; the round-trip is conformance-tested. Runtime injects are reattached on the worker side per §5.7.

**No new spec surface needed.**

### 4.5 Pattern E — Logical-call deduplication / replay short-circuit

For "the same logical call MUST NOT execute side effects twice":

1. The originator sets a stable identifier (UUID, ULID, business key) in `context.data["x-correlation-id"]` (or another `x-*` key of the runtime's choosing).
2. A middleware on the worker side reads the key on `before` and consults a dedup store (Redis, DB) for prior completion.
3. If a prior result exists, the middleware short-circuits by returning the cached output (or re-raising the recorded error). Otherwise it executes normally and records the result on `after`.

The middleware owns the dedup logic; apcore guarantees only that the key survives serialization and child-context derivation when stored in `data`.

**No new spec surface needed.**

---

## 5. Gap watchlist

The following items have been considered and **deliberately deferred**. They are documented here so downstream integrators understand which gaps are intentional and where to prototype today.

| Deferred item | Today's workaround | Why deferred |
|---|---|---|
| Explicit `is_replay` hint on Context | `context.data["x-is-replay"]` (or runtime-chosen `x-*` key) | No concrete consumer exists yet; promoting before usage evidence risks getting the semantics wrong (does it apply mid-call-chain? does it propagate to children?). |
| Explicit `attempt_number` field | `context.data["x-attempt"]` | Same. Different runtimes count attempts differently (per-call vs per-workflow). |
| `compensatable: bool` annotation | Application-level convention | Compensation is a workflow concept; apcore has no notion of paired forward/reverse module pairs. |
| `deterministic: bool` annotation | `idempotent` is the closest existing primitive | Determinism (same input → same output) is stricter than idempotency and harder to verify. Speculative without a runtime that consumes it. |
| Dedicated `idempotency_key` first-class field | `context.data["x-correlation-id"]` (or runtime-chosen `x-*` key) | apcore the SDK doesn't consume the key; promoting it adds spec surface without behavioral benefit. Revisit when a runtime demonstrates the `data` convention is insufficient. |
| Mid-pipeline `CheckpointStore` extension point | Wrap calls externally with state-machine + `Context.serialize()` | Would invade the executor's stateless invariant. |
| Cost / budget enforcement | Application-level middleware | Cost is multi-dimensional and runtime-specific. |

Each deferred item is a candidate for a future spec proposal once a concrete downstream runtime makes a sustained, evidence-backed case.

---

## 6. Versioning

This document is part of apcore's spec surface and is governed by `protocol-spec.md` §13.5 compatibility rules.

- Stable hooks listed in §2 MAY evolve only in additive, backward-compatible ways within a major version.
- Non-goals listed in §3 MAY become goals in a future major version with appropriate community discussion and evidence.
- The Gap watchlist (§5) is non-normative and reflects current judgment; entries may be promoted to either §2 (added) or §3 (rejected) over time.

---

## 7. References

- `protocol-spec.md` (repository root) — full normative specification
  - §4.6 — `x-` extension mechanism
  - §5.7 — Context structure and serialization
  - §7 — Approval System (including `_approval_token` Phase B)
  - §11 — Extension points
  - §13.5 — Versioning and compatibility
- [`POSITIONING.md`](../POSITIONING.md) — apcore's place in the AI stack
- [`docs/features/async-tasks.md`](../features/async-tasks.md) — `AsyncTaskManager` and the `TaskStore` interface
- [`docs/features/approval-system.md`](../features/approval-system.md) — Approval handler protocol
- [`docs/features/extension-system.md`](../features/extension-system.md) — `ExtensionManager` and extension point lifecycle
- [`docs/features/middleware-system.md`](../features/middleware-system.md) — Middleware contract and `RetrySignal`
