/**
 * Package integrity tests.
 *
 * Verifies that the npm package is correctly configured for publishing:
 * - Entry points (main, types, exports) reference files that exist after build
 * - The "files" field restricts what gets published
 * - The "prepublishOnly" script ensures build runs before publish
 * - The VERSION constant matches package.json version
 */

import { describe, it, expect } from 'vitest';
import { existsSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { readFileSync } from 'node:fs';

const ROOT = resolve(import.meta.dirname, '..');
const pkg = JSON.parse(readFileSync(join(ROOT, 'package.json'), 'utf-8'));

// Use a variable so TypeScript doesn't statically resolve the dist path during `tsc --noEmit`.
const DIST_ENTRY = resolve(ROOT, 'dist', 'index.js');

// The dist entry re-exports ~234 symbols across many modules. A cold import takes ~1.5s in
// isolation, but under full-suite parallelism (100+ test files) the first import can easily
// exceed the default 5s timeout. Give these tests headroom.
const IMPORT_TIMEOUT_MS = 30_000;

describe('package.json publishing config', () => {
  it('has "files" field that includes dist', () => {
    expect(pkg.files).toBeDefined();
    expect(pkg.files).toContain('dist');
  });

  it('"files" does not include source or dev directories', () => {
    const forbidden = ['src', 'tests', 'planning', 'coverage', '.claude', '.github'];
    for (const dir of forbidden) {
      expect(pkg.files, `files should not include "${dir}"`).not.toContain(dir);
    }
  });

  it('has prepublishOnly script that runs build', () => {
    expect(pkg.scripts.prepublishOnly).toBeDefined();
    expect(pkg.scripts.prepublishOnly).toContain('build');
  });
});

describe('dist entry points exist', () => {
  it('main entry (./dist/index.js) exists', () => {
    const mainPath = resolve(ROOT, pkg.main);
    expect(existsSync(mainPath), `${pkg.main} should exist`).toBe(true);
  });

  it('types entry (./dist/index.d.ts) exists', () => {
    const typesPath = resolve(ROOT, pkg.types);
    expect(existsSync(typesPath), `${pkg.types} should exist`).toBe(true);
  });

  it('exports the same portable root for Node and browser', async () => {
    const rootEntry = pkg.exports['.'];
    expect(pkg.sideEffects).toBe(false);
    expect(rootEntry.default).toBe(pkg.main);
    expect(rootEntry.types).toBe(pkg.types);
    expect(pkg.browser).toBe(rootEntry.default);
    const root = await import(resolve(ROOT, rootEntry.default));
    const core = await import(resolve(ROOT, pkg.exports['./core'].default));
    expect(Object.keys(root)).toEqual(Object.keys(core));
    expect(root.APCore).toBe(core.APCore);
    expect(root.Registry).toBe(core.Registry);
    expect(root.ACL).toBe(core.ACL);
  }, IMPORT_TIMEOUT_MS);

  it('ships declarations and runtime targets for the explicit profiles', async () => {
    const coreEntry = pkg.exports['./core'];
    const fullEntry = pkg.exports['./full'];
    for (const [subpath, condition] of [
      ['./core', 'default'], ['./full', 'node'], ['./config', 'node'],
      ['./discovery', 'node'], ['./sys-modules', 'node'], ['./events', 'node'],
    ]) {
      const entry = pkg.exports[subpath];
      expect(existsSync(resolve(ROOT, entry.types)), subpath).toBe(true);
      expect(existsSync(resolve(ROOT, entry[condition])), subpath).toBe(true);
    }
    const core = await import(resolve(ROOT, coreEntry.default));
    const full = await import(resolve(ROOT, fullEntry.node));
    expect(core.Context).toBe(full.Context);
    expect(core.Executor).toBe(full.Executor);
    expect(core).not.toHaveProperty('Config');
    expect(full.Config).toBeTypeOf('function');
    expect(full).not.toHaveProperty('PrometheusExporter');
    expect(full).not.toHaveProperty('A2ASubscriber');
    expect(full).not.toHaveProperty('registerSubscriberType');
  }, IMPORT_TIMEOUT_MS);

  it('exports the portable observability declarations and runtime', async () => {
    const entry = pkg.exports['./observability'];
    expect(existsSync(resolve(ROOT, entry.types))).toBe(true);
    const observability = await import(resolve(ROOT, entry.default));
    expect(observability.renderPrometheusMetrics({ exportPrometheus: () => 'sample 1\n' }))
      .toBe('sample 1\n');
    expect(observability).not.toHaveProperty('PrometheusExporter');
  }, IMPORT_TIMEOUT_MS);

  it('top-level "browser" field points at the browser build', () => {
    expect(existsSync(resolve(ROOT, pkg.browser)), `${pkg.browser} should exist`).toBe(true);
  });
});

describe('dist exports are loadable', () => {
  it('can import the package entry point', async () => {
    const mod = await import(DIST_ENTRY);
    expect(mod).toBeDefined();
    expect(typeof mod).toBe('object');
  }, IMPORT_TIMEOUT_MS);

  it('exports key symbols', async () => {
    const mod = await import(DIST_ENTRY);
    // Core classes/functions that consumers depend on
    expect(mod.Registry).toBeDefined();
    expect(mod.Executor).toBeDefined();
    expect(mod.Context).toBeDefined();
    expect(mod).not.toHaveProperty('Config');
    expect(mod.ACL).toBeDefined();
    expect(mod.CancelToken).toBeDefined();
    expect(mod.MiddlewareManager).toBeDefined();
    expect(mod).not.toHaveProperty('ExtensionManager');
    expect(mod).not.toHaveProperty('AsyncTaskManager');
    expect(mod).not.toHaveProperty('BindingLoader');
    expect(mod).not.toHaveProperty('SchemaLoader');
    expect(mod.SchemaValidator).toBeDefined();
    expect(mod).not.toHaveProperty('TracingMiddleware');
    expect(mod).not.toHaveProperty('MetricsCollector');
    expect(mod).not.toHaveProperty('ContextLogger');
    expect(mod.TraceContext).toBeDefined();
    // Error classes
    expect(mod.ModuleError).toBeDefined();
    expect(mod.ErrorCodes).toBeDefined();
  }, IMPORT_TIMEOUT_MS);
});

describe('VERSION constant', () => {
  it('matches package.json version', async () => {
    const mod = await import(DIST_ENTRY);
    expect(mod.VERSION).toBe(pkg.version);
  }, IMPORT_TIMEOUT_MS);
});
