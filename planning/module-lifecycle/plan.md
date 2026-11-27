# Module Lifecycle Implementation Plan

## Phase A

Expose an owned shutdown operation and an ordered detailed removal batch in all three SDKs.

### Task Breakdown

Each SDK implements its public interfaces and driver in parallel from the same design and fixture. Reuse registration generation and lease accounting, unload, event delivery, and native coordination primitives. Integrate with the existing APIs and preserve their signatures and legacy behavior.

### Acceptance Criteria

- One shutdown coordinator per Registry, with a shared operation ID and per-client event flush policy. Waiters alone can time out or cancel while teardown continues.
- Drain expiry is observable under `continue_waiting`. Work remains active until its leases end. Batch dispatch stops at its own deadline, tracks started work, and never removes a replacement generation.
- Validate inputs before mutation. Distinguish conflicting explicit shutdown options, same-operation blocking waits, and pending, completed, and failed reports.
- Keep public lifecycle and existing conformance tests green. Cover the new paths through the three drivers of `module_lifecycle.json`.

### Verification

Run the applicable SDK suites and strict conformance integrity and documentation checks. Phase A requests use SDK-native options and reports and require no canonical Schema change.

The full SDK runs passed with 6,679 Python tests, 5,886 TypeScript tests, and 3,274 Rust tests. The four additive lifecycle cases run in all three drivers. TypeScript type checking and build, Rust Clippy and formatting, Python Pyright and Ruff, the shared integrity guards, and the strict documentation build passed. The Rust full run used a separate temporary directory to prevent contention on fixed test fixture paths.

## Phase B

Implement generation-owned drain and recovery, asynchronous hook adapters, and optional cancellation followed by forced teardown. Extend governed reload through the existing Executor with a managed lifecycle report. Specify the canonical lifecycle events and validate the report against the output Schema.

### Acceptance criteria

- Deliver the drain signal before suspend so an admitted call can finish cleanly.
- Preserve the old generation and its memory after pre-unload abort. Refuse new calls during readiness recovery and reject an unpaired hook before closing admission.
- Keep a replacement unpublished while its hooks run. Report the primary error and compensation after a failed load.
- Preserve legacy responses and governance while returning terminal opt-in reports, including an empty successful selection.
- Scope forced cancellation to retiring invocations. Emit a resource-completion event once per participating client, correlated to the shared Registry operation.

### Verification

Run the shared lifecycle and version-negotiation fixtures in Python, TypeScript, and Rust. Run SDK-local lifecycle and governance checks, the complete SDK suites, shared integrity guards, and a strict documentation build.

All three drivers passed the 21 shared lifecycle cases, including the seven managed lifecycle cases. Per-SDK fixture mutation confirmed that each of the 21 cases detects an incorrect expectation. The drivers verified terminal opt-in reload responses against the canonical output Schema. The no-match case sends `0.0` to exercise the Draft 2020-12 integer rule in each SDK. The full suite results were 6,693 Python tests passed, 5,902 TypeScript tests passed, and 3,284 Rust tests passed. Python Ruff and Pyright, TypeScript type checking and build, Rust formatting, Clippy, character checks, and examples, the conformance integrity guards, and strict MkDocs all passed.

Two Python latency samples showed no repeatable ordinary asynchronous call regression above the proposed 5% release threshold. Benchmarking TypeScript and Rust latency and allocation changes remains part of release verification.

## Review remediation

Keep unload ownership through hook settlement and lease release, including teardown that starts through a compatibility entry point. A synchronous Python registration rejects asynchronous hooks even when called inside an asynchronous parent hook. A declined candidate publication runs cleanup with the same callback ownership as loading.

Exercise managed reload when a source is deleted during a gated drain, the audit store rejects a write, or a candidate hook reports private error text. Observe the committed version and verify that shutdown prevents a later publication. Preserve each compatibility entry point's documented result and timing.

Require two shared regressions through the public Executor. A native stream must receive the drain signal, complete its cleanup, and transfer state in order. Denied approval must leave the original module callable and emit zero managed lifecycle events. Confirm all 23 cases run and detect incorrect expectations in each SDK.

### Verification

All 23 shared cases passed in each SDK. Per-SDK fixture mutation verified every expectation, including stream completion ordering and rejection without lifecycle events. The full suites passed 6,703 Python tests, 5,910 TypeScript tests, and 3,290 Rust tests. Python Ruff and Pyright, TypeScript type checking and build, Rust strict Clippy, formatting, source checks and examples, the conformance integrity guards, and strict MkDocs passed.

TypeScript's browser callback guard covers synchronous callbacks. Browser hosts inspect an operation with `snapshot()` inside the callback and await it from host code outside the callback. Rust keeps lifecycle timers and owned cleanup on a time-enabled SDK worker so a host Tokio runtime can start lifecycle operations without enabling its own timer driver.
