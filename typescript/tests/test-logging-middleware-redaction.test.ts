/**
 * PROTOCOL_SPEC 10.6.1 "Where the rules apply", requirement 5 (D-131).
 *
 * A logging middleware shipped by the SDK MUST log `context.redactedInputs` /
 * `context.redactedOutput`, never the raw `inputs` / `output` it is handed, and
 * a field marked `x-sensitive` MUST NOT appear unredacted whether or not a
 * redaction configuration was supplied.
 *
 * The field names below are chosen to match NO default `sensitive_keys`
 * pattern, so the only rule that can redact them is `x-sensitive` — the one
 * rule the logger's own key-based pass cannot apply.
 */

import { describe, expect, it } from 'vitest';

import { Context } from '../src/context.js';
import { Executor, REDACTED_VALUE } from '../src/executor.js';
import { LoggingMiddleware } from '../src/middleware/logging.js';
import { ContextLogger, ObsLoggingMiddleware } from '../src/observability/context-logger.js';
import { Registry } from '../src/registry/registry.js';

const INPUT_SECRET = 'favourite-colour-VALUE-1234';
const OUTPUT_SECRET = 'shoe-size-VALUE-5678';

async function registry(): Promise<Registry> {
  const reg = new Registry();
  await reg.register('demo.sensitive', {
    description: 'module with x-sensitive input and output fields',
    inputSchema: {
      type: 'object',
      properties: {
        favourite_colour: { type: 'string', 'x-sensitive': true },
        visible: { type: 'string' },
      },
    },
    outputSchema: {
      type: 'object',
      properties: {
        shoe_size: { type: 'string', 'x-sensitive': true },
        visible: { type: 'string' },
      },
    },
    execute: () => ({ shoe_size: OUTPUT_SECRET, visible: 'out-plain' }),
  });
  return reg;
}

function captureObsLogger(): { logger: ContextLogger; lines: string[] } {
  const lines: string[] = [];
  const logger = new ContextLogger({ name: 'test', output: { write: (s: string) => lines.push(s) } });
  return { logger, lines };
}

function captureLegacyLogger(): {
  logger: { info: (m: string, e?: Record<string, unknown>) => void; error: (m: string, e?: Record<string, unknown>) => void };
  lines: string[];
} {
  const lines: string[] = [];
  const record = (m: string, e?: Record<string, unknown>) => lines.push(`${m} ${JSON.stringify(e ?? {})}`);
  return { logger: { info: record, error: record }, lines };
}

const INPUTS = { favourite_colour: INPUT_SECRET, visible: 'in-plain' };

describe('D-131: built-in logging middleware logs the captured (redacted) values', () => {
  it('ObsLoggingMiddleware redacts x-sensitive input and output with no RedactionConfig', async () => {
    const { logger, lines } = captureObsLogger();
    const executor = new Executor({ registry: await registry() });
    executor.use(new ObsLoggingMiddleware({ logger }));

    await executor.call('demo.sensitive', { ...INPUTS });

    const text = lines.join('\n');
    expect(lines.length).toBeGreaterThanOrEqual(2);
    expect(text).not.toContain(INPUT_SECRET);
    expect(text).not.toContain(OUTPUT_SECRET);
    expect(text).toContain(REDACTED_VALUE);
    // The non-sensitive fields are still logged, so the record is not simply empty.
    expect(text).toContain('in-plain');
    expect(text).toContain('out-plain');
  });

  it('LoggingMiddleware redacts x-sensitive input and output with no RedactionConfig', async () => {
    const { logger, lines } = captureLegacyLogger();
    const executor = new Executor({ registry: await registry() });
    executor.use(new LoggingMiddleware({ logger }));

    await executor.call('demo.sensitive', { ...INPUTS });

    const text = lines.join('\n');
    expect(text).not.toContain(INPUT_SECRET);
    expect(text).not.toContain(OUTPUT_SECRET);
    expect(text).toContain(REDACTED_VALUE);
    expect(text).toContain('in-plain');
    expect(text).toContain('out-plain');
  });

  it('LoggingMiddleware.onError logs the redacted inputs', async () => {
    const reg = new Registry();
    await reg.register('demo.failing', {
      description: 'fails after input capture',
      inputSchema: {
        type: 'object',
        properties: { favourite_colour: { type: 'string', 'x-sensitive': true } },
      },
      outputSchema: { type: 'object' },
      execute: () => {
        throw new Error('boom');
      },
    });
    const { logger, lines } = captureLegacyLogger();
    const executor = new Executor({ registry: reg });
    executor.use(new LoggingMiddleware({ logger }));

    await expect(executor.call('demo.failing', { favourite_colour: INPUT_SECRET })).rejects.toThrow();

    const text = lines.join('\n');
    expect(text).toContain('ERROR');
    expect(text).not.toContain(INPUT_SECRET);
  });

  it('never logs the raw arguments when no captured values exist', () => {
    // Outside the pipeline nothing filled redactedInputs / redactedOutput, so
    // the middleware cannot know which fields are x-sensitive. Logging the raw
    // argument is exactly what requirement 5 forbids.
    const ctx = Context.create();
    const obs = captureObsLogger();
    const obsMw = new ObsLoggingMiddleware({ logger: obs.logger });
    obsMw.before('demo.sensitive', { ...INPUTS }, ctx);
    obsMw.after('demo.sensitive', { ...INPUTS }, { shoe_size: OUTPUT_SECRET }, ctx);

    const legacy = captureLegacyLogger();
    const legacyMw = new LoggingMiddleware({ logger: legacy.logger });
    const ctx2 = Context.create();
    legacyMw.before('demo.sensitive', { ...INPUTS }, ctx2);
    legacyMw.after('demo.sensitive', { ...INPUTS }, { shoe_size: OUTPUT_SECRET }, ctx2);
    legacyMw.onError('demo.sensitive', { ...INPUTS }, new Error('x'), ctx2);

    const text = [...obs.lines, ...legacy.lines].join('\n');
    expect(obs.lines.length).toBe(2);
    expect(legacy.lines.length).toBe(3);
    expect(text).not.toContain(INPUT_SECRET);
    expect(text).not.toContain(OUTPUT_SECRET);
  });

  it('a module with no output schema still has its output redacted by the configured rules', async () => {
    const reg = new Registry();
    await reg.register('demo.schemaless', {
      description: 'no output schema',
      inputSchema: { type: 'object' },
      execute: () => ({ password: 'hunter2-VALUE', visible: 'out-plain' }),
    });
    const { logger, lines } = captureLegacyLogger();
    const executor = new Executor({ registry: reg });
    executor.use(new LoggingMiddleware({ logger }));

    await executor.call('demo.schemaless', { password: 'in-hunter2-VALUE' });

    const text = lines.join('\n');
    expect(text).not.toContain('hunter2-VALUE');
    expect(text).toContain('out-plain');
  });
});
