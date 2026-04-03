# apcore Implementation Plans — Project Overview

## Features

| Feature | Tasks | Status | Source |
|---------|-------|--------|--------|
| [context-redesign](./context-redesign/overview.md) | 7 | pending | [docs/features/context-redesign.md](../docs/features/context-redesign.md) |

## Recommended Implementation Order

1. **context-redesign** — foundational; ACL and Pipeline depend on Context changes

## Upcoming (not yet planned)

- annotations-redesign — depends on: none (parallel with context)
- acl-conditions-redesign — depends on: context-redesign
- execution-pipeline — depends on: context-redesign, acl-conditions-redesign
