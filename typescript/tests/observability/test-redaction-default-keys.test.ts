/**
 * Issue #45 §3 — DEFAULT_REDACTION_FIELD_PATTERNS must match the Python
 * canonical superset (15 entries). Python is authoritative because broader
 * default redaction is safer than narrower.
 *
 * Canonical PY list:
 *   _secret_*, password, passwd, secret, token, api_key, apikey, apiKey,
 *   access_key, private_key, authorization, auth, credential, cookie,
 *   session, bearer
 *
 * Note: 16 strings above but `apikey` and `apiKey` are matched
 * case-insensitively in Python; in TypeScript we keep both spellings as
 * separate entries because matchPattern is case-sensitive — the same
 * coverage with explicit camelCase parity. Final TS count: 16 if
 * `apikey` is also kept; the spec says "15 entries with apiKey camelCase
 * parity", treating `apikey`/`apiKey` as one logical sensitive key but
 * two TS patterns. We assert the full superset is present.
 */
import { describe, it, expect } from 'vitest';
import {
  DEFAULT_REDACTION_FIELD_PATTERNS,
  RedactionConfig,
} from '../../src/observability/context-logger.js';
import { Config } from '../../src/config.js';

describe('DEFAULT_REDACTION_FIELD_PATTERNS canonical superset (#45 §3)', () => {
  const expected = [
    '_secret_*',
    'password',
    'passwd',
    'secret',
    'token',
    'api_key',
    'apikey',
    'apiKey',
    'access_key',
    'private_key',
    'authorization',
    'auth',
    'credential',
    'cookie',
    'session',
    'bearer',
  ];

  it('contains every entry from the Python canonical set', () => {
    for (const key of expected) {
      expect(DEFAULT_REDACTION_FIELD_PATTERNS).toContain(key);
    }
  });

  it('keeps apiKey camelCase parity alongside api_key/apikey', () => {
    expect(DEFAULT_REDACTION_FIELD_PATTERNS).toContain('apiKey');
    expect(DEFAULT_REDACTION_FIELD_PATTERNS).toContain('api_key');
    expect(DEFAULT_REDACTION_FIELD_PATTERNS).toContain('apikey');
  });

  it('default config redacts every canonical sensitive field', () => {
    const rc = RedactionConfig.fromConfig(new Config());

    const sample: Record<string, unknown> = {
      _secret_x: 'a',
      password: 'b',
      passwd: 'c',
      secret: 'd',
      token: 'e',
      api_key: 'f',
      apikey: 'g',
      apiKey: 'h',
      access_key: 'i',
      private_key: 'j',
      authorization: 'k',
      auth: 'l',
      credential: 'm',
      cookie: 'n',
      session: 'o',
      bearer: 'p',
      visible: 'ok',
    };

    const result = rc.apply(sample);
    for (const key of Object.keys(sample)) {
      if (key === 'visible') {
        expect(result[key]).toBe('ok');
      } else {
        expect(result[key]).toBe('***REDACTED***');
      }
    }
  });
});

// ---------------------------------------------------------------------------
// An explicitly-empty sensitive_keys REPLACES the default (Issue #34 §4, D-54)
// ---------------------------------------------------------------------------

describe('RedactionConfig.fromConfig — absent vs. explicitly empty sensitive_keys', () => {
  // docs/features/observability.md "Canonical default `sensitive_keys`":
  // "Implementations MUST allow operators to fully override the default by
  // setting `obs.redaction.sensitive_keys` in `apcore.yaml` (the override
  // REPLACES the default; it does not merge)." An empty override is still an
  // override, so it means "no key-based redaction" — not "unset".
  //
  // These keys are chosen so the assertion DISCRIMINATES: `password` and
  // `_secret_token` both match the shipped defaults, so they come through
  // plain only if the empty list was honoured. The previous implementation
  // fell back on `rawFields.length > 0`, and reverting the fix was verified to
  // leave the entire suite green — nothing pinned it.
  const discriminating = { password: 'hunter2', _secret_token: 'abc', username: 'alice' };

  function configWith(sensitiveKeys: unknown): Config {
    return new Config({
      version: '1.0',
      project: { name: 'redaction-test' },
      obs: { redaction: { sensitive_keys: sensitiveKeys } },
    } as never);
  }

  it('an ABSENT key falls back to the shipped defaults', () => {
    const rc = RedactionConfig.fromConfig(new Config());
    expect(rc.fieldPatterns).toEqual([...DEFAULT_REDACTION_FIELD_PATTERNS]);
    expect(rc.apply({ ...discriminating })).toEqual({
      password: '***REDACTED***',
      _secret_token: '***REDACTED***',
      username: 'alice',
    });
  });

  it('an explicit NULL falls back to the shipped defaults (matches apcore-python)', () => {
    const rc = RedactionConfig.fromConfig(configWith(null));
    expect(rc.fieldPatterns).toEqual([...DEFAULT_REDACTION_FIELD_PATTERNS]);
    expect(rc.apply({ ...discriminating })['password']).toBe('***REDACTED***');
  });

  it('an explicitly EMPTY list disables key-based redaction entirely', () => {
    const rc = RedactionConfig.fromConfig(configWith([]));
    expect(rc.fieldPatterns).toEqual([]);
    // Nothing is redacted by NAME — including `_secret_*`, which is entry [0]
    // of the default list and not a separate hard-coded rule.
    expect(rc.apply({ ...discriminating })).toEqual(discriminating);
  });

  it('a non-empty override replaces rather than merges with the default', () => {
    const rc = RedactionConfig.fromConfig(configWith(['username']));
    expect(rc.fieldPatterns).toEqual(['username']);
    expect(rc.apply({ ...discriminating })).toEqual({
      // `password` is in the DEFAULT list; a replacing override drops it.
      password: 'hunter2',
      _secret_token: 'abc',
      username: '***REDACTED***',
    });
  });

  it('the value-regex rule is independent of the key list and survives an empty one', () => {
    const rc = RedactionConfig.fromConfig(
      new Config({
        version: '1.0',
        project: { name: 'redaction-test' },
        obs: { redaction: { sensitive_keys: [], regex_patterns: ['^sk-[A-Za-z0-9]+$'] } },
      } as never),
    );
    expect(rc.fieldPatterns).toEqual([]);
    expect(rc.apply({ anything: 'sk-abc123', plain: 'hello' })).toEqual({
      anything: '***REDACTED***',
      plain: 'hello',
    });
  });
});
