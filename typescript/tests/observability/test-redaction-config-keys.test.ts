/**
 * Sync finding (CRITICAL #4) — RedactionConfig key alignment with PY/Rust spec.
 *
 * Canonical keys:
 *   - `obs.redaction.regex_patterns`    (was: observability.redaction.value_patterns)
 *   - `obs.redaction.sensitive_keys`    (was: observability.redaction.field_patterns)
 *   - `obs.redaction.replacement`       (was: observability.redaction.replacement)
 *
 * Legacy keys must still work, but reading them must emit a one-shot
 * deprecation warning so consumers know to migrate.
 */
import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from 'vitest';
import { Config } from '../../src/config.js';
import { RedactionConfig } from '../../src/observability/context-logger.js';

describe('RedactionConfig key alignment (CRITICAL #4)', () => {
  let warnSpy: MockInstance<typeof console.warn>;

  beforeEach(() => {
    warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
  });

  afterEach(() => {
    warnSpy.mockRestore();
  });

  it('reads canonical obs.redaction.* keys (sensitive_keys, regex_patterns, replacement)', () => {
    const config = new Config();
    config.set('obs.redaction.sensitive_keys', ['_secret_*', 'apiKey', 'authorization']);
    config.set('obs.redaction.regex_patterns', ['^sk-[A-Za-z0-9]+$']);
    config.set('obs.redaction.replacement', '<HIDDEN>');

    const rc = RedactionConfig.fromConfig(config);
    expect(rc.fieldPatterns).toContain('_secret_*');
    expect(rc.fieldPatterns).toContain('apiKey');
    expect(rc.fieldPatterns).toContain('authorization');
    expect(rc.replacement).toBe('<HIDDEN>');

    const result = rc.apply({ apiKey: 'k', _secret_x: 'y', token: 'sk-abc123', visible: 'ok' });
    expect(result.apiKey).toBe('<HIDDEN>');
    expect(result._secret_x).toBe('<HIDDEN>');
    // regex_patterns matches the value
    expect(result.token).toBe('<HIDDEN>');
    expect(result.visible).toBe('ok');
  });

  it('default sensitive_keys mirror PY/Rust ["_secret_*", "apiKey", "api_key", "token", "authorization", "password"]', () => {
    const config = new Config();
    const rc = RedactionConfig.fromConfig(config);

    const result = rc.apply({
      _secret_password: 'pw',
      apiKey: 'k',
      api_key: 'k2',
      token: 't',
      authorization: 'Bearer x',
      password: 'p',
      visible: 'ok',
    });
    expect(result._secret_password).toBe('***REDACTED***');
    expect(result.apiKey).toBe('***REDACTED***');
    expect(result.api_key).toBe('***REDACTED***');
    expect(result.token).toBe('***REDACTED***');
    expect(result.authorization).toBe('***REDACTED***');
    expect(result.password).toBe('***REDACTED***');
    expect(result.visible).toBe('ok');
  });

  it('legacy observability.redaction.field_patterns still honored (backwards-compat)', () => {
    const config = new Config();
    // CORRECTED (redaction_config.json `discriminating_payload`): this case
    // used to configure ['legacy_secret', 'apiKey'] and assert both were
    // redacted. Both match a SHIPPED DEFAULT anyway (`legacy_secret` contains
    // `secret`; `apiKey` is a default), so it stayed green whether the legacy
    // key was read or silently ignored — the exact blind spot that let
    // apcore-rust ship a redaction config nobody read (apcore-rust#32).
    // `username` matches no default, and `password` matches one, so the two
    // assertions below now prove the legacy key was READ and that its value
    // REPLACED the defaults.
    config.set('observability.redaction.field_patterns', ['username']);

    const rc = RedactionConfig.fromConfig(config);
    const result = rc.apply({ username: 'alice', password: 'hunter2', other: 'ok' });
    expect(result.username).toBe('***REDACTED***'); // only the legacy key can do this
    expect(result.password).toBe('hunter2'); // ...and it replaced the defaults
    expect(result.other).toBe('ok');
  });

  it('legacy observability.redaction.value_patterns still honored', () => {
    const config = new Config();
    config.set('observability.redaction.value_patterns', ['^sk-[A-Za-z0-9]+$']);

    const rc = RedactionConfig.fromConfig(config);
    // CORRECTED, same reason: the payload key used to be `token`, a shipped
    // default field pattern, so it was redacted whether or not the legacy
    // value_patterns key was read. `username` matches no default, so only a
    // VALUE-pattern match can redact it.
    const result = rc.apply({ username: 'sk-abc123', other: 'plain' });
    expect(result.username).toBe('***REDACTED***');
    expect(result.other).toBe('plain');
  });

  it('emits a one-shot deprecation warning when legacy keys are read', async () => {
    // Use vi.resetModules() + dynamic import to get a fresh module state so
    // the one-shot bookkeeping is reset for this test.
    vi.resetModules();
    const mod = await import('../../src/observability/context-logger.js');
    const cfgMod = await import('../../src/config.js');

    const config1 = new cfgMod.Config();
    config1.set('observability.redaction.field_patterns', ['legacy_secret']);
    mod.RedactionConfig.fromConfig(config1);

    const config2 = new cfgMod.Config();
    config2.set('observability.redaction.value_patterns', ['legacy_value']);
    mod.RedactionConfig.fromConfig(config2);

    const config3 = new cfgMod.Config();
    config3.set('observability.redaction.field_patterns', ['x']);
    mod.RedactionConfig.fromConfig(config3);

    const deprecationCalls = warnSpy.mock.calls.filter((call) =>
      String(call[0] ?? '').includes('observability.redaction'),
    );
    // One-shot per process: only the first legacy read warns.
    expect(deprecationCalls.length).toBe(1);
    expect(String(deprecationCalls[0][0])).toContain('obs.redaction');
  });

  it('canonical keys take precedence over legacy keys when both are set', () => {
    const config = new Config();
    config.set('observability.redaction.field_patterns', ['legacy_only']);
    config.set('obs.redaction.sensitive_keys', ['canonical_only']);

    const rc = RedactionConfig.fromConfig(config);
    expect(rc.fieldPatterns).toContain('canonical_only');
    expect(rc.fieldPatterns).not.toContain('legacy_only');
  });
});
