/**
 * Structured logging: ContextLogger, RedactionConfig, and ObsLoggingMiddleware.
 */

import type { Config } from '../config.js';
import type { Context } from '../context.js';
import { Middleware } from '../middleware/base.js';
import { matchGlob } from '../utils/pattern.js';
// `matchGlob` (Algorithm A25, PROTOCOL_SPEC 9.2.3) is the matcher for
// sensitive_keys glob entries; `matchPattern` (A08) is deliberately NOT used
// here — it matches module IDs and has no `?`.

/**
 * Default sensitive field patterns (Issue #45 §3 — canonical superset
 * promoted from #43 §5). Used when no `redaction.sensitive_keys`
 * (canonical) / `redaction.field_patterns` (legacy) is configured.
 * Wildcards follow apcore's `matchPattern` semantics (segment-aware globs).
 *
 * Aligned with apcore-python's authoritative list (Python is the canonical
 * superset because broader default redaction is safer than narrower).
 * `apiKey` is kept alongside `api_key` / `apikey` for camelCase parity
 * because TypeScript's `matchPattern` is case-sensitive.
 */
/**
 * Maximum recursion depth for nested redaction. Matches the spec's schema
 * validation depth limit (32) so the redactor can never run away on a
 * deeply-nested or cyclic structure.
 */
const MAX_REDACTION_DEPTH = 32;

export const DEFAULT_REDACTION_FIELD_PATTERNS: readonly string[] = [
  '_secret_*',
  'password',
  'passwd',
  'secret',
  'token',
  'api_key',
  'apikey',
  'apiKey',
  'access_key',
  'private_key',
  'authorization',
  'auth',
  'credential',
  'cookie',
  'session',
  'bearer',
];

/**
 * One-shot deprecation warning bookkeeping for legacy
 * `observability.redaction.*` keys (sync finding CRITICAL #4). Mirrors the
 * cross-language pattern — warn once per process when legacy keys are read,
 * pointing migrators at the canonical `obs.redaction.*` namespace.
 */
const _REDACTION_LEGACY_DEPRECATION_EMITTED: { value: boolean } = { value: false };

function _emitRedactionLegacyDeprecation(legacyKeys: string[]): void {
  if (_REDACTION_LEGACY_DEPRECATION_EMITTED.value) return;
  _REDACTION_LEGACY_DEPRECATION_EMITTED.value = true;
  console.warn(
    `[apcore] Config keys ${legacyKeys.join(', ')} are deprecated; ` +
      'use obs.redaction.sensitive_keys / obs.redaction.regex_patterns / obs.redaction.replacement instead. ' +
      'Legacy keys will be removed in a future release.',
  );
}

// ---------------------------------------------------------------------------
// RedactionConfig
// ---------------------------------------------------------------------------

const PROTECTED_LOG_FIELDS = new Set([
  'trace_id',
  'caller_id',
  'target_id',
  'module_id',
  'span_id',
]);

/**
 * Normalize a field name for substring matching: lowercase + collapse
 * hyphens / spaces to underscores. Mirrors apcore-python's
 * `_normalize_key_for_match` so `X-API-Key` matches `api_key`.
 */
function _normalizeKeyForMatch(s: string): string {
  return s.toLowerCase().replace(/[- ]/g, '_');
}

/**
 * Compact normalization: lowercase with hyphen / underscore / space
 * stripped entirely. Mirrors apcore-python's `_compact_for_match` so
 * camelCase keys like `AccessKey` substring-match the `access_key` pattern
 * (D-54 canonical default expectation).
 */
function _compactKeyForMatch(s: string): string {
  return s.toLowerCase().replace(/[-_ ]/g, '');
}

// `_globToRegExp` was removed in v1.37.0. It translated a sensitive_keys entry
// into a RegExp and passed `[...]` through VERBATIM, so `[!p]` meant "`!` or
// `p`" instead of "not `p`": `[!p]assword` redacted `password` — the one field
// the other two SDKs deliberately exclude — and leaked `bassword`, which they
// catch. The meaning was inverted, not merely weakened (#117 section 1).
// PROTOCOL_SPEC 9.2.3 makes `[` a literal and 10.6.1 routes such an entry
// through the substring branch instead, so it is inert rather than backwards.
// Use `matchGlob` (Algorithm A25) from utils/pattern.

/**
 * Compile `obs.redaction.regex_patterns` once, reporting the entries that will not.
 *
 * PROTOCOL_SPEC §10.6.1 requirement 4: a pattern the engine cannot compile
 * **MUST NOT** be discarded in silence. It was — the original code substituted
 * `/(?!)/`, a regular expression that can never match, and said nothing, so an
 * operator-authored redaction rule that redacts nothing looked exactly like one
 * that works. On this surface that difference is credentials in plaintext
 * (#117 §2). JavaScript is the engine that rejects the inline `(?i)` flag,
 * which the other two accept, so this fires on portable-looking patterns rather
 * than only on malformed ones.
 *
 * Requirement 5 is why this is a batch function holding no module state.
 * Compilation belongs at the point the *configuration* is read, and the
 * de-duplication that keeps the diagnostic to one line **MUST NOT outlive that
 * configuration**. The `Set` this replaced was module-level and never cleared,
 * so the SECOND configuration carrying the same broken pattern was told
 * nothing: the reload case and the multi-tenant case, which are the two where
 * an operator most needs telling.
 *
 * Already-compiled patterns pass through, so nothing is recompiled per record.
 *
 * @returns The compiled patterns, and `[pattern, message]` for each rejected one.
 */
function compileValuePatterns(
  patterns: readonly (RegExp | string)[],
): { compiled: RegExp[]; invalid: [string, string][] } {
  const compiled: RegExp[] = [];
  const invalid: [string, string][] = [];
  const reported = new Set<string>();
  for (const pattern of patterns) {
    if (pattern instanceof RegExp) {
      compiled.push(pattern);
      continue;
    }
    if (!pattern) continue;
    try {
      compiled.push(new RegExp(pattern, 'i'));
    } catch (err) {
      invalid.push([pattern, String(err)]);
      if (!reported.has(pattern)) {
        reported.add(pattern);
        console.warn(
          `[apcore] obs.redaction.regex_patterns entry ${JSON.stringify(pattern)} does not ` +
            `compile and will redact nothing: ${String(err)}. Patterns should stay inside the ` +
            `portable subset (no lookaround, no backreferences, no inline (?i) flags) — see ` +
            `PROTOCOL_SPEC 9.2.3 requirement 6.`,
        );
      }
    }
  }
  return { compiled, invalid };
}

/**
 * Runtime-configurable redaction rules for ObsLoggingMiddleware.
 * Applied in addition to schema-level x-sensitive annotations.
 */
export class RedactionConfig {
  readonly fieldPatterns: readonly string[];

  /**
   * `regex_patterns`, compiled ONCE at construction (§10.6.1 requirement 5).
   *
   * String entries passed to the constructor are compiled here, so this is
   * always `RegExp[]` however the patterns arrived. Compiling per record was
   * the previous behaviour on the programmatic path, and it is what made the
   * requirement-4 diagnostic need a process-global set to stay quiet.
   */
  readonly valuePatterns: readonly RegExp[];

  /** `[pattern, engine message]` for every entry that did not compile. */
  readonly invalidValuePatterns: readonly (readonly [string, string])[];

  readonly replacement: string;

  constructor(
    options: {
      fieldPatterns?: string[];
      valuePatterns?: (RegExp | string)[];
      replacement?: string;
    } = {},
  ) {
    this.fieldPatterns = options.fieldPatterns ?? [];
    const { compiled, invalid } = compileValuePatterns(options.valuePatterns ?? []);
    this.valuePatterns = compiled;
    this.invalidValuePatterns = invalid;
    this.replacement = options.replacement ?? '***REDACTED***';
  }

  /**
   * Build a RedactionConfig from an apcore {@link Config}.
   *
   * Reads canonical keys first (sync finding CRITICAL #4 — aligns with
   * apcore-python / apcore-rust):
   *   - `obs.redaction.sensitive_keys` (string[])
   *   - `obs.redaction.regex_patterns` (string[]; compiled case-insensitively)
   *   - `obs.redaction.replacement`    (string)
   *
   * For backwards compatibility, falls back to legacy keys (Issue #43 §5)
   * when the canonical key is unset, and emits a one-shot deprecation
   * warning the first time a legacy key is read:
   *   - `observability.redaction.field_patterns`   (legacy of sensitive_keys)
   *   - `observability.redaction.value_patterns`   (legacy of regex_patterns)
   *   - `observability.redaction.replacement`      (legacy of replacement)
   *
   * Falls back to {@link DEFAULT_REDACTION_FIELD_PATTERNS} when no sensitive
   * keys are configured so `_secret_*` and standard sensitive keys (apiKey,
   * api_key, token, authorization, password) remain redacted out of the box.
   *
   * "Not configured" means the key is absent or null. An operator who
   * explicitly writes `sensitive_keys: []` has disabled key-based redaction
   * and gets exactly that — the override REPLACES the default list rather
   * than merging with it, and an empty override is not re-interpreted as
   * "unset" (docs/features/observability.md, D-54; matches apcore-python).
   */
  static fromConfig(config: Config): RedactionConfig {
    const legacyKeysUsed: string[] = [];

    // --- sensitive_keys (formerly field_patterns) ---
    let rawFields = config.get('obs.redaction.sensitive_keys');
    if (rawFields === undefined || rawFields === null) {
      const legacyFields = config.get('observability.redaction.field_patterns');
      if (legacyFields !== undefined && legacyFields !== null) {
        rawFields = legacyFields;
        legacyKeysUsed.push('observability.redaction.field_patterns');
      }
    }

    // --- regex_patterns (formerly value_patterns) ---
    let rawValues = config.get('obs.redaction.regex_patterns');
    if (rawValues === undefined || rawValues === null) {
      const legacyValues = config.get('observability.redaction.value_patterns');
      if (legacyValues !== undefined && legacyValues !== null) {
        rawValues = legacyValues;
        legacyKeysUsed.push('observability.redaction.value_patterns');
      }
    }

    // --- replacement ---
    let replacement = config.get('obs.redaction.replacement');
    if (replacement === undefined || replacement === null) {
      const legacyReplacement = config.get('observability.redaction.replacement');
      if (legacyReplacement !== undefined && legacyReplacement !== null) {
        replacement = legacyReplacement;
        legacyKeysUsed.push('observability.redaction.replacement');
      }
    }

    if (legacyKeysUsed.length > 0) {
      _emitRedactionLegacyDeprecation(legacyKeysUsed);
    }

    // An operator-supplied list REPLACES the default; it does not merge
    // (docs/features/observability.md "Canonical default sensitive_keys").
    // An explicitly-configured EMPTY list therefore means "no key-based
    // redaction" and MUST be honoured — only a missing/null value falls back
    // to the shipped defaults. Matches apcore-python's
    // `RedactionConfig.from_config` ("Empty lists are permitted; callers that
    // want NO redaction must explicitly set `sensitive_keys: []`").
    const fieldPatterns = Array.isArray(rawFields)
      ? (rawFields as unknown[]).filter((p): p is string => typeof p === 'string')
      : [...DEFAULT_REDACTION_FIELD_PATTERNS];

    const valueStrings = Array.isArray(rawValues)
      ? (rawValues as unknown[]).filter((p): p is string => typeof p === 'string')
      : [];
    return new RedactionConfig({
      fieldPatterns,
      // Compiled by the constructor — one compile site, one place the
      // requirement-4 diagnostic can fire.
      valuePatterns: valueStrings,
      replacement: typeof replacement === 'string' ? replacement : undefined,
    });
  }

  /**
   * Default redaction config so callers that don't supply one still get the
   * standard sensitive-key redaction (`_secret_*`, password, token, …).
   * Mirrors apcore-python's `RedactionConfig.default()`.
   */
  static default(): RedactionConfig {
    return new RedactionConfig({ fieldPatterns: [...DEFAULT_REDACTION_FIELD_PATTERNS] });
  }

  /** Apply redaction rules to a flat object of field name → value. */
  apply(obj: Record<string, unknown>): Record<string, unknown> {
    const result: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(obj)) {
      if (this._shouldRedact(k, v)) {
        result[k] = this.replacement;
      } else {
        result[k] = v;
      }
    }
    return result;
  }

  /**
   * Recursively redact a value: any key matching a sensitive-key pattern is
   * replaced; nested objects and arrays are descended (depth-bounded). Mirrors
   * apcore-python's `_redact_secrets_recursive`.
   */
  redact(value: unknown, depth: number = 0): unknown {
    if (depth > MAX_REDACTION_DEPTH) return value;
    if (Array.isArray(value)) {
      return value.map((item) =>
        // A string ELEMENT is matched against `regex_patterns` at its own
        // position. It used to be handed straight back to `redact`, which only
        // ever consults the value rule from inside `_shouldRedact` — and that
        // needs a field name an array element does not have. So a secret in a
        // list was returned in plaintext here while apcore-python
        // (`_redact_in_list`) and apcore-rust (`redact_inner(item, None)`)
        // both replaced it. Found by the fixture case added for §10.6.1
        // requirement 2, which is the requirement's other half: containers are
        // descended into, so declining to STRINGIFY a container must not turn
        // into skipping what is inside it.
        typeof item === 'string' && this._valueMatches(item)
          ? this.replacement
          : this.redact(item, depth + 1),
      );
    }
    if (value !== null && typeof value === 'object') {
      const result: Record<string, unknown> = {};
      for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
        result[k] = this._shouldRedact(k, v) ? this.replacement : this.redact(v, depth + 1);
      }
      return result;
    }
    return value;
  }

  private _shouldRedact(fieldName: string, value: unknown): boolean {
    if (PROTECTED_LOG_FIELDS.has(fieldName)) return false;

    const lowerKey = fieldName.toLowerCase();
    const normKey = _normalizeKeyForMatch(fieldName);
    const compactKey = _compactKeyForMatch(fieldName);
    for (const pattern of this.fieldPatterns) {
      if (!pattern) continue;
      const lowerPat = pattern.toLowerCase();
      // PROTOCOL_SPEC 10.6.1: an entry containing `*` or `?` is a glob-dialect
      // pattern (A25, anchored to the whole name); anything else is a
      // substring. `[` is NOT a trigger — brackets are literals under A25
      // (9.2.3 requirement 4), and reading them as a character class is what
      // inverted `[!p]assword` here (#117). The case fold is applied to BOTH
      // sides, which is the half apcore-rust was missing.
      const isGlob = lowerPat.includes('*') || lowerPat.includes('?');
      if (isGlob) {
        if (matchGlob(lowerPat, lowerKey)) return true;
      } else {
        // Plain case-insensitive substring match with hyphen/space ↔ underscore
        // equivalence (apcore-python behavioral parity).
        const normPat = _normalizeKeyForMatch(pattern);
        if (normKey.includes(normPat)) return true;
        // Also try the compact (separator-stripped) form so that camelCase
        // keys like "AccessKey" match the "access_key" canonical pattern.
        const compactPat = _compactKeyForMatch(pattern);
        if (compactKey.includes(compactPat)) return true;
      }
    }

    return typeof value === 'string' && this._valueMatches(value);
  }

  /**
   * The `regex_patterns` rule, for a value already known to be a string.
   *
   * PROTOCOL_SPEC §10.6.1 requirement 2: the rule applies to STRING values
   * only, and a non-string value **MUST NOT** be converted to a string in
   * order to test it — the three host languages render one non-string value
   * three different ways, so a rule defined over the rendering is three rules.
   * apcore-python's log-emission path did convert, and answered
   * `***REDACTED***` for `amount: 42` where its own capture path answered `42`.
   *
   * Separate from `_shouldRedact` because it is reached from two places: a
   * named field, and an array ELEMENT, which has no name to check against
   * `sensitive_keys` but is still a value.
   *
   * §9.2.3 requirement 6a: an UNANCHORED, case-insensitive search. The
   * patterns are compiled at construction (§10.6.1 requirement 5), so this
   * costs no compile per record.
   */
  private _valueMatches(value: string): boolean {
    for (const re of this.valuePatterns) {
      if (re.test(value)) return true;
    }
    return false;
  }
}

// ---------------------------------------------------------------------------
// ContextLogger
// ---------------------------------------------------------------------------

const LEVELS: Record<string, number> = {
  trace: 0,
  debug: 10,
  info: 20,
  warn: 30,
  error: 40,
  fatal: 50,
};


interface WritableOutput {
  write(s: string): void;
}

export class ContextLogger {
  private _name: string;
  private _format: string;
  private _level: string;
  private _levelValue: number;
  private _redactSensitive: boolean;
  private _redaction: RedactionConfig;
  private _output: WritableOutput;
  private _traceId: string | null = null;
  private _moduleId: string | null = null;
  private _callerId: string | null = null;

  constructor(options?: {
    name?: string;
    format?: string;
    level?: string;
    redactSensitive?: boolean;
    redaction?: RedactionConfig;
    output?: WritableOutput;
  }) {
    this._name = options?.name ?? 'apcore';
    this._format = options?.format ?? 'json';
    this._level = options?.level ?? 'info';
    this._levelValue = LEVELS[this._level] ?? 20;
    this._redactSensitive = options?.redactSensitive ?? true;
    // Config-driven redaction. When no explicit config is supplied, fall back
    // to the default sensitive-key set so legacy callers still get `_secret_*`
    // (and the standard keys: password, token, …) redacted out of the box.
    this._redaction = options?.redaction ?? RedactionConfig.default();
    // Default output uses console.error for universal compatibility (Node.js + browser)
    this._output = options?.output ?? { write: (s: string) => console.error(s) };
  }

  /**
   * Replace the rules this logger's own recursive pass applies.
   *
   * `_emit` redacts the WHOLE `extra` object recursively with `this._redaction`,
   * which defaults to {@link RedactionConfig.default}. That default is the
   * shipped `sensitive_keys` list and carries no `regex_patterns`, so a logger
   * left at the default and a caller-supplied config are two different rule
   * sets applied to two different parts of one log record.
   *
   * {@link ObsLoggingMiddleware} calls this so both passes honour the same
   * rules, mirroring apcore-python's `ObsLoggingMiddleware.__init__`
   * (`self._logger._redaction_config = redaction_config`). Without it the
   * middleware's flat `apply()` redacts a top-level string and the logger's
   * recursive pass then walks the SAME record under the default rules, so a
   * secret nested in an object or an array is written out in plaintext.
   */
  setRedactionConfig(config: RedactionConfig): void {
    this._redaction = config;
  }

  static fromContext(
    context: Context<unknown>,
    name: string,
    options?: {
      format?: string;
      level?: string;
      redactSensitive?: boolean;
      redaction?: RedactionConfig;
      output?: WritableOutput;
    },
  ): ContextLogger {
    const logger = new ContextLogger({ name, ...options });
    logger._traceId = context.traceId;
    logger._moduleId =
      context.callChain.length > 0 ? context.callChain[context.callChain.length - 1] : null;
    logger._callerId = context.callerId;
    return logger;
  }

  private _emit(levelName: string, message: string, extra?: Record<string, unknown> | null): void {
    const levelValue = LEVELS[levelName] ?? 20;
    if (levelValue < this._levelValue) return;

    let redactedExtra = extra ?? null;
    if (extra != null && this._redactSensitive) {
      redactedExtra = this._redaction.redact(extra) as Record<string, unknown>;
    }

    const now = new Date();
    const entry: Record<string, unknown> = {
      timestamp: now.toISOString(),
      level: levelName,
      message,
      trace_id: this._traceId,
      module_id: this._moduleId,
      caller_id: this._callerId,
      logger: this._name,
      extra: redactedExtra,
    };

    if (this._format === 'json') {
      this._output.write(JSON.stringify(entry) + '\n');
    } else {
      const ts = now
        .toISOString()
        .replace('T', ' ')
        .replace(/\.\d+Z$/, '');
      const lvl = levelName.toUpperCase();
      const trace = this._traceId ?? 'none';
      const mod = this._moduleId ?? 'none';
      let extrasStr = '';
      if (redactedExtra) {
        extrasStr =
          ' ' +
          Object.entries(redactedExtra)
            .map(([k, v]) => `${k}=${v}`)
            .join(' ');
      }
      this._output.write(
        `${ts} [${lvl}] [trace=${trace}] [module=${mod}] ${message}${extrasStr}\n`,
      );
    }
  }

  trace(message: string, extra?: Record<string, unknown>): void {
    this._emit('trace', message, extra);
  }

  debug(message: string, extra?: Record<string, unknown>): void {
    this._emit('debug', message, extra);
  }

  info(message: string, extra?: Record<string, unknown>): void {
    this._emit('info', message, extra);
  }

  warn(message: string, extra?: Record<string, unknown>): void {
    this._emit('warn', message, extra);
  }

  error(message: string, extra?: Record<string, unknown>): void {
    this._emit('error', message, extra);
  }

  fatal(message: string, extra?: Record<string, unknown>): void {
    this._emit('fatal', message, extra);
  }
}

export class ObsLoggingMiddleware extends Middleware {
  private _logger: ContextLogger;
  private _logInputs: boolean;
  private _logOutputs: boolean;
  private _redactionConfig: RedactionConfig | null;

  constructor(options?: {
    logger?: ContextLogger;
    logInputs?: boolean;
    logOutputs?: boolean;
    redactionConfig?: RedactionConfig | null;
  }) {
    super();
    this._logger = options?.logger ?? new ContextLogger({ name: 'apcore.obs_logging' });
    this._logInputs = options?.logInputs ?? true;
    this._logOutputs = options?.logOutputs ?? true;
    this._redactionConfig = options?.redactionConfig ?? null;
    // An explicit config governs the WHOLE record, not just the `inputs` /
    // `output` sub-object this middleware redacts itself. `ContextLogger._emit`
    // runs a second, RECURSIVE pass over the entire `extra`, and until this
    // line it ran under `RedactionConfig.default()` — a rule set with no
    // `regex_patterns` at all. Measured with the wiring observability.md
    // documents (`new ObsLoggingMiddleware({ redactionConfig })`):
    //
    //   top:    "***REDACTED***"          <- the flat pass, correct rules
    //   items:  ["sk-abcdef123456"]       <- LEAKED
    //   nested: { key: "sk-abcdef123456" }<- LEAKED
    //
    // apcore-python has assigned the logger's config here since Issue #43 §5
    // and answers `***REDACTED***` for all three; apcore-rust redacts
    // recursively with its own config and has no second pass. This SDK was the
    // only one that leaked. Aligning also stops the reverse error: a caller who
    // NARROWS the rules (`sensitive_keys: []`) was still having the default
    // list applied by the logger underneath.
    if (this._redactionConfig !== null) {
      this._logger.setRedactionConfig(this._redactionConfig);
    }
  }

  override before(moduleId: string, _inputs: Record<string, unknown>, context: Context): null {
    const starts = (context.data['_apcore.mw.logging.obs_starts'] as number[]) ?? [];
    starts.push(performance.now());
    context.data['_apcore.mw.logging.obs_starts'] = starts;

    const extra: Record<string, unknown> = {
      module_id: moduleId,
      caller_id: context.callerId,
    };
    // PROTOCOL_SPEC 10.6.1 requirement 5 (D-131): log the CAPTURED inputs,
    // never the raw argument. The capture point applied `x-sensitive`, which
    // no key-based pass here can; when nothing was captured (the middleware
    // was invoked outside the pipeline) the inputs are omitted, not logged raw.
    if (this._logInputs && context.redactedInputs != null) {
      let loggableInputs = context.redactedInputs;
      if (this._redactionConfig !== null) {
        loggableInputs = this._redactionConfig.apply(loggableInputs);
      }
      extra['inputs'] = loggableInputs;
    }
    this._logger.info('Module call started', extra);
    return null;
  }

  override after(
    moduleId: string,
    _inputs: Record<string, unknown>,
    _output: Record<string, unknown>,
    context: Context,
  ): null {
    const starts = context.data['_apcore.mw.logging.obs_starts'] as number[] | undefined;
    if (!starts || starts.length === 0) return null;
    const startTime = starts.pop()!;
    const durationMs = performance.now() - startTime;

    const extra: Record<string, unknown> = {
      module_id: moduleId,
      duration_ms: durationMs,
    };
    // D-131: the captured output only — see before(). The raw `output`
    // argument is never logged, so it is not read here at all.
    if (this._logOutputs && context.redactedOutput != null) {
      let loggableOutput = context.redactedOutput;
      if (this._redactionConfig !== null) {
        loggableOutput = this._redactionConfig.apply(loggableOutput);
      }
      extra['output'] = loggableOutput;
    }
    this._logger.info('Module call completed', extra);
    return null;
  }

  override onError(
    moduleId: string,
    _inputs: Record<string, unknown>,
    error: Error,
    context: Context,
  ): null {
    const starts = context.data['_apcore.mw.logging.obs_starts'] as number[] | undefined;
    if (!starts || starts.length === 0) return null;
    const startTime = starts.pop()!;
    const durationMs = performance.now() - startTime;

    this._logger.error('Module call failed', {
      module_id: moduleId,
      duration_ms: durationMs,
      error_type: error.constructor.name,
      error_message: String(error),
    });
    return null;
  }
}
