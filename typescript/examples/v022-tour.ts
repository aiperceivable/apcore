// examples/v022-tour.ts — Tour of v0.22.0 features
// Run: node examples/v022-tour.ts   (Node 23+, or 22.6+ with --experimental-strip-types)
//      npx tsx examples/v022-tour.ts  (any Node)
// The file is an ESM module using top-level await; ts-node is not a dependency
// of this repo.
//
// Demonstrates all five v0.22.0 surfaces in a single runnable script.
// Sections that need external infrastructure (real A2A endpoint, durable DLQ)
// are marked with TODO and a stub implementation that runs locally.

import { Type } from '@sinclair/typebox';
import {
  APCore,
  Config,
  Context,
  ContextKey,
  type EventSubscriber,
  type ApCoreEvent,
  EventEmitter,
  FunctionModule,
  isStreamingModule,
  MiddlewareManager,
  Middleware,
  Registry,
  RESERVED_NAMESPACES,
  STREAMING_MARKER,
  type StreamingModule,
} from 'apcore-js';

// ---------------------------------------------------------------------------
// 1. ContextKey<T> typed context state (#63)
// ---------------------------------------------------------------------------
console.log('\n--- 1. ContextKey<T> typed context state (#63) ---');
const RequestIdKey = new ContextKey<string>('demo.requestId');
const RetryCountKey = new ContextKey<number>('demo.retryCount');

const ctx1 = Context.create();
RequestIdKey.set(ctx1, 'req-abc-123');
RetryCountKey.set(ctx1, 0);

console.log('requestId =', RequestIdKey.get(ctx1));
console.log('retryCount exists:', RetryCountKey.exists(ctx1));
const Scoped = RequestIdKey.scoped('sub');
console.log('scoped key name:', Scoped['name']);

// ---------------------------------------------------------------------------
// 2. StreamingModule interface (#62)
// ---------------------------------------------------------------------------
console.log('\n--- 2. StreamingModule interface (#62) ---');
class Counter implements StreamingModule {
  readonly moduleId = 'demo.counter';
  readonly description = 'Streams integers 0..n-1';
  readonly inputSchema = Type.Object({ n: Type.Number() });
  readonly outputSchema = Type.Object({ value: Type.Number() });
  readonly [STREAMING_MARKER] = true as const;

  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  async execute(_inputs: Record<string, unknown>, _context: Context): Promise<Record<string, unknown>> {
    throw new Error('Use stream()');
  }

  async *stream(
    inputs: Record<string, unknown>,
    _context: Context,
  ): AsyncGenerator<Record<string, unknown>> {
    const n = inputs['n'] as number;
    for (let i = 0; i < n; i++) {
      yield { value: i };
    }
  }
}
const counter = new Counter();
console.log('isStreamingModule(counter) =', isStreamingModule(counter));
for await (const chunk of counter.stream({ n: 3 }, Context.create())) {
  console.log('  chunk:', chunk);
}

// ---------------------------------------------------------------------------
// 3. Middleware duplicate detection (#64)
// ---------------------------------------------------------------------------
console.log('\n--- 3. Middleware duplicate detection (#64) ---');
const mgr = new MiddlewareManager();
class TimingMw extends Middleware {
  override before(
    _moduleId: string,
    _inputs: Record<string, unknown>,
    _context: Context,
  ): null {
    return null;
  }
}
mgr.add(new TimingMw());
// Duplicate identity (default: constructor name). Detection WARNS — it does not
// throw — so an existing application never breaks on an accidental double
// registration; the console.warn carries both registration sites.
mgr.add(new TimingMw());
console.log('Duplicate TimingMw registered — see the [apcore:middleware] warning above.');
mgr.add(new TimingMw(), { allowDuplicate: true });
console.log('Two TimingMw instances coexist via allowDuplicate.');
mgr.add(new TimingMw(), { identityKey: 'metrics-timing' });
console.log('Third TimingMw registered under custom identityKey.');

// ---------------------------------------------------------------------------
// 4. EventSubscriber retry + DLQ (#61)
// ---------------------------------------------------------------------------
console.log('\n--- 4. EventSubscriber retry + DLQ (#61) ---');
// TODO: in production wire a durable DLQ sink (S3, Postgres, Kafka topic).
const dlq: Array<{ event: ApCoreEvent; error: string; attempts: number }> = [];

class FlakySubscriber implements EventSubscriber {
  readonly subscriberId = 'flaky-demo';
  readonly retry = { maxAttempts: 3, initialBackoffMs: 10, maxBackoffMs: 100 };
  private _calls = 0;

  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  async onEvent(_event: ApCoreEvent): Promise<void> {
    this._calls++;
    throw new Error(`Synthetic failure (attempt ${this._calls})`);
  }

  onFailure(event: ApCoreEvent, error: Error, attemptCount: number): void {
    dlq.push({ event, error: error.message, attempts: attemptCount });
  }
}

const emitter = new EventEmitter();
emitter.subscribe(new FlakySubscriber());
emitter.emit({
  eventType: 'demo.heartbeat',
  moduleId: 'demo',
  timestamp: new Date().toISOString(),
  severity: 'info',
  data: { tick: 1 },
});
// Wait for retry/backoff to complete.
await new Promise((r) => setTimeout(r, 500));
console.log('DLQ entries:', dlq.length, '— last:', dlq[0]?.error, `(attempts=${dlq[0]?.attempts})`);

// ---------------------------------------------------------------------------
// 5. Registry async deferred-publish (#65)
// ---------------------------------------------------------------------------
console.log('\n--- 5. Registry async deferred-publish (#65) ---');
const reg = new Registry();
class SlowLoader extends FunctionModule {
  constructor() {
    super({
      moduleId: 'demo.slow',
      description: 'Module with async onLoad',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ ok: Type.Boolean() }),
      execute: () => ({ ok: true }),
    });
  }
  // TODO: in production, async onLoad typically warms caches, opens
  // connection pools, or fetches remote schemas.
  async onLoad(): Promise<void> {
    await new Promise((r) => setTimeout(r, 50));
  }
}
const loadPromise = reg.register('demo.slow', new SlowLoader());
console.log('Before await: has(demo.slow) =', reg.has('demo.slow')); // false — deferred
await loadPromise;
console.log('After  await: has(demo.slow) =', reg.has('demo.slow')); // true

// ---------------------------------------------------------------------------
// 6. Reserved-namespace query (#60)
// ---------------------------------------------------------------------------
console.log('\n--- 6. Reserved-namespace query (#60) ---');
console.log('RESERVED_NAMESPACES (top-level export):', Array.from(RESERVED_NAMESPACES));
console.log('Config.reservedNamespaces (static getter):', Array.from(Config.reservedNamespaces));
const candidate = 'myPlugin';
if (Config.reservedNamespaces.has(candidate)) {
  console.log(`'${candidate}' is reserved — cannot register.`);
} else {
  console.log(
    `'${candidate}' is free — safe to call Config.registerNamespace({ name: '${candidate}', ... }).`,
  );
}

// Touch APCore to demonstrate end-to-end client wiring is unchanged.
const _client = new APCore();
void _client; // suppress unused

console.log('\nv0.22.0 tour complete.');
