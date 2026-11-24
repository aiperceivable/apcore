/**
 * Strict mode conversion for JSON Schemas (Algorithm A23).
 */

import { deepCopy } from '../utils/index.js';

export function toStrictSchema(schema: Record<string, unknown>): Record<string, unknown> {
  const result = deepCopy(schema);
  stripExtensions(result);
  convertToStrict(result);
  return result;
}

export function applyLlmDescriptions(node: unknown): void {
  if (typeof node !== 'object' || node === null || Array.isArray(node)) return;

  const obj = node as Record<string, unknown>;
  // Only substitute when a description already exists; never inject a
  // description onto a node that lacked one (Python/Rust parity — avoids
  // fabricating descriptions the spec did not declare).
  if ('x-llm-description' in obj && 'description' in obj) {
    obj['description'] = obj['x-llm-description'];
  }

  if ('properties' in obj && typeof obj['properties'] === 'object' && obj['properties'] !== null) {
    for (const prop of Object.values(obj['properties'] as Record<string, unknown>)) {
      applyLlmDescriptions(prop);
    }
  }
  if ('items' in obj && typeof obj['items'] === 'object') {
    applyLlmDescriptions(obj['items']);
  }
  if ('prefixItems' in obj && Array.isArray(obj['prefixItems'])) {
    for (const item of obj['prefixItems'] as unknown[]) {
      applyLlmDescriptions(item);
    }
  }
  for (const keyword of ['oneOf', 'anyOf', 'allOf']) {
    if (keyword in obj && Array.isArray(obj[keyword])) {
      for (const sub of obj[keyword] as unknown[]) {
        applyLlmDescriptions(sub);
      }
    }
  }
  for (const defsKey of ['definitions', '$defs']) {
    if (defsKey in obj && typeof obj[defsKey] === 'object' && obj[defsKey] !== null) {
      for (const defn of Object.values(obj[defsKey] as Record<string, unknown>)) {
        applyLlmDescriptions(defn);
      }
    }
  }
}

export function stripExtensions(node: unknown, stripDefaults: boolean = true, nameMap: boolean = false): void {
  if (typeof node !== 'object' || node === null || Array.isArray(node)) return;

  const obj = node as Record<string, unknown>;
  const keysToRemove = Object.keys(obj).filter(
    (k) => !nameMap && (k.startsWith('x-') || (stripDefaults && k === 'default')),
  );
  for (const k of keysToRemove) {
    delete obj[k];
  }

  for (const [key, value] of Object.entries(obj)) {
    if (typeof value === 'object' && value !== null) {
      if (Array.isArray(value)) {
        for (const item of value) {
          if (typeof item === 'object' && item !== null) {
            stripExtensions(item, stripDefaults);
          }
        }
      } else {
        stripExtensions(value, stripDefaults, !nameMap && ['properties', 'patternProperties', '$defs', 'definitions', 'dependentSchemas'].includes(key));
      }
    }
  }
}

/**
 * `true` when a `type` keyword declares "object", in either the string form
 * (`"object"`) or the union form (`["object", "null"]`).
 */
function declaresObjectType(typeValue: unknown): boolean {
  if (typeof typeValue === 'string') return typeValue === 'object';
  if (Array.isArray(typeValue)) return typeValue.some((member) => member === 'object');
  return false;
}

function convertToStrict(node: unknown): void {
  if (typeof node !== 'object' || node === null || Array.isArray(node)) return;

  const obj = node as Record<string, unknown>;

  // `type` may be a string ("object"), an array (["object", "null"] — what the
  // nullable wrapper below produces for an optional nested object), or absent
  // entirely: `properties` alone already implies an object schema, and
  // requiring a `type` keyword let `{"properties": {…}}` through unhardened,
  // which OpenAI structured outputs then rejects (A23 / spec §4.16).
  const isObjectWithProps =
    'properties' in obj && (!('type' in obj) || declaresObjectType(obj['type']));

  if (isObjectWithProps) {
    obj['additionalProperties'] = false;
    const existingRequired = new Set(
      Array.isArray(obj['required']) ? (obj['required'] as string[]) : [],
    );
    const properties = obj['properties'] as Record<string, unknown>;
    const allNames = Object.keys(properties);
    const optionalNames = allNames.filter((n) => !existingRequired.has(n));

    for (const name of optionalNames) {
      const prop = properties[name] as Record<string, unknown>;
      const constrained = ['enum', 'const', '$ref', 'allOf', 'anyOf', 'oneOf', 'not', 'if']
        .some((key) => key in prop);
      if (!('type' in prop) || constrained) {
        // Constraints can reject null independently of the type keyword.
        // Preserve the complete original schema in the non-null branch.
        properties[name] = { anyOf: [prop, { type: 'null' }] };
      } else if (typeof prop['type'] === 'string') {
        if (prop['type'] !== 'null') prop['type'] = [prop['type'], 'null'];
      } else if (Array.isArray(prop['type']) && !prop['type'].includes('null')) {
        prop['type'].push('null');
      }
    }

    obj['required'] = [...allNames].sort();
  }

  if ('properties' in obj && typeof obj['properties'] === 'object' && obj['properties'] !== null) {
    for (const prop of Object.values(obj['properties'] as Record<string, unknown>)) {
      convertToStrict(prop);
    }
  }
  if ('items' in obj && typeof obj['items'] === 'object') {
    convertToStrict(obj['items']);
  }
  // `prefixItems` — the Draft 2020-12 tuple form. Without this, an object
  // sitting at a tuple position was never hardened.
  if ('prefixItems' in obj && Array.isArray(obj['prefixItems'])) {
    for (const item of obj['prefixItems'] as unknown[]) {
      convertToStrict(item);
    }
  }
  for (const keyword of ['oneOf', 'anyOf', 'allOf']) {
    if (keyword in obj && Array.isArray(obj[keyword])) {
      for (const sub of obj[keyword] as unknown[]) {
        convertToStrict(sub);
      }
    }
  }
  for (const defsKey of ['definitions', '$defs']) {
    if (defsKey in obj && typeof obj[defsKey] === 'object' && obj[defsKey] !== null) {
      for (const defn of Object.values(obj[defsKey] as Record<string, unknown>)) {
        convertToStrict(defn);
      }
    }
  }
}
