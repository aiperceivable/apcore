/**
 * D-112 — a failed reload restores the previous module.
 *
 * This SDK is the decision's authority: it already re-registered the original on
 * failure while apcore-python and apcore-rust left it unregistered, and the
 * contract endorsed that ("callers must handle the partial state"). For a
 * control plane that is the wrong default — a failed hot-fix should not make a
 * WORKING module disappear — so the other two were changed to match this.
 *
 * It had no test. The recurring shape: a decision's AUTHORITY is the SDK least
 * likely to be covered, because the SDKs that had to CHANGE got tests as part of
 * the change.
 *
 * The decision is stated in four parts, each asserted separately:
 *
 *   1. Restoration is COMPENSATING, not transactional — no claim of atomic
 *      replacement.
 *   2. The restore MUST re-run the restored module's `onLoad`; its `onUnload`
 *      already ran, so republishing without it yields a module that is visible
 *      but torn down.
 *   3. If the restoring load ALSO fails, the module MAY remain unavailable.
 *   4. On the bulk path, restoration is PER MODULE; modules that already
 *      reloaded are NOT rolled back.
 */

import { describe, it, expect, vi } from 'vitest';
import { Registry } from '../../src/registry/registry.js';
import { EventEmitter } from '../../src/events/emitter.js';
import { ReloadModule } from '../../src/sys-modules/control.js';

function hookRecordingModule(tag: string, calls: string[]) {
  return {
    inputSchema: { type: 'object' as const },
    outputSchema: { type: 'object' as const },
    description: `records lifecycle hooks (${tag})`,
    tag,
    onLoad: () => {
      calls.push(`${tag}:onLoad`);
    },
    onUnload: () => {
      calls.push(`${tag}:onUnload`);
    },
    execute: async () => ({ tag }),
  };
}

describe('D-112: a failed reload restores the previous module', () => {
  it('the original instance is back after a failed re-discovery', async () => {
    const calls: string[] = [];
    const registry = new Registry();
    const original = hookRecordingModule('original', calls);
    await registry.register('executor.probe', original);

    vi.spyOn(registry, 'discover').mockRejectedValue(new Error('discovery is broken'));
    const mod = new ReloadModule(registry, new EventEmitter());

    await expect(
      mod.execute({ module_id: 'executor.probe', reason: 'hot-fix' }, null),
    ).rejects.toThrow();

    expect(registry.get('executor.probe')).toBe(original);
  });

  it('the restore re-runs onLoad', async () => {
    // Rule 2. `onUnload` already ran during the unregister, so a restore that
    // skips `onLoad` republishes a module that is visible but torn down —
    // harder to diagnose than one that is absent.
    const calls: string[] = [];
    const registry = new Registry();
    const original = hookRecordingModule('original', calls);
    await registry.register('executor.probe', original);
    calls.length = 0;

    vi.spyOn(registry, 'discover').mockRejectedValue(new Error('discovery is broken'));
    const mod = new ReloadModule(registry, new EventEmitter());

    await expect(
      mod.execute({ module_id: 'executor.probe', reason: 'hot-fix' }, null),
    ).rejects.toThrow();

    expect(calls).toEqual(['original:onUnload', 'original:onLoad']);
  });

  it('a module absent after re-discovery is also restored', async () => {
    // The second failure point: `discover()` SUCCEEDS but does not bring the
    // module back. Without covering it, an implementation that restores only on
    // a thrown discovery error leaves exactly the same hole.
    const calls: string[] = [];
    const registry = new Registry();
    const original = hookRecordingModule('original', calls);
    await registry.register('executor.probe', original);

    vi.spyOn(registry, 'discover').mockResolvedValue(0);
    const mod = new ReloadModule(registry, new EventEmitter());

    await expect(
      mod.execute({ module_id: 'executor.probe', reason: 'hot-fix' }, null),
    ).rejects.toThrow();

    expect(registry.get('executor.probe')).toBe(original);
  });

  it('control: a successful reload publishes the NEW instance', async () => {
    // Without this, "the original is registered afterwards" is also satisfied
    // by an implementation that never swaps anything in.
    const calls: string[] = [];
    const registry = new Registry();
    const original = hookRecordingModule('original', calls);
    const replacement = hookRecordingModule('replacement', calls);
    await registry.register('executor.probe', original);

    vi.spyOn(registry, 'discover').mockImplementation(async () => {
      registry.registerInternal('executor.probe', replacement);
      return 1;
    });
    const mod = new ReloadModule(registry, new EventEmitter());

    await mod.execute({ module_id: 'executor.probe', reason: 'hot-fix' }, null);

    expect(registry.get('executor.probe')).toBe(replacement);
  });
});
