/**
 * Global client example — module-level APCore instance for minimal boilerplate.
 */
import { Type } from '@sinclair/typebox';
import { APCore } from 'apcore-js';

// Module-level client — no explicit initialization needed
const client = new APCore();

// Register a module
client.module({
  id: 'math.add',
  description: 'Add two integers',
  inputSchema: Type.Object({ a: Type.Number(), b: Type.Number() }),
  outputSchema: Type.Object({ result: Type.Number() }),
  execute: (inputs) => ({ result: (inputs.a as number) + (inputs.b as number) }),
});

// Call directly
const result = await client.call('math.add', { a: 10, b: 5 });
console.log('Global call result:', result); // { result: 15 }
