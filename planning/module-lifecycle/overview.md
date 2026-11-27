# Module Lifecycle Implementation

Implement the [technical design](./tech-design.md) across Python, TypeScript, and Rust. Phase A establishes owned host operations and wait budgets. Phase B implements module hooks, governed reload, scoped cancellation, and canonical events.

## Task Execution Order

| Task | Scope | Dependency |
|---|---|---|
| [python](./tasks/python.md) | Python host APIs and lifecycle assertions | Shared interface and fixtures |
| [typescript](./tasks/typescript.md) | TypeScript host APIs and lifecycle assertions | Shared interface and fixtures |
| [rust](./tasks/rust.md) | Rust host APIs and lifecycle assertions | Shared interface and fixtures |
| [managed-lifecycle](./tasks/managed-lifecycle.md) | Managed hooks, forced removal, governed reload, events, and shared verification | Phase A in all three SDKs |
