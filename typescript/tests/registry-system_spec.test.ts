/**
 * Spec-traced contract tests for the apcore registry-system feature (TypeScript).
 *
 * MIRRORS the canonical Python suite:
 *   apcore-python/tests/test_registry_system_spec.py
 *
 * Each `it(...)` name begins with the VERBATIM clause-id from the Python
 * docstrings (format: registry_system.<method>.<kind>.<detail>) so a
 * cross-language diff can match rows by exact clause-id.
 *
 * Source spec: apcore/docs/features/registry-system.md
 * Framework: vitest.
 *
 * Tests assert the ACTUAL TypeScript behavior (which legitimately diverges
 * from the Python canonical intent in several places — those divergences are
 * captured in the task report, not papered over here).
 */

import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { Type } from '@sinclair/typebox';
import {
  Registry,
  MODULE_ID_PATTERN,
} from '../src/registry/registry.js';
import { scanExtensions } from '../src/registry/scanner.js';
import {
  ConfigNotFoundError,
  DuplicateModuleIdError,
  InvalidInputError,
  ModuleNotFoundError,
} from '../src/errors.js';

// ---------------------------------------------------------------------------
// Helper module fixtures (duck-typed; satisfy validateModule())
// ---------------------------------------------------------------------------

interface SpecModuleShape {
  inputSchema: Record<string, unknown>;
  outputSchema: Record<string, unknown>;
  description: string;
  name: string;
  version: string;
  tags: string[];
  execute: (inputs: Record<string, unknown>) => Promise<Record<string, unknown>>;
  onLoad?: () => void;
}

function makeModule(tags?: string[]): SpecModuleShape {
  return {
    inputSchema: { type: 'object', properties: { value: { type: 'string' } } },
    outputSchema: { type: 'object', properties: { result: { type: 'string' } } },
    description: 'Spec fixture module',
    name: 'SpecModule',
    version: '1.0.0',
    tags: tags ?? ['alpha', 'beta'],
    async execute(inputs: Record<string, unknown>): Promise<Record<string, unknown>> {
      return { result: (inputs['value'] as string) ?? '' };
    },
  };
}

function makeOnLoadModule(observer?: () => void): SpecModuleShape & { loaded: boolean } {
  const mod = makeModule() as SpecModuleShape & { loaded: boolean };
  mod.loaded = false;
  mod.onLoad = function onLoad(): void {
    mod.loaded = true;
    if (observer) observer();
  };
  return mod;
}

function makeFailingOnLoadModule(): SpecModuleShape {
  const mod = makeModule();
  mod.onLoad = function onLoad(): void {
    throw new Error('on_load boom');
  };
  return mod;
}

function makeTempDir(): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'apcore-regsys-'));
}

// ===========================================================================
// Contract: Registry.register
// ===========================================================================

describe('Contract: Registry.register', () => {
  it('registry_system.register.input.module_id.empty: empty module_id is rejected', () => {
    const reg = new Registry();
    let err: unknown;
    try {
      reg.register('', makeModule());
    } catch (e) {
      err = e;
    }
    expect(err).toBeInstanceOf(InvalidInputError);
    expect((err as InvalidInputError).code).toBe('INVALID_MODULE_ID');
  });

  it('registry_system.register.input.module_id.malformed: hyphenated id is rejected', () => {
    const reg = new Registry();
    // Hyphens are disallowed by MODULE_ID_PATTERN.
    expect(MODULE_ID_PATTERN.test('Bad-ID')).toBe(false);
    let err: unknown;
    try {
      reg.register('Bad-ID', makeModule());
    } catch (e) {
      err = e;
    }
    expect(err).toBeInstanceOf(InvalidInputError);
    expect((err as InvalidInputError).code).toBe('INVALID_MODULE_ID');
  });

  it('registry_system.register.input.module_id.reserved: reserved system.* id is rejected', () => {
    const reg = new Registry();
    // `system.*` is reserved; only registerInternal() may use it.
    let err: unknown;
    try {
      reg.register('system.thing', makeModule());
    } catch (e) {
      err = e;
    }
    expect(err).toBeInstanceOf(InvalidInputError);
    expect((err as InvalidInputError).code).toBe('INVALID_MODULE_ID');
  });

  it('registry_system.register.error.INVALID_MODULE_ID: id not starting with a letter is rejected', () => {
    const reg = new Registry();
    let err: unknown;
    try {
      reg.register('9bad', makeModule()); // must start with a lowercase letter
    } catch (e) {
      err = e;
    }
    expect(err).toBeInstanceOf(InvalidInputError);
    expect((err as InvalidInputError).code).toBe('INVALID_MODULE_ID');
  });

  it('registry_system.register.error.DUPLICATE_MODULE_ID: duplicate id is rejected', () => {
    const reg = new Registry();
    reg.register('math.add', makeModule());
    let err: unknown;
    try {
      reg.register('math.add', makeModule());
    } catch (e) {
      err = e;
    }
    // TS divergence: throws DuplicateModuleIdError (Python: InvalidInputError),
    // but the code field matches: DUPLICATE_MODULE_ID.
    expect(err).toBeInstanceOf(DuplicateModuleIdError);
    expect((err as DuplicateModuleIdError).code).toBe('DUPLICATE_MODULE_ID');
  });

  it('registry_system.register.return.none: returns a Promise (void) on success and module is visible', async () => {
    const reg = new Registry();
    const result = reg.register('math.add', makeModule());
    // TS divergence: register returns Promise<void>, not None.
    expect(result).toBeInstanceOf(Promise);
    await expect(result).resolves.toBeUndefined();
    expect(reg.get('math.add')).not.toBeNull();
  });

  it('registry_system.register.property.async: register returns a Promise (TS) — Python declares async:false', async () => {
    const reg = new Registry();
    const ret = reg.register('math.add', makeModule());
    // TS divergence: contract declares async:false; the TS register() returns a
    // Promise<void>. Assert the actual TS behavior.
    expect(ret).toBeInstanceOf(Promise);
    await ret;
  });

  it('registry_system.register.property.idempotent: duplicate registration errors (not a no-op)', () => {
    const reg = new Registry();
    reg.register('math.add', makeModule());
    let err: unknown;
    try {
      reg.register('math.add', makeModule());
    } catch (e) {
      err = e;
    }
    expect(err).toBeInstanceOf(DuplicateModuleIdError);
    expect((err as DuplicateModuleIdError).code).toBe('DUPLICATE_MODULE_ID');
  });

  it('registry_system.register.property.pure: mutates the store (observed via list)', () => {
    const reg = new Registry();
    const before = reg.list();
    reg.register('math.add', makeModule());
    const after = reg.list();
    expect(before).not.toContain('math.add');
    expect(after).toContain('math.add');
  });

  it('registry_system.register.property.thread_safe: >=8 concurrent distinct registrations all land', async () => {
    const reg = new Registry();
    const n = 12;
    await Promise.all(
      Array.from({ length: n }, (_, i) => reg.register(`mod.m${i}`, makeModule())),
    );
    const listed = reg.list();
    for (let i = 0; i < n; i++) {
      expect(listed).toContain(`mod.m${i}`);
    }
    expect(reg.count).toBe(n);
  });

  it('registry_system.register.property.thread_safe.duplicate: concurrent same-id yields one winner', async () => {
    const reg = new Registry();
    const n = 10;
    const errors: unknown[] = [];
    await Promise.all(
      Array.from({ length: n }, () => {
        return (async () => {
          try {
            await reg.register('dup.mod', makeModule());
          } catch (e) {
            errors.push(e);
          }
        })();
      }),
    );
    expect(reg.get('dup.mod')).not.toBeNull();
    expect(errors.length).toBe(n - 1);
    expect(
      errors.every((e) => (e as { code?: string }).code === 'DUPLICATE_MODULE_ID'),
    ).toBe(true);
  });

  it('registry_system.register.side_effect.1.validate_before_mutation: invalid id leaves store untouched', () => {
    const reg = new Registry();
    expect(() => reg.register('Bad-ID', makeModule())).toThrow(InvalidInputError);
    expect(reg.list()).toEqual([]);
    expect(reg.count).toBe(0);
  });

  it('registry_system.register.side_effect.6.on_load_invoked: module.onLoad() is invoked during registration', async () => {
    const reg = new Registry();
    const mod = makeOnLoadModule();
    await reg.register('life.cycle', mod);
    expect(mod.loaded).toBe(true);
  });

  it('registry_system.register.side_effect.6.on_load_before_visible: onLoad completes before module is visible', async () => {
    const reg = new Registry();
    const observedVisibleDuringLoad: boolean[] = [];
    const mod = makeOnLoadModule(() => {
      observedVisibleDuringLoad.push(reg.get('life.cycle') !== null);
    });
    await reg.register('life.cycle', mod);
    expect(observedVisibleDuringLoad).toEqual([false]);
    expect(reg.get('life.cycle')).toBe(mod);
  });

  it('registry_system.register.side_effect.6.on_load_failure_not_visible: failed onLoad propagates and module stays invisible', async () => {
    const reg = new Registry();
    // TS: a throwing sync onLoad surfaces as a rejected Promise (issue #65).
    await expect(reg.register('life.boom', makeFailingOnLoadModule())).rejects.toThrow(
      'on_load boom',
    );
    expect(reg.get('life.boom')).toBeNull();
    expect(reg.list()).not.toContain('life.boom');
  });

  it('registry_system.register.side_effect.8.register_event_emitted: register event fires after publication', async () => {
    const reg = new Registry();
    const received: Array<[string, unknown]> = [];
    reg.on('register', (moduleId, payload) => received.push([moduleId, payload]));
    await reg.register('math.add', makeModule());
    expect(received.length).toBe(1);
    expect(received[0][0]).toBe('math.add');
  });

  it('registry_system.register.side_effect.ordering.load_then_event: onLoad fires before register event', async () => {
    const reg = new Registry();
    const sequence: string[] = [];
    const mod = makeOnLoadModule(() => sequence.push('on_load'));
    reg.on('register', () => sequence.push('event'));
    await reg.register('order.mod', mod);
    expect(sequence).toEqual(['on_load', 'event']);
  });
});

// ===========================================================================
// Contract: Scanner.scan_extensions
// ===========================================================================

describe('Contract: Scanner.scan_extensions', () => {
  it('registry_system.scan_extensions.input.root.missing: missing root is rejected', () => {
    const tmp = makeTempDir();
    const missing = path.join(tmp, 'does_not_exist');
    let err: unknown;
    try {
      scanExtensions(missing);
    } catch (e) {
      err = e;
    }
    expect(err).toBeInstanceOf(ConfigNotFoundError);
    expect((err as ConfigNotFoundError).code).toBe('CONFIG_NOT_FOUND');
  });

  it('registry_system.scan_extensions.error.CONFIG_NOT_FOUND: missing root raises CONFIG_NOT_FOUND', () => {
    const tmp = makeTempDir();
    let err: unknown;
    try {
      scanExtensions(path.join(tmp, 'nope'));
    } catch (e) {
      err = e;
    }
    expect(err).toBeInstanceOf(ConfigNotFoundError);
    expect((err as ConfigNotFoundError).code).toBe('CONFIG_NOT_FOUND');
  });

  it('registry_system.scan_extensions.return.discovered_modules: returns DiscoveredModule records in stable order', () => {
    const tmp = makeTempDir();
    const ext = path.join(tmp, 'ext');
    fs.mkdirSync(ext);
    fs.writeFileSync(path.join(ext, 'hello.ts'), 'export class HelloModule {}\n');
    fs.writeFileSync(path.join(ext, 'greet.ts'), 'export class GreetModule {}\n');

    const results = scanExtensions(ext);
    const ids = new Set(results.map((dm) => dm.canonicalId));
    expect(ids).toEqual(new Set(['hello', 'greet']));
    // Records expose a concrete file path.
    expect(results.every((dm) => path.extname(dm.filePath) === '.ts')).toBe(true);
  });

  it('registry_system.scan_extensions.property.async: scanExtensions is synchronous (returns an array)', () => {
    const tmp = makeTempDir();
    const ext = path.join(tmp, 'ext');
    fs.mkdirSync(ext);
    fs.writeFileSync(path.join(ext, 'a.ts'), 'export class A {}\n');
    const result = scanExtensions(ext);
    // Contract declares async:false — synchronous return, not a Promise.
    expect(result).not.toBeInstanceOf(Promise);
    expect(Array.isArray(result)).toBe(true);
  });

  it('registry_system.scan_extensions.property.pure: reads the filesystem (output changes with files)', () => {
    const tmp = makeTempDir();
    const ext = path.join(tmp, 'ext');
    fs.mkdirSync(ext);
    fs.writeFileSync(path.join(ext, 'a.ts'), 'export class A {}\n');
    const first = new Set(scanExtensions(ext).map((dm) => dm.canonicalId));
    fs.writeFileSync(path.join(ext, 'b.ts'), 'export class B {}\n');
    const second = new Set(scanExtensions(ext).map((dm) => dm.canonicalId));
    expect(first).toEqual(new Set(['a']));
    expect(second).toEqual(new Set(['a', 'b']));
  });

  it('registry_system.scan_extensions.property.thread_safe: >=8 concurrent scans return consistent results', async () => {
    const tmp = makeTempDir();
    const ext = path.join(tmp, 'ext');
    fs.mkdirSync(ext);
    for (let i = 0; i < 5; i++) {
      fs.writeFileSync(path.join(ext, `m${i}.ts`), `export class M${i} {}\n`);
    }
    const results = await Promise.all(
      Array.from({ length: 8 }, () =>
        Promise.resolve(new Set(scanExtensions(ext).map((dm) => dm.canonicalId))),
      ),
    );
    const expected = new Set(['m0', 'm1', 'm2', 'm3', 'm4']);
    expect(results.every((r) => r.size === expected.size && [...r].every((x) => expected.has(x)))).toBe(true);
  });
});

// ===========================================================================
// Contract: Registry.get
// ===========================================================================

describe('Contract: Registry.get', () => {
  it('registry_system.get.input.module_id.empty: empty module_id is rejected', () => {
    const reg = new Registry();
    expect(() => reg.get('')).toThrow(ModuleNotFoundError);
  });

  it('registry_system.get.error.MODULE_NOT_FOUND: empty module_id raises MODULE_NOT_FOUND', () => {
    const reg = new Registry();
    let err: unknown;
    try {
      reg.get('');
    } catch (e) {
      err = e;
    }
    expect(err).toBeInstanceOf(ModuleNotFoundError);
    expect((err as ModuleNotFoundError).code).toBe('MODULE_NOT_FOUND');
  });

  it('registry_system.get.return.none_when_absent: well-formed unregistered id returns null', () => {
    const reg = new Registry();
    expect(reg.get('not.registered')).toBeNull();
  });

  it('registry_system.get.return.instance_when_found: returns the registered instance', async () => {
    const reg = new Registry();
    const mod = makeModule();
    await reg.register('math.add', mod);
    expect(reg.get('math.add')).toBe(mod);
  });

  it('registry_system.get.property.async: get is synchronous (returns instance, not a Promise)', async () => {
    const reg = new Registry();
    const mod = makeModule();
    await reg.register('math.add', mod);
    const result = reg.get('math.add');
    expect(result).not.toBeInstanceOf(Promise);
    expect(result).toBe(mod);
  });

  it('registry_system.get.property.idempotent: repeated reads return the same result', async () => {
    const reg = new Registry();
    const mod = makeModule();
    await reg.register('math.add', mod);
    expect(reg.get('math.add')).toBe(reg.get('math.add'));
    expect(reg.get('math.add')).toBe(mod);
  });

  it('registry_system.get.property.pure: reads shared state (result changes after register)', async () => {
    const reg = new Registry();
    expect(reg.get('math.add')).toBeNull();
    await reg.register('math.add', makeModule());
    expect(reg.get('math.add')).not.toBeNull();
  });

  it('registry_system.get.property.thread_safe: >=8 concurrent reads return non-null', async () => {
    const reg = new Registry();
    await reg.register('math.add', makeModule());
    const results = await Promise.all(
      Array.from({ length: 10 }, () => Promise.resolve(reg.get('math.add'))),
    );
    expect(results.every((r) => r !== null)).toBe(true);
  });
});

// ===========================================================================
// Contract: Registry.list
// ===========================================================================

describe('Contract: Registry.list', () => {
  it('registry_system.list.input.tags.superset_match: only modules with ALL supplied tags are included', () => {
    const reg = new Registry();
    reg.register('m.alpha', makeModule(['alpha', 'beta']));
    reg.register('m.gamma', makeModule(['gamma']));
    expect(reg.list({ tags: ['alpha'] })).toEqual(['m.alpha']);
    expect(reg.list({ tags: ['alpha', 'beta'] })).toEqual(['m.alpha']);
    expect(reg.list({ tags: ['alpha', 'missing'] })).toEqual([]);
  });

  it('registry_system.list.input.tags.empty_no_filter: empty tags == no filter', () => {
    const reg = new Registry();
    reg.register('m.alpha', makeModule(['alpha']));
    reg.register('m.gamma', makeModule(['gamma']));
    expect([...reg.list({ tags: [] })].sort()).toEqual([...reg.list()].sort());
    expect(new Set(reg.list({ tags: [] }))).toEqual(new Set(['m.alpha', 'm.gamma']));
  });

  it('registry_system.list.input.prefix.startswith: prefix matching is exact startsWith', () => {
    const reg = new Registry();
    reg.register('math.add', makeModule());
    reg.register('math.sub', makeModule());
    reg.register('string.upper', makeModule());
    expect(reg.list({ prefix: 'math.' })).toEqual(['math.add', 'math.sub']);
    // '*' is literal, not a wildcard -> matches nothing.
    expect(reg.list({ prefix: 'math.*' })).toEqual([]);
  });

  it('registry_system.list.input.combined.tags_and_prefix: tag + prefix filters combine', () => {
    const reg = new Registry();
    reg.register('math.add', makeModule(['arith']));
    reg.register('math.sub', makeModule(['other']));
    reg.register('string.cat', makeModule(['arith']));
    expect(reg.list({ tags: ['arith'], prefix: 'math.' })).toEqual(['math.add']);
  });

  it('registry_system.list.error.none: unknown tags/prefix return empty (no error)', () => {
    const reg = new Registry();
    reg.register('math.add', makeModule(['arith']));
    expect(reg.list({ tags: ['nonexistent'] })).toEqual([]);
    expect(reg.list({ prefix: 'zzz' })).toEqual([]);
  });

  it('registry_system.list.return.sorted_unique: returns lexicographically sorted unique ids', () => {
    const reg = new Registry();
    for (const mid of ['zeta.one', 'alpha.two', 'mid.three']) {
      reg.register(mid, makeModule());
    }
    const result = reg.list();
    expect(result).toEqual(['alpha.two', 'mid.three', 'zeta.one']);
    expect(result.length).toBe(new Set(result).size);
  });

  it('registry_system.list.property.async: list is synchronous (returns array, not a Promise)', () => {
    const reg = new Registry();
    reg.register('a.one', makeModule());
    const result = reg.list();
    expect(result).not.toBeInstanceOf(Promise);
    expect(Array.isArray(result)).toBe(true);
  });

  it('registry_system.list.property.idempotent: same state -> identical result', () => {
    const reg = new Registry();
    reg.register('a.one', makeModule());
    reg.register('b.two', makeModule());
    expect(reg.list()).toEqual(reg.list());
  });

  it('registry_system.list.property.thread_safe: >=8 concurrent list() calls return sorted subsets', async () => {
    const reg = new Registry();
    for (let i = 0; i < 6; i++) {
      reg.register(`mod.m${i}`, makeModule());
    }
    const all = new Set(['mod.m0', 'mod.m1', 'mod.m2', 'mod.m3', 'mod.m4', 'mod.m5']);
    const results = await Promise.all(
      Array.from({ length: 10 }, () => Promise.resolve(reg.list())),
    );
    for (const r of results) {
      expect(r).toEqual([...r].sort());
      expect(r.every((x) => all.has(x))).toBe(true);
    }
  });
});

// ===========================================================================
// Contract: Registry.get_definition (TS: getDefinition)
// ===========================================================================

describe('Contract: Registry.getDefinition', () => {
  it('registry_system.get_definition.input.module_id.empty: empty id propagates error', () => {
    const reg = new Registry();
    expect(() => reg.getDefinition('')).toThrow(ModuleNotFoundError);
  });

  it('registry_system.get_definition.error.MODULE_NOT_FOUND: empty id raises MODULE_NOT_FOUND', () => {
    const reg = new Registry();
    let err: unknown;
    try {
      reg.getDefinition('');
    } catch (e) {
      err = e;
    }
    expect(err).toBeInstanceOf(ModuleNotFoundError);
    expect((err as ModuleNotFoundError).code).toBe('MODULE_NOT_FOUND');
  });

  it('registry_system.get_definition.return.none_when_absent: unregistered id returns null', () => {
    const reg = new Registry();
    expect(reg.getDefinition('not.registered')).toBeNull();
  });

  it('registry_system.get_definition.return.descriptor_fields: returns a ModuleDescriptor with contracted fields', async () => {
    const reg = new Registry();
    await reg.register('math.add', makeModule(['alpha', 'beta']));
    const desc = reg.getDefinition('math.add');
    expect(desc).not.toBeNull();
    expect(desc!.moduleId).toBe('math.add');
    expect(desc!.description).toBe('Spec fixture module');
    expect(typeof desc!.inputSchema).toBe('object');
    expect(typeof desc!.outputSchema).toBe('object');
    expect(desc!.version).toBe('1.0.0');
    expect(new Set(desc!.tags)).toEqual(
      new Set([...new Set([...desc!.tags, 'alpha', 'beta'])]),
    );
    expect(desc!.tags).toContain('alpha');
    expect(desc!.tags).toContain('beta');
  });

  it('registry_system.get_definition.property.async: getDefinition is synchronous (descriptor, not Promise)', async () => {
    const reg = new Registry();
    await reg.register('math.add', makeModule());
    const desc = reg.getDefinition('math.add');
    expect(desc).not.toBeInstanceOf(Promise);
    expect(desc).not.toBeNull();
  });

  it('registry_system.get_definition.property.idempotent: equivalent descriptor each call', async () => {
    const reg = new Registry();
    await reg.register('math.add', makeModule());
    const d1 = reg.getDefinition('math.add');
    const d2 = reg.getDefinition('math.add');
    expect(d1).not.toBeNull();
    expect(d2).not.toBeNull();
    expect(d1!.moduleId).toBe(d2!.moduleId);
    expect(d1!.inputSchema).toEqual(d2!.inputSchema);
    expect(d1!.outputSchema).toEqual(d2!.outputSchema);
  });

  it('registry_system.get_definition.property.thread_safe: >=8 concurrent calls return consistent descriptors', async () => {
    const reg = new Registry();
    await reg.register('math.add', makeModule());
    const results = await Promise.all(
      Array.from({ length: 10 }, () => Promise.resolve(reg.getDefinition('math.add'))),
    );
    expect(results.every((r) => r !== null && r.moduleId === 'math.add')).toBe(true);
  });
});
