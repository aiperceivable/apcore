/**
 * Decorated function example: add two numbers.
 *
 * Demonstrates the `module()` function for creating FunctionModules
 * with a simple function as the execute handler.
 */

import { Type } from '@sinclair/typebox';
import { module } from 'apcore-js';

export const addModule = module({
  id: 'math.add',
  description: 'Add two integers',
  inputSchema: Type.Object({ a: Type.Number(), b: Type.Number() }),
  outputSchema: Type.Object({ sum: Type.Number() }),
  execute: (inputs) => ({ sum: (inputs.a as number) + (inputs.b as number) }),
});
