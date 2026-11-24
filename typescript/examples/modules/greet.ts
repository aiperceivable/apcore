/**
 * Minimal example module: greet a user by name.
 *
 * Demonstrates the FunctionModule interface:
 * - inputSchema (TypeBox schema)
 * - outputSchema (TypeBox schema)
 * - execute function
 * - description string
 */

import { Type } from '@sinclair/typebox';
import { FunctionModule } from 'apcore-js';

export const greetModule = new FunctionModule({
  moduleId: 'greet',
  description: 'Greet a user by name',
  inputSchema: Type.Object({ name: Type.String() }),
  outputSchema: Type.Object({ message: Type.String() }),
  execute: (inputs) => ({ message: `Hello, ${inputs.name}!` }),
});
