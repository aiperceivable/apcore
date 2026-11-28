# Cross-Language and Candidate Verification

Check the shared behaviors, existing full suites, typed and protocol
integration tests, and installed candidates listed in [the plan](../plan.md).

The shared call budget fixture runs in Python, TypeScript, and Rust. The
focused case-pinning run detected mutations of all three cases in every SDK.
The conformance inventory covers 102 fixtures and 1,272 cases.

The paired run installs the wheel, npm tarball, and Rust crate from the Core
distribution run, validates their digests, and checks the supported minimum
for each applicable adapter. All 16 jobs passed. The four routine Surfaces
SDK and documentation workflows passed for the same Surfaces commit.

- Core runtime commit `7573f0de32634e00ba0585d42094613c0add9a76`. [Distribution verification](https://github.com/aiperceivable/apcore/actions/runs/46143380753)
- Surfaces commit `33e4d6d864d163ae3f297e04454fcba28da1a584`. [Paired compatibility](https://github.com/aiperceivable/apcore-surfaces/actions/runs/46143692117)

The proposed protocol revision awaits maintainer approval before the final PRs.
Released packages and external pilot evidence are tracked separately in
the blueprint's GA criteria.
