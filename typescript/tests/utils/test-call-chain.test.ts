import { describe, it, expect } from 'vitest';
import { guardCallChain, DEFAULT_MAX_CALL_DEPTH, DEFAULT_MAX_MODULE_REPEAT } from '../../src/utils/call-chain.js';
import { CallDepthExceededError, CircularCallError, CallFrequencyExceededError, InvalidInputError, ModuleError } from '../../src/errors.js';

describe('guardCallChain', () => {
  it('passes for normal call chain', () => {
    expect(() => guardCallChain('c', ['a', 'b', 'c'])).not.toThrow();
  });

  it('throws CallDepthExceededError when chain too deep', () => {
    const chain = Array.from({ length: 33 }, (_, i) => `m${i}`);
    chain.push('target');
    expect(() => guardCallChain('target', chain)).toThrow(CallDepthExceededError);
  });

  it('throws CircularCallError on strict cycle', () => {
    // A -> B -> A is a cycle of length >= 2
    expect(() => guardCallChain('a', ['a', 'b', 'a'])).toThrow(CircularCallError);
  });

  it('does not throw for self-call (cycle length 1)', () => {
    // A -> A is not a strict cycle (length < 2)
    expect(() => guardCallChain('a', ['a', 'a'])).not.toThrow();
  });

  it('throws CallFrequencyExceededError when module called too many times', () => {
    // Non-circular pattern: a appears 4 times but no strict A->B->A cycle
    // Each 'a' is separated by the same module, so no cycle of length >= 2
    expect(() => guardCallChain('a', ['a', 'a', 'a', 'a'], 32, 3)).toThrow(CallFrequencyExceededError);
  });

  it('uses default constants', () => {
    expect(DEFAULT_MAX_CALL_DEPTH).toBe(32);
    expect(DEFAULT_MAX_MODULE_REPEAT).toBe(3);
  });

  // D-84: the floor rejection is a typed apcore error, not a bare Error. A bare
  // Error escapes `instanceof ModuleError` handlers and carries no registry code.
  it('throws InvalidInputError(GENERAL_INVALID_INPUT) on invalid maxCallDepth', () => {
    expect(() => guardCallChain('a', ['a'], 0)).toThrow('maxCallDepth must be >= 1');
    expect(() => guardCallChain('a', ['a'], 0)).toThrow(InvalidInputError);
    try {
      guardCallChain('a', ['a'], 0);
      expect.unreachable('guardCallChain should have thrown');
    } catch (e) {
      expect(e).toBeInstanceOf(ModuleError);
      expect((e as ModuleError).code).toBe('GENERAL_INVALID_INPUT');
    }
  });

  it('throws InvalidInputError(GENERAL_INVALID_INPUT) on invalid maxModuleRepeat', () => {
    expect(() => guardCallChain('a', ['a'], 32, 0)).toThrow('maxModuleRepeat must be >= 1');
    expect(() => guardCallChain('a', ['a'], 32, 0)).toThrow(InvalidInputError);
    try {
      guardCallChain('a', ['a'], 32, 0);
      expect.unreachable('guardCallChain should have thrown');
    } catch (e) {
      expect(e).toBeInstanceOf(ModuleError);
      expect((e as ModuleError).code).toBe('GENERAL_INVALID_INPUT');
    }
  });

  it('allows at most maxModuleRepeat occurrences', () => {
    // 3 occurrences of 'a' (self-calls, no strict cycle) with maxModuleRepeat=3 should pass
    expect(() => guardCallChain('a', ['a', 'a', 'a'], 32, 3)).not.toThrow();
  });
});
