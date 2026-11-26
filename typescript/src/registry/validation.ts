/**
 * Module validation for the registry system.
 *
 * In TypeScript, modules use TypeBox TSchema (which are plain objects) instead of
 * Pydantic BaseModel classes. Duck-type checks validate that the module has
 * inputSchema, outputSchema, description, and execute.
 */

export function validateModule(
  moduleOrClass: unknown,
  { requireMetadata = true, allowStaticSchemas = true } = {},
): string[] {
  if (moduleOrClass === null || (typeof moduleOrClass !== 'object' && typeof moduleOrClass !== 'function')) {
    return ['Module must be an object'];
  }
  const errors: string[] = [];
  const obj = moduleOrClass as Record<string, unknown>;
  const moduleConstructor = allowStaticSchemas
    ? obj.constructor as unknown as Record<string, unknown>
    : undefined;

  // Check inputSchema
  const inputSchema = obj['inputSchema'] ?? moduleConstructor?.['inputSchema'];
  if ((inputSchema == null && requireMetadata) ||
      (inputSchema != null && (typeof inputSchema !== 'object' || Array.isArray(inputSchema)))) {
    errors.push('Missing or invalid inputSchema: must be a TSchema object');
  }

  // Check outputSchema
  const outputSchema = obj['outputSchema'] ?? moduleConstructor?.['outputSchema'];
  if ((outputSchema == null && requireMetadata) ||
      (outputSchema != null && (typeof outputSchema !== 'object' || Array.isArray(outputSchema)))) {
    errors.push('Missing or invalid outputSchema: must be a TSchema object');
  }

  // Check description
  const description = obj['description'];
  if ((description == null && requireMetadata) ||
      (description != null && (typeof description !== 'string' || description.length === 0))) {
    errors.push('Missing or empty description');
  }

  // Check execute
  const execute = obj['execute'];
  if (execute == null || typeof execute !== 'function') {
    errors.push('Missing execute method');
  }

  return errors;
}
