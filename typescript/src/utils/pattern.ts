/**
 * Wildcard pattern matching.
 *
 * {@link matchPattern} (Algorithm A08) matches **module IDs** — ACL rule
 * patterns and pipeline `match_modules`. {@link matchGlob} (Algorithm A25)
 * matches every other pattern-valued value in the specification: binding
 * filenames, redaction field names, event types and
 * `system.control.reload_module`'s `path_filter`.
 *
 * The two are deliberately separate and PROTOCOL_SPEC §9.2.3 requirement 5
 * says why: A08 has `*` alone, and promoting `?` there would widen ACL
 * `allow` rules that are inert today (§2.7 forbids `?` in a module ID),
 * which is the one direction an authorization matcher must not move
 * silently. §6.2.2 closes that hole with a diagnostic instead.
 */

export function matchPattern(pattern: string, moduleId: string): boolean {
  if (pattern === '*') return true;
  if (!pattern.includes('*')) return pattern === moduleId;

  const segments = pattern.split('*');
  let pos = 0;

  if (!pattern.startsWith('*')) {
    if (!moduleId.startsWith(segments[0])) return false;
    pos = segments[0].length;
  }

  for (let i = 1; i < segments.length; i++) {
    const segment = segments[i];
    if (!segment) continue;
    const idx = moduleId.indexOf(segment, pos);
    if (idx === -1) return false;
    pos = idx + segment.length;
  }

  if (!pattern.endsWith('*')) {
    if (!moduleId.endsWith(segments[segments.length - 1])) return false;
  }

  return true;
}


/**
 * Match `value` against a glob-dialect `pattern` (Algorithm A25).
 *
 * PROTOCOL_SPEC §9.2.3. This is the matcher for every glob-dialect
 * pattern-valued value in the specification: `bindings.pattern`,
 * `obs.redaction.sensitive_keys` glob entries, event `event_pattern` /
 * `include_events` / `exclude_events`, and `path_filter`.
 *
 * Exactly two metacharacters:
 *
 * - `*` — zero or more characters, crossing `.` and `/`
 * - `?` — exactly one character
 *
 * **Every other character is a literal**, `[`, `]`, `{`, `}`, `\`, `!`, `^`
 * and `-` included. There is no escape character, and the match is anchored
 * to the whole value.
 *
 * **Do not implement this by translating to a RegExp.** That is how
 * `[!p]assword` came to mean "`!` or `p`" on the redaction surface, inverting
 * an operator's intent so that `password` was redacted and `bassword` leaked
 * (#117). Nor is it `fnmatch` or the Rust `glob` crate: those read `[…]` as a
 * character class, each with its own negation spelling. Every string is a
 * valid pattern here — there is no parse phase and this function never throws.
 *
 * @param pattern The pattern. Any string is accepted.
 * @param value The name to test — a filename, field name, event type or
 *   module ID, depending on the surface.
 * @returns True when the pattern matches the entire value.
 */
export function matchGlob(pattern: string, value: string): boolean {
  const segments = pattern.split('*');
  if (segments.length === 1) return matchExact(segments[0], value);

  if (!matchPrefix(segments[0], value)) return false;
  let pos = segments[0].length;

  for (let i = 1; i < segments.length - 1; i++) {
    const segment = segments[i];
    if (!segment) continue;
    const end = value.length - segment.length;
    let found = -1;
    for (let j = pos; j <= end; j++) {
      if (matchExact(segment, value.slice(j, j + segment.length))) {
        found = j;
        break;
      }
    }
    if (found === -1) return false;
    pos = found + segment.length;
  }

  const last = segments[segments.length - 1];
  if (!last) return true;
  if (value.length - pos < last.length) return false;
  return matchExact(last, value.slice(value.length - last.length));
}

/** True when `text` starts with `segment`, treating `?` as any character. */
function matchPrefix(segment: string, text: string): boolean {
  if (text.length < segment.length) return false;
  for (let k = 0; k < segment.length; k++) {
    if (segment[k] !== '?' && segment[k] !== text[k]) return false;
  }
  return true;
}

/** True when `segment` covers `text` exactly, treating `?` as any character. */
function matchExact(segment: string, text: string): boolean {
  return segment.length === text.length && matchPrefix(segment, text);
}

/**
 * Calculate the specificity score of an ACL pattern (Algorithm A10).
 *
 * Higher scores indicate more specific patterns. Scoring per segment:
 * - "*" (pure wildcard) -> 0
 * - Segment containing "*" (partial wildcard) -> +1
 * - Exact segment (no wildcard) -> +2
 *
 * @example
 * calculateSpecificity("*")                       // 0
 * calculateSpecificity("api.*")                   // 2
 * calculateSpecificity("api.handler.*")           // 4
 * calculateSpecificity("api.handler.task_submit") // 6
 */
export function calculateSpecificity(pattern: string): number {
  if (pattern === '*') return 0;

  let score = 0;
  for (const segment of pattern.split('.')) {
    if (segment === '*') {
      // +0
    } else if (segment.includes('*')) {
      score += 1;
    } else {
      score += 2;
    }
  }
  return score;
}
