# Core SDK Implementation

The Python, TypeScript, and Rust implementations can proceed in parallel after the shared contract is approved. Each owns its SDK and conformance driver. Use the active built-in pipeline gate lookup and existing effective annotation and approval policy logic. Reuse the registry's registration generation and version resolution to guard the metadata view across asynchronous boundaries.

1. Add failing public-entry tests from the shared cases. Show that hidden entries stay hidden and single-ID expansion is equally protected. Assert that discovery leaves audit, error history, approval, evaluator, and module callback counters untouched.
2. Add the silent visibility branch at the ACL decision layer. Preserve existing call-path auditing and first-match semantics. Exercise policies with argument-dependent and custom rules without synthesizing request arguments.
3. Build the filtered L1 and L2 projections in the executor, forward the same trusted context from `APCore`, and reuse the same service in the read-only manifest search module. Keep registry factories and application adapters free of authorization logic.
4. Check candidate generation changes, provider output with mixed eligibility, and invalid, expired, tampered, or cross-identity cursors. Run the language-specific checks and fixtures through the normal runtime API.
