import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, it, vi } from 'vitest';

import { APCore } from '../src/client.js';
import { ACL as CoreACL, APCore as CoreAPCore, Registry as CoreRegistry } from '../src/core.js';
import { ACL as FullACL, APCore as FullAPCore } from '../src/full.js';
import { Config, createAclDiscoveryPlugin, loadAclFile } from '../src/config-entry.js';
import { createDiscoveryRegistry, createFilesystemDiscoveryPlugin } from '../src/discovery.js';
import { createNodeLifecyclePlugin } from '../src/lifecycle-node.js';
import type { LifecycleOperation } from '../src/lifecycle.js';
import { createEvent } from '../src/events/emitter.js';
import { Registry } from '../src/registry/registry.js';
import { createSysModulesPlugin } from '../src/sys-modules/entry.js';
import { uuidV4 } from '../src/utils/uuid.js';

const moduleSource = `export default {
  description: 'Return a sample greeting',
  inputSchema: { type: 'object' },
  outputSchema: { type: 'object' },
  execute: () => ({ message: 'hello' }),
};`;

describe('instance-scoped assembly extensions', () => {
  it('discovers into one registry without enabling filesystem access in another', async () => {
    const root = await mkdtemp(join(tmpdir(), 'apcore-explicit-discovery-'));
    try {
      await writeFile(join(root, 'greeter.js'), moduleSource);
      const enabled = createDiscoveryRegistry({ extensionsDir: root });
      const memory = new Registry({ extensionsDir: root, filesystem: null });
      await expect(enabled.discover()).resolves.toBe(1);
      await expect(memory.discover()).rejects.toThrow(/filesystem registry operations/i);
      expect(enabled.has('greeter')).toBe(true);
      expect(memory.has('greeter')).toBe(false);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

  it('uses the discovery plugin only for the client-owned registry', async () => {
    const root = await mkdtemp(join(tmpdir(), 'apcore-discovery-plugin-'));
    try {
      await writeFile(join(root, 'greeter.js'), moduleSource);
      const plugin = createFilesystemDiscoveryPlugin({ extensionsDir: root });
      const owned = new APCore({ plugins: [plugin] });
      await expect(owned.discover()).resolves.toBe(1);
      const supplied = new Registry({ filesystem: null });
      const injected = new APCore({ registry: supplied, plugins: [plugin] });
      expect(injected.registry).toBe(supplied);
      await expect(injected.discover()).rejects.toThrow(/filesystem registry operations/i);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

  it('loads ACL files explicitly and lets the client assemble only its own executor', async () => {
    const root = await mkdtemp(join(tmpdir(), 'apcore-acl-plugin-'));
    try {
      const aclPath = join(root, 'global_acl.yaml');
      await writeFile(aclPath, 'default_effect: deny\nrules: []\n');
      const plugin = createAclDiscoveryPlugin();
      const config = new Config({ acl: { root } });
      const client = new APCore({ config, plugins: [plugin] });
      expect(client.executor.governanceState().aclConfigured).toBe(true);
      expect(loadAclFile(aclPath).check('caller.sample', 'target.sample')).toBe(false);
      const other = new APCore();
      const injected = new APCore({ executor: other.executor, config, plugins: [plugin] });
      expect(injected.executor).toBe(other.executor);
      expect(injected.executor.governanceState().aclConfigured).toBe(false);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });
  it('rejects waiting on the same shutdown after an async callback suspends', async () => {
    const lifecycle = createNodeLifecyclePlugin();
    const first = new APCore({ plugins: [lifecycle] });
    const second = new APCore({ plugins: [lifecycle] });
    expect(first.registry.lifecycleCallbacks).not.toBe(second.registry.lifecycleCallbacks);
    let shutdown: LifecycleOperation;
    let inside: unknown;
    await first.register('letter.send', {
      description: 'Return a sample result',
      inputSchema: { type: 'object' },
      outputSchema: { type: 'object' },
      execute: () => ({}),
      async onUnload(): Promise<void> {
        await Promise.resolve();
        inside = await shutdown.wait(100).catch(error => error);
      },
    });
    shutdown = first.startShutdown();
    await expect(shutdown.wait(500)).resolves.toMatchObject({ status: 'completed' });
    expect(inside).toMatchObject({ message: 'A lifecycle callback cannot wait for its own operation' });
  });

  it('resolves subscriber factories independently for each system-module installation', async () => {
    const config = new Config({
      sys_modules: {
        enabled: true,
        events: { enabled: true, subscribers: [{ type: 'sample-subscriber' }] },
      },
    });
    const deliveries: string[] = [];
    const runtime = (name: string) => new FullAPCore({
      config,
      plugins: [createSysModulesPlugin({
        subscriberFactories: {
          'sample-subscriber': () => ({
            eventPattern: 'sample.notice',
            onEvent: () => { deliveries.push(name); },
          }),
        },
      })],
    });
    const first = runtime('first');
    const second = runtime('second');
    const event = createEvent('sample.notice', null, 'info', {});
    first.events!.emit(event);
    await first.events!.flush();
    expect(deliveries).toEqual(['first']);
    second.events!.emit(event);
    await second.events!.flush();
    expect(deliveries).toEqual(['first', 'second']);
    await Promise.all([first.shutdown(), second.shutdown()]);
  });

  it('keeps an in-memory core isolated alongside the full profile', async () => {
    const root = await mkdtemp(join(tmpdir(), 'apcore-runtime-profiles-'));
    try {
      await writeFile(join(root, 'greeter.js'), moduleSource);
      const aclPath = join(root, 'global_acl.yaml');
      await writeFile(aclPath, 'default_effect: deny\nrules: []\n');
      const config = new Config({ extensions: { root }, acl: { root: aclPath } });
      const full = new FullAPCore({ config });
      const slim = new CoreAPCore();
      expect(slim.registry).toBeInstanceOf(CoreRegistry);
      await expect(full.discover()).resolves.toBe(1);
      expect(full.executor.governanceState().aclConfigured).toBe(true);
      await expect(slim.discover()).rejects.toThrow(/filesystem registry operations/i);
      expect(() => CoreACL.load(aclPath)).toThrow(/config subpath/i);
      const fullAcl = FullACL.load(aclPath);
      const explicitAcl = loadAclFile(aclPath);
      expect(fullAcl).toBeInstanceOf(FullACL);
      expect(fullAcl.check('sample.caller', 'sample.module')).toBe(false);
      await writeFile(aclPath, 'default_effect: allow\nrules: []\n');
      fullAcl.reload();
      explicitAcl.reload();
      expect(fullAcl.check('sample.caller', 'sample.module')).toBe(true);
      expect(explicitAcl.check('sample.caller', 'sample.module')).toBe(true);
      expect(() => new CoreAPCore({ config })).toThrow(/ACL discovery plugin/i);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

  it('generates an RFC 9562 UUID with Web Crypto entropy before randomUUID support', () => {
    vi.stubGlobal('crypto', {
      getRandomValues: (bytes: Uint8Array) => bytes.fill(0x56),
    });
    try {
      expect(uuidV4()).toBe('56565656-5656-4656-9656-565656565656');
    } finally {
      vi.unstubAllGlobals();
    }
  });
});
