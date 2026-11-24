/**
 * Module validation for the registry system.
 *
 * In TypeScript, modules use TypeBox TSchema (which are plain objects) instead of
 * Pydantic BaseModel classes. Duck-type checks validate that the module has
 * inputSchema, outputSchema, description, and execute.
 */

export function validateModule(moduleOrClass: unknown): string[] {
  const errors: string[] = [];
  const obj = moduleOrClass as Record<string, unknown>;

  // Check inputSchema
  const inputSchema = obj['inputSchema'] ?? (obj.constructor as unknown as Record<string, unknown>)?.['inputSchema'];
  if (inputSchema == null || typeof inputSchema !== 'object') {
    errors.push('Missing or invalid inputSchema: must be a TSchema object');
  }

  // Check outputSchema
  const outputSchema = obj['outputSchema'] ?? (obj.constructor as unknown as Record<string, unknown>)?.['outputSchema'];
  if (outputSchema == null || typeof outputSchema !== 'object') {
    errors.push('Missing or invalid outputSchema: must be a TSchema object');
  }

  // Check description
  const description = obj['description'];
  if (!description || typeof description !== 'string') {
    errors.push('Missing or empty description');
  }

  // Check execute
  const execute = obj['execute'];
  if (execute == null || typeof execute !== 'function') {
    errors.push('Missing execute method');
  }

  return errors;
}
