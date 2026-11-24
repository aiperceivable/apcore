/**
 * Version negotiation (Algorithm A14).
 */

import { ModuleError } from './errors.js';
import type { ErrorOptions } from './errors.js';

/** Regex for parsing semver strings (major.minor.patch with optional pre-release). */
const SEMVER_RE = /^(?<major>0|[1-9]\d*)\.(?<minor>0|[1-9]\d*)\.(?<patch>0|[1-9]\d*)(?:-(?<pre>[0-9A-Za-z\-]+(?:\.[0-9A-Za-z\-]+)*))?$/;

/** Deprecation warning threshold. */
const DEPRECATION_THRESHOLD = 2;

interface SemVer {
  readonly major: number;
  readonly minor: number;
  readonly patch: number;
  readonly pre: string | null;
}

function parseSemver(version: string): SemVer {
  const m = SEMVER_RE.exec(version.trim());
  if (!m || !m.groups) {
    throw new Error(`Invalid semantic version: '${version}'`);
  }
  return {
    major: parseInt(m.groups['major'], 10),
    minor: parseInt(m.groups['minor'], 10),
    patch: parseInt(m.groups['patch'], 10),
    pre: m.groups['pre'] ?? null,
  };
}

type SortKey = [number, number, number, number, Array<[number, number, string]>];

function sortKey(v: SemVer): SortKey {
  if (v.pre === null) {
    return [v.major, v.minor, v.patch, 1, []];
  }
  const parts: Array<[number, number, string]> = [];
  for (const ident of v.pre.split('.')) {
    if (/^\d+$/.test(ident)) {
      parts.push([0, parseInt(ident, 10), '']);
    } else {
      parts.push([1, 0, ident]);
    }
  }
  return [v.major, v.minor, v.patch, 0, parts];
}

function compareSortKeys(a: SortKey, b: SortKey): number {
  for (let i = 0; i < 4; i++) {
    if ((a[i] as number) !== (b[i] as number)) return (a[i] as number) - (b[i] as number);
  }
  const ap = a[4];
  const bp = b[4];
  const len = Math.min(ap.length, bp.length);
  for (let i = 0; i < len; i++) {
    if (ap[i][0] !== bp[i][0]) return ap[i][0] - bp[i][0];
    if (ap[i][1] !== bp[i][1]) return ap[i][1] - bp[i][1];
    if (ap[i][2] !== bp[i][2]) return ap[i][2] < bp[i][2] ? -1 : 1;
  }
  return ap.length - bp.length;
}

function semverToString(v: SemVer): string {
  const base = `${v.major}.${v.minor}.${v.patch}`;
  return v.pre ? `${base}-${v.pre}` : base;
}

export class VersionIncompatibleError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(declared: string, sdk: string, reason: string, options?: ErrorOptions) {
    super(
      'VERSION_INCOMPATIBLE',
      `Version incompatible: declared=${declared}, sdk=${sdk} — ${reason}`,
      { declaredVersion: declared, sdkVersion: sdk, reason },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'VersionIncompatibleError';
  }
}

/**
 * Negotiate the effective version between declared and SDK versions (Algorithm A14).
 *
 * Steps:
 *   1. Parse both versions as semver.
 *   2. Major mismatch -> error.
 *   3. Declared minor > SDK minor -> error (SDK too old).
 *   4. Declared minor < SDK minor by >2 -> deprecation warning.
 *   5. Same minor -> effective = max(declared, sdk).
 *
 * @param declaredVersion - Version declared in configuration or schema.
 * @param sdkVersion - Maximum version supported by the current SDK.
 * @returns The effective version string.
 * @throws {VersionIncompatibleError} When versions are incompatible.
 * @throws {Error} When a version string is not valid semver.
 */
export function negotiateVersion(declaredVersion: string, sdkVersion: string): string {
  const declared = parseSemver(declaredVersion);
  const sdk = parseSemver(sdkVersion);

  // Major version mismatch
  if (declared.major !== sdk.major) {
    throw new VersionIncompatibleError(declaredVersion, sdkVersion, 'Major version mismatch');
  }

  // Declared minor > SDK minor -> SDK too old
  if (declared.minor > sdk.minor) {
    throw new VersionIncompatibleError(declaredVersion, sdkVersion, 'SDK version too low, please upgrade');
  }

  // Declared minor < SDK minor
  if (declared.minor < sdk.minor) {
    const gap = sdk.minor - declared.minor;
    if (gap > DEPRECATION_THRESHOLD) {
      console.warn(
        `[apcore:version] Declared version ${declaredVersion} is ${gap} minor versions behind SDK ${sdkVersion} — consider upgrading your configuration`,
      );
    }
    return declaredVersion; // Backward compatibility mode
  }

  // Same minor -> effective = max(declared, sdk)
  const cmp = compareSortKeys(sortKey(declared), sortKey(sdk));
  return cmp >= 0 ? semverToString(declared) : semverToString(sdk);
}
