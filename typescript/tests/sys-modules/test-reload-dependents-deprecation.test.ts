/**
 * D-121 — `reload_dependents` is deprecated for removal.
 *
 * Declared in all three SDKs' input schemas and read by none: a spec MUST
 * ("also reload modules that depend on matched modules") that no implementation
 * satisfied. That is the §9.1.3 "declared surface reaches no mechanism" shape
 * the spec forbids for configuration keys, here applied to a module INPUT
 * FIELD, and three independent implementations skipping it is the evidence the
 * maintainer decision rests on — deprecate now, remove at 2.0, do NOT
 * implement.
 *
 * Deprecated rather than removed today because the input schema sets
 * `additionalProperties: false`: at 2.0 the same call stops being a silent
 * no-op and becomes a VALIDATION ERROR, so a caller passing it needs a release
 * in which they are told.
 */

import { describe, it, expect, vi, afterEach } from 'vitest';
import { ReloadModule } from '../../src/sys-modules/control.js';
import { Registry } from '../../src/registry/registry.js';
import { EventEmitter } from '../../src/events/emitter.js';

function reloadModule(): ReloadModule {
  return new ReloadModule(new Registry(), new EventEmitter());
}

/** Reach the private warner the way the executor's `execute` does. */
function warn(mod: ReloadModule, inputs: Record<string, unknown>): void {
  (mod as unknown as { _warnReloadDependents(i: Record<string, unknown>): void })._warnReloadDependents(
    inputs,
  );
}

function countWarnings(f: () => void): string[] {
  const spy = vi.spyOn(console, 'warn').mockImplementation(() => {});
  try {
    f();
    return spy.mock.calls
      .map((c) => String(c[0]))
      .filter((m) => m.includes('reload_dependents'));
  } finally {
    spy.mockRestore();
  }
}

describe('D-121: reload_dependents is deprecated', () => {
  afterEach(() => vi.restoreAllMocks());

  it('passing it warns', () => {
    expect(countWarnings(() => warn(reloadModule(), { reload_dependents: true }))).toHaveLength(1);
  });

  it('the warning names the replacement and the removal', () => {
    // A deprecation notice that does not say what to do instead, or when the
    // field stops being ignored, leaves the caller to discover both.
    const [message] = countWarnings(() => warn(reloadModule(), { reload_dependents: true }));
    expect(message).toContain('path_filter');
    expect(message).toContain('2.0');
    expect(message).toContain('validation error');
  });

  it('it warns once per instance', () => {
    // `reload` is called by hot-reload loops and watchers, so an advisory whose
    // volume is proportional to traffic is one operators learn to filter out —
    // the cadence D-89 settled.
    const mod = reloadModule();
    expect(
      countWarnings(() => {
        for (let i = 0; i < 5; i++) warn(mod, { reload_dependents: true });
      }),
    ).toHaveLength(1);
  });

  it('the cadence is per instance, not per process', () => {
    // A process-wide one-shot tells the FIRST caller and leaves every later one
    // to discover it in production — the reason D-90's notice is per token and
    // D-89's dedupe is per registry instance.
    expect(
      countWarnings(() => {
        warn(reloadModule(), { reload_dependents: true });
        warn(reloadModule(), { reload_dependents: true });
      }),
    ).toHaveLength(2);
  });

  it.each([[{}], [{ reload_dependents: false }]])(
    'control: omitting it or passing false says nothing (%o)',
    (inputs) => {
      // Without this, "it warns" is also satisfied by warning on every reload,
      // which is noise for the ordinary call the method exists for.
      expect(countWarnings(() => warn(reloadModule(), inputs))).toHaveLength(0);
    },
  );

  it('the schema marks it deprecated and says what replaces it', () => {
    // The notice has to be readable WITHOUT triggering it: a host reading the
    // input schema — which is what `describe` / `getDefinition` hand an agent —
    // must see the deprecation without having to call with the field.
    const field = (
      reloadModule().inputSchema.properties as Record<string, Record<string, unknown>>
    )['reload_dependents'];
    expect(field['deprecated']).toBe(true);
    expect(String(field['description'])).toContain('path_filter');
    expect(String(field['description'])).toContain('2.0');
  });
});
