# Shared Contract and Conformance

After maintainer approval, update the protocol draft revision, history, changelog, applicable wire schemas, and decision record. Preserve existing anchors and old optional-field wire shape. Pin these behaviors in shared fixtures before changing SDK implementation.

- A direct identity has depth zero. A chain of eight edges succeeds. A ninth edge, an object cycle, an upward scope transition and a missing delegated scope fail. Repeated principals cannot recover authority.
- Absent and empty scope collections round-trip distinctly. Legacy direct identity input remains valid. A malformed present delegation fails restoration without becoming an ordinary identity.
- Decision tests cover actor roles, root-subject roles/types, depth, partial matching of required scopes and a legacy allow without a scope predicate. Discovery and invocation apply the same trusted scope.
- Test approval and cursor isolation when the actor stays constant and the subject or effective scope changes. Verify that an unauthorized resume leaves the stored approval available to its original authority.
- Exercise signed JWT claims with valid and invalid issuer and audience, expired tokens, malformed opt-in claims and conflicting scope sources. Nested actors never contribute authorization. Verify redaction at every identity depth.
