import { describe, it, expect, afterEach, afterAll } from 'vitest';
import { writeFileSync, mkdtempSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { Type } from '@sinclair/typebox';
import { Config } from '../src/config.js';
import { Registry } from '../src/registry/registry.js';
import { Executor } from '../src/executor.js';
import {
  registerSysModules,
  registerSubscriberType,
  unregisterSubscriberType,
  resetSubscriberRegistry,
} from '../src/sys-modules/registration.js';
import { ToggleState } from '../src/sys-modules/toggle.js';

describe('registerSysModules', () => {
  it('returns empty context when sys_modules.enabled is false', () => {
    const registry = new Registry();
    const executor = new Executor({ registry });
    const config = new Config({ sys_modules: { enabled: false } });
    const ctx = registerSysModules(registry, executor, config);
    expect(ctx.errorHistory).toBeUndefined();
    expect(ctx.eventEmitter).toBeUndefined();
  });

  it('registers health modules when enabled', () => {
    const registry = new Registry();
    const executor = new Executor({ registry });
    const config = new Config({ sys_modules: { enabled: true } });
    const ctx = registerSysModules(registry, executor, config);
    expect(ctx.errorHistory).toBeDefined();
    expect(registry.has('system.health.summary')).toBe(true);
    expect(registry.has('system.health.module')).toBe(true);
  });

  it('registers manifest modules when enabled', () => {
    const registry = new Registry();
    const executor = new Executor({ registry });
    const config = new Config({ sys_modules: { enabled: true } });
    registerSysModules(registry, executor, config);
    expect(registry.has('system.manifest.module')).toBe(true);
    expect(registry.has('system.manifest.full')).toBe(true);
  });

  it('registers usage modules when enabled', () => {
    const registry = new Registry();
    const executor = new Executor({ registry });
    const config = new Config({ sys_modules: { enabled: true } });
    const ctx = registerSysModules(registry, executor, config);
    expect(ctx.usageCollector).toBeDefined();
    expect(ctx.usageMiddleware).toBeDefined();
    expect(registry.has('system.usage.summary')).toBe(true);
    expect(registry.has('system.usage.module')).toBe(true);
  });

  it('registers control modules only when events are enabled', () => {
    const registry = new Registry();
    const executor = new Executor({ registry });
    const config = new Config({
      sys_modules: { enabled: true, events: { enabled: false } },
    });
    registerSysModules(registry, executor, config);
    expect(registry.has('system.control.toggle_feature')).toBe(false);
    expect(registry.has('system.control.update_config')).toBe(false);
    expect(registry.has('system.control.reload_module')).toBe(false);
  });

  it('registers control modules and PlatformNotifyMiddleware when events enabled', () => {
    const registry = new Registry();
    const executor = new Executor({ registry });
    const config = new Config({
      sys_modules: { enabled: true, events: { enabled: true } },
    });
    const ctx = registerSysModules(registry, executor, config);
    expect(ctx.eventEmitter).toBeDefined();
    expect(ctx.platformNotifyMiddleware).toBeDefined();
    expect(registry.has('system.control.toggle_feature')).toBe(true);
    expect(registry.has('system.control.update_config')).toBe(true);
    expect(registry.has('system.control.reload_module')).toBe(true);
  });

  it('calls system.health.summary successfully', async () => {
    const registry = new Registry();
    const executor = new Executor({ registry });
    const config = new Config({ sys_modules: { enabled: true } });
    registerSysModules(registry, executor, config);

    // Register a dummy module
    registry.registerInternal('test.mod', {
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ ok: Type.Boolean() }),
      description: 'test',
      execute: () => ({ ok: true }),
    });

    const result = await executor.call('system.health.summary', {});
    expect(result['summary']).toBeDefined();
    expect((result['summary'] as Record<string, unknown>)['total_modules']).toBeGreaterThan(0);
  });
});

describe('subscriber registry', () => {
  afterEach(() => {
    resetSubscriberRegistry();
  });

  it('registers and uses a subscriber type', () => {
    const factory = (cfg: Record<string, unknown>) => ({
      onEvent: () => { void cfg; },
    });
    registerSubscriberType('custom', factory);
    // No error means registration succeeded
  });

  it('unregisters a subscriber type', () => {
    registerSubscriberType('temp', () => ({ onEvent: () => {} }));
    unregisterSubscriberType('temp');
    expect(() => unregisterSubscriberType('temp')).toThrow('not registered');
  });

  it('throws when unregistering unknown type', () => {
    expect(() => unregisterSubscriberType('unknown')).toThrow('not registered');
  });

  it('resets custom types but preserves built-in types', () => {
    registerSubscriberType('custom1', () => ({ onEvent: () => {} }));
    registerSubscriberType('custom2', () => ({ onEvent: () => {} }));
    resetSubscriberRegistry();
    expect(() => unregisterSubscriberType('custom1')).toThrow('not registered');
    // Built-in types should survive reset
    unregisterSubscriberType('webhook');
    unregisterSubscriberType('a2a');
  });

  it('has built-in webhook and a2a types', () => {
    // Should not throw — they are pre-registered
    unregisterSubscriberType('webhook');
    unregisterSubscriberType('a2a');
  });
});

describe('toggle.* overrides are restored to ToggleState (sync finding A-D-013)', () => {
  // `system.control.toggle_feature` persists its decision as
  // `toggle.<module_id>: boolean` (toggle.ts `_persistToggleOverride`).
  // registerSysModules used to feed EVERY override key through `config.set()`,
  // so the restore became an inert config entry named `toggle.x` and a module
  // disabled before a restart came back ENABLED — on the approval-gated kill
  // switch. apcore-python and apcore-rust both strip the prefix and drive the
  // ToggleState.

  function makeStore(overrides: Record<string, unknown>) {
    return {
      load: () => overrides,
      save: () => {
        /* not exercised here */
      },
    };
  }

  it('restores a disabled module into the injected ToggleState', () => {
    const registry = new Registry();
    const executor = new Executor({ registry });
    const config = new Config({ sys_modules: { enabled: true } });
    const toggleState = new ToggleState();

    expect(toggleState.isDisabled('executor.email.send')).toBe(false);

    registerSysModules(registry, executor, config, undefined, {
      toggleState,
      overridesStore: makeStore({ 'toggle.executor.email.send': false }),
    });

    expect(toggleState.isDisabled('executor.email.send')).toBe(true);
  });

  it('restores an enabled module (true clears the disable)', () => {
    const registry = new Registry();
    const executor = new Executor({ registry });
    const config = new Config({ sys_modules: { enabled: true } });
    const toggleState = new ToggleState();
    toggleState.disable('executor.email.send');

    registerSysModules(registry, executor, config, undefined, {
      toggleState,
      overridesStore: makeStore({ 'toggle.executor.email.send': true }),
    });

    expect(toggleState.isDisabled('executor.email.send')).toBe(false);
  });

  it('does not leak the toggle key into Config', () => {
    const registry = new Registry();
    const executor = new Executor({ registry });
    const config = new Config({ sys_modules: { enabled: true } });
    const toggleState = new ToggleState();

    registerSysModules(registry, executor, config, undefined, {
      toggleState,
      overridesStore: makeStore({ 'toggle.executor.email.send': false }),
    });

    expect(config.get('toggle.executor.email.send')).toBeUndefined();
  });

  it('still applies non-toggle override keys to Config', () => {
    const registry = new Registry();
    const executor = new Executor({ registry });
    const config = new Config({ sys_modules: { enabled: true } });

    registerSysModules(registry, executor, config, undefined, {
      toggleState: new ToggleState(),
      overridesStore: makeStore({ 'executor.default_timeout': 1234 }),
    });

    expect(config.get('executor.default_timeout')).toBe(1234);
  });

  it('ignores a toggle key whose value is not a boolean', () => {
    const registry = new Registry();
    const executor = new Executor({ registry });
    const config = new Config({ sys_modules: { enabled: true } });
    const toggleState = new ToggleState();

    registerSysModules(registry, executor, config, undefined, {
      toggleState,
      overridesStore: makeStore({ 'toggle.executor.email.send': 'nope' }),
    });

    expect(toggleState.isDisabled('executor.email.send')).toBe(false);
    expect(config.get('toggle.executor.email.send')).toBeUndefined();
  });
});

describe('sys_modules activation is opt-in in namespace mode (sync finding B-012)', () => {
  // §6.6.3: "`sys_modules.enabled = false (default)` -> 0 modules registered.
  // Nothing to call, nothing to list." The registration defaults in §9.15.3
  // declared `enabled: True`, so in namespace mode a project that configured
  // nothing had the six read modules stood up for it. Asserted end-to-end here
  // rather than only on the config value, because the config value is one
  // refactor away from the behaviour it is supposed to guard.
  const tmp = mkdtempSync(join(tmpdir(), 'apcore-sysmod-'));
  afterAll(() => rmSync(tmp, { recursive: true, force: true }));

  function loadNamespaceConfig(body: string): Config {
    const p = join(tmp, `cfg-${Math.abs(hashCode(body))}.yaml`);
    writeFileSync(p, body);
    return Config.load(p, { validate: false });
  }
  function hashCode(s: string): number {
    let h = 0;
    for (let i = 0; i < s.length; i++) h = (Math.imul(31, h) + s.charCodeAt(i)) | 0;
    return h;
  }

  it('registers nothing when sys_modules is not configured', () => {
    const cfg = loadNamespaceConfig('apcore:\n  version: "1.0.0"\n');
    const registry = new Registry();
    registerSysModules(registry, new Executor({ registry }), cfg);
    expect(registry.moduleIds).toEqual([]);
  });

  it('registers the six read modules when explicitly enabled', () => {
    const cfg = loadNamespaceConfig('apcore:\n  version: "1.0.0"\nsys_modules:\n  enabled: true\n');
    const registry = new Registry();
    registerSysModules(registry, new Executor({ registry }), cfg);
    expect(registry.moduleIds).toHaveLength(6);
    expect(registry.moduleIds.some((id) => id.startsWith('system.control.'))).toBe(false);
  });

  it('adds the three control modules only when events are also enabled', () => {
    const cfg = loadNamespaceConfig(
      'apcore:\n  version: "1.0.0"\nsys_modules:\n  enabled: true\n  events:\n    enabled: true\n',
    );
    const registry = new Registry();
    registerSysModules(registry, new Executor({ registry }), cfg);
    expect(registry.moduleIds).toHaveLength(9);
    expect(registry.moduleIds.filter((id) => id.startsWith('system.control.'))).toHaveLength(3);
  });
});

describe('overrides_path is read from Config, not options alone (sync finding A-D-015)', () => {
  // apcore-python resolves `sys_modules.control.overrides_path` from Config
  // when no store/path is passed explicitly. This SDK read only
  // `options.overridesPath` — and `APCore` never passes it (client.ts installs
  // sys modules with `{ toggleState }` only), so the config key was the only
  // route a real deployment had, and it was not wired.
  const tmp2 = mkdtempSync(join(tmpdir(), 'apcore-ovr-'));
  afterAll(() => rmSync(tmp2, { recursive: true, force: true }));

  it('loads overrides named by the config key', () => {
    const overridesFile = join(tmp2, 'overrides.yaml');
    writeFileSync(overridesFile, 'executor.default_timeout: 4321\n');

    const config = new Config({
      sys_modules: { enabled: true, control: { overrides_path: overridesFile } },
    });
    const registry = new Registry();
    registerSysModules(registry, new Executor({ registry }), config);

    expect(config.get('executor.default_timeout')).toBe(4321);
  });

  it('an explicit option still wins over the config key', () => {
    const fromOption = join(tmp2, 'from-option.yaml');
    writeFileSync(fromOption, 'executor.default_timeout: 1111\n');
    const fromConfig = join(tmp2, 'from-config.yaml');
    writeFileSync(fromConfig, 'executor.default_timeout: 2222\n');

    const config = new Config({
      sys_modules: { enabled: true, control: { overrides_path: fromConfig } },
    });
    const registry = new Registry();
    registerSysModules(registry, new Executor({ registry }), config, undefined, {
      overridesPath: fromOption,
    });

    expect(config.get('executor.default_timeout')).toBe(1111);
  });
});
