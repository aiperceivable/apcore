import { InvalidInputError } from '../errors.js';
import { ExportProfile, type SchemaDefinition } from './types.js';

/** Export protocol-neutral module schemas. */
export class SchemaExporter {
  export(
    schema: SchemaDefinition,
    profile: ExportProfile = ExportProfile.GENERIC,
  ): Record<string, unknown> {
    if (profile !== ExportProfile.GENERIC) {
      throw new InvalidInputError("Only the 'generic' export profile is supported by Core");
    }
    return this.exportGeneric(schema);
  }

  exportGeneric(schemaDef: SchemaDefinition): Record<string, unknown> {
    return {
      module_id: schemaDef.moduleId,
      description: schemaDef.description,
      input_schema: schemaDef.inputSchema,
      output_schema: schemaDef.outputSchema,
      definitions: schemaDef.definitions,
    };
  }
}
