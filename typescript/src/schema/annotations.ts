/**
 * Annotation conflict resolution — merge YAML and code metadata.
 */

import type { ModuleAnnotations, ModuleExample } from '../module.js';
import { DEFAULT_ANNOTATIONS } from '../module.js';

const ANNOTATION_FIELDS: ReadonlyArray<keyof ModuleAnnotations> = [
  'readonly',
  'destructive',
  'idempotent',
  'requiresApproval',
  'openWorld',
  'streaming',
  'cacheable',
  'cacheTtl',
  'cacheKeyFields',
  'paginated',
  'paginationStyle',
  'discoverable',
  'extra',
];

/**
 * Wire (snake_case) annotation key -> `ModuleAnnotations` struct field.
 *
 * This is the same key set as `KNOWN_WIRE_KEYS` / `annotationsToJSON` in
 * `../module.ts`; it is spelled as a map here because the merge needs the
 * translation, not just membership. `annotationsFromJSON` is not reused
 * because it folds unknown keys into `extra` as legacy overflow, which is
 * correct for a §4.4.1 wire payload and wrong for a `*_meta.yaml` override
 * layer — an unknown key there is a typo and must stay ignored.
 */
const WIRE_TO_FIELD: Readonly<Record<string, keyof ModuleAnnotations>> = Object.freeze({
  readonly: 'readonly',
  destructive: 'destructive',
  idempotent: 'idempotent',
  requires_approval: 'requiresApproval',
  open_world: 'openWorld',
  streaming: 'streaming',
  cacheable: 'cacheable',
  cache_ttl: 'cacheTtl',
  cache_key_fields: 'cacheKeyFields',
  paginated: 'paginated',
  pagination_style: 'paginationStyle',
  discoverable: 'discoverable',
  extra: 'extra',
});

/** Reverse of {@link WIRE_TO_FIELD}, used only to name the portable spelling in a warning. */
const FIELD_TO_WIRE: Readonly<Record<string, string>> = Object.freeze(
  Object.fromEntries(Object.entries(WIRE_TO_FIELD).map(([wire, field]) => [field, wire])),
);

/**
 * Union the two governance sources a module's requirement can come from.
 *
 * PROTOCOL_SPEC §7.4 D-96: the approval gate fires when *either* the live
 * module instance or the registry's declared (descriptor) annotations ask for
 * it. Only `requiresApproval` and `destructive` are unioned; every other field
 * describes behaviour rather than governance and is taken from the live
 * instance, which is authoritative for it.
 *
 * Why a union and not {@link mergeAnnotations}. That function implements
 * YAML > code > defaults, which is right for a DESCRIPTOR — the operator's
 * document is the more specific statement about what a module is. It is wrong
 * for a gate, because it lets the weaker declaration win in both directions: a
 * YAML `requires_approval: false` would cancel a module that asks to be gated,
 * and a YAML `requires_approval: true` reached only the descriptor while the
 * gate read the instance and let the call through UNGATED. Both are fail-OPEN,
 * and on an approval gate the direction is the whole argument — requiring an
 * approval that was not strictly needed costs a prompt, skipping one that was
 * needed is a bypass.
 *
 * Returns `null` only when neither source exists. Accepts the wire dict shape
 * hosts sometimes set, as the rest of this module does.
 */
export function governanceUnion(
  moduleAnnotations: unknown,
  declaredAnnotations: unknown,
): ModuleAnnotations | null {
  const mod = coerceAnnotations(moduleAnnotations);
  const declared = coerceAnnotations(declaredAnnotations);
  if (mod === null && declared === null) return null;
  if (declared === null) return mod;
  if (mod === null) return declared;
  return {
    ...mod,
    requiresApproval: Boolean(mod.requiresApproval) || Boolean(declared.requiresApproval),
    destructive: Boolean(mod.destructive) || Boolean(declared.destructive),
  };
}

/** Accept a `ModuleAnnotations`, the wire dict shape, or null/undefined. */
function coerceAnnotations(annotations: unknown): ModuleAnnotations | null {
  if (annotations == null || typeof annotations !== 'object') return null;
  const obj = annotations as Record<string, unknown>;
  // Already a struct when it spells a field the wire shape does not.
  if ('requiresApproval' in obj || 'openWorld' in obj || 'cacheTtl' in obj) {
    return annotations as ModuleAnnotations;
  }
  return mergeAnnotations(obj, null);
}

export function mergeAnnotations(
  yamlAnnotations: Record<string, unknown> | null | undefined,
  codeAnnotations: ModuleAnnotations | null | undefined,
): ModuleAnnotations {
  const values: Record<string, unknown> = {};
  for (const f of ANNOTATION_FIELDS) {
    values[f] = DEFAULT_ANNOTATIONS[f];
  }

  if (codeAnnotations != null) {
    for (const f of ANNOTATION_FIELDS) {
      values[f] = codeAnnotations[f];
    }
  }

  if (yamlAnnotations != null) {
    // MOD-001: match on the WIRE spelling.
    //
    // `loadMetadata` returns the `*_meta.yaml` mapping un-normalized, so its
    // keys are snake_case — the spelling protocol-spec.md's own canonical
    // `*_meta.yaml` example uses (`requires_approval:` / `open_world:`).
    // This loop used to test them against `ANNOTATION_FIELDS`, which holds the
    // camelCase STRUCT field names, so five of thirteen fields ignored the
    // metadata file entirely while the other eight survived only because
    // their two spellings coincide. §4.13 makes that file the highest-priority
    // layer (a MUST), and the most consequential of the five silently dropped
    // was `requires_approval`. apcore-python matches its snake_case dataclass
    // field names and apcore-rust overlays the raw keys, so both already
    // carried the YAML through.
    //
    // The camelCase spelling is still accepted, because for those five fields
    // it was this SDK's ONLY working spelling and a project may have written
    // it — but it warns, since such a file is inert on the other two SDKs.
    // When both spellings appear the wire one wins: it is the canonical form.
    const camelOnly: Record<string, unknown> = {};
    const wire: Record<string, unknown> = {};
    for (const [key, val] of Object.entries(yamlAnnotations)) {
      const field = WIRE_TO_FIELD[key];
      if (field !== undefined) {
        wire[field] = val;
      } else if ((ANNOTATION_FIELDS as readonly string[]).includes(key)) {
        camelOnly[key] = val;
        console.warn(
          `[apcore:annotations] Annotation key '${key}' in module metadata is a ` +
            `TypeScript-only spelling and is ignored by apcore-python and ` +
            `apcore-rust. Use the wire spelling '${FIELD_TO_WIRE[key] ?? key}' ` +
            `(PROTOCOL_SPEC §4.4.1).`,
        );
      }
    }
    Object.assign(values, camelOnly, wire);
  }

  return sanitizeAnnotationValues(values) as unknown as ModuleAnnotations;
}

/**
 * D-115: a malformed annotation value is TOLERATED and dropped, the rest
 * survives, and it warns.
 *
 * `extra` is declared an object; a string there is neither an object nor a
 * reason to discard the module. Keeping the string is worse than lenient — it
 * then reads as a real declaration to everything downstream, which is invented
 * data indistinguishable from a declaration the author actually wrote. And
 * `cacheTtl: -5` silently survived, so a negative TTL reached the cache layer.
 *
 * Applied at the single point every merge returns through, rather than at each
 * caller: the same value arriving by a second door is how this kind of
 * tolerance ends up existing in one path and not the other.
 */
function sanitizeAnnotationValues(values: Record<string, unknown>): Record<string, unknown> {
  const extra = values['extra'];
  if (extra !== undefined && (typeof extra !== 'object' || extra === null || Array.isArray(extra))) {
    console.warn(
      `[apcore:annotations] ModuleAnnotations.extra must be an object, got ` +
        `${Array.isArray(extra) ? 'array' : typeof extra}; dropping it (D-115).`,
    );
    values['extra'] = {};
  }
  const ttl = values['cacheTtl'];
  if (typeof ttl !== 'number' || !Number.isInteger(ttl)) {
    if (ttl !== undefined) {
      console.warn(
        `[apcore:annotations] cacheTtl must be an integer, got ${typeof ttl}; ` +
          `dropping it (D-115).`,
      );
    }
    values['cacheTtl'] = 0;
  } else if (ttl < 0) {
    console.warn(`[apcore:annotations] cacheTtl ${ttl} is negative, clamping to 0 (D-115).`);
    values['cacheTtl'] = 0;
  }
  return values;
}

export function mergeExamples(
  yamlExamples: Array<Record<string, unknown>> | null | undefined,
  codeExamples: ModuleExample[] | null | undefined,
): ModuleExample[] {
  if (yamlExamples != null) {
    return yamlExamples.map((d) => ({
      title: d['title'] as string,
      inputs: (d['inputs'] as Record<string, unknown>) ?? {},
      output: (d['output'] as Record<string, unknown>) ?? {},
      description: d['description'] as string | undefined,
    }));
  }
  if (codeExamples != null) return codeExamples;
  return [];
}

export function mergeMetadata(
  yamlMetadata: Record<string, unknown> | null | undefined,
  codeMetadata: Record<string, unknown> | null | undefined,
): Record<string, unknown> {
  const result = codeMetadata != null ? { ...codeMetadata } : {};
  if (yamlMetadata != null) {
    Object.assign(result, yamlMetadata);
  }
  return result;
}
