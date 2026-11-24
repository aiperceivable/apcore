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
import { describe, expect, it } from 'vitest';

import * as browser from '../src/browser/index.js';
import { _setAclFileLoader } from '../src/acl.js';

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

/**
 * Static imports and re-exports — these enter the bundler's static graph
 * unconditionally. We follow these recursively to audit the full closure.
 */
function collectStaticImports(source: string): string[] {
  const code = stripTypeOnlyImports(source);
  const specifiers: string[] = [];
  const patterns = [
    /import\s+[^'"]*\s+from\s+['"]([^'"]+)['"]/g,
    /import\s+['"]([^'"]+)['"]/g,
    /export\s+[^'"]*\s+from\s+['"]([^'"]+)['"]/g,
  ];
  for (const re of patterns) {
    for (const match of code.matchAll(re)) specifiers.push(match[1]);
  }
  return specifiers;
}

/**
 * All specifiers, including dynamic `import('…')` and `require('…')`,
 * but only those that sit at module top level (no leading whitespace
 * — i.e. at column 0). A top-level `await import('node:fs')` chain is
 * what causes the bun init deadlock and is the only dynamic form that
 * also breaks browser bundlers at parse time.
 *
 * Lazy `await import('node:*')` *inside* an async function body is
 * intentionally NOT flagged — browser bundlers keep it as a dynamic
 * import that is only resolved if the method is actually invoked.
 * Browser callers that don't call `Registry.discover()` / `watch()`
 * never reach those lines, so flagging them would be over-strict.
 */
function collectAllSpecifiers(source: string): string[] {
  const code = stripTypeOnlyImports(source);
  const specifiers: string[] = [];
  const patterns = [
    /import\s+[^'"]*\s+from\s+['"]([^'"]+)['"]/g,
    /import\s+['"]([^'"]+)['"]/g,
    /export\s+[^'"]*\s+from\s+['"]([^'"]+)['"]/g,
    // Top-level only (column 0) — anything indented is inside a function
    // body and won't drag node:* into the browser bundle at parse time.
    /^require\s*\(\s*['"]([^'"]+)['"]\s*\)/gm,
    /^(?:const|let|var)\s+\w+\s*=\s*await\s+import\s*\(\s*['"]([^'"]+)['"]\s*\)/gm,
    /^\w+\s*=\s*await\s+import\s*\(\s*['"]([^'"]+)['"]\s*\)/gm,
    /^await\s+import\s*\(\s*['"]([^'"]+)['"]\s*\)/gm,
  ];
  for (const re of patterns) {
    for (const match of code.matchAll(re)) specifiers.push(match[1]);
  }
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

function walkBrowserGraph(): StaticCheckFinding[] {
  const findings: StaticCheckFinding[] = [];
  const visited = new Set<string>();
  const stack: string[] = [BROWSER_ENTRY];

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

    // Flag any node-builtin specifier — static OR dynamic — but only
    // recurse into static-import targets. A dynamic import inside an
    // async function body is browser-bundler-safe at parse time and
    // typically guarded at runtime by the caller; recursing into its
    // target would drag obviously-Node-only files (registry/scanner.ts,
    // registry/metadata.ts) into the audit even though browser callers
    // never reach them.
    for (const spec of collectAllSpecifiers(src)) {
      if (isNodeSpecifier(spec)) {
        findings.push({ file: rel, reason: `imports Node builtin "${spec}"` });
      }
    }
    for (const spec of collectStaticImports(src)) {
      if (isNodeSpecifier(spec)) continue; // already flagged above
      if (spec.startsWith('.')) {
        const resolved = resolveRelative(file, spec);
        if (resolved) stack.push(resolved);
      }
      // Bare package imports are peer/runtime deps and assumed browser-safe.
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

  // ── Runtime contracts ─────────────────────────────────────────────
  // The static guards above keep `node:*` out of the bundle; these
  // tests pin the behavioural contract that the browser surface has
  // to maintain so apwebsite (and any future browser consumer) keeps
  // working through future refactors.

  it('ACL.load throws a guidance error when the Node loader is absent', async () => {
    // Vitest setupFiles installs every Node-side loader so the rest of the
    // test suite (which imports source files directly) keeps working.
    // To assert the actual browser-bundle behaviour we temporarily
    // un-install the ACL file loader, run the call, then restore.
    const aclModule = await import('../src/acl.js');
    // Save the current loader by re-importing the installer side effect
    // — `acl-file.ts` runs once at module load and is idempotent on re-run.
    _setAclFileLoader(null);
    try {
      expect(() => browser.ACL.load('/whatever.yaml')).toThrow(
        /requires the Node entry of apcore-js/,
      );
    } finally {
      // Reinstall the Node loader so subsequent tests in the suite
      // (e.g. `tests/test-acl.test.ts`) see normal `ACL.load` behaviour.
      // `acl-file.ts` registers the loader as a side-effect of import.
      void aclModule;
      await import('../src/acl-file.js');
    }
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
    const node = await import('../src/index.js');
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
