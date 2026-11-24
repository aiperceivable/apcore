import { describe, it, expect } from 'vitest';
import { Type } from '@sinclair/typebox';
import { validateModule } from '../../src/registry/validation.js';

describe('validateModule', () => {
  it('valid module returns no errors', () => {
    const mod = {
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      description: 'A test module',
      execute: () => ({}),
    };
    expect(validateModule(mod)).toEqual([]);
  });

  it('missing inputSchema reports error', () => {
    const mod = {
      outputSchema: Type.Object({}),
      description: 'test',
      execute: () => ({}),
    };
    const errors = validateModule(mod);
    expect(errors.length).toBeGreaterThan(0);
    expect(errors.some(e => e.includes('inputSchema'))).toBe(true);
  });

  it('missing outputSchema reports error', () => {
    const mod = {
      inputSchema: Type.Object({}),
      description: 'test',
      execute: () => ({}),
    };
    const errors = validateModule(mod);
    expect(errors.length).toBeGreaterThan(0);
    expect(errors.some(e => e.includes('outputSchema'))).toBe(true);
  });

  it('missing description reports error', () => {
    const mod = {
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      execute: () => ({}),
    };
    const errors = validateModule(mod);
    expect(errors.length).toBeGreaterThan(0);
    expect(errors.some(e => e.includes('description'))).toBe(true);
  });

  it('missing execute reports error', () => {
    const mod = {
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      description: 'test',
    };
    const errors = validateModule(mod);
    expect(errors.length).toBeGreaterThan(0);
    expect(errors.some(e => e.includes('execute'))).toBe(true);
  });

  it('completely empty object reports multiple errors', () => {
    const errors = validateModule({});
    expect(errors.length).toBe(4);
  });

  it('non-function execute reports error', () => {
    const mod = {
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      description: 'test',
      execute: 'not-a-function',
    };
    const errors = validateModule(mod);
    expect(errors.some(e => e.includes('execute'))).toBe(true);
  });
});
