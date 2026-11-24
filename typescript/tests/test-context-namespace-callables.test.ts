import { describe, expect, it } from 'vitest';
import { isAsyncHandler } from '../src/middleware/context-namespace.js';

describe('async handler detection observes callable metadata', () => {
  it('accepts typed async handlers without invoking them', () => {
    let invoked = false;
    const handler = async (value: string): Promise<string> => {
      invoked = true;
      return value;
    };
    expect(isAsyncHandler(handler)).toBe(true);
    expect(invoked).toBe(false);
  });

  it('accepts sync handlers with required parameters without invoking them', () => {
    let invoked = false;
    const handler = (value: number, suffix: string): string => {
      invoked = true;
      return `${value}${suffix}`;
    };
    expect(isAsyncHandler(handler)).toBe(false);
    expect(invoked).toBe(false);
  });
});
