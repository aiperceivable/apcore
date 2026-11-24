/**
 * PROTOCOL_SPEC §10.6.1 "Where the rules apply" — the configured redaction
 * rules must reach the executor's capture point, not only log emission
 * (aiperceivable/apcore#120).
 *
 * Every case builds a real `Config`, constructs a real client, executes a real
 * module, and reads `context.redactedInputs` / `redactedOutput`. That shape is
 * the point of the file, and it is the acceptance condition the issue names:
 * EVERY pre-existing redaction test drove either the config object or the
 * logger, which is why a MUST written in `docs/features/observability.md` went
 * unimplemented in all three SDKs for the entire life of the keys.
 *
 * The fields are read through a MIDDLEWARE rather than a `Context` handed to
 * `call`: the pipeline derives a child context and the capture point writes to
 * that, so an assertion against the caller's object would test the wrong
 * object. Middleware is also where `ObsLoggingMiddleware` reads these fields,
 * so it is the surface the contract is about.
 */

import { Type } from '@sinclair/typebox';
import { describe, it, expect } from 'vitest';

import { APCore } from '../src/client.js';
import { Config } from '../src/config.js';
import { Middleware } from '../src/middleware/base.js';

const SECRET = 'sk-abcdef123456';

const EchoIn = Type.Object({
  note: Type.String(),
  label: Type.String(),
  amount: Type.Number(),
});
const EchoOut = Type.Object({ echoed: Type.String(), label: Type.String() });

class Capture extends Middleware {
  inputs: Record<string, unknown> | null = null;
  output: Record<string, unknown> | null = null;

  override before(_id: string, _inputs: Record<string, unknown>, context: unknown): null {
    this.inputs = (context as { redactedInputs: Record<string, unknown> | null }).redactedInputs;
    return null;
  }

  override after(
    _id: string,
    _inputs: Record<string, unknown>,
    _output: Record<string, unknown>,
    context: unknown,
  ): null {
    const c = context as {
      redactedInputs: Record<string, unknown> | null;
      redactedOutput: Record<string, unknown> | null;
    };
    this.inputs = c.redactedInputs;
    this.output = c.redactedOutput;
    return null;
  }
}

function harness(redaction: Record<string, unknown> | null) {
  const raw: Record<string, unknown> = { version: '1.0', project: { name: 'capture-point' } };
  if (redaction !== null) raw['obs'] = { redaction };
  const client = new APCore({ config: new Config(raw) });
  client.register('executor.test.echo', {
    inputSchema: EchoIn,
    outputSchema: EchoOut,
    description: 'Echo the note back so the OUTPUT capture point has something to redact.',
    async execute(inputs: Record<string, unknown>) {
      return { echoed: inputs.note as string, label: inputs.label as string };
    },
  });
  const seen = new Capture();
  client.use(seen);
  return { client, seen };
}

async function run(redaction: Record<string, unknown> | null): Promise<Capture> {
  const { client, seen } = harness(redaction);
  await client.call('executor.test.echo', { note: SECRET, label: 'public', amount: 42 });
  expect(seen.inputs, 'the capture point must have run at all').not.toBeNull();
  return seen;
}

describe('the configured rules reach the capture point', () => {
  const valueRule = { sensitive_keys: [], regex_patterns: ['sk-[A-Za-z0-9]{6,}'] };

  it('a configured regex redacts the captured input', async () => {
    const seen = await run(valueRule);
    expect(seen.inputs?.note).toBe('***REDACTED***');
    // The discriminating half: a field the rule does NOT match must survive, or
    // "redacted everything" would satisfy the assertion above.
    expect(seen.inputs?.label).toBe('public');
  });

  it('a configured regex redacts the captured output', async () => {
    const seen = await run(valueRule);
    expect(seen.output?.echoed).toBe('***REDACTED***');
    expect(seen.output?.label).toBe('public');
  });

  it('a configured sensitive key redacts the captured input', async () => {
    // `label` matches nothing in the shipped default list, so this can only
    // pass if the OPERATOR's list was read.
    const seen = await run({ sensitive_keys: ['label'] });
    expect(seen.inputs?.label).toBe('***REDACTED***');
    expect(seen.inputs?.note).toBe(SECRET);
  });

  it('a configured replacement token is used', async () => {
    const seen = await run({ sensitive_keys: ['label'], replacement: '<<GONE>>' });
    expect(seen.inputs?.label).toBe('<<GONE>>');
  });

  it('a non-string value is still not tested', async () => {
    // §10.6.1 requirement 2 holds at this surface too, not only at logging.
    const seen = await run({ sensitive_keys: [], regex_patterns: ['[0-9]+'] });
    expect(seen.inputs?.amount).toBe(42);
  });
});

describe('the defaults still apply', () => {
  // Requirement 3: "no configuration" means the DEFAULTS, never no redaction.
  // Wiring the capture point must not change what an unconfigured caller gets.

  it('an explicitly narrowed list is honoured', async () => {
    const seen = await run({ sensitive_keys: [] });
    expect(seen.inputs?.note).toBe(SECRET);
  });

  for (const [label, redaction] of [
    ['no obs block at all', null],
    ['an empty obs.redaction block', {}],
  ] as const) {
    it(`with ${label} the default list applies`, async () => {
      const raw: Record<string, unknown> = { version: '1.0', project: { name: 'capture-point' } };
      if (redaction !== null) raw['obs'] = { redaction };
      const client = new APCore({ config: new Config(raw) });
      client.register('executor.test.secret', {
        inputSchema: Type.Object({ password: Type.String(), keep: Type.String() }),
        outputSchema: Type.Object({ ok: Type.Boolean() }),
        description: 'A module whose input carries a field the default list covers.',
        async execute() {
          return { ok: true };
        },
      });
      const seen = new Capture();
      client.use(seen);
      await client.call('executor.test.secret', { password: 'hunter2', keep: 'v' });

      expect(seen.inputs?.password).toBe('***REDACTED***');
      expect(seen.inputs?.keep).toBe('v');
    });
  }
});
