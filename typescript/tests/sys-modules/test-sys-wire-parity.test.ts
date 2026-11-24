/**
 * Four verified `system.*` divergences (SYS-5, SYS-8, SYS-13, SYS-19).
 *
 * SYS-5  — `system.manifest.*` emitted `annotations` with camelCase keys
 *          straight off the descriptor. apcore-python and apcore-rust emit
 *          snake_case, and the spec's own output example
 *          (system-modules.md:196-202) is snake_case. This repo already
 *          exports an `annotationsToJSON` serializer and never called it.
 * SYS-8  — `manifest.full` omitted `dependencies` from each entry even though
 *          the same file emits it at `manifest.module`. `sys-manifest-full
 *          .schema.json` `$ref`s `sys-manifest-module.schema.json`, so the two
 *          entry shapes have to match.
 * SYS-13 — `system.control.update_config` masked a sensitive value with
 *          `'***'` where the peers use the canonical `'***REDACTED***'`.
 * SYS-19 — all three `system.control.*` modules declared `idempotent: true`.
 *          The spec is per-module: update_config false, reload_module false,
 *          toggle_feature true. Middleware that retries or dedupes on
 *          `idempotent` would retry a config WRITE.
 */

import { describe, it, expect } from 'vitest';
import { Type } from '@sinclair/typebox';
import { Registry } from '../../src/registry/registry.js';
import { Config } from '../../src/config.js';
import { EventEmitter } from '../../src/events/emitter.js';
import { ManifestModule, ManifestFullModule } from '../../src/sys-modules/manifest.js';
import { UpdateConfigModule, ReloadModule } from '../../src/sys-modules/control.js';
import { ToggleFeatureModule } from '../../src/sys-modules/toggle.js';
import { REDACTED_VALUE } from '../../src/executor.js';

function registryWithModule(): Registry {
  const registry = new Registry();
  registry.registerInternal('executor.email.send', {
    inputSchema: Type.Object({}),
    outputSchema: Type.Object({ ok: Type.Boolean() }),
    description: 'send an email',
    annotations: { requiresApproval: true, openWorld: false, cacheTtl: 30 },
    dependencies: [{ module_id: 'common.smtp', version: '1.0.0' }],
    execute: () => ({ ok: true }),
  });
  return registry;
}

describe('manifest annotations are emitted on the wire spelling (SYS-5)', () => {
  it('system.manifest.module emits snake_case annotation keys', () => {
    const mod = new ManifestModule(registryWithModule(), new Config({}));
    const out = mod.execute({ module_id: 'executor.email.send' }, null);
    const annotations = out['annotations'] as Record<string, unknown>;

    expect(annotations['requires_approval']).toBe(true);
    expect(annotations['open_world']).toBe(false);
    expect(annotations['cache_ttl']).toBe(30);
    expect(annotations['requiresApproval']).toBeUndefined();
    expect(annotations['openWorld']).toBeUndefined();
    expect(annotations['cacheTtl']).toBeUndefined();
  });

  it('system.manifest.full emits snake_case annotation keys too', () => {
    const mod = new ManifestFullModule(registryWithModule(), new Config({}));
    const out = mod.execute({}, null);
    const entry = (out['modules'] as Record<string, unknown>[])[0];
    const annotations = entry['annotations'] as Record<string, unknown>;

    expect(annotations['requires_approval']).toBe(true);
    expect(annotations['requiresApproval']).toBeUndefined();
  });
});

describe('manifest.full entries carry dependencies (SYS-8)', () => {
  it('a full entry has the same shape as a module entry', () => {
    const registry = registryWithModule();
    const single = new ManifestModule(registry, new Config({})).execute(
      { module_id: 'executor.email.send' },
      null,
    );
    const full = new ManifestFullModule(registry, new Config({})).execute({}, null);
    const entry = (full['modules'] as Record<string, unknown>[]).find(
      (m) => m['module_id'] === 'executor.email.send',
    )!;

    expect(entry['dependencies']).toBeDefined();
    expect(entry['dependencies']).toEqual(single['dependencies']);
  });
});

describe('update_config masks with the canonical redaction constant (SYS-13)', () => {
  it('a sensitive key is masked with ***REDACTED***', () => {
    const config = new Config({ service: { api_key: 'old-secret' } });
    const mod = new UpdateConfigModule(config, new EventEmitter());
    const out = mod.execute(
      { key: 'service.api_key', value: 'new-secret', reason: 'rotation' },
      null,
    );

    expect(out['old_value']).toBe(REDACTED_VALUE);
    expect(out['new_value']).toBe(REDACTED_VALUE);
    expect(REDACTED_VALUE).toBe('***REDACTED***');
  });

  it('a non-sensitive key is not masked', () => {
    const config = new Config({ some: { name: 'value' } });
    const mod = new UpdateConfigModule(config, new EventEmitter());
    const out = mod.execute({ key: 'some.name', value: 'updated', reason: 'test' }, null);
    expect(out['new_value']).toBe('updated');
  });
});

describe('system.control.* idempotence is per module (SYS-19)', () => {
  it('update_config is NOT idempotent', () => {
    const mod = new UpdateConfigModule(new Config({}), new EventEmitter());
    expect(mod.annotations.idempotent).toBe(false);
  });

  it('reload_module is NOT idempotent', () => {
    const mod = new ReloadModule(new Registry(), new EventEmitter());
    expect(mod.annotations.idempotent).toBe(false);
  });

  it('toggle_feature IS idempotent', () => {
    const mod = new ToggleFeatureModule(new Registry(), new EventEmitter());
    expect(mod.annotations.idempotent).toBe(true);
  });
});
