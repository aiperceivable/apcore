/**
 * Schema type definitions and data structures for the apcore schema system.
 */

import type { TSchema } from '@sinclair/typebox';
import { SchemaValidationError } from '../errors.js';

export enum SchemaStrategy {
  YAML_FIRST = 'yaml_first',
  NATIVE_FIRST = 'native_first',
  YAML_ONLY = 'yaml_only',
}

export enum ExportProfile {
  MCP = 'mcp',
  OPENAI = 'openai',
  ANTHROPIC = 'anthropic',
  GENERIC = 'generic',
}

export interface SchemaDefinition {
  moduleId: string;
  description: string;
  inputSchema: Record<string, unknown>;
  outputSchema: Record<string, unknown>;
  errorSchema?: Record<string, unknown> | null;
  definitions: Record<string, unknown>;
  /**
   * Absolute path of the `*.schema.yaml` this definition was loaded from, or
   * null for a definition that never came off disk.
   *
   * Spec v1.50.0 D-104 makes this load-bearing: a local `#/…` reference
   * resolves its pointer against the FILE ROOT first, so the resolver has to
   * be told which file that is. Without it `#/definitions/User` could only
   * ever address inside the `input_schema` node, and §4.11's own example —
   * `definitions:` as a top-level sibling of `input_schema` — did not load.
   */
  sourcePath?: string | null;
  version: string;
  documentation?: string | null;
  schemaUrl?: string | null;
}

export interface ResolvedSchema {
  jsonSchema: Record<string, unknown>;
  schema: TSchema;
  moduleId: string;
  direction: string;
}

export interface SchemaValidationErrorDetail {
  path: string;
  message: string;
  keyword?: string;
  constraint?: string | null;
  expected?: unknown;
  actual?: unknown;
}

export interface SchemaValidationResult {
  valid: boolean;
  errors: SchemaValidationErrorDetail[];
  /** Set when format-annotated fields fail soft validation (SHOULD-level). */
  warnLogged?: boolean;
  /** Error code when validation fails (e.g. SCHEMA_UNION_NO_MATCH). */
  errorCode?: string | null;
}

export function validationResultToError(result: SchemaValidationResult): SchemaValidationError {
  if (result.valid) {
    throw new Error('Cannot convert valid result to error');
  }
  const errorDicts = result.errors.map((e) => ({
    path: e.path,
    message: e.message,
    keyword: e.keyword ?? e.constraint ?? 'type',
  }));
  return new SchemaValidationError(
    'Schema validation failed',
    errorDicts,
    undefined,
    result.errorCode ?? 'SCHEMA_VALIDATION_ERROR',
  );
}

export interface LLMExtensions {
  llmDescription?: string | null;
  examples?: unknown[] | null;
  sensitive: boolean;
  constraints?: string | null;
  deprecated?: Record<string, unknown> | null;
  sunsetDate?: string | null;
}
