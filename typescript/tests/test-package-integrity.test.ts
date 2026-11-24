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

  it('exports "." node entry exists', () => {
    const nodeEntry = pkg.exports['.'].node;
    expect(existsSync(resolve(ROOT, nodeEntry)), `${nodeEntry} should exist`).toBe(true);
  });

  it('exports "." browser entry exists', () => {
    const browserEntry = pkg.exports['.'].browser;
    expect(existsSync(resolve(ROOT, browserEntry)), `${browserEntry} should exist`).toBe(true);
  });

  it('exports "." default entry exists', () => {
    const defaultEntry = pkg.exports['.'].default;
    expect(existsSync(resolve(ROOT, defaultEntry)), `${defaultEntry} should exist`).toBe(true);
  });

  it('exports "." types entries exist for both conditions', () => {
    const types = pkg.exports['.'].types;
    expect(existsSync(resolve(ROOT, types.browser)), `${types.browser} should exist`).toBe(true);
    expect(existsSync(resolve(ROOT, types.default)), `${types.default} should exist`).toBe(true);
  });

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
    expect(mod.Config).toBeDefined();
    expect(mod.ACL).toBeDefined();
    expect(mod.CancelToken).toBeDefined();
    expect(mod.MiddlewareManager).toBeDefined();
    expect(mod.ExtensionManager).toBeDefined();
    expect(mod.AsyncTaskManager).toBeDefined();
    expect(mod.BindingLoader).toBeDefined();
    expect(mod.SchemaLoader).toBeDefined();
    expect(mod.SchemaValidator).toBeDefined();
    expect(mod.TracingMiddleware).toBeDefined();
    expect(mod.MetricsCollector).toBeDefined();
    expect(mod.ContextLogger).toBeDefined();
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
