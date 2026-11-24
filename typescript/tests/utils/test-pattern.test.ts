import { describe, it, expect } from 'vitest';
import { matchPattern } from '../../src/utils/pattern.js';

describe('matchPattern – exact and trivial patterns', () => {
  it('returns true when pattern is the lone wildcard *', () => {
    expect(matchPattern('*', 'anything')).toBe(true);
    expect(matchPattern('*', '')).toBe(true);
  });

  it('returns true when pattern has no wildcard and equals moduleId', () => {
    expect(matchPattern('module.a', 'module.a')).toBe(true);
  });

  it('returns false when pattern has no wildcard and does not equal moduleId', () => {
    expect(matchPattern('module.a', 'module.b')).toBe(false);
  });
});

describe('matchPattern – prefix wildcard (pattern starts with non-*)', () => {
  it('returns false when moduleId does not start with the prefix segment', () => {
    // Pattern split: ['core.', ''] – prefix check fails immediately
    expect(matchPattern('core.*', 'other.module')).toBe(false);
  });

  it('returns true when moduleId starts with the prefix segment', () => {
    expect(matchPattern('core.*', 'core.auth')).toBe(true);
  });
});

describe('matchPattern – suffix wildcard (pattern does not end with *)', () => {
  it('returns false when moduleId does not end with the last segment (lines 26-27)', () => {
    // Pattern 'a*z': segments = ['a', 'z']. moduleId 'a.b.c' starts with 'a',
    // then the endsWith check for 'z' fails.
    expect(matchPattern('a*z', 'a.b.c')).toBe(false);
  });

  it('returns true when moduleId ends with the last segment (lines 25-27 pass)', () => {
    // Pattern 'a*c': segments = ['a', 'c']. moduleId 'a.b.c' starts with 'a'
    // and ends with 'c'.
    expect(matchPattern('a*c', 'a.b.c')).toBe(true);
  });
});

describe('matchPattern – multi-segment wildcard (lines 20-23)', () => {
  it('returns true when all middle segments are found in order (lines 20-23 exercised)', () => {
    // Pattern 'a*b*c': segments = ['a', 'b', 'c'].
    // moduleId 'aXXbYYc': startsWith 'a', then indexOf('b') found, then endsWith 'c'.
    expect(matchPattern('a*b*c', 'aXXbYYc')).toBe(true);
  });

  it('returns false when a middle segment is not found – idx === -1 (line 21)', () => {
    // Pattern 'a*MISSING*c': the substring 'MISSING' is absent in 'aXXbYYc'.
    expect(matchPattern('a*MISSING*c', 'aXXbYYc')).toBe(false);
  });

  it('returns false when the second middle segment is not found after the first', () => {
    // Pattern 'a*b*z': 'b' is found in 'aXXbYYc', but 'z' is not found after it.
    expect(matchPattern('a*b*z', 'aXXbYYc')).toBe(false);
  });

  it('advances pos correctly so segments must appear in sequence, not out of order', () => {
    // 'b' appears before 'X' in the string, so 'X' cannot be found *after* 'b'.
    // Pattern 'a*b*X': 'b' at index 3, then indexOf('X', 4) returns -1.
    expect(matchPattern('a*b*X', 'aXXbYYc')).toBe(false);
  });
});

describe('matchPattern – pattern starting with * (no prefix check)', () => {
  it('returns true when moduleId ends with the suffix of a leading-* pattern', () => {
    // Pattern '*suffix': segments = ['', 'suffix']. The startsWith block is skipped.
    // Loop i=1: 'suffix' is found in 'prefix.suffix'. Pattern ends with 'suffix'
    // so endsWith check also passes.
    expect(matchPattern('*suffix', 'prefix.suffix')).toBe(true);
  });

  it('returns false when moduleId does not end with the suffix', () => {
    expect(matchPattern('*suffix', 'prefix.other')).toBe(false);
  });

  it('returns true for *-only-surrounded pattern when middle is found anywhere', () => {
    // Pattern '*mid*': segments = ['', 'mid', '']. startsWith skipped (starts with *).
    // Loop i=1: 'mid' found. Loop i=2: empty segment, continue. Pattern ends with *,
    // so endsWith block is skipped. Returns true.
    expect(matchPattern('*mid*', 'before.mid.after')).toBe(true);
  });
});

describe('matchPattern – consecutive wildcards (empty middle segments, line 19)', () => {
  it('returns true for a**b when moduleId starts with a and ends with b (empty segment skipped)', () => {
    // Pattern 'a**b': segments = ['a', '', 'b']. Prefix 'a' matches.
    // Loop i=1: segment '' → continue (line 19). Loop i=2: 'b' is found.
    // endsWith 'b' check passes.
    expect(matchPattern('a**b', 'aXYb')).toBe(true);
  });

  it('returns false for a**b when moduleId does not start with a', () => {
    expect(matchPattern('a**b', 'zXYb')).toBe(false);
  });

  it('returns false for a**b when moduleId does not end with b', () => {
    expect(matchPattern('a**b', 'aXYz')).toBe(false);
  });

  it('handles three consecutive wildcards – all middle segments are empty and skipped', () => {
    // Pattern 'x***y': segments = ['x', '', '', 'y'].
    expect(matchPattern('x***y', 'xABCy')).toBe(true);
    expect(matchPattern('x***y', 'xABCz')).toBe(false);
  });
});
