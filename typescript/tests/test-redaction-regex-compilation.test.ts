/**
 * PROTOCOL_SPEC §10.6.1 requirements 2, 4 and 5 — what `regex_patterns` is
 * matched against, where it is compiled, and how long the diagnostic for a
 * broken entry may stay suppressed.
 *
 * Each of these three has a failure mode that looks like success from the
 * outside, so each case here asserts the half an implementation could skip:
 *
 * - Requirement 2: a stringifying implementation redacts MORE, so it passes any
 *   test that only checks a secret was caught. The discriminating assertion is
 *   that an ordinary number was left alone — and, in the other direction, that
 *   declining to stringify a container did not turn into skipping what is
 *   inside it.
 * - Requirement 4: swallowing the compile error produces a rule that redacts
 *   nothing, indistinguishable from one that matches nothing.
 * - Requirement 5: a module-global "already reported" set makes the FIRST
 *   configuration warn, so only a SECOND configuration can see the bug.
 */

import { describe, it, expect, vi, afterEach } from 'vitest';

import { RedactionConfig } from '../src/observability/context-logger.js';

const BAD = '[invalid';

afterEach(() => {
  vi.restoreAllMocks();
});

describe('§10.6.1 requirement 2 — string values only', () => {
  const config = () =>
    new RedactionConfig({ fieldPatterns: [], valuePatterns: ['[0-9]+', 'true', 'a'] });

  it('leaves a number alone', () => {
    // `[0-9]+` matches "42", so a stringifying implementation redacts this.
    // The damage there is not a leak but its opposite: ordinary numeric
    // telemetry vanishing from logs.
    expect(config().redact({ amount: 42 })).toEqual({ amount: 42 });
  });

  it('leaves a boolean alone', () => {
    // Python renders `True` and this runtime renders `true`, against a pattern
    // spelled `true` — so a stringifying Python matches nothing here and a
    // stringifying TypeScript matches. One value, two answers, which is why
    // requirement 2 forbids the conversion instead of defining it.
    expect(config().redact({ flag: true })).toEqual({ flag: true });
  });

  it('leaves a container alone rather than matching its rendering', () => {
    // `String({a: 1})` is `[object Object]` here and `{'a': 1}` in Python; the
    // pattern `a` matches the second and not the first.
    const payload = { mapping: { a: 1 }, listing: [1, 2] };
    expect(config().redact(payload)).toEqual(payload);
  });

  it('still matches a string', () => {
    expect(config().redact({ text: 'order 42' })).toEqual({ text: '***REDACTED***' });
  });

  it('reaches a string inside an object', () => {
    const rc = new RedactionConfig({ fieldPatterns: [], valuePatterns: ['sk-'] });
    expect(rc.redact({ nested: { key: 'sk-secret', count: 7 } })).toEqual({
      nested: { key: '***REDACTED***', count: 7 },
    });
  });

  it('reaches a string inside an ARRAY', () => {
    // The half this SDK was missing. `redact` handed an array element straight
    // back to itself, and the value rule is only ever consulted from
    // `_shouldRedact`, which needs a field name an element does not have — so a
    // secret in a list came back in plaintext here while apcore-python
    // (`_redact_in_list`) and apcore-rust (`redact_inner(item, None)`) both
    // replaced it. Requirement 2's "containers are descended into" is the same
    // clause that forbids the stringification: declining to convert a container
    // must not become skipping what is inside it.
    const rc = new RedactionConfig({ fieldPatterns: [], valuePatterns: ['sk-'] });
    expect(rc.redact({ items: ['sk-secret', 7] })).toEqual({ items: ['***REDACTED***', 7] });
  });
});

describe('§10.6.1 requirement 5 — compiled once, at the configuration read', () => {
  it('compiles constructor strings, so nothing is compiled per record', () => {
    const rc = new RedactionConfig({ valuePatterns: ['^Bearer\\s', 'sk-'] });
    expect(rc.valuePatterns.map((r) => r.source)).toEqual(['^Bearer\\s', 'sk-']);
    expect(rc.valuePatterns.every((r) => r.flags.includes('i'))).toBe(true);
  });

  it('passes an already-compiled pattern through', () => {
    const compiled = /sk-/i;
    expect(new RedactionConfig({ valuePatterns: [compiled] }).valuePatterns).toEqual([compiled]);
  });

  it('drops an empty entry rather than letting it match everything', () => {
    expect(new RedactionConfig({ valuePatterns: ['', 'sk-'] }).valuePatterns.length).toBe(1);
  });
});

describe('§10.6.1 requirement 4 — an uncompilable entry is reported', () => {
  it('names the pattern at construction', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const rc = new RedactionConfig({ valuePatterns: [BAD] });
    expect(warn).toHaveBeenCalledTimes(1);
    expect(String(warn.mock.calls[0]?.[0])).toContain(BAD);
    expect(rc.valuePatterns).toEqual([]);
    expect(rc.invalidValuePatterns.map(([p]) => p)).toEqual([BAD]);
  });

  it('tells the SECOND configuration too', () => {
    // The requirement-5 bound, and the one a single-configuration test cannot
    // see. The suppression used to be a module-level Set that was never
    // cleared, so the second deployment to load the same broken pattern got
    // silence — the reload case and the multi-tenant case, which are the two
    // where an operator most needs telling.
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    new RedactionConfig({ valuePatterns: [BAD] });
    warn.mockClear();
    new RedactionConfig({ valuePatterns: [BAD] });
    expect(warn).toHaveBeenCalledTimes(1);
  });

  it('still reports each entry of ONE configuration only once', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const rc = new RedactionConfig({ fieldPatterns: [], valuePatterns: [BAD] });
    for (let i = 0; i < 5; i += 1) rc.redact({ a: 'x', b: 'y' });
    expect(warn).toHaveBeenCalledTimes(1);
  });
});
