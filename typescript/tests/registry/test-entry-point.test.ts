import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { pathToFileURL } from 'node:url';
import { snakeToPascal, resolveEntryPoint } from '../../src/registry/entry-point.js';
import { ModuleLoadError } from '../../src/errors.js';

function validModuleSource(name: string): string {
  return `{
  inputSchema: { type: 'object' },
  outputSchema: { type: 'object' },
  description: '${name} module',
  execute: function() { return {}; },
}`;
}

describe('snakeToPascal', () => {
  it('returns empty string for empty input', () => {
    expect(snakeToPascal('')).toBe('');
  });

  it('capitalises a single word', () => {
    expect(snakeToPascal('hello')).toBe('Hello');
  });

  it('converts two-word snake_case', () => {
    expect(snakeToPascal('hello_world')).toBe('HelloWorld');
  });

  it('converts multi-word snake_case', () => {
    expect(snakeToPascal('my_module_name')).toBe('MyModuleName');
  });

  it('capitalises a word with no underscores', () => {
    expect(snakeToPascal('already')).toBe('Already');
  });
});

describe('resolveEntryPoint', () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = mkdtempSync(join(tmpdir(), 'apcore-entry-test-'));
  });

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true });
  });

  function fileUrl(filePath: string): string {
    return pathToFileURL(filePath).href;
  }

  it('throws ModuleLoadError for a non-existent file path', async () => {
    const badPath = fileUrl(join(tmpDir, 'does_not_exist.js'));
    await expect(resolveEntryPoint(badPath)).rejects.toThrow(ModuleLoadError);
  });

  it('finds default export that satisfies isModuleClass', async () => {
    const filePath = join(tmpDir, 'default_mod.mjs');
    writeFileSync(filePath, `const mod = ${validModuleSource('Default')};\nexport default mod;\n`);
    const result = await resolveEntryPoint(fileUrl(filePath));
    expect(result).toBeDefined();
    expect((result as Record<string, unknown>)['description']).toBe('Default module');
  });

  it('finds a single named export that satisfies isModuleClass', async () => {
    const filePath = join(tmpDir, 'named_mod.mjs');
    writeFileSync(filePath, `export const MyModule = ${validModuleSource('Named')};\n`);
    const result = await resolveEntryPoint(fileUrl(filePath));
    expect(result).toBeDefined();
    expect((result as Record<string, unknown>)['description']).toBe('Named module');
  });

  it('throws ModuleLoadError when no export satisfies isModuleClass', async () => {
    const filePath = join(tmpDir, 'no_valid.mjs');
    writeFileSync(filePath, `export const notAModule = { foo: 'bar' };\nexport const alsoNot = 42;\n`);
    await expect(resolveEntryPoint(fileUrl(filePath))).rejects.toThrow(ModuleLoadError);
    await expect(resolveEntryPoint(fileUrl(filePath))).rejects.toThrow(/No Module subclass found/);
  });

  it('throws ModuleLoadError when multiple exports satisfy isModuleClass', async () => {
    const filePath = join(tmpDir, 'ambiguous.mjs');
    writeFileSync(filePath, `export const ModuleA = ${validModuleSource('A')};\nexport const ModuleB = ${validModuleSource('B')};\n`);
    await expect(resolveEntryPoint(fileUrl(filePath))).rejects.toThrow(ModuleLoadError);
    await expect(resolveEntryPoint(fileUrl(filePath))).rejects.toThrow(/Ambiguous entry point/);
  });

  it('uses meta entry_point to find a specific named class', async () => {
    const filePath = join(tmpDir, 'meta_override.mjs');
    writeFileSync(filePath, `export const Alpha = ${validModuleSource('Alpha')};\nexport const Beta = ${validModuleSource('Beta')};\n`);
    const meta = { entry_point: 'some.path:Beta' };
    const result = await resolveEntryPoint(fileUrl(filePath), meta);
    expect((result as Record<string, unknown>)['description']).toBe('Beta module');
  });

  it('throws ModuleLoadError when meta entry_point class does not exist', async () => {
    const filePath = join(tmpDir, 'meta_missing.mjs');
    writeFileSync(filePath, `export const Alpha = ${validModuleSource('Alpha')};\n`);
    const meta = { entry_point: 'mod:NonExistent' };
    await expect(resolveEntryPoint(fileUrl(filePath), meta)).rejects.toThrow(ModuleLoadError);
    await expect(resolveEntryPoint(fileUrl(filePath), meta)).rejects.toThrow(/Entry point class 'NonExistent' not found/);
  });

  it('prefers default export over named exports when both are valid', async () => {
    const filePath = join(tmpDir, 'default_priority.mjs');
    writeFileSync(filePath, `const def = ${validModuleSource('DefaultPriority')};\nexport default def;\nexport const Named = ${validModuleSource('NamedSibling')};\n`);
    const result = await resolveEntryPoint(fileUrl(filePath));
    expect((result as Record<string, unknown>)['description']).toBe('DefaultPriority module');
  });

  it('falls back to auto-infer when meta is null', async () => {
    const filePath = join(tmpDir, 'null_meta.mjs');
    writeFileSync(filePath, `export const OnlyModule = ${validModuleSource('OnlyMeta')};\n`);
    const result = await resolveEntryPoint(fileUrl(filePath), null);
    expect((result as Record<string, unknown>)['description']).toBe('OnlyMeta module');
  });

  it('ignores exports missing execute function', async () => {
    const filePath = join(tmpDir, 'missing_execute.mjs');
    writeFileSync(filePath, `export const Incomplete = {\n  inputSchema: { type: 'object' },\n  outputSchema: { type: 'object' },\n  description: 'no execute',\n};\nexport const Complete = ${validModuleSource('Complete')};\n`);
    const result = await resolveEntryPoint(fileUrl(filePath));
    expect((result as Record<string, unknown>)['description']).toBe('Complete module');
  });

  it('ignores primitive and null exports', async () => {
    const filePath = join(tmpDir, 'primitives.mjs');
    writeFileSync(filePath, `export const aNumber = 42;\nexport const aString = 'hello';\nexport const aNull = null;\nexport const Valid = ${validModuleSource('Valid')};\n`);
    const result = await resolveEntryPoint(fileUrl(filePath));
    expect((result as Record<string, unknown>)['description']).toBe('Valid module');
  });
});
