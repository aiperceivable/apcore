/**
 * Spec-traced contract tests for the apcore Module Interface (TypeScript SDK).
 *
 * Source of truth: apcore/docs/features/module-interface.md, the single
 * `## Contract: Module conformance` block. Mirrors the canonical Python suite
 * apcore-python/tests/test_module_interface_spec.py clause-for-clause.
 *
 * Every `it(...)` name carries the verbatim cross-language clause id in the form
 * `module_interface.<method>.<kind>.<detail>` so rows line up across the
 * Python / TypeScript / Rust SDKs.
 *
 * CROSS-LANGUAGE NOTES:
 * - Structural conformance in TS is verified by `validateModule()` returning a
 *   non-empty string[] (duck-typed), NOT a runtime `Module` instanceof — the
 *   `Module` symbol is a TypeScript interface with no runtime representation.
 * - The contract's ### Errors section names six dedicated error types
 *   (MissingRequiredAttribute, InvalidSchemaType, DescriptionTooLong,
 *   DocumentationTooLong, InvalidAnnotations, InvalidExample). The TS SDK does
 *   NOT expose those as distinct error types; conformance is duck-typed via
 *   `validateModule()` string output. Those clauses are emitted as skips tagged
 *   `missing symbol` to document the cross-language gap.
 */

import { describe, it, expect } from 'vitest';
import { Type } from '@sinclair/typebox';
import { Context } from '../src/context.js';
import { ErrorCodes, ModuleTimeoutError, SchemaValidationError } from '../src/errors.js';
import { createAnnotations, type ModuleExample } from '../src/module.js';
import { Registry } from '../src/registry/registry.js';
import { validateModule } from '../src/registry/validation.js';
import { SchemaValidator } from '../src/schema/validator.js';

// ---------------------------------------------------------------------------
// Schema + module fixtures exercising the required/optional surface
// ---------------------------------------------------------------------------

const EchoInput = Type.Object({
  value: Type.String({ description: 'The value to echo back' }),
});

const EchoOutput = Type.Object({
  echoed: Type.String({ description: 'The echoed value' }),
});

/** Minimal module satisfying the full required surface. */
const conformingModule = {
  inputSchema: EchoInput,
  outputSchema: EchoOutput,
  description: 'Echoes the supplied value back unchanged.',
  execute(inputs: Record<string, unknown>, _context: Context): Record<string, unknown> {
    return { echoed: inputs.value };
  },
};

const missingInputSchemaModule = {
  outputSchema: EchoOutput,
  description: 'missing inputSchema',
  execute(_inputs: Record<string, unknown>, _context: Context): Record<string, unknown> {
    return { echoed: '' };
  },
};

const missingOutputSchemaModule = {
  inputSchema: EchoInput,
  description: 'missing outputSchema',
  execute(_inputs: Record<string, unknown>, _context: Context): Record<string, unknown> {
    return { echoed: '' };
  },
};

const missingDescriptionModule = {
  inputSchema: EchoInput,
  outputSchema: EchoOutput,
  // description deliberately absent
  execute(_inputs: Record<string, unknown>, _context: Context): Record<string, unknown> {
    return { echoed: '' };
  },
};

const missingExecuteModule = {
  inputSchema: EchoInput,
  outputSchema: EchoOutput,
  description: 'missing execute',
};

function makeContext(): Context {
  return Context.create();
}

// ===========================================================================
// Inputs / required-surface validation
// ===========================================================================

describe('Module conformance — required surface', () => {
  it('module_interface.execute.input.input_schema.missing: lacking inputSchema fails conformance', () => {
    const errors = validateModule(missingInputSchemaModule);
    expect(errors.length).toBeGreaterThan(0);
    expect(errors.some((e) => e.toLowerCase().includes('inputschema'))).toBe(true);
    // A fully-conforming module produces no errors (control assertion).
    expect(validateModule(conformingModule)).toEqual([]);
  });

  it('module_interface.execute.input.output_schema.missing: lacking outputSchema fails conformance', () => {
    const errors = validateModule(missingOutputSchemaModule);
    expect(errors.length).toBeGreaterThan(0);
    expect(errors.some((e) => e.toLowerCase().includes('outputschema'))).toBe(true);
  });

  it('module_interface.execute.input.description.missing: lacking description fails conformance', () => {
    const errors = validateModule(missingDescriptionModule);
    expect(errors.length).toBeGreaterThan(0);
    expect(errors.some((e) => e.toLowerCase().includes('description'))).toBe(true);
  });

  it('module_interface.execute.input.execute.missing: lacking execute fails conformance', () => {
    const errors = validateModule(missingExecuteModule);
    expect(errors.length).toBeGreaterThan(0);
    expect(errors.some((e) => e.toLowerCase().includes('execute'))).toBe(true);
  });

  it('module_interface.execute.input.inputs.invalid_against_schema: invalid inputs raise SCHEMA_VALIDATION_ERROR', () => {
    const validator = new SchemaValidator();
    let caught: unknown;
    try {
      // value is required and must be a string; a number violates the schema.
      validator.validateInput({ value: 123 }, EchoInput);
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(SchemaValidationError);
    expect((caught as SchemaValidationError).code).toBe(ErrorCodes.SCHEMA_VALIDATION_ERROR);
  });
});

// ===========================================================================
// Errors — declared error types
// ===========================================================================

describe('Module conformance — errors', () => {
  it.skip('module_interface.errors.MissingRequiredAttribute: missing symbol MissingRequiredAttribute (contract gap)', () => {
    // TS SDK validates required surface via validateModule() string output, not a typed error.
  });

  it.skip('module_interface.errors.InvalidSchemaType: missing symbol InvalidSchemaType (contract gap)', () => {
    // No such error type in src/errors.ts.
  });

  it.skip('module_interface.errors.DescriptionTooLong: missing symbol DescriptionTooLong (contract gap)', () => {
    // No such error type; the 200-char limit is not enforced via a typed error.
  });

  it.skip('module_interface.errors.DocumentationTooLong: missing symbol DocumentationTooLong (contract gap)', () => {
    // No such error type in src/errors.ts.
  });

  it.skip('module_interface.errors.InvalidAnnotations: missing symbol InvalidAnnotations (contract gap)', () => {
    // No such error type in src/errors.ts.
  });

  it.skip('module_interface.errors.InvalidExample: missing symbol InvalidExample (contract gap)', () => {
    // ModuleExample is a plain interface without typed-error validation of title/inputs.
  });

  it('module_interface.errors.MODULE_TIMEOUT: ModuleTimeoutError carries code MODULE_TIMEOUT', () => {
    const err = new ModuleTimeoutError('slow.module', 30000);
    expect(err.code).toBe(ErrorCodes.MODULE_TIMEOUT);
    expect(err.code).toBe('MODULE_TIMEOUT');
    // Detail fields are preserved for AI-facing guidance.
    expect(err.moduleId).toBe('slow.module');
    expect(err.timeoutMs).toBe(30000);
  });
});

// ===========================================================================
// Properties
// ===========================================================================

/** Module whose execute() is async. */
const asyncModule = {
  inputSchema: EchoInput,
  outputSchema: EchoOutput,
  description: 'Async echo module.',
  async execute(inputs: Record<string, unknown>, _context: Context): Promise<Record<string, unknown>> {
    await Promise.resolve();
    return { echoed: inputs.value };
  },
};

/** Thread-safe module: only uses per-call local state, never shared mutation. */
const concurrentModule = {
  inputSchema: EchoInput,
  outputSchema: EchoOutput,
  description: 'Concurrent-safe echo module.',
  async execute(inputs: Record<string, unknown>, _context: Context): Promise<Record<string, unknown>> {
    await Promise.resolve();
    return { echoed: inputs.value };
  },
};
// A shared marker that MUST NOT be mutated by execute().
const SHARED_MARKER = 'untouched';

/** Module with observable side effects — pure is NOT required by the contract. */
class SideEffectfulModule {
  inputSchema = EchoInput;
  outputSchema = EchoOutput;
  description = 'Counts how many times it ran.';
  callCount = 0;
  execute(inputs: Record<string, unknown>, _context: Context): Record<string, unknown> {
    this.callCount += 1;
    return { echoed: inputs.value };
  }
}

describe('Module conformance — properties', () => {
  it('module_interface.execute.property.async: async execute() returns a Promise resolving to a valid output', async () => {
    const result = asyncModule.execute({ value: 'hi' }, makeContext());
    expect(result).toBeInstanceOf(Promise);
    const resolved = await result;
    expect(resolved).toEqual({ echoed: 'hi' });
    // Result validates against outputSchema.
    const validator = new SchemaValidator();
    expect(validator.validateOutput(resolved, EchoOutput)).toEqual({ echoed: 'hi' });
  });

  it('module_interface.execute.property.thread_safe: >=8 concurrent execute() calls all succeed, shared state untouched', async () => {
    const markerBefore = SHARED_MARKER;
    const n = 16;
    const inputs = Array.from({ length: n }, (_, i) => ({ value: `v${i}` }));
    const results = await Promise.all(
      inputs.map((inp) => concurrentModule.execute(inp, makeContext())),
    );
    expect(results.map((r) => r.echoed)).toEqual(
      Array.from({ length: n }, (_, i) => `v${i}`),
    );
    // Shared marker must not have been mutated during concurrent execution.
    expect(SHARED_MARKER).toBe(markerBefore);
    expect(SHARED_MARKER).toBe('untouched');
  });

  it('module_interface.execute.property.pure_false: side effects are permitted and module still conforms', () => {
    const module = new SideEffectfulModule();
    expect(module.callCount).toBe(0);
    module.execute({ value: 'a' }, makeContext());
    module.execute({ value: 'b' }, makeContext());
    // Side effect is observable — the contract permits this.
    expect(module.callCount).toBe(2);
    // Despite the side effect, the module still satisfies the surface.
    expect(validateModule(module)).toEqual([]);
  });
});

// ===========================================================================
// Return-value constraints
// ===========================================================================

describe('Module conformance — return value', () => {
  it('module_interface.execute.return.must_be_dict: execute() returns an object validating against outputSchema', () => {
    const result = conformingModule.execute({ value: 'payload' }, makeContext());
    expect(typeof result).toBe('object');
    expect(result).not.toBeNull();
    const validator = new SchemaValidator();
    expect(validator.validateOutput(result as Record<string, unknown>, EchoOutput)).toEqual({
      echoed: 'payload',
    });
  });

  it('module_interface.execute.return.must_be_dict: a return value not matching outputSchema raises SCHEMA_VALIDATION_ERROR', () => {
    const validator = new SchemaValidator();
    let caught: unknown;
    try {
      // 'echoed' missing entirely violates the required output schema.
      validator.validateOutput({}, EchoOutput);
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(SchemaValidationError);
    expect((caught as SchemaValidationError).code).toBe(ErrorCodes.SCHEMA_VALIDATION_ERROR);
  });
});

// ===========================================================================
// Side effects — lifecycle hook ordering
// ===========================================================================

/** Records lifecycle-hook invocation order into a shared log list. */
class LifecycleModule {
  inputSchema = EchoInput;
  outputSchema = EchoOutput;
  description = 'Lifecycle-instrumented module.';
  private readonly log: string[];
  constructor(log: string[]) {
    this.log = log;
  }
  execute(inputs: Record<string, unknown>, _context: Context): Record<string, unknown> {
    return { echoed: inputs.value };
  }
  onLoad(): void {
    this.log.push('on_load');
  }
  onUnload(): void {
    this.log.push('on_unload');
  }
}

/** Module implementing onSuspend / onResume for hot-reload round-trip. */
class SuspendResumeModule {
  inputSchema = EchoInput;
  outputSchema = EchoOutput;
  description = 'Stateful suspend/resume module.';
  version = '1.0.0';
  counter = 0;
  resumed: Record<string, unknown> | null = null;
  execute(inputs: Record<string, unknown>, _context: Context): Record<string, unknown> {
    this.counter += 1;
    return { echoed: inputs.value };
  }
  onSuspend(): Record<string, unknown> {
    return { counter: this.counter };
  }
  onResume(state: Record<string, unknown>): void {
    this.counter = (state.counter as number) ?? 0;
    this.resumed = state;
  }
}

describe('Module conformance — lifecycle side effects', () => {
  it('module_interface.lifecycle.side_effect.1.on_load: onLoad() invoked on register', async () => {
    const log: string[] = [];
    const registry = new Registry();
    await registry.register('test.lifecycle_load', new LifecycleModule(log));
    expect(log).toEqual(['on_load']);
  });

  it('module_interface.lifecycle.side_effect.2.on_unload: onUnload() invoked on unregister, ordered after onLoad', async () => {
    const log: string[] = [];
    const registry = new Registry();
    await registry.register('test.lifecycle_unload', new LifecycleModule(log));
    registry.unregister('test.lifecycle_unload');
    // Order MUST be on_load then on_unload.
    expect(log).toEqual(['on_load', 'on_unload']);
  });

  it('module_interface.lifecycle.side_effect.3.suspend_resume: onSuspend() state survives JSON round-trip into onResume()', () => {
    const old = new SuspendResumeModule();
    // Build up some state via execute().
    old.execute({ value: 'x' }, makeContext());
    old.execute({ value: 'y' }, makeContext());
    expect(old.counter).toBe(2);

    // Export state (onSuspend) — MUST be JSON-serializable.
    const state = old.onSuspend();
    expect(state).toEqual({ counter: 2 });
    expect(JSON.parse(JSON.stringify(state))).toEqual({ counter: 2 });

    // Restore into a new instance (onResume) — state is preserved.
    const fresh = new SuspendResumeModule();
    expect(fresh.counter).toBe(0);
    fresh.onResume(state);
    expect(fresh.counter).toBe(2);
    expect(fresh.resumed).toEqual({ counter: 2 });
  });
});

// ===========================================================================
// Structural conformance (duck-typed via validateModule)
// ===========================================================================

describe('Module conformance — structural', () => {
  it('module_interface.execute.property.structural: conformance is structural (duck-typed), not inheritance', () => {
    // A class carrying the required surface passes structural conformance...
    expect(validateModule(conformingModule)).toEqual([]);
    // ...one missing execute does NOT.
    expect(validateModule(missingExecuteModule).length).toBeGreaterThan(0);
    expect(validateModule(missingExecuteModule).some((e) => e.toLowerCase().includes('execute'))).toBe(
      true,
    );
    // Sanity: createAnnotations / ModuleExample are the real exported surface.
    expect(createAnnotations().openWorld).toBe(true);
    const example: ModuleExample = { title: 't', inputs: { value: 'v' }, output: { echoed: 'v' } };
    expect(example.title).toBe('t');
  });
});
