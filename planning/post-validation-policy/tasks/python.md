# Python Post-validation Policy

## Goal

Implement the approved contract through Python's normal execution, preflight, and stream APIs.

## Files Involved

- `python/src/apcore/builtin_steps.py`, `executor.py`, `pipeline.py`, `registry/`, `extensions.py`, `approval.py`, `errors.py`, and existing public entry points.
- `python/tests/conformance/` and existing policy, governance, and stream tests under `python/tests/`.

## Steps

1. Read `python/CLAUDE.md`. Add the public conformance driver and run the new cases against the baseline for a red result. Exercise Pydantic-native validated arguments, module-owned defaults, and before middleware changing the submitted value.
2. Add evaluator wiring through `ExtensionManager`. Reuse the current approval gate's requirement calculation, handler path, and private credential slot. Capture the selected registration's version and dispatch only a snapshot bound to the final check.
3. Reuse the existing terminal error machinery. Route preflight through the evaluator after authorized validation. Verify that a denial cannot become a recovery result and that evaluator timeout and cancellation stay distinct.
4. Run `python -m pytest python/tests/conformance/` plus focused governance, strategy, and stream suites. Build and test both installation profiles and run the SDK's normal lint and type checks.

## Acceptance Criteria

- [ ] Native execution argument fidelity and approval scope are covered by the shared cases.
- [ ] Terminal failures, rejected token reuse, and no-provider preflight effects are observed at public APIs.
- [ ] The minimal installed profile and the legacy no-evaluator strategy pass their existing coverage.

## Dependencies

Depends on the shared contract. Required by Surfaces and integration.

## Estimated Time

6 hours.

## Design Decisions Reference

See [plan.md](../plan.md#design-decisions) for approval timing and the argument snapshot.
