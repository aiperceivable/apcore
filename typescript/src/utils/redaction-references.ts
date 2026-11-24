/** Local-only reference resolution for sensitive-field redaction. */
interface RedactionSchemaResource {
  readonly rootSchema: Record<string, unknown>;
  readonly anchors: Map<string, Record<string, unknown>>;
}

interface SchemaScope {
  readonly schema: Record<string, unknown>;
  readonly baseUri: string;
  readonly resource: RedactionSchemaResource;
  readonly dialect: unknown;
}

export interface RedactionReferenceContext {
  readonly rootSchema: Record<string, unknown>;
  readonly rootUri: string;
  readonly resources: ReadonlyMap<string, RedactionSchemaResource>;
  readonly scopes: WeakMap<object, string>;
  readonly dialects: WeakMap<object, unknown>;
}

const DEFAULT_ROOT_URI = 'https://apcore.invalid/redaction-root';
const SCHEMA_MAPS = [
  '$defs',
  'definitions',
  'properties',
  'patternProperties',
  'dependentSchemas',
  'dependencies',
];
const SCHEMA_ARRAYS = ['anyOf', 'oneOf', 'allOf', 'prefixItems', 'items'];
const SINGLE_SCHEMAS = [
  'items',
  'additionalItems',
  'additionalProperties',
  'contains',
  'not',
  'if',
  'then',
  'else',
  'propertyNames',
  'unevaluatedProperties',
  'unevaluatedItems',
];

function isSchema(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function schemaChildren(
  schema: Record<string, unknown>,
  dialect: unknown,
): Record<string, unknown>[] {
  const draft7 = typeof dialect === 'string' && dialect.includes('/draft-07/');
  const draft2019 = typeof dialect === 'string' && dialect.includes('/2019-09/');
  const mapKeys = draft7
    ? ['definitions', 'properties', 'patternProperties', 'dependencies']
    : SCHEMA_MAPS.filter((key) => key !== 'definitions' && key !== 'dependencies');
  const arrayKeys =
    draft7 || draft2019
      ? SCHEMA_ARRAYS.filter((key) => key !== 'prefixItems')
      : SCHEMA_ARRAYS.filter((key) => key !== 'items');
  const singleKeys = SINGLE_SCHEMAS.filter(
    (key) =>
      (draft7 || draft2019 || key !== 'additionalItems') &&
      (!draft7 || !key.startsWith('unevaluated')),
  );
  const children: Record<string, unknown>[] = [];
  for (const key of singleKeys) if (isSchema(schema[key])) children.push(schema[key]);
  for (const key of mapKeys) {
    const map = schema[key];
    if (isSchema(map))
      for (const child of Object.values(map)) if (isSchema(child)) children.push(child);
  }
  for (const key of arrayKeys) {
    const array = schema[key];
    if (Array.isArray(array)) for (const child of array) if (isSchema(child)) children.push(child);
  }
  return children;
}

function resolveSchemaUri(reference: string, baseUri: string): URL | undefined {
  try {
    return new URL(reference, baseUri);
  } catch (_error) {
    // Invalid URI references cannot identify any indexed schema resource.
    return undefined;
  }
}

function withoutFragment(uri: URL): string {
  const clean = new URL(uri.href);
  clean.hash = '';
  return clean.href;
}

function indexSchemaScope(
  parent: SchemaScope,
  resources: Map<string, RedactionSchemaResource>,
): SchemaScope {
  const schema = parent.schema;
  let baseUri = parent.baseUri;
  let resource = parent.resource;
  const identifier = schema['$id'];
  const uri =
    typeof identifier === 'string' && schema !== parent.resource.rootSchema
      ? resolveSchemaUri(identifier, baseUri)
      : undefined;
  if (uri !== undefined) {
    baseUri = withoutFragment(uri);
    if (!uri.hash && baseUri !== parent.baseUri) {
      resource = { rootSchema: schema, anchors: new Map() };
      resources.set(baseUri, resource);
    } else if (uri.hash && !uri.hash.startsWith('#/')) {
      resource.anchors.set(uri.hash.slice(1), schema);
    }
  }
  for (const key of ['$anchor', '$dynamicAnchor']) {
    const anchor = schema[key];
    if (typeof anchor === 'string') resource.anchors.set(anchor, schema);
  }
  return { schema, baseUri, resource, dialect: schema['$schema'] ?? parent.dialect };
}

export function createRedactionReferenceContext(
  rootSchema: Record<string, unknown>,
): RedactionReferenceContext {
  const identifier = rootSchema['$id'];
  const rootUri = withoutFragment(
    resolveSchemaUri(typeof identifier === 'string' ? identifier : '', DEFAULT_ROOT_URI) ??
      new URL(DEFAULT_ROOT_URI),
  );
  const rootResource: RedactionSchemaResource = { rootSchema, anchors: new Map() };
  const resources = new Map([[rootUri, rootResource]]);
  const scopes = new WeakMap<object, string>();
  const dialects = new WeakMap<object, unknown>();
  const pending: SchemaScope[] = [
    {
      schema: rootSchema,
      baseUri: rootUri,
      resource: rootResource,
      dialect: rootSchema['$schema'],
    },
  ];
  const visited = new Set<object>();
  while (pending.length > 0) {
    const current = pending.pop();
    if (current === undefined || visited.has(current.schema)) continue;
    visited.add(current.schema);
    const scope = indexSchemaScope(current, resources);
    scopes.set(scope.schema, scope.baseUri);
    dialects.set(scope.schema, scope.dialect);
    for (const child of schemaChildren(scope.schema, scope.dialect))
      pending.push({ ...scope, schema: child });
  }
  return { rootSchema, rootUri, resources, scopes, dialects };
}

export function resolveRedactionReference(
  reference: string,
  context: RedactionReferenceContext,
  fromSchema = context.rootSchema,
): Record<string, unknown> | undefined {
  const uri = resolveSchemaUri(reference, context.scopes.get(fromSchema) ?? context.rootUri);
  if (uri === undefined) return undefined;
  const resource = context.resources.get(withoutFragment(uri));
  if (resource === undefined) return undefined;
  let pointer: string;
  try {
    pointer = decodeURIComponent(uri.hash.slice(1));
  } catch (_error) {
    // A malformed fragment cannot identify any local schema.
    return undefined;
  }
  if (pointer === '') return resource.rootSchema;
  if (!pointer.startsWith('/')) return resource.anchors.get(pointer);
  let target: unknown = resource.rootSchema;
  for (const token of pointer.slice(1).split('/')) {
    const key = token.replace(/~1/g, '/').replace(/~0/g, '~');
    if (
      target === null ||
      typeof target !== 'object' ||
      !Object.prototype.hasOwnProperty.call(target, key)
    )
      return undefined;
    target = (target as Record<string, unknown>)[key];
  }
  return isSchema(target) ? target : undefined;
}
