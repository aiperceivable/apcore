/**
 * Static guard for the browser entry point's import graph.
 *
 * Walks every relative import reachable from `src/browser/index.ts` and
 * fails CI if any module on the closure references a Node-only specifier
 * — `node:*`, the bare names `fs` / `path` / `os` / `crypto` / etc. —
 * either via static `import` / `export ... from` / `require(...)` / dynamic
 * `import('...')`. Also flags global `process.*` references since browsers
 * don't expose `process`.
 *
 * Lazy `await import('node:*')` inside an async function body still flags
 * here — the goal is to catch every reachable Node reference, not just
 * top-level ones, because a method that throws at runtime in browser is
 * worse UX than not exposing the method at all.
 *
 * Patterned after `apcore-toolkit-typescript/tests/browser-entry.test.ts`.
 */

import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Type } from '@sinclair/typebox';
import ts from 'typescript';
import { describe, expect, it } from 'vitest';

import * as browser from '../src/browser/index.js';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const SRC_ROOT = resolve(__dirname, '..', 'src');
const BROWSER_ENTRY = resolve(SRC_ROOT, 'browser', 'index.ts');

const NODE_BUILTINS = new Set([
  'fs', 'path', 'os', 'crypto', 'child_process', 'url', 'util',
  'stream', 'events', 'buffer', 'process', 'module', 'http', 'https',
  'net', 'tls', 'dns', 'zlib', 'readline', 'vm', 'worker_threads',
  'perf_hooks', 'assert', 'querystring', 'string_decoder',
]);

function isNodeSpecifier(spec: string): boolean {
  if (spec.startsWith('node:')) return true;
  if (NODE_BUILTINS.has(spec)) return true;
  if (spec.startsWith('fs/') || spec.startsWith('path/')) return true;
  return false;
}

/**
 * Strip pieces of source that should NOT be treated as real imports:
 *   - `import type ... from 'pkg'` / `export type ... from 'pkg'`
 *   - `typeof import('pkg')` (TS type expression)
 *   - `/* … *​/` block comments and `//` line comments (a JSDoc string
 *     literal containing `import { … } from '…'` is otherwise matched
 *     by the regexes below)
 */
function stripTypeOnlyImports(source: string): string {
  return source
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/(^|[^:'"`\\])\/\/[^\n]*/g, '$1')
    .replace(/import\s+type\b[^;]*?from\s+['"][^'"]+['"]\s*;?/g, '')
    .replace(/export\s+type\b[^;]*?from\s+['"][^'"]+['"]\s*;?/g, '')
    .replace(/typeof\s+import\s*\(\s*['"][^'"]+['"]\s*\)/g, 'typeof __ERASED__');
}

/** Follow static and literal dynamic edges, including lazy chunks emitted by bundlers. */
function collectImportSpecifiers(file: string, source: string): string[] {
  const parsed = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true);
  const specifiers: string[] = [];
  const visit = (node: ts.Node): void => {
    if (ts.isImportDeclaration(node) && !node.importClause?.isTypeOnly &&
        ts.isStringLiteralLike(node.moduleSpecifier)) {
      specifiers.push(node.moduleSpecifier.text);
    } else if (ts.isExportDeclaration(node) && !node.isTypeOnly &&
        node.moduleSpecifier && ts.isStringLiteralLike(node.moduleSpecifier)) {
      specifiers.push(node.moduleSpecifier.text);
    } else if (ts.isCallExpression(node) &&
        (node.expression.kind === ts.SyntaxKind.ImportKeyword ||
          (ts.isIdentifier(node.expression) && node.expression.text === 'require'))) {
      const [arg] = node.arguments;
      if (arg && ts.isStringLiteralLike(arg)) specifiers.push(arg.text);
    }
    ts.forEachChild(node, visit);
  };
  visit(parsed);
  return specifiers;
}

function resolveRelative(fromFile: string, spec: string): string | null {
  if (!spec.startsWith('.')) return null;
  const withoutJs = spec.endsWith('.js') ? spec.slice(0, -3) : spec;
  const base = resolve(dirname(fromFile), withoutJs);
  for (const candidate of [`${base}.ts`, `${base}/index.ts`]) {
    try {
      readFileSync(candidate, 'utf8');
      return candidate;
    } catch {
      // try next
    }
  }
  throw new Error(`Could not resolve relative import "${spec}" from ${fromFile}`);
}

interface StaticCheckFinding {
  file: string;
  reason: string;
}

function walkBrowserGraph(entry = BROWSER_ENTRY, forbiddenModules: readonly string[] = []): StaticCheckFinding[] {
  const findings: StaticCheckFinding[] = [];
  const visited = new Set<string>();
  const stack: string[] = [entry];

  while (stack.length > 0) {
    const file = stack.pop()!;
    if (visited.has(file)) continue;
    visited.add(file);

    const src = readFileSync(file, 'utf8');
    const rel = file.slice(SRC_ROOT.length + 1);

    // Strip line + block comments and type-only constructs before
    // pattern-matching to avoid false positives.
    const code = stripTypeOnlyImports(src)
      .replace(/\/\*[\s\S]*?\*\//g, '')
      .replace(/(^|[^:'"`\\])\/\/[^\n]*/g, '$1');

    if (/\bprocess\s*\./.test(code) || /\bprocess\.env\b/.test(code)) {
      findings.push({ file: rel, reason: 'references global `process`' });
    }

    // `createRequire` from `node:module` is the static import (caught
    // below) — but a polyfilled browser bundler that injects a stub for
    // `node:module` would still hand us a non-functional `require`. Flag
    // the call site explicitly.
    if (/\bcreateRequire\s*\(/.test(code)) {
      findings.push({ file: rel, reason: 'uses `createRequire`' });
    }

    for (const spec of collectImportSpecifiers(file, src)) {
      if (isNodeSpecifier(spec)) {
        findings.push({ file: rel, reason: `imports Node builtin "${spec}"` });
      } else if (forbiddenModules.includes(spec)) {
        findings.push({ file: rel, reason: `imports optional dependency "${spec}"` });
      } else {
        const resolved = resolveRelative(file, spec);
        if (resolved) stack.push(resolved);
      }
    }
  }

  return findings;
}

describe('browser entry point', () => {
  it('exposes the runtime symbols apwebsite imports', () => {
    // These are the symbols apwebsite's in-browser apcore-demo currently
    // imports from `apcore-js`. Adding to or trimming this list is fine,
    // but anything in here MUST resolve through the browser entry without
    // dragging Node-only code along.
    const expected = [
      'APCore',
      'Registry',
      'Executor',
      'AutoApproveHandler',
      'AlwaysDenyHandler',
      'createAnnotations',
      'DEFAULT_ANNOTATIONS',
      'FunctionModule',
      'module',
      'Context',
      'TraceContext',
      'createIdentity',
      'jsonSchemaToTypeBox',
      'ACL',
      'canonicalizeName',
    ];
    for (const name of expected) {
      expect(browser, `expected "${name}" to be exported from /browser`).toHaveProperty(name);
    }
  });

  it('does not leak Node-only symbols', () => {
    const nodeOnly = [
      'Config',
      'discoverConfigFile',
      'BindingLoader',
      'SchemaLoader',
      'RefResolver',
      'contentHash', // sync hash needs node:crypto — use contentHashAsync
    ];
    for (const name of nodeOnly) {
      expect(browser, `"${name}" must not be exported from /browser`).not.toHaveProperty(name);
    }
  });

  it('transitive dep graph has zero Node-only imports or globals', () => {
    const findings = walkBrowserGraph();
    const formatted = findings.map((f) => `  - ${f.file}: ${f.reason}`).join('\n');
    expect(findings, `Browser entry drags in Node-only code:\n${formatted}`).toEqual([]);
  });

  it('package root has no Node, YAML, or Node-conditioned UUID dependencies', () => {
    expect(walkBrowserGraph(resolve(SRC_ROOT, 'index.ts'), ['js-yaml', 'uuid'])).toEqual([]);
  });

  it('observability subpath has no Node-only imports or globals', () => {
    const entry = resolve(SRC_ROOT, 'observability', 'portable.ts');
    expect(walkBrowserGraph(entry)).toEqual([]);
  });

  // ── Runtime contracts ─────────────────────────────────────────────
  // The static guards above keep `node:*` out of the bundle; these
  // tests pin the behavioural contract that the browser surface has
  // to maintain so apwebsite (and any future browser consumer) keeps
  // working through future refactors.

  it('ACL file load gives portable callers the explicit configuration path', async () => {
    await import('../src/full.js');
    expect(() => browser.ACL.load('/whatever.yaml')).toThrow(/apcore-js\/config/);
  });

  it('Registry + FunctionModule + Executor + AutoApproveHandler runs end-to-end', async () => {
    const registry = new browser.Registry();
    const fm = new browser.FunctionModule({
      moduleId: 'demo.double',
      inputSchema: Type.Object({ n: Type.Number() }),
      outputSchema: Type.Object({ r: Type.Number() }),
      description: 'Double the input',
      execute: async (input: Record<string, unknown>) => {
        return { r: (input.n as number) * 2 };
      },
    });
    registry.register('demo.double', fm);

    const executor = browser.Executor.fromRegistry(
      registry,
      null,
      null,
      null,
      new browser.AutoApproveHandler(),
    );
    const result = await executor.call('demo.double', { n: 5 });
    expect(result).toEqual({ r: 10 });
  });

  it('contentHashAsync output matches the Node-only sync contentHash', async () => {
    // Cross-language parity (PROTOCOL_SPEC §schema-system §4.15.5):
    // sha256 of canonical-form JSON. WebCrypto must agree byte-for-byte
    // with `node:crypto` — any divergence breaks Node↔browser↔Python↔Rust
    // cache-key alignment.
    const node = await import('../src/full.js');
    const fixtures: unknown[] = [
      { a: 1, b: 2 },
      { z: 'last', a: 'first', m: [3, 1, 2] },
      { nested: { c: true, b: null, a: [{ inner: 'x' }] } },
      [],
      'plain string',
    ];
    for (const f of fixtures) {
      const sync = node.contentHash(f);
      const async_ = await browser.contentHashAsync(f);
      expect(async_, `digest mismatch for ${JSON.stringify(f)}`).toBe(sync);
    }
  });
});
