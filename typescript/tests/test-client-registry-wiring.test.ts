/**
 * CLI-1 / CLI-3 — the client's Registry must be the executor's, and must be
 * built from the Config.
 *
 * CLI-1: `new APCore({ executor })` created a SECOND `Registry` and kept it as
 * `client.registry`, so `register()` wrote into one registry and `call()`
 * looked up in the executor's other one. apcore-rust adopts the executor's
 * registry (`client.rs:104-107`) and `apcore-client.md` states the rule for
 * that constructor: registry is "Ignored when executor is also provided".
 *
 * CLI-3: the fallback `new Registry()` was built with NO options even when a
 * Config was present, although the Registry constructor resolves
 * `extensions.root` / `extensions.roots` / `id_map.overrides` from one — so
 * every registry-side config key was inert through the client door and
 * `client.discover()` scanned a hardcoded `./extensions`.
 */

import { describe, it, expect, afterAll } from 'vitest';
import { mkdtempSync, rmSync, mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { Type } from '@sinclair/typebox';
import { APCore } from '../src/client.js';
import { Config } from '../src/config.js';
import { Registry } from '../src/registry/registry.js';
import { Executor } from '../src/executor.js';

const DUMMY = {
  inputSchema: Type.Object({}),
  outputSchema: Type.Object({ ok: Type.Boolean() }),
  description: 'dummy',
  execute: () => ({ ok: true }),
};

describe('APCore adopts a supplied Executor’s Registry (CLI-1)', () => {
  it('client.registry IS the executor’s registry', () => {
    const executorRegistry = new Registry();
    const executor = new Executor({ registry: executorRegistry });
    const client = new APCore({ executor });
    expect(client.registry).toBe(executorRegistry);
  });

  it('a module registered through the client can be called through the client', async () => {
    const executor = new Executor({ registry: new Registry() });
    const client = new APCore({ executor });

    client.register('executor.demo.ping', DUMMY);
    expect(client.listModules()).toContain('executor.demo.ping');

    const result = await client.call('executor.demo.ping', {});
    expect(result['ok']).toBe(true);
  });

  it('an explicit registry option is ignored when an executor is also given', () => {
    const executorRegistry = new Registry();
    const executor = new Executor({ registry: executorRegistry });
    const ignored = new Registry();
    const client = new APCore({ executor, registry: ignored });
    expect(client.registry).toBe(executorRegistry);
    expect(client.registry).not.toBe(ignored);
  });

  it('client.disable() reaches the sys module when an executor was supplied', async () => {
    const registry = new Registry();
    const executor = new Executor({ registry });
    const config = new Config({
      sys_modules: { enabled: true, events: { enabled: true }, control: { enabled: true } },
    });
    const client = new APCore({ executor, config });

    client.register('executor.demo.ping', DUMMY);
    const out = await client.disable('executor.demo.ping', 'CLI-1 regression');
    expect(out['success']).toBe(true);
  });
});

describe('APCore builds its fallback Registry from the Config (CLI-3)', () => {
  const tmp = mkdtempSync(join(tmpdir(), 'apcore-cli3-'));
  afterAll(() => rmSync(tmp, { recursive: true, force: true }));

  function writeModule(dir: string, name: string, description: string): void {
    mkdirSync(dir, { recursive: true });
    writeFileSync(
      join(dir, name),
      `export default {
        execute: async () => ({ ok: true }),
        description: '${description}',
        inputSchema: { type: 'object', properties: {} },
        outputSchema: { type: 'object', properties: { ok: { type: 'boolean' } } },
      };`,
    );
  }

  it('extensions.root from config reaches discover()', async () => {
    const root = join(tmp, 'my-extensions');
    writeModule(root, 'greeter.js', 'a greeter');

    const config = new Config({ extensions: { root } });
    const client = new APCore({ config });

    const count = await client.discover();
    expect(count).toBe(1);
    expect(client.listModules()).toContain('greeter');
  });

  it('extensions.roots from config reaches discover()', async () => {
    const root = join(tmp, 'roots_mode');
    writeModule(root, 'farewell.js', 'a farewell');

    const config = new Config({ extensions: { roots: [root] } });
    const client = new APCore({ config });
    const count = await client.discover();
    // Multi-root mode namespaces each root by its last path segment.
    expect(count).toBe(1);
    expect(client.listModules()).toContain('roots_mode.farewell');
  });

  it('an explicit registry option still wins over the config', () => {
    const explicit = new Registry();
    const config = new Config({ extensions: { root: join(tmp, 'never-scanned') } });
    const client = new APCore({ config, registry: explicit });
    expect(client.registry).toBe(explicit);
  });
});
