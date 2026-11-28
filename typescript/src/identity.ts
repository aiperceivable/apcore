/** Trusted caller identity and its active local delegation chain. */

import { compareCodePoints } from './utils/compare-code-points.js';

export const MAX_DELEGATION_DEPTH = 8;
const trustedIdentities = new WeakSet<Identity>();

export interface IdentityOptions {
  readonly scopes?: readonly string[];
  readonly onBehalfOf?: Identity;
  readonly issuer?: string;
}

export class Identity {
  readonly id: string;
  readonly type: string;
  readonly roles: readonly string[];
  readonly attrs: Readonly<Record<string, unknown>>;
  readonly scopes?: readonly string[];
  readonly onBehalfOf?: Identity;
  readonly issuer?: string;

  constructor(
    id: string, type: string = 'user', roles: readonly string[] = [],
    attrs: Record<string, unknown> = {}, options: IdentityOptions = {},
  ) {
    const { scopes, onBehalfOf, issuer } = options;
    if (scopes !== undefined && (!Array.isArray(scopes) || ![...scopes].every(isScopeLiteral))) {
      throw new TypeError('Identity scopes must be a list of nonempty strings without whitespace');
    }
    if (issuer !== undefined && !isNonemptyString(issuer)) {
      throw new TypeError('Identity issuer must be a nonempty string');
    }
    if ((scopes !== undefined || issuer !== undefined || onBehalfOf !== undefined) &&
        (!isNonemptyString(id) || !isNonemptyString(type) ||
          !Array.isArray(roles) || ![...roles].every(role => typeof role === 'string') || !isRecord(attrs))) {
      throw new TypeError('Invalid scoped identity');
    }
    if (onBehalfOf !== undefined) {
      if (!isTrustedIdentity(onBehalfOf) || delegationDepthOf(onBehalfOf) >= MAX_DELEGATION_DEPTH) {
        throw new TypeError('Invalid identity delegation chain');
      }
      const ceiling = onBehalfOf.scopes;
      if (scopes === undefined || ceiling === undefined || !scopes.every(scope => ceiling.includes(scope))) {
        throw new TypeError('Delegated scopes must be a subset of the parent scopes');
      }
    }
    this.id = id;
    this.type = type;
    this.roles = Object.freeze([...roles]);
    this.attrs = Object.freeze({ ...attrs });
    if (scopes !== undefined) this.scopes = Object.freeze(sortedSet(scopes));
    if (onBehalfOf !== undefined) this.onBehalfOf = onBehalfOf;
    if (issuer !== undefined) this.issuer = issuer;
    trustedIdentities.add(this);
    Object.freeze(this);
  }

  /** Read an attribute, preserving a stored null. */
  getAttr<T = unknown>(key: string, defaultValue?: T): T | undefined {
    return key in this.attrs ? (this.attrs[key] as T) : defaultValue;
  }

  get rootSubject(): Identity {
    return rootSubjectOf(this);
  }

  get delegationDepth(): number {
    return delegationDepthOf(this);
  }

  /** Issue explicitly attenuated authority to the supplied actor. */
  delegate(actor: Identity, scopes: readonly string[]): Identity {
    if (!isTrustedIdentity(actor) || actor.onBehalfOf !== undefined) {
      throw new TypeError('A trusted direct actor is required');
    }
    const ceiling = actor.scopes;
    if (ceiling !== undefined && (!Array.isArray(scopes) || !scopes.every(scope => ceiling.includes(scope)))) {
      throw new TypeError('Delegated scopes must be a subset of the actor scopes');
    }
    return new Identity(actor.id, actor.type, actor.roles, actor.attrs, { scopes, onBehalfOf: this, issuer: actor.issuer });
  }

  /** The current authorization scope, including trusted local lineage. */
  authorityKey(): string | null {
    return authorityKeyOf(this);
  }

  /** The transport owner remains stable when the same actor refreshes its scopes. */
  ownerKey(): string | null {
    return this.scopes === undefined ? null : JSON.stringify([principal(this), principal(rootSubjectOf(this))]);
  }
}

function isNonemptyString(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0;
}

/** @internal Check a case-sensitive scope literal at all authority entry points. */
export function isScopeLiteral(value: unknown): value is string {
  return isNonemptyString(value) && !/\p{White_Space}/u.test(value);
}

/** @internal Reject identities that were not constructed by the native SDK. */
export function isTrustedIdentity(identity: Identity | null | undefined): identity is Identity {
  return identity != null && trustedIdentities.has(identity);
}

/** @internal Follow the validated active links without getter overrides. */
export function rootSubjectOf(identity: Identity): Identity {
  let subject = identity;
  while (subject.onBehalfOf !== undefined) subject = subject.onBehalfOf;
  return subject;
}

/** @internal Count the validated active links without getter overrides. */
export function delegationDepthOf(identity: Identity): number {
  let depth = 0;
  for (let ancestor = identity.onBehalfOf; ancestor !== undefined; ancestor = ancestor.onBehalfOf) depth++;
  return depth;
}

function sortedSet(values: readonly string[]): string[] {
  return [...new Set(values)].sort(compareCodePoints);
}

function principal(identity: Identity): [string | null, string, string] {
  return [identity.issuer ?? null, identity.type, identity.id];
}

function authorityChain(identity: Identity): unknown[] {
  const chain: unknown[] = [];
  for (let actor: Identity | undefined = identity; actor !== undefined; actor = actor.onBehalfOf) {
    chain.push([principal(actor), sortedSet(actor.roles), actor.scopes ?? null]);
  }
  return chain;
}

/** @internal Resolve the native authority key without dispatching to a caller override. */
export function authorityKeyOf(identity: Identity | null | undefined): string | null {
  return identity?.scopes === undefined ? null : JSON.stringify(authorityChain(requireTrustedIdentity(identity)));
}

function requireTrustedIdentity(identity: Identity): Identity {
  if (!isTrustedIdentity(identity)) throw new TypeError('A trusted identity is required');
  return identity;
}

export function createIdentity(
  id: string, type: string = 'user', roles: readonly string[] = [],
  attrs: Record<string, unknown> = {}, options?: IdentityOptions,
): Identity {
  return new Identity(id, type, roles, attrs, options);
}

/** @internal Whether the caller opted into the scoped grant requirement. */
export function hasScopeCeiling(identity: Identity | null | undefined): boolean {
  return identity?.scopes !== undefined;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** @internal Encode only present optional identity fields. */
export function identityToWire(identity: Identity): Record<string, unknown> {
  return {
    id: identity.id, type: identity.type, roles: [...identity.roles], attrs: { ...identity.attrs },
    ...(identity.scopes === undefined ? {} : { scopes: [...identity.scopes] }),
    ...(identity.issuer === undefined ? {} : { issuer: identity.issuer }),
    ...(identity.onBehalfOf === undefined ? {} : { on_behalf_of: identityToWire(identity.onBehalfOf) }),
  };
}

/** @internal Restore a detached trusted identity after structurally validating its chain. */
export function identityFromWire(data: Record<string, unknown>, depth: number = 0): Identity {
  const { id, type, roles, attrs, scopes, issuer, on_behalf_of: parent } = data;
  const strict = depth > 0 || ['scopes', 'issuer', 'on_behalf_of'].some(key => Object.hasOwn(data, key));
  if (depth > MAX_DELEGATION_DEPTH ||
      (strict && (!isNonemptyString(id) || !isNonemptyString(type) ||
        (roles !== undefined && (!Array.isArray(roles) || ![...roles].every(role => typeof role === 'string'))) ||
        (attrs !== undefined && !isRecord(attrs)))) ||
      (Object.hasOwn(data, 'scopes') && !Array.isArray(scopes)) ||
      (Object.hasOwn(data, 'issuer') && !isNonemptyString(issuer)) ||
      (Object.hasOwn(data, 'on_behalf_of') && !isRecord(parent))) {
    throw new TypeError('Invalid wire identity');
  }
  return new Identity(
    typeof id === 'string' ? id : String(id ?? ''),
    typeof type === 'string' ? type : 'user',
    Array.isArray(roles) ? roles.filter((r): r is string => typeof r === 'string') : [],
    isRecord(attrs) ? attrs : {},
    {
      scopes: Array.isArray(scopes) ? scopes : undefined,
      issuer: typeof issuer === 'string' ? issuer : undefined,
      onBehalfOf: isRecord(parent) ? identityFromWire(parent, depth + 1) : undefined,
    },
  );
}

/** @internal Keep the legacy cursor shape and bind scoped authority without its attributes. */
export function cursorPrincipal(identity: Identity | null | undefined): unknown {
  if (identity == null) return null;
  const authorityKey = authorityKeyOf(identity);
  return authorityKey === null ? { ...identityToWire(identity), roles: sortedSet(identity.roles) } : { authorityKey };
}

/** @internal Detach a snapshot of an identity, including its trusted active lineage. */
export function detachIdentity(identity: Identity | null): Identity | null {
  return identity === null ? null : identityFromWire(structuredClone(identityToWire(identity)));
}
