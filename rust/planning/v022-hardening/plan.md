# apcore-rust — v0.22.0 Hardening Implementation Plan

**Scope:** implement apcore spec issues #61–#65 (commit `1e9051c` in `aiperceivable/apcore`).
**Tracking issue:** [aiperceivable/apcore-rust#26](https://github.com/aiperceivable/apcore-rust/issues/26).
**Spec source of truth:** `/Users/tercel/WorkSpace/aipartnerup/apcore` repo. Read these before coding:
- `docs/features/event-system.md` §"Event Delivery Semantics (Issue #61)"
- `docs/features/streaming.md` §"Streaming Module Interface (Issue #62)"
- `docs/features/context-object.md` §"Typed Access via ContextKey[T]"
- `docs/features/middleware-system.md` §"Duplicate Middleware Detection (Issue #64)"
- `docs/features/registry-system.md` §"Registration Ordering Invariants (Issue #65)"
- `docs/features/error-system.md` §"Streaming Errors"
- `docs/spec/design-context-annotations-acl.md` §1.4–§1.5 for `ContextKey<T>` ground truth (note: Rust impl uses `Cow<'static, str>` for zero-allocation static keys)
- `conformance/fixtures/event_delivery_semantics.json` (4 cases)
- `conformance/fixtures/registry_load_ordering.json` (4 cases)

---

## Working tree

`git status` is clean — start with `git checkout -b feat/v022-hardening-61-65` from `main`.

## Tooling

- Test runner: `cargo test`. Filter by name: `cargo test v022`.
- Lint: `cargo clippy --all-targets -- -D warnings` (Makefile target likely exists; check `Makefile`).
- Format: `cargo fmt --check`.
- `.code-forge.json` present → this repo uses code-forge tooling; honor existing conventions if anything conflicts.

## Existing layout (use these paths)

```
src/
├── context_key.rs            # ContextKey<T> (already exists; Cow<'static, str> impl)
├── context_keys.rs           # built-in constants
├── errors.rs                 # error classes + ErrorCode enum — add StreamingInterfaceError + StreamingInterfaceMismatch here
├── module.rs                 # Module trait, includes existing `stream() -> Option<ChunkStream>` — add StreamingModule trait + as_streaming() default method here OR new streaming.rs
├── executor.rs               # Executor — update streaming detection
├── client.rs                 # client builder — middleware entry
├── lib.rs                    # public re-exports
├── events/
│   ├── mod.rs
│   ├── emitter.rs            # EventEmitter — main change site for #61
│   ├── subscribers.rs        # WebhookSubscriber / A2ASubscriber
│   └── circuit_breaker.rs
├── middleware/
│   ├── manager.rs            # main change site for #64
│   ├── base.rs
│   └── ...
└── registry/
    ├── mod.rs
    └── registry.rs           # main change site for #65

tests/                        # flat tests/*.rs files; integration-style
├── test_chunk_shape.rs       # existing streaming test — use as reference
├── test_compute_delay_ms.rs  # existing retry-math test — extend for #61
├── conformance_test.rs       # existing conformance harness
└── (new) test_v022_*.rs      # one per issue
```

---

## Rust-specific design notes

- **`StreamingModule` trait + `as_streaming()` accessor:** the base `Module` trait already has `fn stream(&self, inputs: Value, ctx: &Context<Value>) -> Option<ChunkStream>`. Per spec §"Streaming Module Interface (Issue #62)", **both paths coexist**:
  - `Module::stream()` — used by executor / pipeline (type-erased return)
  - `Module::as_streaming() -> Option<&dyn StreamingModule>` — used by adapter / bridge code that needs the typed handle
  - **Invariant:** a module that returns `Some(_)` from `as_streaming()` MUST return `Some(_)` from `Module::stream()`, and vice versa. Document + test this.
- **Rust default trait methods:** `fn as_streaming(&self) -> Option<&dyn StreamingModule> { None }` on base `Module` trait so existing modules don't need to change. Streaming modules override.
- **Concurrency model for #65:** use `tokio::sync::RwLock` for the visible store, `DashMap<ModuleId, Arc<Mutex<()>>>` for per-module init locks, and a separate `Mutex<HashSet<ModuleId>>` for the in-flight loading set (or one combined `Mutex<RegistryState>` if simpler).
- **Async runtime:** `tokio`. EventEmitter retry uses `tokio::time::sleep`. Per-subscriber retry isolation via `tokio::spawn`.
- **Middleware identity:** `std::any::type_name::<T>()`. Captured at the generic registration site.
- **Errors:** existing `ErrorCode` enum in `src/errors.rs` — add `StreamingInterfaceMismatch` variant. `ModuleError` struct already carries `code` + `message`; extend with `extra: HashMap<String, Value>` if there isn't already a way to attach `module_id` / `mismatch_reason` etc.

---

## TDD order (lowest risk → highest)

1. **#63 ContextKey verification** — exports
2. **#62 Streaming trait + accessor** — additive
3. **#64 Middleware duplicate detection** — additive
4. **#61 Event delivery** — emitter core
5. **#65 Registry on_load ordering** — registration hot path

One commit per issue. Footer: `Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>`.

---

## Issue #63 — ContextKey<T> export verification

### Tasks
- [ ] Confirm `ContextKey` exported from `lib.rs`. Add if missing.
- [ ] Confirm 6 framework constants in `src/context_keys.rs` exist as `pub static` items with the spec identifier strings (`_apcore.mw.tracing.spans`, `_apcore.mw.tracing.sampled`, `_apcore.mw.metrics.starts`, `_apcore.mw.logging.start_time`, `_apcore.executor.redacted_output`, `_apcore.mw.retry.count`).
- [ ] Write `tests/test_v022_context_key_promotion.rs`:
  - 6 constants' `.name` values match spec
  - `KEY.set(ctx, v)` / `.get(ctx)` round-trip
  - `KEY.scoped(suffix)` produces correctly-named sub-key
  - Third-party `ext.*` key works equivalently
- [ ] Commit: `feat: promote ContextKey<T> as documented public API (apcore #63)`

### Test skeleton
```rust
// tests/test_v022_context_key_promotion.rs
use apcore::{Context, ContextKey};
use apcore::context_keys::{
    TRACING_SPANS, TRACING_SAMPLED, METRICS_STARTS,
    LOGGING_START, REDACTED_OUTPUT, RETRY_COUNT_BASE,
};
use serde_json::Value;

#[test]
fn builtin_identifiers_match_spec() {
    assert_eq!(TRACING_SPANS.name.as_ref(), "_apcore.mw.tracing.spans");
    assert_eq!(TRACING_SAMPLED.name.as_ref(), "_apcore.mw.tracing.sampled");
    assert_eq!(METRICS_STARTS.name.as_ref(), "_apcore.mw.metrics.starts");
    assert_eq!(LOGGING_START.name.as_ref(), "_apcore.mw.logging.start_time");
    assert_eq!(REDACTED_OUTPUT.name.as_ref(), "_apcore.executor.redacted_output");
    assert_eq!(RETRY_COUNT_BASE.name.as_ref(), "_apcore.mw.retry.count");
}

#[test]
fn key_anchored_api_roundtrip() {
    static KEY: ContextKey<u32> = ContextKey::new("ext.test.retry.count");
    let ctx: Context<Value> = make_test_context();
    KEY.set(&ctx, 3u32);
    assert_eq!(KEY.get(&ctx), Some(3u32));
}
```

---

## Issue #62 — StreamingModule trait + as_streaming accessor

### Tasks
- [ ] In `src/module.rs` (or a new `src/streaming.rs` re-exported from `module`):
  ```rust
  pub trait StreamingModule: Module {
      fn stream(
          &self,
          inputs: serde_json::Value,
          context: &Context<serde_json::Value>,
      ) -> ChunkStream;
  }
  ```
- [ ] Add default method to the base `Module` trait:
  ```rust
  pub trait Module: Send + Sync {
      // ... existing methods ...
      
      fn as_streaming(&self) -> Option<&dyn StreamingModule> {
          None
      }
  }
  ```
- [ ] Document the **consistency invariant** in trait docs: a module that returns `Some(_)` from `as_streaming()` MUST return `Some(_)` from `Module::stream()` and vice versa. Add a debug-mode assertion in `Executor::stream` or `Registry::register` that catches violations.
- [ ] Add `StreamingInterfaceError` in `src/errors.rs`:
  ```rust
  // Add to ErrorCode enum:
  StreamingInterfaceMismatch,
  
  // Helper constructor for ModuleError:
  impl ModuleError {
      pub fn streaming_interface_mismatch(
          module_id: impl Into<String>,
          expected_signature: impl Into<String>,
          actual_signature: impl Into<String>,
          mismatch_reason: impl Into<String>,
      ) -> Self { ... }
  }
  ```
- [ ] At registration time in `Registry::register`, if `module.annotations().streaming` is true and `module.as_streaming().is_none()` → return `Err(ModuleError::streaming_interface_mismatch(...))`. Reason: `missing_marker`.
- [ ] Update bridge / adapter examples in `examples/` to use `module.as_streaming()` instead of probing `module.stream()`.
- [ ] Export `StreamingModule` from `lib.rs`.
- [ ] Write `tests/test_v022_streaming_interface.rs`:
  - streaming module returns `Some(self)` from `as_streaming`
  - non-streaming module returns `None` (default method)
  - module with `annotations.streaming = true` but no `StreamingModule` impl → registration `Err(StreamingInterfaceMismatch)`
  - invariant check: a faulty module that returns `Some` from one path and `None` from the other → `Executor::stream` panics in debug / returns error in release
- [ ] Commit: `feat: add StreamingModule trait and Module::as_streaming accessor (apcore #62)`

---

## Issue #64 — Duplicate middleware detection

### Tasks
- [ ] In `src/middleware/manager.rs` (the `MiddlewareManager` or builder), maintain `HashMap<&'static str, RegistrationInfo>` for identity → first registration site. Use `std::any::type_name::<M>()` as default identity, captured inside the generic registration fn:
  ```rust
  impl MiddlewareManager {
      pub fn register<M: Middleware + 'static>(
          &mut self,
          middleware: M,
          opts: MiddlewareRegistration,
      ) -> Result<(), ModuleError> {
          let identity = opts.identity_key
              .clone()
              .unwrap_or_else(|| std::any::type_name::<M>().to_string());
          // ... see below for the rest ...
      }
  }
  ```
- [ ] `MiddlewareRegistration` builder with `.allow_duplicate(bool)` and `.identity_key(impl Into<String>)`. Default `allow_duplicate = false`, `identity_key = None`.
- [ ] On duplicate: emit `tracing::warn!` naming the identity + first site + duplicate site. Capture sites with `#[track_caller]` on `register` + `std::panic::Location::caller()` at the call site (preserved through the generic).
- [ ] Registration MUST succeed regardless; order preserved (push to the chain `Vec`).
- [ ] Write `tests/test_v022_middleware_duplicate_detection.rs`:
  - one registration: no `tracing::warn` event (use `tracing-test` crate or capture via custom subscriber)
  - two same-type registrations: one warning event
  - `allow_duplicate(true)`: no warning
  - distinct `identity_key`s: no warning
- [ ] Commit: `feat: warn on duplicate middleware registration with identity-based detection (apcore #64)`

### Builder skeleton
```rust
// src/middleware/manager.rs
pub struct MiddlewareRegistration<M> {
    middleware: M,
    allow_duplicate: bool,
    identity_key: Option<String>,
}

impl<M: Middleware + 'static> MiddlewareRegistration<M> {
    pub fn new(middleware: M) -> Self {
        Self { middleware, allow_duplicate: false, identity_key: None }
    }
    pub fn allow_duplicate(mut self, v: bool) -> Self {
        self.allow_duplicate = v;
        self
    }
    pub fn identity_key(mut self, k: impl Into<String>) -> Self {
        self.identity_key = Some(k.into());
        self
    }
}
```

---

## Issue #61 — Event delivery semantics

Biggest change.

### Part A — `RetryConfig`
- [ ] Create `src/events/retry.rs`:
  ```rust
  #[derive(Debug, Clone, Copy)]
  pub struct RetryConfig {
      pub max_attempts: u32,
      pub initial_backoff_ms: u64,
      pub max_backoff_ms: u64,
      pub backoff_multiplier: f64,
  }
  
  impl Default for RetryConfig {
      fn default() -> Self {
          Self {
              max_attempts: 3,
              initial_backoff_ms: 100,
              max_backoff_ms: 30_000,
              backoff_multiplier: 2.0,
          }
      }
  }
  
  impl RetryConfig {
      /// attempt is zero-based; attempt=0 is the first retry after the initial try.
      pub fn compute_delay_ms(&self, attempt: u32) -> u64 {
          let raw = (self.initial_backoff_ms as f64)
              * self.backoff_multiplier.powi(attempt as i32);
          (raw.min(self.max_backoff_ms as f64) as u64).max(0)
      }
  }
  ```
- [ ] Extend or reuse the existing `tests/test_compute_delay_ms.rs` to cover the unified formula and per-subscriber retry exhaustion.

### Part B — Extend `EventSubscriber` trait
- [ ] In `src/events/subscribers.rs` (`EventSubscriber` trait), add default methods:
  ```rust
  #[async_trait]
  pub trait EventSubscriber: Send + Sync + std::fmt::Debug {
      fn subscriber_id(&self) -> &str { "default" }
      fn event_pattern(&self) -> &str { "*" }
      
      fn retry(&self) -> RetryConfig { RetryConfig::default() }
      
      async fn on_event(&self, event: &ApCoreEvent) -> Result<(), ModuleError>;
      
      async fn on_failure(
          &self,
          _event: &ApCoreEvent,
          _error: &ModuleError,
          _attempt_count: u32,
      ) {
          // default no-op
      }
  }
  ```
- [ ] Built-in subscribers gain `id: Option<String>` builder field. SDK-generates `format!("{}-{}", type_label, counter)` when None; counter is per-type via an `AtomicU64`.

### Part C — EventEmitter retry loop
- [ ] Rewrite delivery path in `src/events/emitter.rs`:
  ```rust
  async fn deliver(
      &self,
      subscriber: Arc<dyn EventSubscriber>,
      event: ApCoreEvent,
  ) {
      let retry = subscriber.retry();
      let mut last_error: Option<ModuleError> = None;
      for attempt in 0..retry.max_attempts {
          match subscriber.on_event(&event).await {
              Ok(()) => return,
              Err(e) => {
                  last_error = Some(e);
                  if attempt + 1 < retry.max_attempts {
                      tokio::time::sleep(
                          Duration::from_millis(retry.compute_delay_ms(attempt))
                      ).await;
                  }
              }
          }
      }
      // Exhausted: emit DLQ event + invoke on_failure
      let err = last_error.unwrap();
      self.emit_dlq(&subscriber, &event, &err, retry.max_attempts).await;
      subscriber.on_failure(&event, &err, retry.max_attempts).await;
  }
  
  async fn emit_dlq(...) {
      let dlq = ApCoreEvent::new("apcore.event.delivery_failed", json!({
          "subscriber_type": ...,
          "subscriber_id": subscriber.subscriber_id(),
          "original_event": ...,
          "error": { "type": ..., "message": err.message() },
          "attempt_count": attempt_count,
          "timestamp": chrono::Utc::now().to_rfc3339(),
      }));
      // Single-attempt delivery to DLQ subscribers — NO retry loop.
      self.deliver_no_retry(dlq).await;
  }
  ```
- [ ] Per-subscriber isolation: `emit` spawns one `tokio::task` per subscriber via `tokio::spawn`, so a slow one doesn't block others.

### Part D — A2ASubscriber
- [ ] Add `skill_id: String` field to `A2ASubscriber`, default `"apevo.event_receiver"`. Use in outgoing payload.
- [ ] Remove single-attempt behavior — A2A follows unified retry.

### Tasks
- [ ] Write `tests/test_v022_event_delivery_semantics.rs` covering: retry-before-exhaustion, DLQ-on-permanent-failure, no-DLQ-retry, SDK-generated subscriber_id.
- [ ] Extend `tests/conformance_test.rs` (or add `tests/test_v022_event_delivery_conformance.rs`) to load `/Users/tercel/WorkSpace/aipartnerup/apcore/conformance/fixtures/event_delivery_semantics.json` and assert each case.
- [ ] Commit: `feat: implement unified event delivery semantics with retry, DLQ, on_failure (apcore #61)`

---

## Issue #65 — Registry on_load ordering

Highest-risk refactor; ship last.

### Part A — Deferred-publish refactor
- [ ] In `src/registry/registry.rs`, restructure registry state:
  ```rust
  pub struct Registry {
      visible: RwLock<HashMap<ModuleId, Arc<dyn Module>>>,
      in_flight: Mutex<HashSet<ModuleId>>,
      init_locks: DashMap<ModuleId, Arc<tokio::sync::Mutex<()>>>,
      // ... existing fields ...
  }
  ```
- [ ] Refactor `register()`:
  ```rust
  pub async fn register(&self, module_id: ModuleId, module: Arc<dyn Module>) -> Result<(), ModuleError> {
      // Step 1: validate
      validate_module_id(&module_id)?;
      
      // Step 2: reserve slot in in_flight (synchronous check + insert under one lock)
      {
          let visible = self.visible.read().await;
          let mut in_flight = self.in_flight.lock().unwrap();
          if visible.contains_key(&module_id) || in_flight.contains(&module_id) {
              return Err(ModuleError::duplicate_module_id(&module_id));
          }
          in_flight.insert(module_id.clone());
      }
      
      // Step 3: run on_load under per-module init lock (NOT global registry lock)
      let init_lock = self.init_locks
          .entry(module_id.clone())
          .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
          .clone();
      let _guard = init_lock.lock().await;
      
      // Step 4: invoke on_load
      let load_result = module.on_load().await;
      
      match load_result {
          Ok(()) => {
              // Atomic publish: visible.insert + in_flight.remove
              let mut visible = self.visible.write().await;
              let mut in_flight = self.in_flight.lock().unwrap();
              visible.insert(module_id.clone(), module);
              in_flight.remove(&module_id);
              // ... emit register event ...
              Ok(())
          }
          Err(e) => {
              // Roll back in_flight; emit failure event; re-raise
              self.in_flight.lock().unwrap().remove(&module_id);
              self.emit_load_failed(&module_id, "module.on_load", &e).await;
              Err(e)
          }
      }
  }
  ```
- [ ] Discovery APIs (`get`, `list`, `get_definition`) consult only `visible`. In-flight modules NOT visible.
- [ ] Emit `apcore.registry.module_load_failed` event with payload `{module_id, callback_name, error_type, error_message, timestamp}`.

### Tasks
- [ ] Write `tests/test_v022_registry_load_ordering.rs`:
  - successful `on_load` → module visible after `register()` returns
  - failing `on_load` → `register()` returns `Err`, module NOT visible, `apcore.registry.module_load_failed` emitted
  - concurrent same-ID via `tokio::join!(reg(id), reg(id))` → one `Ok`, one `Err(DuplicateModuleId)`
  - concurrent distinct-ID with 50ms `on_load` delays → wall-clock < 90ms (proves per-module parallelism)
- [ ] Add conformance fixture cases via `tests/test_v022_registry_ordering_conformance.rs` loading `registry_load_ordering.json`.
- [ ] Commit: `feat: enforce on_load completion before module visibility via deferred-publish (apcore #65)`

---

## Cross-cutting

- [ ] Bump `Cargo.toml` `version = "0.22.0"`.
- [ ] Append `## [0.22.0]` entry to `CHANGELOG.md` with `### Added` (issues #61–#65) and `### Changed` (A2A retry; registry concurrent same-ID).
- [ ] Re-export new public types from `lib.rs` (`StreamingModule`, `StreamingInterfaceError`, `RetryConfig`).
- [ ] Run `cargo test` — MUST pass.
- [ ] Run `cargo clippy --all-targets -- -D warnings` — MUST pass.
- [ ] Run `cargo fmt --check` — MUST pass.

## Success criteria

- Branch `feat/v022-hardening-61-65` with 5 commits.
- All existing tests + new tests pass.
- Both conformance fixtures pass.
- Clippy + fmt clean.
- `Cargo.toml` + `CHANGELOG.md` updated.
- No push. No merge.

## Blockers — STOP and report if you hit

- Existing test fails after change → root-cause; don't suppress.
- Conformance fixture ambiguous → cite + STOP.
- `tokio::sync::Mutex` vs `std::sync::Mutex` mix produces deadlock risk → reconsider ordering; document the synchronous-vs-async lock split (the in_flight set uses sync `Mutex` for fast check-and-insert, init locks use async `tokio::sync::Mutex` because they're held across `await`).
- The existing `Module::stream() -> Option<ChunkStream>` signature differs from what spec docs imply → check `src/module.rs` HEAD; respect actual signature; the spec's invariant (both `Some` or both `None`) must hold regardless of the concrete return type.
