/**
 * Load the co-located YAML binding and call it through the Executor.
 *
 * Run with:
 *   node examples/bindings/format-date/run.ts
 * (Node 22.6+ with --experimental-strip-types, or Node 23+; otherwise
 *  `npx tsx examples/bindings/format-date/run.ts`.)
 *
 * The counterpart of apcore-python's `examples/bindings/format_date/run.py`.
 * Python makes the target importable with `sys.path.insert(...)`; Node has no
 * equivalent for a bare specifier, so this script rewrites `binding.yaml`'s
 * `target` module path to the absolute path of `format-date.ts` and loads the
 * rewritten copy from a temp directory.
 */

import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import yaml from 'js-yaml';
import { BindingLoader, Executor, Registry } from 'apcore-js';

const here = dirname(fileURLToPath(import.meta.url));

// 1. Read the canonical binding and make its target resolvable by Node.
const doc = yaml.load(readFileSync(join(here, 'binding.yaml'), 'utf-8')) as {
  bindings: Array<Record<string, unknown>>;
};
for (const entry of doc.bindings) {
  const [, symbol] = (entry['target'] as string).split(':', 2);
  entry['target'] = `${join(here, 'format-date.ts')}:${symbol}`;
}

const workDir = mkdtempSync(join(tmpdir(), 'apcore-binding-'));
const bindingFile = join(workDir, 'format-date.binding.yaml');
writeFileSync(bindingFile, yaml.dump(doc), 'utf-8');

try {
  // 2. Load the binding into a registry. `auto_schema: true` picks up the
  //    `inputSchema` / `outputSchema` exports from format-date.ts.
  const registry = new Registry();
  const loader = new BindingLoader();
  const modules = await loader.loadBindings(bindingFile, registry);
  console.log(`Loaded ${modules.length} module(s): ${modules.map((m) => m.moduleId).join(', ')}`);

  // 3. Call it through the full executor pipeline.
  const executor = new Executor({ registry });
  const result = await executor.call('utils.format_date', {
    dateString: '2024-01-15',
    outputFormat: '%B %d, %Y',
  });
  console.log(result); // { formatted: 'January 15, 2024' }
} finally {
  rmSync(workDir, { recursive: true, force: true });
}
