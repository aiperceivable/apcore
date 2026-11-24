/**
 * Demonstrate the 11-step ExecutionStrategy pipeline.
 *
 * Three sections in one run:
 *   1. Introspection      — print the 11 default step names.
 *   2. Middleware tracing — register a StepMiddleware that logs entry,
 *                           exit, and per-step duration.
 *   3. Orchestration      — insertAfter() adds a custom AuditLogStep,
 *                           then replace() swaps it for a quieter one.
 *
 * Run: node examples/pipeline-demo.ts
 */

import { Type } from '@sinclair/typebox';
import {
  APCore,
  type ExecutionStrategy,
  type PipelineContext,
  type PipelineEngine,
  type PipelineState,
  type Step,
  type StepMiddleware,
  type StepResult,
} from 'apcore-js';

// The canonical 11-step pipeline defined by the apcore protocol spec.
// Anything not in this set is a user-inserted custom step.
const CANONICAL_STEPS: ReadonlySet<string> = new Set([
  'context_creation',
  'call_chain_guard',
  'module_lookup',
  'acl_check',
  'approval_gate',
  'middleware_before',
  'input_validation',
  'execute',
  'output_validation',
  'middleware_after',
  'return_result',
]);

const STEP_ROLES: Record<string, string> = {
  context_creation:  'create execution context, set global deadline',
  call_chain_guard:  'check call depth & repeat limits',
  module_lookup:     'resolve module from registry',
  acl_check:         'enforce access control (default-deny)',
  approval_gate:     'human approval gate (if required)',
  middleware_before: 'run before-middleware chain (in order)',
  input_validation:  'validate inputs against schema',
  execute:           'invoke the module',
  output_validation: 'validate output against schema',
  middleware_after:  'run after-middleware chain (reverse order)',
  return_result:     'finalize and return output',
};

function summarize(stepName: string, ctx: PipelineContext, result: unknown): string {
  const r = result as StepResult | undefined;
  switch (stepName) {
    case 'context_creation': {
      const cid = ctx.context.callerId ?? 'anonymous';
      const tid = (ctx.context.traceId ?? '').slice(0, 8);
      return `caller=${cid} trace_id=${tid}…`;
    }
    case 'module_lookup':
      return ctx.module ? `resolved module '${ctx.moduleId}'` : 'no module';
    case 'middleware_before':
      return `inputs=${JSON.stringify(ctx.inputs)}`;
    case 'input_validation':
      return `validated_inputs=${JSON.stringify(ctx.validatedInputs ?? null)}`;
    case 'execute':
      return `output=${JSON.stringify(ctx.output ?? null)}`;
    case 'output_validation':
      return `validated_output=${JSON.stringify(ctx.validatedOutput ?? null)}`;
    case 'return_result':
      return `returning ${JSON.stringify(ctx.validatedOutput ?? ctx.output ?? null)}`;
    default:
      return r?.explanation ?? r?.action ?? 'continue';
  }
}

// ── Section 2: a StepMiddleware that traces each step ────────────────────
class TracingMiddleware implements StepMiddleware {
  private starts = new Map<string, number>();
  private coreIdx = 0;
  private readonly strategy: ExecutionStrategy;

  constructor(strategy: ExecutionStrategy) {
    this.strategy = strategy;
  }

  beforeStep(stepName: string, _state: PipelineState): void {
    if (stepName === 'context_creation') {
      this.coreIdx = 0;
    }
    this.starts.set(stepName, performance.now());
    let label: string;
    let role: string;
    if (CANONICAL_STEPS.has(stepName)) {
      this.coreIdx += 1;
      label = `[${String(this.coreIdx).padStart(2)}/11]`;
      role = STEP_ROLES[stepName];
    } else {
      label = '[  +  ]';
      role = 'CUSTOM step inserted via insertAfter / replace';
    }
    console.log(`  ${label} ${stepName.padEnd(19)} — ${role}`);
  }

  afterStep(stepName: string, state: PipelineState, result: unknown): void {
    const start = this.starts.get(stepName) ?? performance.now();
    this.starts.delete(stepName);
    const elapsedMs = performance.now() - start;
    const detail = summarize(stepName, state.context, result);
    console.log(`          ✓ ${elapsedMs.toFixed(2).padStart(6)} ms · ${detail}`);
  }

  onStepError(stepName: string, _state: PipelineState, error: Error): null {
    console.log(`          ✗ ${error.name}: ${error.message}`);
    return null;
  }
}

// ── Section 3: a custom step inserted after output_validation ───────────
function makeAuditLogStep(): Step {
  return {
    name: 'audit_log',
    description: 'Emit an audit record after output validation.',
    removable: true,
    replaceable: true,
    async execute(ctx: PipelineContext): Promise<StepResult> {
      const callerId = ctx.context.callerId ?? 'anonymous';
      console.log(`    [audit] caller=${callerId} target=${ctx.moduleId} ok=true`);
      return { action: 'continue' };
    },
  };
}

function makeQuietAuditLogStep(): Step {
  return {
    name: 'audit_log',
    description: 'Quiet audit step (replacement demo).',
    removable: true,
    replaceable: true,
    async execute(_ctx: PipelineContext): Promise<StepResult> {
      return { action: 'continue', explanation: 'quiet audit recorded' };
    },
  };
}

function banner(title: string): void {
  console.log(`\n=== ${title} ===`);
}

function indexed(names: string[], showCustom = false): void {
  for (let i = 0; i < names.length; i++) {
    const isCustom = !CANONICAL_STEPS.has(names[i]);
    const tag = showCustom && isCustom ? '  ← CUSTOM (inserted)' : '';
    console.log(`  ${String(i + 1).padStart(2)}. ${names[i]}${tag}`);
  }
}

const client = new APCore();

client.module({
  id: 'math.add',
  description: 'Add two integers',
  inputSchema: Type.Object({ a: Type.Number(), b: Type.Number() }),
  outputSchema: Type.Object({ sum: Type.Number() }),
  execute: (inputs) => ({ sum: (inputs.a as number) + (inputs.b as number) }),
});

// Public accessors: `executor.currentStrategy` exposes the strategy; the
// pipeline engine is held privately by the Executor, so we reach for it via
// a typed cast — this keeps the example focused on the pipeline surface.
const strategy = client.executor.currentStrategy;
const engine = (client.executor as unknown as { _pipelineEngine: PipelineEngine })
  ._pipelineEngine;

// ── Section 1: Introspection ────────────────────────────────────────────
banner('Section 1: Introspection — the default 11-step pipeline');
const info = strategy.info();
console.log(`strategy: ${info.name}  (steps: ${info.stepCount})`);
indexed(strategy.stepNames());

// ── Section 2: Middleware tracing ───────────────────────────────────────
banner('Section 2: Middleware tracing — one call through 11 steps');
engine.addStepMiddleware(new TracingMiddleware(strategy));
const result = await client.call('math.add', { a: 10, b: 5 });
console.log(`result: ${JSON.stringify(result)}`);

// ── Section 3: Orchestration ────────────────────────────────────────────
banner('Section 3: Orchestration — insertAfter + replace');
strategy.insertAfter('output_validation', makeAuditLogStep());
const customCount = strategy.stepNames().filter((n) => !CANONICAL_STEPS.has(n)).length;
console.log(
  `after insertAfter: 11 standard + ${customCount} custom = ${strategy.steps.length} steps`,
);
indexed(strategy.stepNames(), true);

console.log('\ncalling with the inserted audit step:');
await client.call('math.add', { a: 2, b: 3 });

strategy.replace('audit_log', makeQuietAuditLogStep());
const idx = strategy.stepNames().indexOf('audit_log');
console.log(`\nafter replace: ${strategy.steps.length} steps (audit_log still at index ${idx})`);

console.log('\ncalling with the quiet replacement:');
await client.call('math.add', { a: 7, b: 9 });
