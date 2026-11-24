/** Public bare-name contract: protocol-spec §2.2.1; A02 remains strict. */
import { describe, expect, it } from 'vitest';
import * as node from '../../src/index.js';
import * as browser from '../../src/browser/index.js';

describe('canonicalizeName public bare-name normalization', () => {
  const successes: readonly [string, string][] = [
    ['ProcessPayment', 'process_payment'], ['getDBUrl', 'get_db_url'],
    ['HTTPServer-v2', 'http_server_v2'], ['  HTTP::Request!  ', 'http_request'],
    ['foo / - bar', 'foo_bar'], ['already__snake', 'already__snake'],
    ['foo__Bar', 'foo__bar'], ['system', 'system'], ['ephemeral', 'ephemeral'],
    ['\u0000Name\u007f', 'name'], ['a'.repeat(192), 'a'.repeat(192)],
    [`${'!'.repeat(200)}a`, 'a'], ['foo_--_bar', 'foo___bar'],
  ];
  for (const [name, canonicalName] of successes) {
    it(`canonicalizes ${JSON.stringify(name)} without registry decisions`, () => {
      expect(node.canonicalizeName(name)).toEqual({ originalName: name, canonicalName, error: null });
      expect(browser.canonicalizeName(name)).toEqual(node.canonicalizeName(name));
      expect(node.canonicalizeName(canonicalName).canonicalName).toBe(canonicalName);
    });
  }
  const failures: readonly [string, string][] = [
    ['', 'empty_name'], [' \t!@#\r\n', 'empty_name'],
    ['2fa', 'invalid_start'], ['7z', 'invalid_start'], ['_lead', 'invalid_start'],
    ['___', 'invalid_start'], ['`_lead`', 'invalid_start'],
    ['a'.repeat(193), 'name_too_long'], [`${'a'.repeat(191)}B`, 'name_too_long'],
    [`7${'a'.repeat(193)}`, 'invalid_start'], ['Éclair', 'non_ascii'],
    ['\u00a0Name', 'non_ascii'], ['Name\u2003', 'non_ascii'],
    ['\u01c5Name', 'non_ascii'], ['a\ud800b', 'non_ascii'], ['a\udc00b', 'non_ascii'],
    [`7${'a'.repeat(200)}é`, 'non_ascii'],
  ];
  for (const [name, error] of failures) {
    it(`returns ${error} for ${JSON.stringify(name)} without throwing or repair`, () => {
      expect(node.canonicalizeName(name)).toEqual({ originalName: name, canonicalName: null, error });
      expect(browser.canonicalizeName(name)).toEqual(node.canonicalizeName(name));
    });
  }
  it('keeps strict A02 behavior separate from repairing one segment', () => {
    expect(() => node.normalizeToCanonicalId(' HTTP::Request! ', 'rust')).toThrow();
    expect(node.normalizeToCanonicalId('api.HttpJsonParser', 'typescript')).toBe('api.http_json_parser');
    expect(node.canonicalizeName('api.HttpJsonParser')).toEqual({ originalName: 'api.HttpJsonParser', canonicalName: 'api_http_json_parser', error: null });
  });
});
