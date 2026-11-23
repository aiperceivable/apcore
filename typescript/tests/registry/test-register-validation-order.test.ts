/**
 * D-86 (spec v1.49.0) — `Registry.register` validation order.
 *
 * `registry-system.md` "Side Effects (ordered)" pins it as
 * `module_id` → structure/streaming → custom validator → duplicate:
 * intrinsic-then-extrinsic, because what is wrong with the MODULE must be
 * fixed either way, whereas a duplicate id may only mean the author picked the
 * wrong name.
 *
 * The Side Effects list used to name neither the structure check nor the custom
 * validator, and the three SDKs each filled the silence differently — Python
 * ran validator → streaming, this SDK ran duplicate → validator, Rust ran
 * streaming → validator → duplicate. A module that was malformed,
 * validator-rejected AND duplicate at once reported three different errors.
 * apcore-python pinned the resolved order; this SDK did not.
 *
 * Each test makes TWO checks fail at once and asserts which one is reported —
 * that is the entire discriminator. A test tripping one check at a time passes
 * under every ordering.
 */

import { describe, it, expect } from 'vitest';
import { Type } from '@sinclair/typebox';
import { Registry } from '../../src/registry/registry.js';
import {
  DuplicateModuleIdError,
  InvalidInputError,
  StreamingInterfaceError,
} from '../../src/errors.js';

/** Declares `streaming: true` and provides no `stream()`. */
const streamingLiar = {
  inputSchema: Type.Object({}),
  outputSchema: Type.Object({}),
  description: 'Claims to stream',
  annotations: { streaming: true },
  execute: async () => ({}),
};

const validModule = {
  inputSchema: Type.Object({}),
  outputSchema: Type.Object({}),
  description: 'A well-formed module',
  execute: async () => ({}),
};

const rejectAll = { validate: () => ['rejected by the custom validator'] };

/**
 * Assert `register` fails with `expected`, however it signals.
 *
 * `register` is declared `Promise<void>` but several of its checks throw
 * SYNCHRONOUSLY — the id check says so in its own comment ("always sync —
 * throws synchronously for backward compat"), and the structure check does the
 * same. `expect(reg.register(...)).rejects` never receives a promise in that
 * case, so it reports the wrong thing. Wrapping in `Promise.resolve().then`
 * turns a synchronous throw into a rejection and covers both shapes, which is
 * also what a caller writing `await reg.register(...)` inside try/catch gets.
 */
async function expectRegisterToFail(
  fn: () => Promise<void>,
  expected: new (...args: never[]) => Error,
): Promise<void> {
  await expect(Promise.resolve().then(fn)).rejects.toThrow(expected);
}

describe('D-86: register reports failures in a fixed order', () => {
  it('structure is reported before the custom validator', async () => {
    const reg = new Registry();
    reg.setValidator(rejectAll);

    await expectRegisterToFail(() => reg.register('order.streaming_and_validator', streamingLiar), StreamingInterfaceError);
  });

  it('structure is reported before the duplicate check', async () => {
    // Malformed, validator-rejected AND duplicate at once: structure wins.
    const reg = new Registry();
    await reg.register('order.all_three', validModule);
    reg.setValidator(rejectAll);

    await expectRegisterToFail(() => reg.register('order.all_three', streamingLiar), StreamingInterfaceError);

    // The incumbent registration is untouched — a rejected register must not
    // have removed or replaced what was already there.
    expect(reg.get('order.all_three')).toBe(validModule);
  });

  it('the custom validator is reported before the duplicate check', async () => {
    const reg = new Registry();
    await reg.register('order.validator_and_dup', validModule);
    reg.setValidator(rejectAll);

    await expectRegisterToFail(() => reg.register('order.validator_and_dup', validModule), InvalidInputError);
  });

  it('the module id is reported before everything else', async () => {
    // A malformed id on a module that is ALSO streaming-malformed and a
    // duplicate: the id check is first in the list.
    const reg = new Registry();
    reg.setValidator(rejectAll);

    await expectRegisterToFail(() => reg.register('Not A Valid Id', streamingLiar), InvalidInputError);
  });

  it('control: each check still fires on its own', async () => {
    // Without this, "structure wins" is also satisfied by an implementation
    // that reports StreamingInterfaceError for everything, and "validator
    // wins" by one that never reaches the duplicate check at all.
    const streamingOnly = new Registry();
    await expectRegisterToFail(() => streamingOnly.register('order.stream_only', streamingLiar), StreamingInterfaceError);

    const validatorOnly = new Registry();
    validatorOnly.setValidator(rejectAll);
    await expectRegisterToFail(() => validatorOnly.register('order.validator_only', validModule), InvalidInputError);

    const duplicateOnly = new Registry();
    await duplicateOnly.register('order.dup_only', validModule);
    await expectRegisterToFail(() => duplicateOnly.register('order.dup_only', validModule), DuplicateModuleIdError);
  });
});
