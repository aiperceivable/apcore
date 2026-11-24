<div align="center">
  <img src="https://raw.githubusercontent.com/aiperceivable/apcore/main/apcore-logo.svg" alt="apcore logo" width="200"/>
</div>

# apcore

[![TypeScript](https://img.shields.io/badge/TypeScript-Node_20+-blue.svg)](https://github.com/aiperceivable/apcore-typescript)
[![License](https://img.shields.io/badge/license-Apache%202.0-green.svg)](https://opensource.org/licenses/Apache-2.0)
[![OpenSSF Best Practices](https://www.bestpractices.dev/projects/12294/badge)](https://www.bestpractices.dev/projects/12294)

**AI-Perceivable Core**

> **Build once, invoke by Code or AI.**
> Every call validated, authorized, and evidenced.

A governed runtime for agent-callable capabilities — schema, ACL, approval, and audit enforced at every call.

apcore is an AI-Perceivable module standard that makes every interface naturally perceivable and understandable by AI through enforced Schema definitions and behavioral annotations. It provides schema validation, access control, middleware pipelines, and observability built in.

## Features

- **Schema-driven modules** — Define input/output schemas with TypeBox for runtime validation
- **Executor pipeline** — Secured execution lifecycle: context → call chain guard → lookup → ACL → approval gate → middleware before → validation → execute → output validation → middleware after → return
- **Registry system** — File-based module discovery with metadata, dependencies, and topological ordering; multi-class discovery from a single file
- **Binding loader** — YAML-based module registration for no-code integration
- **Access control (ACL)** — Pattern-based rules with identity types, roles, and call-depth conditions
- **Approval system** — Pluggable approval gate in the executor pipeline with sync and async (polling) flows, built-in handlers, and tracing integration
- **Middleware** — Onion-model middleware with before/after/onError hooks and error recovery; built-in `CircuitBreakerMiddleware` (CLOSED/OPEN/HALF_OPEN); tracing is provided by the observability `TracingMiddleware`
- **Observability** — Tracing (spans + `BatchSpanProcessor` + exporters), metrics (counters + histograms + Prometheus export with `/metrics`/`/healthz`/`/readyz`), structured logging with `RedactionConfig`
- **System modules** — Built-in `system.*` modules for AI bidirectional introspection: health, manifest, usage, and runtime control (`update_config`, `reload_module`, `toggle_feature`). Audit trail via `AuditStore`, config persistence via `overridesPath`, usage metrics in Prometheus, bulk reload via `path_filter` glob
- **Event system** — `EventEmitter` with subscriber-level `CircuitBreakerWrapper`, built-in `FileSubscriber`, `StdoutSubscriber`, `FilterSubscriber`, and pluggable custom types
- **Async tasks** — `AsyncTaskManager` with injectable `TaskStore` (bring your own Redis/Postgres backend), `RetryConfig` with exponential backoff, and opt-in background reaper
- **Schema export** — JSON/YAML schema export with strict and compact modes
- **Caching & pagination annotations** — `cacheable`, `cacheTtl`, `cacheKeyFields` for result caching; `paginated`, `paginationStyle` for paginated modules
- **Config Bus** — Namespace-based configuration registry with typed access, env prefix dispatch, hot-reload, and external config mounting (`Config.registerNamespace()`, `config.namespace()`, `config.bind<T>()`, `config.mount()`)

## What's New in v0.28.0

- **SECURITY: an unevaluable approval rule stepped aside and the call ran unapproved** (apcore#109, spec v1.29.0 §6.1.1 rule 5) — a narrow `approval: required` rule ahead of a broad `allow` is the shape argument-scoped approval was written for, and when the narrow rule's condition could not be *evaluated* it did the one thing §6.1.1 told it to: step aside. The broad rule then granted, carrying no requirement of its own, so `git push --force` came back `allow` with `approvalRequired: false` — the exact call the operator gated. A misspelled predicate or an unregistered condition key reaches this **with a projection present**, on the ordinary Executor pipeline, and `defaultEffect: 'allow'` reaches it with no second rule at all. The requirement is now **pending** rather than discarded: it composes by disjunction with whatever grants next (a later rule *or* the default effect, which makes `approvalRequired: true` with `matchedRuleIndex: null` legal), and a `deny` decision clears it. A rule whose patterns do not match this call raises nothing; a rule whose own `callers` / `targets` is malformed does, because its scope cannot be read.
- **SECURITY: an ACL condition that could not be *evaluated* silently disabled the `deny` rule carrying it** (apcore#100, spec v1.22.0 §6.1.1) — "a handler answered no" and "no answer was obtainable" reached the rule loop identically, and both meant *this rule does not match*. That is safe in one direction only: an `allow` rule that cannot evaluate its condition does not grant, but a `deny` rule that cannot evaluate its condition does not block. One misspelled key (`role:` for `roles:`) turned a rule its author believed was blocking into decoration. An unevaluable condition now resolves the rule toward refusing access — a `deny` rule takes effect, an `allow` rule still does not grant — and `handlerError` names the condition path.
- **SECURITY: a `callers` or `targets` written as a string instead of a list turned an `allow` rule into a wildcard** (apcore#106, spec v1.25.0 §6.1.4.1) — a bare string is iterable, so `callers: "admin.*"` was read character by character and its `*` matched everything. Such a rule is now unevaluable: it does not grant, and `check()` does not throw.
- **SECURITY: an unknown or reserved ACL rule key was dropped in silence** (apcore#107, spec v1.27.0 §6.1) — a rule carrying `actions: ["describe"]`, written to mean introspection only, loaded cleanly and granted **execute**. The key set is now closed to `callers`, `targets`, `effect`, `description`, `conditions`, and anything else fails the load with `ACLRuleError`.
- **A rule's `effect` accepted any string outside the YAML loader** (apcore#111, spec v1.30.0 §6.1.5) — the same defect one level down: there an unknown *key* was dropped in silence, here a legal key's *value* was. `ACL.load()` rejected `effect: "Allow"` while `new ACL([...])` and `addRule()` accepted it, and the accepted value was then read as `deny` — so under `defaultEffect: 'allow'` a rule written to **permit** denied everything it matched, with no error and nothing from `validateRules()`. `allow` and `deny` are now the only values accepted at **every** entry point, `defaultEffect` on the same terms.
- **An ACL rule can now ask a human about *this* call** (apcore#108, spec v1.28.0 §6.1.6–§6.1.8) — the ACL could *refuse* on arguments and an `ApprovalHandler` could *wave through* on arguments, but nothing could *ask*, so gating `git push --force` meant gating every `git push`. A rule gains an optional `approval: required` beside `effect` (rejected on a `deny` rule — the combination means nothing), plus one built-in `arguments` condition with `has_key` / `has_all_keys` / `has_none_of`. No predicate reads a value: it reads a **governance projection** of key names and JSON types computed at Step 3, never `redactedInputs`. `checkAccess()` returns the structured `AccessDecision`; the legacy `check()` **fails closed** on an approval requirement, because a non-Executor caller can only read a boolean as "let it through". The Step 5 gate and `validate()` both fire on the **union** of the module annotation, the ACL decision and `gateDestructive` — and an `ExecutionPolicy` may add a requirement but never remove one the ACL set.
- **A context-independent precheck runs before evaluation** (spec v1.25.0 §6.1.4) — the whole `conditions` tree is walked for structural and registry faults without a context and without running a handler, *before* the "no context supplied" check. That closes a bypass where a misspelled key on a context-less call escaped §6.1.1 entirely. Because the precheck is exhaustive and handler-free, its diagnostics are identical across SDKs.
- **`ACL` gains read-only accessors** (apcore#101, spec v1.23.0 §6.8) — `defaultEffect` and `rules` were `private` with no getter, so tooling had to re-read and re-parse the ACL file to recover a value the loaded object already held.
- **`validateRules()`** — a deploy-time validator reporting every rule that fails the precheck, with `syncResolvable` / `asyncResolvable` reported separately, since a key registered only as an async handler resolves under `asyncCheck()` and not under `check()`.
- **Policy resolution receives the call site** (apcore#102, spec v1.24.0 §7.9.6) — `resolve()` takes an optional `PolicyCallSite`. Built-in pattern rules must not consult it, so existing verdicts are unchanged; it exists so the call site can reach the audit trail. `_approvalToken` is now stripped before policy resolution, not merely before the module runs.

## What's New in v0.27.0

- **BREAKING (security): a denied caller no longer sees module-level introspection from `validate()`** (apcore#96, spec v1.13.0 §12.8.5.1) — `validate()` looked the module up and ran `preflight()` / `preview()` on the strength of that lookup alone, so a caller the ACL had just denied still made module-authored code run and still received what it returned: for a command-wrapping module, the resolved binary and its argv. When the `acl` check fails, neither hook runs, no `module_preflight` / `module_preview` check is emitted, and `predictedChanges` stays empty. The failed `acl` check itself is still reported, so a denied caller still learns *why*. A failed `schema` check does **not** suppress introspection — a permitted caller is entitled to the module's account of what would happen even when its inputs are malformed.
- **`dependencies` is a parsed field on the module descriptor** (apcore#90 follow-up, spec v1.18.0) — `getDefinition(moduleId).dependencies` returns what the caller declared. `system.manifest.*` had been reporting `dependencies: []` for every module that declared them, because it read `descriptor.metadata['dependencies']` and `mergeModuleMetadata` extracts `dependencies` as a canonical field, so it never landed in the metadata bag.

## What's New in v0.26.0

- **`ExecutionPolicy` / `PolicyRule` execution-time governance** (#76) — An external policy can force or exempt approval on already-registered modules by ID pattern, make a `destructive` annotation imply approval via `gateDestructive`, and fail **closed** with `strict: true` when a gated module has no `ApprovalHandler`. The most specific matching rule wins; on a tie the more restrictive one does. See [Execution policy](#execution-policy) and [`examples/execution-policy.ts`](./examples/execution-policy.ts).
- **Governance events** (#77) — `apcore.approval.decision` and `apcore.policy.override` are published on the event bus alongside the existing `apcore.acl.denied`.

## What's New in v0.25.0

- **Config-driven ACL discovery** (#74, D-64) — `ACL.discover(config)` resolves `acl.root` (default `./acl`) relative to the config file's own directory and loads an ACL only if that path exists; a missing path attaches **no** ACL rather than synthesizing a default-deny. Auto-wired in the `APCore` constructor. See [`examples/acl-config-driven.ts`](./examples/acl-config-driven.ts).

## What's New in v0.24.0

- **Per-instance `ToggleState` isolation** (#71) — each `APCore` instance owns one `ToggleState` (`new APCore({ toggleState? })`, read-only via `client.toggleState`) threaded into both the write path (`system.control.toggle_feature`) and the read path (`Executor` → module lookup), so disabling a module on one instance no longer affects another in the same process.
- **Agent-governance conformance coverage** (#72) — drivers for the canonical `toggle_state_isolation.json` and `acl_agent_scoping.json` fixtures lock the AI-agent tool-governance scenario as a cross-language contract.

## What's New in v0.23.0

- **AI error-recovery metadata at the source** (#70) — Framework errors now carry a default `userFixable` resolved per error code (`USER_FIXABLE_BY_CODE`), plus filled `aiGuidance` defaults, so the recovery contract flows to every surface (MCP/CLI/A2A) from one module/error definition instead of being backfilled per adapter. At parity with apcore-python / apcore-rust 0.23.0, locked by the shared `error_recovery_metadata.json` conformance fixture.

## What's New in v0.22.0

- **`ContextKey<T>` typed context state** (#63) — Type-safe accessor for `Context.data` slots with `get`/`set`/`delete`/`exists`/`scoped`; also available via the `apcore-js/context-keys` subpath import for tree-shakeable consumers.
- **`StreamingModule` interface** (#62) — Formal streaming contract via `STREAMING_MARKER` symbol; `isStreamingModule()` detects implementations and emits `StreamingInterfaceError` when contract is violated.
- **Middleware duplicate detection** (#64) — `MiddlewareManager.add(mw, { allowDuplicate, identityKey })` rejects accidental double-registration; opt-out and custom identity keys supported for explicit duplicates.
- **Event retry + DLQ** (#61) — `EventSubscriber` now exposes a `retry` config field (max attempts + exponential backoff) and an optional `onFailure(event, error, attemptCount)` hook acting as a per-subscriber dead-letter queue.
- **Registry async deferred-publish** (#65) — `Registry.register()` returns a `Promise<void>`; modules with an async `onLoad()` stay hidden from `get()`/`has()` until load completes, and `apcore.registry.module_load_failed` is emitted on rejection.
- **Reserved-namespace query API** (#60) — `Config.reservedNamespaces` static getter and top-level `RESERVED_NAMESPACES` export let callers pre-validate namespace names before calling `Config.registerNamespace()`.

See [`examples/v022-tour.ts`](./examples/v022-tour.ts) for a runnable tour of each surface.

## Documentation

For full documentation, including Quick Start guides and API reference, visit:
**[https://aiperceivable.github.io/apcore/getting-started/](https://aiperceivable.github.io/apcore/getting-started/)**

## Requirements

- Node.js >= 20.0.0
- TypeScript >= 5.5

## Installation

```bash
npm install apcore-js
```

> **Note:** The npm package is published as `apcore-js` (the `apcore` name is reserved on npm). Python uses `apcore`, Rust uses the `apcore` crate.

## Browser support

`apcore-js` ships dual entry points via `package.json` conditional exports:

- **Node (default):** `./dist/index.js` — the full surface.
- **Browser:** `./dist/browser/index.js` — auto-selected by tree-shaking bundlers (Vite, Webpack, esbuild, Rollup) through the `"browser"` exports condition.

The browser bundle **excludes** Node-only surfaces:

- `Config` class, `BindingLoader`, `SchemaLoader` (filesystem-bound)
- All `system.*` sys-modules (control, manifest, health, usage, audit)
- Events runtime: `EventEmitter`, all built-in subscribers, `AsyncTaskManager`, `TaskStatus`
- Observability runtime: `TracingMiddleware`, `MetricsCollector`, exporters, `PrometheusExporter`
- `RESERVED_NAMESPACES`, `StreamingInterfaceError`, `DuplicateModuleIdError`

`ACL.load(yamlPath)` throws `ACLRuleError` in the browser because the filesystem is unavailable; ACL programmatic construction still works.

**Available in the browser:** `Registry` programmatic `register`/`get`/`list`/`iter`/`unregister`, `Executor`, `ACL` programmatic, `Context`, middleware pipeline, schema validation, all error classes, and the utility helpers.

## Canonicalizing a bare name

Use `canonicalizeName()` for one CLI command, operation name, or label. It is
available from both Node and browser entry points and returns a structured result
for every string, without throwing. Non-ASCII input is rejected before trimming;
ASCII edge punctuation is trimmed, camel/Pascal case becomes snake case, and each
internal punctuation run becomes one underscore. Existing underscores remain.

```typescript
import { canonicalizeName, type CanonicalNameResult } from 'apcore-js';

const valid: CanonicalNameResult = canonicalizeName('  GetHTTPResponse!  ');
console.log(valid);
// { originalName: '  GetHTTPResponse!  ', canonicalName: 'get_http_response', error: null }

const invalid: CanonicalNameResult = canonicalizeName('7z');
console.log(invalid);
// { originalName: '7z', canonicalName: null, error: 'invalid_start' }

if (invalid.error !== null) {
  console.log(`Choose a different operation name: ${invalid.error}`);
}
```

Diagnostics are `empty_name`, `non_ascii`, `invalid_start`, and `name_too_long`.
A canonical segment starts with an ASCII lowercase letter and is at most 192
characters after conversion. The helper never invents a prefix or truncates a
name. Reservation and collisions are checked during registration; `system` alone
is a valid segment. This is not full module-ID normalization:
`normalizeToCanonicalId(localId, language)` retains its strict Algorithm A02
behavior and language-specific path separators.

## Quick Start

### Simplified Client (Recommended)

The `APCore` client provides a unified entry point that manages Registry and Executor for you:

```typescript
import { Type } from '@sinclair/typebox';
import { APCore } from 'apcore-js';

const client = new APCore();

// Register a module
client.module({
  id: 'math.add',
  description: 'Add two numbers',
  inputSchema: Type.Object({ a: Type.Number(), b: Type.Number() }),
  outputSchema: Type.Object({ sum: Type.Number() }),
  execute: (inputs) => ({ sum: (inputs.a as number) + (inputs.b as number) }),
});

// Call, validate, stream — all from one client
const result = await client.call('math.add', { a: 10, b: 5 });
// => { sum: 15 }

const preflight = await client.validate('math.add', { a: 10, b: 5 });
// => { valid: true, checks: [...], requiresApproval: false, errors: [] }
```

### Enabling sys_modules for control/events APIs

`APCore.disable`, `APCore.enable`, `APCore.on`, and `APCore.off` are gated on the
optional system-modules subsystem. They require a `Config` instance with
`sys_modules.enabled: true` to be passed into the `APCore` constructor —
otherwise the calls throw with a clear "sys_modules must be enabled" message.

```typescript
import { APCore, Config } from 'apcore-js';

// Inline config (equivalent to writing the same YAML and calling Config.load):
const config = new Config({
  sys_modules: {
    enabled: true,
    // Optional sub-systems:
    events: { enabled: true },     // required for on/off (event emitter)
    control: { enabled: true },    // required for disable/enable (toggle)
  },
});

const client = new APCore({ config });

// Now safe to call:
await client.disable('math.add');
await client.enable('math.add');
// Enabling/disabling a module fires the canonical apcore.module.toggled event;
// read event.data.enabled to see the new state (see examples/events.ts).
client.on('apcore.module.toggled', (event) => {
  console.log('Toggled:', event.data.module_id, 'enabled:', event.data.enabled);
});
```

If you do not need these control/events APIs, omit the `Config` entirely (as in
the basic Quick Start above).

### Advanced: Manual Registry + Executor

```typescript
import { Type } from '@sinclair/typebox';
import { FunctionModule, Registry, Executor } from 'apcore-js';

const greet = new FunctionModule({
  execute: (inputs) => ({ greeting: `Hello, ${inputs.name}!` }),
  moduleId: 'example.greet',
  inputSchema: Type.Object({ name: Type.String() }),
  outputSchema: Type.Object({ greeting: Type.String() }),
  description: 'Greet a user',
});

const registry = new Registry();
registry.register('example.greet', greet);

const executor = new Executor({ registry });
const result = await executor.call('example.greet', { name: 'World' });
// => { greeting: 'Hello, World!' }
```

## API Overview

| Class | Description |
|-------|-------------|
| `APCore` | High-level client — register modules, call, stream, validate, listModules, describe, on/off, disable/enable. Note: `disable`, `enable`, `on`, and `off` require `sys_modules.enabled: true` in the `Config` passed to `APCore` (see [Quick Start: Enabling sys_modules](#enabling-sys_modules-for-controlevents-apis)). |
| `Registry` | Module storage — discover, register, get, list, watch |
| `Executor` | Execution engine — call with middleware pipeline, ACL, approval |
| `Context` | Request context — trace ID, identity, call chain, cancel token |
| `Config` | Configuration — load from YAML, namespace bus, get/set/bind values |
| `ACL` | Access control — rule-based caller/target authorization |
| `ExecutionPolicy` / `PolicyRule` | Execution-time governance overrides (#76) — force/exempt approval by ID pattern, `gateDestructive`, `strict` fail-closed |
| `Middleware` | Pipeline hooks — before/after/onError interception |
| `CircuitBreakerMiddleware` | Per-(module, caller) circuit breaker — CLOSED/OPEN/HALF_OPEN with configurable threshold and cooldown |
| `TracingMiddleware` | Span tracing — `apcore.module.execute` spans kept as a stack in `_apcore.mw.tracing.spans` with `parent_span_id` links; self-contained, no runtime `@opentelemetry/*` dependency |
| `EventEmitter` | Event system — subscribe, emit, flush |
| `CircuitBreakerWrapper` | Subscriber-level circuit breaker — protects `EventEmitter` subscribers from cascading failures |
| `AsyncTaskManager` | Background task execution — injectable store, retry with backoff, opt-in reaper |
| `PrometheusExporter` | HTTP metrics server — `/metrics`, `/healthz`, `/readyz`; optional `usageCollector` for usage gauges |
| `InMemoryAuditStore` | Control module audit log — records actor, action, before/after change for every control call |

## Configuration

### Config Bus

`Config` acts as an ecosystem-level Config Bus. Any package can register a namespace with optional JSON Schema validation, environment variable prefix, and defaults.

```typescript
import { Config } from 'apcore-js';

// Register a namespace (class-level, shared across all Config instances).
// registerNamespace takes a single options object; `name` is a field on it.
Config.registerNamespace({
  name: 'myPlugin',
  envPrefix: 'MY_PLUGIN',
  defaults: { timeout: 5000, retries: 3 },
  schema: {
    type: 'object',
    properties: {
      timeout: { type: 'number' },
      retries: { type: 'number' },
    },
  },
});

const config = Config.load('apcore.yaml');

// Dot-path access with namespace resolution
const timeout = config.get('myPlugin.timeout');   // 5000 (or env override)

// Full namespace subtree
const pluginConfig = config.namespace('myPlugin');

// Typed access — pass a class constructor; its constructor receives the namespace dict
class MyPluginConfig {
  timeout: number;
  retries: number;
  constructor(data: Record<string, unknown>) {
    this.timeout = (data['timeout'] as number) ?? 5000;
    this.retries = (data['retries'] as number) ?? 3;
  }
}
const typed = config.bind('myPlugin', MyPluginConfig);

// Mount an external config source (e.g. an existing config file)
config.mount('myPlugin', { fromFile: './my-plugin.yaml' });
// Or from an in-memory object:
config.mount('myPlugin', { fromDict: { timeout: 10000 } });

// Introspect registered namespaces
// => Array<{ name: string; envPrefix: string | null; hasSchema: boolean }>
const namespaces = Config.registeredNamespaces();
```

### Environment Variable Overrides

Merge priority (highest wins): **environment variables > config file > namespace defaults**.

Two prefix conventions are supported:

| Convention | Applies to | Example |
|------------|------------|---------|
| `APCORE_` + `KEY_PATH` | Legacy flat keys | `APCORE_EXECUTOR_DEFAULT__TIMEOUT=5000` |
| `APCORE_` + namespace prefix | apcore sub-package namespaces | `APCORE_OBS_REDACTION_REPLACEMENT='[hidden]'` |

Within the suffix, a single `_` maps to `.` (a path separator) and `__` maps to a
literal `_`. So `APCORE_EXECUTOR_DEFAULT__TIMEOUT` sets `executor.default_timeout`
— writing `APCORE_EXECUTOR_DEFAULT_TIMEOUT` would instead target the key
`executor.default.timeout`, which does not exist. This matches apcore-python and
apcore-rust.

Legacy `APCORE_*` overrides apply in namespace mode too: they are merged into the
`apcore` namespace (PROTOCOL_SPEC §9.6.2), so the example above also resolves via
`config.get('apcore.executor.default_timeout')`.

apcore pre-registers the following namespaces and env prefixes:

| Namespace | Env prefix | Wraps |
|-----------|-----------|-------|
| `observability` | `APCORE_OBSERVABILITY` | `observability.*` — tracing, metrics, logging, error_history, platform_notify |
| `obs` | `APCORE_OBS` | `obs.redaction.*` — `sensitive_keys`, `regex_patterns`, `replacement` |
| `sys_modules` | `APCORE_SYS` | `sys_modules.*` — health, manifest, usage, control, events |

> Namespace names are **snake_case**, matching apcore-python and the YAML config
> keys: use `config.namespace('sys_modules')`, not `'sysModules'`.

Sub-packages use their own `APCORE_` prefixed name (e.g. `APCORE_MCP` for apcore-mcp). The longest-prefix-match dispatch algorithm disambiguates correctly (`APCORE_OBS_*` vs `APCORE_OBSERVABILITY_*`).

### Hot Reload

`config.reload()` re-reads the source YAML, re-detects legacy/namespace mode, re-applies all namespace defaults and env overrides, re-validates, and re-reads any mounted files.

```typescript
const config = Config.load('apcore.yaml');
// ... runtime config change on disk ...
config.reload(); // picks up all changes
```

### YAML File Format

Configuration files support two modes. **Legacy mode** (no `apcore:` key) is fully backward compatible. **Namespace mode** is activated when an `apcore:` top-level key is present; each namespace occupies its own top-level section. The `_config` reserved namespace controls validation behavior.

```yaml
# Namespace mode
apcore:
  version: "1.0.0"

_config:
  strict: true

obs:
  redaction:
    sensitive_keys: ["*password*", "*token*"]
    replacement: "***REDACTED***"

myPlugin:
  timeout: 10000
  retries: 5
```

### Configuration keys that do nothing

Ten declared keys reach no consumer in any SDK. They parse, they validate, they pass strict
mode — and setting them has **no effect**. They are deprecated as of spec v1.39.0 and are
removed no earlier than v2.0; loading a configuration that declares one of them now warns and
names it.

```yaml
# None of the following does anything today. Shown so you can recognise it.
logging:
  level: "info"              # inert — configure your host application's logger
  format: "json"             # inert

observability:
  tracing:
    enabled: true            # inert — construct and install TracingMiddleware yourself
    sampling_rate: 0.1       # inert, and the sharpest case: sampling is decided by
                             # `new TracingMiddleware(exporter, samplingRate,
                             # samplingStrategy)`, constructor arguments no configuration
                             # key feeds, defaulting to 1.0 / 'full'. Ask for 10% here
                             # and you still get 100%.
    exporter: "stdout"       # inert — an exporter is an object, not a name
  metrics:
    enabled: true            # inert
    exporter: "prometheus"   # inert

acl:
  audit:
    enabled: true            # inert — pass an audit logger to the ACL constructor
    include_denied: true     # inert
    log_level: "info"        # inert
```

The same three audit settings are *also* accepted as an `audit:` block at the root of an ACL
YAML file, and are equally inert there — no SDK has ever read it. Loading such a file now warns
too. Only the `audit:` block is called out: every other unrecognised root key in an ACL file is
still ignored in silence, exactly as before.

Observability and audit logging **do** work — they are wired programmatically, with
`TracingMiddleware` (see **API Overview**), `PrometheusExporter` (see **System Modules**) and the
`ACL` constructor's audit logger. Tracking:
[apcore#118](https://github.com/aiperceivable/apcore/issues/118), PROTOCOL_SPEC
§9.2.4 / §9.2.4.1.

There is a second, opposite trap. These keys are **not declared anywhere and read by nothing**,
so a configuration carrying them is *rejected* under `_config.strict: true` rather than merely
ignored:

| Not a key | Notes |
|-----------|-------|
| `observability.prometheus.*` | The Prometheus exporter's port and paths are constructor arguments — see the `PrometheusExporter` example under **System Modules**. Measured: `Unknown key 'observability.prometheus' (strict mode enabled)`. |
| `observability.health.*` | `/healthz` and `/readyz` are served by that same exporter, not configured in YAML. |
| `observability.redaction.*` | The **legacy** spelling. This SDK still reads `field_patterns` / `value_patterns` / `replacement` under it as a fallback, with a deprecation warning — but the key is undeclared, so strict mode rejects the document before the fallback can run. Write the canonical `obs.redaction.sensitive_keys` / `regex_patterns` / `replacement` instead; apcore-python does not honour the legacy spelling at all. |

### System Modules

`registerSysModules()` auto-registers the built-in `system.*` modules that let AI agents query, monitor, and control the apcore runtime. Enable them via `sys_modules.enabled: true` in config, and pass the optional hardening options for production use:

```typescript
import { registerSysModules, InMemoryAuditStore } from 'apcore-js';

const auditStore = new InMemoryAuditStore();

registerSysModules(registry, executor, config, null, {
  failOnError: true,              // throw on any registration failure (default: false)
  overridesPath: '/etc/apcore/overrides.yaml',  // persist runtime changes across restarts
  auditStore,                     // record every control-module action with actor + change
});

// Available system modules:
// system.health.summary / system.health.module     — health status + error rates
// system.manifest.module / system.manifest.full    — module introspection
// system.usage.summary / system.usage.module       — call counts + latency trends
// system.control.update_config                     — hot-patch config values
// system.control.reload_module                     — hot-reload from disk; supports path_filter glob
// system.control.toggle_feature                    — disable/enable modules at runtime

// Query the audit log after control calls:
const entries = auditStore.query({ moduleId: 'system.control.update_config' });
// entries[0] = { timestamp, action, targetModuleId, actorId, actorType, traceId, change }
```

**Prometheus usage metrics** — wire `PrometheusExporter` with the `UsageCollector` returned by `registerSysModules`:

```typescript
import { PrometheusExporter, MetricsCollector } from 'apcore-js';

const ctx = registerSysModules(registry, executor, config);
const exporter = new PrometheusExporter({
  collector: new MetricsCollector(),
  usageCollector: ctx.usageCollector,  // adds apcore_usage_* metrics to /metrics
});
exporter.start({ port: 9090 });
// GET /metrics now includes:
//   apcore_usage_calls_total{module_id="math.add",status="success"} 5000
//   apcore_usage_error_rate{module_id="math.add"} 0.0004
//   apcore_usage_p99_latency_ms{module_id="math.add"} 45.0
```

### Bindings

`BindingLoader` registers modules from a YAML file instead of code — the
no-code integration path.

```yaml
# format-date.binding.yaml
spec_version: "1.0"
bindings:
  - module_id: "utils.format_date"
    # "<module path>:<exported symbol>". The module path goes to Node's
    # `import()` verbatim, so it must be a package specifier or an absolute
    # path — a bare relative name is NOT resolved against the YAML file.
    target: "/abs/path/to/format-date.ts:formatDateString"
    description: "Format a date string into a specified format"
    version: "1.0.0"
    auto_schema: true
```

```typescript
import { BindingLoader, Executor, Registry } from 'apcore-js';

const registry = new Registry();
const loader = new BindingLoader();
await loader.loadBindings('./format-date.binding.yaml', registry);
// Or every *.binding.yaml in a directory:
// await loader.loadBindingDir('./bindings', registry);

const executor = new Executor({ registry });
await executor.call('utils.format_date', {
  dateString: '2024-01-15',
  outputFormat: '%B %d, %Y',
});
```

The target is invoked as `func(inputs, context)`. Schemas come from one of four
mutually exclusive modes: inline `input_schema` / `output_schema`, an external
`schema_ref`, `auto_schema` (infers from the target module's exported
`inputSchema` / `outputSchema`), or — with none specified — a permissive
fallback. Pass `new BindingLoader({ trustedPackagePrefixes: [...] })` to refuse
importing any target outside an allowlist.

See [`examples/bindings/format-date/`](./examples/bindings/format-date/) for a
runnable end-to-end binding.

### Execution policy

`ExecutionPolicy` (with `PolicyRule`) applies governance to **already-registered**
modules at execution time, without touching their code or annotations. It is
consulted by the approval gate (pipeline step 5).

```typescript
import { APCore, ExecutionPolicy, PolicyRule } from 'apcore-js';

const policy = new ExecutionPolicy(
  [
    new PolicyRule('orders.delete_*', {
      requiresApproval: true,
      reason: 'destructive order ops need human sign-off',
    }),
    // A rule can also exempt: requiresApproval: false.
    new PolicyRule('orders.read_*', { requiresApproval: false }),
  ],
  {
    // Any module annotated destructive: true also needs approval.
    gateDestructive: true,
    // A module that needs approval but has no ApprovalHandler fails CLOSED
    // (ApprovalDeniedError) instead of silently executing.
    strict: true,
  },
);

const client = new APCore({ policy, approvalHandler });
// Runtime swap: executor.setPolicy(otherPolicy)
```

Pattern matching reuses the ACL wildcard semantics (Algorithm A08) and
specificity scoring (A10): the most specific matching rule wins, and on a tie the
more restrictive rule does.

An operator-authored YAML/JSON governance document loads via
`ExecutionPolicy.fromObject(parsed)`:

```yaml
gate_destructive: true
strict: true
rules:
  - pattern: "orders.delete_*"
    requires_approval: true
    reason: "destructive order operations need human sign-off"
```

Parsing is **strict**: unknown keys throw, and `gate_destructive` / `strict` must
be real booleans — a typo or a `"false"` string cannot silently disable a
control.

See [`examples/execution-policy.ts`](./examples/execution-policy.ts) for a
runnable end-to-end demo of all three behaviours.

### Error Codes

New error codes added in v0.15.0:

| Code | Description |
|------|-------------|
| `CONFIG_NAMESPACE_DUPLICATE` | `Config.registerNamespace()` called with an already-registered name |
| `CONFIG_NAMESPACE_RESERVED` | `Config.registerNamespace()` called with a reserved name (e.g. `_config`) |
| `CONFIG_ENV_PREFIX_CONFLICT` | Two namespaces declare the same `envPrefix` |
| `CONFIG_MOUNT_ERROR` | `config.mount()` cannot read or parse the external source |
| `CONFIG_BIND_ERROR` | `config.bind<T>()` or `config.getTyped<T>()` type guard fails |
| `ERROR_FORMATTER_DUPLICATE` | `ErrorFormatterRegistry.register()` called for an already-registered surface |

New error codes added in v0.20.0:

| Code | Description |
|------|-------------|
| `CIRCUIT_BREAKER_OPEN` | `CircuitBreakerMiddleware` short-circuited a call because the circuit is OPEN |
| `MODULE_RELOAD_CONFLICT` | Both `module_id` and `path_filter` supplied to `system.control.reload_module` |
| `SYS_MODULE_REGISTRATION_FAILED` | `registerSysModules()` with `failOnError: true` and a module failed to register |
| `MODULE_ID_CONFLICT` | Two classes in the same file produce the same module ID segment (`discoverMultiClass`) |
| `INVALID_SEGMENT` | A derived class segment does not match `^[a-z][a-z0-9_]*$` |
| `ID_TOO_LONG` | A derived module ID exceeds 192 characters |

### Event Type Canonical Names

apcore 0.15.0 resolved two event-type collisions in favor of dot-namespaced canonical
names. The legacy short-form aliases (`module_health_changed`, `config_changed`) were
emitted during the 0.15.x transition and have been removed as of 0.18.0.

| Event name | Meaning |
|------------|---------|
| `"apcore.module.toggled"` | Module enabled/disabled toggle |
| `"apcore.health.recovered"` | Error-rate recovery after spike |
| `"apcore.config.updated"` | Config key updated at runtime |
| `"apcore.module.reloaded"` | Module reloaded from disk |

Naming convention: `apcore.*` is reserved for core events. Adapter packages use their own prefix (`apcore-mcp.*`, `apcore-a2a.*`, `apcore-cli.*`).

---

## Examples

The `examples/` directory contains runnable demos:

---

### `simple-client` — APCore client with module registration and calls

Initializes an `APCore` client, registers modules inline, and calls them.

```typescript
import { Type } from '@sinclair/typebox';
import { APCore } from 'apcore-js';

const client = new APCore();

client.module({
  id: 'math.add',
  description: 'Add two integers',
  inputSchema: Type.Object({ a: Type.Number(), b: Type.Number() }),
  outputSchema: Type.Object({ sum: Type.Number() }),
  execute: (inputs) => ({ sum: (inputs.a as number) + (inputs.b as number) }),
});

client.module({
  id: 'greet',
  description: 'Greet a user by name',
  inputSchema: Type.Object({
    name: Type.String(),
    greeting: Type.Optional(Type.String()),
  }),
  outputSchema: Type.Object({ message: Type.String() }),
  execute: (inputs) => ({
    message: `${(inputs.greeting as string) || 'Hello'}, ${inputs.name}!`,
  }),
});

const result = await client.call('math.add', { a: 10, b: 5 });
console.log(result); // { sum: 15 }

const greetResult = await client.call('greet', { name: 'Alice' });
console.log(greetResult); // { message: 'Hello, Alice!' }
```

---

### `greet` — Minimal FunctionModule

Demonstrates the core `FunctionModule` structure with TypeBox schemas.

```typescript
import { Type } from '@sinclair/typebox';
import { FunctionModule } from 'apcore-js';

export const greetModule = new FunctionModule({
  moduleId: 'greet',
  description: 'Greet a user by name',
  inputSchema: Type.Object({ name: Type.String() }),
  outputSchema: Type.Object({ message: Type.String() }),
  execute: (inputs) => ({ message: `Hello, ${inputs.name}!` }),
});
```

---

### `get-user` — Readonly + idempotent annotations

Shows behavioral annotations and simulated database lookup.

```typescript
import { Type } from '@sinclair/typebox';
import { FunctionModule, createAnnotations } from 'apcore-js';

const users: Record<string, { id: string; name: string; email: string }> = {
  'user-1': { id: 'user-1', name: 'Alice', email: 'alice@example.com' },
  'user-2': { id: 'user-2', name: 'Bob', email: 'bob@example.com' },
};

export const getUserModule = new FunctionModule({
  moduleId: 'user.get',
  description: 'Get user details by ID',
  inputSchema: Type.Object({ userId: Type.String() }),
  outputSchema: Type.Object({
    id: Type.String(),
    name: Type.String(),
    email: Type.String(),
  }),
  // `ModuleAnnotations` is a total interface — `createAnnotations()` fills the
  // rest (cacheable, cacheTtl, cacheKeyFields, paginated, paginationStyle,
  // extra) from `DEFAULT_ANNOTATIONS`. A partial object literal will not
  // compile.
  annotations: createAnnotations({
    readonly: true,
    destructive: false,
    idempotent: true,
    requiresApproval: false,
    openWorld: true,
    streaming: false,
  }),
  execute: (inputs) => {
    const user = users[inputs.userId as string];
    if (!user) {
      return { id: inputs.userId as string, name: 'Unknown', email: 'unknown@example.com' };
    }
    return { ...user };
  },
});
```

---

### `send-email` — Full-featured: annotations, examples, metadata, ContextLogger

Demonstrates destructive annotations, `ModuleExample` for AI-perceivable documentation, metadata, and `ContextLogger` usage.

```typescript
import { Type } from '@sinclair/typebox';
import { FunctionModule, ContextLogger, createAnnotations } from 'apcore-js';
import type { Context } from 'apcore-js';

export const sendEmailModule = new FunctionModule({
  moduleId: 'email.send',
  description: 'Send an email message',
  inputSchema: Type.Object({
    to: Type.String(),
    subject: Type.String(),
    body: Type.String(),
    // `x-sensitive: true` (or a `_secret_` field-name prefix) is what makes the
    // executor redact the value in captured inputs/outputs and logs.
    apiKey: Type.String({ 'x-sensitive': true }),
  }),
  outputSchema: Type.Object({
    status: Type.String(),
    messageId: Type.String(),
  }),
  tags: ['email', 'communication', 'external'],
  version: '1.2.0',
  metadata: { provider: 'example-smtp', maxRetries: 3 },
  annotations: createAnnotations({
    readonly: false,
    destructive: true,
    idempotent: false,
    requiresApproval: false,
    openWorld: true,
    streaming: false,
  }),
  examples: [
    {
      title: 'Send a welcome email',
      inputs: { to: 'user@example.com', subject: 'Welcome!', body: '...', apiKey: 'sk-xxx' },
      output: { status: 'sent', messageId: 'msg-12345' },
      description: 'Sends a welcome email to a new user.',
    },
  ],
  execute: (inputs, context: Context) => {
    const logger = ContextLogger.fromContext(context, 'send_email');
    logger.info('Sending email', { to: inputs.to as string, subject: inputs.subject as string });
    const hash = Math.abs(
      (inputs.to as string).split('').reduce((a, c) => a + c.charCodeAt(0), 0) % 100000,
    );
    const messageId = `msg-${String(hash).padStart(5, '0')}`;
    logger.info('Email sent successfully', { messageId });
    return { status: 'sent', messageId };
  },
});
```

---

### `decorated-add` — `module()` function for creating modules

```typescript
import { Type } from '@sinclair/typebox';
import { module } from 'apcore-js';

export const addModule = module({
  id: 'math.add',
  description: 'Add two integers',
  inputSchema: Type.Object({ a: Type.Number(), b: Type.Number() }),
  outputSchema: Type.Object({ sum: Type.Number() }),
  execute: (inputs) => ({ sum: (inputs.a as number) + (inputs.b as number) }),
});
```

## Development

```bash
# Install dependencies
npm install

# Type check
npm run typecheck

# Run tests
npm test

# Run tests in watch mode
npm run test:watch

# Build
npm run build
```

## Testing

- Core executor pipeline
- Schema validation (strict mode, type coercion)
- Middleware chain (ordering, transforms, error recovery, circuit breaker)
- ACL enforcement (patterns, conditions, identity types)
- Registry system (scanner, metadata, entry points, dependencies, multi-class discovery)
- Binding loader (YAML loading, target resolution, schema modes)
- Observability (tracing, BatchSpanProcessor, metrics, Prometheus export, structured logging with redaction)
- Sensitive-field redaction resolves local JSON Schema `$ref` pointers, anchors,
  and in-document embedded `$id` resources with scoped anchors, including recursive
  object/array schemas, positional items, declared legacy tuples and
  combinator branches. It preserves sibling `x-sensitive` markers, never fetches
  external schemas or files, and leaves the original values and schema unchanged.
- Event system (circuit breaker wrapper, subscriber types, filter/file/stdout)
- System modules (health, manifest, usage, control, audit trail, overrides persistence, Prometheus usage metrics)
- Async tasks (pluggable store, retry backoff, reaper)
- Cross-language conformance suite (`tests/conformance.test.ts`) — canonical JSON fixtures from `apcore/conformance/fixtures/` run identically across Python, TypeScript, and Rust SDKs

## Links

- **Documentation:** [https://aiperceivable.github.io/apcore/getting-started/](https://aiperceivable.github.io/apcore/getting-started/)
- **Specification:** [https://github.com/aiperceivable/apcore](https://github.com/aiperceivable/apcore)
- **GitHub:** [https://github.com/aiperceivable/apcore-typescript](https://github.com/aiperceivable/apcore-typescript)
- **npm:** [https://www.npmjs.com/package/apcore-js](https://www.npmjs.com/package/apcore-js)
- **Issues:** [https://github.com/aiperceivable/apcore-typescript/issues](https://github.com/aiperceivable/apcore-typescript/issues)

## License

Apache-2.0

### Cancellation and execution cleanup

`CancelToken.cancel()` and executor timeouts interrupt the caller promptly. A
module's asynchronous `finally` block can still be running after that interruption.
Lifecycle owners can `await token.drain()` after stopping new work to join actual
module execution and finalizers, including child scopes. Call `drain()` from the
owner, never from a module tracked by the same token. A module that ignores
cancellation can keep graceful drain pending.

`AsyncTaskManager.shutdown()` joins its workers even when their records are already
cancelled or reclaimed, retains concurrency capacity until physical execution
finishes, and joins an in-flight reaper sweep. Store errors are reported after
owned work settles. This cleanup capability is additive; ordinary cancellation and
execution deadlines retain their prompt response behavior.
