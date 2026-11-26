import { describe, it, expect, vi } from 'vitest';
import { Type } from '@sinclair/typebox';
import { FunctionModule, module, normalizeResult, makeAutoId } from '../src/decorator.js';
import { Context, createIdentity } from '../src/context.js';
import { Registry } from '../src/registry/registry.js';

const inputSchema = Type.Object({ name: Type.String() });
const outputSchema = Type.Object({ greeting: Type.String() });

describe('FunctionModule', () => {
  it('wraps an execute function', async () => {
    const fm = new FunctionModule({
      execute: (inputs) => ({ greeting: `Hello, ${inputs['name']}!` }),
      moduleId: 'greet.hello',
      inputSchema,
      outputSchema,
      description: 'Says hello',
    });
    const ctx = Context.create(createIdentity('test-user'));
    const result = await fm.execute({ name: 'World' }, ctx);
    expect(result).toEqual({ greeting: 'Hello, World!' });
    await expect(fm.registration).resolves.toBeUndefined();
  });

  it('exposes correct properties', () => {
    const fm = new FunctionModule({
      execute: () => ({}),
      moduleId: 'test.props',
      inputSchema,
      outputSchema,
      description: 'Test props',
      documentation: 'Some docs',
      tags: ['tag1', 'tag2'],
      version: '2.0.0',
    });
    expect(fm.moduleId).toBe('test.props');
    expect(fm.description).toBe('Test props');
    expect(fm.documentation).toBe('Some docs');
    expect(fm.tags).toEqual(['tag1', 'tag2']);
    expect(fm.version).toBe('2.0.0');
  });

  it('uses sensible defaults', () => {
    const fm = new FunctionModule({
      execute: () => ({}),
      moduleId: 'test.defaults',
      inputSchema,
      outputSchema,
    });
    expect(fm.description).toBe('Module test.defaults');
    expect(fm.documentation).toBeNull();
    expect(fm.tags).toBeNull();
    expect(fm.version).toBe('1.0.0');
    expect(fm.annotations).toBeNull();
    expect(fm.metadata).toBeNull();
    expect(fm.examples).toBeNull();
  });

  it('normalizes null return value', async () => {
    const fm = new FunctionModule({
      execute: () => null as unknown as Record<string, unknown>,
      moduleId: 'test.normalize',
      inputSchema,
      outputSchema,
    });
    const ctx = Context.create(createIdentity('test-user'));
    const result = await fm.execute({}, ctx);
    expect(result).toEqual({});
  });
});

describe('normalizeResult', () => {
  it('null returns empty object', () => {
    expect(normalizeResult(null)).toEqual({});
  });

  it('undefined returns empty object', () => {
    expect(normalizeResult(undefined)).toEqual({});
  });

  it('plain object passes through', () => {
    const obj = { a: 1, b: 'two' };
    expect(normalizeResult(obj)).toBe(obj);
  });

  it('string is wrapped in { result }', () => {
    expect(normalizeResult('hello')).toEqual({ result: 'hello' });
  });

  it('number is wrapped in { result }', () => {
    expect(normalizeResult(42)).toEqual({ result: 42 });
  });

  it('boolean is wrapped in { result }', () => {
    expect(normalizeResult(true)).toEqual({ result: true });
  });

  it('array is wrapped in { result }', () => {
    expect(normalizeResult([1, 2, 3])).toEqual({ result: [1, 2, 3] });
  });
});

describe('module() factory', () => {
  it('creates FunctionModule with correct properties', () => {
    const fm = module({
      id: 'factory.test',
      inputSchema,
      outputSchema,
      description: 'Factory module',
      execute: () => ({ greeting: 'hi' }),
    });
    expect(fm).toBeInstanceOf(FunctionModule);
    expect(fm.moduleId).toBe('factory.test');
    expect(fm.description).toBe('Factory module');
  });

  it('throws InvalidInputError when id is not provided (spec §5.11.6)', () => {
    // JavaScript cannot derive `{module_path}.{name}` at runtime, so module()
    // requires an explicit id rather than silently colliding on a literal
    // 'anonymous' default. Aligned with apcore-rust which has never had
    // auto-ID generation.
    expect(() =>
      module({
        inputSchema,
        outputSchema,
        execute: () => ({}),
      }),
    ).toThrow(/requires an explicit 'id' option/);
  });

  it('passes through optional fields', () => {
    const fm = module({
      id: 'opts.check',
      inputSchema,
      outputSchema,
      description: 'desc',
      documentation: 'docs here',
      tags: ['t1'],
      version: '3.0.0',
      metadata: { key: 'val' },
      execute: () => ({}),
    });
    expect(fm.documentation).toBe('docs here');
    expect(fm.tags).toEqual(['t1']);
    expect(fm.version).toBe('3.0.0');
    expect(fm.metadata).toEqual({ key: 'val' });
  });

  it('warns about and ignores an unknown option', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      const options = {
        id: 'opts.unknown',
        inputSchema,
        outputSchema,
        execute: () => ({}),
        customOption: true,
      };
      expect(module(options)).not.toHaveProperty('customOption');
      expect(warn).toHaveBeenCalledWith(expect.stringContaining("unknown option 'customOption'"));
    } finally {
      warn.mockRestore();
    }
  });

  it.each([-1, Number.POSITIVE_INFINITY])('warns about an invalid timeout of %s', (timeout) => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      module({ id: 'opts.budget', inputSchema, outputSchema, execute: () => ({}), resources: { timeout } });
      expect(warn).toHaveBeenCalledWith(expect.stringContaining('invalid resources.timeout'));
    } finally {
      warn.mockRestore();
    }
  });

  it('auto-registers with registry', async () => {
    const registry = new Registry();
    const fm = module({
      id: 'auto.registered',
      inputSchema,
      outputSchema,
      execute: () => ({ ok: true }),
      registry,
    });
    expect(registry.has('auto.registered')).toBe(true);
    expect(registry.get('auto.registered')).toBe(fm);
    await expect(fm.registration).resolves.toBeUndefined();
  });
});

describe('makeAutoId', () => {
  it('lowercases and replaces non-alphanumeric', () => {
    expect(makeAutoId('Hello World')).toBe('hello_world');
  });

  it('preserves dots', () => {
    expect(makeAutoId('my.module.name')).toBe('my.module.name');
  });

  it('prefixes digit-leading segments', () => {
    expect(makeAutoId('2fast.4you')).toBe('_2fast._4you');
  });

  it('handles valid IDs unchanged', () => {
    expect(makeAutoId('valid_id')).toBe('valid_id');
  });
});
