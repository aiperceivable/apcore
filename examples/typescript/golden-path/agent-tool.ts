import { jsonSchema, tool } from 'ai';
import type { BoundModule } from 'apcore-js';

type Note = { noteId: string };

export function lookupTool(lookup: BoundModule<Note, Note>, wireInputSchema: Record<string, unknown>) {
  return tool({
    description: 'Look up a note',
    inputSchema: jsonSchema<Note>(wireInputSchema),
    execute: (args) => lookup.invoke(args),
  });
}
