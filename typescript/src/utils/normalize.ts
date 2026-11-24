/**
 * Cross-language module ID normalization (Algorithm A02).
 */

const SEPARATORS: Record<string, string> = {
  python: '.',
  rust: '::',
  go: '.',
  java: '.',
  typescript: '.',
};

const SUPPORTED_LANGUAGES = new Set(Object.keys(SEPARATORS));

/**
 * Regex for splitting PascalCase / camelCase into words.
 * Handles transitions like: "Http" | "JSON" | "Parser" | "v2".
 */
const CASE_BOUNDARY = /(?<=[a-z0-9])(?=[A-Z])|(?<=[A-Z])(?=[A-Z][a-z])/g;

/** Canonical ID format from PROTOCOL_SPEC section 2.7 EBNF grammar. */
const CANONICAL_ID_RE = /^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$/;

function toSnakeCase(segment: string): string {
  if (!segment) return segment;

  // A02 is ASCII-scoped. Segments with no ASCII uppercase are left byte-for-byte
  // unchanged for grammar validation; this preserves existing underscores and
  // rejects out-of-alphabet characters instead of repairing them.
  if (!/[A-Z]/.test(segment)) {
    return segment;
  }

  // CASE_BOUNDARY and ASCII uppercase input make this an ASCII-only transform.
  const words = segment.split(CASE_BOUNDARY).filter(Boolean);
  return words.map((w) => w.replace(/[A-Z]/g, (ch) => ch.toLowerCase())).join('_');
}

/** Structured bare-name diagnostics from protocol-spec §2.2.1. */
export type CanonicalNameError = 'empty_name' | 'non_ascii' | 'invalid_start' | 'name_too_long';

/** The original name and either its canonical segment or a diagnostic. */
export interface CanonicalNameResult {
  readonly originalName: string;
  readonly canonicalName: string | null;
  readonly error: CanonicalNameError | null;
}

/**
 * Repair one ASCII name into a canonical segment without throwing.
 * Non-ASCII input is rejected before trimming or case conversion. Existing
 * underscores are preserved; punctuation runs become one underscore. Names
 * cannot begin with a digit or underscore, and are never prefixed or truncated.
 * Namespace reservation and collision detection belong to registration.
 */
export function canonicalizeName(name: string): CanonicalNameResult {
  if (/[^\x00-\x7f]/.test(name)) {
    return { originalName: name, canonicalName: null, error: 'non_ascii' };
  }
  const trimmed = name.replace(/^[^A-Za-z0-9_]+|[^A-Za-z0-9_]+$/g, '');
  const candidate = toSnakeCase(trimmed).replace(/[^A-Za-z0-9_]+/g, '_');
  let error: CanonicalNameError | null = null;
  if (candidate.length === 0) error = 'empty_name';
  else if (!/^[a-z]/.test(candidate)) error = 'invalid_start';
  else if (candidate.length > 192) error = 'name_too_long';
  return { originalName: name, canonicalName: error === null ? candidate : null, error };
}

/**
 * Convert a language-local module ID to Canonical ID format (Algorithm A02).
 *
 * Steps:
 *   1. Split by language-specific separator.
 *   2. Normalize each segment from PascalCase/camelCase to snake_case.
 *   3. Join with "." and validate against Canonical ID EBNF.
 *
 * @param localId - Language-local format ID (e.g. "executor::validator::DbParams").
 * @param language - Source language ("python" | "rust" | "go" | "java" | "typescript").
 * @returns Dot-separated snake_case Canonical ID.
 * @throws {Error} If language is unsupported or the result is not a valid Canonical ID.
 */
export function normalizeToCanonicalId(localId: string, language: string): string {
  if (!localId) {
    throw new Error('localId must be a non-empty string');
  }

  if (!SUPPORTED_LANGUAGES.has(language)) {
    const supported = [...SUPPORTED_LANGUAGES].sort().join(', ');
    throw new Error(`Unsupported language '${language}'. Must be one of: ${supported}`);
  }

  const separator = SEPARATORS[language];
  const segments = localId.split(separator);
  const normalized = segments.map(toSnakeCase);
  const canonicalId = normalized.join('.');

  if (!CANONICAL_ID_RE.test(canonicalId)) {
    throw new Error(
      `Normalized ID '${canonicalId}' (from '${localId}', language='${language}') ` +
      'does not conform to Canonical ID grammar',
    );
  }

  return canonicalId;
}
