/**
 * Regression tests for D-93 (IDN-1 / SYS-1): the contextual-audit identity
 * redaction list.
 *
 * `extractAuditIdentity` had NO tests, which is how its substring list came to
 * diverge from apcore-python's `_IDENTITY_SENSITIVE_SUBSTRINGS` and
 * apcore-rust's `IDENTITY_SENSITIVE_SUBSTRINGS` (those two are byte-identical)
 * without anything going red.
 *
 * The list enumerated compounds -- `api_key`, `apikey`, `access_key`,
 * `private_key`, `authorization` -- where the peers carry the bare `key`,
 * `auth` and `session`. Enumerating compounds only catches the spellings
 * someone thought of, so `signing_key`, `auth_header` and `session_id` matched
 * nothing and were published verbatim on the event bus by TypeScript alone.
 *
 * `conformance/fixtures/contextual_audit.json` exercises only `bearer_token`,
 * which every list catches -- so CI could not see this. The cases below are
 * chosen specifically to DISCRIMINATE between the two lists.
 */

import { describe, it, expect } from 'vitest';
import { Context, Identity } from '../../src/context.js';
import { extractAuditIdentity } from '../../src/sys-modules/audit.js';

const REDACTED = '<redacted>';

function snapshotFor(attrs: Record<string, unknown>): Record<string, unknown> {
  const ctx = Context.create(new Identity('u1', 'user', [], attrs));
  const { identity } = extractAuditIdentity(ctx);
  expect(identity).not.toBeNull();
  return identity as Record<string, unknown>;
}

describe('extractAuditIdentity redaction', () => {
  it('sys_modules.audit.redaction.bare_key_substring_catches_signing_key', () => {
    // Matched none of api_key / apikey / access_key / private_key.
    expect(snapshotFor({ signing_key: 'MIIEvQIBADAN' })['signing_key']).toBe(REDACTED);
  });

  it('sys_modules.audit.redaction.bare_auth_substring_catches_auth_header', () => {
    // 'auth_header' does not contain 'authorization'.
    expect(snapshotFor({ auth_header: 'Basic dXNlcjpwdw==' })['auth_header']).toBe(REDACTED);
  });

  it('sys_modules.audit.redaction.bare_session_substring_catches_session_id', () => {
    expect(snapshotFor({ session_id: 's-abc123' })['session_id']).toBe(REDACTED);
  });

  it('sys_modules.audit.redaction.hyphenated_and_cased_spellings_are_caught', () => {
    const snap = snapshotFor({
      'API-Key': 'sk-live-1',
      'X-Auth-Token': 'tok',
      SessionKey: 'sk',
    });
    expect(snap['API-Key']).toBe(REDACTED);
    expect(snap['X-Auth-Token']).toBe(REDACTED);
    expect(snap['SessionKey']).toBe(REDACTED);
  });

  it('sys_modules.audit.redaction.canonical_ten_substrings_all_match', () => {
    // One probe per substring in the canonical list, so a future edit that
    // drops any single entry goes red here.
    const probes: Record<string, string> = {
      my_token: 'a',
      my_secret: 'b',
      my_password: 'c',
      my_passwd: 'd',
      my_key: 'e',
      my_auth: 'f',
      my_credential: 'g',
      my_cookie: 'h',
      my_session: 'i',
      my_bearer: 'j',
    };
    const snap = snapshotFor(probes);
    for (const name of Object.keys(probes)) {
      expect(snap[name], `'${name}' must be redacted`).toBe(REDACTED);
    }
  });

  it('sys_modules.audit.redaction.non_sensitive_attrs_pass_through', () => {
    // The guard must not over-redact: an audit record with everything hidden
    // is as useless as one with nothing hidden.
    const snap = snapshotFor({ tenant: 'acme', region: 'eu-west-1', seat_count: 12 });
    expect(snap['tenant']).toBe('acme');
    expect(snap['region']).toBe('eu-west-1');
    expect(snap['seat_count']).toBe(12);
  });

  it('sys_modules.audit.redaction.sensitive_key_is_replaced_not_dropped', () => {
    // Subscribers must still see that a credential was involved.
    const snap = snapshotFor({ api_key: 'sk-live-1' });
    expect(Object.hasOwn(snap, 'api_key')).toBe(true);
    expect(snap['api_key']).toBe(REDACTED);
  });

  it('sys_modules.audit.redaction.display_name_is_first_class_and_not_redacted', () => {
    const snap = snapshotFor({ display_name: 'Ada Lovelace', api_key: 'sk' });
    expect(snap['display_name']).toBe('Ada Lovelace');
    expect(snap['api_key']).toBe(REDACTED);
  });
});
