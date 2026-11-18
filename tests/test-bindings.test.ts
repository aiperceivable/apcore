import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { mkdtempSync, writeFileSync, mkdirSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { dump } from 'js-yaml';
import { Config } from '../src/config.js';
import { BindingLoader } from '../src/bindings.js';
import { Registry } from '../src/registry/registry.js';
import {
  BindingInvalidTargetError,
  BindingFileInvalidError,
  BindingModuleNotFoundError,
  BindingCallableNotFoundError,
  BindingNotCallableError,
  BindingSchemaInferenceFailedError,
} from '../src/errors.js';

let tmpDir: string;
let loader: BindingLoader;
let registry: Registry;

beforeEach(() => {
  tmpDir = mkdtempSync(join(tmpdir(), 'apcore-binding-test-'));
  loader = new BindingLoader();
  registry = new Registry();
});

afterEach(() => {
  rmSync(tmpDir, { recursive: true, force: true });
});

function writeTempModule(filename: string, content: string, inferable: boolean = true): string {
  const filePath = join(tmpDir, filename);
  const schemas = inferable ? "\nexport const inputSchema = {type:'object'};\nexport const outputSchema = {type:'object'};\n" : '';
  writeFileSync(filePath, content + schemas, 'utf-8');
  return filePath;
}

function writeTempYaml(filename: string, content: string): string {
  const filePath = join(tmpDir, filename);
  writeFileSync(filePath, content, 'utf-8');
  return filePath;
}

describe('BindingLoader', () => {
  describe('instantiation', () => {
    it('creates a new instance', () => {
      expect(new BindingLoader()).toBeInstanceOf(BindingLoader);
    });

    it('has loadBindings, loadBindingDir, and resolveTarget methods', () => {
      const bl = new BindingLoader();
      expect(typeof bl.loadBindings).toBe('function');
      expect(typeof bl.loadBindingDir).toBe('function');
      expect(typeof bl.resolveTarget).toBe('function');
    });
  });

  describe('resolveTarget', () => {
    it('throws BindingInvalidTargetError for target without colon', async () => {
      await expect(loader.resolveTarget('no_colon_here')).rejects.toThrow(BindingInvalidTargetError);
    });

    it('throws BindingModuleNotFoundError for non-existent module path', async () => {
      await expect(
        loader.resolveTarget('/nonexistent/path/to/module.mjs:someFunc'),
      ).rejects.toThrow(BindingModuleNotFoundError);
    });

    it('successfully resolves a function export from a real JS module', async () => {
      const modPath = writeTempModule(
        'func_export.mjs',
        'export function greet(name) { return `Hello, ${name}`; }\n',
      );
      const fn = await loader.resolveTarget(`${modPath}:greet`);
      expect(typeof fn).toBe('function');
      expect(fn('World')).toBe('Hello, World');
    });

    it('successfully resolves a class method', async () => {
      const modPath = writeTempModule(
        'class_export.mjs',
        `export class Calculator {\n  add(a, b) { return a + b; }\n}\n`,
      );
      const fn = await loader.resolveTarget(`${modPath}:Calculator.add`);
      expect(typeof fn).toBe('function');
      expect(fn(2, 3)).toBe(5);
    });

    it('throws BindingCallableNotFoundError for missing callable', async () => {
      const modPath = writeTempModule('missing_callable.mjs', 'export function exists() { return true; }\n');
      await expect(loader.resolveTarget(`${modPath}:doesNotExist`)).rejects.toThrow(BindingCallableNotFoundError);
    });

    it('throws BindingNotCallableError for non-function export', async () => {
      const modPath = writeTempModule('non_callable.mjs', 'export const MY_CONSTANT = 42;\n');
      await expect(loader.resolveTarget(`${modPath}:MY_CONSTANT`)).rejects.toThrow(BindingNotCallableError);
    });

    it('throws BindingNotCallableError (not BindingCallableNotFoundError) when class constructor requires arguments', async () => {
      const modPath = writeTempModule(
        'class_requires_args.mjs',
        `export class NeedsArgs {\n  constructor(required) {\n    if (required === undefined) throw new Error('required arg missing');\n  }\n  doThing() { return 42; }\n}\n`,
      );
      const error = await loader.resolveTarget(`${modPath}:NeedsArgs.doThing`).catch((e: unknown) => e);
      expect(error).toBeInstanceOf(BindingNotCallableError);
    });
  });

  describe('trustedPackagePrefixes allowlist (A-D-10)', () => {
    it('rejects a target outside the allowlist before importing', async () => {
      // Write a module that WOULD import successfully if import() were reached.
      const modPath = writeTempModule(
        'allowlist_outside.mjs',
        'export function greet() { return "hello"; }\n',
      );
      // Allowlist only permits "@trusted/" packages; the temp file path does
      // not match, so resolveTarget must reject BEFORE the dynamic import.
      const guarded = new BindingLoader({ trustedPackagePrefixes: ['@trusted/'] });
      await expect(guarded.resolveTarget(`${modPath}:greet`)).rejects.toThrow(
        BindingInvalidTargetError,
      );
    });

    it('allows a target matching an allowed prefix', async () => {
      const modPath = writeTempModule(
        'allowlist_inside.mjs',
        'export function greet() { return "ok"; }\n',
      );
      // Use the module's own absolute path as the allowed prefix so the import
      // proceeds and resolves the function.
      const guarded = new BindingLoader({ trustedPackagePrefixes: [modPath] });
      const fn = await guarded.resolveTarget(`${modPath}:greet`);
      expect(typeof fn).toBe('function');
      expect(fn()).toBe('ok');
    });

    it('preserves back-compat: no allowlist permits any importable target', async () => {
      const modPath = writeTempModule(
        'allowlist_unset.mjs',
        'export function greet() { return "any"; }\n',
      );
      const fn = await loader.resolveTarget(`${modPath}:greet`);
      expect(fn()).toBe('any');
    });
  });

  describe('loadBindings', () => {
    it('throws BindingFileInvalidError for non-existent file', async () => {
      await expect(
        loader.loadBindings('/nonexistent/path/binding.yaml', registry),
      ).rejects.toThrow(BindingFileInvalidError);
    });

    it('throws BindingFileInvalidError for invalid YAML', async () => {
      const yamlPath = writeTempYaml('invalid.binding.yaml', '{ invalid yaml: [unclosed');
      await expect(loader.loadBindings(yamlPath, registry)).rejects.toThrow(BindingFileInvalidError);
    });

    it('throws BindingFileInvalidError for empty file', async () => {
      const yamlPath = writeTempYaml('empty.binding.yaml', '');
      await expect(loader.loadBindings(yamlPath, registry)).rejects.toThrow(BindingFileInvalidError);
    });

    it('throws BindingFileInvalidError for missing bindings key', async () => {
      const yamlPath = writeTempYaml('nokey.binding.yaml', 'other_key: value\n');
      await expect(loader.loadBindings(yamlPath, registry)).rejects.toThrow(BindingFileInvalidError);
    });

    it('throws BindingFileInvalidError when the top-level root is a list', async () => {
      const yamlPath = writeTempYaml('rootlist.binding.yaml', '- one\n- two\n');
      await expect(loader.loadBindings(yamlPath, registry)).rejects.toThrow(
        /Top-level must be a mapping/,
      );
    });

    it('throws BindingFileInvalidError when the top-level root is a scalar', async () => {
      const yamlPath = writeTempYaml('rootscalar.binding.yaml', 'just-a-string\n');
      await expect(loader.loadBindings(yamlPath, registry)).rejects.toThrow(
        /Top-level must be a mapping/,
      );
    });

    it('throws BindingFileInvalidError for non-array bindings value', async () => {
      const yamlPath = writeTempYaml('notarray.binding.yaml', 'bindings: "not an array"\n');
      await expect(loader.loadBindings(yamlPath, registry)).rejects.toThrow(BindingFileInvalidError);
    });

    it('throws BindingFileInvalidError for binding entry missing module_id', async () => {
      const modPath = writeTempModule('dummy_mod.mjs', 'export function dummy() { return {}; }\n');
      const yamlPath = writeTempYaml('noid.binding.yaml', `bindings:\n  - target: "${modPath}:dummy"\n`);
      await expect(loader.loadBindings(yamlPath, registry)).rejects.toThrow(BindingFileInvalidError);
    });

    it('throws BindingFileInvalidError for binding entry missing target', async () => {
      const yamlPath = writeTempYaml('notarget.binding.yaml', 'bindings:\n  - module_id: "test.module"\n');
      await expect(loader.loadBindings(yamlPath, registry)).rejects.toThrow(BindingFileInvalidError);
    });

    it('successfully loads valid binding with inline schemas', async () => {
      const modPath = writeTempModule(
        'inline_schema_mod.mjs',
        'export function process(inputs) { return { result: inputs.name }; }\n',
      );
      const yamlPath = writeTempYaml(
        'inline.binding.yaml',
        `bindings:\n  - module_id: "test.inline"\n    target: "${modPath}:process"\n    description: "Inline schema test"\n    version: "2.0.0"\n    tags:\n      - demo\n    input_schema:\n      type: object\n      properties:\n        name:\n          type: string\n    output_schema:\n      type: object\n      properties:\n        result:\n          type: string\n`,
      );
      const results = await loader.loadBindings(yamlPath, registry);
      expect(results).toHaveLength(1);
      expect(results[0].moduleId).toBe('test.inline');
      expect(results[0].description).toBe('Inline schema test');
      expect(results[0].version).toBe('2.0.0');
    });

    it('carries a resources budget from the binding onto the module', async () => {
      const modPath = writeTempModule(
        'resources_mod.mjs',
        'export function process(inputs) { return { result: inputs.name }; }\n',
      );
      const yamlPath = writeTempYaml(
        'resources.binding.yaml',
        `bindings:\n  - module_id: "test.resources"\n    target: "${modPath}:process"\n    resources:\n      timeout: 1234\n    input_schema:\n      type: object\n    output_schema:\n      type: object\n`,
      );
      const results = await loader.loadBindings(yamlPath, registry);
      expect(results).toHaveLength(1);
      expect(results[0].resources).toEqual({ timeout: 1234 });
    });

    it('rejects implicit inference without a schema source (D-139)', async () => {
      const modPath = writeTempModule('permissive_mod.mjs', 'export function loose(inputs) { return { ok: true }; }\n', false);
      const yamlPath = writeTempYaml(
        'permissive.binding.yaml',
        `bindings:\n  - module_id: "test.permissive"\n    target: "${modPath}:loose"\n`,
      );
      await expect(loader.loadBindings(yamlPath, registry)).rejects.toThrow(BindingSchemaInferenceFailedError);
      expect(registry.has('test.permissive')).toBe(false);
    });

    it('registers modules in the registry', async () => {
      const modPath = writeTempModule('registered_mod.mjs', 'export function handler() { return {}; }\n');
      const yamlPath = writeTempYaml(
        'register.binding.yaml',
        `bindings:\n  - module_id: "test.registered"\n    target: "${modPath}:handler"\n`,
      );
      await loader.loadBindings(yamlPath, registry);
      expect(registry.has('test.registered')).toBe(true);
    });

    it('loads multiple binding entries from single file', async () => {
      const modPath = writeTempModule(
        'multi_mod.mjs',
        `export function funcA() { return { a: true }; }\nexport function funcB() { return { b: true }; }\n`,
      );
      const yamlPath = writeTempYaml(
        'multi.binding.yaml',
        `bindings:\n  - module_id: "test.multi.a"\n    target: "${modPath}:funcA"\n  - module_id: "test.multi.b"\n    target: "${modPath}:funcB"\n`,
      );
      const results = await loader.loadBindings(yamlPath, registry);
      expect(results).toHaveLength(2);
      expect(registry.has('test.multi.a')).toBe(true);
      expect(registry.has('test.multi.b')).toBe(true);
    });
  });

  describe('configured binding limits', () => {
    const limits = {
      description_max_length: 5,
      documentation_max_length: 8,
      tags_pattern: '^[a-z]+$',
    };

    function writeBinding(fields: Record<string, unknown>): string {
      const target = writeTempModule('limited.mjs', 'export function run() { return {}; }');
      return writeTempYaml('limited.binding.yaml', dump({
        spec_version: '1.0',
        bindings: [{ module_id: 'test.limited', target: `${target}:run`, ...fields }],
      }));
    }

    it('accepts values at the length limits and matching tags', async () => {
      const path = writeBinding({ description: 'Hello', documentation: 'See docs', tags: ['blue'] });
      const config = new Config({ validation: { binding: limits } });
      await expect(loader.loadBindings(path, registry, config)).resolves.toHaveLength(1);
      expect(registry.has('test.limited')).toBe(true);
    });

    it('accepts an omitted optional documentation field and tags', async () => {
      const path = writeBinding({ description: 'Hello' });
      const config = new Config({ validation: { binding: limits } });
      await expect(loader.loadBindings(path, registry, config)).resolves.toHaveLength(1);
    });

    it.each([
      { fields: { description: 'Longer' }, message: /description.*6 characters/ },
      { fields: { documentation: 'More docs' }, message: /documentation.*9 characters/ },
      { fields: { tags: ['Blue'] }, message: /tag "Blue" does not match/ },
    ])('rejects a binding with $fields outside the configured limits', async ({ fields, message }) => {
      const path = writeBinding(fields);
      const config = new Config({ validation: { binding: limits } });
      await expect(loader.loadBindings(path, registry, config)).rejects.toThrow(message);
      expect(registry.has('test.limited')).toBe(false);
    });
  });

  describe('loadBindingDir', () => {
    it('throws BindingFileInvalidError for non-existent directory', async () => {
      await expect(loader.loadBindingDir('/nonexistent/dir/path', registry)).rejects.toThrow(BindingFileInvalidError);
    });

    it('loads all *.binding.yaml files in directory', async () => {
      const bindDir = join(tmpDir, 'bindings');
      mkdirSync(bindDir);

      const modPath = writeTempModule(
        'dir_mod.mjs',
        `export function alpha() { return { alpha: true }; }\nexport function beta() { return { beta: true }; }\n`,
      );

      writeTempYaml(
        join('bindings', 'alpha.binding.yaml'),
        `bindings:\n  - module_id: "dir.alpha"\n    target: "${modPath}:alpha"\n`,
      );
      writeTempYaml(
        join('bindings', 'beta.binding.yaml'),
        `bindings:\n  - module_id: "dir.beta"\n    target: "${modPath}:beta"\n`,
      );

      const results = await loader.loadBindingDir(bindDir, registry);
      expect(results).toHaveLength(2);
      expect(registry.has('dir.alpha')).toBe(true);
      expect(registry.has('dir.beta')).toBe(true);
    });

    it('returns empty array for directory with no binding files', async () => {
      const emptyDir = join(tmpDir, 'empty');
      mkdirSync(emptyDir);

      const results = await loader.loadBindingDir(emptyDir, registry);
      expect(results).toHaveLength(0);
    });
  });
});
