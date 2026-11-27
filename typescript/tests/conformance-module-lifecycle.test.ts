import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, it, vi } from 'vitest';
import { AlwaysDenyHandler, APCore, AutoApproveHandler, CancelToken, Config, Context, ModuleError, ModuleLoadError, STREAMING_MARKER } from '../src/full.js';
import { jsonSchemaToTypeBox } from '../src/schema/loader.js';
import { RefResolver } from '../src/schema/ref-resolver.js';
import { SchemaValidator } from '../src/schema/validator.js';
import { findFixturesRoot, findSchemasRoot } from './spec-repo.js';

interface LifecycleCase {
  id: string;
  operation: 'unregister' | 'reload' | 'reload_bulk' | 'shutdown' | 'batch' | 'batch_drain' | 'toggle_self' | 'reload_self'
    | 'managed_shutdown_waiter' | 'managed_shutdown_shared' | 'managed_batch_budget' | 'managed_batch_stale_generation'
    | 'managed_reload_drain' | 'managed_reload_abort' | 'managed_reload_hook_failure'
    | 'managed_reload_replacement_failure' | 'managed_reload_no_match' | 'managed_force_scope' | 'managed_shutdown_event'
    | 'managed_reload_stream' | 'managed_reload_denied';
  module_id: string;
  other_module_id?: string;
  selector?: Record<string, string>;
  path_filter?: string;
  drain_timeout_ms?: number;
  wait_timeout_ms?: number;
  operation_timeout_ms?: number;
  cancel_grace_ms?: number;
  expected: Record<string, unknown>;
}

const cases: LifecycleCase[] = JSON.parse(
  readFileSync(join(findFixturesRoot(), 'module_lifecycle.json'), 'utf-8'),
).test_cases;

function signal(): { done: Promise<void>; resolve: () => void } {
  let resolve!: () => void;
  const done = new Promise<void>((settle) => { resolve = settle; });
  return { done, resolve };
}

function counter(events: string[], label?: string, initialCount: number = 0) {
  const entered = signal();
  const release = signal();
  const unloaded = signal();
  let closed = false;
  let count = initialCount;
  const record = (event: string) => { events.push(label ? `${event}:${label}` : event); };
  const module = {
    description: 'Record module lifecycle and transfer a counter',
    inputSchema: { type: 'object', properties: { hold: { type: 'boolean' } } },
    outputSchema: { type: 'object', properties: { count: { type: 'integer' } }, required: ['count'] },
    async execute(inputs: Record<string, unknown>) {
      if (inputs.hold === true) {
        entered.resolve();
        await release.done;
        if (closed) throw new Error('module resources closed during its call');
        count += 1;
        record('finish');
      }
      return { count };
    },
    onLoad() {
      if (label) record('load');
    },
    onSuspend() {
      record('suspend');
      return { count };
    },
    onUnload() {
      closed = true;
      record('unload');
      unloaded.resolve();
    },
    onResume(state: { count: number }) {
      count = state.count;
      record('resume');
    },
  };
  return { module, entered, release, unloaded };
}

async function callError(client: APCore, moduleId: string): Promise<string | null> {
  try {
    await client.call(moduleId, {});
    return null;
  } catch (error) {
    if (!(error instanceof ModuleError)) throw error;
    return error.code;
  }
}

async function admissionClosed(client: APCore, moduleId: string): Promise<string> {
  return vi.waitFor(async () => {
    const code = await callError(client, moduleId);
    expect(code).toBe('MODULE_NOT_FOUND');
    return code!;
  }, { interval: 2, timeout: 2_000 });
}

async function exerciseBatch(client: APCore, testCase: LifecycleCase, events: string[]) {
  const moduleId = testCase.module_id;
  const otherId = testCase.other_module_id!;
  let batchError: string | null = null;
  try {
    await client.registerMany([
      [moduleId, counter(events).module],
      [moduleId, counter(events).module],
      [otherId, counter(events).module],
    ]);
  } catch (error) {
    if (!(error instanceof ModuleError)) throw error;
    batchError = error.code;
  }
  const registered = client.listModules();
  const removedItems = await client.unregisterMany([moduleId, otherId, moduleId]);
  return { batch_error: batchError, registered, removed_items: removedItems, events };
}

async function exerciseBatchDrain(client: APCore, testCase: LifecycleCase, events: string[]) {
  const moduleId = testCase.module_id;
  const otherId = testCase.other_module_id!;
  const first = counter(events, 'first');
  await client.registerMany([[moduleId, first.module], [otherId, counter(events, 'second').module]]);
  const active = client.call(moduleId, { hold: true });
  let removal: Promise<boolean[]> | undefined;
  try {
    await first.entered.done;
    let finished = false;
    removal = client.unregisterMany([moduleId, otherId]).then((results) => {
      finished = true;
      return results;
    });
    const newCallError = await admissionClosed(client, moduleId);
    const otherOutput = await client.call(otherId, {});
    expect(finished).toBe(false);
    first.release.resolve();
    return {
      new_call_error: newCallError,
      other_output_during_drain: otherOutput,
      active_output: await active,
      removed_items: await removal,
      events,
    };
  } finally {
    first.release.resolve();
    await Promise.allSettled([active, removal]);
  }
}

async function exerciseLifecycle(client: APCore, testCase: LifecycleCase, events: string[]) {
  const moduleId = testCase.module_id;
  const probe = counter(events);
  if (testCase.operation === 'reload' || testCase.operation === 'reload_bulk') {
    client.registry.setDiscoverer({
      discover: () => [{ moduleId, module: counter(events).module }],
    });
  }
  await client.register(moduleId, probe.module);
  const active = client.call(moduleId, { hold: true });
  let lifecycle: Promise<unknown> | undefined;
  try {
    await probe.entered.done;
    if (testCase.operation === 'unregister') {
      const removed = client.registry.unregister(moduleId);
      const newCallError = await admissionClosed(client, moduleId);
      probe.release.resolve();
      const activeOutput = await active;
      return { removed, new_call_error: newCallError, active_output: activeOutput, events };
    }
    if (testCase.operation === 'shutdown') {
      let finished = false;
      lifecycle = client.shutdown().then(() => { finished = true; });
      const newCallError = await admissionClosed(client, moduleId);
      expect(finished).toBe(false);
      probe.release.resolve();
      const activeOutput = await active;
      await lifecycle;
      await client.shutdown();
      return { active_output: activeOutput, new_call_error: newCallError, events };
    }
    const selector = testCase.operation === 'reload' ? { module_id: moduleId } : { path_filter: moduleId };
    const reload = client.call('system.control.reload_module', { ...selector, reason: 'conformance' });
    lifecycle = reload;
    await admissionClosed(client, moduleId);
    probe.release.resolve();
    const activeOutput = await active;
    const result = await reload;
    const nextOutput = await client.call(moduleId, {});
    return { active_output: activeOutput, reload_succeeded: result.success, next_output: nextOutput, events };
  } finally {
    probe.release.resolve();
    await Promise.allSettled([active, lifecycle]);
  }
}

async function exerciseControlGuard(client: APCore, testCase: LifecycleCase, events: string[]) {
  const moduleId = testCase.module_id;
  await client.register('aaa.counter', counter(events).module);
  client.registry.on('unregister', (id) => events.push(`unregister:${id}`));
  const registered = client.listModules();
  const toggle = testCase.operation === 'toggle_self';
  const inputs = toggle ? { module_id: moduleId, enabled: false } : testCase.selector;
  const followup = toggle ? { module_id: moduleId, enabled: true } : { path_filter: 'missing.module' };
  try {
    let controlError: string | null = null;
    try {
      await client.call(moduleId, { ...inputs, reason: 'conformance' });
    } catch (error) {
      if (!(error instanceof ModuleError)) throw error;
      controlError = error.code;
    }
    const registrationsPreserved = JSON.stringify(client.listModules()) === JSON.stringify(registered);
    const result = await client.call(moduleId, { ...followup, reason: 'conformance' });
    return {
      control_error: controlError,
      registrations_preserved: registrationsPreserved,
      control_usable: result.success,
      events: [...events],
    };
  } finally {
    await client.shutdown();
  }
}

async function managedShutdownWaiter(client: APCore, testCase: LifecycleCase, events: string[]) {
  const moduleId = testCase.module_id;
  const probe = counter(events);
  await client.register(moduleId, probe.module);
  const running = client.call(moduleId, { hold: true });
  try {
    await probe.entered.done;
    const operation = client.startShutdown({ drainTimeoutMs: testCase.drain_timeout_ms });
    const newCallError = await admissionClosed(client, moduleId);
    const waitReport = await operation.wait(testCase.wait_timeout_ms);
    const drainTimedOut = await vi.waitFor(() => {
      const value = operation.snapshot().modules[0]?.drainTimedOut;
      expect(value).toBe(true);
      return value;
    });
    probe.release.resolve();
    const activeOutput = await running;
    await probe.unloaded.done;
    const final = await client.startShutdown().wait();
    return {
      wait_status: waitReport.status, drain_timed_out: drainTimedOut, new_call_error: newCallError,
      active_output: activeOutput, same_operation: waitReport.operationId === final.operationId,
      final_status: final.status, outcomes: final.modules.map(item => item.outcome), events,
    };
  } finally {
    probe.release.resolve();
    await Promise.allSettled([running]);
  }
}

async function managedShutdownShared(client: APCore, testCase: LifecycleCase, events: string[]) {
  await client.register(testCase.module_id, counter(events).module);
  const second = new APCore({ registry: client.registry });
  const firstOperation = client.startShutdown({ drainTimeoutMs: testCase.drain_timeout_ms });
  const secondOperation = second.startShutdown();
  let conflictError;
  try {
    second.startShutdown({ drainTimeoutMs: testCase.drain_timeout_ms! + 1 });
  } catch (error) {
    if (!(error instanceof ModuleError)) throw error;
    conflictError = error.code;
  }
  const [first, final] = await Promise.all([firstOperation.wait(), secondOperation.wait()]);
  expect(first).toEqual(final);
  return {
    same_operation: first.operationId === final.operationId, conflict_error: conflictError,
    final_status: final.status, outcomes: final.modules.map(item => item.outcome), events,
  };
}

async function managedBatch(client: APCore, testCase: LifecycleCase, events: string[]) {
  const { module_id: moduleId, other_module_id: otherId } = testCase;
  const first = counter(events, 'first');
  await client.registerMany([[moduleId, first.module], [otherId!, counter(events, 'second').module]]);
  const running = client.call(moduleId, { hold: true });
  try {
    await first.entered.done;
    const operation = client.registry.startUnregisterMany([moduleId, otherId!], {
      operationTimeoutMs: testCase.operation_timeout_ms,
    });
    await admissionClosed(client, moduleId);
    if (testCase.operation === 'managed_batch_stale_generation') {
      expect(client.registry.unregister(otherId!)).toBe(true);
      await client.register(otherId!, counter(events, 'replacement').module);
      first.release.resolve();
      await running;
      const final = await operation.wait();
      return {
        final_status: final.status, outcomes: final.modules.map(item => item.outcome),
        replacement_output: await client.call(otherId!, {}), events,
      };
    }
    const waitReport = await operation.wait(0);
    const otherOutput = await client.call(otherId!, {});
    const drainTimedOut = await vi.waitFor(() => {
      const value = operation.snapshot().modules[0]?.drainTimedOut;
      expect(value).toBe(true);
      return value;
    });
    first.release.resolve();
    const activeOutput = await running;
    const final = await operation.wait();
    return {
      wait_status: waitReport.status, drain_timed_out: drainTimedOut,
      other_output_during_drain: otherOutput, active_output: activeOutput,
      final_status: final.status, stop_reason: final.stopReason,
      outcomes: final.modules.map(item => item.outcome), other_output_after: await client.call(otherId!, {}), events,
    };
  } finally {
    first.release.resolve();
    await Promise.allSettled([running]);
  }
}

async function callManagedReload(client: APCore, testCase: LifecycleCase) {
  const output = await client.call('system.control.reload_module', {
    ...(testCase.path_filter === undefined ? { module_id: testCase.module_id } : { path_filter: testCase.path_filter }),
    reason: 'conformance',
    lifecycle: testCase.drain_timeout_ms === undefined ? {} : { drain_timeout_ms: testCase.drain_timeout_ms },
  });
  const root = findSchemasRoot();
  const path = join(root, 'sys-control-reload-module.schema.json');
  const schema = new RefResolver(root).resolve(JSON.parse(readFileSync(path, 'utf-8')), path);
  const validation = new SchemaValidator().validate(JSON.parse(JSON.stringify(output)), jsonSchemaToTypeBox(schema));
  expect(validation).toMatchObject({ valid: true });
  const report = output.lifecycle_report as {
    status: string; operation_id: string; kind: string; event_flush: string;
    modules: Array<{ outcome: string; abort_reason?: string; compensation?: string; error?: { code: string } }>;
  };
  return {
    output, report, first: report.modules[0],
    summary: { reload_succeeded: output.success, report_status: report.status, outcomes: report.modules.map(row => row.outcome) },
  };
}

async function managedReloadDrain(client: APCore, testCase: LifecycleCase, events: string[]) {
  const moduleId = testCase.module_id;
  const old = counter(events);
  const replacement = counter(events);
  await client.register(moduleId, Object.assign(old.module, {
    onDrain() { events.push('drain'); old.release.resolve(); },
    onDrainAborted() {},
  }));
  client.registry.setDiscoverer({ discover: () => [{ moduleId, module: Object.assign(replacement.module, {
    onLoad() { events.push('load'); },
  }) }] });
  const active = client.call(moduleId, { hold: true });
  const completed = active.catch(() => null);
  try {
    await old.entered.done;
    const { summary } = await callManagedReload(client, testCase);
    return { ...summary, active_output: await active, next_output: await client.call(moduleId, {}), events: [...events] };
  } finally {
    old.release.resolve();
    await completed;
    await client.shutdown();
  }
}

async function managedReloadStream(client: APCore, testCase: LifecycleCase, events: string[]) {
  const moduleId = testCase.module_id;
  const old = counter(events);
  const replacement = counter(events);
  let count = 0;
  await client.register(moduleId, {
    ...old.module,
    annotations: { streaming: true },
    [STREAMING_MARKER]: true,
    async *stream() {
      old.entered.resolve();
      try {
        await old.release.done;
        count++;
        yield { count };
      } finally {
        events.push('stream_finished');
      }
    },
    onDrain() { events.push('drain'); old.release.resolve(); },
    onDrainAborted() {},
    onSuspend() { events.push('suspend'); return { count }; },
  });
  client.registry.setDiscoverer({ discover: () => [{ moduleId, module: {
    ...replacement.module,
    onLoad() { events.push('load'); },
  } }] });
  const chunks: Record<string, unknown>[] = [];
  const active = (async () => {
    for await (const chunk of client.stream(moduleId, {})) chunks.push(chunk);
  })();
  const completed = active.catch(() => null);
  try {
    await old.entered.done;
    const { summary } = await callManagedReload(client, testCase);
    await active;
    return { ...summary, chunks, next_output: await client.call(moduleId, {}), events: [...events] };
  } finally {
    old.release.resolve();
    await completed;
    await client.shutdown();
  }
}

async function managedReloadDenied(client: APCore, testCase: LifecycleCase, events: string[]) {
  const moduleId = testCase.module_id;
  client.executor.setApprovalHandler(new AlwaysDenyHandler());
  await client.register(moduleId, {
    ...counter(events, undefined, 1).module,
    onDrain() { events.push('drain'); },
    onDrainAborted() { events.push('drain_aborted'); },
  });
  let lifecycleEventCount = 0;
  client.events!.subscribe({ eventPattern: 'apcore.registry.*', onEvent() { lifecycleEventCount++; } });
  let controlError: string | null = null;
  try {
    try {
      await callManagedReload(client, testCase);
    } catch (error) {
      if (!(error instanceof ModuleError)) throw error;
      controlError = error.code;
    }
    const nextOutput = await client.call(moduleId, {});
    await client.events!.flush();
    return { control_error: controlError, next_output: nextOutput, lifecycle_event_count: lifecycleEventCount, events: [...events] };
  } finally {
    await client.shutdown();
  }
}

async function managedReloadAbort(client: APCore, testCase: LifecycleCase, events: string[]) {
  const moduleId = testCase.module_id;
  const old = counter(events);
  const recovering = signal();
  const releaseRecovery = signal();
  await client.register(moduleId, Object.assign(old.module, {
    onDrain() { events.push('drain'); },
    async onDrainAborted() { events.push('drain_aborted'); recovering.resolve(); await releaseRecovery.done; },
  }));
  const active = client.call(moduleId, { hold: true });
  await old.entered.done;
  const reload = callManagedReload(client, testCase);
  try {
    await Promise.race([recovering.done, reload.then(() => { throw new Error('Reload completed before readiness recovery'); })]);
    const newCallError = await callError(client, moduleId);
    releaseRecovery.resolve();
    const { summary, first } = await reload;
    old.release.resolve();
    return {
      ...summary, abort_reason: first?.abort_reason, new_call_error: newCallError, active_output: await active,
      next_output: await client.call(moduleId, {}), events: [...events],
    };
  } finally {
    releaseRecovery.resolve();
    old.release.resolve();
    await Promise.allSettled([reload, active]);
    await client.shutdown();
  }
}

async function managedReloadHookFailure(client: APCore, testCase: LifecycleCase, events: string[]) {
  const moduleId = testCase.module_id;
  await client.register(moduleId, Object.assign(counter(events).module, {
    onDrain() { events.push('drain'); throw new Error('Fixture drain failed'); },
    onDrainAborted() { events.push('drain_aborted'); },
  }));
  try {
    const { summary, first } = await callManagedReload(client, testCase);
    return { ...summary, abort_reason: first?.abort_reason, next_output: await client.call(moduleId, {}), events: [...events] };
  } finally {
    await client.shutdown();
  }
}

async function managedReloadReplacementFailure(client: APCore, testCase: LifecycleCase, events: string[]) {
  const moduleId = testCase.module_id;
  const replacementLoading = signal();
  const releaseReplacement = signal();
  const restoring = signal();
  const releaseRestore = signal();
  let loadCount = 0;
  await client.register(moduleId, Object.assign(counter(events).module, {
    async onLoad() { if (++loadCount > 1) { restoring.resolve(); await releaseRestore.done; } },
  }));
  client.registry.setDiscoverer({ discover: () => [{ moduleId, module: Object.assign(counter(events).module, {
    async onLoad() { replacementLoading.resolve(); await releaseReplacement.done; throw new ModuleLoadError(moduleId, 'Fixture load failed'); },
  }) }] });
  const reload = callManagedReload(client, testCase);
  try {
    const finishedBeforeLoad = reload.then(() => { throw new Error('Reload completed before load settled'); });
    await Promise.race([replacementLoading.done, finishedBeforeLoad]);
    const newCallError = await callError(client, moduleId);
    releaseReplacement.resolve();
    await Promise.race([restoring.done, finishedBeforeLoad]);
    expect(await callError(client, moduleId)).toBe('MODULE_NOT_FOUND');
    releaseRestore.resolve();
    const { summary, first } = await reload;
    return {
      ...summary, new_call_error: newCallError, compensation: first?.compensation, primary_error: first?.error?.code,
      next_output: await client.call(moduleId, {}),
    };
  } finally {
    releaseReplacement.resolve();
    releaseRestore.resolve();
    await Promise.allSettled([reload]);
    await client.shutdown();
  }
}

async function managedReloadNoMatch(client: APCore, testCase: LifecycleCase) {
  try {
    const { summary, output, report } = await callManagedReload(client, testCase);
    return {
      ...summary, report_kind: report.kind, event_flush: report.event_flush,
      operation_id_present: !!report.operation_id, reloaded_modules: output.reloaded_modules,
    };
  } finally {
    await client.shutdown();
  }
}

async function managedForceScope(client: APCore, testCase: LifecycleCase, events: string[]) {
  const moduleId = testCase.module_id;
  const otherId = testCase.other_module_id!;
  const target = counter(events);
  const sibling = counter(events, 'sibling');
  await client.registerMany([[moduleId, target.module], [otherId, sibling.module]]);
  const parent = new CancelToken();
  const context = Context.create(null, null, parent);
  const running = client.call(moduleId, { hold: true }, context).then(() => null, error => {
    if (!(error instanceof ModuleError)) throw error;
    return error.code;
  });
  const other = client.call(otherId, { hold: true }, context);
  try {
    await Promise.all([target.entered.done, sibling.entered.done]);
    const operation = client.startUnregisterMany([moduleId], {
      drainTimeoutMs: testCase.drain_timeout_ms, cancelGraceMs: testCase.cancel_grace_ms, onDrainTimeout: 'cancel_then_force',
    });
    const targetCancelError = await running;
    target.release.resolve();
    sibling.release.resolve();
    const [report, otherOutput] = await Promise.all([operation.wait(), other]);
    return {
      target_cancel_error: targetCancelError, parent_cancelled: parent.isCancelled,
      other_output_after: otherOutput, final_status: report.status, unload_count: events.filter(item => item === 'unload').length,
    };
  } finally {
    target.release.resolve();
    sibling.release.resolve();
    await Promise.allSettled([running, other]);
    await client.shutdown();
  }
}

async function managedShutdownEvent(client: APCore, testCase: LifecycleCase, events: string[]) {
  await client.register(testCase.module_id, counter(events).module);
  const payloads: Record<string, unknown>[] = [];
  client.events!.subscribe({
    eventPattern: 'apcore.client.shutdown_resources_completed',
    async onEvent(event) { payloads.push(event.data); },
  });
  const report = await client.startShutdown().wait();
  return {
    final_status: report.status, event_flush: report.eventFlush, event_count: payloads.length,
    event_matches_operation: payloads[0]?.operation_id === report.operationId, events,
  };
}

it.each(cases)('$id', async (testCase) => {
  const controls = ['reload', 'reload_bulk', 'toggle_self', 'reload_self', 'managed_shutdown_event'].includes(testCase.operation) ||
    testCase.operation.startsWith('managed_reload_');
  const client = controls
    ? new APCore({ config: new Config({ sys_modules: { enabled: true, events: { enabled: true } } }) })
    : new APCore();
  if (controls) client.executor.setApprovalHandler(new AutoApproveHandler());
  const events: string[] = [];
  const exercise = {
    batch: exerciseBatch,
    batch_drain: exerciseBatchDrain,
    unregister: exerciseLifecycle,
    shutdown: exerciseLifecycle,
    reload: exerciseLifecycle,
    reload_bulk: exerciseLifecycle,
    toggle_self: exerciseControlGuard,
    reload_self: exerciseControlGuard,
    managed_shutdown_waiter: managedShutdownWaiter,
    managed_shutdown_shared: managedShutdownShared,
    managed_batch_budget: managedBatch,
    managed_batch_stale_generation: managedBatch,
    managed_reload_drain: managedReloadDrain,
    managed_reload_stream: managedReloadStream,
    managed_reload_denied: managedReloadDenied,
    managed_reload_abort: managedReloadAbort,
    managed_reload_hook_failure: managedReloadHookFailure,
    managed_reload_replacement_failure: managedReloadReplacementFailure,
    managed_reload_no_match: managedReloadNoMatch,
    managed_force_scope: managedForceScope,
    managed_shutdown_event: managedShutdownEvent,
  }[testCase.operation];
  const result = await exercise(client, testCase, events);
  expect(result).toEqual(testCase.expected);
});
