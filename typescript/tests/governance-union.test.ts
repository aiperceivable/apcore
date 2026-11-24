/**
 * Every governance reader sees the same two sources (PROTOCOL_SPEC §7.4, D-96).
 *
 * D-96 settled that the approval gate fires on the UNION of the governance
 * sources, and recorded that it was "unobservable in implementations whose
 * descriptors are derived from the module (apcore-python, apcore-typescript),
 * which is why the union costs them nothing."
 *
 * **That was wrong, and this file is the proof.** These descriptors are not
 * derived — `mergeModuleMetadata` folds a `*_meta.yaml` / `metadata`
 * declaration into them with YAML > code precedence, so an operator has a
 * second place to declare governance that the module instance never carries.
 * Every governance reader here read the instance alone, so a metadata source
 * declaring `requires_approval: true` reached `getDefinition()` and the
 * manifest, and reached **no gate**: the module executed with an approval
 * handler configured and the handler never consulted.
 *
 * Why a union rather than `mergeAnnotations`' YAML > code precedence: the merge
 * lets the weaker declaration win in *both* directions. A metadata `false`
 * would cancel a module that asks to be gated, and a metadata `true` reached
 * only the descriptor. Both are fail-OPEN, and on an approval gate the
 * direction is the whole argument. The controls at the bottom pin both.
 */

import { describe, it, expect } from 'vitest';
import { Registry } from '../src/registry/registry.js';
import { Executor } from '../src/executor.js';
import { ManifestModule } from '../src/sys-modules/manifest.js';

const MODULE_ID = 'executor.probe.governance';

/** Declares no governance in code. The operator declares it in metadata. */
const plainModule = {
  inputSchema: { type: 'object' },
  outputSchema: { type: 'object' },
  description: 'declares nothing in code',
  execute: async () => ({ ran: true }),
};

/** Declares the requirement in code, so metadata must not be able to cancel it. */
const declaringModule = {
  ...plainModule,
  description: 'declares requiresApproval in code',
  annotations: { requiresApproval: true },
};

function countingHandler(): { handler: unknown; calls: () => number; seen: () => any } {
  let calls = 0;
  let seen: any = null;
  return {
    handler: {
      requestApproval: async (request: any) => {
        calls++;
        seen = request.annotations;
        return { status: 'approved', approvedBy: 'probe' };
      },
      checkApproval: async () => ({ status: 'approved' }),
    },
    calls: () => calls,
    seen: () => seen,
  };
}

function executorOver(registry: Registry, handler?: unknown): Executor {
  const ex = new Executor({ registry });
  if (handler !== undefined) ex.setApprovalHandler(handler as never);
  return ex;
}

describe('metadata-declared governance is enforced (D-96)', () => {
  it('the gate fires on a metadata-declared requirement', async () => {
    const registry = new Registry();
    registry.register(MODULE_ID, plainModule, undefined, {
      annotations: { requires_approval: true },
    });
    const h = countingHandler();

    await executorOver(registry, h.handler).call(MODULE_ID, {});

    // The bypass: before this fix the module executed with a handler
    // configured and the handler was never consulted.
    expect(h.calls()).toBe(1);
  });

  it('the request carries the metadata-declared destructive flag', async () => {
    const registry = new Registry();
    registry.register(MODULE_ID, plainModule, undefined, {
      annotations: { requires_approval: true, destructive: true },
    });
    const h = countingHandler();

    await executorOver(registry, h.handler).call(MODULE_ID, {});

    // A handler routing by risk takes the low-risk path when told
    // destructive=false for a call the operator marked high-risk.
    expect(h.seen()?.destructive).toBe(true);
  });

  it('preflight reports a metadata-declared requirement', async () => {
    const registry = new Registry();
    registry.register(MODULE_ID, plainModule, undefined, {
      annotations: { requires_approval: true },
    });

    const report = await executorOver(registry).validate(MODULE_ID, {});

    // §7.9.5: reporting false sends the caller into a gate it was told
    // would not fire.
    expect((report as any).requiresApproval).toBe(true);
  });

  it('governanceState reads a metadata-declared requirement', () => {
    const registry = new Registry();
    registry.registerInternal('system.control.probe', plainModule);
    // The merged slot is the operator's door; reach it the way the registry
    // itself records it.
    (registry as any)._moduleMeta.get('system.control.probe')['annotations'] = {
      requiresApproval: true,
    };

    // A serve-time adapter may refuse to start over this flag.
    expect(executorOver(registry).governanceState().allControlModulesRequireApproval).toBe(true);
  });

  it('the manifest advertises what the gate enforces', () => {
    // The discriminating case for this SDK is a metadata `false` over a code
    // `true`: `mergeAnnotations` gives the descriptor `false` by YAML > code
    // precedence, while the gate — reading the union — fires. A manifest
    // projected off the descriptor alone advertised `requires_approval: false`
    // for a call that will be stopped.
    const registry = new Registry();
    registry.register(MODULE_ID, declaringModule, undefined, {
      annotations: { requires_approval: false },
    });
    expect(registry.getDefinition(MODULE_ID)?.annotations?.requiresApproval).toBe(false);

    const entry = new ManifestModule(registry).execute({ module_id: MODULE_ID }, null as never) as any;

    expect(entry.annotations['requires_approval']).toBe(true);
  });
});

describe('the union is a union', () => {
  it('a metadata false does not cancel a code-declared requirement', async () => {
    const registry = new Registry();
    registry.register(MODULE_ID, declaringModule, undefined, {
      annotations: { requires_approval: false },
    });
    const h = countingHandler();

    await executorOver(registry, h.handler).call(MODULE_ID, {});

    expect(h.calls()).toBe(1);
  });

  it('no gate when neither source declares one', async () => {
    // The control — without it a hardcoded `true` would pass every test above.
    const registry = new Registry();
    registry.register(MODULE_ID, plainModule);
    const h = countingHandler();

    await executorOver(registry, h.handler).call(MODULE_ID, {});

    expect(h.calls()).toBe(0);
    expect((await executorOver(registry).validate(MODULE_ID, {}) as any).requiresApproval).toBe(false);
  });
});
