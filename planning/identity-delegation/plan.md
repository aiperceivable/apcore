# Identity Delegation Implementation Plan

## Local authority

Keep the existing Identity constructors and the six-argument Context creation contract. Add immutable optional `scopes` and an optional `on_behalf_of` link. Preserve absent fields in the legacy wire shape. A missing scope collection means that scope enforcement is not enabled for that ordinary identity. An explicit empty collection means there is no scoped authority. Treat scope strings as case-sensitive literals.

A trusted host can issue a scoped root. A local delegation accepts a trusted actor and an explicit requested scope set. The requested set must be a subset of the parent ceiling at every hop. A parent with no explicit ceiling cannot seed delegated scope. Check the direct constructor, the delegation helper and trusted restoration against the same depth and subset invariants. Count edges from a direct identity at depth zero and allow up to eight. Reject object cycles and excessive nesting. Repeated actors within a bounded chain do not reset its authority. Deserialization validates structure and attenuation. It does not authenticate the supplied identity.

## Authorization, binding, and audit

For scope-enabled identities, require the first matching ACL grant to include a nonempty top-level `scopes` conjunct. The actor must possess every listed scope. A scope check that can be bypassed through a compound branch does not establish that grant. An allow without a scope grant, including a permissive default, denies scoped execution. A scoped request reaching a pipeline without a recognized enforcing gate fails closed. A scope-free legacy Identity continues to use the ordinary ACL contract. Keep actor types and roles on the actual actor. Add explicit root-subject type and role predicates and a delegation-depth predicate. The root subject of a direct call is its actor. Discovery evaluates these predicates using the same prospective caller and trusted scope as execution.

Use a shared detached authority projection per SDK for discovery cursors, policy requests and approval scope. Include the effective actor, root subject and scope ceiling where applicable. Include trusted local lineage where it contributes to authority. Keep prior actors from an RFC 8693 history out of authority decisions. An approval issued for one authority must never resume or be consumed under another. Existing direct approvals keep their direct-call scope. A legacy credential does not satisfy a delegated request.

Expose actor ID, root subject ID and active delegation depth in the applicable audit records. Reuse D-93 redaction for attributes at every depth. Audit snapshots must not expose raw signed tokens or unredacted attributes from an ancestor.

## JWT projection and transport ownership

Make the scope and delegation mapping opt-in in all six authenticators. Require a verified signature, expected issuer and audience, and applicable time claims before mapping a configured scope or delegation. A present malformed delegation or invalid configured scope fails authentication. Keep the current claim format when mapping is disabled. Parse `scope` and configured provider `scp` according to their declared formats. If both configured representations are present, require the normalized sets to agree.

Map the verified top-level subject and the outermost actor to one active Core delegation edge. The token scopes are the current ceiling. Derive actor roles only from explicitly mapped claims about the current actor. Validate nested actor claims against the same structural limit and discard the history after validation. The active authorization depth remains one regardless of that history. See [RFC 8693 §4.1](https://www.rfc-editor.org/rfc/rfc8693.html#section-4.1) for the actor distinction and [RFC 8693 §4.2](https://www.rfc-editor.org/rfc/rfc8693.html#section-4.2) for the scope representation.

Bind transport and stored task ownership to the issuer-qualified actor and effective subject where supported. Keep that ownership key separate from the authority projection. Apply current scope to each protected operation. Include current authority in permission caches and recheck governed result retrieval and resumption so narrower claims do not inherit stale permissions. Equivalent token refresh may retain the same owner.

Bind live MCP session reuse to equivalent authority so a narrowed credential cannot cancel an earlier invocation or answer its approval. An actor with changed scopes or roles opens a new session and can still access its stored tasks when the current Core decision permits it.

## Candidate boundary

Complete the Core contract first, then the six adapter mappings, then installed-artifact validation. The SDK wrappers reuse the current Context, ACL, policy, and approval entry points. This candidate covers resource-server consumption of credential claims and local trusted delegation. Opening a PR, merging a candidate, and releasing packages are separate steps.
