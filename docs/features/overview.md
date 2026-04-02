# Feature Overview

> Auto-generated from tech-design. See docs/context-annotations-acl/tech-design.md for architecture context.
> Updated: 2026-04-01

## Features

| # | Feature | Description | Dependencies | Priority | Status |
|---|---------|-------------|--------------|----------|--------|
| 1 | [context-redesign](./context-redesign.md) | ContextKey typed accessor, canonical field alignment, data key naming, serialization with _context_version | -- | P0 | draft |
| 2 | [annotations-redesign](./annotations-redesign.md) | ModuleAnnotations extra field extension, wire format, createAnnotations factory, ecosystem migration | -- | P0 | draft |
| 3 | [acl-conditions-redesign](./acl-conditions-redesign.md) | Condition handler registration, $or/$not compound operators, async_check, fail-closed behavior | context-redesign | P0 | draft |

> The `#` column defines the canonical execution order based on the dependency graph. Features with no dependencies come first. This is the ONLY place where feature ordering is assigned -- individual feature specs must NOT contain order numbers or numeric IDs.

## Execution Order

1. **context-redesign** -- No dependencies. Foundation for ACL (handlers read from Context). Must be implemented first.
2. **annotations-redesign** -- No dependencies. Independent of Context and ACL. Can be implemented in parallel with context-redesign.
3. **acl-conditions-redesign** -- Depends on context-redesign (handlers receive Context parameter). Must be implemented after context-redesign is complete.

> Note: context-redesign (#1) and annotations-redesign (#2) are independent and CAN be implemented in parallel. acl-conditions-redesign (#3) MUST wait for context-redesign to complete.

## Architecture Reference

For system-level concerns (solution design, API specifications, security, performance, deployment), see:
- **Tech Design**: `docs/context-annotations-acl/tech-design.md`
- **Design Input**: `docs/spec/design-context-annotations-acl.md`
