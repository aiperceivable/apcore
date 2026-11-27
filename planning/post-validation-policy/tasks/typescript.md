# TypeScript Post-validation Policy

## Goal

Implement the approved contract in the portable and Node TypeScript SDK assemblies.

## Files Involved

- `typescript/src/builtin-steps.ts`, `executor.ts`, `pipeline.ts`, `registry/`, `extensions.ts`, `approval.ts`, `errors.ts`, and existing public entry points.
- `typescript/tests/` public conformance, approval, execution, and streaming tests.

## Steps

1. Read `typescript/CLAUDE.md`. Add red public-entry tests for the contract and for recovery that would otherwise hide a terminal policy denial.
2. Integrate the evaluator through instance-owned `ExtensionManager` wiring. Capture the resolved registration version and the validated invocation snapshot. Ensure the actual execute and producer input is the covered snapshot.
3. Reuse existing gate and token storage. Protect the executor-level result from policy bypass through step jumps or error middleware. Verify a policy evaluator installed on one client does not affect other clients.
4. Run `pnpm exec vitest run` with the shared driver and focused suites, then the normal SDK checks. Package and verify direct calls and stream behavior from the portable and full entries.

## Acceptance Criteria

- [ ] Shared policy cases pass through `call`, preflight, and the native stream API.
- [ ] The portable profile remains free of Node-only imports from policy wiring and digest implementation.
- [ ] Conformance covers mutation attempts and denied recovery after the final authorization boundary.

## Dependencies

Depends on the shared contract. Required by Surfaces and integration.

## Estimated Time

6 hours.

## Design Decisions Reference

See [plan.md](../plan.md#design-decisions) for approval timing and the argument snapshot.
