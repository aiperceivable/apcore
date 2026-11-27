# TypeScript Lifecycle Operations

Follow [Phase A design](../tech-design.md#compatibility-and-rollout) and the shared cases in [module_lifecycle.json](../../../conformance/fixtures/module_lifecycle.json). Own runtime and tests under `typescript/`. Validate the new driver against the unimplemented entry points before introducing production behavior. Verify the new and existing lifecycle, registry, event, client, and execution paths afterwards.
