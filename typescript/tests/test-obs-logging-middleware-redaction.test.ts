/**
 * The redaction contract at the surface an operator actually configures:
 * `ObsLoggingMiddleware`, end to end, asserting the LOG LINE rather than the
 * helper the log line happens to call.
 *
 * The distinction is the whole point of this file. `RedactionConfig.redact()`
 * was correct for nested objects and (after the array fix) for arrays, and its
 * unit tests were green — while the wiring observability.md documents still
 * wrote secrets to the log, because two different rule sets were applied to two
 * different parts of one record:
 *
 *   - the middleware redacts `inputs` / `output` with the caller's config,
 *     through the FLAT `apply()`;
 *   - `ContextLogger._emit` then redacts the WHOLE `extra` recursively — and
 *     did so under `RedactionConfig.default()`, which has no `regex_patterns`.
 *
 * So a test that exercises `redact()` proves nothing about what is logged.
 * Every case here goes through `mw.before` / `mw.after` and reads the emitted
 * JSON.
 */

import { describe, it, expect } from 'vitest';

import {
  ContextLogger,
  ObsLoggingMiddleware,
  RedactionConfig,
} from '../src/observability/context-logger.js';
import { Context } from '../src/context.js';

const SECRET = 'sk-abcdef123456';

function harness(config: RedactionConfig | null) {
  const lines: string[] = [];
  const logger = new ContextLogger({
    name: 'test',
    output: { write: (s: string) => lines.push(s) },
  });
  const mw = new ObsLoggingMiddleware({ logger, redactionConfig: config });
  const emitted = () => lines.map((l) => JSON.parse(l) as Record<string, unknown>);
  return { mw, emitted };
}

function ctx() {
  return Context.create() as never;
}

/**
 * A context carrying what the executor's capture point would have filled.
 * The middleware logs ONLY these captured values, never the raw arguments
 * (PROTOCOL_SPEC 10.6.1 requirement 5, D-131), so a record-level test has to
 * put the payload where the pipeline puts it.
 */
function capturedCtx(captured: {
  inputs: Record<string, unknown>;
  output?: Record<string, unknown>;
}) {
  const c = Context.create();
  c.redactedInputs = captured.inputs;
  c.redactedOutput = captured.output ?? null;
  return c as never;
}

describe('ObsLoggingMiddleware honours the configured rules for the WHOLE record', () => {
  const payload = () => ({
    top: SECRET,
    items: [SECRET],
    nested: { key: SECRET },
  });

  it('redacts a secret at every position of a logged input', () => {
    const rc = new RedactionConfig({ fieldPatterns: [], valuePatterns: ['sk-[A-Za-z0-9]{6,}'] });
    const { mw, emitted } = harness(rc);
    mw.before('executor.x.y', payload(), capturedCtx({ inputs: payload() }));

    const inputs = (emitted()[0]?.extra as Record<string, unknown>)?.['inputs'] as Record<
      string,
      unknown
    >;
    expect(inputs['top']).toBe('***REDACTED***');
    // The two that leaked. `items` / `nested` are never seen by the flat
    // `apply()` pass — only by the logger's recursive one, which is why the
    // logger has to be holding the same rules.
    expect(inputs['items']).toEqual(['***REDACTED***']);
    expect(inputs['nested']).toEqual({ key: '***REDACTED***' });
  });

  it('redacts a secret at every position of a logged output', () => {
    const rc = new RedactionConfig({ fieldPatterns: [], valuePatterns: ['sk-[A-Za-z0-9]{6,}'] });
    const { mw, emitted } = harness(rc);
    const c = capturedCtx({ inputs: {}, output: payload() });
    mw.before('executor.x.y', {}, c);
    mw.after('executor.x.y', {}, payload(), c);

    const out = (emitted()[1]?.extra as Record<string, unknown>)?.['output'] as Record<
      string,
      unknown
    >;
    expect(out['top']).toBe('***REDACTED***');
    expect(out['items']).toEqual(['***REDACTED***']);
    expect(out['nested']).toEqual({ key: '***REDACTED***' });
  });

  it('a NARROWED rule set is not widened again by the logger default', () => {
    // The reverse error the same split produced. An operator who writes
    // `sensitive_keys: []` has disabled key-based redaction; the logger was
    // still applying the shipped default list underneath, so `password` came
    // back redacted from a configuration that asked for no key rule at all.
    const rc = new RedactionConfig({ fieldPatterns: [], valuePatterns: [] });
    const { mw, emitted } = harness(rc);
    mw.before('executor.x.y', { password: 'hunter2' }, capturedCtx({ inputs: { password: 'hunter2' } }));

    const inputs = (emitted()[0]?.extra as Record<string, unknown>)?.['inputs'] as Record<
      string,
      unknown
    >;
    expect(inputs['password']).toBe('hunter2');
  });

  it('with NO config supplied the shipped defaults still apply', () => {
    // The other half: aligning the two passes must not disable the
    // out-of-the-box redaction that callers who configure nothing rely on.
    const { mw, emitted } = harness(null);
    const inputs0 = { password: 'hunter2', nested: { token: 'abc' } };
    mw.before('executor.x.y', inputs0, capturedCtx({ inputs: inputs0 }));

    const inputs = (emitted()[0]?.extra as Record<string, unknown>)?.['inputs'] as Record<
      string,
      unknown
    >;
    expect(inputs['password']).toBe('***REDACTED***');
    expect(inputs['nested']).toEqual({ token: '***REDACTED***' });
  });

  it('correlation fields survive a rule that would match them', () => {
    const rc = new RedactionConfig({ fieldPatterns: [], valuePatterns: ['.*'] });
    const { mw, emitted } = harness(rc);
    mw.before('executor.x.y', {}, ctx());

    const extra = emitted()[0]?.extra as Record<string, unknown>;
    expect(extra['module_id']).toBe('executor.x.y');
  });
});
