import type { TSchema } from '@sinclair/typebox';
import { awaitExecution } from '../cancel.js';
import type { Context } from '../context.js';
import { InvalidInputError, ModuleTimeoutError, SchemaValidationError } from '../errors.js';
import { snapshotJsonObject } from '../policy-evaluation.js';
import type { PipelineContext } from '../pipeline.js';
import { jsonSchemaToTypeBox } from './loader-pure.js';

type Awaitable<T> = T | Promise<T>;
type Direction = 'input' | 'output';

export interface StandardSchemaIssue {
  readonly message: string;
  readonly path?: readonly (PropertyKey | { readonly key: PropertyKey })[];
}

type StandardResult<Output> =
  | { readonly value: Output; readonly issues?: undefined }
  | { readonly issues: readonly StandardSchemaIssue[] };

/** The structural Standard Schema v1 contract with optional Standard JSON Schema support. */
export interface StandardSchemaV1<Input = unknown, Output = Input> {
  readonly '~standard': {
    readonly version: 1;
    readonly vendor: string;
    readonly validate: (value: unknown) => Awaitable<StandardResult<Output>>;
    readonly types?: { readonly input: Input; readonly output: Output } | undefined;
    readonly jsonSchema?: {
      readonly input: (options: { readonly target: 'draft-2020-12' }) => Record<string, unknown>;
      readonly output: (options: { readonly target: 'draft-2020-12' }) => Record<string, unknown>;
    };
  };
}

export type StandardSchemaInput<S extends StandardSchemaV1> = NonNullable<S['~standard']['types']>['input'];
export type StandardSchemaOutput<S extends StandardSchemaV1> = NonNullable<S['~standard']['types']>['output'];

/** @internal Preserves Standard Schema issue details in the preflight result. */
export class StandardValidationError extends SchemaValidationError {}

const DRAFT_2020_12 = 'https://json-schema.org/draft/2020-12/schema';

/** @internal Prepare a stable public schema and compile its runtime equivalent once. */
export function prepareStandardSchema(
  schema: StandardSchemaV1,
  direction: Direction,
  explicit: Record<string, unknown> | undefined,
): { readonly document: Record<string, unknown>; readonly compiled: TSchema } {
  const standard = schema?.['~standard'];
  if (standard?.version !== 1 || typeof standard.vendor !== 'string' || typeof standard.validate !== 'function') {
    throw new InvalidInputError(`Typed ${direction} must implement Standard Schema v1`);
  }

  let document: Record<string, unknown>;
  try {
    const source = explicit ?? standard.jsonSchema?.[direction]({ target: 'draft-2020-12' });
    // Providers such as Zod attach non-enumerable adapter metadata to the
    // returned document. Export only JSON data, without retaining the provider.
    document = snapshotJsonObject(structuredClone(source)).arguments;
  } catch {
    throw new InvalidInputError(`Typed ${direction} requires a Draft 2020-12 JSON Schema. Supply ${direction}Schema explicitly if conversion is unavailable.`);
  }
  const draft = document['$schema'];
  if (draft !== undefined && draft !== DRAFT_2020_12) {
    throw new InvalidInputError(`Typed ${direction} JSON Schema must use Draft 2020-12`);
  }
  return { document, compiled: jsonSchemaToTypeBox(document) };
}

function standardIssue(issue: StandardSchemaIssue): Record<string, unknown> {
  const path = (issue.path ?? []).map(segment => {
    const key = typeof segment === 'object' ? segment.key : segment;
    return '/' + String(key).replaceAll('~', '~0').replaceAll('/', '~1');
  }).join('');
  return { path, message: issue.message, keyword: 'standardSchema' };
}

function validationFailure(direction: Direction, errors: Array<Record<string, unknown>>): StandardValidationError {
  const label = direction === 'input' ? 'Input' : 'Output';
  return new StandardValidationError(`${label} validation failed`, errors);
}

/** @internal Validate JSON data passing between a typed module and the runtime. */
export function declarationJsonObject(value: unknown, direction: Direction): Record<string, unknown> {
  try {
    const record = snapshotJsonObject(value).arguments;
    if (direction === 'input' && Object.hasOwn(record, '_approval_token')) throw new Error('Reserved property');
    return record;
  } catch {
    throw validationFailure(direction, [{ path: '', message: 'Expected a plain JSON object', keyword: 'type' }]);
  }
}

/** @internal Validate or normalize data using the vendor's structural contract. */
export async function validateStandard(
  schema: StandardSchemaV1,
  value: unknown,
  direction: Direction,
): Promise<Record<string, unknown>> {
  let result: StandardResult<unknown>;
  try {
    result = await schema['~standard'].validate(value);
  } catch {
    throw validationFailure(direction, [{ path: '', message: 'Standard Schema validator failed', keyword: 'standardSchema' }]);
  }
  if (result.issues !== undefined) throw validationFailure(direction, result.issues.map(standardIssue));
  return declarationJsonObject(result.value, direction);
}

interface DeclarationValidation {
  readonly inputSchema: TSchema;
  readonly outputSchema: TSchema;
  readonly input: StandardSchemaV1;
}

const declarations = new WeakMap<object, DeclarationValidation>();
const validated = new WeakMap<PipelineContext, { readonly module: unknown; readonly inputs: Record<string, unknown> }>();

/** @internal Bind validation to the exact registry instance. */
export function bindDeclaration(module: object, validation: DeclarationValidation): void {
  declarations.set(module, validation);
}

/** @internal Retrieve the internal adapter without reading user-supplied module properties. */
export function declarationOf(module: unknown): DeclarationValidation | undefined {
  return declarations.get(module as object);
}

/** @internal Normalize the invocation's detached inputs before final policy. */
export async function validateDeclarationInputs(
  ctx: PipelineContext,
  declaration: DeclarationValidation,
  inputs: Record<string, unknown>,
): Promise<Record<string, unknown>> {
  const module = ctx.module;
  const context: Context = ctx.context;
  const timeout = context.globalDeadline === null ? 0 : Math.ceil(context.globalDeadline * 1000 - Date.now());
  context.cancelToken?.check();
  if (context.globalDeadline !== null && timeout <= 0) throw new ModuleTimeoutError(ctx.moduleId, 0);
  const parsed = await awaitExecution(validateStandard(declaration.input, inputs, 'input'),
    context.cancelToken, ctx.moduleId, timeout, false);
  validated.set(ctx, { module, inputs: parsed });
  return parsed;
}

/** @internal Consume the invocation-owned inputs instead of using mutable pipeline fields. */
export function takeDeclarationInputs(ctx: PipelineContext): Record<string, unknown> {
  const binding = validated.get(ctx);
  validated.delete(ctx);
  if (!binding || binding.module !== ctx.module) {
    throw new InvalidInputError('Typed module execution requires built-in input validation');
  }
  return binding.inputs;
}
