/**
 * Shared constants for the schema system.
 * Must be imported by both loader.ts (writes) and validator.ts (reads)
 * so that renaming either copy without the other causes a compile error.
 */

/** TypeBox schema property recording which JSON Schema keyword a union came
 * from — `'oneOf'` or `'anyOf'`. loader.ts writes it; validator.ts reads it to
 * apply the matching branch semantics (exhaustive counting for `oneOf`,
 * at-least-one for `anyOf`) and to report the specific union error codes.
 * A plain TypeBox `Type.Union` carries no marker and is treated as `anyOf`. */
export const KEYWORD_MARKER = 'x-apcore-keyword';
