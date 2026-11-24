# apcore-python — v0.22.0 Hardening Implementation Plan

**Scope:** implement apcore spec issues #61–#65 (commit `1e9051c` in `aiperceivable/apcore`).
**Tracking issue:** [aiperceivable/apcore-python#27](https://github.com/aiperceivable/apcore-python/issues/27).
**Spec source of truth:** `/Users/tercel/WorkSpace/aipartnerup/apcore` repo. Read these before coding:
- `docs/features/event-system.md` §"Event Delivery Semantics (Issue #61)" (~line 1026)
- `docs/features/streaming.md` §"Streaming Module Interface (Issue #62)"
- `docs/features/context-object.md` §"Typed Access via ContextKey[T]"
- `docs/features/middleware-system.md` §"Duplicate Middleware Detection (Issue #64)"
- `docs/features/registry-system.md` §"Registration Ordering Invariants (Issue #65)" + updated `Contract: Registry.register` Side Effects
- `docs/features/error-system.md` §"Streaming Errors"
- `docs/spec/design-context-annotations-acl.md` §1.4–§1.5 for `ContextKey[T]` ground truth
- `conformance/fixtures/event_delivery_semantics.json` (4 cases)
- `conformance/fixtures/registry_load_ordering.json` (4 cases)

---

## Branch setup (no WIP — start clean)

Before this revision, the plan reserved a "WIP protection" section because `main` had uncommitted changes for the in-flight #60 implementation. **That WIP has since been committed to `main` as `a45fa7f`** (`feat: implement Config.reserved_namespaces() and add RESERVED_NAMESPACES constant per PROTOCOL_SPEC §9.9.5`). The working tree is now clean and `pyproject.toml` already reads `version = "0.22.0"`.

**Hard rules:**
1. First action: confirm `git status` is clean and current branch is `main` (commit `a45fa7f` or descendant). If not, STOP and report.
2. `git checkout -b feat/v022-hardening-61-65` from current `main`.
3. You may freely modify `CHANGELOG.md`, `pyproject.toml` (no further version bump needed — already at `0.22.0`), `src/apcore/__init__.py`, and any source/test file. There is no longer an `ext.*`-prefixed or reserved-file list.
4. CHANGELOG: append/append-after the existing `## [0.22.0]` entry with `### Added` (issues #61–#65) and `### Changed` (A2A retry; registry concurrent same-ID) sub-sections, mirroring the apcore spec repo's `[0.22.0]` shape. Do NOT create a separate `NOTES.md` — you can write directly to `CHANGELOG.md`.

---

## Existing layout (use these paths)

```
src/apcore/
├── context_key.py            # ContextKey[T] class (already exists)
├── context_keys.py           # built-in constants (TRACING_SPANS, etc.)
├── errors.py                 # all error classes — add StreamingInterfaceError here
├── module.py                 # Module / stream() — extract StreamingModule Protocol here or new streaming.py
├── events/
│   ├── __init__.py
│   ├── emitter.py            # EventEmitter — main change site for #61
│   ├── subscribers.py        # WebhookSubscriber / A2ASubscriber / etc.
│   └── circuit_breaker.py
├── middleware/
│   ├── manager.py            # main change site for #64
│   └── base.py
└── registry/
    └── registry.py           # main change site for #65

tests/
├── conformance/              # add test_v022_event_delivery.py + test_v022_registry_ordering.py here
├── events/                   # existing; add test_event_delivery_semantics.py
└── (new) v022/               # OR put all new tests under tests/v022/
```

---

## TDD order (lowest risk → highest)

1. **#63 ContextKey promotion** — mostly docs/exports verification
2. **#62 Streaming interface** — contained type addition
3. **#64 Middleware duplicate detection** — additive, behind opt-in
4. **#61 Event delivery semantics** — touches EventEmitter core
5. **#65 Registry on_load ordering** — touches the registration hot path

One commit per issue, signed with `Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>`.

---

## Issue #63 — ContextKey[T] export verification

**Status check first:** `ContextKey` and built-in constants already exist (`src/apcore/context_key.py`, `src/apcore/context_keys.py`). This task is largely verification.

### Tasks
- [ ] Verify `ContextKey` is exported from top-level `apcore` package (read `src/apcore/__init__.py`). If not present, add it.
- [ ] Verify these 6 constants are exported from `apcore.context_keys`: `TRACING_SPANS`, `TRACING_SAMPLED`, `METRICS_STARTS`, `LOGGING_START`, `REDACTED_OUTPUT`, `RETRY_COUNT_BASE`. Identifier strings must match `_apcore.*` exactly per spec §1.5.
- [ ] Write `tests/v022/test_context_key_promotion.py` asserting: (a) the 6 constants exist and have the correct identifier strings; (b) `KEY.set(ctx, v)` / `KEY.get(ctx)` / `KEY.delete(ctx)` / `KEY.exists(ctx)` / `KEY.scoped(suffix)` round-trip correctly; (c) a `KEY` defined under `ext.*` works equivalently (third-party usage).
- [ ] Commit: `feat: promote ContextKey[T] as documented public API (apcore #63)`

### Test skeleton
```python
# tests/v022/test_context_key_promotion.py
import pytest
from apcore import ContextKey, Context
from apcore.context_keys import (
    TRACING_SPANS, TRACING_SAMPLED, METRICS_STARTS,
    LOGGING_START, REDACTED_OUTPUT, RETRY_COUNT_BASE,
)

def test_builtin_identifiers():
    assert TRACING_SPANS.name == "_apcore.mw.tracing.spans"
    assert TRACING_SAMPLED.name == "_apcore.mw.tracing.sampled"
    assert METRICS_STARTS.name == "_apcore.mw.metrics.starts"
    assert LOGGING_START.name == "_apcore.mw.logging.start_time"
    assert REDACTED_OUTPUT.name == "_apcore.executor.redacted_output"
    assert RETRY_COUNT_BASE.name == "_apcore.mw.retry.count"

def test_key_anchored_api_roundtrip(make_context):
    KEY: ContextKey[int] = ContextKey("ext.test.retry.count")
    ctx = make_context()
    assert KEY.exists(ctx) is False
    KEY.set(ctx, 3)
    assert KEY.exists(ctx) is True
    assert KEY.get(ctx) == 3
    assert KEY.get(ctx, default=99) == 3
    KEY.delete(ctx)
    assert KEY.exists(ctx) is False
    assert KEY.get(ctx, default=99) == 99

def test_scoped_subkey(make_context):
    BASE: ContextKey[int] = ContextKey("ext.test.base")
    sub = BASE.scoped("module-a")
    assert sub.name == "ext.test.base.module-a"
```

---

## Issue #62 — Streaming module interface

### Tasks
- [ ] Create `src/apcore/streaming.py` exporting `StreamingModule(Protocol)` with `@runtime_checkable` and the signature `async def stream(self, inputs: dict, context: Context) -> AsyncIterator[dict]`.
- [ ] Add `StreamingInterfaceError` to `src/apcore/errors.py` with code `STREAMING_INTERFACE_MISMATCH` and fields `module_id`, `expected_signature`, `actual_signature`, `mismatch_reason` (literal: `"wrong_arity" | "not_async" | "wrong_return_type" | "missing_marker"`).
- [ ] In `Registry.register` (or whoever handles `annotations.streaming = True`), validate the module's `stream()` signature using `inspect.signature` at **registration time**. On mismatch raise `StreamingInterfaceError`.
- [ ] Update `Executor.stream` (look in `src/apcore/executor.py`) to use `isinstance(module, StreamingModule)` instead of `hasattr(module, 'stream')`.
- [ ] Export `StreamingModule` from top-level `apcore` package.
- [ ] Write `tests/v022/test_streaming_interface.py`: (a) module satisfying Protocol passes `isinstance`; (b) module missing `stream()` does not; (c) module with `annotations.streaming=True` but wrong signature raises `StreamingInterfaceError` at registration; (d) `Executor.stream` falls back to `execute()` for non-streaming modules (existing behavior preserved).
- [ ] Commit: `feat: define StreamingModule Protocol with registration-time signature validation (apcore #62)`

### Implementation skeleton
```python
# src/apcore/streaming.py
from typing import AsyncIterator, Protocol, runtime_checkable
from apcore.context import Context

@runtime_checkable
class StreamingModule(Protocol):
    """Modules that produce output incrementally MUST satisfy this Protocol.
    
    Note: @runtime_checkable only verifies method presence, not signatures.
    Signature mismatches are caught at registration time by Registry.register
    when annotations.streaming = True.
    """
    async def stream(
        self,
        inputs: dict,
        context: Context,
    ) -> AsyncIterator[dict]:
        ...
```

```python
# src/apcore/errors.py — add to existing file
class StreamingInterfaceError(ModuleError):
    code = "STREAMING_INTERFACE_MISMATCH"
    
    def __init__(
        self,
        module_id: str,
        expected_signature: str,
        actual_signature: str,
        mismatch_reason: str,
    ):
        super().__init__(
            f"Module {module_id!r} declared streaming but stream() does not "
            f"match the StreamingModule Protocol "
            f"(reason={mismatch_reason}; expected {expected_signature}, got {actual_signature})"
        )
        self.module_id = module_id
        self.expected_signature = expected_signature
        self.actual_signature = actual_signature
        self.mismatch_reason = mismatch_reason
```

---

## Issue #64 — Duplicate middleware detection

### Tasks
- [ ] In `src/apcore/middleware/manager.py` (or wherever `use()` / `register_middleware()` lives — check `MiddlewareManager`), maintain a `dict[str, _RegistrationInfo]` mapping identity → first registration site.
- [ ] Identity formula: `f"{type(mw).__module__}.{type(mw).__qualname__}"`, OR if `identity_key` kwarg is provided, use that string verbatim.
- [ ] On `register(...)` call: compute identity. If already in the dict, emit `logger.warning(...)` naming both call sites. Capture call sites via `inspect.stack()[1]` (frame just above `register`) at each registration time. Registration MUST proceed.
- [ ] Add `allow_duplicate: bool = False` kwarg — when `True`, skip the warning emission but still add to the dict.
- [ ] Add `identity_key: str | None = None` kwarg.
- [ ] Document: identity keys starting with `apcore.` are reserved for framework middleware. Vendor prefix recommended.
- [ ] Write `tests/v022/test_middleware_duplicate_detection.py`: (a) one registration, no warning; (b) two same-class registrations, one `WARNING` log captured naming both sites; (c) `allow_duplicate=True` suppresses; (d) distinct `identity_key` values prevent collision; (e) order preserved (both instances fire on `before/after`).
- [ ] Commit: `feat: warn on duplicate middleware registration with identity-based detection (apcore #64)`

### Test skeleton
```python
# tests/v022/test_middleware_duplicate_detection.py
import logging
import pytest
from apcore.middleware import MiddlewareManager
from apcore.middleware.retry import RetryMiddleware

def test_first_registration_no_warning(caplog):
    mgr = MiddlewareManager()
    with caplog.at_level(logging.WARNING):
        mgr.use(RetryMiddleware(max_attempts=3))
    assert not any(r.levelno == logging.WARNING for r in caplog.records)

def test_duplicate_emits_warning(caplog):
    mgr = MiddlewareManager()
    mgr.use(RetryMiddleware(max_attempts=3))
    with caplog.at_level(logging.WARNING):
        mgr.use(RetryMiddleware(max_attempts=2))
    msgs = [r.message for r in caplog.records if r.levelno == logging.WARNING]
    assert any("RetryMiddleware" in m for m in msgs)

def test_allow_duplicate_suppresses(caplog):
    mgr = MiddlewareManager()
    mgr.use(RetryMiddleware(max_attempts=3))
    with caplog.at_level(logging.WARNING):
        mgr.use(RetryMiddleware(max_attempts=5), allow_duplicate=True)
    assert not any(r.levelno == logging.WARNING for r in caplog.records)

def test_distinct_identity_keys_no_warning(caplog):
    mgr = MiddlewareManager()
    mgr.use(RetryMiddleware(max_attempts=3), identity_key="ext.myapp.retry.http")
    with caplog.at_level(logging.WARNING):
        mgr.use(RetryMiddleware(max_attempts=2), identity_key="ext.myapp.retry.db")
    assert not any(r.levelno == logging.WARNING for r in caplog.records)
```

---

## Issue #61 — Event delivery semantics

This is the biggest change. Touches EventEmitter, every subscriber type, and adds DLQ event emission.

### Tasks

#### Part A — `RetryConfig` dataclass
- [ ] Create `src/apcore/events/retry.py` with `RetryConfig` dataclass:
  ```python
  @dataclass(frozen=True)
  class RetryConfig:
      max_attempts: int = 3
      initial_backoff_ms: int = 100
      max_backoff_ms: int = 30_000
      backoff_multiplier: float = 2.0
      
      def compute_delay_ms(self, attempt: int) -> int:
          """attempt is zero-based; attempt=0 is the first retry after the initial try."""
          return min(
              self.max_backoff_ms,
              int(self.initial_backoff_ms * (self.backoff_multiplier ** attempt))
          )
  ```

#### Part B — extend `EventSubscriber` Protocol
- [ ] In `src/apcore/events/subscribers.py` (or wherever `EventSubscriber` Protocol is), add optional members:
  - `event_pattern: str` (default `"*"` for backward compat)
  - `retry: RetryConfig` (default `RetryConfig()`)
  - `async on_failure(self, event, error, attempt_count)` (default no-op)
  - `subscriber_id: str` (SDK-generated if absent; details below)
- [ ] All existing subscriber implementations (`WebhookSubscriber`, `A2ASubscriber`, etc.) MUST gain the `id` constructor parameter (optional) and a `retry: RetryConfig | None = None` parameter.

#### Part C — `EventEmitter._deliver` retry loop
- [ ] Replace fire-and-forget delivery in `src/apcore/events/emitter.py`:
  ```python
  async def _deliver(self, subscriber, event):
      retry = getattr(subscriber, "retry", RetryConfig())
      last_error = None
      for attempt in range(retry.max_attempts):
          try:
              await subscriber.on_event(event)
              return  # success
          except Exception as e:
              last_error = e
              if attempt + 1 < retry.max_attempts:
                  await asyncio.sleep(retry.compute_delay_ms(attempt) / 1000)
      # exhausted — emit DLQ event AND call on_failure if defined
      await self._emit_dlq_event(subscriber, event, last_error, retry.max_attempts)
      if hasattr(subscriber, "on_failure"):
          try:
              await subscriber.on_failure(event, last_error, retry.max_attempts)
          except Exception:
              logger.exception("on_failure callback raised")
  
  async def _emit_dlq_event(self, subscriber, original_event, error, attempt_count):
      # Construct apcore.event.delivery_failed event; emit through self
      # CRITICAL: this event MUST NOT be retried
      dlq_event = ApCoreEvent(
          name="apcore.event.delivery_failed",
          payload={
              "subscriber_type": subscriber.__class__.__name__.lower().replace("subscriber", ""),
              "subscriber_id": subscriber.subscriber_id,
              "original_event": {
                  "name": original_event.name,
                  "payload": original_event.payload,
                  "metadata": getattr(original_event, "metadata", {}),
              },
              "error": {
                  "type": type(error).__name__,
                  "message": str(error),
              },
              "attempt_count": attempt_count,
              "timestamp": _iso_now(),
          },
      )
      # Deliver to subscribers WITHOUT triggering the retry loop (single attempt)
      await self._deliver_no_retry(dlq_event)
  ```
- [ ] Per-subscriber retry isolation: each subscriber's retry loop runs in its own `asyncio.create_task(...)` so a slow subscriber doesn't block others.

#### Part D — `A2ASubscriber` migration
- [ ] Add `skill_id: str = "apevo.event_receiver"` constructor parameter to `A2ASubscriber`. Replace the hardcoded constant in the payload.
- [ ] Remove A2A's previous "no retry, single attempt" behavior — A2A now follows the unified retry policy.

#### Part E — `subscriber_id` defaulting
- [ ] Each subscriber instance gets a stable `subscriber_id`:
  - If constructor `id=` was passed: use that string.
  - Else: SDK-generated, format `f"{type_name}-{counter}"` where `counter` is per-type and monotonic across the process. Use a class-level counter or a module-level registry.

### Tasks (continued)
- [ ] Write `tests/v022/test_event_delivery_semantics.py` covering all 4 conformance fixture cases (see fixture for exact behavior).
- [ ] Write `tests/conformance/test_v022_event_delivery.py` that loads `/Users/tercel/WorkSpace/aipartnerup/apcore/conformance/fixtures/event_delivery_semantics.json` and asserts each case.
- [ ] Commit: `feat: implement unified event delivery semantics with retry, DLQ, on_failure (apcore #61)`

---

## Issue #65 — Registry on_load ordering

This is the highest-risk refactor. Save for last.

### Tasks

#### Part A — deferred-publish refactor of `Registry.register`
- [ ] Restructure `Registry.register` in `src/apcore/registry/registry.py`:
  ```
  Step 1: acquire registry lock (RLock)
  Step 2: validate module_id format and module structure
  Step 3: check duplicate against visible store AND in-flight loading set
          → raise InvalidInputError(code=DUPLICATE_MODULE_ID) on collision
  Step 4: add module_id to in-flight loading set
  Step 5: release registry lock
  Step 6: acquire per-module init lock (from dict[str, threading.Lock])
  Step 7: invoke module.on_load() if defined
          on failure:
            - re-acquire registry lock, remove from in-flight set, release
            - emit apcore.registry.module_load_failed event
            - re-raise original exception
  Step 8: re-acquire registry lock briefly; atomically:
            - insert into visible store
            - remove from in-flight loading set
          release registry lock
  Step 9: release per-module init lock
  Step 10: emit register event to subscribers (existing behavior)
  ```
- [ ] Discovery APIs (`get`, `list`, `get_definition`) MUST consult only the visible store — the in-flight set is NOT visible.
- [ ] Add `apcore.registry.module_load_failed` event with payload `{module_id, callback_name, error_type, error_message, timestamp}`.

#### Part B — concurrency tests
- [ ] Write `tests/v022/test_registry_load_ordering.py`:
  - successful `on_load` → module visible after `register()` returns
  - failing `on_load` → `register()` re-raises, module NOT visible, DLQ event emitted
  - concurrent same-ID registration via `threading.Thread` or `asyncio.gather` → one succeeds, one raises `DUPLICATE_MODULE_ID`
  - concurrent distinct-ID with overlapping 50ms `on_load` delays → wall-clock < 90ms (proves per-module parallelism)
- [ ] Add `tests/conformance/test_v022_registry_ordering.py` loading the `registry_load_ordering.json` fixture.
- [ ] Commit: `feat: enforce on_load completion before module visibility via deferred-publish (apcore #65)`

---

## Cross-cutting

After all 5 issues are committed:

- [ ] Update `CHANGELOG.md` — append `### Added` and `### Changed` sub-sections **inside the existing `## [0.22.0]` block** (do NOT create a new release block; #60 already opened `[0.22.0]`). Mirror the apcore spec CHANGELOG `[0.22.0]` shape: 5 Added bullets (one per issue) + 2 Changed bullets (A2A retry; registry concurrent same-ID).
- [ ] `pyproject.toml` version is already `0.22.0` — no bump needed.
- [ ] Confirm new public API is re-exported from `src/apcore/__init__.py` (`StreamingModule`, `StreamingInterfaceError`, `RetryConfig`, plus any new ContextKey constants).
- [ ] Run full test suite: `pytest -x` — all tests MUST pass.
- [ ] Run mypy / pyright if the project uses static type checking: `python -m mypy src/apcore/` — MUST pass.

## Success criteria

- Branch `feat/v022-hardening-61-65` exists with 5 commits (one per issue) — see suggested commit titles above.
- All existing tests still pass.
- New tests cover the conformance fixture cases.
- `CHANGELOG.md` `## [0.22.0]` block has both `### Added` (with 5 bullets for #61–#65) and `### Changed` (with 2 bullets) sub-sections.
- No push to remote. No merge to main.

## Blockers — STOP and report if you hit

- Existing test fails after your change → root-cause; don't paper over.
- Conformance fixture case has ambiguous interpretation against spec → cite the ambiguity, propose interpretation, STOP.
- `Registry.register` refactor breaks integration test → likely an internal caller relies on insertion-then-on_load ordering; investigate, document, ask before changing the caller.
