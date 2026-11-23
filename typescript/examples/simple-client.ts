import { Type } from '@sinclair/typebox';
import { APCore } from 'apcore-js';

// 1. Initialize the simplified client
const client = new APCore();

// 2. Register modules
client.module({
  id: 'math.add',
  description: 'Add two integers',
  inputSchema: Type.Object({ a: Type.Number(), b: Type.Number() }),
  outputSchema: Type.Object({ sum: Type.Number() }),
  execute: (inputs) => ({ sum: (inputs.a as number) + (inputs.b as number) }),
});

client.module({
  id: 'greet',
  description: 'Greet a user by name',
  inputSchema: Type.Object({
    name: Type.String(),
    greeting: Type.Optional(Type.String()),
  }),
  outputSchema: Type.Object({ message: Type.String() }),
  execute: (inputs) => ({
    message: `${(inputs.greeting as string) || 'Hello'}, ${inputs.name}!`,
  }),
});

// 3. Call the modules
const result = await client.call('math.add', { a: 10, b: 5 });
console.log('Sync result:', result); // { sum: 15 }

const greetResult = await client.call('greet', { name: 'Alice' });
console.log('Greet result:', greetResult); // { message: 'Hello, Alice!' }
