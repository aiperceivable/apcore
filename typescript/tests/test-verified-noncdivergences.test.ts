/**
 * Two divergences where TypeScript is the CORRECT implementation and the peer
 * SDKs are being aligned to it. Pinned here so this repo cannot drift off the
 * behaviour the others are adopting.
 *
 * STR-4 / CAN-002 — `Executor.stream` Phase 3 rethrows
 *   `ExecutionCancelledError` while apcore-python and apcore-rust swallow it
 *   with the rest of the post-stream failures. D-20 says cancellation must
 *   never be swallowed, so the rethrow stands.
 *
 * MMD-5 — `scanExtensions` throws `ConfigNotFoundError` when the extensions
 *   root is not a usable directory, where apcore-python and apcore-rust log
 *   and return `[]`. A root that is not a directory is a misconfiguration,
 *   and silently returning zero modules is exactly the fail-open shape
 *   apcore#118 was about.
 */

import { describe, it, expect, afterAll } from 'vitest';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { Type } from '@sinclair/typebox';
import { Registry } from '../src/registry/registry.js';
import { Executor } from '../src/executor.js';
import { Middleware } from '../src/middleware/base.js';
import { ExecutionCancelledError } from '../src/cancel.js';
import { SchemaValidationError, ConfigNotFoundError } from '../src/errors.js';
import { scanExtensions } from '../src/registry/scanner.js';

const STREAMER = {
  inputSchema: Type.Object({}),
  outputSchema: Type.Object({ n: Type.Number() }),
  description: 'streams two chunks',
  annotations: { streaming: true },
  stream: async function* () {
    yield { n: 1 };
    yield { n: 2 };
  },
  execute: () => ({ n: 0 }),
};

class ThrowsInAfter extends Middleware {
  constructor(private readonly _error: Error) {
    super();
  }
  after(): Record<string, unknown> {
    throw this._error;
  }
}

async function drain(gen: AsyncGenerator<Record<string, unknown>>): Promise<number> {
  let count = 0;
  for await (const _chunk of gen) count += 1;
  return count;
}

describe('stream Phase 3 never swallows a cancellation (STR-4 / CAN-002)', () => {
  it('ExecutionCancelledError from Phase 3 reaches the caller', async () => {
    const registry = new Registry();
    registry.registerInternal('executor.demo.stream', STREAMER);
    const executor = new Executor({ registry });
    executor.use(new ThrowsInAfter(new ExecutionCancelledError()));

    await expect(drain(executor.stream('executor.demo.stream', {}))).rejects.toThrow(
      ExecutionCancelledError,
    );
  });

  it('every OTHER Phase 3 failure is still swallowed, as the spec requires', async () => {
    const registry = new Registry();
    registry.registerInternal('executor.demo.stream', STREAMER);
    const executor = new Executor({ registry });
    executor.use(new ThrowsInAfter(new SchemaValidationError('boom', [])));

    // Chunks are already delivered and cannot be recalled, so the post-stream
    // failure is logged rather than raised.
    await expect(drain(executor.stream('executor.demo.stream', {}))).resolves.toBe(2);
  });
});

describe('a non-directory extensions root is a misconfiguration, not zero modules (MMD-5)', () => {
  const tmp = mkdtempSync(join(tmpdir(), 'apcore-mmd5-'));
  afterAll(() => rmSync(tmp, { recursive: true, force: true }));

  it('throws when the root exists but is a FILE', () => {
    const notADir = join(tmp, 'extensions');
    writeFileSync(notADir, 'this is a file, not a directory\n');
    expect(() => scanExtensions(notADir)).toThrow(ConfigNotFoundError);
  });

  it('throws when the root does not exist at all', () => {
    expect(() => scanExtensions(join(tmp, 'absent'))).toThrow(ConfigNotFoundError);
  });
});
