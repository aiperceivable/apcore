# Contributing to apcore

Thank you for your interest in contributing to apcore!

## Ways to Contribute

- **Report bugs**: Open an issue describing the problem
- **Propose features**: Open an issue with the `enhancement` label
- **Submit code**: Fork, branch, and open a pull request
- **Improve docs**: Fix typos, add examples, clarify language
- **Review PRs**: Help review open pull requests

## Protocol Specification

The specification lives in [docs/spec/protocol-spec.md](docs/spec/protocol-spec.md). Changes to
the spec require an issue discussing the change before a PR is opened.
Spec changes must be reviewed and approved by at least 2 maintainers — **or by all maintainers when fewer than 3 exist**, per [GOVERNANCE.md](GOVERNANCE.md) § Decision Making. With a single maintainer listed in [MAINTAINERS.md](MAINTAINERS.md), that is one approval. This qualifier used to be missing here, which made the requirement read as unsatisfiable for a single-maintainer project and put this file in conflict with GOVERNANCE.md.

## Language SDKs

Clone this repository once. Each SDK has its own setup and test commands.

| SDK | Directory | Setup |
|-----|-----------|-------|
| Python | [`python/`](./python/) | `python -m pip install -e '.[dev]'` |
| TypeScript | [`typescript/`](./typescript/) | `pnpm install --frozen-lockfile` |
| Rust | [`rust/`](./rust/) | `cargo build` |

Run the commands from the corresponding SDK directory. The test suites read the shared fixtures in `conformance/` at the repository root.

The SDK workflows run the Python and Rust suites on their declared minimum versions as well as the primary versions. The TypeScript source suite uses the development Node toolchain, and the [distribution workflow](.github/workflows/packages.yml) installs the packed package on the minimum Node version. The distribution workflow extracts the marked Quick Start and governed-stream code blocks from the docs and runs them against all three installed SDKs. Update the code blocks directly when their public API calls change.

## Pull Request Process

1. Fork the repository and create a branch from `main`
2. Make your changes with clear commit messages
3. Add or update tests as appropriate
4. Ensure all tests pass
5. Open a PR with a description of the changes and why they are needed
6. Wait for review — maintainers aim to respond within 7 days

## Developer Certificate of Origin (DCO)

All contributions must be signed off with DCO:

```
git commit -s -m "Description of change"
```

This certifies that you have the right to submit the contribution under
the project's license. See [developercertificate.org](https://developercertificate.org/).

## Coding Guidelines

- Follow the existing code style in each SDK
- Write tests for new functionality
- Keep PRs focused — one logical change per PR
- Update documentation when changing user-facing behavior

## Cross-Language Consistency

When a change affects the protocol or module behavior, include the affected
SDKs and their conformance drivers in the same pull request.

## Reporting Security Issues

Please see [SECURITY.md](SECURITY.md) for instructions on reporting
security vulnerabilities. **Do not open public issues for security bugs.**

## License

By contributing, you agree that your contributions will be licensed under
the [Apache License 2.0](LICENSE).
