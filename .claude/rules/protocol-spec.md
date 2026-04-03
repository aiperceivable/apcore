---
paths:
  - "PROTOCOL_SPEC.md"
---

# Protocol Specification Editing Rules

You are editing the authoritative protocol specification. This is the single source of truth for all SDK implementations.

## Before any edit
- Read the full section you're modifying, not just the paragraph
- Search for related normative statements elsewhere in the file to avoid contradictions
- Check if anchor IDs exist that external repos link to — never rename or delete them

## Normative language
- Use RFC 2119 keywords: `MUST`, `MUST NOT`, `SHOULD`, `SHOULD NOT`, `MAY` (uppercase only)
- Never use lowercase "should" or "must" for normative intent
- Every new normative requirement needs a version bump in the relevant section

## Schema examples in spec
- JSON Schema Draft 2020-12 only
- Every property needs a `description` field
- Show complete, valid schema objects — never omit required fields for brevity
- Do not invent `x-` extension fields not already listed in §4.6

## Cross-language impact
- Any behavioral change affects Python, TypeScript, and Rust SDKs
- Note cross-language impact explicitly in commit messages
- Do not add Python-specific idioms to language-agnostic sections
