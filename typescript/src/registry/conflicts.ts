/**
 * ID conflict detection (Algorithm A03).
 */

/** The type of conflict detected. */
export type ConflictType = 'duplicate_id' | 'reserved_word' | 'case_collision';

/** The severity of a conflict. */
export type ConflictSeverity = 'error' | 'warning';

/** Result of an ID conflict check. */
export interface ConflictResult {
  readonly type: ConflictType;
  readonly severity: ConflictSeverity;
  readonly message: string;
}

/**
 * Check if a new module ID conflicts with existing IDs or reserved words (Algorithm A03).
 *
 * Steps:
 *   1. Exact duplicate detection.
 *   2. Reserved word detection (first segment).
 *   3. Case collision detection.
 *
 * @param newId - Canonical ID to be registered.
 * @param existingIds - Set of already registered IDs.
 * @param reservedWords - Reserved words that cannot be used as the first ID segment.
 * @param lowercaseMap - Optional pre-built lowercase-to-original_id mapping for O(1) case collision.
 * @returns ConflictResult if a conflict is found, null if the ID is safe.
 */
export function detectIdConflicts(
  newId: string,
  existingIds: ReadonlySet<string>,
  reservedWords: ReadonlySet<string>,
  lowercaseMap?: ReadonlyMap<string, string>,
): ConflictResult | null {
  // Step 1: Exact duplicate
  if (existingIds.has(newId)) {
    return {
      type: 'duplicate_id',
      severity: 'error',
      message: `Module ID '${newId}' is already registered`,
    };
  }

  // Step 2: Reserved word check (first segment only)
  const firstSegment = newId.split('.')[0];
  if (reservedWords.has(firstSegment)) {
    return {
      type: 'reserved_word',
      severity: 'error',
      message: `Module ID '${newId}' contains reserved word '${firstSegment}'`,
    };
  }

  // Step 3: Case collision
  const normalizedNew = newId.toLowerCase();
  if (lowercaseMap != null) {
    const existing = lowercaseMap.get(normalizedNew);
    if (existing != null && existing !== newId) {
      return {
        type: 'case_collision',
        severity: 'warning',
        message: `Module ID '${newId}' has a case collision with existing '${existing}'`,
      };
    }
  } else {
    for (const existingId of existingIds) {
      if (existingId.toLowerCase() === normalizedNew && existingId !== newId) {
        return {
          type: 'case_collision',
          severity: 'warning',
          message: `Module ID '${newId}' has a case collision with existing '${existingId}'`,
        };
      }
    }
  }

  return null;
}
