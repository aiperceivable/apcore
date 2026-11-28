# MCP and A2A

Start from the approved Core identity APIs. Extend the existing JWT authenticators in Python, TypeScript, and Rust. Keep verification and claim projection together per adapter and reuse the existing route-to-Context paths.

- Exercise actual authentication middleware and real protocol calls. Check invalid signatures, absent or mismatched required verification claims, scope conflicts and audit-only historical actors. The resource server accepts the token's current scope ceiling and does not infer grants from previous actors.
- Check ownership and permission separately for stream sessions, MCP and A2A tasks, event subscriptions, approval polling/resumption and A2A visibility caches. Cover the same actor acting for distinct subjects, a refreshed credential with narrower authority, and an anonymous request targeting a delegated task. Exercise Python and Rust MCP session ownership, TypeScript MCP SSE reuse, and all A2A task and push-store owner buckets. Warm the Rust A2A capability cache before reducing the caller's scope.
- Verify that incoming protocol request fields cannot supply authority. Ensure malformed opt-in claims do not continue as an anonymous request. Preserve the direct mapping path when delegation is disabled.
