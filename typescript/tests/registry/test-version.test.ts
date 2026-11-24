import { describe, it, expect } from 'vitest';
import {
  parseSemver,
  compareSemver,
  matchesVersionHint,
  selectBestVersion,
  VersionedStore,
} from '../../src/registry/version.js';
import { VersionConstraintError } from '../../src/errors.js';

describe('parseSemver', () => {
  it('returns null for non-semver strings', () => {
    expect(parseSemver('not-a-version')).toBeNull();
    expect(parseSemver('abc')).toBeNull();
  });

  it('parses major-only version', () => {
    expect(parseSemver('1')).toEqual([1, 0, 0]);
  });

  it('parses major.minor version', () => {
    expect(parseSemver('1.2')).toEqual([1, 2, 0]);
  });

  it('parses full major.minor.patch version', () => {
    expect(parseSemver('1.2.3')).toEqual([1, 2, 3]);
  });

  it('trims whitespace', () => {
    expect(parseSemver('  2.3.4  ')).toEqual([2, 3, 4]);
  });
});

describe('compareSemver', () => {
  it('returns 0 for equal versions', () => {
    expect(compareSemver('1.2.3', '1.2.3')).toBe(0);
  });

  it('returns negative when a < b', () => {
    expect(compareSemver('1.0.0', '2.0.0')).toBeLessThan(0);
    expect(compareSemver('1.2.3', '1.2.4')).toBeLessThan(0);
  });

  it('returns positive when a > b', () => {
    expect(compareSemver('2.0.0', '1.0.0')).toBeGreaterThan(0);
  });

  it('treats invalid versions as 0.0.0', () => {
    expect(compareSemver('invalid', '0.0.0')).toBe(0);
    expect(compareSemver('invalid', '1.0.0')).toBeLessThan(0);
  });
});

describe('matchesVersionHint', () => {
  describe('caret operator (^)', () => {
    it('matches within same major (major > 0)', () => {
      expect(matchesVersionHint('1.2.3', '^1.0.0')).toBe(true);
      expect(matchesVersionHint('1.9.9', '^1.0.0')).toBe(true);
      expect(matchesVersionHint('2.0.0', '^1.0.0')).toBe(false);
    });

    it('matches within same minor (major == 0, minor > 0)', () => {
      expect(matchesVersionHint('0.2.5', '^0.2.3')).toBe(true);
      expect(matchesVersionHint('0.3.0', '^0.2.3')).toBe(false);
    });

    it('matches only patch increment (major == 0, minor == 0)', () => {
      expect(matchesVersionHint('0.0.3', '^0.0.3')).toBe(true);
      expect(matchesVersionHint('0.0.4', '^0.0.3')).toBe(false);
    });
  });

  describe('tilde operator (~)', () => {
    it('matches patch range for 3-part constraint', () => {
      expect(matchesVersionHint('1.2.5', '~1.2.3')).toBe(true);
      expect(matchesVersionHint('1.3.0', '~1.2.3')).toBe(false);
    });

    it('matches patch range for 2-part constraint', () => {
      expect(matchesVersionHint('1.2.9', '~1.2')).toBe(true);
      expect(matchesVersionHint('1.3.0', '~1.2')).toBe(false);
    });

    it('matches minor+patch range for 1-part constraint', () => {
      expect(matchesVersionHint('1.9.9', '~1')).toBe(true);
      expect(matchesVersionHint('2.0.0', '~1')).toBe(false);
    });
  });

  describe('equality operator (=)', () => {
    it('matches any patch when 1-part constraint', () => {
      expect(matchesVersionHint('1.5.3', '1')).toBe(true);
      expect(matchesVersionHint('2.0.0', '1')).toBe(false);
    });

    it('matches any patch when 2-part constraint', () => {
      expect(matchesVersionHint('1.2.9', '1.2')).toBe(true);
      expect(matchesVersionHint('1.3.0', '1.2')).toBe(false);
    });

    it('matches exact version for 3-part constraint', () => {
      expect(matchesVersionHint('1.2.3', '=1.2.3')).toBe(true);
      expect(matchesVersionHint('1.2.4', '=1.2.3')).toBe(false);
    });

    it('matches exact version without operator', () => {
      expect(matchesVersionHint('1.2.3', '1.2.3')).toBe(true);
      expect(matchesVersionHint('1.2.4', '1.2.3')).toBe(false);
    });
  });

  describe('comparison operators', () => {
    it('>= matches equal and greater', () => {
      expect(matchesVersionHint('1.2.3', '>=1.2.3')).toBe(true);
      expect(matchesVersionHint('1.2.4', '>=1.2.3')).toBe(true);
      expect(matchesVersionHint('1.2.2', '>=1.2.3')).toBe(false);
    });

    it('> matches strictly greater', () => {
      expect(matchesVersionHint('1.2.4', '>1.2.3')).toBe(true);
      expect(matchesVersionHint('1.2.3', '>1.2.3')).toBe(false);
    });

    it('<= matches equal and less', () => {
      expect(matchesVersionHint('1.2.3', '<=1.2.3')).toBe(true);
      expect(matchesVersionHint('1.2.2', '<=1.2.3')).toBe(true);
      expect(matchesVersionHint('1.2.4', '<=1.2.3')).toBe(false);
    });

    it('< matches strictly less', () => {
      expect(matchesVersionHint('1.2.2', '<1.2.3')).toBe(true);
      expect(matchesVersionHint('1.2.3', '<1.2.3')).toBe(false);
    });
  });

  it('supports multiple comma-separated constraints', () => {
    expect(matchesVersionHint('1.5.0', '>=1.0.0,<2.0.0')).toBe(true);
    expect(matchesVersionHint('2.0.0', '>=1.0.0,<2.0.0')).toBe(false);
  });

  describe('malformed constraints (A-D-19)', () => {
    it('throws VersionConstraintError on empty constraint', () => {
      expect(() => matchesVersionHint('1.0.0', '')).toThrow(VersionConstraintError);
    });

    it('throws VersionConstraintError on non-digit-leading constraint "latest"', () => {
      expect(() => matchesVersionHint('1.0.0', 'latest')).toThrow(VersionConstraintError);
    });

    it('throws VersionConstraintError on non-digit-leading constraint "v1.0"', () => {
      expect(() => matchesVersionHint('1.0.0', 'v1.0')).toThrow(VersionConstraintError);
    });

    it('still accepts a valid digit-leading constraint', () => {
      expect(matchesVersionHint('1.0.0', '>=1.0.0')).toBe(true);
      expect(matchesVersionHint('1.2.3', '1.2.3')).toBe(true);
    });
  });
});

describe('selectBestVersion', () => {
  it('returns null for empty array', () => {
    expect(selectBestVersion([])).toBeNull();
  });

  it('returns latest when no hint', () => {
    expect(selectBestVersion(['1.0.0', '2.0.0', '1.5.0'])).toBe('2.0.0');
  });

  it('returns null hint treated as latest', () => {
    expect(selectBestVersion(['1.0.0', '2.0.0'], null)).toBe('2.0.0');
  });

  it('returns best matching version for hint', () => {
    expect(selectBestVersion(['1.0.0', '1.5.0', '2.0.0'], '^1.0.0')).toBe('1.5.0');
  });

  it('returns null when no version matches hint', () => {
    expect(selectBestVersion(['1.0.0', '1.5.0'], '^2.0.0')).toBeNull();
  });
});

describe('VersionedStore', () => {
  it('add and get a specific version', () => {
    const store = new VersionedStore<string>();
    store.add('mod-a', '1.0.0', 'value-1');
    expect(store.get('mod-a', '1.0.0')).toBe('value-1');
  });

  it('add to existing moduleId map', () => {
    const store = new VersionedStore<string>();
    store.add('mod-a', '1.0.0', 'v1');
    store.add('mod-a', '2.0.0', 'v2');
    expect(store.get('mod-a', '2.0.0')).toBe('v2');
  });

  it('get returns null for unknown moduleId', () => {
    const store = new VersionedStore<string>();
    expect(store.get('unknown', '1.0.0')).toBeNull();
  });

  it('get returns null for unknown version', () => {
    const store = new VersionedStore<string>();
    store.add('mod-a', '1.0.0', 'v1');
    expect(store.get('mod-a', '9.9.9')).toBeNull();
  });

  it('getLatest returns null for unknown moduleId', () => {
    const store = new VersionedStore<string>();
    expect(store.getLatest('unknown')).toBeNull();
  });

  it('getLatest returns highest semver', () => {
    const store = new VersionedStore<string>();
    store.add('mod-a', '1.0.0', 'v1');
    store.add('mod-a', '2.0.0', 'v2');
    store.add('mod-a', '1.5.0', 'v1.5');
    expect(store.getLatest('mod-a')).toBe('v2');
  });

  it('resolve without hint returns latest', () => {
    const store = new VersionedStore<string>();
    store.add('mod-a', '1.0.0', 'v1');
    store.add('mod-a', '2.0.0', 'v2');
    expect(store.resolve('mod-a')).toBe('v2');
  });

  it('resolve with hint returns best match', () => {
    const store = new VersionedStore<string>();
    store.add('mod-a', '1.0.0', 'v1');
    store.add('mod-a', '2.0.0', 'v2');
    expect(store.resolve('mod-a', '^1.0.0')).toBe('v1');
  });

  it('resolve returns null for unknown moduleId', () => {
    const store = new VersionedStore<string>();
    expect(store.resolve('unknown')).toBeNull();
  });

  it('listVersions returns sorted versions', () => {
    const store = new VersionedStore<string>();
    store.add('mod-a', '2.0.0', 'v2');
    store.add('mod-a', '1.0.0', 'v1');
    expect(store.listVersions('mod-a')).toEqual(['1.0.0', '2.0.0']);
  });

  it('listVersions returns empty for unknown moduleId', () => {
    const store = new VersionedStore<string>();
    expect(store.listVersions('unknown')).toEqual([]);
  });

  it('listIds returns all module IDs', () => {
    const store = new VersionedStore<string>();
    store.add('mod-a', '1.0.0', 'v1');
    store.add('mod-b', '1.0.0', 'v1');
    expect(store.listIds()).toContain('mod-a');
    expect(store.listIds()).toContain('mod-b');
  });

  it('remove returns false for unknown moduleId', () => {
    const store = new VersionedStore<string>();
    expect(store.remove('unknown', '1.0.0')).toBe(false);
  });

  it('remove returns false for unknown version', () => {
    const store = new VersionedStore<string>();
    store.add('mod-a', '1.0.0', 'v1');
    expect(store.remove('mod-a', '9.9.9')).toBe(false);
  });

  it('remove deletes version and returns true', () => {
    const store = new VersionedStore<string>();
    store.add('mod-a', '1.0.0', 'v1');
    store.add('mod-a', '2.0.0', 'v2');
    expect(store.remove('mod-a', '1.0.0')).toBe(true);
    expect(store.get('mod-a', '1.0.0')).toBeNull();
    expect(store.get('mod-a', '2.0.0')).toBe('v2');
  });

  it('remove cleans up moduleId when last version is removed', () => {
    const store = new VersionedStore<string>();
    store.add('mod-a', '1.0.0', 'v1');
    expect(store.remove('mod-a', '1.0.0')).toBe(true);
    expect(store.has('mod-a')).toBe(false);
    expect(store.listIds()).not.toContain('mod-a');
  });

  it('removeAll returns false for unknown moduleId', () => {
    const store = new VersionedStore<string>();
    expect(store.removeAll('unknown')).toBe(false);
  });

  it('removeAll removes all versions and returns true', () => {
    const store = new VersionedStore<string>();
    store.add('mod-a', '1.0.0', 'v1');
    store.add('mod-a', '2.0.0', 'v2');
    expect(store.removeAll('mod-a')).toBe(true);
    expect(store.has('mod-a')).toBe(false);
  });

  it('has returns true when versions exist', () => {
    const store = new VersionedStore<string>();
    store.add('mod-a', '1.0.0', 'v1');
    expect(store.has('mod-a')).toBe(true);
  });

  it('has returns false for unknown moduleId', () => {
    const store = new VersionedStore<string>();
    expect(store.has('unknown')).toBe(false);
  });

  it('hasVersion returns true for existing version', () => {
    const store = new VersionedStore<string>();
    store.add('mod-a', '1.0.0', 'v1');
    expect(store.hasVersion('mod-a', '1.0.0')).toBe(true);
  });

  it('hasVersion returns false for unknown moduleId', () => {
    const store = new VersionedStore<string>();
    expect(store.hasVersion('unknown', '1.0.0')).toBe(false);
  });

  it('hasVersion returns false for unknown version', () => {
    const store = new VersionedStore<string>();
    store.add('mod-a', '1.0.0', 'v1');
    expect(store.hasVersion('mod-a', '9.9.9')).toBe(false);
  });
});
