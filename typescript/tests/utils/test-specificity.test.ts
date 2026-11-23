import { describe, it, expect } from 'vitest';
import { calculateSpecificity } from '../../src/utils/pattern.js';

describe('calculateSpecificity', () => {
  it('returns 0 for pure wildcard', () => {
    expect(calculateSpecificity('*')).toBe(0);
  });

  it('scores exact segments as +2', () => {
    expect(calculateSpecificity('api')).toBe(2);
    expect(calculateSpecificity('api.handler')).toBe(4);
    expect(calculateSpecificity('api.handler.task_submit')).toBe(6);
  });

  it('scores partial wildcards as +1', () => {
    expect(calculateSpecificity('api*')).toBe(1);
  });

  it('scores mixed patterns correctly', () => {
    // "api" = +2, "*" = +0
    expect(calculateSpecificity('api.*')).toBe(2);
    // "api" = +2, "handler" = +2, "*" = +0
    expect(calculateSpecificity('api.handler.*')).toBe(4);
  });

  it('handles wildcard-only segments', () => {
    expect(calculateSpecificity('*.*')).toBe(0);
  });
});
