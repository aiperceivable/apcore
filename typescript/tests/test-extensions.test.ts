import { describe, it, expect, vi } from 'vitest';
import { ExtensionManager } from '../src/extensions.js';
import type { ExtensionPoint } from '../src/extensions.js';
import { Middleware } from '../src/middleware/index.js';
import { ACL } from '../src/acl.js';
import { InvalidInputError } from '../src/errors.js';
import type { ModuleError } from '../src/errors.js';
import { Registry } from '../src/registry/registry.js';
import { Executor } from '../src/executor.js';
import { TracingMiddleware, InMemoryExporter } from '../src/observability/tracing.js';
import type { Discoverer, ModuleValidator } from '../src/registry/registry.js';
import type { SpanExporter, Span } from '../src/observability/tracing.js';

// ---------------------------------------------------------------------------
// Helpers: concrete implementations that satisfy the interfaces
// ---------------------------------------------------------------------------

class StubDiscoverer implements Discoverer {
  discover(_roots: string[]) {
    return [];
  }
}

class StubValidator implements ModuleValidator {
  validate(_module: unknown) {
    return [];
  }
}

class StubMiddleware extends Middleware {}

class StubExporter implements SpanExporter {
  export(_span: Span): void {
    // no-op
  }
}

// ---------------------------------------------------------------------------
// Tests: ExtensionManager basics
// ---------------------------------------------------------------------------

describe('ExtensionManager init', () => {
  it('has six built-in extension points', () => {
    const mgr = new ExtensionManager();
    const points = mgr.listPoints();
    expect(points).toHaveLength(6);
  });

  it('built-in point names match expected set', () => {
    const mgr = new ExtensionManager();
    const names = new Set(mgr.listPoints().map((p: ExtensionPoint) => p.name));
    expect(names).toEqual(new Set(['discoverer', 'middleware', 'acl', 'span_exporter', 'module_validator', 'approval_handler']));
  });

  it('listPoints returns ExtensionPoint objects', () => {
    const mgr = new ExtensionManager();
    for (const p of mgr.listPoints()) {
      expect(p).toHaveProperty('name');
      expect(p).toHaveProperty('description');
      expect(p).toHaveProperty('multiple');
    }
  });
});

// ---------------------------------------------------------------------------
// Tests: register / get / getAll / unregister
// ---------------------------------------------------------------------------

describe('Discoverer extension', () => {
  it('register and get', () => {
    const mgr = new ExtensionManager();
    const disc = new StubDiscoverer();
    mgr.register('discoverer', disc);
    expect(mgr.get('discoverer')).toBe(disc);
  });

  it('register replaces single', () => {
    const mgr = new ExtensionManager();
    const disc1 = new StubDiscoverer();
    const disc2 = new StubDiscoverer();
    mgr.register('discoverer', disc1);
    mgr.register('discoverer', disc2);
    expect(mgr.get('discoverer')).toBe(disc2);
  });

  it('unregister returns true', () => {
    const mgr = new ExtensionManager();
    const disc = new StubDiscoverer();
    mgr.register('discoverer', disc);
    expect(mgr.unregister('discoverer', disc)).toBe(true);
    expect(mgr.get('discoverer')).toBeNull();
  });

  it('unregister returns false when missing', () => {
    const mgr = new ExtensionManager();
    const disc = new StubDiscoverer();
    expect(mgr.unregister('discoverer', disc)).toBe(false);
  });

  it('get returns null when empty', () => {
    const mgr = new ExtensionManager();
    expect(mgr.get('discoverer')).toBeNull();
  });
});

describe('Middleware extension', () => {
  it('register and getAll', () => {
    const mgr = new ExtensionManager();
    const mw1 = new StubMiddleware();
    const mw2 = new StubMiddleware();
    mgr.register('middleware', mw1);
    mgr.register('middleware', mw2);
    expect(mgr.getAll('middleware')).toEqual([mw1, mw2]);
  });

  it('unregister specific', () => {
    const mgr = new ExtensionManager();
    const mw1 = new StubMiddleware();
    const mw2 = new StubMiddleware();
    mgr.register('middleware', mw1);
    mgr.register('middleware', mw2);
    mgr.unregister('middleware', mw1);
    expect(mgr.getAll('middleware')).toEqual([mw2]);
  });
});

describe('ACL extension', () => {
  it('register and get', () => {
    const mgr = new ExtensionManager();
    const acl = new ACL([]);
    mgr.register('acl', acl);
    expect(mgr.get('acl')).toBe(acl);
  });
});

describe('SpanExporter extension', () => {
  it('register multiple', () => {
    const mgr = new ExtensionManager();
    const exp1 = new StubExporter();
    const exp2 = new StubExporter();
    mgr.register('span_exporter', exp1);
    mgr.register('span_exporter', exp2);
    expect(mgr.getAll('span_exporter')).toEqual([exp1, exp2]);
  });
});

describe('ModuleValidator extension', () => {
  it('register and get', () => {
    const mgr = new ExtensionManager();
    const val = new StubValidator();
    mgr.register('module_validator', val);
    expect(mgr.get('module_validator')).toBe(val);
  });
});

/** The `code` carried by whatever `fn` throws, or undefined for an untyped throw. */
function codeOfThrow(fn: () => unknown): string | undefined {
  try {
    fn();
  } catch (e) {
    return (e as ModuleError).code;
  }
  return undefined;
}

// ---------------------------------------------------------------------------
// Tests: validation errors
// ---------------------------------------------------------------------------

describe('Validation', () => {
  it('unknown point throws InvalidInputError on register', () => {
    const mgr = new ExtensionManager();
    expect(() => mgr.register('nonexistent', {})).toThrow(InvalidInputError);
    expect(codeOfThrow(() => mgr.register('nonexistent', {}))).toBe('GENERAL_INVALID_INPUT');
  });

  it('wrong type throws TypeError on register', () => {
    const mgr = new ExtensionManager();
    expect(() => mgr.register('middleware', 'not_a_middleware')).toThrow(TypeError);
  });

  // D-108: the code is the assertion. A bare `Error` carries none, so a caller
  // could not tell a misspelled point name from any other failure, and every
  // cross-language assertion had to fall back to matching on message text.
  it('get unknown point throws InvalidInputError', () => {
    const mgr = new ExtensionManager();
    expect(() => mgr.get('nonexistent')).toThrow(InvalidInputError);
    expect(codeOfThrow(() => mgr.get('nonexistent'))).toBe('GENERAL_INVALID_INPUT');
  });

  it('getAll unknown point throws InvalidInputError', () => {
    const mgr = new ExtensionManager();
    expect(() => mgr.getAll('nonexistent')).toThrow(InvalidInputError);
    expect(codeOfThrow(() => mgr.getAll('nonexistent'))).toBe('GENERAL_INVALID_INPUT');
  });

  it('unregister unknown point throws InvalidInputError', () => {
    const mgr = new ExtensionManager();
    expect(() => mgr.unregister('nonexistent', {})).toThrow(InvalidInputError);
    expect(codeOfThrow(() => mgr.unregister('nonexistent', {}))).toBe('GENERAL_INVALID_INPUT');
  });

  it('a registered point holding nothing does not throw', () => {
    // D-108's other half, and the control for the three above: if the rejection
    // were "throw for anything not holding an extension", these would be red.
    const mgr = new ExtensionManager();
    expect(mgr.get('acl')).toBeNull();
    expect(mgr.getAll('middleware')).toEqual([]);
    expect(mgr.unregister('middleware', {})).toBe(false);
  });

  it('discoverer rejects wrong type', () => {
    const mgr = new ExtensionManager();
    expect(() => mgr.register('discoverer', new StubMiddleware())).toThrow(TypeError);
  });

  it('acl rejects wrong type', () => {
    const mgr = new ExtensionManager();
    expect(() => mgr.register('acl', new StubMiddleware())).toThrow(TypeError);
  });
});

// ---------------------------------------------------------------------------
// Tests: apply()
// ---------------------------------------------------------------------------

describe('apply()', () => {
  it('wires discoverer into registry', () => {
    const mgr = new ExtensionManager();
    const disc = new StubDiscoverer();
    mgr.register('discoverer', disc);

    const registry = { setDiscoverer: vi.fn(), setValidator: vi.fn() } as unknown as Registry;
    const executor = { use: vi.fn(), setAcl: vi.fn(), middlewares: [] } as unknown as Executor;
    mgr.apply(registry, executor);

    expect(registry.setDiscoverer).toHaveBeenCalledWith(disc);
  });

  it('wires validator into registry', () => {
    const mgr = new ExtensionManager();
    const val = new StubValidator();
    mgr.register('module_validator', val);

    const registry = { setDiscoverer: vi.fn(), setValidator: vi.fn() } as unknown as Registry;
    const executor = { use: vi.fn(), setAcl: vi.fn(), middlewares: [] } as unknown as Executor;
    mgr.apply(registry, executor);

    expect(registry.setValidator).toHaveBeenCalledWith(val);
  });

  it('wires ACL into executor', () => {
    const mgr = new ExtensionManager();
    const acl = new ACL([]);
    mgr.register('acl', acl);

    const registry = { setDiscoverer: vi.fn(), setValidator: vi.fn() } as unknown as Registry;
    const executor = { use: vi.fn(), setAcl: vi.fn(), middlewares: [] } as unknown as Executor;
    mgr.apply(registry, executor);

    expect(executor.setAcl).toHaveBeenCalledWith(acl);
  });

  it('wires middleware into executor', () => {
    const mgr = new ExtensionManager();
    const mw = new StubMiddleware();
    mgr.register('middleware', mw);

    const registry = { setDiscoverer: vi.fn(), setValidator: vi.fn() } as unknown as Registry;
    const executor = { use: vi.fn(), setAcl: vi.fn(), middlewares: [] } as unknown as Executor;
    mgr.apply(registry, executor);

    expect(executor.use).toHaveBeenCalledWith(mw);
  });

  it('wires single span exporter into TracingMiddleware', () => {
    const mgr = new ExtensionManager();
    const newExp = new StubExporter();
    mgr.register('span_exporter', newExp);

    const inMem = new InMemoryExporter();
    const tracingMw = new TracingMiddleware(inMem);

    const registry = { setDiscoverer: vi.fn(), setValidator: vi.fn() } as unknown as Registry;
    const executor = { use: vi.fn(), setAcl: vi.fn(), middlewares: [tracingMw] } as unknown as Executor;
    mgr.apply(registry, executor);

    // setExporter is used instead of direct private access
    expect((tracingMw as unknown as Record<string, unknown>)['_exporter']).toBe(newExp);
  });

  it('wires multiple span exporters via composite exporter', () => {
    const mgr = new ExtensionManager();
    const exp1 = new StubExporter();
    const exp2 = new StubExporter();
    mgr.register('span_exporter', exp1);
    mgr.register('span_exporter', exp2);

    const inMem = new InMemoryExporter();
    const tracingMw = new TracingMiddleware(inMem);

    const registry = { setDiscoverer: vi.fn(), setValidator: vi.fn() } as unknown as Registry;
    const executor = { use: vi.fn(), setAcl: vi.fn(), middlewares: [tracingMw] } as unknown as Executor;
    mgr.apply(registry, executor);

    // A composite exporter should have been set that delegates to both
    const composite = (tracingMw as unknown as Record<string, unknown>)['_exporter'] as Record<string, unknown>;
    expect(composite['_exporters']).toEqual([exp1, exp2]);
  });

  it('apply with no extensions is safe', () => {
    const mgr = new ExtensionManager();
    const registry = { setDiscoverer: vi.fn(), setValidator: vi.fn() } as unknown as Registry;
    const executor = { use: vi.fn(), setAcl: vi.fn(), middlewares: [] } as unknown as Executor;
    mgr.apply(registry, executor);

    expect(registry.setDiscoverer).not.toHaveBeenCalled();
    expect(registry.setValidator).not.toHaveBeenCalled();
    expect(executor.use).not.toHaveBeenCalled();
  });

  it('warns when span_exporter registered but no TracingMiddleware present', () => {
    const mgr = new ExtensionManager();
    const newExp = new StubExporter();
    mgr.register('span_exporter', newExp);

    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});

    const registry = { setDiscoverer: vi.fn(), setValidator: vi.fn() } as unknown as Registry;
    const executor = { use: vi.fn(), setAcl: vi.fn(), middlewares: [] } as unknown as Executor;
    mgr.apply(registry, executor);

    expect(warnSpy).toHaveBeenCalledWith(
      expect.stringContaining('span_exporter'),
    );
    warnSpy.mockRestore();
  });
});
