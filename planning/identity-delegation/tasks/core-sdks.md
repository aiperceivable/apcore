# Core SDKs

Python owns `python/`, TypeScript owns `typescript/`, and Rust owns `rust/`. Each SDK uses the existing identity and context APIs, ACL condition registry, discovery projection, policy snapshot and approval verification hooks.

1. Pin existing construction and wire shapes. Add the checked delegated data model and scope narrowing with explicit parent authority.
2. Align ACL predicates, discovery and actual execution. Reuse the existing matcher, audit producer and fail-closed execution path.
3. Carry a stable authority projection into cursor binding, request snapshots and permission to resume approvals. Preserve the direct path and fail safely on a malformed delegated representation.
4. Run the shared fixtures plus SDK-specific regressions through public APIs. Cover legacy compatibility, concurrent use of detached identities and audit redaction.
