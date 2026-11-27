# Python Golden Path

Register an existing application function as an APCore module, call it from Python, and expose the same executor over MCP. The example uses an in-memory notebook and an application-owned approval queue. The MCP server binds only to a temporary loopback port and uses the local process as its trust boundary.

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

1. `app.py` registers the notebook's archive operation with an input type inferred from the function signature and an output schema inferred from `ArchiveResult`. It marks the operation as destructive and configures an explicit `ExecutionPolicy(strict=True, gate_destructive=True)`.
2. `verify.py` calls the same registered module from Python and through MCP. An ACL denial and invalid input stop execution. Archiving returns pending without modifying the notebook. The application reviewer approves the request and the initiator resumes it with the same arguments and its private credential. The review queue matches the request and consumes the credential on successful authorization.
3. The verifier checks that each authorized archive executes once. It prints ACL decisions with trace ID, caller ID, and target ID so the two entry points can be compared. The approval credential stays out of the audit output.

This MCP adapter returns pending status as an error message. The local application hands the initiator its private resume credential through the review queue. A deployed application can replace the demonstration queue with its own authenticated review service and persistent store.
