import { describe, expect, it, vi } from 'vitest';

import { APCore, AutoApproveHandler, CancelToken, Config, Context, Executor, Middleware, Registry, registerSysModules } from '../../src/index.js';

function gate() {
  let open!: () => void;
  const ready = new Promise<void>((resolve) => { open = resolve; });
  return { ready, open };
}

function moduleWith(overrides: Record<string, unknown> = {}) {
  return {
    description: 'Lifecycle probe',
    inputSchema: { type: 'object' },
    outputSchema: { type: 'object' },
    execute: () => ({}),
    ...overrides,
  };
}

function reloadClient(): APCore {
  const client = new APCore({ config: new Config({ sys_modules: { enabled: true, events: { enabled: true } } }) });
  client.executor.setApprovalHandler(new AutoApproveHandler());
  return client;
}

describe('managed lifecycle hooks', () => {
  const controllerId = 'system.control.reload_module';
  const moduleId = 'probe.managed_reload';

  it('joins a formerly forced legacy removal until its execution finalizes', async () => {
    const client = new APCore();
    const entered = gate();
    const finish = gate();
    const finalized = vi.fn();
    const onUnload = vi.fn();
    await client.register(moduleId, moduleWith({
      async execute() {
        entered.open();
        try { await finish.ready; return {}; } finally { finalized(); }
      },
      onUnload,
    }));
    const call = client.call(moduleId);
    await entered.ready;
    try {
      await expect(client.registry.safeUnregister(moduleId, 0)).resolves.toBe(false);
      expect(onUnload).toHaveBeenCalledOnce();
      expect(finalized).not.toHaveBeenCalled();
      const shutdown = client.startShutdown();
      await expect(shutdown.wait(10)).resolves.toMatchObject({ status: 'pending', modules: [
        { moduleId, outcome: 'forced_removed', activeAtUnload: 1, cleanupPending: true },
      ] });
      finish.open();
      await expect(shutdown.wait()).resolves.toMatchObject({ status: 'completed', modules: [
        { moduleId, outcome: 'forced_removed', cleanupPending: false },
      ] });
      expect(finalized).toHaveBeenCalledOnce();
      expect(onUnload).toHaveBeenCalledOnce();
    } finally {
      finish.open();
      await Promise.allSettled([call, client.shutdown()]);
    }
  });

  it('declines public registration after managed shutdown and waits for its cleanup', async () => {
    const client = new APCore();
    const finishLoad = gate();
    const unloading = gate();
    const finishUnload = gate();
    const published = vi.fn();
    const onUnload = vi.fn(async () => { unloading.open(); await finishUnload.ready; });
    client.registry.on('register', published);
    const registration = client.register(moduleId, moduleWith({ onLoad: () => finishLoad.ready, onUnload }));
    const shutdown = client.startShutdown();
    try {
      finishLoad.open();
      await unloading.ready;
      expect(published).not.toHaveBeenCalled();
      expect(client.registry.get(moduleId)).toBeNull();
      expect((await shutdown.wait(5)).status).toBe('pending');
      finishUnload.open();
      await expect(registration).rejects.toMatchObject({ code: 'GENERAL_INVALID_INPUT' });
      await shutdown.wait();
      expect(onUnload).toHaveBeenCalledOnce();
    } finally {
      finishLoad.open();
      finishUnload.open();
      await Promise.allSettled([registration, shutdown.wait()]);
    }
  });

  it('continues a governed managed bulk reload when the audit store throws', async () => {
    const config = new Config({ sys_modules: { enabled: true, events: { enabled: true } } });
    const registry = new Registry({ config });
    const executor = new Executor({ registry, config });
    const append = vi.fn(() => { throw new Error('Persistent audit store unavailable'); });
    const { eventEmitter } = registerSysModules(registry, executor, config, null, { auditStore: { append, query: () => [] } });
    executor.setApprovalHandler(new AutoApproveHandler());
    const firstId = 'probe.bulk_first';
    const secondId = 'probe.bulk_second';
    const first = moduleWith();
    const second = moduleWith();
    await registry.register(firstId, moduleWith());
    await registry.register(secondId, moduleWith());
    registry.setDiscoverer({ discover: () => [{ moduleId: firstId, module: first }, { moduleId: secondId, module: second }] });
    const observe = vi.fn();
    eventEmitter!.subscribe({ eventPattern: 'apcore.module.reloaded', onEvent: observe });
    await expect(executor.call(controllerId, { path_filter: 'probe.bulk_*', reason: 'Reload', lifecycle: {} }))
      .resolves.toMatchObject({ success: true, reloaded_modules: [firstId, secondId], lifecycle_report: { status: 'completed' } });
    expect(registry.get(firstId)).toBe(first);
    expect(registry.get(secondId)).toBe(second);
    expect(append).toHaveBeenCalledTimes(2);
    await eventEmitter!.flush();
    expect(observe).toHaveBeenCalledTimes(2);
    await registry.shutdown();
    await eventEmitter!.shutdown();
  });

  it('reports and audits effective versions rather than the module object defaults', async () => {
    const client = reloadClient();
    await client.registry.register(moduleId, moduleWith({ version: '1.0.0' }), '2.3.4');
    client.registry.setDiscoverer({ discover: () => [{ moduleId, module: moduleWith({ version: '3.0.0' }) }] });
    const observe = vi.fn();
    client.events!.subscribe({ eventPattern: 'apcore.module.reloaded', onEvent: observe });
    await expect(client.call(controllerId, { module_id: moduleId, reason: 'Reload', lifecycle: {} }))
      .resolves.toMatchObject({ success: true, previous_version: '2.3.4', new_version: '3.0.0' });
    await client.events!.flush();
    expect(observe).toHaveBeenCalledWith(expect.objectContaining({ data: expect.objectContaining({ previous_version: '2.3.4', new_version: '3.0.0' }) }));
    await client.shutdown();
  });

  it('emits only sanitized lifecycle failures for a replacement onLoad error', async () => {
    const client = reloadClient();
    await client.register(moduleId, moduleWith());
    const observe = vi.fn();
    client.events!.subscribe({ eventPattern: 'apcore.registry.module_load_failed', onEvent: observe });
    client.events!.subscribe({ eventPattern: 'apcore.registry.module_reload_failed', onEvent: observe });
    const secret = 'private replacement details';
    client.registry.setDiscoverer({ discover: () => [{ moduleId, module: moduleWith({ onLoad() { throw new Error(secret); } }) }] });
    const output = await client.call(controllerId, { module_id: moduleId, reason: 'Reload', lifecycle: {} });
    await client.events!.flush();
    expect(output).toMatchObject({ success: false, lifecycle_report: { modules: [{ compensation: 'restored' }] } });
    expect(observe).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ eventType: 'apcore.registry.module_reload_failed' }));
    expect(JSON.stringify([output, observe.mock.calls])).not.toContain(secret);
    await client.shutdown();
  });

  it('records an exception while retrieving the drain hook and still unloads', async () => {
    const client = new APCore();
    const onUnload = vi.fn();
    const target = moduleWith({ onUnload });
    Object.defineProperty(target, 'onDrain', { get() { throw new Error('Unable to obtain the drain hook'); } });
    await client.register(moduleId, target);
    const shutdown = client.startShutdown();
    await expect(shutdown.wait(20)).resolves.toMatchObject({ status: 'failed', modules: [
      { moduleId, outcome: 'failed', error: { phase: 'draining' } },
    ] });
    expect(onUnload).toHaveBeenCalledOnce();
  });

  it('rejects an incomplete hook pair before closing admission', async () => {
    const client = reloadClient();
    const onDrain = vi.fn();
    await client.register(moduleId, moduleWith({ onDrain }));
    await expect(client.call(controllerId, { module_id: moduleId, reason: 'Reload', lifecycle: {} }))
      .rejects.toMatchObject({ code: 'GENERAL_INVALID_INPUT' });
    await expect(client.call(moduleId)).resolves.toEqual({});
    expect(onDrain).not.toHaveBeenCalled();
    await client.shutdown();
  });

  it('retains the same generation when a zero-budget reload aborts an active execution', async () => {
    const client = reloadClient();
    const body = gate();
    const entered = gate();
    const restore = gate();
    const recovering = gate();
    const onUnload = vi.fn();
    const onDrainAborted = vi.fn(async () => { recovering.open(); await restore.ready; });
    await client.register(moduleId, moduleWith({
      async execute() { entered.open(); await body.ready; return {}; },
      onDrain() {}, onDrainAborted, onUnload,
    }));
    const active = client.call(moduleId);
    await entered.ready;
    const reload = client.call(controllerId, { module_id: moduleId, reason: 'Reload', lifecycle: { drain_timeout_ms: 0 } });
    try {
      const barrier = await Promise.race([reload.then(() => 'reload', () => 'reload'), recovering.ready.then(() => 'recovering')]);
      expect(barrier).toBe('recovering');
      await expect(client.call(moduleId)).rejects.toMatchObject({ code: 'MODULE_NOT_FOUND' });
      expect(onUnload).not.toHaveBeenCalled();
      restore.open();
      await expect(reload).resolves.toMatchObject({
        success: false, lifecycle_report: { kind: 'reload', status: 'failed', modules: [
          { outcome: 'retained_ready', abort_reason: 'drain_timeout', drain_timed_out: true },
        ] },
      });
      expect(onDrainAborted).toHaveBeenCalledWith(expect.objectContaining({ abortReason: 'drain_timeout' }));
    } finally {
      body.open();
      restore.open();
      await Promise.allSettled([active, reload]);
      await client.shutdown();
    }
  });

  it('keeps an asynchronously resumed replacement invisible until publication', async () => {
    const client = reloadClient();
    const resume = gate();
    const release = gate();
    const old = moduleWith({ async onSuspend() { return { count: 3 }; } });
    const replacement = moduleWith({ async onResume(state: unknown) {
      expect(state).toEqual({ count: 3 });
      resume.open();
      await release.ready;
    } });
    await client.register(moduleId, old);
    client.registry.setDiscoverer({ discover: () => [{ moduleId, module: replacement }] });
    const reload = client.call(controllerId, { module_id: moduleId, reason: 'Reload', lifecycle: {} });
    try {
      const barrier = await Promise.race([reload.then(() => 'reload', () => 'reload'), resume.ready.then(() => 'resume')]);
      expect(barrier).toBe('resume');
      expect(client.registry.get(moduleId)).toBeNull();
      release.open();
      await expect(reload).resolves.toMatchObject({ success: true, lifecycle_report: { status: 'completed' } });
      expect(client.registry.get(moduleId)).toBe(replacement);
    } finally {
      release.open();
      await Promise.allSettled([reload]);
      await client.shutdown();
    }
  });

  it('cancels only the retired generation and reports physical cleanup after force', async () => {
    const client = new APCore();
    const entered = gate();
    const release = gate();
    const unloaded = gate();
    const parent = new CancelToken();
    await client.register(moduleId, moduleWith({
      async execute() { entered.open(); await release.ready; return {}; },
      onUnload() { unloaded.open(); },
    }));
    await client.register('probe.healthy', moduleWith());
    const context = Context.create(null, null, parent);
    const active = client.call(moduleId, {}, context);
    await entered.ready;
    try {
      const removal = client.startUnregisterMany([moduleId], {
        drainTimeoutMs: 0, cancelGraceMs: 0, onDrainTimeout: 'cancel_then_force',
      });
      await expect(active).rejects.toMatchObject({ code: 'EXECUTION_CANCELLED' });
      await unloaded.ready;
      expect(removal.snapshot()).toMatchObject({ status: 'pending', modules: [
        { outcome: 'forced_removed', activeAtUnload: 1, cleanupPending: true },
      ] });
      expect(parent.isCancelled).toBe(false);
      await expect(client.call('probe.healthy', {}, context)).resolves.toEqual({});
      release.open();
      await expect(removal.wait()).resolves.toMatchObject({ status: 'completed', modules: [
        { outcome: 'forced_removed', cleanupPending: false },
      ] });
    } finally {
      release.open();
      await Promise.allSettled([active]);
      await client.shutdown();
    }
  });

  it('signals a blocked drain hook and waits for it before starting readiness recovery', async () => {
    const client = reloadClient();
    const signalled = gate();
    const releaseDrain = gate();
    const onUnload = vi.fn();
    const onDrainAborted = vi.fn();
    await client.register(moduleId, moduleWith({
      async onDrain(context: { signal: AbortSignal }) {
        context.signal.addEventListener('abort', signalled.open);
        await releaseDrain.ready;
      },
      onDrainAborted,
      onUnload,
    }));
    const reload = client.call(controllerId, { module_id: moduleId, reason: 'Reload', lifecycle: { drain_timeout_ms: 0 } });
    try {
      await Promise.race([signalled.ready, reload.then(() => { throw new Error('Reload completed before its drain hook'); })]);
      expect(onDrainAborted).not.toHaveBeenCalled();
      expect(onUnload).not.toHaveBeenCalled();
      await expect(client.call(moduleId)).rejects.toMatchObject({ code: 'MODULE_NOT_FOUND' });
      releaseDrain.open();
      await expect(reload).resolves.toMatchObject({ success: false, lifecycle_report: { modules: [
        { outcome: 'retained_ready', drain_timed_out: true, abort_reason: 'drain_timeout' },
      ] } });
      expect(onDrainAborted).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({
        abortReason: 'drain_timeout', signal: expect.objectContaining({ aborted: false }),
      }));
      expect(onUnload).not.toHaveBeenCalled();
    } finally {
      releaseDrain.open();
      await Promise.allSettled([reload]);
      await client.shutdown();
    }
  });

  it('emits the shared resource outcome once to each participating client', async () => {
    const first = reloadClient();
    const second = new APCore({ registry: first.registry, config: new Config({
      sys_modules: { enabled: true, events: { enabled: true } },
    }) });
    expect(first.events).not.toBe(second.events);
    const payloads = [first, second].map(client => {
      const payload: unknown[] = [];
      client.events!.subscribe({
        eventPattern: 'apcore.client.shutdown_resources_completed',
        async onEvent(event) { payload.push(event.data); },
      });
      return payload;
    });
    const onUnload = vi.fn();
    await first.register(moduleId, moduleWith({ onUnload }));
    const one = first.startShutdown();
    const two = second.startShutdown();
    first.startShutdown();
    second.startShutdown();
    const [firstReport, secondReport] = await Promise.all([one.wait(), two.wait()]);
    expect(firstReport.operationId).toBe(secondReport.operationId);
    expect(firstReport.eventFlush).toBe('completed');
    expect(secondReport.eventFlush).toBe('completed');
    const expected = { operation_id: firstReport.operationId, registry_outcome: 'completed', forced_count: 0, failure_count: 0 };
    expect(payloads).toEqual([[expected], [expected]]);
    expect(onUnload).toHaveBeenCalledOnce();
  });

  it('reports a failed readiness hook through normal output and keeps the generation unavailable', async () => {
    const client = reloadClient();
    const after = vi.fn();
    const onError = vi.fn();
    client.use(new class extends Middleware {
      override after(id: string, inputs: Record<string, unknown>, output: Record<string, unknown>) {
        if (id === controllerId) after(inputs, output);
        return null;
      }
      override onError(id: string, inputs: Record<string, unknown>, error: Error) {
        if (id === controllerId) onError(inputs, error);
        return null;
      }
    }());
    await client.register(moduleId, moduleWith({
      onDrain() { throw new Error('Private drain details'); },
      onDrainAborted() { throw new Error('Private recovery details'); },
    }));
    client.registry.setDiscoverer({ discover: () => [] });
    const input = { module_id: moduleId, reason: 'Reload' };
    await expect(client.call(controllerId, { ...input, lifecycle: {} })).resolves.toMatchObject({
      success: false, lifecycle_report: { modules: [{ outcome: 'retained_unavailable', error: { phase: 'recovering' } }] },
    });
    expect(after).toHaveBeenCalledOnce();
    expect(onError).not.toHaveBeenCalled();
    client.registry.endDrain(moduleId);
    await expect(client.call(moduleId)).rejects.toMatchObject({ code: 'MODULE_NOT_FOUND' });

    await expect(client.call(controllerId, input)).rejects.toMatchObject({ code: 'RELOAD_FAILED' });
    expect(onError).toHaveBeenCalledOnce();
    await client.shutdown();
  });
});

describe('module lifecycle through APCore', () => {
  it('completes an active call before ordinary unregister releases its resources', async () => {
    const client = new APCore();
    const entered = gate();
    const finish = gate();
    let released = false;
    const onUnload = vi.fn(() => { released = true; });
    await client.register('probe.slow', moduleWith({
      async execute() {
        entered.open();
        await finish.ready;
        return { usable: !released };
      },
      onUnload,
    }));

    const call = client.call('probe.slow');
    await entered.ready;
    expect(client.registry.unregister('probe.slow')).toBe(true);
    const denied = client.call('probe.slow');
    finish.open();

    await expect(denied).rejects.toMatchObject({ code: 'MODULE_NOT_FOUND' });
    await expect(call).resolves.toEqual({ usable: true });
    expect(onUnload).toHaveBeenCalledOnce();
  });

  it('rejects a call paused before execute even when the same module object was registered again', async () => {
    const client = new APCore();
    const entered = gate();
    const finish = gate();
    const execute = vi.fn(() => ({ called: true }));
    const module = moduleWith({ execute });
    client.use(new class extends Middleware {
      override async before(_id: string, inputs: Record<string, unknown>): Promise<null> {
        if (inputs['hold'] === true) {
          entered.open();
          await finish.ready;
        }
        return null;
      }
    });
    await client.register('probe.transient', module);

    const waiting = client.call('probe.transient', { hold: true });
    await entered.ready;
    expect(client.registry.unregister('probe.transient')).toBe(true);
    await client.register('probe.transient', module);
    finish.open();

    await expect(waiting).rejects.toMatchObject({ code: 'MODULE_NOT_FOUND' });
    expect(execute).not.toHaveBeenCalled();
    await expect(client.call('probe.transient')).resolves.toEqual({ called: true });
  });

  it.each([{ module_id: 'probe.counter' }, { path_filter: 'probe.*' }])(
    'transfers updates from calls that complete while explicitly reloading %j',
    async (selector) => {
      const client = new APCore({ config: new Config({
        sys_modules: { enabled: true, events: { enabled: true } },
      }) });
      client.executor.setApprovalHandler(new AutoApproveHandler());
      const entered = gate();
      const finish = gate();
      const drainStarted = gate();
      const hooks: string[] = [];
      let oldCount = 0;
      let newCount = 0;
      await client.register('probe.counter', moduleWith({
        async execute() {
          entered.open();
          await finish.ready;
          oldCount += 1;
          hooks.push('execute');
          return { count: oldCount };
        },
        onSuspend() {
          hooks.push('suspend');
          return { count: oldCount };
        },
        onUnload() { hooks.push('unload'); },
      }));
      const unregister = client.registry.safeUnregister.bind(client.registry);
      vi.spyOn(client.registry, 'safeUnregister').mockImplementation((...args) => {
        const removal = unregister(...args);
        drainStarted.open();
        return removal;
      });
      client.registry.on('register', (id) => {
        if (id === 'probe.counter') hooks.push('publish');
      });
      client.registry.setDiscoverer({
        async discover() {
          return [{ moduleId: 'probe.counter', module: moduleWith({
            execute: () => ({ count: newCount }),
            onLoad() { hooks.push('load'); },
            onResume(state: { count: number }) {
              hooks.push('resume');
              newCount = state.count;
            },
          }) }];
        },
      });

      const active = client.call('probe.counter');
      await entered.ready;
      const reload = client.call('system.control.reload_module', { ...selector, reason: 'Refresh module' });
      await drainStarted.ready;
      finish.open();

      await expect(active).resolves.toEqual({ count: 1 });
      await expect(reload).resolves.toMatchObject({ success: true });
      await expect(client.call('probe.counter')).resolves.toEqual({ count: 1 });
      expect(hooks).toEqual(['execute', 'suspend', 'unload', 'load', 'resume', 'publish']);
    },
  );

  it('returns false for a second safe unregister while the first awaits cleanup', async () => {
    const registry = new Registry();
    const started = gate();
    const finish = gate();
    const onUnload = vi.fn(() => {
      started.open();
      return finish.ready;
    });
    await registry.register('probe.cleanup', moduleWith({ onUnload }));
    const first = registry.safeUnregister('probe.cleanup');
    await started.ready;
    const lateHook = vi.fn();
    let result: boolean | undefined;
    const late = registry.safeUnregister('probe.cleanup', 10, lateHook).then((clean) => { result = clean; });
    await Promise.resolve();
    const resultBeforeCleanup = result;
    finish.open();

    await expect(first).resolves.toBe(true);
    await late;
    expect(resultBeforeCleanup).toBe(false);
    expect(lateHook).not.toHaveBeenCalled();
    expect(onUnload).toHaveBeenCalledOnce();
  });

  it('applies batch operations in input order and retains successful registrations on error', async () => {
    const client = new APCore();
    const hooks: string[] = [];
    const first = moduleWith({
      async onLoad() { hooks.push('first:load'); },
      onUnload() { hooks.push('first:unload'); },
    });
    const second = moduleWith({ onLoad() { hooks.push('second:load'); } });
    await expect(client.registerMany([
      ['probe.first', first],
      ['probe.first', first],
      ['probe.second', second],
    ])).rejects.toMatchObject({ code: 'DUPLICATE_MODULE_ID' });
    expect(client.listModules()).toEqual(['probe.first']);
    await client.registerMany([['probe.second', second]]);

    await expect(client.unregisterMany([
      'probe.second', 'probe.missing', 'probe.first', 'probe.first',
    ])).resolves.toEqual([true, false, true, false]);
    expect(hooks).toEqual(['first:load', 'second:load', 'first:unload']);
    expect(client.listModules()).toEqual([]);
  });

  it('stops accepting calls and awaits hidden module teardown at host shutdown', async () => {
    const client = new APCore({ config: new Config({
      sys_modules: { enabled: true, events: { enabled: true } },
    }) });
    const entered = gate();
    const finish = gate();
    const events = client.events!;
    const unloads: string[] = [];
    await client.register('probe.hidden', moduleWith({
      annotations: { discoverable: false },
      async execute() {
        entered.open();
        await finish.ready;
        return { active: unloads.length === 0 };
      },
      async onUnload() { unloads.push('hidden'); },
    }));

    const active = client.call('probe.hidden');
    await entered.ready;
    const shutdown = client.shutdown();
    const secondShutdown = client.shutdown();
    expect(secondShutdown).toBe(shutdown);
    const denied = client.call('probe.hidden');
    finish.open();

    await expect(denied).rejects.toMatchObject({ code: 'MODULE_NOT_FOUND' });
    await expect(active).resolves.toEqual({ active: true });
    await Promise.all([shutdown, secondShutdown]);
    expect(unloads).toEqual(['hidden']);
    expect(client.registry.list({ visibility: ['public', 'hidden'] })).toEqual([]);
    expect(events.tryEmit({
      eventType: 'probe.closed', moduleId: null, timestamp: '', severity: 'info', data: {},
    })).toBe(false);
  });

  it('cleans up a registration whose onLoad was already running at host shutdown', async () => {
    const client = new APCore();
    const finishLoad = gate();
    const unloaded = vi.fn();
    const registration = client.register('probe.compiling', moduleWith({
      onLoad: () => finishLoad.ready,
      onUnload: unloaded,
    }));

    const shutdown = client.shutdown();
    expect(() => client.register('probe.late', moduleWith())).toThrow('Registry has been shut down');
    finishLoad.open();

    await registration;
    await shutdown;
    expect(unloaded).toHaveBeenCalledOnce();
    expect(client.listModules()).toEqual([]);
  });

  it('force-unloads at the shutdown drain deadline and still waits for cleanup', async () => {
    const client = new APCore();
    const entered = gate();
    const finishCall = gate();
    const finishCleanup = gate();
    const onUnload = vi.fn(() => finishCleanup.ready);
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    vi.useFakeTimers();
    try {
      await client.register('probe.stuck', moduleWith({
        async execute() {
          entered.open();
          await finishCall.ready;
          return {};
        },
        onUnload,
      }));
      const call = client.call('probe.stuck');
      await entered.ready;
      const shutdown = client.shutdown(10);
      expect(client.shutdown()).toBe(shutdown);
      let completed = false;
      void shutdown.then(() => { completed = true; });

      await vi.advanceTimersByTimeAsync(9);
      expect(onUnload).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(1);
      expect(onUnload).toHaveBeenCalledOnce();
      expect(completed).toBe(false);
      finishCleanup.open();
      await shutdown;
      expect(completed).toBe(true);
      finishCall.open();
      await call;
    } finally {
      warn.mockRestore();
      vi.useRealTimers();
    }
  });

  it('bounds the drain of a registration detached before shutdown', async () => {
    vi.useFakeTimers();
    const client = new APCore();
    const onUnload = vi.fn();
    const module = moduleWith({ onUnload });
    await client.register('probe.retiring', module);
    const release = client.registry.acquireExecution('probe.retiring', module);
    try {
      expect(client.registry.unregister('probe.retiring')).toBe(true);
      const shutdown = client.shutdown(25);
      await vi.advanceTimersByTimeAsync(24);
      expect(onUnload).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(1);
      expect(onUnload).toHaveBeenCalledOnce();
      await shutdown;
    } finally {
      release();
      vi.useRealTimers();
    }
  });

  it('rejects a shutdown wait from an async unload hook while independent callers can wait', async () => {
    const client = new APCore();
    const entered = gate();
    const release = gate();
    const onUnload = vi.fn(async () => {
      await Promise.resolve();
      const own = client.startShutdown();
      expect(own.snapshot().modules[0]?.activeHook).toBe('on_unload');
      await expect(own.wait(1)).rejects.toMatchObject({ code: 'GENERAL_INVALID_INPUT' });
      await expect(client.shutdown()).rejects.toMatchObject({ code: 'GENERAL_INVALID_INPUT' });
      await expect(own.wait(0)).resolves.toMatchObject({ status: 'pending' });
      entered.open();
      await release.ready;
    });
    await client.register('probe.reentry', moduleWith({ onUnload }));
    const operation = client.startShutdown();
    try {
      await entered.ready;
      await expect(operation.wait(1)).resolves.toMatchObject({ status: 'pending' });
    } finally {
      release.open();
      await operation.wait();
    }
    expect(onUnload).toHaveBeenCalledOnce();
  });

  it('keeps the legacy drain mode when an explicit operation joins and reports its unload error', async () => {
    const client = new APCore();
    const unload = vi.fn(() => { throw new Error('private error'); });
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    await client.register('probe.failed_unload', moduleWith({ onUnload: unload }));
    try {
      const legacy = client.shutdown(23);
      expect(client.shutdown(99)).toBe(legacy);
      expect(() => client.startShutdown({ onDrainTimeout: 'continue_waiting' })).toThrow('conflicts');
      const operation = client.startShutdown();
      await expect(legacy).resolves.toBeUndefined();
      await expect(operation.wait()).resolves.toMatchObject({
        status: 'failed', error: { code: 'GENERAL_INTERNAL_ERROR', phase: 'unloading' },
        modules: [{ moduleId: 'probe.failed_unload', phase: 'done', outcome: 'failed' }],
      });
      expect(unload).toHaveBeenCalledOnce();
    } finally {
      warn.mockRestore();
    }
  });

  it('gives all deliveries one client-owned deadline and rejects an event subscriber waiting on itself', async () => {
    const firstDelivery = gate();
    const lastDelivery = gate();
    const called = gate();
    const client = new APCore({ config: new Config({ sys_modules: { enabled: true, events: { enabled: true } } }) });
    const peer = new APCore({ registry: client.registry });
    const events = client.events!;
    const event = { eventType: 'probe.deliver', moduleId: null, timestamp: '', severity: 'info', data: {} };
    const eventType = event.eventType;
    events.subscribe({ eventPattern: eventType, onEvent: () => firstDelivery.ready });
    events.subscribe({ eventPattern: eventType, async onEvent() {
      await Promise.resolve();
      const operation = client.startShutdown({ eventFlushTimeoutMs: 20 });
      await expect(operation.wait(1)).rejects.toMatchObject({ code: 'GENERAL_INVALID_INPUT' });
      called.open();
      await lastDelivery.ready;
    } });
    vi.useFakeTimers();
    try {
      expect(events.tryEmit(event)).toBe(true);
      await called.ready;
      const operation = client.startShutdown();
      const peerOperation = peer.startShutdown({ eventFlushTimeoutMs: 0 });
      await expect(peerOperation.wait()).resolves.toMatchObject({
        eventFlush: 'not_applicable', operationId: operation.snapshot().operationId,
      });
      await vi.advanceTimersByTimeAsync(14);
      firstDelivery.open();
      await vi.advanceTimersByTimeAsync(5);
      expect(operation.snapshot()).toMatchObject({ status: 'pending', eventFlush: 'pending' });
      await vi.advanceTimersByTimeAsync(1);
      await expect(operation.wait()).resolves.toMatchObject({ status: 'completed', eventFlush: 'timeout' });
      expect(events.tryEmit(event)).toBe(false);
    } finally {
      firstDelivery.open();
      lastDelivery.open();
      vi.useRealTimers();
    }
  });

  it('validates managed options and batch IDs before changing admission', async () => {
    const client = new APCore();
    await client.register('probe.keep', moduleWith());
    for (const options of [{ drainTimeoutMs: -1 }, { eventFlushTimeoutMs: 2_147_483_648 }, { extra: true }]) {
      expect(() => client.startShutdown(options as never)).toThrow();
    }
    expect(() => client.startUnregisterMany(['probe.keep', ''])).toThrow();
    await expect(client.call('probe.keep')).resolves.toEqual({});
    const empty = client.startUnregisterMany([], { operationTimeoutMs: 0 });
    expect(empty.snapshot()).toMatchObject({ status: 'completed', modules: [] });
    await expect(empty.wait(Number.NaN)).rejects.toMatchObject({ code: 'GENERAL_INVALID_INPUT' });
    await client.shutdown();
  });

  it('aborts just one waiter without interrupting an owned shutdown', async () => {
    const client = new APCore();
    const unloaded = vi.fn();
    const module = moduleWith({ onUnload: unloaded });
    await client.register('probe.retained_call', module);
    const release = client.registry.acquireExecution('probe.retained_call', module);
    const operation = client.startShutdown({ drainTimeoutMs: 0 });
    const controller = new AbortController();
    const reason = new Error('Stop waiting');
    const abandoned = operation.wait(10_000, controller.signal);
    const retained = operation.wait();
    try {
      controller.abort(reason);
      await expect(abandoned).rejects.toBe(reason);
      await expect(operation.wait(undefined, controller.signal)).rejects.toBe(reason);
      expect(operation.snapshot()).toMatchObject({ status: 'pending' });
      expect(unloaded).not.toHaveBeenCalled();
    } finally {
      release();
      await retained;
    }
    await expect(retained).resolves.toMatchObject({ status: 'completed', operationId: operation.snapshot().operationId });
    expect(unloaded).toHaveBeenCalledOnce();
  });
});
