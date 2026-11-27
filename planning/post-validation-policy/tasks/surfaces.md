# Surfaces Scoped Approval

## Goal

Carry final-scope approval through MCP without changing transport metadata or duplicating Core policy.

## Files Involved

- Existing Python and TypeScript MCP `adapters/approval` and `approval_store` or `approval-store` files in `apcore-surfaces`.
- Existing Rust MCP approval adapter and `approval_store.rs`.
- MCP adapter tests and the existing compatibility build workflows that consume Core candidate packages.

## Steps

1. Add red store and transport tests for final-scope records in all three languages. Cover legacy unversioned records, expiry, concurrent consumption, changed module version, and a real MCP resumption attempt.
2. Reuse the stores' atomic `consume_scoped` paths and enforce matching scope on both issue and resume. Existing MCP `_meta` credential fields and business-argument filtering remain compatible. Avoid returning digest, credential, or raw policy input in diagnostics.
3. Build against all three candidate SDKs. Run component and transport tests, and check A2A, CLI, or other consumers where handler construction or forwarded results are affected. Keep unaffected consumers on their established compatibility gates.

## Acceptance Criteria

- [ ] Installed MCP and Core packages demonstrate pending approval and exactly one resumed execution with matching scope.
- [ ] Older incompatible records are rejected in the new scope without a wildcard or consume-then-fallback path.
- [ ] Supported MCP transport fields and adapter compatibility checks pass in all affected profiles.

## Dependencies

Depends on Python, TypeScript, and Rust. Required by integration.

## Estimated Time

6 hours.

## Design Decisions Reference

See [plan.md](../plan.md#design-decisions) for persisted approval records and public compatibility.
