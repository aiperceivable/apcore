/**
 * Cross-language conformance driver for registry_load_ordering.json (Issue #65).
 *
 * Fixture source: apcore/conformance/fixtures/registry_load_ordering.json
 * (single source of truth). apcore-python drives it from
 * tests/conformance/test_registry_load_ordering.py and apcore-rust from
 * tests/test_registry_ordering_conformance.rs; TypeScript had no driver.
 *
 * The strong-guarantee invariant: a module MUST NOT be observable through any
 * discovery API until every `onLoad` callback has completed successfully. On
 * failure the module MUST NOT become visible, the original error MUST surface
 * unchanged, and `apcore.registry.module_load_failed` MUST be emitted.
 *
 * JS is single-threaded, so "concurrent" here means two overlapping in-flight
 * promises rather than two OS threads — the same interleaving the invariant is
 * written against.
 */

import { describe, it, expect } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { Registry } from '../src/registry/registry.js';
import { EventEmitter } from '../src/events/emitter.js';
import type { ApCoreEvent } from '../src/events/emitter.js';
import { DuplicateModuleIdError, ModuleError } from '../src/errors.js';
import { findFixturesRoot } from './spec-repo.js';

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const FIXTURE = JSON.parse(
  fs.readFileSync(path.join(findFixturesRoot(), 'registry_load_ordering.json'), 'utf-8'),
) as { test_cases: Array<Record<string, any>> };

function caseById(id: string): Record<string, any> {
  const found = FIXTURE.test_cases.find((c) => c['id'] === id);
  if (!found) throw new Error(`Fixture case '${id}' not found`);
  return found;
}

const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));

describe('Conformance: registry_load_ordering.json', () => {
  it('visibility_after_successful_on_load', async () => {
    const tc = caseById('visibility_after_successful_on_load');
    const setup = tc['setup']['module'];
    const expected = tc['expected'];
    const delayMs: number = setup['on_load_delay_ms'] ?? 0;
    const modId: string = setup['id'];

    const registry = new Registry();
    const observed: Record<string, unknown> = {};
    const midLoadListed: boolean[] = [];
    const midLoadGot: unknown[] = [];

    const mod = {
      execute: async () => ({}),
      onLoad: async () => {
        await sleep(delayMs / 2);
        // Mid-onLoad: the module must be invisible to every discovery API.
        midLoadListed.push(registry.list().includes(modId));
        // get() is called unconditionally: routing it through has() first would
        // have made this assert has() twice and never exercise get() at all.
        midLoadGot.push(registry.get(modId));
        await sleep(delayMs / 2);
        observed['_test.warmed'] = true;
      },
    };

    // `expect(expected['registration_succeeds']).toBe(true)` used to stand here
    // — the fixture compared to itself. Whether register() resolved is the
    // observation.
    let registered = false;
    await registry.register(modId, mod).then(() => {
      registered = true;
    });
    expect(registered).toBe(expected['registration_succeeds']);

    if (expected['post_register_visible']) {
      expect(registry.list()).toContain(modId);
      expect(registry.get(modId)).not.toBeNull();
    }
    if (!expected['concurrent_check_visible']) {
      expect(midLoadListed).toEqual([false]);
      // Bound to the fixture. The key used to be `concurrent_check_get_raises:
      // "MODULE_NOT_FOUND"`, a behaviour no SDK implements — get() returns the
      // empty value for an id that is not visible, per registry-system.md "On
      // success (not found)". It is now `concurrent_check_get_returns: null`.
      expect(midLoadGot).toEqual([expected['concurrent_check_get_returns'] ?? null]);
    }
    for (const [key, value] of Object.entries(expected['on_load_observed_data'] ?? {})) {
      expect(observed[key]).toEqual(value);
    }
  });

  it('callback_failure_blocks_visibility', async () => {
    const tc = caseById('callback_failure_blocks_visibility');
    const setup = tc['setup']['module'];
    const expected = tc['expected'];
    const modId: string = setup['id'];
    const errorCfg = setup['on_load_raises'];

    const emitter = new EventEmitter();
    const registry = new Registry();
    registry.setEventEmitter(emitter);

    const loadFailed: ApCoreEvent[] = [];
    emitter.subscribe({
      subscriberId: 'load-failed-recorder',
      onEvent: (e) => {
        if (e.eventType === 'apcore.registry.module_load_failed') loadFailed.push(e);
      },
    });

    // The fixture names ConnectionError; JS has no such built-in, so a named
    // Error subclass carries the same `error_type` on the wire.
    class ConnectionError extends Error {
      constructor(message: string) {
        super(message);
        this.name = 'ConnectionError';
      }
    }

    const failing = {
      execute: async () => ({}),
      onLoad: async () => {
        throw new ConnectionError(errorCfg['message']);
      },
    };

    let thrown: unknown = null;
    try {
      await registry.register(modId, failing);
    } catch (e) {
      thrown = e;
    }
    await emitter.flush();

    // `registration_raises` names the HOST exception the callback threw, not an
    // apcore code: register() MUST re-raise it unchanged. Wrapping it in an
    // apcore error (or swallowing it) is the regression this catches — the
    // message alone would survive both.
    if (expected['registration_raises']) {
      expect(thrown).toBeInstanceOf(ConnectionError);
      expect((thrown as Error).name).toBe('ConnectionError');
      expect((thrown as Error).message).toBe(expected['registration_error_message']);
      expect(thrown).not.toBeInstanceOf(ModuleError);
    }

    expect(registry.list().includes(modId)).toBe(expected['post_register_list_contains']);
    expect(registry.has(modId)).toBe(expected['post_register_visible']);
    // Same correction: the key declared a raise no SDK performs and is now
    // `post_register_get_returns: null`, which is observable.
    expect(registry.get(modId)).toBe(expected['post_register_get_returns'] ?? null);

    if (expected['load_failed_event_emitted']) {
      expect(loadFailed).toHaveLength(1);
      const evt = loadFailed[0];
      const expectedEvt = expected['load_failed_event'];
      expect(evt.eventType).toBe(expectedEvt['event_type']);
      const dataContains = expectedEvt['data_contains'];
      expect(evt.data['module_id']).toBe(dataContains['module_id']);
      expect(evt.data['error_type']).toBe(dataContains['error_type']);
      expect(String(evt.data['error_message'])).toContain(dataContains['error_message']);
      for (const key of expectedEvt['data_required_keys'] as string[]) {
        expect(evt.data).toHaveProperty(key);
      }
    }
  });

  it('concurrent_same_id_rejects_duplicate', async () => {
    const tc = caseById('concurrent_same_id_rejects_duplicate');
    const setup = tc['setup'];
    const expected = tc['expected'];
    const modId: string = setup['module_a']['id'];
    const delayMs: number = setup['module_a']['on_load_delay_ms'] ?? 0;

    const registry = new Registry();
    const makeSlow = () => ({
      execute: async () => ({}),
      onLoad: async () => {
        await sleep(delayMs);
      },
    });

    const results = await Promise.allSettled([
      registry.register(modId, makeSlow()),
      // Started synchronously in the same tick — the first registration is
      // already in-flight, so this one must be rejected.
      (async () => registry.register(modId, makeSlow()))(),
    ]);

    const fulfilled = results.filter((r) => r.status === 'fulfilled');
    const rejected = results.filter((r) => r.status === 'rejected');
    expect(fulfilled).toHaveLength(1);
    expect(rejected).toHaveLength(1);
    const err = (rejected[0] as PromiseRejectedResult).reason;
    expect(err).toBeInstanceOf(DuplicateModuleIdError);
    expect((err as DuplicateModuleIdError).code).toBe(expected['raised_error_code']);

    if (expected['post_register_visible']) {
      expect(registry.get(modId)).not.toBeNull();
    }
    expect(registry.list()).toHaveLength(expected['post_register_count']);
  });

  it.each([0, 150])('concurrent_distinct_ids_run_in_parallel (startup delay %ims)', async (startupDelayMs) => {
    const tc = caseById('concurrent_distinct_ids_run_in_parallel');
    const setup = tc['setup'];
    const expected = tc['expected'];
    const registry = new Registry();
    let entered = 0;
    let callbacksOverlap = false;
    let release!: () => void;
    let fail!: (error: Error) => void;
    const gate = new Promise<void>((resolve, reject) => {
      release = resolve;
      fail = reject;
    });
    // Deadlock protection, not a performance assertion.
    const watchdog = setTimeout(() => fail(new Error('onLoad callbacks did not overlap')), 5000);
    const makeSlow = (delayMs: number) => ({
      execute: async () => ({}),
      onLoad: async () => {
        entered += 1;
        if (entered === 2) {
          callbacksOverlap = true;
          release();
        }
        await gate;
        await sleep(delayMs);
      },
    });
    try {
      const results = await Promise.allSettled(['module_x', 'module_y'].map(async (key) => {
        await sleep(startupDelayMs);
        await registry.register(setup[key]['id'], makeSlow(setup[key]['on_load_delay_ms']));
      }));
      expect(results.every((result) => result.status === 'fulfilled')).toBe(expected['both_succeed']);
      expect(callbacksOverlap).toBe(expected['callbacks_overlap']);
      expect(registry.get(setup['module_x']['id'])).not.toBeNull();
      expect(registry.get(setup['module_y']['id'])).not.toBeNull();
      expect(registry.list()).toHaveLength(expected['post_register_count']);
    } finally {
      clearTimeout(watchdog);
    }
  });
});
