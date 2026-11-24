/**
 * Issue #43 §5 — Configurable redaction.
 *
 * Verifies RedactionConfig can be built from a Config namespace, supports
 * env-driven overrides, and applies the configured patterns at runtime.
 */
import { describe, expect, it } from 'vitest';
import { Config } from '../../src/config.js';
import { RedactionConfig } from '../../src/observability/context-logger.js';

describe('RedactionConfig.fromConfig', () => {
  it('reads field_patterns and value_patterns from observability namespace', () => {
    const config = new Config();
    config.set('observability.redaction.field_patterns', ['_secret_*', 'apiKey', 'authorization']);
    config.set('observability.redaction.value_patterns', ['^sk-[A-Za-z0-9]+$']);
    config.set('observability.redaction.replacement', '<HIDDEN>');

    const rc = RedactionConfig.fromConfig(config);
    expect(rc.fieldPatterns).toContain('_secret_*');
    expect(rc.fieldPatterns).toContain('apiKey');
    expect(rc.fieldPatterns).toContain('authorization');
    expect(rc.replacement).toBe('<HIDDEN>');

    const result = rc.apply({ apiKey: 'live-1234', name: 'ok', _secret_token: 'tk' });
    expect(result.apiKey).toBe('<HIDDEN>');
    expect(result._secret_token).toBe('<HIDDEN>');
    expect(result.name).toBe('ok');
  });

  it('redacts string values matching configured value patterns', () => {
    const config = new Config();
    config.set('observability.redaction.field_patterns', []);
    config.set('observability.redaction.value_patterns', ['^sk-[A-Za-z0-9]+$']);

    const rc = RedactionConfig.fromConfig(config);
    const result = rc.apply({ token: 'sk-abcdef123', other: 'plain' });
    expect(result.token).toBe('***REDACTED***');
    expect(result.other).toBe('plain');
  });

  it('applies sensible defaults when nothing is configured', () => {
    const config = new Config();
    const rc = RedactionConfig.fromConfig(config);

    // Defaults should at minimum cover legacy `_secret_*` plus standard
    // sensitive header/body keys.
    const result = rc.apply({
      _secret_password: 'pw',
      apiKey: 'k',
      token: 't',
      authorization: 'Bearer x',
      password: 'p',
      keep_me: 'visible',
    });
    expect(result._secret_password).toBe('***REDACTED***');
    expect(result.apiKey).toBe('***REDACTED***');
    expect(result.token).toBe('***REDACTED***');
    expect(result.authorization).toBe('***REDACTED***');
    expect(result.password).toBe('***REDACTED***');
    expect(result.keep_me).toBe('visible');
  });

  it('compiles string value patterns case-insensitively', () => {
    const config = new Config();
    config.set('observability.redaction.value_patterns', ['bearer\\s+\\S+']);

    const rc = RedactionConfig.fromConfig(config);
    const result = rc.apply({ header: 'BEARER abc.def.ghi' });
    expect(result.header).toBe('***REDACTED***');
  });
});
