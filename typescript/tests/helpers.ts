/**
 * Shared test fixtures and helpers.
 */

import { Type } from '@sinclair/typebox';
import { Context, createIdentity } from '../src/context.js';
import { FunctionModule } from '../src/decorator.js';
import { Registry } from '../src/registry/registry.js';

export function createTestModule(options?: {
  moduleId?: string;
  description?: string;
  execute?: (inputs: Record<string, unknown>, context: Context) => Record<string, unknown>;
}): FunctionModule {
  return new FunctionModule({
    execute: options?.execute ?? ((inputs) => ({ echo: inputs['name'] ?? 'world' })),
    moduleId: options?.moduleId ?? 'test.module',
    inputSchema: Type.Object({ name: Type.Optional(Type.String()) }),
    outputSchema: Type.Object({ echo: Type.String() }),
    description: options?.description ?? 'A test module',
  });
}

export function createTestRegistry(): Registry {
  return new Registry();
}

export function createTestContext(_executor?: unknown): Context {
  // Issue #66: executor is no longer a Context.create() parameter — the
  // Executor auto-binds itself on the first .call(). The `executor`
  // parameter is kept for backward call-site compatibility but ignored.
  return Context.create(createIdentity('test-user'));
}
