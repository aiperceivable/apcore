/**
 * User-facing before/after middleware — end-to-end demo.
 *
 * apcore is a standalone product: this example shows the middleware hooks an SDK
 * *user* writes — `client.useBefore(...)` and `client.useAfter(...)` — which wrap
 * the whole module call (distinct from the internal pipeline *step* middleware
 * shown in `pipeline-demo.ts`). It
 *
 *   1. registers a real `executor.math.add` tool,
 *   2. installs a before-hook that augments inputs (injects a default `b`) and an
 *      after-hook that transforms output (adds a `verified` flag), with both hooks
 *      appending to a shared ordered trace,
 *   3. calls the tool through `client.call(...)`, and
 *   4. prints the original vs. middleware-transformed result and the trace.
 *
 * The before-hook returns a replacement inputs object (or null to pass through);
 * the after-hook returns a replacement output object (same contract). See the
 * feature doc `docs/features/middleware-system.md` (§"Contract: Middleware.before/after").
 *
 * Each expectation is asserted, so this script doubles as a smoke test: it exits
 * non-zero if the hook order or the transformed output drifts.
 *
 * Run (from the apcore-typescript repo root):
 *     node examples/middleware.ts    # Node 23+ (or 22.6+ with --experimental-strip-types)
 *     npx tsx examples/middleware.ts  # any Node
 */

import { Type } from '@sinclair/typebox';
import { APCore } from 'apcore-js';

// A single ordered trace the tool and both hooks append to. Proving it ends as
// ["before", "execute", "after"] is the whole point: the before-hook ran before
// module execution and the after-hook ran after it.
const trace: string[] = [];

const DEFAULT_B = 22;

// ---------------------------------------------------------------------------
// 1. Register a real tool.
// ---------------------------------------------------------------------------
const client = new APCore();

client.module({
  id: 'executor.math.add',
  description: 'Add two integers',
  inputSchema: Type.Object({ a: Type.Number(), b: Type.Number() }),
  outputSchema: Type.Object({ result: Type.Number() }),
  execute: (inputs) => {
    trace.push('execute');
    return { result: (inputs.a as number) + (inputs.b as number) };
  },
});

// ---------------------------------------------------------------------------
// 2. User-facing before/after middleware.
// ---------------------------------------------------------------------------
// Observe + augment inputs: inject a default `b` when the caller omits it.
client.useBefore((_moduleId, inputs, _ctx) => {
  trace.push('before');
  if (!('b' in inputs)) {
    return { ...inputs, b: DEFAULT_B }; // replacement inputs
  }
  return inputs;
});

// Observe + transform output: stamp it as verified by middleware.
client.useAfter((_moduleId, _inputs, output, _ctx) => {
  trace.push('after');
  return { ...output, verified: true }; // replacement output
});

// ---------------------------------------------------------------------------
// 3. Call the tool through the pipeline and report.
// ---------------------------------------------------------------------------
async function main(): Promise<number> {
  console.log('User-facing before/after middleware — end-to-end demo');
  console.log('Tool: executor.math.add — before injects default b, after stamps verified');
  console.log('='.repeat(72));

  // Call with `b` omitted so the before-hook's injection is observable.
  const result = await client.call('executor.math.add', { a: 20 });

  console.log("  inputs (as called)   : { a: 20 }  (b omitted)");
  console.log(`  before injected      : b = ${DEFAULT_B}`);
  console.log(`  raw add result       : { result: ${20 + DEFAULT_B} }`);
  console.log(`  after-transformed    : ${JSON.stringify(result)}`);
  console.log(`  hook/execute trace   : ${JSON.stringify(trace)}`);
  console.log('='.repeat(72));

  const expectedTrace = ['before', 'execute', 'after'];
  const expectedOutput = { result: 42, verified: true };

  const checks: [string, boolean][] = [
    ['trace order is before -> execute -> after', JSON.stringify(trace) === JSON.stringify(expectedTrace)],
    ['before-hook injected default b', result.result === 42],
    ['after-hook stamped verified=true', result.verified === true],
    ['transformed output matches', JSON.stringify(result) === JSON.stringify(expectedOutput)],
  ];

  let failures = 0;
  for (const [label, ok] of checks) {
    if (!ok) failures += 1;
    console.log(`  ${ok ? 'PASS' : 'FAIL'}  ${label}`);
    if (!ok) console.log(`        ^ trace=${JSON.stringify(trace)} output=${JSON.stringify(result)}`);
  }

  console.log('='.repeat(72));
  const total = checks.length;
  console.log(`${total - failures}/${total} checks passed.`);
  return failures > 0 ? 1 : 0;
}

process.exit(await main());
