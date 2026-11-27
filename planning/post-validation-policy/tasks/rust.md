# Rust Post-validation Policy

## Goal

Implement the approved contract under Rust's default and full feature profiles.

## Files Involved

- `rust/src/builtin_steps.rs`, `executor.rs`, `pipeline.rs`, `registry/registry.rs`, `extensions.rs`, `approval.rs`, `errors.rs`, and crate exports.
- `rust/tests/core_profile.rs`, `rust/tests/conformance_env.rs`, and the aggregate suite declared by `rust/tests/it.rs`.

## Steps

1. Read `rust/CLAUDE.md`. Add red public-entry fixture tests for the default profile and the execution suite. Include a descriptor version that differs from instance metadata or request hints.
2. Extend the execution lease with the selected descriptor version under the existing registry lock. Snapshot the evaluator with the other providers on an invocation. Reuse stock validation and terminal governance paths for normal, dry-run, and stream preparation.
3. Choose the approved compatibility shape for `ApprovalRequest`. Check public struct literals and exhaustive matches against the current API. Reuse the private credential and atomic resumption mechanisms.
4. Run `cargo test --locked --no-default-features --lib --test core_profile` and `cargo test --locked --all-features`. Run the repository's formatting and lint checks. Build the profile archive with the supported MSRV and exercise the installed package.

## Acceptance Criteria

- [ ] Shared tests prove the exact selected descriptor version, final input, and producer admission.
- [ ] Default and full profiles pass on supported toolchains with no unnecessary host dependencies.
- [ ] Existing public provider construction and strategy compatibility are verified or covered by an approved migration.

## Dependencies

Depends on the shared contract. Required by Surfaces and integration.

## Estimated Time

8 hours.

## Design Decisions Reference

See [plan.md](../plan.md#design-decisions) for the execution lease and provider contract.
