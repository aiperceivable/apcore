# Implementation Plans (internal)

Shared internal working area for implementation plans and delivery records. Write these
documents in English and keep them in version control. Each active feature plan uses
`planning/<feature>/` with `overview.md`, `plan.md`, `state.json` and `tasks/` (see
`.claude/rules/planning.md`). Remove a feature's planning directory once it has shipped.
Record lasting specifications, changes, and decisions in the spec, CHANGELOG, and decision records.

## Active implementation plans

| Plan | Scope | Progress |
|---|---|---|
| [Module lifecycle](module-lifecycle/overview.md) | Owned shutdown, detailed removal, and managed reload in Python, TypeScript, and Rust | [Phases A and B complete](module-lifecycle/state.json) |
| [APCore project review remediation](apcore-project-review-2026-11-26/overview.md) | Findings 1–8 and 26 across the runtime, wire contracts, observability, and conformance | [9 of 9 complete](apcore-project-review-2026-11-26/state.json) |

The review remediation restored the fixture inventory before the eight runtime changes. Its verification record covers the combined SDK, fixture, and documentation checks.

## Cross-repository execution documents

- [APCore microkernel cross-repository remediation and delivery plan](microkernel-delivery/plan.md)
- [Microkernel first-batch execution record](microkernel-delivery/execution.md)
- [Streaming completion contract and A2 candidate acceptance](microkernel-delivery/c2-execution.md)
- [Component migration, downstream integration, and release verification](microkernel-delivery/remaining-execution.md)
