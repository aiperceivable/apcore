/**
 * Spec-traced contract tests for the decorator-bindings feature (TypeScript SDK).
 *
 * Mirrors the canonical Python suite
 * (apcore-python/tests/test_decorator_bindings_spec.py). Each `it(...)` embeds a
 * verbatim clause id of the form `decorator_bindings.<method>.<kind>.<detail>`
 * so a cross-language diff lines up row-by-row across the Python / TypeScript /
 * Rust SDKs.
 *
 * TESTS ONLY — no production source is modified here.
 *
 * Major Python<->TypeScript API divergences discovered while authoring (asserted
 * against ACTUAL TS behavior, documented inline rather than papered over):
 *
 * 1. `module()` IDIOM. The Python `@module` decorator wraps a function and
 *    auto-infers Pydantic models from its signature, raising
 *    FuncMissingTypeHintError / FuncMissingReturnTypeError on untyped params /
 *    missing return. The TS `module(options)` takes an options object with
 *    EXPLICIT TypeBox `inputSchema` / `outputSchema` and performs NO signature
 *    inference. Therefore:
 *      - `module.input.func_or_none.untyped_param_no_schema`,
 *      - `module.error.FUNC_MISSING_TYPE_HINT`,
 *      - `module.error.FUNC_MISSING_RETURN_TYPE`
 *    have NO equivalent enforcement at the `module()` call site in TS. The error
 *    CLASSES exist (FuncMissingTypeHintError / FuncMissingReturnTypeError) but
 *    `module()` never raises them. We assert the closest real TS behavior: a
 *    missing `id` raises InvalidInputError(code=GENERAL_INVALID_INPUT), and we
 *    flag the missing inference enforcement as a documented cross-language gap.
 *
 * 2. ASYNC. Python `load_bindings` / `load_binding_dir` are SYNCHRONOUS
 *    (`property.async.false`). The TS equivalents are `async` and RETURN A
 *    PROMISE. We assert the real TS behavior (Promise returned / resolves) and
 *    flag the raises-vs-returns divergence.
 *
 * 3. SCHEMA MISSING. Like Python, the canonical `BINDING_SCHEMA_MISSING` clause
 *    resolves at runtime to BindingSchemaInferenceFailedError with the REAL code
 *    `BINDING_SCHEMA_INFERENCE_FAILED`. In TS this only fires under
 *    `auto_schema: true` when no `inputSchema`/`outputSchema` exports are found;
 *    a bare target (no schema key) falls back to a permissive schema instead of
 *    erroring.
 *
 * 4. TARGET RESOLUTION. The Python binding resolves dotted Python import paths
 *    (`binding_helpers:typed_function`). TS resolves ESM module *file paths* via
 *    dynamic `import()`, so binding targets here point at on-disk `.mjs` files.
 */

import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { mkdtempSync, writeFileSync, mkdirSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { Type } from '@sinclair/typebox';

import { module, FunctionModule } from '../src/decorator.js';
import { BindingLoader } from '../src/bindings.js';
import { Registry } from '../src/registry/registry.js';
import {
  InvalidInputError,
  BindingFileInvalidError,
  BindingInvalidTargetError,
  BindingModuleNotFoundError,
  BindingCallableNotFoundError,
  BindingNotCallableError,
  BindingSchemaInferenceFailedError,
  ModuleError,
} from '../src/errors.js';

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

const InputSchema = Type.Object({ name: Type.String(), count: Type.Number() });
const OutputSchema = Type.Object({ result: Type.Unknown() });

let tmpDir: string;
let loader: BindingLoader;
let registry: Registry;

beforeEach(() => {
  tmpDir = mkdtempSync(join(tmpdir(), 'apcore-decbind-spec-'));
  loader = new BindingLoader();
  registry = new Registry();
});

afterEach(() => {
  rmSync(tmpDir, { recursive: true, force: true });
});

function writeFile(name: string, content: string): string {
  const p = join(tmpDir, name);
  writeFileSync(p, content, 'utf-8');
  return p;
}

/** A target module file exporting a plain function (no schema exports). */
function writeTargetModule(name: string): string {
  return writeFile(
    name,
    'export function typedFunction(inputs) { return { result: inputs }; }\n',
  );
}

/** Build a one-entry binding YAML pointing at a typed function target. */
function bindingYaml(moduleId: string, target: string): string {
  return (
    'bindings:\n' +
    `  - module_id: ${moduleId}\n` +
    `    target: "${target}:typedFunction"\n` +
    '    input_schema:\n' +
    '      type: object\n' +
    '      properties:\n' +
    '        name:\n' +
    '          type: string\n' +
    '    output_schema:\n' +
    '      type: object\n' +
    '      properties:\n' +
    '        result:\n' +
    '          type: string\n'
  );
}

// ===========================================================================
// Contract: module
// ===========================================================================

describe("Contract: module", () => {
  // The Python untyped-param rule has no TS equivalent: module() takes explicit
  // schemas and never inspects a function signature. The closest real input
  // failure is a missing `id`, which raises InvalidInputError.
  it("decorator_bindings.module.input.func_or_none.untyped_param_no_schema: missing id rejected (TS has no signature inference)", () => {
    let caught: unknown;
    try {
      // Omit `id` to exercise the runtime guard. The TS type does not require a
      // compile error here (unlike Python signature inference), so the rejection
      // is purely a runtime check below.
      module({
        inputSchema: InputSchema,
        outputSchema: OutputSchema,
        execute: (i) => ({ result: i }),
      } as Parameters<typeof module>[0]);
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(InvalidInputError);
    expect((caught as InvalidInputError).code).toBe('GENERAL_INVALID_INPUT');
  });

  // Python raises FuncMissingTypeHintError(FUNC_MISSING_TYPE_HINT) from @module.
  // TS module() never performs signature inference, so it cannot raise it.
  // The error class exists but is unreachable from module(); skip as a gap.
  it.skip("decorator_bindings.module.error.FUNC_MISSING_TYPE_HINT: missing symbol — module() performs no signature inference (contract gap)", () => {
    // intentionally empty: see file-level divergence note #1
  });

  // Same as above for the missing-return-type rule.
  it.skip("decorator_bindings.module.error.FUNC_MISSING_RETURN_TYPE: missing symbol — module() performs no signature inference (contract gap)", () => {
    // intentionally empty: see file-level divergence note #1
  });

  it("decorator_bindings.module.property.async.false: module() is synchronous and returns a FunctionModule, never a Promise", () => {
    const result = module({
      id: 'spec.async_false',
      inputSchema: InputSchema,
      outputSchema: OutputSchema,
      execute: (i) => ({ result: i }),
    });
    expect(result).toBeInstanceOf(FunctionModule);
    expect(result).not.toBeInstanceOf(Promise);
    expect(typeof (result as unknown as { then?: unknown }).then).not.toBe('function');
  });

  it("decorator_bindings.module.property.thread_safe.true: >=8 concurrent creations with distinct ids produce independent modules, no cross-talk", async () => {
    const tasks = Array.from({ length: 12 }, (_, i) =>
      Promise.resolve().then(() =>
        module({
          id: `spec.concurrent.${i}`,
          inputSchema: InputSchema,
          outputSchema: OutputSchema,
          execute: (x) => ({ result: x }),
        }),
      ),
    );
    const modules = await Promise.all(tasks);
    expect(modules).toHaveLength(12);
    expect(modules.every((m) => m instanceof FunctionModule)).toBe(true);
    const ids = modules.map((m) => m.moduleId).sort();
    const expected = Array.from({ length: 12 }, (_, i) => `spec.concurrent.${i}`).sort();
    expect(ids).toEqual(expected);
  });

  it("decorator_bindings.module.property.pure.false_when_registry: passing a registry mutates registry state (observable via registry.has)", () => {
    expect(registry.has('spec.pure.registered')).toBe(false);
    module({
      id: 'spec.pure.registered',
      inputSchema: InputSchema,
      outputSchema: OutputSchema,
      execute: (i) => ({ result: i }),
      registry,
    });
    expect(registry.has('spec.pure.registered')).toBe(true);
  });
});

// ===========================================================================
// Contract: BindingLoader.load_bindings
// ===========================================================================

describe("Contract: BindingLoader.load_bindings", () => {
  it("decorator_bindings.load_bindings.error.BINDING_FILE_INVALID: empty file rejected with exact code", async () => {
    const f = writeFile('empty.binding.yaml', '');
    const err = await loader.loadBindings(f, registry).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(BindingFileInvalidError);
    expect((err as BindingFileInvalidError).code).toBe('BINDING_FILE_INVALID');
  });

  it("decorator_bindings.load_bindings.error.BINDING_INVALID_TARGET: target missing ':' separator", async () => {
    const f = writeFile(
      't.binding.yaml',
      'bindings:\n  - module_id: bad.target\n    target: no_colon_separator\n    auto_schema: true\n',
    );
    const err = await loader.loadBindings(f, registry).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(BindingInvalidTargetError);
    expect((err as BindingInvalidTargetError).code).toBe('BINDING_INVALID_TARGET');
  });

  it("decorator_bindings.load_bindings.error.BINDING_MODULE_NOT_FOUND: unimportable module path", async () => {
    const f = writeFile(
      't.binding.yaml',
      'bindings:\n  - module_id: missing.mod\n    target: "/definitely/not/a/real/module_xyz.mjs:fn"\n    auto_schema: true\n',
    );
    const err = await loader.loadBindings(f, registry).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(BindingModuleNotFoundError);
    expect((err as BindingModuleNotFoundError).code).toBe('BINDING_MODULE_NOT_FOUND');
  });

  it("decorator_bindings.load_bindings.error.BINDING_CALLABLE_NOT_FOUND: callable absent from module", async () => {
    const mod = writeTargetModule('callable_mod.mjs');
    const f = writeFile(
      't.binding.yaml',
      `bindings:\n  - module_id: missing.callable\n    target: "${mod}:noSuchCallable"\n    auto_schema: true\n`,
    );
    const err = await loader.loadBindings(f, registry).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(BindingCallableNotFoundError);
    expect((err as BindingCallableNotFoundError).code).toBe('BINDING_CALLABLE_NOT_FOUND');
  });

  it("decorator_bindings.load_bindings.error.BINDING_NOT_CALLABLE: resolved export is not a function", async () => {
    const mod = writeFile('not_callable_mod.mjs', 'export const NOT_CALLABLE = 42;\n');
    const f = writeFile(
      't.binding.yaml',
      `bindings:\n  - module_id: not.callable\n    target: "${mod}:NOT_CALLABLE"\n    auto_schema: true\n`,
    );
    const err = await loader.loadBindings(f, registry).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(BindingNotCallableError);
    expect((err as BindingNotCallableError).code).toBe('BINDING_NOT_CALLABLE');
  });

  it("decorator_bindings.load_bindings.error.BINDING_SCHEMA_MISSING: auto_schema over a target lacking schema exports -> BINDING_SCHEMA_INFERENCE_FAILED", async () => {
    // Python contract code 'BINDING_SCHEMA_MISSING' is stale; the real runtime
    // error is BindingSchemaInferenceFailedError(BINDING_SCHEMA_INFERENCE_FAILED).
    // In TS this requires auto_schema:true AND a target module that exports no
    // inputSchema/outputSchema (a plain function only).
    const mod = writeTargetModule('untyped_mod.mjs');
    const f = writeFile(
      't.binding.yaml',
      `bindings:\n  - module_id: schema.missing\n    target: "${mod}:typedFunction"\n    auto_schema: true\n`,
    );
    const err = await loader.loadBindings(f, registry).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(BindingSchemaInferenceFailedError);
    expect((err as BindingSchemaInferenceFailedError).code).toBe('BINDING_SCHEMA_INFERENCE_FAILED');
  });

  it("decorator_bindings.load_bindings.property.async.false: TS load_bindings is ASYNC — returns a Promise resolving to FunctionModule[]", async () => {
    const mod = writeTargetModule('async_mod.mjs');
    const f = writeFile('ok.binding.yaml', bindingYaml('async.false', mod));
    const promise = loader.loadBindings(f, registry);
    // Divergence from Python (async:false): TS returns a Promise.
    expect(typeof (promise as { then?: unknown }).then).toBe('function');
    const result = await promise;
    expect(Array.isArray(result)).toBe(true);
    expect(result.every((m) => m instanceof FunctionModule)).toBe(true);
  });

  it("decorator_bindings.load_bindings.property.idempotent.false: second load re-registers and raises a duplicate error; state stays consistent", async () => {
    const mod = writeTargetModule('idem_mod.mjs');
    const f = writeFile('dup.binding.yaml', bindingYaml('idem.false', mod));
    const first = await loader.loadBindings(f, registry);
    expect(first).toHaveLength(1);
    expect(registry.has('idem.false')).toBe(true);
    const err = await loader.loadBindings(f, registry).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(Error);
    // Post-state remains consistent: still registered exactly once.
    expect(registry.has('idem.false')).toBe(true);
  });
});

// ===========================================================================
// Contract: BindingLoader.load_binding_dir
// ===========================================================================

describe("Contract: BindingLoader.load_binding_dir", () => {
  it("decorator_bindings.load_binding_dir.error.BINDING_FILE_INVALID: nonexistent directory rejected with exact code", async () => {
    const missing = join(tmpDir, 'does_not_exist');
    const err = await loader.loadBindingDir(missing, registry).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(BindingFileInvalidError);
    expect((err as BindingFileInvalidError).code).toBe('BINDING_FILE_INVALID');
  });

  it("decorator_bindings.load_binding_dir.return.empty_dir_empty_list: empty directory yields empty list", async () => {
    const empty = join(tmpDir, 'empty_dir');
    mkdirSync(empty);
    const result = await loader.loadBindingDir(empty, registry);
    expect(result).toEqual([]);
  });

  it("decorator_bindings.load_binding_dir.side_effect.1.sorted_file_order: files load in sorted order; 'a' before 'b' in returned modules", async () => {
    const scan = join(tmpDir, 'scan');
    mkdirSync(scan);
    const mod = writeFile(
      join('scan', 'dir_mod.mjs'),
      'export function typedFunction(inputs) { return { result: inputs }; }\n',
    );
    writeFile(join('scan', 'a.binding.yaml'), bindingYaml('dir.a', mod));
    writeFile(join('scan', 'b.binding.yaml'), bindingYaml('dir.b', mod));
    const result = await loader.loadBindingDir(scan, registry);
    const ids = result.map((m) => m.moduleId);
    expect(ids).toEqual(['dir.a', 'dir.b']);
    expect(registry.has('dir.a')).toBe(true);
    expect(registry.has('dir.b')).toBe(true);
  });

  it("decorator_bindings.load_binding_dir.property.idempotent.false: re-scanning the same directory re-registers and raises a duplicate error", async () => {
    const scan = join(tmpDir, 'scan2');
    mkdirSync(scan);
    const mod = writeFile(
      join('scan2', 'dir_mod.mjs'),
      'export function typedFunction(inputs) { return { result: inputs }; }\n',
    );
    writeFile(join('scan2', 'x.binding.yaml'), bindingYaml('dir.idem', mod));
    const first = await loader.loadBindingDir(scan, registry);
    expect(first.map((m) => m.moduleId)).toEqual(['dir.idem']);
    const err = await loader.loadBindingDir(scan, registry).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(Error);
    expect(registry.has('dir.idem')).toBe(true);
  });
});

// Reference ModuleError so the import is load-bearing and stays type-checked
// even if every concrete-error assertion narrows to a subclass.
void (ModuleError as unknown);
