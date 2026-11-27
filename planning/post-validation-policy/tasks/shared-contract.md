# Shared Policy Contract and Conformance

## Goal

Turn the approved policy placement and scope into a shared SDK contract and executable fixture expectations.

## Files Involved

- `docs/spec/protocol-spec.md`, `docs/index.md`, `CHANGELOG.md`, and the relevant decision register and feature references.
- `conformance/fixtures/post_validation_policy.json`, `conformance/README.md`, and `docs/spec/conformance.md`.
- Canonical schemas and error mappings where the approved contract introduces wire-visible fields or codes.

## Steps

1. Record maintainer approval of the opted-in behavior and protocol draft revision. Define the effective argument projection and digest from representative native and JSON values before committing the contract. Check the exact numeric and Unicode vectors across all three languages and preserve application-observable distinctions.
2. Specify the evaluator shape, policy verdict and error behavior, private credential lifecycle, sensitive-field projection, fail-closed ordering, module version origin, and strategy validity. Keep the existing raw ACL view distinct from validated policy inputs and preserve default assembly semantics.
3. Define public-API fixtures for phase B replay, unchanged scope, changed scope, middleware normalization, caller cancellation, evaluator error, denied ACL disclosure, and zero producer starts. Provide case IDs and expected fields for all three language drivers.
4. Align the approved draft header, history entry, and changelog. Run the repository's conformance inventory and strict documentation checks. Open a discussion issue before any specification PR.

## Acceptance Criteria

- [ ] The maintainer decision, default compatibility, final approval, and record migration boundaries are explicit.
- [ ] Shared fixture data and inventory agree and every expectation has an intended public driver.
- [ ] The chosen canonical domain is supported by the profiles and rejects observable ambiguity.

## Dependencies

Depends on maintainer approval. Required by Python, TypeScript, and Rust.

## Estimated Time

4 hours.

## Design Decisions Reference

See [plan.md](../plan.md#design-decisions) for approval timing, execution coverage, scope, and canonical arguments.
