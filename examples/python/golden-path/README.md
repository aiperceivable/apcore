# Python Golden Path

Register an existing application function as an APCore module and call it from Python and through MCP. The two executors use the same notebook, module definitions, and governance policy. Each uses an in-memory approval store. The MCP server binds to a temporary loopback port and uses the local process as its trust boundary.

## Run the example

Use Python 3.11 or newer. From this directory, create a clean environment and install a compatible Core and MCP pair. To test a release candidate, install its two wheels explicitly so the resolver uses the intended builds.

```bash
python -m venv .venv
. .venv/bin/activate
python -m pip install "$APCORE_WHEEL" "$APCORE_MCP_WHEEL"
python -m pip check
python verify.py
```

Set `APCORE_WHEEL` and `APCORE_MCP_WHEEL` to the absolute paths of the Core and MCP wheels. The verification runs in seconds once the dependencies are installed.

## Follow a call

1. `app.py` registers the notebook's archive operation with an input type inferred from the function signature and an output schema inferred from `ArchiveResult`. It marks the operation as destructive and configures `ExecutionPolicy(strict=True, gate_destructive=True)`.
2. `verify.py` checks a normal lookup, an ACL denial, and invalid input through both entry points. An archive request stays pending until the reviewer records a decision.
3. In Python, the verifier resolves the pending result's public `audit_id` in the store. It retries the same arguments through Core with the private `approval_id` in the reserved `_approval_token` field.
4. Through MCP, the verifier reads `approvalId` and `resumeToken` from the pending tool result's `_meta.apcore.approval`. It polls `__apcore_approval_check` with the public ID, resolves the request, and checks that the status changes to approved. It then retries the original tool arguments with the private token in request `_meta.approvalId`.
5. The verifier checks that each archive executes once and prints the ACL decisions with trace ID, caller ID, and target ID. The approval credential stays out of the audit output.

The in-memory stores are for this local example. A deployed application can use an authenticated review service and a persistent scoped approval store.
