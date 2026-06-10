# apcore Governance

## Overview

apcore is a specification and set of reference implementations for AI-perceivable
module standards. This document describes the governance model for the apcore project
and its sub-projects.

## Scope

This governance covers:
- **apcore** — Protocol specification (docs/spec/protocol-spec.md)
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
- **Spec changes**: Changes to docs/spec/protocol-spec.md require review and approval by at least 2 maintainers (or all maintainers if fewer than 3 exist)

## Roles and Promotion Ladder

apcore recognizes a progression of roles. Each rung lists what the role may do, how it is
earned, and who decides. Movement up the ladder is based on demonstrated, sustained
contribution — not tenure.

| Role | Responsibilities & access | How it is earned | Who decides |
|------|---------------------------|------------------|-------------|
| **Contributor** | Opens issues and PRs, participates in discussions. | Sign off your first PR with the DCO. Open to anyone. | — (open to all) |
| **Reviewer** | Trusted to review PRs in a specific area; listed in `CODEOWNERS` for that area. Reviews carry weight in merge decisions. | A track record of quality PRs and helpful reviews in an area (as a guideline, ~5+ merged PRs over 1+ month). | Proposed by any maintainer; confirmed by lazy consensus of maintainers. |
| **Maintainer** | Full commit/merge access, sets technical direction, votes on governance, approves spec changes. | Sustained, high-quality contribution and stewardship over 3+ months, demonstrated architectural understanding, and reliability as a reviewer. | Nominated by an existing maintainer; requires majority approval of current maintainers. |

> As the project grows under a foundation, an additional **Steering / Technical Committee**
> rung may be added for cross-sub-project direction. It is intentionally left undefined for now
> to avoid premature structure.

### Promotion process

1. Any maintainer opens a nomination (an issue, or a private thread if the candidate prefers),
   citing concrete contributions — PRs, reviews, and design input.
2. Maintainers discuss for at least 7 days.
3. Decision: **Reviewer** promotions pass by lazy consensus (no objection within 7 days);
   **Maintainer** promotions require an explicit majority of current maintainers.
4. On acceptance, the person is added to [MAINTAINERS.md](MAINTAINERS.md) and granted the
   corresponding access.

### Inactivity, emeritus, and stepping down

- A maintainer or reviewer may step down at any time by opening a PR that moves themselves to
  the Emeritus list.
- After 6 months of inactivity (no reviews, commits, or governance participation), a maintainer
  may be moved to Emeritus by majority vote. Emeritus members remain credited in
  [MAINTAINERS.md](MAINTAINERS.md) and may return by request to the active maintainers.
- Elevated access (commit, merge, release secrets) is revoked when a member becomes Emeritus and
  restored if they return.

## Sub-projects

Each language SDK and bridge is a sub-project under this governance. Sub-projects
may have additional maintainers specific to that language or domain, but all
sub-projects follow this governance document.

## Code of Conduct

All participants must follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Changes to Governance

Changes to this document require approval from 2/3 of current maintainers.
