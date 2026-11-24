/**
 * Multi-class module discovery: opt-in scanner, snake_case ID derivation, conflict detection.
 *
 * Implements PROTOCOL_SPEC §2.1.1 (Multi-Module Discovery).
 */

import { IdTooLongError, InvalidSegmentError, ModuleIdConflictError } from '../errors.js';

const SEGMENT_RE = /^[a-z][a-z0-9_]*$/;
const CANONICAL_ID_RE = /^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$/;
const MAX_MODULE_ID_LEN = 192;

export interface ClassDescriptor {
  readonly name: string;
  readonly implementsModule: boolean;
  /**
   * Per-class opt-in marker for multi-class mode (apcore decision-log D-06).
   *
   * When at least one qualifying class in the file has `multiClass: true`,
   * the discovery routine derives a distinct module ID per class. When no
   * qualifying class sets the flag, whole-file mode is used and the bare
   * base_id is returned.
   *
   * This field replaces the previous global `multiClassEnabled` parameter
   * on `Registry.discoverMultiClass`. See apcore commit 973410b for the
   * upstream cleanup that removed the dead `extensions.multi_class_discovery`
   * config toggle.
   */
  readonly multiClass?: boolean;
}

export interface MultiClassEntry {
  readonly moduleId: string;
  readonly className: string;
}

/**
 * Convert a class name to a snake_case segment per PROTOCOL_SPEC §2.1.1.
 *
 * Algorithm:
 * 1. Insert boundary at ALLCAPS→CamelCase transitions (HTTPSender → HTTP_Sender).
 * 2. Insert boundary at lowercase/digit→uppercase transitions (MathOps → Math_Ops).
 * 3. Replace every non-alphanumeric character with `_`.
 * 4. Lowercase.
 * 5. Collapse consecutive `_` to a single `_`.
 * 6. Strip leading and trailing `_`.
 */
export function classNameToSegment(className: string): string {
  let s = className.replace(/([A-Z]+)([A-Z][a-z])/g, '$1_$2');
  s = s.replace(/([a-z\d])([A-Z])/g, '$1_$2');
  s = s.replace(/[^a-zA-Z0-9]/g, '_');
  s = s.toLowerCase();
  s = s.replace(/_+/g, '_');
  return s.replace(/^_+|_+$/g, '');
}

function computeBaseId(filePath: string, extensionsRoot: string): string {
  const normalized = filePath.replace(/\\/g, '/');
  const parts = normalized.split('/');
  const rootIdx = parts.findIndex(p => p === extensionsRoot);
  const relParts = rootIdx === -1 ? [parts[parts.length - 1]] : parts.slice(rootIdx + 1);
  // Strip file extension from the last segment
  relParts[relParts.length - 1] = relParts[relParts.length - 1].replace(/\.[^.]+$/, '');
  return relParts.join('.');
}

/**
 * One-shot notice for the withdrawn `multiClassEnabled` argument, so a caller
 * still passing it learns the value is no longer consulted rather than
 * silently getting a different answer than before.
 */
let _multiClassEnabledArgWarned = false;

/** @internal exported for tests so they can reset between cases. */
export function _resetMultiClassEnabledArgWarned(): void {
  _multiClassEnabledArgWarned = false;
}

function warnMultiClassEnabledArg(): void {
  if (_multiClassEnabledArgWarned) return;
  _multiClassEnabledArgWarned = true;
  console.warn(
    '[apcore:multi-class] DEPRECATION: the `multiClassEnabled` argument to ' +
      '`discoverMultiClass()` is no longer consulted. Per-class markers are the ' +
      'only multi-class opt-in path (spec v1.50.0 D-107, decision-log D-06): ' +
      'set `multiClass: true` on each participating `ClassDescriptor`. The ' +
      'parameter will be removed at 2.0.',
  );
}

/**
 * Discover module IDs for classes in a single file under multi-class mode.
 *
 * Implements the Registry.discover_multi_class contract from PROTOCOL_SPEC §2.1.1.
 *
 * **Opt-in is per class** (spec v1.50.0 D-107, decision-log D-06): the file is
 * in multi-class mode when at least one QUALIFYING class carries
 * `multiClass: true`. Otherwise only the first qualifying class is used and the
 * bare base_id is returned.
 *
 * This used to gate on the `multiClassEnabled` boolean below and read the
 * `multiClass` field nowhere, while `Registry.discoverMultiClass` read the
 * field — so this repo shipped two doors with opposite defaults, and
 * multi-module-discovery.md's own TypeScript example (which passes three
 * arguments and marks both classes) silently returned one module where it
 * documents two. A file-level toggle also cannot express the case the feature
 * exists for: two participating classes beside a helper class that must not
 * become a module.
 *
 * In multi-class mode only the marked classes receive IDs, each
 * `base_id.segment` (D-147). A file with exactly one Module class keeps the
 * bare base_id — the single-class identity guarantee (§2.1.1 rule 5).
 *
 * @param multiClassEnabled - **Deprecated and ignored.** Retained so existing
 *   4-argument call sites keep compiling; passing it warns once per process.
 *
 * @internal Prefer `Registry.discoverMultiClass` (D-15) for the canonical
 * cross-language API surface; this free function is retained for backwards
 * compatibility and direct use by the scanner.
 *
 * @throws ModuleIdConflictError — two classes produce the same class_segment
 * @throws InvalidSegmentError   — a segment does not match ^[a-z][a-z0-9_]*$
 * @throws IdTooLongError        — a derived module_id exceeds 192 characters
 */
export function discoverMultiClass(
  filePath: string,
  classes: readonly ClassDescriptor[],
  extensionsRoot: string = 'extensions',
  multiClassEnabled?: boolean,
): MultiClassEntry[] {
  if (multiClassEnabled !== undefined) {
    warnMultiClassEnabledArg();
  }

  const qualifying = classes.filter(c => c.implementsModule);
  if (qualifying.length === 0) return [];

  const baseId = computeBaseId(filePath, extensionsRoot);

  // D-107: the per-class marker is the sole opt-in, resolved here exactly as
  // `Registry.discoverMultiClass` resolves it. A marker on a class that does
  // not implement Module opts nothing in.
  const enabled = qualifying.some(c => c.multiClass === true);
  if (!enabled) {
    return [{ moduleId: baseId, className: qualifying[0].name }];
  }

  if (qualifying.length === 1) {
    // Single-class identity guarantee (§2.1.1 rule 5): a file with exactly one
    // Module class yields the bare base_id whether or not it is marked.
    return [{ moduleId: baseId, className: qualifying[0].name }];
  }

  // Multi-class path (D-147): only marked classes receive IDs, and a marked
  // class's ID is always base_id.segment however many are marked — so marking
  // a second class never renames the first, and an unmarked helper that
  // implements Module is not registered.
  const seenSegments = new Map<string, string>(); // segment → className
  const results: MultiClassEntry[] = [];

  for (const cls of qualifying.filter(c => c.multiClass === true)) {
    const segment = classNameToSegment(cls.name);

    if (!SEGMENT_RE.test(segment)) {
      throw new InvalidSegmentError(filePath, cls.name, segment);
    }

    if (seenSegments.has(segment)) {
      console.warn(
        `[apcore:multi-class] MODULE_ID_CONFLICT in '${filePath}': ` +
        `classes '${seenSegments.get(segment)}' and '${cls.name}' both produce segment '${segment}'`,
      );
      throw new ModuleIdConflictError(filePath, [seenSegments.get(segment)!, cls.name], segment);
    }
    seenSegments.set(segment, cls.name);

    const moduleId = `${baseId}.${segment}`;

    if (!CANONICAL_ID_RE.test(moduleId)) {
      throw new InvalidSegmentError(filePath, cls.name, segment);
    }

    if (moduleId.length > MAX_MODULE_ID_LEN) {
      throw new IdTooLongError(filePath, moduleId);
    }

    results.push({ moduleId, className: cls.name });
  }

  return results;
}

/**
 * Internal alias of {@link discoverMultiClass}. Use {@link Registry.discoverMultiClass}
 * instead — this name is preserved so the scanner and other internal callers
 * have a stable reference that signals the surface is not part of the public
 * API.
 *
 * @internal
 */
export const _discoverMultiClass = discoverMultiClass;
