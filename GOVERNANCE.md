# apcore Governance

## Overview

apcore is a specification and set of reference implementations for AI-perceivable
module standards. This document describes the governance model for the apcore project
and its sub-projects.

## Scope

This governance covers:
- **apcore** — Protocol specification (PROTOCOL_SPEC.md)
- **apcore-python**, **apcore-typescript**, **apcore-rust** — Language SDKs
- **apcore-toolkit**, **apcore-toolkit-python/typescript/rust** — Schema transformation toolkit
- **apcore-mcp**, **apcore-mcp-python/typescript/rust** — MCP protocol bridge
- **apcore-a2a**, **apcore-a2a-python/typescript** — A2A protocol bridge
- **apcore-cli**, **apcore-cli-python/typescript/rust** — CLI adapter

## Maintainers

The project is governed by its maintainers as listed in [MAINTAINERS.md](MAINTAINERS.md).

Maintainers have commit access and are responsible for:
- Reviewing and merging contributions
- Maintaining the specification and reference implementations
- Setting technical direction
- Ensuring cross-language consistency

## Decision Making

- **Consensus-based**: Decisions are made by consensus among maintainers
- **Lazy consensus**: Proposals are accepted if no maintainer objects within 7 days
- **Voting**: If consensus cannot be reached, a simple majority vote among maintainers decides
- **Spec changes**: Changes to PROTOCOL_SPEC.md require review and approval by at least 2 maintainers (or all maintainers if fewer than 3 exist)

## Becoming a Maintainer

New maintainers are nominated by existing maintainers based on:
- Sustained, quality contributions over 3+ months
- Demonstrated understanding of the project's goals and architecture
- Willingness to commit time to reviews and project stewardship

Nomination requires approval from a majority of current maintainers.

## Contributor Roles

| Role | Description | Requirements |
|------|-------------|--------------|
| **Contributor** | Anyone who submits a PR or issue | Sign DCO |
| **Reviewer** | Trusted to review PRs in a specific area | Consistent quality contributions |
| **Maintainer** | Full commit access, sets direction | Nominated + majority approval |

## Sub-projects

Each language SDK and bridge is a sub-project under this governance. Sub-projects
may have additional maintainers specific to that language or domain, but all
sub-projects follow this governance document.

## Code of Conduct

All participants must follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Changes to Governance

Changes to this document require approval from 2/3 of current maintainers.
