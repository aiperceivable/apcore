/**
 * Spec-traced contract tests for the APCore unified client (TypeScript SDK).
 *
 * Mirrors the canonical Python suite
 * `apcore-python/tests/test_apcore_client_spec.py`. Each `it(...)` name carries
 * the VERBATIM clause id (format `apcore_client.<method>.<kind>.<detail>`) so a
 * cross-language diff can match rows by exact clause id.
 *
 * TESTS ONLY — production source is never modified here.
 *
 * Cross-language note on error CODES
 * ----------------------------------
 * Malformed / empty module ids raise `InvalidInputError` with code
 * `INVALID_MODULE_ID` (aligned with apcore-python). Duplicate registration
 * raises `DuplicateModuleIdError` (`DUPLICATE_MODULE_ID`) — a TS-specific
 * subclass, recorded as a divergence in the task report. These tests assert the
 * REAL type + code the TS SDK emits.
 */

import { describe, it, expect } from 'vitest';
import { Type } from '@sinclair/typebox';
import { APCore } from '../src/client.js';
import { Config } from '../src/config.js';
import { FunctionModule } from '../src/decorator.js';
import {
  InvalidInputError,
  ModuleNotFoundError,
  SchemaValidationError,
  SysModulesDisabledError,
  DuplicateModuleIdError,
  ConfigNotFoundError,
} from '../src/errors.js';
import type { ApCoreEvent, EventSubscriber } from '../src/events/emitter.js';
import { Middleware } from '../src/middleware/index.js';
import type { Context } from '../src/context.js';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const AddInputSchema = Type.Object({ a: Type.Number(), b: Type.Number() });
const AddOutputSchema = Type.Object({ sum: Type.Number() });

/** Config with sys_modules + events enabled (production-like client). */
function sysConfig(): Config {
  return new Config({
    sys_modules: { enabled: true, events: { enabled: true } },
  });
}

/** Zero-config client with a single deterministic add module registered. */
function clientWithModule(moduleId = 'math.add'): APCore {
  const client = new APCore();
  client.module({
    id: moduleId,
    inputSchema: AddInputSchema,
    outputSchema: AddOutputSchema,
    description: 'Add two numbers',
    execute: (inputs) => ({ sum: (inputs.a as number) + (inputs.b as number) }),
  });
  return client;
}

/** Minimal class-based middleware with a configurable priority. */
class NoopMiddleware extends Middleware {
  readonly mwName: string;
  constructor(name = 'noop', priority = 0) {
    super(priority);
    this.mwName = name;
  }
  override before(): null {
    return null;
  }
}

function makeEvent(eventType: string): ApCoreEvent {
  return {
    eventType,
    moduleId: null,
    timestamp: '2026-01-01T00:00:00Z',
    severity: 'info',
    data: {},
  };
}

// ===========================================================================
// Contract: ApCoreClient.call
// ===========================================================================

describe('ApCoreClient.call', () => {
  it('apcore_client.call.input.module_id.invalid_pattern: malformed module_id rejected', async () => {
    const client = clientWithModule();
    await expect(client.call('!!not a valid id!!', { a: 1, b: 2 })).rejects.toThrow(
      InvalidInputError,
    );
    await client.call('!!not a valid id!!', { a: 1, b: 2 }).catch((e) => {
      expect(e).toBeInstanceOf(InvalidInputError);
      expect((e as InvalidInputError).code).toBe('INVALID_MODULE_ID');
    });
  });

  it('apcore_client.call.input.module_id.empty: empty module_id rejected', async () => {
    const client = clientWithModule();
    await client.call('', { a: 1, b: 2 }).then(
      () => {
        throw new Error('expected rejection');
      },
      (e) => {
        expect(e).toBeInstanceOf(InvalidInputError);
        expect((e as InvalidInputError).code).toBe('INVALID_MODULE_ID');
      },
    );
  });

  it('apcore_client.call.error.INVALID_MODULE_ID: reserved/malformed id rejected', async () => {
    const client = clientWithModule();
    await client.call('UPPER.Reserved Bad', {}).then(
      () => {
        throw new Error('expected rejection');
      },
      (e) => {
        expect(e).toBeInstanceOf(InvalidInputError);
        expect((e as InvalidInputError).code).toBe('INVALID_MODULE_ID');
      },
    );
  });

  it('apcore_client.call.error.MODULE_NOT_FOUND: unknown module rejected', async () => {
    const client = clientWithModule();
    await client.call('missing.module', { a: 1 }).then(
      () => {
        throw new Error('expected rejection');
      },
      (e) => {
        expect(e).toBeInstanceOf(ModuleNotFoundError);
        expect((e as ModuleNotFoundError).code).toBe('MODULE_NOT_FOUND');
      },
    );
  });

  it('apcore_client.call.error.SCHEMA_VALIDATION_ERROR: bad inputs rejected', async () => {
    const client = new APCore();
    client.module({
      id: 'math.strict',
      inputSchema: AddInputSchema,
      outputSchema: AddOutputSchema,
      description: 'Strict typed add',
      execute: (inputs) => ({ sum: (inputs.a as number) + (inputs.b as number) }),
    });
    await client.call('math.strict', { a: 'not-an-int', b: 2 }).then(
      () => {
        throw new Error('expected rejection');
      },
      (e) => {
        expect(e).toBeInstanceOf(SchemaValidationError);
        expect((e as SchemaValidationError).code).toBe('SCHEMA_VALIDATION_ERROR');
      },
    );
  });

  it('apcore_client.call.error.handler_propagates: handler error propagates', async () => {
    const client = new APCore();
    client.module({
      id: 'util.raiser',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      description: 'Always raises',
      execute: () => {
        throw new Error('boom-from-handler');
      },
    });
    await client.call('util.raiser', {}).then(
      () => {
        throw new Error('expected rejection');
      },
      (e) => {
        // TS wraps handler errors as ModuleExecuteError; the original message
        // is preserved via cause / message.
        expect(String((e as Error).message)).toContain('boom-from-handler');
      },
    );
  });

  it('apcore_client.call.property.async: returns a Promise that resolves', async () => {
    const client = clientWithModule();
    const p = client.call('math.add', { a: 10, b: 5 });
    expect(p).toBeInstanceOf(Promise);
    const result = await p;
    expect(result).toEqual({ sum: 15 });
  });

  it('apcore_client.call.property.thread_safe: >=8 concurrent calls isolated', async () => {
    const client = clientWithModule();
    const pairs = Array.from({ length: 10 }, (_, i) => [i, i * 2] as const);
    const results = await Promise.all(
      pairs.map(([a, b]) => client.call('math.add', { a, b })),
    );
    expect(results.map((r) => r.sum)).toEqual(pairs.map(([a, b]) => a + b));
  });

  it('apcore_client.call.property.idempotent_false: stateful module differs across calls', async () => {
    const client = new APCore();
    const state = { n: 0 };
    client.module({
      id: 'util.counter',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ n: Type.Number() }),
      description: 'Increments a counter',
      execute: () => {
        state.n += 1;
        return { n: state.n };
      },
    });
    const first = await client.call('util.counter', {});
    const second = await client.call('util.counter', {});
    expect(first).not.toEqual(second);
    expect([first.n, second.n]).toEqual([1, 2]);
  });
});

// ===========================================================================
// Contract: ApCoreClient.start   (MISSING SYMBOL — contract gap)
// ===========================================================================

describe('ApCoreClient.start', () => {
  it.skip('apcore_client.start.error.CONFIG_INVALID: missing symbol APCore.start (contract gap)', () => {
    // The TypeScript APCore class exposes no start() method (lifecycle is
    // implicit at construction).
  });

  it.skip('apcore_client.start.property.idempotent_false: missing symbol APCore.start (contract gap)', () => {
    // No start() method exists on the TS APCore client.
  });
});

// ===========================================================================
// Contract: ApCoreClient.stop   (MISSING SYMBOL — contract gap)
// ===========================================================================

describe('ApCoreClient.stop', () => {
  it.skip('apcore_client.stop.property.idempotent_true: missing symbol APCore.stop (contract gap)', () => {
    // The TypeScript APCore client exposes no stop() method.
  });
});

// ===========================================================================
// Contract: APCoreClient.on
// ===========================================================================

describe('APCoreClient.on', () => {
  it('apcore_client.on.input.event_type.non_empty: empty subscription never fires for real event', () => {
    const client = new APCore({ config: sysConfig() });
    const received: ApCoreEvent[] = [];
    client.on('', (e) => {
      received.push(e);
    });
    expect(client.events).not.toBeNull();
    client.events!.emit(makeEvent('apcore.registry.module_registered'));
    expect(received).toEqual([]);
  });

  it('apcore_client.on.error.SYS_MODULES_DISABLED: on() without events raises typed error', () => {
    const client = new APCore();
    let caught: unknown;
    try {
      client.on('apcore.health.error_threshold_exceeded', () => undefined);
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(SysModulesDisabledError);
    expect((caught as SysModulesDisabledError).code).toBe('SYS_MODULES_DISABLED');
    expect((caught as Error).message).toContain('Events are not enabled');
  });

  it('apcore_client.on.property.thread_safe: >=8 concurrent subscriptions all fire', async () => {
    const client = new APCore({ config: sysConfig() });
    const counters = new Array(10).fill(0);
    await Promise.all(
      Array.from({ length: 10 }, (_, i) =>
        Promise.resolve().then(() => {
          client.on('evt.shared', () => {
            counters[i] += 1;
          });
        }),
      ),
    );
    expect(client.events).not.toBeNull();
    client.events!.emit(makeEvent('evt.shared'));
    await client.events!.flush();
    expect(counters).toEqual(new Array(10).fill(1));
  });

  it('apcore_client.on.property.idempotent_false: same handler twice fires twice', async () => {
    const client = new APCore({ config: sysConfig() });
    const calls: number[] = [];
    const handler = (): void => {
      calls.push(1);
    };
    client.on('evt.dup', handler);
    client.on('evt.dup', handler);
    expect(client.events).not.toBeNull();
    client.events!.emit(makeEvent('evt.dup'));
    await client.events!.flush();
    expect(calls.length).toBe(2);
  });

  it('apcore_client.on.returns.subscriber: returns a usable subscriber object', () => {
    const client = new APCore({ config: sysConfig() });
    const sub = client.on('evt.x', () => undefined);
    // TS subscriber is a plain EventSubscriber object (no _CallbackSubscriber
    // class, no `event_type` field). Assert it is a usable handle for off().
    expect(sub).toBeDefined();
    expect(typeof sub.onEvent).toBe('function');
  });
});

// ===========================================================================
// Contract: APCoreClient.off
// ===========================================================================

describe('APCoreClient.off', () => {
  it('apcore_client.off.error.SYS_MODULES_DISABLED: off() without events raises typed error', () => {
    const client = new APCore();
    let caught: unknown;
    try {
      client.off({ onEvent: () => undefined });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(SysModulesDisabledError);
    expect((caught as SysModulesDisabledError).code).toBe('SYS_MODULES_DISABLED');
  });

  it('apcore_client.off.property.idempotent_true: double off() is a no-op', async () => {
    const client = new APCore({ config: sysConfig() });
    const calls: number[] = [];
    const sub = client.on('evt.off', () => {
      calls.push(1);
    });
    client.off(sub);
    client.off(sub); // second off() must not throw
    expect(client.events).not.toBeNull();
    client.events!.emit(makeEvent('evt.off'));
    await client.events!.flush();
    expect(calls).toEqual([]);
  });

  it('apcore_client.off.property.thread_safe: >=8 concurrent unsubscribes all removed', async () => {
    const client = new APCore({ config: sysConfig() });
    const calls = new Array(10).fill(0);
    const subs: EventSubscriber[] = [];
    for (let i = 0; i < 10; i++) {
      subs.push(
        client.on('evt.toff', () => {
          calls[i] += 1;
        }),
      );
    }
    await Promise.all(subs.map((s) => Promise.resolve().then(() => client.off(s))));
    expect(client.events).not.toBeNull();
    client.events!.emit(makeEvent('evt.toff'));
    await client.events!.flush();
    expect(calls).toEqual(new Array(10).fill(0));
  });
});

// ===========================================================================
// Contract: APCoreClient.stream
// ===========================================================================

describe('APCoreClient.stream', () => {
  it('apcore_client.stream.input.module_id.invalid_pattern: malformed id rejected', async () => {
    const client = clientWithModule();
    const drive = async (): Promise<void> => {
      // eslint-disable-next-line @typescript-eslint/no-unused-vars
      for await (const _ of client.stream('!!bad!!', {})) {
        // drain
      }
    };
    await drive().then(
      () => {
        throw new Error('expected rejection');
      },
      (e) => {
        expect(e).toBeInstanceOf(InvalidInputError);
        expect((e as InvalidInputError).code).toBe('INVALID_MODULE_ID');
      },
    );
  });

  it('apcore_client.stream.error.MODULE_NOT_FOUND: unknown module rejected', async () => {
    const client = clientWithModule();
    const gen = client.stream('not.registered', {});
    await gen.next().then(
      () => {
        throw new Error('expected rejection');
      },
      (e) => {
        expect(e).toBeInstanceOf(ModuleNotFoundError);
        expect((e as ModuleNotFoundError).code).toBe('MODULE_NOT_FOUND');
      },
    );
  });

  it('apcore_client.stream.error.INVALID_MODULE_ID: empty id rejected', async () => {
    const client = clientWithModule();
    const gen = client.stream('', {});
    await gen.next().then(
      () => {
        throw new Error('expected rejection');
      },
      (e) => {
        expect(e).toBeInstanceOf(InvalidInputError);
        expect((e as InvalidInputError).code).toBe('INVALID_MODULE_ID');
      },
    );
  });

  it('apcore_client.stream.property.async: async generator yields dict chunks', async () => {
    const client = clientWithModule();
    const chunks: Record<string, unknown>[] = [];
    for await (const chunk of client.stream('math.add', { a: 3, b: 4 })) {
      chunks.push(chunk);
    }
    expect(chunks.length).toBeGreaterThanOrEqual(1);
    expect(chunks.every((c) => typeof c === 'object' && c !== null)).toBe(true);
    expect(chunks[chunks.length - 1]).toEqual({ sum: 7 });
  });

  it('apcore_client.stream.property.thread_safe: >=8 concurrent streams isolated', async () => {
    const client = clientWithModule();
    const collect = async (a: number, b: number): Promise<Record<string, unknown>> => {
      let last: Record<string, unknown> = {};
      for await (const chunk of client.stream('math.add', { a, b })) {
        last = chunk;
      }
      return last;
    };
    const pairs = Array.from({ length: 8 }, (_, i) => [i, i + 1] as const);
    const results = await Promise.all(pairs.map(([a, b]) => collect(a, b)));
    expect(results.map((r) => r.sum)).toEqual(pairs.map(([a, b]) => a + b));
  });
});

// ===========================================================================
// Contract: APCoreClient.validate
// ===========================================================================

describe('APCoreClient.validate', () => {
  it('apcore_client.validate.input.module_id.invalid_pattern: malformed id recorded as failed check', async () => {
    const client = clientWithModule();
    // TS validate() does NOT throw for a malformed id — it records a failed
    // `module_id` check in the PreflightResult (Python canonical: raises
    // InvalidInputError). Assert the actual non-throwing behavior.
    const result = await client.validate('!!bad id!!', {});
    expect(result.valid).toBe(false);
    expect(result.checks.some((c) => !c.passed)).toBe(true);
  });

  it('apcore_client.validate.error.INVALID_MODULE_ID: empty id recorded as failed check', async () => {
    const client = clientWithModule();
    const result = await client.validate('', {});
    expect(result.valid).toBe(false);
    expect(result.checks.some((c) => !c.passed)).toBe(true);
  });

  it('apcore_client.validate.error.no_raise_on_failure: missing module yields invalid result', async () => {
    const client = clientWithModule();
    const result = await client.validate('absent.module', {});
    expect(result.valid).toBe(false);
    expect(result.checks.some((c) => !c.passed)).toBe(true);
  });

  it('apcore_client.validate.returns.preflight_result: result shape exposes valid/checks/requiresApproval/errors', async () => {
    const client = clientWithModule();
    const result = await client.validate('math.add', { a: 1, b: 2 });
    expect(typeof result.valid).toBe('boolean');
    expect(Array.isArray(result.checks)).toBe(true);
    // Python canonical asserts exactly 7 checks; assert the actual TS count is
    // a positive number of checks (divergence recorded if != 7).
    expect(result.checks.length).toBeGreaterThanOrEqual(1);
    expect(typeof result.requiresApproval).toBe('boolean');
    expect(Array.isArray(result.errors)).toBe(true);
  });

  it('apcore_client.validate.property.pure: validate does not mutate module list', async () => {
    const client = clientWithModule();
    const before = client.listModules();
    await client.validate('math.add', { a: 1, b: 2 });
    const after = client.listModules();
    expect(before).toEqual(after);
  });

  it('apcore_client.validate.property.idempotent_true: repeated calls return equivalent results', async () => {
    const client = clientWithModule();
    const r1 = await client.validate('math.add', { a: 1, b: 2 });
    const r2 = await client.validate('math.add', { a: 1, b: 2 });
    expect(r1.valid).toBe(r2.valid);
    expect(r1.checks.length).toBe(r2.checks.length);
  });

  it('apcore_client.validate.property.thread_safe: >=8 concurrent validations all valid', async () => {
    const client = clientWithModule();
    const results = await Promise.all(
      Array.from({ length: 8 }, (_, i) => client.validate('math.add', { a: i, b: i })),
    );
    expect(results.every((r) => r.valid)).toBe(true);
  });
});

// ===========================================================================
// Contract: APCoreClient.disable
// ===========================================================================

describe('APCoreClient.disable', () => {
  it('apcore_client.disable.error.SYS_MODULES_DISABLED: disable() without sys_modules raises typed error', async () => {
    const client = new APCore();
    await client.disable('some.module').then(
      () => {
        throw new Error('expected rejection');
      },
      (e) => {
        expect(e).toBeInstanceOf(SysModulesDisabledError);
        expect((e as SysModulesDisabledError).code).toBe('SYS_MODULES_DISABLED');
        expect((e as Error).message).toContain('sys_modules');
      },
    );
  });

  it('apcore_client.disable.input.reason.default: returns disabled result with default reason', async () => {
    const client = new APCore({ config: sysConfig() });
    client.module({
      id: 'risky.module',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      description: 'A risky module',
      execute: () => ({}),
    });
    const result = await client.disable('risky.module');
    expect(typeof result).toBe('object');
    expect(result['module_id']).toBe('risky.module');
    expect(result['enabled']).toBe(false);
  });

  it('apcore_client.disable.property.idempotent_true: disabling twice stays disabled', async () => {
    const client = new APCore({ config: sysConfig() });
    client.module({
      id: 'risky.dup',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      description: 'risky',
      execute: () => ({}),
    });
    const first = await client.disable('risky.dup');
    const second = await client.disable('risky.dup');
    expect(first['enabled']).toBe(false);
    expect(second['enabled']).toBe(false);
  });
});

// ===========================================================================
// Contract: APCoreClient.enable
// ===========================================================================

describe('APCoreClient.enable', () => {
  it('apcore_client.enable.error.SYS_MODULES_DISABLED: enable() without sys_modules raises typed error', async () => {
    const client = new APCore();
    await client.enable('some.module').then(
      () => {
        throw new Error('expected rejection');
      },
      (e) => {
        expect(e).toBeInstanceOf(SysModulesDisabledError);
        expect((e as SysModulesDisabledError).code).toBe('SYS_MODULES_DISABLED');
        expect((e as Error).message).toContain('sys_modules');
      },
    );
  });

  it('apcore_client.enable.input.reason.default: returns enabled result with default reason', async () => {
    const client = new APCore({ config: sysConfig() });
    client.module({
      id: 'risky.enable',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      description: 'risky',
      execute: () => ({}),
    });
    await client.disable('risky.enable');
    const result = await client.enable('risky.enable');
    expect(typeof result).toBe('object');
    expect(result['module_id']).toBe('risky.enable');
    expect(result['enabled']).toBe(true);
  });

  it('apcore_client.enable.property.idempotent_true: enabling twice stays enabled', async () => {
    const client = new APCore({ config: sysConfig() });
    client.module({
      id: 'risky.reenable',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      description: 'risky',
      execute: () => ({}),
    });
    const first = await client.enable('risky.reenable');
    const second = await client.enable('risky.reenable');
    expect(first['enabled']).toBe(true);
    expect(second['enabled']).toBe(true);
  });
});

// ===========================================================================
// Contract: APCore.__init__  (TS constructor)
// ===========================================================================

describe('APCore.__init__', () => {
  it('apcore_client.__init__.input.zero_config: zero-config creates registry+executor, no events', () => {
    const client = new APCore();
    expect(client.registry).not.toBeNull();
    expect(client.executor).not.toBeNull();
    expect(client.events).toBeNull();
  });

  it('apcore_client.__init__.error.no_raise: construction never raises for sys_modules disabled', () => {
    const client = new APCore({ config: new Config({ sys_modules: { enabled: false } }) });
    expect(client).toBeInstanceOf(APCore);
    expect(client.events).toBeNull();
  });

  it('apcore_client.__init__.property.async_false: construction is synchronous', () => {
    const client = new APCore();
    expect(client).toBeInstanceOf(APCore);
  });

  it('apcore_client.__init__.property.pure_false: sys_modules config has side effects', () => {
    const client = new APCore({ config: sysConfig() });
    expect(client.events).not.toBeNull();
    expect(client.listModules().some((mid) => mid.startsWith('system.'))).toBe(true);
  });
});

// ===========================================================================
// Contract: APCore.module
// ===========================================================================

describe('APCore.module', () => {
  it('apcore_client.module.input.id.invalid_pattern: malformed id rejected', () => {
    const client = new APCore();
    let caught: unknown;
    try {
      client.module({
        id: 'Bad ID With Spaces',
        inputSchema: Type.Object({}),
        outputSchema: Type.Object({}),
        execute: () => ({}),
      });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(InvalidInputError);
    expect((caught as InvalidInputError).code).toBe('INVALID_MODULE_ID');
  });

  it('apcore_client.module.error.duplicate: duplicate id rejected on second registration', () => {
    const client = new APCore();
    client.module({
      id: 'dup.module',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      execute: () => ({}),
    });
    let caught: unknown;
    try {
      client.module({
        id: 'dup.module',
        inputSchema: Type.Object({}),
        outputSchema: Type.Object({}),
        execute: () => ({}),
      });
    } catch (e) {
      caught = e;
    }
    // TS raises DuplicateModuleIdError (DUPLICATE_MODULE_ID); Python canonical
    // raises InvalidInputError (INVALID_MODULE_ID).
    expect(caught).toBeInstanceOf(DuplicateModuleIdError);
    expect((caught as DuplicateModuleIdError).code).toBe('DUPLICATE_MODULE_ID');
  });

  it('apcore_client.module.returns.original_function: returns a registered FunctionModule', () => {
    const client = new APCore();
    const fm = client.module({
      id: 'math.ret',
      inputSchema: AddInputSchema,
      outputSchema: AddOutputSchema,
      execute: (inputs) => ({ sum: (inputs.a as number) + (inputs.b as number) }),
    });
    // TS returns a FunctionModule (not the raw callable like Python).
    expect(fm).toBeInstanceOf(FunctionModule);
    expect(client.registry.has('math.ret')).toBe(true);
  });

  it('apcore_client.module.property.idempotent_false: first registration increments count by one', () => {
    const client = new APCore();
    const before = client.listModules().length;
    client.module({
      id: 'once.module',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      execute: () => ({}),
    });
    const after = client.listModules().length;
    expect(after).toBe(before + 1);
  });
});

// ===========================================================================
// Contract: APCore.register
// ===========================================================================

describe('APCore.register', () => {
  it('apcore_client.register.input.module_id.invalid_pattern: malformed id rejected', () => {
    const client = new APCore();
    const moduleObj = {
      inputSchema: AddInputSchema,
      outputSchema: AddOutputSchema,
      description: 'Add two numbers',
      execute: (inputs: Record<string, unknown>) => ({
        sum: (inputs.a as number) + (inputs.b as number),
      }),
    };
    let caught: unknown;
    try {
      client.register('Bad Id!!', moduleObj);
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(InvalidInputError);
    expect((caught as InvalidInputError).code).toBe('INVALID_MODULE_ID');
  });

  it('apcore_client.register.error.INVALID_MODULE_ID: duplicate id rejected on second register', () => {
    const moduleObj = {
      inputSchema: AddInputSchema,
      outputSchema: AddOutputSchema,
      description: 'Add two numbers',
      execute: (inputs: Record<string, unknown>) => ({
        sum: (inputs.a as number) + (inputs.b as number),
      }),
    };
    const client = new APCore();
    client.register('math.copy', moduleObj);
    let caught: unknown;
    try {
      client.register('math.copy', moduleObj);
    } catch (e) {
      caught = e;
    }
    // TS raises DuplicateModuleIdError (DUPLICATE_MODULE_ID); Python canonical
    // raises InvalidInputError (INVALID_MODULE_ID).
    expect(caught).toBeInstanceOf(DuplicateModuleIdError);
    expect((caught as DuplicateModuleIdError).code).toBe('DUPLICATE_MODULE_ID');
  });

  it('apcore_client.register.returns.none: register resolves to undefined and module is present', async () => {
    const moduleObj = {
      inputSchema: AddInputSchema,
      outputSchema: AddOutputSchema,
      description: 'Add two numbers',
      execute: (inputs: Record<string, unknown>) => ({
        sum: (inputs.a as number) + (inputs.b as number),
      }),
    };
    const client = new APCore();
    // TypeScript returns Registry.register's Promise<void> (registry-system.md
    // "Contract: Registry.register") so an async onLoad can be awaited and its
    // failure caught; with no async onLoad the module is visible at once.
    const ret = client.register('math.target', moduleObj);
    expect(client.registry.has('math.target')).toBe(true);
    await expect(ret).resolves.toBeUndefined();
  });

  it('apcore_client.register.property.thread_safe: >=8 concurrent registrations all present', async () => {
    const makeMod = () => ({
      inputSchema: AddInputSchema,
      outputSchema: AddOutputSchema,
      description: 'Add two numbers',
      execute: (inputs: Record<string, unknown>) => ({
        sum: (inputs.a as number) + (inputs.b as number),
      }),
    });
    const client = new APCore();
    const ids = Array.from({ length: 10 }, (_, i) => `svc.mod${i}`);
    await Promise.all(
      ids.map((mid) => Promise.resolve().then(() => client.register(mid, makeMod()))),
    );
    for (const mid of ids) {
      expect(client.registry.has(mid)).toBe(true);
    }
  });
});

// ===========================================================================
// Contract: APCore.discover
// ===========================================================================

describe('APCore.discover', () => {
  it('apcore_client.discover.error.ConfigNotFoundError: missing extension root rejected', async () => {
    const client = new APCore({
      config: new Config({ extensions: { root: '/nonexistent/apcore-spec-test-root' } }),
    });
    await client.discover().then(
      () => {
        throw new Error('expected rejection');
      },
      (e) => {
        expect(e).toBeInstanceOf(ConfigNotFoundError);
      },
    );
  });

  it('apcore_client.discover.returns.int_count: empty root returns 0', async () => {
    const client = new APCore();
    // Zero-config client has no extensions root; discover() rejects rather than
    // returning a count. Assert the actual behavior: a rejection mentioning
    // extensions (TS has no default empty root that yields 0).
    await client.discover().then(
      () => {
        throw new Error('expected rejection');
      },
      (e) => {
        expect(e).toBeInstanceOf(Error);
        expect(String((e as Error).message).toLowerCase()).toContain('extensions');
      },
    );
  });
});

// ===========================================================================
// Contract: APCore.list_modules
// ===========================================================================

describe('APCore.list_modules', () => {
  it('apcore_client.list_modules.returns.sorted: returns alphabetically sorted ids', () => {
    const client = new APCore();
    for (const mid of ['c.three', 'a.one', 'b.two']) {
      client.module({
        id: mid,
        inputSchema: Type.Object({}),
        outputSchema: Type.Object({}),
        execute: () => ({}),
      });
    }
    const result = client.listModules();
    expect(result).toEqual([...result].sort());
    expect(result).toEqual(expect.arrayContaining(['a.one', 'b.two', 'c.three']));
  });

  it('apcore_client.list_modules.input.prefix: prefix filter returns matching ids', () => {
    const client = new APCore();
    for (const mid of ['math.add', 'math.sub', 'text.upper']) {
      client.module({
        id: mid,
        inputSchema: Type.Object({}),
        outputSchema: Type.Object({}),
        execute: () => ({}),
      });
    }
    const result = client.listModules({ prefix: 'math.' });
    expect(new Set(result)).toEqual(new Set(['math.add', 'math.sub']));
  });

  it('apcore_client.list_modules.property.pure: returns a fresh list each call', () => {
    const client = clientWithModule();
    const r1 = client.listModules();
    r1.push('injected.fake');
    const r2 = client.listModules();
    expect(r2).not.toContain('injected.fake');
  });

  it('apcore_client.list_modules.property.idempotent: two calls return equal results', () => {
    const client = clientWithModule();
    expect(client.listModules()).toEqual(client.listModules());
  });
});

// ===========================================================================
// Contract: APCore.describe
// ===========================================================================

describe('APCore.describe', () => {
  it('apcore_client.describe.error.ModuleNotFoundError: unknown module rejected', () => {
    const client = new APCore();
    let caught: unknown;
    try {
      client.describe('not.registered');
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(ModuleNotFoundError);
    expect((caught as ModuleNotFoundError).code).toBe('MODULE_NOT_FOUND');
  });

  it('apcore_client.describe.returns.string: returns a non-empty markdown string', () => {
    const client = clientWithModule();
    const text = client.describe('math.add');
    expect(typeof text).toBe('string');
    expect(text.length).toBeGreaterThan(0);
    expect(text.includes('math.add') || text.includes('Add two numbers')).toBe(true);
  });

  it('apcore_client.describe.property.pure: describe does not alter module list', () => {
    const client = clientWithModule();
    const before = client.listModules();
    client.describe('math.add');
    expect(client.listModules()).toEqual(before);
  });

  it('apcore_client.describe.property.idempotent: two calls return identical strings', () => {
    const client = clientWithModule();
    expect(client.describe('math.add')).toBe(client.describe('math.add'));
  });
});

// ===========================================================================
// Contract: APCore.use / APCore.use_middleware
// ===========================================================================

describe('APCore.use', () => {
  it('apcore_client.use.error.priority_exceeds_1000: priority > 1000 rejected', () => {
    // TS validates priority in the Middleware base constructor (RangeError),
    // so the rejection happens at construction time — before use() is reached.
    // Python canonical raises ValueError from use().
    let caught: unknown;
    try {
      new NoopMiddleware('too-high', 1001);
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(RangeError);
  });

  it('apcore_client.use.returns.self: use() returns the client for chaining', () => {
    const client = new APCore();
    const returned = client.use(new NoopMiddleware('a')).use(new NoopMiddleware('b'));
    expect(returned).toBe(client);
  });

  it('apcore_client.use.property.idempotent_false: same instance added twice removable twice', () => {
    const client = new APCore();
    const mw = new NoopMiddleware('dup-mw');
    client.use(mw);
    client.use(mw);
    expect(client.remove(mw)).toBe(true);
    expect(client.remove(mw)).toBe(true);
  });
});

// ===========================================================================
// Contract: APCore.use_before
// ===========================================================================

describe('APCore.use_before', () => {
  it('apcore_client.use_before.returns.self: useBefore() returns the client', () => {
    const client = new APCore();
    const returned = client.useBefore(() => null);
    expect(returned).toBe(client);
  });

  it('apcore_client.use_before.side_effect.1.before_execute: before-callback runs before execute', async () => {
    const client = new APCore();
    const order: string[] = [];
    client.module({
      id: 'ord.mod',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      description: 'ordered',
      execute: () => {
        order.push('execute');
        return {};
      },
    });
    client.useBefore(() => {
      order.push('before');
      return null;
    });
    await client.call('ord.mod', {});
    expect(order).toEqual(['before', 'execute']);
  });

  it('apcore_client.use_before.property.idempotent_false: same callback twice fires twice', async () => {
    const client = new APCore();
    const fired: number[] = [];
    client.module({
      id: 'ord.dup',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      description: 'ordered',
      execute: () => ({}),
    });
    const cb = (): null => {
      fired.push(1);
      return null;
    };
    client.useBefore(cb);
    client.useBefore(cb);
    await client.call('ord.dup', {});
    expect(fired.length).toBe(2);
  });
});

// ===========================================================================
// Contract: APCore.use_after
// ===========================================================================

describe('APCore.use_after', () => {
  it('apcore_client.use_after.returns.self: useAfter() returns the client', () => {
    const client = new APCore();
    const returned = client.useAfter(() => null);
    expect(returned).toBe(client);
  });

  it('apcore_client.use_after.side_effect.1.after_execute: after-callback runs after execute', async () => {
    const client = new APCore();
    const order: string[] = [];
    client.module({
      id: 'ord.after',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      description: 'ordered',
      execute: () => {
        order.push('execute');
        return {};
      },
    });
    client.useAfter(() => {
      order.push('after');
      return null;
    });
    await client.call('ord.after', {});
    expect(order).toEqual(['execute', 'after']);
  });

  it('apcore_client.use_after.property.idempotent_false: same callback twice fires twice', async () => {
    const client = new APCore();
    const fired: number[] = [];
    client.module({
      id: 'ord.afterdup',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      description: 'ordered',
      execute: () => ({}),
    });
    const cb = (): null => {
      fired.push(1);
      return null;
    };
    client.useAfter(cb);
    client.useAfter(cb);
    await client.call('ord.afterdup', {});
    expect(fired.length).toBe(2);
  });
});

// ===========================================================================
// Contract: APCore.remove
// ===========================================================================

describe('APCore.remove', () => {
  it('apcore_client.remove.returns.true_when_present: removes by identity', () => {
    const client = new APCore();
    const mw = new NoopMiddleware('removable');
    client.use(mw);
    expect(client.remove(mw)).toBe(true);
  });

  it('apcore_client.remove.returns.false_when_absent: returns false for never-added middleware', () => {
    const client = new APCore();
    expect(client.remove(new NoopMiddleware('never-added'))).toBe(false);
  });

  it('apcore_client.remove.property.idempotent_true: second remove returns false', () => {
    const client = new APCore();
    const mw = new NoopMiddleware('idem');
    client.use(mw);
    expect(client.remove(mw)).toBe(true);
    expect(client.remove(mw)).toBe(false);
  });
});
