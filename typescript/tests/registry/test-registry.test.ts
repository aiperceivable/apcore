import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { mkdtempSync, writeFileSync, rmSync, mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { Type } from '@sinclair/typebox';
import { Registry, MAX_MODULE_ID_LENGTH } from '../../src/registry/registry.js';
import { FunctionModule } from '../../src/decorator.js';
import { DuplicateModuleIdError, InvalidInputError, ModuleNotFoundError } from '../../src/errors.js';
import { Config } from '../../src/config.js';
import { createAnnotations } from '../../src/module.js';
import { EventEmitter } from '../../src/events/emitter.js';
import type { ApCoreEvent } from '../../src/events/emitter.js';

function createMod(id: string): FunctionModule {
  return new FunctionModule({
    execute: () => ({ ok: true }),
    moduleId: id,
    inputSchema: Type.Object({}),
    outputSchema: Type.Object({ ok: Type.Boolean() }),
    description: `Module ${id}`,
  });
}

describe('Registry', () => {
  it('creates empty registry', () => {
    const registry = new Registry();
    expect(registry.count).toBe(0);
    expect(registry.list()).toEqual([]);
  });

  it('register and get module', () => {
    const registry = new Registry();
    const mod = createMod('test.a');
    registry.register('test.a', mod);
    expect(registry.get('test.a')).toBe(mod);
    expect(registry.has('test.a')).toBe(true);
    expect(registry.count).toBe(1);
  });

  it('get returns null for unknown module', () => {
    const registry = new Registry();
    expect(registry.get('unknown')).toBeNull();
  });

  it('get throws for empty string', () => {
    const registry = new Registry();
    expect(() => registry.get('')).toThrow(ModuleNotFoundError);
  });

  it('register throws for empty moduleId', () => {
    const registry = new Registry();
    expect(() => registry.register('', createMod('x'))).toThrow(InvalidInputError);
  });

  it('register throws for duplicate moduleId', () => {
    const registry = new Registry();
    registry.register('test.a', createMod('test.a'));
    expect(() => registry.register('test.a', createMod('test.a'))).toThrow(DuplicateModuleIdError);
  });

  // Regression: sync finding A-001 — register accepts optional `version` and
  // `metadata` parameters for cross-language signature parity with
  // apcore-python `Registry.register(module_id, module, version=None, metadata=None)`.
  // Multi-version coexistence is not implemented yet (single-version registry),
  // but the supplied values flow into the merged module metadata.
  it('register accepts optional version and metadata parameters (A-001 parity)', () => {
    const registry = new Registry();
    const mod = createMod('test.versioned');
    registry.register('test.versioned', mod, '2.5.0', { custom_tag: 'spec-compliance' });
    const def = registry.getDefinition('test.versioned');
    expect(def).not.toBeNull();
    expect(def?.version).toBe('2.5.0');
    // The arbitrary metadata override is layered into the merged metadata via
    // mergeModuleMetadata; readers can access it through getDefinition().
  });

  it('get accepts optional versionHint for cross-language API parity (A-002)', () => {
    const registry = new Registry();
    const mod = createMod('test.a');
    registry.register('test.a', mod);
    // Single-version registry: hint is accepted (no compile error) but
    // ignored — get() always returns the live registration.
    expect(registry.get('test.a', '^1.0.0')).toBe(mod);
    expect(registry.get('test.a', null)).toBe(mod);
    expect(registry.get('test.a')).toBe(mod);
  });

  it('MAX_MODULE_ID_LENGTH matches PROTOCOL_SPEC §2.7 (192)', () => {
    // Bumped from 128 in spec 1.6.0-draft (2026-04-08) to accommodate
    // Java/.NET deep-namespace FQN-derived IDs. Filesystem-safe:
    // 192 + ".binding.yaml".length = 205 < 255-byte filename limit.
    expect(MAX_MODULE_ID_LENGTH).toBe(192);
  });

  it('register accepts module ID at exactly MAX_MODULE_ID_LENGTH', () => {
    const registry = new Registry();
    // Pattern requires [a-z][a-z0-9_]* — pure 'a' run is valid.
    const exactId = 'a'.repeat(MAX_MODULE_ID_LENGTH);
    registry.register(exactId, createMod(exactId));
    expect(registry.has(exactId)).toBe(true);
  });

  it('register rejects module ID exceeding MAX_MODULE_ID_LENGTH', () => {
    const registry = new Registry();
    const overlongId = 'a'.repeat(MAX_MODULE_ID_LENGTH + 1);
    expect(() => registry.register(overlongId, createMod(overlongId))).toThrow(
      /maximum length/,
    );
  });

  // PROTOCOL_SPEC §2.7 EBNF compliance — parity with apcore-python and apcore-rust
  it('register rejects invalid pattern (uppercase, hyphens, leading digit, etc.)', () => {
    const registry = new Registry();
    for (const badId of [
      'INVALID-ID',
      '1abc',
      'Module',
      'a..b',
      '.leading',
      'trailing.',
      'has space',
      'has!bang',
    ]) {
      expect(
        () => registry.register(badId, createMod(badId)),
        `pattern-invalid '${badId}' must throw`,
      ).toThrow(/Invalid module ID|Must match pattern/);
    }
  });

  it('register rejects reserved word as first segment', () => {
    const registry = new Registry();
    expect(() => registry.register('system.health', createMod('system.health'))).toThrow(
      /reserved word/,
    );
    expect(() => registry.register('core.utils', createMod('core.utils'))).toThrow(
      /reserved word/,
    );
  });

  it('register allows reserved word in middle segments', () => {
    const registry = new Registry();
    expect(() => registry.register('email.system', createMod('email.system'))).not.toThrow();
    expect(registry.has('email.system')).toBe(true);
    expect(() => registry.register('myapp.core.x', createMod('myapp.core.x'))).not.toThrow();
    expect(registry.has('myapp.core.x')).toBe(true);
  });

  // registerInternal — bypasses ONLY reserved word check (parity with python/rust)
  it('registerInternal accepts reserved first segment', () => {
    const registry = new Registry();
    expect(() => registry.registerInternal('system.health', createMod('system.health'))).not.toThrow();
    expect(registry.has('system.health')).toBe(true);
  });

  it('registerInternal accepts reserved word in non-first segment', () => {
    const registry = new Registry();
    expect(() => registry.registerInternal('myapp.system.config', createMod('myapp.system.config'))).not.toThrow();
  });

  it('registerInternal still rejects empty ID', () => {
    const registry = new Registry();
    expect(() => registry.registerInternal('', createMod('x'))).toThrow(
      /non-empty/,
    );
  });

  it('registerInternal still rejects invalid pattern', () => {
    const registry = new Registry();
    expect(() => registry.registerInternal('INVALID-ID', createMod('INVALID-ID'))).toThrow(
      /Invalid module ID|Must match pattern/,
    );
  });

  it('registerInternal still rejects over-length ID', () => {
    const registry = new Registry();
    const overlongId = 'a'.repeat(MAX_MODULE_ID_LENGTH + 1);
    expect(() => registry.registerInternal(overlongId, createMod(overlongId))).toThrow(
      /maximum length/,
    );
  });

  it('registerInternal rejects duplicate', () => {
    const registry = new Registry();
    registry.registerInternal('system.dup', createMod('system.dup'));
    expect(() => registry.registerInternal('system.dup', createMod('system.dup'))).toThrow(
      /already registered/,
    );
  });

  // D11-007: TS registerInternal previously rejected duplicates with a bare
  // `_modules.has` check, losing the case-collision detection that Python
  // (registry.py:1674) and Rust (registry.rs:727) enforce via
  // detect_id_conflicts on the same path. The lowercase-only EBNF pattern
  // makes case mismatches unreachable today, but the contract surface must
  // stay aligned across SDKs.
  it('registerInternal duplicate rejection emits "already registered" message via detectIdConflicts', () => {
    const registry = new Registry();
    registry.registerInternal('system.dup', createMod('system.dup'));
    // detectIdConflicts produces this exact message format; the bare
    // _modules.has fallback used a slightly different format. Pinning the
    // message keeps the conflict-detection routing observable.
    expect(() => registry.registerInternal('system.dup', createMod('system.dup'))).toThrow(
      "Module ID 'system.dup' is already registered",
    );
  });

  it('unregister removes module', () => {
    const registry = new Registry();
    registry.register('test.a', createMod('test.a'));
    const removed = registry.unregister('test.a');
    expect(removed).toBe(true);
    expect(registry.has('test.a')).toBe(false);
    expect(registry.count).toBe(0);
  });

  it('unregister returns false for unknown module', () => {
    const registry = new Registry();
    expect(registry.unregister('nonexistent')).toBe(false);
  });

  it('list returns sorted module IDs', () => {
    const registry = new Registry();
    registry.register('b.mod', createMod('b.mod'));
    registry.register('a.mod', createMod('a.mod'));
    registry.register('c.mod', createMod('c.mod'));
    expect(registry.list()).toEqual(['a.mod', 'b.mod', 'c.mod']);
  });

  it('list filters by prefix', () => {
    const registry = new Registry();
    registry.register('foo.a', createMod('foo.a'));
    registry.register('foo.b', createMod('foo.b'));
    registry.register('bar.a', createMod('bar.a'));
    expect(registry.list({ prefix: 'foo.' })).toEqual(['foo.a', 'foo.b']);
  });

  it('moduleIds returns sorted IDs', () => {
    const registry = new Registry();
    registry.register('z.mod', createMod('z.mod'));
    registry.register('a.mod', createMod('a.mod'));
    expect(registry.moduleIds).toEqual(['a.mod', 'z.mod']);
  });

  it('iter returns entries', () => {
    const registry = new Registry();
    registry.register('test.a', createMod('test.a'));
    const entries = [...registry.iter()];
    expect(entries).toHaveLength(1);
    expect(entries[0][0]).toBe('test.a');
  });

  it('on register event fires', () => {
    const registry = new Registry();
    const events: string[] = [];
    registry.on('register', (id) => events.push(id));
    registry.register('test.a', createMod('test.a'));
    expect(events).toEqual(['test.a']);
  });

  it('on unregister event fires', () => {
    const registry = new Registry();
    const events: string[] = [];
    registry.on('unregister', (id) => events.push(id));
    registry.register('test.a', createMod('test.a'));
    registry.unregister('test.a');
    expect(events).toEqual(['test.a']);
  });

  it('on throws for invalid event', () => {
    const registry = new Registry();
    expect(() => registry.on('invalid', () => {})).toThrow(InvalidInputError);
  });

  it('off removes a registered callback and returns true', () => {
    const registry = new Registry();
    const events: string[] = [];
    const cb = (id: string) => events.push(id);
    registry.on('register', cb);
    expect(registry.off('register', cb)).toBe(true);
    registry.register('test.a', createMod('test.a'));
    expect(events).toEqual([]);
  });

  it('off returns false when callback was not registered', () => {
    const registry = new Registry();
    expect(registry.off('register', () => {})).toBe(false);
  });

  it('off throws InvalidInputError for invalid event', () => {
    const registry = new Registry();
    expect(() => registry.off('invalid', () => {})).toThrow(InvalidInputError);
  });

  it('reload delegates to discover and returns module count', async () => {
    const registry = new Registry();
    registry.setDiscoverer({ discover: async () => [] });
    const count = await registry.reload();
    expect(count).toBe(0);
  });

  it('getDefinition returns descriptor', () => {
    const registry = new Registry();
    const mod = createMod('test.a');
    registry.register('test.a', mod);
    const def = registry.getDefinition('test.a');
    expect(def).not.toBeNull();
    expect(def!.moduleId).toBe('test.a');
    expect(def!.description).toBe('Module test.a');
  });

  it('getDefinition returns null for unknown module', () => {
    const registry = new Registry();
    expect(registry.getDefinition('nonexistent')).toBeNull();
  });

  // D10-011: Spec registry-system.md:382 says "Any error that get(module_id)
  // raises is propagated (e.g., ModuleNotFoundError on an empty string)".
  // Python (registry.py:1058) routes through get() so the empty-string guard
  // applies. TypeScript getDefinition was reading _modules directly and
  // returning null silently on empty input — divergent from spec + Python.
  it('getDefinition throws ModuleNotFoundError for empty moduleId', () => {
    const registry = new Registry();
    expect(() => registry.getDefinition('')).toThrow(ModuleNotFoundError);
  });

  it('getDefinition reads annotations from merged metadata for manually-registered modules', () => {
    // Pins the invariant that the manual register() path goes through
    // mergeModuleMetadata, so getDefinition's `meta`-side reads always
    // succeed. (Regression for code-forge:review warnings about a
    // dead `mod['annotations']` fallback in getDefinition that the
    // following cleanup commit removed.)
    const annotations = createAnnotations({ readonly: true, idempotent: true });
    const mod = new FunctionModule({
      execute: () => ({ ok: true }),
      moduleId: 'test.annotated',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ ok: Type.Boolean() }),
      description: 'annotated module',
      annotations,
    });
    const registry = new Registry();
    registry.register('test.annotated', mod);

    const def = registry.getDefinition('test.annotated');
    expect(def).not.toBeNull();
    expect(def!.annotations).not.toBeNull();
    expect(def!.annotations!.readonly).toBe(true);
    expect(def!.annotations!.idempotent).toBe(true);
    // destructive defaults to false; verify the merged result carries
    // the full ModuleAnnotations shape, not a partial dict.
    expect(def!.annotations!.destructive).toBe(false);
  });

  it('clearCache does not throw', () => {
    const registry = new Registry();
    registry.clearCache();
  });
});

/* -----------------------------------------------------------
 * Integration tests for Registry.discover() and related APIs
 * --------------------------------------------------------- */

/**
 * Helper: write a valid ESM module file (.js) that the scanner and
 * entry-point resolver can dynamically import.
 */
function writeModuleFile(
  dir: string,
  filename: string,
  content: string,
): string {
  const filePath = join(dir, filename);
  writeFileSync(filePath, content, 'utf-8');
  return filePath;
}

describe('Registry.discover()', () => {
  let tempDir: string;

  beforeEach(() => {
    tempDir = mkdtempSync(join(tmpdir(), 'apcore-registry-test-'));
  });

  afterEach(() => {
    rmSync(tempDir, { recursive: true, force: true });
  });

  it('discovers valid .js module files and registers them', async () => {
    writeModuleFile(
      tempDir,
      'greeter.js',
      `export default {
        execute: async (inputs) => ({ greeting: 'Hello ' + inputs.name }),
        description: 'A greeter module',
        inputSchema: { type: 'object', properties: { name: { type: 'string' } } },
        outputSchema: { type: 'object', properties: { greeting: { type: 'string' } } },
      };`,
    );

    const registry = new Registry({ extensionsDir: tempDir });
    const count = await registry.discover();

    expect(count).toBe(1);
    expect(registry.has('greeter')).toBe(true);
  });

  it('discovers multiple modules in nested directories', async () => {
    writeModuleFile(
      tempDir,
      'alpha.js',
      `export default {
        execute: async () => ({}),
        description: 'Alpha module',
        inputSchema: { type: 'object' },
        outputSchema: { type: 'object' },
      };`,
    );

    const subDir = join(tempDir, 'sub');
    mkdirSync(subDir, { recursive: true });
    writeModuleFile(
      subDir,
      'beta.js',
      `export default {
        execute: async () => ({}),
        description: 'Beta module',
        inputSchema: { type: 'object' },
        outputSchema: { type: 'object' },
      };`,
    );

    const registry = new Registry({ extensionsDir: tempDir });
    const count = await registry.discover();

    expect(count).toBe(2);
    expect(registry.has('alpha')).toBe(true);
    expect(registry.has('sub.beta')).toBe(true);
  });

  it('calls onLoad during discover when module exports onLoad', async () => {
    writeModuleFile(
      tempDir,
      'withload.js',
      `let loaded = false;
      export default {
        execute: async () => ({}),
        description: 'Module with onLoad',
        inputSchema: { type: 'object' },
        outputSchema: { type: 'object' },
        onLoad() { loaded = true; },
        isLoaded() { return loaded; },
      };`,
    );

    const registry = new Registry({ extensionsDir: tempDir });
    const count = await registry.discover();

    expect(count).toBe(1);
    expect(registry.has('withload')).toBe(true);

    const mod = registry.get('withload') as Record<string, unknown>;
    const isLoaded = (mod['isLoaded'] as () => boolean)();
    expect(isLoaded).toBe(true);
  });

  it('skips modules that fail validation (no execute method)', async () => {
    writeModuleFile(
      tempDir,
      'valid.js',
      `export default {
        execute: async () => ({}),
        description: 'Valid module',
        inputSchema: { type: 'object' },
        outputSchema: { type: 'object' },
      };`,
    );

    // Invalid module: no default export that passes isModuleClass
    writeModuleFile(
      tempDir,
      'invalid.js',
      `export const someData = 42;
      export const description = 'Invalid module - no execute';`,
    );

    const registry = new Registry({ extensionsDir: tempDir });
    const count = await registry.discover();

    expect(count).toBe(1);
    expect(registry.has('valid')).toBe(true);
    expect(registry.has('invalid')).toBe(false);
  });

  it('merges companion _meta.yaml metadata into discovered module', async () => {
    writeModuleFile(
      tempDir,
      'tagged.js',
      `export default {
        execute: async () => ({}),
        description: 'Tagged module from code',
        inputSchema: { type: 'object' },
        outputSchema: { type: 'object' },
      };`,
    );

    writeFileSync(
      join(tempDir, 'tagged_meta.yaml'),
      [
        'description: "Overridden description from YAML"',
        'version: "2.0.0"',
        'tags:',
        '  - yaml_tag',
        '  - production',
      ].join('\n'),
      'utf-8',
    );

    const registry = new Registry({ extensionsDir: tempDir });
    const count = await registry.discover();

    expect(count).toBe(1);
    expect(registry.has('tagged')).toBe(true);

    const def = registry.getDefinition('tagged');
    expect(def).not.toBeNull();
    expect(def!.description).toBe('Overridden description from YAML');
    expect(def!.version).toBe('2.0.0');
    expect(def!.tags).toEqual(['yaml_tag', 'production']);
  });

  it('returns 0 when extensions directory contains no valid modules', async () => {
    // Write a file that is not a module (plain text)
    writeFileSync(join(tempDir, 'readme.txt'), 'Not a module', 'utf-8');

    const registry = new Registry({ extensionsDir: tempDir });
    const count = await registry.discover();

    expect(count).toBe(0);
    expect(registry.count).toBe(0);
  });
});

describe('Registry.getDefinition() with discover', () => {
  let tempDir: string;

  beforeEach(() => {
    tempDir = mkdtempSync(join(tmpdir(), 'apcore-registry-def-'));
  });

  afterEach(() => {
    rmSync(tempDir, { recursive: true, force: true });
  });

  it('returns full ModuleDescriptor for a discovered module', async () => {
    writeModuleFile(
      tempDir,
      'detailed.js',
      `export default {
        execute: async (inputs) => ({ result: inputs.x * 2 }),
        description: 'A detailed test module',
        version: '3.5.0',
        tags: ['math', 'utility'],
        inputSchema: { type: 'object', properties: { x: { type: 'number' } } },
        outputSchema: { type: 'object', properties: { result: { type: 'number' } } },
      };`,
    );

    const registry = new Registry({ extensionsDir: tempDir });
    await registry.discover();

    const def = registry.getDefinition('detailed');
    expect(def).not.toBeNull();
    expect(def!.moduleId).toBe('detailed');
    expect(def!.description).toBe('A detailed test module');
    expect(def!.version).toBe('3.5.0');
    expect(def!.tags).toEqual(['math', 'utility']);
    expect(def!.inputSchema).toEqual({
      type: 'object',
      properties: { x: { type: 'number' } },
    });
    expect(def!.outputSchema).toEqual({
      type: 'object',
      properties: { result: { type: 'number' } },
    });
  });

  it('returns null for a module ID that was not discovered', async () => {
    const registry = new Registry({ extensionsDir: tempDir });
    await registry.discover();

    expect(registry.getDefinition('nonexistent')).toBeNull();
  });
});

describe('Registry.list() with tag filtering', () => {
  it('filters modules by tags on registered plain objects', () => {
    const registry = new Registry();

    const modA = {
      execute: async () => ({}),
      description: 'Module A',
      inputSchema: { type: 'object' },
      outputSchema: { type: 'object' },
      tags: ['web', 'api'],
    };
    const modB = {
      execute: async () => ({}),
      description: 'Module B',
      inputSchema: { type: 'object' },
      outputSchema: { type: 'object' },
      tags: ['cli', 'api'],
    };
    const modC = {
      execute: async () => ({}),
      description: 'Module C',
      inputSchema: { type: 'object' },
      outputSchema: { type: 'object' },
      tags: ['web'],
    };

    registry.register('mod.a', modA);
    registry.register('mod.b', modB);
    registry.register('mod.c', modC);

    expect(registry.list({ tags: ['api'] })).toEqual(['mod.a', 'mod.b']);
    expect(registry.list({ tags: ['web'] })).toEqual(['mod.a', 'mod.c']);
    expect(registry.list({ tags: ['cli'] })).toEqual(['mod.b']);
    expect(registry.list({ tags: ['web', 'api'] })).toEqual(['mod.a']);
  });

  it('returns empty array when no modules match the tag', () => {
    const registry = new Registry();

    const modA = {
      execute: async () => ({}),
      description: 'Module A',
      inputSchema: { type: 'object' },
      outputSchema: { type: 'object' },
      tags: ['web'],
    };
    registry.register('mod.a', modA);

    expect(registry.list({ tags: ['nonexistent'] })).toEqual([]);
  });

  it('combines tag and prefix filtering', () => {
    const registry = new Registry();

    const modA = {
      execute: async () => ({}),
      description: 'Module A',
      inputSchema: { type: 'object' },
      outputSchema: { type: 'object' },
      tags: ['api'],
    };
    const modB = {
      execute: async () => ({}),
      description: 'Module B',
      inputSchema: { type: 'object' },
      outputSchema: { type: 'object' },
      tags: ['api'],
    };

    registry.register('svc.alpha', modA);
    registry.register('lib.beta', modB);

    expect(registry.list({ prefix: 'svc.', tags: ['api'] })).toEqual(['svc.alpha']);
    expect(registry.list({ prefix: 'lib.', tags: ['api'] })).toEqual(['lib.beta']);
    expect(registry.list({ prefix: 'unknown.', tags: ['api'] })).toEqual([]);
  });

  let tempDir: string;

  beforeEach(() => {
    tempDir = mkdtempSync(join(tmpdir(), 'apcore-registry-tags-'));
  });

  afterEach(() => {
    rmSync(tempDir, { recursive: true, force: true });
  });

  it('filters discovered modules by tags from code exports', async () => {
    writeModuleFile(
      tempDir,
      'svcone.js',
      `export default {
        execute: async () => ({}),
        description: 'Service one',
        tags: ['backend', 'grpc'],
        inputSchema: { type: 'object' },
        outputSchema: { type: 'object' },
      };`,
    );

    writeModuleFile(
      tempDir,
      'svctwo.js',
      `export default {
        execute: async () => ({}),
        description: 'Service two',
        tags: ['frontend', 'rest'],
        inputSchema: { type: 'object' },
        outputSchema: { type: 'object' },
      };`,
    );

    const registry = new Registry({ extensionsDir: tempDir });
    await registry.discover();

    expect(registry.list({ tags: ['backend'] })).toEqual(['svcone']);
    expect(registry.list({ tags: ['frontend'] })).toEqual(['svctwo']);
    expect(registry.list({ tags: ['grpc'] })).toEqual(['svcone']);
    expect(registry.list({ tags: ['rest'] })).toEqual(['svctwo']);
  });

  it('filters discovered modules by tags from companion YAML metadata', async () => {
    writeModuleFile(
      tempDir,
      'yamlmod.js',
      `export default {
        execute: async () => ({}),
        description: 'YAML tagged module',
        inputSchema: { type: 'object' },
        outputSchema: { type: 'object' },
      };`,
    );

    writeFileSync(
      join(tempDir, 'yamlmod_meta.yaml'),
      ['tags:', '  - infra', '  - deploy'].join('\n'),
      'utf-8',
    );

    const registry = new Registry({ extensionsDir: tempDir });
    await registry.discover();

    expect(registry.list({ tags: ['infra'] })).toEqual(['yamlmod']);
    expect(registry.list({ tags: ['deploy'] })).toEqual(['yamlmod']);
    expect(registry.list({ tags: ['web'] })).toEqual([]);
  });
});

/* -----------------------------------------------------------
 * Constructor branch coverage
 * --------------------------------------------------------- */

describe('Registry constructor branches', () => {
  it('accepts extensionsDirs with string entries', () => {
    const registry = new Registry({ extensionsDirs: ['/tmp/ext-a', '/tmp/ext-b'] });
    expect(registry.count).toBe(0);
  });

  it('accepts extensionsDirs with object entries', () => {
    const registry = new Registry({
      extensionsDirs: [{ root: '/tmp/ext-a', namespace: 'ns' }, '/tmp/ext-b'],
    });
    expect(registry.count).toBe(0);
  });

  it('throws when both extensionsDir and extensionsDirs are provided', () => {
    expect(
      () => new Registry({ extensionsDir: '/tmp/ext-a', extensionsDirs: ['/tmp/ext-b'] }),
    ).toThrow(InvalidInputError);
  });

  it('uses extensions.root from config when no extensionsDir is provided', () => {
    const config = new Config({ extensions: { root: '/tmp/from-config' } });
    const registry = new Registry({ config });
    expect(registry.count).toBe(0);
  });

  it('falls back to ./extensions when config has no extensions.root key', () => {
    const config = new Config({});
    const registry = new Registry({ config });
    expect(registry.count).toBe(0);
  });

  it('falls back to ./extensions when no options are provided', () => {
    const registry = new Registry();
    expect(registry.count).toBe(0);
  });
});

/* -----------------------------------------------------------
 * register() with onLoad callback
 * --------------------------------------------------------- */

describe('Registry register() onLoad callback', () => {
  it('calls onLoad when module has an onLoad function', () => {
    const registry = new Registry();
    let loaded = false;
    const mod = {
      execute: async () => ({}),
      description: 'Module with onLoad',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      onLoad() {
        loaded = true;
      },
    };
    registry.register('with.load', mod);
    expect(loaded).toBe(true);
    expect(registry.has('with.load')).toBe(true);
  });

  it('re-deletes module and rejects when onLoad throws', async () => {
    // v0.22.0 A-D-REG-001: register() always returns a Promise; sync onLoad
    // failures now surface as a rejection (and emit
    // apcore.registry.module_load_failed) so the strong-guarantee invariant
    // holds uniformly across sync and async onLoad.
    const registry = new Registry();
    const loadError = new Error('onLoad failed');
    const mod = {
      execute: async () => ({}),
      description: 'Failing onLoad module',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      onLoad() {
        throw loadError;
      },
    };
    await expect(registry.register('bad.load', mod)).rejects.toBe(loadError);
    expect(registry.has('bad.load')).toBe(false);
    expect(registry.count).toBe(0);
  });
});

/* -----------------------------------------------------------
 * unregister() with onUnload callback
 * --------------------------------------------------------- */

describe('Registry unregister() onUnload callback', () => {
  it('calls onUnload when module has an onUnload function', () => {
    const registry = new Registry();
    let unloaded = false;
    const mod = {
      execute: async () => ({}),
      description: 'Module with onUnload',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      onUnload() {
        unloaded = true;
      },
    };
    registry.register('with.unload', mod);
    const result = registry.unregister('with.unload');
    expect(result).toBe(true);
    expect(unloaded).toBe(true);
    expect(registry.has('with.unload')).toBe(false);
  });

  it('still unregisters and warns when onUnload throws', () => {
    const registry = new Registry();
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const unloadError = new Error('onUnload failed');
    const mod = {
      execute: async () => ({}),
      description: 'Module with failing onUnload',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      onUnload() {
        throw unloadError;
      },
    };
    registry.register('bad.unload', mod);
    const result = registry.unregister('bad.unload');
    expect(result).toBe(true);
    expect(registry.has('bad.unload')).toBe(false);
    expect(warnSpy).toHaveBeenCalledWith(
      expect.stringContaining('[apcore:registry]'),
      unloadError,
    );
    warnSpy.mockRestore();
  });
});

/* -----------------------------------------------------------
 * unregister() clears hot-reload/drain state (A-D-001)
 * --------------------------------------------------------- */

describe('Registry unregister() clears drain/refcount state (A-D-001)', () => {
  it('drops stale _draining entry so re-registering the same id can be acquired', () => {
    const registry = new Registry();
    registry.register('drain.me', createMod('drain.me'));

    // Mark the module draining (simulating an in-flight safe_unregister).
    registry.beginDrain('drain.me');
    expect(registry.isDraining('drain.me')).toBe(true);

    // Direct unregister() (not via safe_unregister) must clear drain state.
    expect(registry.unregister('drain.me')).toBe(true);
    expect(registry.isDraining('drain.me')).toBe(false);

    // Re-register the SAME id — acquire() must succeed (no stale draining flag).
    registry.register('drain.me', createMod('drain.me'));
    expect(() => registry.acquire('drain.me')).not.toThrow();
    expect(registry.acquire('drain.me')).not.toBeNull();
  });

  it('drops stale _refCounts entry so the re-registered module starts at zero', () => {
    const registry = new Registry();
    registry.register('ref.me', createMod('ref.me'));

    // Bump the reference count via acquire().
    registry.acquire('ref.me');
    expect(registry.refCount.get('ref.me')).toBe(1);

    // Direct unregister() must clear the ref count.
    expect(registry.unregister('ref.me')).toBe(true);
    expect(registry.refCount.has('ref.me')).toBe(false);

    // Re-register the same id — its ref count must start fresh at zero.
    registry.register('ref.me', createMod('ref.me'));
    expect(registry.refCount.has('ref.me')).toBe(false);
    registry.acquire('ref.me');
    expect(registry.refCount.get('ref.me')).toBe(1);
  });
});

/* -----------------------------------------------------------
 * _triggerEvent error handling
 * --------------------------------------------------------- */

describe('Registry _triggerEvent error handling', () => {
  it('warns and continues when a registered event callback throws', () => {
    const registry = new Registry();
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const callbackError = new Error('callback exploded');

    registry.on('register', () => {
      throw callbackError;
    });

    // register should complete normally despite the callback throwing
    expect(() => registry.register('trigger.test', createMod('trigger.test'))).not.toThrow();
    expect(registry.has('trigger.test')).toBe(true);
    expect(warnSpy).toHaveBeenCalledWith(
      expect.stringContaining('[apcore:registry]'),
      callbackError,
    );
    warnSpy.mockRestore();
  });

  it('warns and continues for unregister event callbacks that throw', () => {
    const registry = new Registry();
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});

    registry.register('trigger.unreg', createMod('trigger.unreg'));

    const callbackError = new Error('unregister callback exploded');
    registry.on('unregister', () => {
      throw callbackError;
    });

    const result = registry.unregister('trigger.unreg');
    expect(result).toBe(true);
    expect(registry.has('trigger.unreg')).toBe(false);
    expect(warnSpy).toHaveBeenCalledWith(
      expect.stringContaining('[apcore:registry]'),
      callbackError,
    );
    warnSpy.mockRestore();
  });
});

/* -----------------------------------------------------------
 * list() with metaTags from _moduleMeta
 * --------------------------------------------------------- */

/* -----------------------------------------------------------
 * register() invalid pattern (MODULE_ID_PATTERN check)
 * --------------------------------------------------------- */

describe('Registry register() invalid module ID pattern', () => {
  it('throws InvalidInputError when moduleId contains a hyphen', () => {
    const registry = new Registry();
    expect(() => registry.register('bad-id', createMod('test.a'))).toThrow(InvalidInputError);
  });

  it('throws InvalidInputError when moduleId starts with a digit', () => {
    const registry = new Registry();
    expect(() => registry.register('1invalid', createMod('test.a'))).toThrow(InvalidInputError);
  });

  it('throws InvalidInputError when moduleId contains uppercase letters', () => {
    const registry = new Registry();
    expect(() => registry.register('Bad.Id', createMod('test.a'))).toThrow(InvalidInputError);
  });
});

/* -----------------------------------------------------------
 * discover() with config: _scanRoots uses config values
 * --------------------------------------------------------- */

describe('Registry discover() with Config', () => {
  let tempDir: string;

  beforeEach(() => {
    tempDir = mkdtempSync(join(tmpdir(), 'apcore-registry-config-'));
  });

  afterEach(() => {
    rmSync(tempDir, { recursive: true, force: true });
  });

  it('uses extensions.root from config and reads max_depth and follow_symlinks during discover()', async () => {
    writeFileSync(
      join(tempDir, 'cfgmod.js'),
      `export default {
        execute: async () => ({}),
        description: 'Config-driven module',
        inputSchema: { type: 'object' },
        outputSchema: { type: 'object' },
      };`,
      'utf-8',
    );

    const config = new Config({
      extensions: { root: tempDir, max_depth: 3, follow_symlinks: false },
    });
    const registry = new Registry({ config });
    const count = await registry.discover();

    expect(count).toBe(1);
    expect(registry.has('cfgmod')).toBe(true);
  });
});

/* -----------------------------------------------------------
 * discover() onLoad failure in _registerInOrder
 * --------------------------------------------------------- */

describe('Registry discover() onLoad failure during _registerInOrder', () => {
  let tempDir: string;

  beforeEach(() => {
    tempDir = mkdtempSync(join(tmpdir(), 'apcore-registry-onloadfail-'));
  });

  afterEach(() => {
    rmSync(tempDir, { recursive: true, force: true });
  });

  it('skips module and warns when onLoad throws during discover()', async () => {
    writeFileSync(
      join(tempDir, 'failload.js'),
      `export default {
        execute: async () => ({}),
        description: 'Module with failing onLoad',
        inputSchema: { type: 'object' },
        outputSchema: { type: 'object' },
        onLoad() { throw new Error('onLoad exploded'); },
      };`,
      'utf-8',
    );

    writeFileSync(
      join(tempDir, 'goodmod.js'),
      `export default {
        execute: async () => ({}),
        description: 'Good module',
        inputSchema: { type: 'object' },
        outputSchema: { type: 'object' },
      };`,
      'utf-8',
    );

    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const registry = new Registry({ extensionsDir: tempDir });
    const count = await registry.discover();

    // Only the good module should be registered; failload is skipped
    expect(count).toBe(1);
    expect(registry.has('goodmod')).toBe(true);
    expect(registry.has('failload')).toBe(false);
    expect(warnSpy).toHaveBeenCalledWith(
      expect.stringContaining('[apcore:registry]'),
      expect.any(Error),
    );
    warnSpy.mockRestore();
  });
});

/* -----------------------------------------------------------
 * discover() deferred-publish contract (audit D11-001 / D10-005)
 *
 * The discover path MUST mirror the public register() deferred-publish
 * contract: a module is NEVER published to the registry until its onLoad
 * (sync or async) has succeeded, and any onLoad failure (sync throw OR
 * async rejection) emits `apcore.registry.module_load_failed` instead of
 * publishing the module.
 * --------------------------------------------------------- */

describe('Registry discover() deferred-publish + module_load_failed (D11-001/D10-005)', () => {
  let tempDir: string;

  beforeEach(() => {
    tempDir = mkdtempSync(join(tmpdir(), 'apcore-registry-deferpub-'));
  });

  afterEach(() => {
    rmSync(tempDir, { recursive: true, force: true });
  });

  it('does not publish module and emits module_load_failed when SYNC onLoad throws during discover()', async () => {
    writeFileSync(
      join(tempDir, 'syncfail.js'),
      `export default {
        execute: async () => ({}),
        description: 'Sync onLoad failure',
        inputSchema: { type: 'object' },
        outputSchema: { type: 'object' },
        onLoad() { throw new Error('sync onLoad exploded'); },
      };`,
      'utf-8',
    );
    writeFileSync(
      join(tempDir, 'goodsync.js'),
      `export default {
        execute: async () => ({}),
        description: 'Good module',
        inputSchema: { type: 'object' },
        outputSchema: { type: 'object' },
      };`,
      'utf-8',
    );

    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const emitter = new EventEmitter();
    const events: ApCoreEvent[] = [];
    emitter.subscribe({ onEvent: (e) => { events.push(e); } });

    const registry = new Registry({ extensionsDir: tempDir });
    registry.setEventEmitter(emitter);

    const count = await registry.discover();

    // Failing module is NOT visible; good module still registers.
    expect(registry.has('syncfail')).toBe(false);
    expect(registry.get('syncfail')).toBeNull();
    expect(registry.has('goodsync')).toBe(true);
    expect(count).toBe(1);

    // module_load_failed emitted for the failing module.
    const failEvents = events.filter(
      (e) => e.eventType === 'apcore.registry.module_load_failed' && e.moduleId === 'syncfail',
    );
    expect(failEvents).toHaveLength(1);

    warnSpy.mockRestore();
  });

  it('does not publish module and emits module_load_failed when ASYNC onLoad rejects during discover()', async () => {
    writeFileSync(
      join(tempDir, 'asyncfail.js'),
      `export default {
        execute: async () => ({}),
        description: 'Async onLoad rejection',
        inputSchema: { type: 'object' },
        outputSchema: { type: 'object' },
        async onLoad() { throw new Error('async onLoad rejected'); },
      };`,
      'utf-8',
    );
    writeFileSync(
      join(tempDir, 'goodasync.js'),
      `export default {
        execute: async () => ({}),
        description: 'Good module',
        inputSchema: { type: 'object' },
        outputSchema: { type: 'object' },
      };`,
      'utf-8',
    );

    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const emitter = new EventEmitter();
    const events: ApCoreEvent[] = [];
    emitter.subscribe({ onEvent: (e) => { events.push(e); } });

    const registry = new Registry({ extensionsDir: tempDir });
    registry.setEventEmitter(emitter);

    // Must resolve without an unhandled promise rejection.
    const count = await registry.discover();

    expect(registry.has('asyncfail')).toBe(false);
    expect(registry.get('asyncfail')).toBeNull();
    expect(registry.has('goodasync')).toBe(true);
    expect(count).toBe(1);

    const failEvents = events.filter(
      (e) => e.eventType === 'apcore.registry.module_load_failed' && e.moduleId === 'asyncfail',
    );
    expect(failEvents).toHaveLength(1);

    warnSpy.mockRestore();
  });

  it('publishes module when ASYNC onLoad resolves during discover()', async () => {
    writeFileSync(
      join(tempDir, 'asyncok.js'),
      `export default {
        execute: async () => ({}),
        description: 'Async onLoad success',
        inputSchema: { type: 'object' },
        outputSchema: { type: 'object' },
        async onLoad() { return; },
      };`,
      'utf-8',
    );

    const emitter = new EventEmitter();
    const events: ApCoreEvent[] = [];
    emitter.subscribe({ onEvent: (e) => { events.push(e); } });

    const registry = new Registry({ extensionsDir: tempDir });
    registry.setEventEmitter(emitter);

    const count = await registry.discover();

    expect(registry.has('asyncok')).toBe(true);
    expect(registry.get('asyncok')).not.toBeNull();
    expect(count).toBe(1);

    const failEvents = events.filter(
      (e) => e.eventType === 'apcore.registry.module_load_failed',
    );
    expect(failEvents).toHaveLength(0);
  });

  it('never makes a failing-onLoad module observable after discover() resolves (strong guarantee)', async () => {
    writeFileSync(
      join(tempDir, 'neverpub.js'),
      `export default {
        execute: async () => ({}),
        description: 'Failing onLoad, must never be observable',
        inputSchema: { type: 'object' },
        outputSchema: { type: 'object' },
        onLoad() { throw new Error('boom'); },
      };`,
      'utf-8',
    );

    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const registry = new Registry({ extensionsDir: tempDir });

    await registry.discover();

    expect(registry.has('neverpub')).toBe(false);
    expect(registry.get('neverpub')).toBeNull();
    expect(registry.list()).not.toContain('neverpub');
    expect(registry.count).toBe(0);

    warnSpy.mockRestore();
  });
});

/* -----------------------------------------------------------
 * discover() with extensionsDirs (multi-root / scanMultiRoot path)
 * --------------------------------------------------------- */

describe('Registry discover() with extensionsDirs (multi-root)', () => {
  let parentDir: string;
  let tempDirA: string;
  let tempDirB: string;

  beforeEach(() => {
    // Use deterministic subdir names so the namespace derived from the dir
    // basename satisfies PROTOCOL_SPEC §2.7 (lowercase, no hyphens, no
    // reserved first segment) — required after sync findings A-D-101 /
    // A-D-102 added validation in the discover paths.
    parentDir = mkdtempSync(join(tmpdir(), 'multiroot_'));
    tempDirA = join(parentDir, 'roota');
    tempDirB = join(parentDir, 'rootb');
    mkdirSync(tempDirA);
    mkdirSync(tempDirB);
  });

  afterEach(() => {
    rmSync(parentDir, { recursive: true, force: true });
  });

  it('discovers modules across multiple extension directories', async () => {
    writeFileSync(
      join(tempDirA, 'alpha.js'),
      `export default {
        execute: async () => ({}),
        description: 'Alpha module',
        inputSchema: { type: 'object' },
        outputSchema: { type: 'object' },
      };`,
      'utf-8',
    );

    writeFileSync(
      join(tempDirB, 'beta.js'),
      `export default {
        execute: async () => ({}),
        description: 'Beta module',
        inputSchema: { type: 'object' },
        outputSchema: { type: 'object' },
      };`,
      'utf-8',
    );

    // When using extensionsDirs with multiple roots, scanMultiRoot prefixes
    // each module ID with the namespace (basename of the root dir by default).
    const registry = new Registry({ extensionsDirs: [tempDirA, tempDirB] });
    const count = await registry.discover();

    expect(count).toBe(2);
    expect(registry.count).toBe(2);
  });

  it('discovers modules from an extensionsDirs object entry with namespace', async () => {
    writeFileSync(
      join(tempDirA, 'nsmod.js'),
      `export default {
        execute: async () => ({}),
        description: 'Namespaced module',
        inputSchema: { type: 'object' },
        outputSchema: { type: 'object' },
      };`,
      'utf-8',
    );

    const registry = new Registry({
      extensionsDirs: [{ root: tempDirA, namespace: 'myns' }],
    });
    const count = await registry.discover();

    expect(count).toBe(1);
  });
});

/* -----------------------------------------------------------
 * discover() with idMapPath (_applyIdMapOverrides path)
 * --------------------------------------------------------- */

describe('Registry discover() with idMapPath', () => {
  let tempDir: string;

  beforeEach(() => {
    tempDir = mkdtempSync(join(tmpdir(), 'apcore-registry-idmap-'));
  });

  afterEach(() => {
    rmSync(tempDir, { recursive: true, force: true });
  });

  it('applies ID map overrides to discovered modules', async () => {
    writeFileSync(
      join(tempDir, 'mymod.js'),
      `export default {
        execute: async () => ({}),
        description: 'ID map overridden module',
        inputSchema: { type: 'object' },
        outputSchema: { type: 'object' },
      };`,
      'utf-8',
    );

    const idMapPath = join(tempDir, 'idmap.yaml');
    writeFileSync(
      idMapPath,
      [
        'mappings:',
        '  - file: mymod.js',
        '    id: custom.mapped.id',
      ].join('\n'),
      'utf-8',
    );

    const registry = new Registry({ extensionsDir: tempDir, idMapPath });
    const count = await registry.discover();

    expect(count).toBe(1);
    expect(registry.has('custom.mapped.id')).toBe(true);
    expect(registry.has('mymod')).toBe(false);
  });

  it('discovers normally when ID map has no matching entry for a file', async () => {
    writeFileSync(
      join(tempDir, 'unmapped.js'),
      `export default {
        execute: async () => ({}),
        description: 'Unmapped module',
        inputSchema: { type: 'object' },
        outputSchema: { type: 'object' },
      };`,
      'utf-8',
    );

    const idMapPath = join(tempDir, 'idmap.yaml');
    writeFileSync(
      idMapPath,
      ['mappings:', '  - file: other.js', '    id: other.id'].join('\n'),
      'utf-8',
    );

    const registry = new Registry({ extensionsDir: tempDir, idMapPath });
    const count = await registry.discover();

    expect(count).toBe(1);
    expect(registry.has('unmapped')).toBe(true);
  });
});

/* -----------------------------------------------------------
 * list() metaTags from companion metadata
 * --------------------------------------------------------- */

describe('Registry list() metaTags from companion metadata', () => {
  let tempDir: string;

  beforeEach(() => {
    tempDir = mkdtempSync(join(tmpdir(), 'apcore-registry-metatags-'));
  });

  afterEach(() => {
    rmSync(tempDir, { recursive: true, force: true });
  });

  it('filters using tags stored in _moduleMeta when module object has no tags', async () => {
    writeFileSync(
      join(tempDir, 'notagmod.js'),
      `export default {
        execute: async () => ({}),
        description: 'No tags on module object',
        inputSchema: { type: 'object' },
        outputSchema: { type: 'object' },
      };`,
      'utf-8',
    );

    writeFileSync(
      join(tempDir, 'notagmod_meta.yaml'),
      ['tags:', '  - alpha', '  - beta'].join('\n'),
      'utf-8',
    );

    const registry = new Registry({ extensionsDir: tempDir });
    await registry.discover();

    expect(registry.list({ tags: ['alpha'] })).toEqual(['notagmod']);
    expect(registry.list({ tags: ['beta'] })).toEqual(['notagmod']);
    expect(registry.list({ tags: ['gamma'] })).toEqual([]);
  });
});

/* -----------------------------------------------------------
 * describe() tests
 * --------------------------------------------------------- */

describe('Registry.describe()', () => {
  it('calls custom describe() method when module has one', () => {
    const registry = new Registry();
    const mod = {
      execute: async () => ({}),
      description: 'Module with custom describe',
      inputSchema: { type: 'object' },
      outputSchema: { type: 'object' },
      describe() {
        return 'Custom description from the module itself.';
      },
    };
    registry.register('test.custom', mod);
    expect(registry.describe('test.custom')).toBe('Custom description from the module itself.');
  });

  it('auto-generates markdown when no custom describe method', () => {
    const registry = new Registry();
    const mod = {
      execute: async () => ({}),
      description: 'A valid test module',
      inputSchema: {
        type: 'object',
        properties: { value: { type: 'string', description: 'Input value' } },
        required: ['value'],
      },
      outputSchema: { type: 'object' },
      tags: ['test', 'sample'],
    };
    registry.register('test.auto', mod);
    const result = registry.describe('test.auto');
    expect(result).toContain('# test.auto');
    expect(result).toContain('A valid test module');
    expect(result).toContain('**Tags:** test, sample');
    expect(result).toContain('**Parameters:**');
    expect(result).toContain('`value`');
    expect(result).toContain('(required)');
  });

  it('includes documentation section when available', () => {
    const registry = new Registry();
    const mod = {
      execute: async () => ({}),
      description: 'A documented module',
      documentation: 'This module does interesting things.',
      inputSchema: { type: 'object' },
      outputSchema: { type: 'object' },
    };
    registry.register('test.documented', mod);
    const result = registry.describe('test.documented');
    expect(result).toContain('**Documentation:**');
    expect(result).toContain('This module does interesting things.');
  });

  it('throws ModuleNotFoundError for unregistered module', () => {
    const registry = new Registry();
    expect(() => registry.describe('nonexistent.module')).toThrow(ModuleNotFoundError);
  });

  /* ---------------------------------------------------------
   * D-77 (spec v1.49.0) — `describe()` is typed `: string` and must
   * never hand back anything else. `Module.describe()` is declared as
   * returning a ModuleDescription MAPPING (optionally a Promise of
   * one), so passing the override through verbatim made
   * `registry.describe(id).split('\n')` a TypeError.
   * --------------------------------------------------------- */

  it('D-77: falls through to the generated envelope for a structured describe()', () => {
    const registry = new Registry();
    const mod = {
      execute: async () => ({}),
      description: 'Module returning an introspection mapping',
      inputSchema: {
        type: 'object',
        properties: { value: { type: 'string', description: 'Input value' } },
        required: ['value'],
      },
      outputSchema: { type: 'object' },
      // The shape module-interface.md actually declares for describe().
      describe() {
        return {
          description: 'Module returning an introspection mapping',
          input_schema: { type: 'object' },
          output_schema: { type: 'object' },
          annotations: {},
        };
      },
    };
    registry.register('test.structured', mod);

    const result = registry.describe('test.structured');
    expect(typeof result).toBe('string');
    // The generated envelope, not a stringified mapping: str(dict) is a
    // language-specific repr, not a description.
    expect(result).toContain('# test.structured');
    expect(result).toContain('**Parameters:**');
    expect(result).not.toContain('[object Object]');
    expect(result).not.toContain('input_schema');
    // The caller's line that used to blow up.
    expect(() => result.split('\n')).not.toThrow();
  });

  it('D-77: falls through to the generated envelope for an async describe()', () => {
    const registry = new Registry();
    const mod = {
      execute: async () => ({}),
      description: 'Module with an async describe',
      inputSchema: { type: 'object' },
      outputSchema: { type: 'object' },
      async describe() {
        return { description: 'from a promise' };
      },
    };
    registry.register('test.async_describe', mod);

    const result = registry.describe('test.async_describe');
    expect(typeof result).toBe('string');
    expect(result).toContain('# test.async_describe');
    expect(result).toContain('Module with an async describe');
  });

  it('D-77: falls through to the generated envelope for a null describe()', () => {
    const registry = new Registry();
    const mod = {
      execute: async () => ({}),
      description: 'Module whose describe opts out',
      inputSchema: { type: 'object' },
      outputSchema: { type: 'object' },
      describe() {
        return null;
      },
    };
    registry.register('test.null_describe', mod);

    const result = registry.describe('test.null_describe');
    expect(typeof result).toBe('string');
    expect(result).toContain('# test.null_describe');
  });

  it('D-77: a string override is still returned verbatim', () => {
    // Rule 1: a genuine string IS the author's deliberate override.
    const registry = new Registry();
    const mod = {
      execute: async () => ({}),
      description: 'Module with a string describe',
      inputSchema: { type: 'object' },
      outputSchema: { type: 'object' },
      describe() {
        return 'Hand-written description.\nSecond line.';
      },
    };
    registry.register('test.string_describe', mod);
    expect(registry.describe('test.string_describe')).toBe(
      'Hand-written description.\nSecond line.',
    );
  });

  it('D-77: describe() always returns a string, for every override shape', () => {
    const registry = new Registry();
    const shapes: Array<[string, unknown]> = [
      ['test.shape_string', () => 'a string'],
      ['test.shape_object', () => ({ description: 'an object' })],
      ['test.shape_promise', async () => ({ description: 'a promise' })],
      ['test.shape_null', () => null],
      ['test.shape_number', () => 42],
      ['test.shape_array', () => ['a', 'b']],
    ];
    for (const [id, describeFn] of shapes) {
      registry.register(id, {
        execute: async () => ({}),
        description: `Module ${id}`,
        inputSchema: { type: 'object' },
        outputSchema: { type: 'object' },
        describe: describeFn,
      });
      expect(typeof registry.describe(id)).toBe('string');
    }
  });
});

/* -----------------------------------------------------------
 * Hot Reload (watch/unwatch)
 * --------------------------------------------------------- */

describe('Registry hot reload (watch/unwatch)', () => {
  let tempDir: string;

  beforeEach(() => {
    tempDir = mkdtempSync(join(tmpdir(), 'apcore-registry-hotreload-'));
  });

  afterEach(() => {
    rmSync(tempDir, { recursive: true, force: true });
  });

  it('watch() does not throw when called with a valid directory', () => {
    const registry = new Registry({ extensionsDir: tempDir });
    expect(() => registry.watch()).not.toThrow();
    registry.unwatch();
  });

  /* ---------------------------------------------------------
   * D-80 (spec v1.49.0) — `file_changed` is part of the closed
   * registry event set, because this SDK's notify-only `watch()`
   * emits it. Before the fix `on('file_changed', ...)` threw
   * InvalidInputError, so the callback list was permanently empty
   * and EVERY hot-reload notification was silently discarded.
   * --------------------------------------------------------- */

  it("D-80: on() accepts 'file_changed' — an SDK must accept every event it can emit", () => {
    const registry = new Registry();
    expect(() => registry.on('file_changed', () => {})).not.toThrow();
  });

  it("D-80: off() accepts 'file_changed' and removes the subscription", () => {
    const registry = new Registry();
    const cb = (): void => {};
    registry.on('file_changed', cb);
    expect(registry.off('file_changed', cb)).toBe(true);
    // Removing it twice reports "was not subscribed", not an invalid event.
    expect(registry.off('file_changed', cb)).toBe(false);
  });

  it('D-80: the event set stays closed — an unknown name is still rejected', () => {
    const registry = new Registry();
    // registry-system.md's own example used to tell readers to call these.
    for (const bogus of ['change', 'add', 'remove', 'fille_changed']) {
      expect(() => registry.on(bogus, () => {})).toThrow(InvalidInputError);
      expect(() => registry.off(bogus, () => {})).toThrow(InvalidInputError);
    }
  });

  it('D-80: a watched file change reaches a callback subscribed through on()', async () => {
    // End-to-end over a real fs.watch: subscribe via the public API, touch a
    // watched file, and require the notification to arrive. Without the fix
    // the `on()` call itself throws, so this test cannot even reach the touch.
    const registry = new Registry({ extensionsDir: tempDir });
    const watchedFile = join(tempDir, 'watched.js');
    writeFileSync(watchedFile, 'export const version = 1;\n');

    const mod = {
      execute: async () => ({}),
      description: 'Watched module',
      inputSchema: { type: 'object' },
      outputSchema: { type: 'object' },
    };
    registry.register('watched', mod);

    const seen: Array<{ moduleId: string; payload: unknown }> = [];
    registry.on('file_changed', (moduleId: string, payload: unknown) => {
      seen.push({ moduleId, payload });
    });

    await registry.watch();
    try {
      // fs.watch delivery is asynchronous and platform-dependent; poll rather
      // than sleeping a fixed amount, and re-touch in case the watcher was not
      // yet armed when the first write landed.
      const deadline = Date.now() + 5000;
      while (seen.length === 0 && Date.now() < deadline) {
        writeFileSync(watchedFile, `export const version = ${Date.now()};\n`);
        await new Promise((resolve) => setTimeout(resolve, 100));
      }
      expect(seen.length).toBeGreaterThan(0);
      expect(seen[0].moduleId).toBe('watched');
      expect((seen[0].payload as { filePath: string }).filePath).toContain('watched.js');
    } finally {
      registry.unwatch();
    }
  }, 15000);

  it('unwatch() is safe to call when not watching', () => {
    const registry = new Registry();
    expect(() => registry.unwatch()).not.toThrow();
    // Call again to verify idempotent
    expect(() => registry.unwatch()).not.toThrow();
  });

  it('watch() is idempotent (calling twice does not throw)', () => {
    const registry = new Registry({ extensionsDir: tempDir });
    registry.watch();
    expect(() => registry.watch()).not.toThrow();
    registry.unwatch();
  });

  it('_pathToModuleId maps a file path to a module ID correctly', () => {
    const registry = new Registry();
    const mod = {
      execute: async () => ({}),
      description: 'Test module',
      inputSchema: { type: 'object' },
      outputSchema: { type: 'object' },
    };
    registry.register('my_module', mod);
    const result = (registry as any)._pathToModuleId('/some/path/my_module.ts');
    expect(result).toBe('my_module');
  });

  it('_pathToModuleId maps a namespaced module correctly', () => {
    const registry = new Registry();
    const mod = {
      execute: async () => ({}),
      description: 'Namespaced module',
      inputSchema: { type: 'object' },
      outputSchema: { type: 'object' },
    };
    registry.register('ns.my_module', mod);
    const result = (registry as any)._pathToModuleId('/some/path/my_module.js');
    expect(result).toBe('ns.my_module');
  });

  it('_pathToModuleId returns null for an unknown file', () => {
    const registry = new Registry();
    const mod = {
      execute: async () => ({}),
      description: 'Test module',
      inputSchema: { type: 'object' },
      outputSchema: { type: 'object' },
    };
    registry.register('my_module', mod);
    const result = (registry as any)._pathToModuleId('/some/path/unknown_file.ts');
    expect(result).toBeNull();
  });

  it('_handleFileDeletion unregisters a known module and calls onUnload exactly once', async () => {
    const registry = new Registry();
    let unloadCount = 0;
    const mod = {
      execute: async () => ({}),
      description: 'Deletable module',
      inputSchema: { type: 'object' },
      outputSchema: { type: 'object' },
      onUnload() { unloadCount += 1; },
    };
    registry.register('deletable', mod);
    expect(registry.has('deletable')).toBe(true);

    await (registry as any)._handleFileDeletion('/extensions/deletable.ts');

    expect(registry.has('deletable')).toBe(false);
    // unregister() owns the onUnload call; a handler that also calls it
    // releases the module's resources twice.
    expect(unloadCount).toBe(1);
  });

  it('_handleFileDeletion does nothing for an unknown file', () => {
    const registry = new Registry();
    const mod = {
      execute: async () => ({}),
      description: 'Existing module',
      inputSchema: { type: 'object' },
      outputSchema: { type: 'object' },
    };
    registry.register('existing', mod);
    // Should not throw, should not affect existing modules
    (registry as any)._handleFileDeletion('/some/path/unknown.ts');
    expect(registry.has('existing')).toBe(true);
  });

  it('A-D-104: _handleFileChange unregisters the module and emits file_changed (TS event-only contract)', async () => {
    // Per sync finding A-D-104, the TS SDK is event-only by design — see the
    // JSDoc on Registry.watch(). Python re-imports, Rust re-discovers, but
    // ES modules cannot be reliably evicted from Node's loader cache, so TS
    // unregisters and emits 'file_changed' for consumers to re-register.
    const registry = new Registry();
    let unloadCount = 0;
    const mod = {
      execute: async () => ({}),
      description: 'Watched module',
      inputSchema: { type: 'object' },
      outputSchema: { type: 'object' },
      onUnload() { unloadCount += 1; },
    };
    registry.register('watched', mod);

    const events: Array<{ moduleId: string; payload: unknown }> = [];
    // D-80: 'file_changed' IS a standard registry event and is subscribed to
    // through the public `on()`. This test used to reach into `_callbacks`
    // directly because `on()` rejected the name — which was the bug, not a
    // testing convenience.
    registry.on('file_changed', (id: string, payload: unknown) => {
      events.push({ moduleId: id, payload });
    });

    await (registry as unknown as { _handleFileChange: (p: string) => Promise<void> })
      ._handleFileChange('/extensions/watched.js');

    // Module is unregistered (Python/Rust diverge: they would re-register).
    expect(registry.has('watched')).toBe(false);
    expect(unloadCount).toBe(1);

    // file_changed event is emitted with the module ID and file path.
    expect(events).toHaveLength(1);
    expect(events[0].moduleId).toBe('watched');
    expect((events[0].payload as { filePath: string }).filePath).toBe('/extensions/watched.js');
  });
});

/* -----------------------------------------------------------
 * Custom Discoverer
 * --------------------------------------------------------- */

describe('Registry custom discoverer', () => {
  it('uses custom discoverer when set', async () => {
    const modA = {
      execute: async () => ({}),
      description: 'Custom module A',
      inputSchema: { type: 'object' },
      outputSchema: { type: 'object' },
    };
    const modB = {
      execute: async () => ({}),
      description: 'Custom module B',
      inputSchema: { type: 'object' },
      outputSchema: { type: 'object' },
    };

    let calledWithRoots: string[] | null = null;
    const discoverer = {
      discover(roots: string[]) {
        calledWithRoots = roots;
        return [
          { moduleId: 'custom.a', module: modA },
          { moduleId: 'custom.b', module: modB },
        ];
      },
    };

    const registry = new Registry();
    registry.setDiscoverer(discoverer);
    const count = await registry.discover();

    expect(count).toBe(2);
    expect(registry.has('custom.a')).toBe(true);
    expect(registry.has('custom.b')).toBe(true);
    expect(registry.get('custom.a')).toBe(modA);
    expect(registry.get('custom.b')).toBe(modB);
    expect(calledWithRoots).toEqual(['./extensions']);
  });

  it('uses default discoverer when none set', async () => {
    const tempDir = mkdtempSync(join(tmpdir(), 'apcore-registry-defdisc-'));
    try {
      writeModuleFile(
        tempDir,
        'default_mod.js',
        `export default {
          execute: async () => ({}),
          description: 'Default module',
          inputSchema: { type: 'object' },
          outputSchema: { type: 'object' },
        };`,
      );

      const registry = new Registry({ extensionsDir: tempDir });
      const count = await registry.discover();

      expect(count).toBe(1);
      expect(registry.has('default_mod')).toBe(true);
    } finally {
      rmSync(tempDir, { recursive: true, force: true });
    }
  });

  it('supports async discoverer', async () => {
    const mod = {
      execute: async () => ({}),
      description: 'Async discovered module',
      inputSchema: { type: 'object' },
      outputSchema: { type: 'object' },
    };

    const discoverer = {
      async discover(_roots: string[]) {
        return [{ moduleId: 'async.mod', module: mod }];
      },
    };

    const registry = new Registry();
    registry.setDiscoverer(discoverer);
    const count = await registry.discover();

    expect(count).toBe(1);
    expect(registry.has('async.mod')).toBe(true);
  });

  it('skips malformed entries and keeps registering valid ones', async () => {
    const modOk = {
      execute: async () => ({}),
      description: 'Valid module',
      inputSchema: { type: 'object' },
      outputSchema: { type: 'object' },
    };

    const discoverer = {
      discover(_roots: string[]) {
        return [
          { module: modOk },
          { moduleId: 'only_id' },
          'not-an-object',
          null,
          { moduleId: 'good.mod', module: modOk },
        ] as Array<{ moduleId: string; module: unknown }>;
      },
    };

    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const registry = new Registry();
    registry.setDiscoverer(discoverer);
    const count = await registry.discover();

    expect(count).toBe(1);
    expect(registry.has('good.mod')).toBe(true);
    const malformedCalls = warnSpy.mock.calls.filter((call) =>
      String(call[0]).includes('Malformed entry'),
    );
    expect(malformedCalls.length).toBe(4);
    warnSpy.mockRestore();
  });

  it('handles non-array return from discoverer', async () => {
    const discoverer = {
      discover(_roots: string[]) {
        return null as unknown as Array<{ moduleId: string; module: unknown }>;
      },
    };

    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const registry = new Registry();
    registry.setDiscoverer(discoverer);
    const count = await registry.discover();

    expect(count).toBe(0);
    expect(warnSpy).toHaveBeenCalledWith(expect.stringContaining('non-array'));
    warnSpy.mockRestore();
  });
});

/* -----------------------------------------------------------
 * Custom Validator
 * --------------------------------------------------------- */

describe('Registry custom validator', () => {
  it('rejects modules when custom validator returns errors', async () => {
    const mod = {
      execute: async () => ({}),
      description: 'To be rejected',
      inputSchema: { type: 'object' },
      outputSchema: { type: 'object' },
    };

    const discoverer = {
      discover(_roots: string[]) {
        return [{ moduleId: 'rejected.mod', module: mod }];
      },
    };

    const validator = {
      validate(_module: unknown) {
        return ['rejected by custom validator'];
      },
    };

    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const registry = new Registry();
    registry.setDiscoverer(discoverer);
    registry.setValidator(validator);
    const count = await registry.discover();

    expect(count).toBe(0);
    expect(registry.has('rejected.mod')).toBe(false);
    warnSpy.mockRestore();
  });

  it('accepts modules when custom validator returns empty list', async () => {
    const mod = {
      execute: async () => ({}),
      description: 'To be accepted',
      inputSchema: { type: 'object' },
      outputSchema: { type: 'object' },
    };

    const discoverer = {
      discover(_roots: string[]) {
        return [{ moduleId: 'accepted.mod', module: mod }];
      },
    };

    const validator = {
      validate(_module: unknown) {
        return [];
      },
    };

    const registry = new Registry();
    registry.setDiscoverer(discoverer);
    registry.setValidator(validator);
    const count = await registry.discover();

    expect(count).toBe(1);
    expect(registry.has('accepted.mod')).toBe(true);
    expect(registry.get('accepted.mod')).toBe(mod);
  });

  it('custom validator works with default file-system discovery', async () => {
    const tempDir = mkdtempSync(join(tmpdir(), 'apcore-registry-customval-'));
    try {
      writeModuleFile(
        tempDir,
        'val_mod.js',
        `export default {
          execute: async () => ({}),
          description: 'Validated module',
          inputSchema: { type: 'object' },
          outputSchema: { type: 'object' },
        };`,
      );

      const validator = {
        validate(_module: unknown) {
          return ['rejected by custom validator'];
        },
      };

      const registry = new Registry({ extensionsDir: tempDir });
      registry.setValidator(validator);
      const count = await registry.discover();

      // Custom validator rejects all, so nothing should be registered
      expect(count).toBe(0);
      expect(registry.has('val_mod')).toBe(false);
    } finally {
      rmSync(tempDir, { recursive: true, force: true });
    }
  });

  it('supports async validator', async () => {
    const mod = {
      execute: async () => ({}),
      description: 'Async validated module',
      inputSchema: { type: 'object' },
      outputSchema: { type: 'object' },
    };

    const discoverer = {
      discover(_roots: string[]) {
        return [{ moduleId: 'async.validated', module: mod }];
      },
    };

    const validator = {
      async validate(_module: unknown) {
        return [];
      },
    };

    const registry = new Registry();
    registry.setDiscoverer(discoverer);
    registry.setValidator(validator);
    const count = await registry.discover();

    expect(count).toBe(1);
    expect(registry.has('async.validated')).toBe(true);
  });
});


describe('Registry hot-reload API (acquire/release/drain)', () => {
  function createMod(id: string): FunctionModule {
    return new FunctionModule({
      execute: () => ({}),
      moduleId: id,
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      description: 'Test module',
    });
  }

  it('acquire increments refCount and release decrements it', () => {
    const registry = new Registry();
    registry.register('test.mod', createMod('test.mod'));

    expect(registry.refCount.get('test.mod')).toBeUndefined();
    registry.acquire('test.mod');
    expect(registry.refCount.get('test.mod')).toBe(1);
    registry.release('test.mod');
    expect(registry.refCount.get('test.mod')).toBeUndefined();
  });

  it('acquire throws ModuleNotFoundError for unknown module', () => {
    const registry = new Registry();
    expect(() => registry.acquire('no.such')).toThrow(ModuleNotFoundError);
  });

  it('acquire throws ModuleNotFoundError when module is draining', () => {
    const registry = new Registry();
    registry.register('drain.mod', createMod('drain.mod'));
    registry.beginDrain('drain.mod');
    expect(() => registry.acquire('drain.mod')).toThrow(ModuleNotFoundError);
  });

  it('isDraining returns true after beginDrain', () => {
    const registry = new Registry();
    registry.register('d.mod', createMod('d.mod'));
    expect(registry.isDraining('d.mod')).toBe(false);
    registry.beginDrain('d.mod');
    expect(registry.isDraining('d.mod')).toBe(true);
    registry.endDrain('d.mod');
    expect(registry.isDraining('d.mod')).toBe(false);
  });

  it('release is a no-op when refCount is 0 or undefined', () => {
    const registry = new Registry();
    registry.register('test.mod', createMod('test.mod'));
    // release without acquire should not throw
    expect(() => registry.release('test.mod')).not.toThrow();
  });

  it('release keeps count when multiple acquires', () => {
    const registry = new Registry();
    registry.register('test.mod', createMod('test.mod'));
    registry.acquire('test.mod');
    registry.acquire('test.mod');
    expect(registry.refCount.get('test.mod')).toBe(2);
    registry.release('test.mod');
    expect(registry.refCount.get('test.mod')).toBe(1);
    registry.release('test.mod');
    expect(registry.refCount.get('test.mod')).toBeUndefined();
  });

  it('waitDrained resolves immediately when no in-flight executions', async () => {
    const registry = new Registry();
    registry.register('test.mod', createMod('test.mod'));
    const result = await registry.waitDrained('test.mod');
    expect(result).toBe(true);
  });

  it('waitDrained resolves when release clears refCount', async () => {
    const registry = new Registry();
    registry.register('test.mod', createMod('test.mod'));
    registry.acquire('test.mod');
    const drainPromise = registry.waitDrained('test.mod', 5000);
    registry.release('test.mod');
    expect(await drainPromise).toBe(true);
  });

  it('waitDrained resolves false on timeout when refCount stays positive', async () => {
    const registry = new Registry();
    registry.register('test.mod', createMod('test.mod'));
    registry.acquire('test.mod');
    const result = await registry.waitDrained('test.mod', 10); // 10ms timeout
    expect(result).toBe(false);
    // Cleanup
    registry.release('test.mod');
  });

  it('safeUnregister returns false for unknown module', async () => {
    const registry = new Registry();
    const result = await registry.safeUnregister('no.such');
    expect(result).toBe(false);
  });

  it('safeUnregister cleanly unregisters a non-acquired module', async () => {
    const registry = new Registry();
    registry.register('test.mod', createMod('test.mod'));
    const result = await registry.safeUnregister('test.mod');
    expect(result).toBe(true);
    expect(registry.has('test.mod')).toBe(false);
  });

  it('safeUnregister force-unloads and warns after timeout', async () => {
    const registry = new Registry();
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    registry.register('test.mod', createMod('test.mod'));
    registry.acquire('test.mod'); // hold a ref so it never drains
    const result = await registry.safeUnregister('test.mod', 10); // 10ms timeout
    expect(result).toBe(false);
    expect(warnSpy).toHaveBeenCalledWith(expect.stringContaining('Force-unloading'));
    warnSpy.mockRestore();
  });

  it('exportSchema returns null for unknown module', () => {
    const registry = new Registry();
    expect(registry.exportSchema('no.such')).toBeNull();
  });

  it('exportSchema returns strict schema when strict=true', () => {
    const registry = new Registry();
    const mod = createMod('test.mod');
    registry.register('test.mod', mod);
    const result = registry.exportSchema('test.mod', true);
    expect(result).not.toBeNull();
  });

  it('clearCache clears the schema cache', () => {
    const registry = new Registry();
    registry.register('test.mod', createMod('test.mod'));
    registry.exportSchema('test.mod'); // populate cache
    expect(() => registry.clearCache()).not.toThrow();
  });
});

/* -----------------------------------------------------------
 * Sync findings A-D-101 / A-D-102 — validation in discover paths
 * --------------------------------------------------------- */

describe('Registry discover() validates module IDs (A-D-101, A-D-102)', () => {
  let tempDir: string;

  beforeEach(() => {
    tempDir = mkdtempSync(join(tmpdir(), 'apcore-registry-validate-'));
  });

  afterEach(() => {
    rmSync(tempDir, { recursive: true, force: true });
  });

  it('A-D-101: _registerInOrder skips entries whose canonical ID violates pattern (default discover path)', async () => {
    // Use a custom discoverer that injects an invalid moduleId ("BadID") to
    // bypass scanner-level normalization and simulate an upstream override
    // (idMap) producing an invalid ID. The default-discovery path itself
    // normally produces lowercase IDs from filesystem names, so we exercise
    // the _registerInOrder gate via its default path here using the custom
    // discoverer entry-points API.
    const goodMod = createMod('good_mod');
    const badMod = createMod('good_mod'); // same shape, but registered under bad ID

    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});

    const registry = new Registry();
    // Use _discoverCustom path (A-D-102) — invalid ID must be skipped.
    registry.setDiscoverer({
      discover: async () => [
        { moduleId: 'BadID', module: badMod }, // invalid — uppercase
        { moduleId: 'good_mod', module: goodMod },
      ],
    });

    const count = await registry.discover();
    expect(count).toBe(1);
    expect(registry.has('BadID')).toBe(false);
    expect(registry.has('good_mod')).toBe(true);

    // Should have warned about the invalid ID
    expect(
      warnSpy.mock.calls.some((args) =>
        args.some((a) => typeof a === 'string' && /BadID|Invalid module ID|reserved/.test(a)) ||
        args.some((a) => a instanceof Error && /BadID|Invalid module ID|reserved/.test(a.message))
      ),
    ).toBe(true);
    warnSpy.mockRestore();
  });

  it('A-D-102: _discoverCustom rejects invalid module IDs (uppercase) and continues with valid ones', async () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});

    const registry = new Registry();
    registry.setDiscoverer({
      discover: async () => [
        { moduleId: 'INVALID-ID', module: createMod('whatever') }, // hyphen + uppercase
        { moduleId: '1leadingdigit', module: createMod('whatever2') }, // pattern violation
        { moduleId: 'valid.mod', module: createMod('valid.mod') },
      ],
    });

    const count = await registry.discover();
    expect(count).toBe(1);
    expect(registry.has('INVALID-ID')).toBe(false);
    expect(registry.has('1leadingdigit')).toBe(false);
    expect(registry.has('valid.mod')).toBe(true);
    warnSpy.mockRestore();
  });

  it('A-D-102: _discoverCustom rejects reserved-word first-segment IDs from custom discoverer', async () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});

    const registry = new Registry();
    registry.setDiscoverer({
      discover: async () => [
        { moduleId: 'system.foo', module: createMod('system.foo') }, // reserved
        { moduleId: 'app.foo', module: createMod('app.foo') },
      ],
    });

    const count = await registry.discover();
    expect(count).toBe(1);
    expect(registry.has('system.foo')).toBe(false);
    expect(registry.has('app.foo')).toBe(true);
    warnSpy.mockRestore();
  });

  it('A-D-101: _registerInOrder skips ID with reserved-word first segment (default discover path)', async () => {
    // Filesystem-based discovery: canonicalId derived from filename. Use a
    // top-level dir named "system" to produce a "system" canonicalId, which
    // is a reserved word and must be filtered.
    mkdirSync(join(tempDir, 'system'));
    writeFileSync(
      join(tempDir, 'system', 'badmod.js'),
      `export default {
        execute: async () => ({}),
        description: 'Reserved-prefix module',
        inputSchema: { type: 'object' },
        outputSchema: { type: 'object' },
      };`,
      'utf-8',
    );
    writeFileSync(
      join(tempDir, 'goodmod.js'),
      `export default {
        execute: async () => ({}),
        description: 'Good module',
        inputSchema: { type: 'object' },
        outputSchema: { type: 'object' },
      };`,
      'utf-8',
    );

    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const registry = new Registry({ extensionsDir: tempDir });
    const count = await registry.discover();

    // The reserved-prefixed module must be skipped; goodmod must register.
    expect(registry.has('goodmod')).toBe(true);
    expect(registry.has('system.badmod')).toBe(false);
    expect(count).toBeGreaterThanOrEqual(1);
    warnSpy.mockRestore();
  });
});

describe('Registry.getModuleMetadata — dependencies reach the accessor (issue #35)', () => {
  // `mergeModuleMetadata` used to drop `dependencies` on its way into
  // `_moduleMeta`. Unit-testing the merge function alone would not have caught
  // that: discovery reads dependencies off the raw YAML metadata *before* the
  // merge, so load ordering looked healthy while every post-registration
  // reader — `ReloadModule._topoSortModules` above all — saw an empty graph.
  // These assertions go through real registration calls for that reason.

  it('registerInternal carries code-declared dependencies to the accessor', () => {
    const registry = new Registry();
    registry.registerInternal('executor.alpha', {
      description: 'depends on zulu',
      version: '1.0.0',
      dependencies: [{ module_id: 'executor.zulu' }],
      execute: () => ({}),
    });

    expect(registry.getModuleMetadata('executor.alpha')['dependencies']).toEqual([
      { module_id: 'executor.zulu' },
    ]);
  });

  it('register() metadata overrides win over code-declared dependencies', async () => {
    const registry = new Registry();
    await registry.register(
      'executor.alpha',
      {
        description: 'declares one dependency in code',
        version: '1.0.0',
        dependencies: [{ module_id: 'executor.ignored' }],
        execute: () => ({}),
        inputSchema: { type: 'object' },
        outputSchema: { type: 'object' },
      },
      null,
      { dependencies: [{ module_id: 'executor.zulu', optional: true }] },
    );

    expect(registry.getModuleMetadata('executor.alpha')['dependencies']).toEqual([
      { module_id: 'executor.zulu', optional: true },
    ]);
  });

  it('returns an empty dependencies list when neither code nor metadata declares any', () => {
    const registry = new Registry();
    registry.registerInternal('executor.zulu', {
      description: 'no dependencies',
      version: '1.0.0',
      execute: () => ({}),
    });

    expect(registry.getModuleMetadata('executor.zulu')['dependencies']).toEqual([]);
  });

  it('returns an empty object for an unknown module', () => {
    expect(new Registry().getModuleMetadata('nope.not.here')).toEqual({});
  });

  it('returns a copy — mutating the result does not corrupt registry state', () => {
    const registry = new Registry();
    registry.registerInternal('executor.zulu', {
      description: 'no dependencies',
      version: '1.0.0',
      execute: () => ({}),
    });

    const meta = registry.getModuleMetadata('executor.zulu');
    meta['description'] = 'mutated';
    expect(registry.getModuleMetadata('executor.zulu')['description']).toBe('no dependencies');
  });
});

describe('registerInternal reserves the ID during onLoad (sync finding A-D-001)', () => {
  // The reservation rule is stated at registry.ts:684 — "The ID is reserved in
  // `_inFlight` for the whole of onLoad. `register()` computes its duplicate
  // set as `_modules ∪ _inFlight`" — and registerInternal did neither: its
  // conflict set read `_modules` alone and it never populated `_inFlight`. A
  // re-entrant same-ID registration during onLoad was therefore accepted; the
  // inner call published its module and fired a `register` event, then the
  // outer publish overwrote it last-writer-wins, orphaning the first instance
  // and leaving its onUnload unreachable.
  //
  // registry-system.md's "Applies to every registration path" forbids
  // per-path exceptions. apcore-python (registry.py:2283) and apcore-rust
  // (registry.rs:1151) both reserve.
  const leaf = () => ({
    description: 'm',
    inputSchema: {},
    outputSchema: {},
    execute: async () => ({}),
  });

  it('rejects a re-entrant registration of the same ID with DUPLICATE_MODULE_ID', () => {
    const registry = new Registry();
    const inner = leaf();
    let innerError: unknown = null;
    const outer = {
      ...leaf(),
      onLoad() {
        try {
          registry.registerInternal('system.x', inner);
        } catch (e) {
          innerError = e;
        }
      },
    };

    registry.registerInternal('system.x', outer);

    expect(innerError).toBeInstanceOf(DuplicateModuleIdError);
    expect((innerError as DuplicateModuleIdError).code).toBe('DUPLICATE_MODULE_ID');
    expect(registry.get('system.x')).toBe(outer);
  });

  it('fires exactly one register event for the ID', () => {
    const registry = new Registry();
    const seen: string[] = [];
    registry.on('register', (id: string) => seen.push(id));

    const inner = leaf();
    const outer = {
      ...leaf(),
      onLoad() {
        try {
          registry.registerInternal('system.y', inner);
        } catch {
          /* expected */
        }
      },
    };
    registry.registerInternal('system.y', outer);

    expect(seen.filter((id) => id === 'system.y')).toHaveLength(1);
  });

  it('releases the reservation so a later registration of a fresh ID works', () => {
    const registry = new Registry();
    registry.registerInternal('system.a', leaf());
    registry.registerInternal('system.b', leaf());
    expect(registry.get('system.a')).not.toBeNull();
    expect(registry.get('system.b')).not.toBeNull();
  });

  it('releases the reservation when onLoad throws', () => {
    const registry = new Registry();
    const failing = {
      ...leaf(),
      onLoad() {
        throw new Error('boom');
      },
    };
    expect(() => registry.registerInternal('system.c', failing)).toThrow('boom');
    // The failed ID must be free again, not stuck reserved.
    expect(() => registry.registerInternal('system.c', leaf())).not.toThrow();
    expect(registry.get('system.c')).not.toBeNull();
  });
});

describe('the custom-discoverer path reserves the ID during onLoad (A-D-001)', () => {
  // `_registerImpl` was the one registration path that never reserved the ID
  // in `_inFlight`, even though `register()` computes its duplicate set as
  // `_modules ∪ _inFlight`. While a custom-discovered module ran its sync
  // onLoad, the ID was in neither set, so a re-entrant `register()` of that
  // same ID was accepted: it published its own module and fired a `register`
  // event, and the discoverer's publish then overwrote it last-writer-wins —
  // orphaning a module whose onLoad had run, whose register event had been
  // delivered, and whose onUnload was thereafter unreachable.
  const leaf = () => ({
    description: 'm',
    inputSchema: {},
    outputSchema: {},
    execute: async () => ({}),
  });

  it('rejects a re-entrant registration of the same ID with DUPLICATE_MODULE_ID', async () => {
    const registry = new Registry();
    const inner = leaf();
    let innerError: unknown = null;
    const outer = {
      ...leaf(),
      onLoad() {
        try {
          void registry.register('custom.x', inner);
        } catch (e) {
          innerError = e;
        }
      },
    };
    registry.setDiscoverer({
      discover: () => [{ moduleId: 'custom.x', module: outer }],
    });

    expect(await registry.discover()).toBe(1);
    expect(innerError).toBeInstanceOf(DuplicateModuleIdError);
    expect((innerError as DuplicateModuleIdError).code).toBe('DUPLICATE_MODULE_ID');
    expect(registry.get('custom.x')).toBe(outer);
  });

  it('fires exactly one register event for the ID', async () => {
    const registry = new Registry();
    const seen: string[] = [];
    registry.on('register', (id: string) => seen.push(id));

    const outer = {
      ...leaf(),
      onLoad() {
        try {
          void registry.register('custom.y', leaf());
        } catch {
          /* expected */
        }
      },
    };
    registry.setDiscoverer({
      discover: () => [{ moduleId: 'custom.y', module: outer }],
    });

    await registry.discover();
    expect(seen.filter((id) => id === 'custom.y')).toHaveLength(1);
  });

  it('releases the reservation on success and on an onLoad failure', async () => {
    const registry = new Registry();
    const failing = {
      ...leaf(),
      onLoad() {
        throw new Error('boom');
      },
    };
    registry.setDiscoverer({
      discover: () => [
        { moduleId: 'custom.ok', module: leaf() },
        { moduleId: 'custom.bad', module: failing },
      ],
    });

    // The failing module is skipped with a warning, not fatal to the run.
    expect(await registry.discover()).toBe(1);
    expect(registry.get('custom.ok')).not.toBeNull();
    // Neither ID may be left stuck reserved: both must be registrable again.
    await expect(registry.register('custom.bad', leaf())).resolves.toBeUndefined();
    expect(registry.get('custom.bad')).not.toBeNull();
  });
});

describe('getDefinition derives sunsetDate from the x-deprecation block', () => {
  // `sunsetDate` used to be read from a top-level `meta['sunsetDate']` key
  // that `mergeModuleMetadata` never writes, so the field was null for every
  // module ever registered and no deprecation notice was ever emitted.
  // apcore-python derives it from `metadata["x-deprecation"]["sunset_date"]`
  // and logs a warning.
  const deprecatedModule = () => ({
    description: 'm',
    inputSchema: {},
    outputSchema: {},
    execute: async () => ({}),
    metadata: {
      'x-deprecation': {
        deprecated_since: '1.2.0',
        sunset_version: '2.0.0',
        sunset_date: '2027-01-01',
        migration_guide: 'use billing.charge_v2',
      },
    },
  });

  it('reads sunset_date and warns once per module version', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      const registry = new Registry();
      await registry.register('billing.charge', deprecatedModule());

      const def = registry.getDefinition('billing.charge');
      expect(def!.sunsetDate).toBe('2027-01-01');

      expect(warn).toHaveBeenCalledTimes(1);
      const msg = String(warn.mock.calls[0]![0]);
      expect(msg).toContain("Module 'billing.charge' v1.0.0 is deprecated");
      expect(msg).toContain('since 1.2.0');
      expect(msg).toContain('sunset in 2.0.0');
      expect(msg).toContain('Migration: use billing.charge_v2');

      // getDefinition is a read callers make repeatedly: the notice is once
      // per module version, not once per call.
      registry.getDefinition('billing.charge');
      expect(warn).toHaveBeenCalledTimes(1);
    } finally {
      warn.mockRestore();
    }
  });

  it('falls back to "unknown" for absent deprecation fields and reports no sunsetDate', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      const registry = new Registry();
      await registry.register('billing.partial', {
        description: 'm',
        inputSchema: {},
        outputSchema: {},
        execute: async () => ({}),
        metadata: { 'x-deprecation': {} },
      });

      expect(registry.getDefinition('billing.partial')!.sunsetDate).toBeNull();
      const msg = String(warn.mock.calls[0]![0]);
      expect(msg).toContain('since unknown, sunset in unknown');
      expect(msg).not.toContain('Migration:');
    } finally {
      warn.mockRestore();
    }
  });

  it('leaves sunsetDate null and warns nothing for a module with no x-deprecation', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      const registry = new Registry();
      await registry.register('billing.current', createMod('billing.current'));

      expect(registry.getDefinition('billing.current')!.sunsetDate).toBeNull();
      expect(warn).not.toHaveBeenCalled();
    } finally {
      warn.mockRestore();
    }
  });
});

describe('register() runs the custom validator before duplicate detection', () => {
  // apcore-python (registry.py:1305) and apcore-rust (registry.rs:1094) both
  // validate first. With the order inverted, one call reported three
  // different error codes across the three SDKs, and a stateful validator
  // was invoked for a duplicate ID on Python/Rust but skipped here.
  it('reports the validator rejection rather than DuplicateModuleIdError', async () => {
    const registry = new Registry();
    await registry.register('dup.mod', createMod('dup.mod'));

    const seen: unknown[] = [];
    registry.setValidator({
      validate: (module: unknown) => {
        seen.push(module);
        return ['not acceptable'];
      },
    });

    expect(() => registry.register('dup.mod', createMod('dup.mod'))).toThrow(InvalidInputError);
    // The validator saw the module: a stateful validator now behaves the
    // same way in all three SDKs.
    expect(seen).toHaveLength(1);
  });

  it('still throws DuplicateModuleIdError synchronously when the validator accepts', async () => {
    const registry = new Registry();
    await registry.register('dup.ok', createMod('dup.ok'));
    registry.setValidator({ validate: () => [] });

    // Synchronous throw — the `.toThrow()` compat property the duplicate
    // check is placed for.
    expect(() => registry.register('dup.ok', createMod('dup.ok'))).toThrow(DuplicateModuleIdError);
  });

  it('defers the duplicate check into the returned promise for an async validator', async () => {
    const registry = new Registry();
    await registry.register('dup.async', createMod('dup.async'));
    registry.setValidator({ validate: async () => [] });

    await expect(registry.register('dup.async', createMod('dup.async'))).rejects.toBeInstanceOf(
      DuplicateModuleIdError,
    );
  });
});
