# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in apcore, please report it responsibly.

**Do NOT open a public issue for security vulnerabilities.**

Please use GitHub's private vulnerability reporting feature:
1. Go to the relevant repository's **Security** tab
2. Click **Report a vulnerability**
3. Fill in the details

Alternatively, email: **team@aiperceivable.org**

## Response Timeline

- **Acknowledgment**: Within 48 hours
- **Initial assessment**: Within 7 days
- **Fix or mitigation**: Depends on severity; critical issues targeted within 30 days

## Supported Versions

| Component | Supported Versions |
|-----------|--------------------|
| apcore spec | Latest published version |
| apcore-python | Latest release |
| apcore-typescript | Latest release |
| apcore-rust | Latest release |
| apcore-mcp-* | Latest release |
| apcore-a2a-* | Latest release |
| apcore-cli-* | Latest release |
| apcore-toolkit-* | Latest release |

Older versions receive security fixes on a best-effort basis.

## Scope

This policy covers all repositories under the
[aiperceivable](https://github.com/aiperceivable) GitHub organization
that are part of the apcore standard.

### Trust boundary: the extensions root

apcore assumes the **extensions root is at least as trusted as the host
process**. That is the case in the two deployments the project supports today:
the root ships inside the deployed artifact and is read-only at runtime, or it
is operator-managed at the host's own trust level.

A root writable by a party *less* trusted than the host is **outside this
model**, and confinement is not the control that would protect it: a writer
there can drop a module that passes every check and does whatever it likes
inside `execute()`. The symlink time-of-check/time-of-use window described in
[Discovery TOCTOU](./docs/spec/security-considerations.md#28-discovery-toctou-ot9) is one
consequence of that boundary, not a defect within it, and the assessment is
**accepted as written**: document the boundary, do not add per-platform
mechanism to close one window in a deployment the project does not claim to
support.

Reports that assume a less-trusted extensions root are still welcome — they are
evidence that the deployment should be supported, which is exactly what the
assessment names as the thing that would change the decision. Please say so in
the report rather than filing it as a confinement bypass.

## Disclosure Policy

We follow coordinated disclosure:
1. Reporter submits vulnerability privately
2. We confirm and assess the issue
3. We develop and test a fix
4. We release the fix and publish an advisory
5. Reporter is credited (unless they prefer anonymity)
