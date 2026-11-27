# Candidate Integration and Verification

## Goal

Verify the approved contract with cross-language fixtures and the actual Core and Surfaces package distributions.

## Files Involved

- Existing Core conformance fixtures and SDK distribution verification workflows.
- Existing Surfaces Core candidate compatibility matrix and MCP integration harnesses.
- Existing documentation and planning records referenced by the delivery.

## Steps

1. Run every affected SDK suite, the shared conformance inventory and drivers, and strict documentation checks. Re-run reviewed denial, credential and snapshot regressions independently in each SDK.
2. Build candidate wheels, npm archives and crates, then run compatibility CI against those artifacts. Record the exact Core and Surfaces SHAs and artifact identities.
3. Verify direct and MCP calls against the same policy and expected behavior. Exercise no-evaluator compatibility, evaluator-enabled denials, pending resumption, default and full profiles, and the supported Toolkit minimum where applicable.
4. Review terminal failure handling, profile dependencies, and no-evaluator performance against the alpha.2 baseline. Record unchecked or failed evidence and keep publication paused.

## Acceptance Criteria

- [ ] Three SDKs pass the same fixture expectations from their public entry points.
- [ ] Exact paired distributions pass the installed compatibility matrix and focused MCP resumption checks.
- [ ] Review findings, performance checks, and compatibility evidence are recorded before merge or release readiness is claimed.

## Dependencies

Depends on Surfaces. Required by the milestone completion review.

## Estimated Time

4 hours.

## Design Decisions Reference

See [plan.md](../plan.md#acceptance-criteria) for the acceptance matrix.
