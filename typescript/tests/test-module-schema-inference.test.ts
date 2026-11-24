import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';
import { describe, expect, it } from 'vitest';
import { APCore } from '../src/client.js';
import { inferModuleSchemas } from '../src/compiler.js';
import { module } from '../src/decorator.js';

function compile(source: string): string {
  const directory = mkdtempSync(join(tmpdir(), 'apcore-module-types-'));
  try {
    const fileName = join(directory, 'sample.ts');
    writeFileSync(fileName, source);
    const program = ts.createProgram([fileName], {
      target: ts.ScriptTarget.ES2022,
      module: ts.ModuleKind.CommonJS,
      strict: true,
      skipLibCheck: true,
      paths: { 'apcore-js': [fileURLToPath(new URL('../src/index.ts', import.meta.url))] },
    });
    let output = '';
    const sourceFile = program.getSourceFile(fileName);
    const diagnostics = program.getSemanticDiagnostics(sourceFile);
    if (diagnostics.length > 0) {
      throw new Error(diagnostics.map(d => ts.flattenDiagnosticMessageText(d.messageText, '\n')).join('\n'));
    }
    program.emit(sourceFile, (_file, content) => { output = content; }, undefined, false, {
      before: [inferModuleSchemas(program)],
    });
    return output;
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

describe('module schema inference', () => {
  it('generates schemas for named callables and executes the resulting module', async () => {
    const javascript = compile(`
      import { module as define } from 'apcore-js';
      interface Input { name: string; punctuation?: '!' | '?' }
      interface Output { greeting: string }
      async function greet(input: Input): Promise<Output> {
        return { greeting: input.name + (input.punctuation ?? '!') };
      }
      export const tool = define({ id: 'review.greet', execute: greet });
    `);
    const exports: Record<string, ReturnType<typeof module>> = {};
    const requireModule = () => ({ module });
    // Execute the compiler output for this fixed local test fixture.
    new Function('exports', 'require', javascript)(exports, requireModule);
    const client = new APCore();
    await client.register('review.greet', exports.tool);
    await expect(client.call('review.greet', { name: 'Ada' })).resolves.toEqual({ greeting: 'Ada!' });
    await expect(client.call('review.greet', { name: 12 })).rejects.toMatchObject({ code: 'SCHEMA_VALIDATION_ERROR' });
    await expect(client.call('review.greet', { name: 'Ada', punctuation: '.' })).rejects.toMatchObject({ code: 'SCHEMA_VALIDATION_ERROR' });
  });

  it('reports a missing type instead of emitting an unconstrained schema', () => {
    expect(() => compile(`
      import { module } from 'apcore-js';
      export const tool = module({ id: 'review.untyped', execute: (input: any) => ({ input }) });
    `)).toThrow(/FUNC_MISSING_TYPE_HINT/);
  });

  it('honors an explicit schema passed through spread options', async () => {
    const javascript = compile(`
      import { module } from 'apcore-js';
      const schemas = {
        inputSchema: { type: 'object', properties: { text: { type: 'string' } }, required: ['text'] },
      };
      export const tool = module({
        id: 'review.spread',
        ...schemas,
        execute: (input: any): { seen: string } => ({ seen: input.text }),
      });
    `);
    const exports: Record<string, ReturnType<typeof module>> = {};
    new Function('exports', 'require', javascript)(exports, () => ({ module }));
    const client = new APCore();
    await client.register('review.spread', exports.tool);
    await expect(client.call('review.spread', { text: 'ready' })).resolves.toEqual({ seen: 'ready' });
    await expect(client.call('review.spread', { text: 5 })).rejects.toMatchObject({
      code: 'SCHEMA_VALIDATION_ERROR',
    });
  });

  it('matches the runtime wire shape for unions containing records, scalars and null', async () => {
    const javascript = compile(`
      import * as apcore from 'apcore-js';
      export const tool = apcore.module({
        id: 'review.union',
        execute(input: { mode: 'record' | 'scalar' | 'empty' }): { value: number } | number | null {
          if (input.mode === 'record') return { value: 4 };
          if (input.mode === 'scalar') return 5;
          return null;
        },
      });
    `);
    const exports: Record<string, ReturnType<typeof module>> = {};
    new Function('exports', 'require', javascript)(exports, () => ({ module }));
    const client = new APCore();
    await client.register('review.union', exports.tool);
    await expect(client.call('review.union', { mode: 'record' })).resolves.toEqual({ value: 4 });
    await expect(client.call('review.union', { mode: 'scalar' })).resolves.toEqual({ result: 5 });
    await expect(client.call('review.union', { mode: 'empty' })).resolves.toEqual({});
  });
});
