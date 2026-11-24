/**
 * SYS-2 — `sys_modules.control.overrides_path` must persist a toggle, not
 * only restore one.
 *
 * `registerSysModules` read the path for the RESTORE side but never turned it
 * into an `OverridesStore`, and `ToggleFeatureModule` persists only through a
 * store — it has no path field. So a deployment that set the config key and
 * passed no programmatic store got a `toggle_feature` call that wrote nothing,
 * and the module came back enabled after a restart. apcore-python
 * (`registration.py:674`) and apcore-rust (`mod.rs:706`) both pass the path
 * through to the toggle module.
 */

import { describe, it, expect, afterAll, vi } from 'vitest';
import { mkdtempSync, rmSync, existsSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import * as yaml from 'js-yaml';
import { Type } from '@sinclair/typebox';
import { Config } from '../../src/config.js';
import { Registry } from '../../src/registry/registry.js';
import { Executor } from '../../src/executor.js';
import { registerSysModules } from '../../src/sys-modules/registration.js';
import { ToggleState } from '../../src/sys-modules/toggle.js';

const tmp = mkdtempSync(join(tmpdir(), 'apcore-ovr-rt-'));
afterAll(() => rmSync(tmp, { recursive: true, force: true }));

const DUMMY = {
  inputSchema: Type.Object({}),
  outputSchema: Type.Object({ ok: Type.Boolean() }),
  description: 'dummy target for the toggle',
  execute: () => ({ ok: true }),
};

function boot(overridesPath: string, toggleState: ToggleState) {
  const registry = new Registry();
  const executor = new Executor({ registry });
  const config = new Config({
    sys_modules: {
      enabled: true,
      events: { enabled: true },
      control: { enabled: true, overrides_path: overridesPath },
    },
  });
  registerSysModules(registry, executor, config, undefined, { toggleState });
  registry.registerInternal('executor.email.send', DUMMY);
  return { registry, executor, config };
}

describe('sys_modules.control.overrides_path round trip (SYS-2)', () => {
  it('a toggle_feature disable is written to the configured overrides file', async () => {
    const overridesPath = join(tmp, 'write.yaml');
    const { executor } = boot(overridesPath, new ToggleState());

    await executor.call('system.control.toggle_feature', {
      module_id: 'executor.email.send',
      enabled: false,
      reason: 'SYS-2 regression',
    });

    expect(existsSync(overridesPath)).toBe(true);
    const parsed = yaml.load(readFileSync(overridesPath, 'utf-8')) as Record<string, unknown>;
    expect(parsed['toggle.executor.email.send']).toBe(false);
  });

  it('survives a restart: the disable is restored into a fresh ToggleState', async () => {
    const overridesPath = join(tmp, 'restart.yaml');

    // --- process 1: disable ---
    const first = new ToggleState();
    const { executor } = boot(overridesPath, first);
    await executor.call('system.control.toggle_feature', {
      module_id: 'executor.email.send',
      enabled: false,
      reason: 'SYS-2 regression',
    });
    expect(first.isDisabled('executor.email.send')).toBe(true);

    // --- process 2: restart, same overrides file, brand-new state ---
    const second = new ToggleState();
    expect(second.isDisabled('executor.email.send')).toBe(false);
    boot(overridesPath, second);
    expect(second.isDisabled('executor.email.send')).toBe(true);
  });

  it('an explicit overridesStore still wins over the config path', async () => {
    const overridesPath = join(tmp, 'explicit.yaml');
    const saved: Record<string, unknown>[] = [];
    const store = {
      load: () => ({}),
      save: (o: Record<string, unknown>) => {
        saved.push({ ...o });
      },
    };

    const registry = new Registry();
    const executor = new Executor({ registry });
    const config = new Config({
      sys_modules: {
        enabled: true,
        events: { enabled: true },
        control: { enabled: true, overrides_path: overridesPath },
      },
    });
    registerSysModules(registry, executor, config, undefined, {
      toggleState: new ToggleState(),
      overridesStore: store,
    });
    registry.registerInternal('executor.email.send', DUMMY);

    await executor.call('system.control.toggle_feature', {
      module_id: 'executor.email.send',
      enabled: false,
      reason: 'SYS-2 regression',
    });

    expect(saved).toHaveLength(1);
    expect(saved[0]['toggle.executor.email.send']).toBe(false);
    // The synthesized file store must not also have been wired.
    expect(existsSync(overridesPath)).toBe(false);
  });
});

describe('restoring file overrides with an explicit store', () => {
  it('loads file defaults and applies the explicit store last', () => {
    const path = join(tmp, 'merge.yaml');
    writeFileSync(path, 'toggle.app.first: false\ntoggle.app.second: false\n');
    const toggleState = new ToggleState();
    const registry = new Registry();

    registerSysModules(registry, new Executor({ registry }), new Config({
      sys_modules: { enabled: true, control: { overrides_path: path } },
    }), undefined, {
      toggleState,
      overridesStore: { load: () => ({ 'toggle.app.second': true }), save: () => {} },
    });

    expect(toggleState.isDisabled('app.first')).toBe(true);
    expect(toggleState.isDisabled('app.second')).toBe(false);
  });

  it.each([
    { name: 'scalar', content: 'scalar', warning: false },
    { name: 'malformed', content: 'broken: [', warning: true },
  ])('restores the explicit store when the override file is $name', ({ name, content, warning }) => {
    const path = join(tmp, `${name}.yaml`);
    writeFileSync(path, content);
    const toggleState = new ToggleState();
    const registry = new Registry();
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      registerSysModules(registry, new Executor({ registry }), new Config({
        sys_modules: { enabled: true, control: { overrides_path: path } },
      }), undefined, {
        toggleState,
        overridesStore: { load: () => ({ 'toggle.app.first': false }), save: () => {} },
      });

      expect(toggleState.isDisabled('app.first')).toBe(true);
      const reportedInvalidFile = warn.mock.calls.some(([message]) =>
        String(message).includes('Failed to load overrides file'),
      );
      expect(reportedInvalidFile).toBe(warning);
    } finally {
      warn.mockRestore();
    }
  });
});
