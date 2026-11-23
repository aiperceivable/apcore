/** Issue #123 regression guards beyond the shared fixture cases. */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { Type } from '@sinclair/typebox';
import { APCore } from '../src/client.js';
import { Config } from '../src/config.js';
import { Context } from '../src/context.js';
import { Executor } from '../src/executor.js';
import { CancelToken, ExecutionCancelledError } from '../src/cancel.js';
import { Registry } from '../src/registry/registry.js';
import { createEvent, type ApCoreEvent } from '../src/events/emitter.js';
import { registerSubscriberType, resetSubscriberRegistry } from '../src/sys-modules/registration.js';
import { SchemaValidator } from '../src/schema/validator.js';
import yaml from 'js-yaml';
import { ExecutionStrategy, PipelineEngine, type PipelineContext, type Step } from '../src/pipeline.js';

describe('D-134 abstract capability dependencies', () => {
  for (const unavailable of ['impure', 'no_match', 'failed']) {
    it(`skips transitive dependencies after an ${unavailable} producer while checking independent steps`, async () => {
      const invoked: string[] = [];
      const producer: Step = {
        name: 'producer', pure: unavailable !== 'impure', provides: ['alpha'],
        description: 'Abstract capability producer', removable: true, replaceable: true,
        matchModules: unavailable === 'no_match' ? ['other.*'] : null,
        async execute() { invoked.push('producer'); throw new Error('Producer failure'); },
      };
      const makeStep = (name: string, requires: string[], provides: string[]): Step => ({
        name, pure: true, requires, provides,
        description: 'Abstract capability consumer', removable: true, replaceable: true,
        async execute() { invoked.push(name); return { action: 'continue' }; },
      });
      const strategy = new ExecutionStrategy('abstract-capabilities', [producer, makeStep('dependent', ['alpha'], ['beta']), makeStep('transitive', ['beta'], []), makeStep('independent', [], [])]);
      const context: PipelineContext = {
        moduleId: 'demo.probe', inputs: {}, context: Context.create(), module: null,
        validatedInputs: null, output: null, validatedOutput: null, stream: false,
        outputStream: null, strategy, trace: null, dryRun: true,
      };
      const [, trace] = await new PipelineEngine().run(strategy, context);
      expect(invoked).toEqual(unavailable === 'failed' ? ['producer', 'independent'] : ['independent']);
      expect(trace.steps.find((step) => step.name === 'dependent')?.skipped).toBe(true);
      expect(trace.steps.find((step) => step.name === 'transitive')?.skipped).toBe(true);
      expect(trace.steps.find((step) => step.name === 'independent')?.skipped).toBe(false);
    });
  }
});

describe('D-151 conformance declaration', () => {
  it('declares the actual verified scope without claiming full certification', () => {
    const declaration = yaml.load(readFileSync(new URL('../apcore-conformance.yaml', import.meta.url), 'utf8')) as {
      implementation: { name: string; version: string; spec_version: string };
      conformance: { level: number; fixture_results: { fixtures: number; cases: number; passed: number; failed: number; skipped: number }; known_deviations: { feature: string; severity: string }[] };
    };
    const project = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8')) as { version: string };
    expect(declaration.implementation).toMatchObject({ name: 'apcore-typescript', version: project.version, spec_version: '1.65.0' });
    expect(declaration.conformance.level).toBe(0);
    expect(declaration.conformance.fixture_results).toMatchObject({ fixtures: 9, cases: 79, passed: 79, failed: 0, skipped: 0 });
    expect(declaration.conformance.known_deviations).toContainEqual(expect.objectContaining({ feature: 'Full fixture verification', severity: 'major' }));
  });
});

describe('registry watch startup lifetime', () => {
  it('unwatch during asynchronous setup prevents a leaked listener', async () => {
    const root = mkdtempSync(join(tmpdir(), 'apcore-watch-startup-'));
    const registry = new Registry({ extensionsDir: root });
    try {
      const pending = registry.watch();
      registry.unwatch();
      await pending;
      expect((registry as unknown as { _watchers?: unknown[] })._watchers ?? []).toHaveLength(0);
    } finally { registry.unwatch(); rmSync(root, { recursive: true, force: true }); }
  });
  it('concurrent watch calls install exactly one listener per root', async () => {
    const root = mkdtempSync(join(tmpdir(), 'apcore-watch-concurrent-'));
    const registry = new Registry({ extensionsDir: root });
    try {
      await Promise.all([registry.watch(), registry.watch()]);
      expect((registry as unknown as { _watchers: unknown[] })._watchers).toHaveLength(1);
    } finally { registry.unwatch(); rmSync(root, { recursive: true, force: true }); }
  });
});

describe('D-149 escaped JSON Pointer diagnostics', () => {
  it('escapes slashes and tildes and locates a required failure at its object', () => {
    const schema = Type.Object({ 'a/b~c': Type.Integer(), nested: Type.Object({ required: Type.String() }) });
    const result = new SchemaValidator().validate({ 'a/b~c': 'invalid', nested: {} }, schema);
    expect(result.errors).toHaveLength(2);
    expect(result.errors).toContainEqual({ path: '/a~1b~0c', keyword: 'type', message: expect.any(String) });
    expect(result.errors).toContainEqual({ path: '/nested', keyword: 'required', message: expect.any(String) });
    for (const error of result.errors) expect(Object.keys(error).sort()).toEqual(['keyword', 'message', 'path']);
  });
});

describe('D-133 timeout owns an abort-listener race', () => {
  it('supports older Node 20 and releases fallback links on successful and early-rejected calls', async () => {
    const descriptor = Object.getOwnPropertyDescriptor(AbortSignal, 'any')!;
    Object.defineProperty(AbortSignal, 'any', { ...descriptor, value: undefined });
    try {
      const registry = new Registry();
      await registry.register('demo.probe', {
        description: 'Fallback cancellation probe', inputSchema: Type.Object({ value: Type.String() }),
        outputSchema: Type.Object({}), execute: () => ({}),
      });
      const parent = new CancelToken();
      const added = vi.spyOn(parent.signal, 'addEventListener');
      const removed = vi.spyOn(parent.signal, 'removeEventListener');
      const executor = new Executor({ registry });
      const context = Context.create(null, null, parent);
      await expect(executor.call('demo.probe', { value: 'valid' }, context)).resolves.toEqual({});
      await expect(executor.call('demo.probe', {}, context)).rejects.toMatchObject({ code: 'SCHEMA_VALIDATION_ERROR' });
      await expect(executor.call('missing.probe', {}, context)).rejects.toMatchObject({ code: 'MODULE_NOT_FOUND' });
      await executor.validate('demo.probe', { value: 'valid' }, context);
      await executor.callWithTrace('demo.probe', { value: 'valid' }, context);
      for await (const chunk of executor.stream('demo.probe', { value: 'valid' }, context)) expect(chunk).toEqual({});
      expect(added.mock.calls.filter(([type]) => type === 'abort')).toHaveLength(6);
      expect(removed.mock.calls.filter(([type]) => type === 'abort')).toHaveLength(6);
      const child = new CancelToken(parent);
      parent.cancel();
      expect(child.signal.aborted).toBe(true);
      expect(child.isCancelled).toBe(true);
    } finally { Object.defineProperty(AbortSignal, 'any', descriptor); vi.restoreAllMocks(); }
  });
  it('a streaming deadline cancels its own scope, not the application token', async () => {
    const registry = new Registry();
    const application = new CancelToken();
    let seen: Context | undefined;
    await registry.register('slow.stream', {
      description: 'Expired streaming deadline', inputSchema: Type.Object({}), outputSchema: Type.Object({}), execute: () => ({}),
      async *stream(_inputs: Record<string, unknown>, context: Context) { seen = context; yield {}; },
    });
    const context = Context.create<null>(null, null, application, undefined, undefined, Date.now() / 1000 - 1);
    const stream = new Executor({ registry }).stream('slow.stream', {}, context);
    await expect(stream.next()).rejects.toMatchObject({ code: 'MODULE_TIMEOUT' });
    expect(seen?.cancelToken?.isCancelled).toBe(true);
    expect(application.isCancelled).toBe(false);
  });
  for (const reaction of ['reject', 'resolve']) {
    it(`reports MODULE_TIMEOUT when module abort listener immediately ${reaction}s`, async () => {
      const registry = new Registry();
      let seen: Context | undefined;
      await registry.register('slow.abort_listener', {
        description: 'Abort listener race regression', inputSchema: Type.Object({}), outputSchema: Type.Object({ ok: Type.Boolean() }), resources: { timeout: 10 },
        execute(_inputs: Record<string, unknown>, context: Context): Promise<Record<string, unknown>> {
          seen = context;
          return new Promise((resolve, reject) => context.signal.addEventListener('abort', () => {
            if (reaction === 'reject') reject(new ExecutionCancelledError());
            else resolve({ ok: true });
          }, { once: true }));
        },
      });
      await expect(new Executor({ registry }).call('slow.abort_listener', {})).rejects.toMatchObject({ code: 'MODULE_TIMEOUT' });
      expect(seen?.cancelToken?.isCancelled).toBe(true);
    });
  }
});

describe('configured subscriber circuit breaker standard bootstrap', () => {
  afterEach(() => resetSubscriberRegistry());
  it('honors circuit_breaker thresholds and suppresses delivery after opening', async () => {
    let attempts = 0;
    registerSubscriberType('issue123-failing', () => ({
      subscriberId: 'fixture-failing-subscriber', subscriberType: 'fixture', eventPattern: 'fixture.delivery',
      onEvent() { attempts += 1; throw new Error('Fixture delivery failure'); },
    }));
    const client = new APCore({ config: new Config({ sys_modules: { enabled: true, events: { enabled: true, subscribers: [{ type: 'issue123-failing', circuit_breaker: { timeout_ms: 50, open_threshold: 2, recovery_window_ms: 60000 } }] } } }) });
    const observed: ApCoreEvent[] = [];
    client.events!.subscribe({ eventPattern: 'apcore.subscriber.circuit_opened', onEvent(event) { observed.push(event); } });
    for (let index = 0; index < 3; index += 1) {
      client.events!.emit(createEvent('fixture.delivery', null, 'info', {}));
      await client.events!.flush();
    }
    expect(attempts).toBe(2);
    expect(observed).toHaveLength(1);
    expect(observed[0].data).toMatchObject({ subscriber_id: 'fixture-failing-subscriber', subscriber_type: 'fixture' });
  });
});
