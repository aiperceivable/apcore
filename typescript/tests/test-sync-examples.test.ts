/**
 * Cross-language sync regressions for the shipped examples.
 *
 * C6 — `examples/bindings/format-date/` must actually load and execute:
 *      the target function must use the `(inputs, context)` calling
 *      convention `BindingLoader` uses, and `auto_schema: true` must find
 *      exported `inputSchema` / `outputSchema`.
 * C7 — `examples/modules/*.ts` must typecheck: `ModuleAnnotations` is a
 *      total interface, so a partial object literal is a TS2739. The
 *      examples must go through `createAnnotations()`.
 */

import { describe, it, expect, beforeEach } from 'vitest';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import yaml from 'js-yaml';
import { BindingLoader } from '../src/bindings.js';
import { Registry } from '../src/registry/registry.js';
import { Executor } from '../src/executor.js';
import { DEFAULT_ANNOTATIONS } from '../src/module.js';

const REPO_ROOT = resolve(fileURLToPath(new URL('.', import.meta.url)), '..');
const BINDING_DIR = join(REPO_ROOT, 'examples', 'bindings', 'format-date');

describe('C6: the format-date YAML binding example loads and runs', () => {
  let tmp: string;
  let registry: Registry;
  let loader: BindingLoader;

  beforeEach(() => {
    tmp = mkdtempSync(join(tmpdir(), 'apcore-binding-example-'));
    registry = new Registry();
    loader = new BindingLoader();
  });

  /**
   * Materialize the shipped binding with an absolute target module path.
   *
   * Node's `import()` cannot resolve the bare specifier `format-date`, so the
   * example's own `run.ts` performs exactly this substitution. The test does
   * the same so it exercises the shipped YAML rather than a hand-written copy.
   */
  function materializeBinding(): string {
    const raw = readFileSync(join(BINDING_DIR, 'binding.yaml'), 'utf-8');
    const doc = yaml.load(raw) as { bindings: Array<Record<string, unknown>> };
    const entry = doc.bindings[0];
    const [modulePath, symbol] = (entry['target'] as string).split(':', 2);
    expect(modulePath).toBe('format-date');
    entry['target'] = `${join(BINDING_DIR, 'format-date.ts')}:${symbol}`;
    const out = join(tmp, 'format-date.binding.yaml');
    writeFileSync(out, yaml.dump(doc), 'utf-8');
    return out;
  }

  it('resolves the target, infers schemas from auto_schema, and executes', async () => {
    const bindingFile = materializeBinding();
    const modules = await loader.loadBindings(bindingFile, registry);

    expect(modules).toHaveLength(1);
    expect(modules[0].moduleId).toBe('utils.format_date');
    // auto_schema: true must have found real exported schemas, not the
    // permissive fallback.
    expect((modules[0].inputSchema as Record<string, unknown>)['properties']).toHaveProperty(
      'dateString',
    );
    expect((modules[0].outputSchema as Record<string, unknown>)['properties']).toHaveProperty(
      'formatted',
    );

    const executor = new Executor({ registry });
    const result = await executor.call('utils.format_date', {
      dateString: '2024-01-15',
      outputFormat: '%B %d, %Y',
    });
    expect(result).toEqual({ formatted: 'January 15, 2024' });

    rmSync(tmp, { recursive: true, force: true });
  });
});

describe('C7: example modules build total ModuleAnnotations', () => {
  it('get-user.ts exposes every required annotation field', async () => {
    const mod = (await import(join(REPO_ROOT, 'examples', 'modules', 'get-user.ts'))) as {
      getUserModule: { annotations: Record<string, unknown> | null };
    };
    const ann = mod.getUserModule.annotations;
    expect(ann).not.toBeNull();
    for (const key of Object.keys(DEFAULT_ANNOTATIONS)) {
      expect(ann).toHaveProperty(key);
    }
    expect(ann!['readonly']).toBe(true);
  });

  it('send-email.ts exposes every required annotation field', async () => {
    const mod = (await import(join(REPO_ROOT, 'examples', 'modules', 'send-email.ts'))) as {
      sendEmailModule: { annotations: Record<string, unknown> | null };
    };
    const ann = mod.sendEmailModule.annotations;
    expect(ann).not.toBeNull();
    for (const key of Object.keys(DEFAULT_ANNOTATIONS)) {
      expect(ann).toHaveProperty(key);
    }
    expect(ann!['destructive']).toBe(true);
  });

  it('send-email.ts marks its apiKey input as x-sensitive so redaction fires', async () => {
    const mod = (await import(join(REPO_ROOT, 'examples', 'modules', 'send-email.ts'))) as {
      sendEmailModule: { inputSchema: Record<string, unknown> };
    };
    const props = mod.sendEmailModule.inputSchema['properties'] as Record<
      string,
      Record<string, unknown>
    >;
    expect(props['apiKey']['x-sensitive']).toBe(true);
  });
});
