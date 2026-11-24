import { describe, it, expect } from 'vitest';
import { ErrorCodeRegistry, ErrorCodeCollisionError, FRAMEWORK_ERROR_CODE_PREFIXES } from '../src/error-code-registry.js';
import { ErrorCodes } from '../src/errors.js';

describe('ErrorCodeRegistry', () => {
  it('registers custom codes successfully', () => {
    const reg = new ErrorCodeRegistry();
    reg.register('my.module', new Set(['CUSTOM_ERROR_1', 'CUSTOM_ERROR_2']));
    expect(reg.allCodes.has('CUSTOM_ERROR_1')).toBe(true);
    expect(reg.allCodes.has('CUSTOM_ERROR_2')).toBe(true);
  });

  it('throws on collision with framework codes', () => {
    const reg = new ErrorCodeRegistry();
    expect(() => reg.register('my.module', new Set([ErrorCodes.MODULE_NOT_FOUND]))).toThrow(ErrorCodeCollisionError);
  });

  it('protects PIPELINE_STEP_ERROR / PIPELINE_STEP_NOT_FOUND as framework codes', () => {
    expect(ErrorCodes.PIPELINE_STEP_ERROR).toBe('PIPELINE_STEP_ERROR');
    expect(ErrorCodes.PIPELINE_STEP_NOT_FOUND).toBe('PIPELINE_STEP_NOT_FOUND');
    const reg = new ErrorCodeRegistry();
    expect(() => reg.register('my.module', new Set([ErrorCodes.PIPELINE_STEP_ERROR]))).toThrow(
      ErrorCodeCollisionError,
    );
    expect(() =>
      reg.register('my.module', new Set([ErrorCodes.PIPELINE_STEP_NOT_FOUND])),
    ).toThrow(ErrorCodeCollisionError);
  });

  it('throws on collision with reserved prefix', () => {
    const reg = new ErrorCodeRegistry();
    expect(() => reg.register('my.module', new Set(['MODULE_CUSTOM']))).toThrow(ErrorCodeCollisionError);
  });

  it('throws on collision with another module', () => {
    const reg = new ErrorCodeRegistry();
    reg.register('module.a', new Set(['CUSTOM_CODE']));
    expect(() => reg.register('module.b', new Set(['CUSTOM_CODE']))).toThrow(ErrorCodeCollisionError);
  });

  it('allows same module to re-register same codes', () => {
    const reg = new ErrorCodeRegistry();
    reg.register('my.module', new Set(['CUSTOM_CODE']));
    expect(() => reg.register('my.module', new Set(['CUSTOM_CODE']))).not.toThrow();
  });

  it('unregisters codes', () => {
    const reg = new ErrorCodeRegistry();
    reg.register('my.module', new Set(['CUSTOM_CODE']));
    reg.unregister('my.module');
    expect(reg.allCodes.has('CUSTOM_CODE')).toBe(false);
  });

  it('skips empty code set', () => {
    const reg = new ErrorCodeRegistry();
    reg.register('my.module', new Set());
    // No error thrown
  });

  it('includes framework codes in allCodes', () => {
    const reg = new ErrorCodeRegistry();
    expect(reg.allCodes.has(ErrorCodes.MODULE_NOT_FOUND)).toBe(true);
  });

  it('has expected framework prefixes', () => {
    expect(FRAMEWORK_ERROR_CODE_PREFIXES.has('MODULE_')).toBe(true);
    expect(FRAMEWORK_ERROR_CODE_PREFIXES.has('SCHEMA_')).toBe(true);
    expect(FRAMEWORK_ERROR_CODE_PREFIXES.has('ACL_')).toBe(true);
  });
});

describe('framework code inventory is complete (sync finding A-C-003)', () => {
  // `collectFrameworkCodes()` is `new Set(Object.values(ErrorCodes))`, but eight
  // codes are thrown from src/pipeline.ts and were never added to that map:
  // PIPELINE_ABORT, PIPELINE_CONFIGURATION_ERROR, PIPELINE_DEPENDENCY_ERROR,
  // STEP_NAME_DUPLICATE, STEP_NOT_FOUND, STEP_NOT_REMOVABLE,
  // STEP_NOT_REPLACEABLE, STRATEGY_NOT_FOUND. None starts with one of the 14
  // reserved prefixes either, so neither half of the A17 guard covered them and
  // `ErrorCodeRegistry.register()` accepted all eight from a user module.
  //
  // apcore-rust derives its set from the exhaustive `ErrorCode` enum, so it
  // rejected them — a 2-of-3 divergence on a collision guard.

  const REGRESSED = [
    'PIPELINE_ABORT',
    'PIPELINE_CONFIGURATION_ERROR',
    'PIPELINE_DEPENDENCY_ERROR',
    'STEP_NAME_DUPLICATE',
    'STEP_NOT_FOUND',
    'STEP_NOT_REMOVABLE',
    'STEP_NOT_REPLACEABLE',
    'STRATEGY_NOT_FOUND',
  ] as const;

  it('declares every pipeline/step/strategy code in ErrorCodes', () => {
    const declared = new Set<string>(Object.values(ErrorCodes));
    const missing = REGRESSED.filter((c) => !declared.has(c));
    expect(missing).toEqual([]);
  });

  it('rejects a user module claiming any of them', () => {
    const registry = new ErrorCodeRegistry();
    const accepted: string[] = [];
    REGRESSED.forEach((code, i) => {
      try {
        registry.register(`user.module_${i}`, new Set([code]));
        accepted.push(code);
      } catch {
        /* expected: ErrorCodeCollisionError */
      }
    });
    expect(accepted).toEqual([]);
  });

  it('still accepts a genuinely module-owned code', () => {
    const registry = new ErrorCodeRegistry();
    expect(() => registry.register('user.mine', new Set(['MY_OWN_FAILURE']))).not.toThrow();
  });
});
