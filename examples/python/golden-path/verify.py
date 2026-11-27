"""Exercise one application through direct calls and a real MCP session."""

import asyncio
import json
from contextlib import asynccontextmanager
from socket import socket
from typing import AsyncIterator

import uvicorn
from apcore import APCore
from apcore.acl import AuditEntry
from apcore.errors import ACLDeniedError, ApprovalPendingError, SchemaValidationError
from apcore_mcp import APCoreMCP
from mcp import ClientSession
from mcp.client.streamable_http import streamable_http_client
from mcp.types import CallToolResult, TextContent

from app import ARCHIVE, LOOKUP, RESTRICTED, DemoReviewQueue, Notebook, build_app, build_mcp


async def verify_direct(client: APCore, notebook: Notebook, reviews: DemoReviewQueue) -> None:
    try:
        await client.call_async(RESTRICTED, {})
    except ACLDeniedError:
        pass
    else:
        raise AssertionError("Expected an ACL denial")

    try:
        await client.call_async(LOOKUP, {"note_id": ["invalid"]})
    except SchemaValidationError:
        pass
    else:
        raise AssertionError("Expected input validation to fail")

    arguments = {"note_id": "from-code"}
    try:
        await client.call_async(ARCHIVE, arguments)
    except ApprovalPendingError as pending:
        approval_id = pending.approval_id
        assert approval_id is not None
    else:
        raise AssertionError("Expected approval to be pending")
    assert notebook.archived == []
    reviews.approve(approval_id)
    assert await client.call_async(ARCHIVE, {**arguments, "_approval_token": approval_id}) == {"archived": "from-code"}
    assert notebook.archived == ["from-code"]


@asynccontextmanager
async def local_session(mcp: APCoreMCP) -> AsyncIterator[ClientSession]:
    async with mcp.async_serve() as application:
        with socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
            config = uvicorn.Config(application, lifespan="off", log_level="warning")
            server = uvicorn.Server(config)
            task = asyncio.create_task(server.serve(sockets=[listener]))
            try:
                async with asyncio.timeout(10):
                    while not server.started:
                        if task.done():
                            task.result()
                            raise RuntimeError("The MCP server stopped before startup")
                        await asyncio.sleep(0.01)
                url = f"http://127.0.0.1:{port}/mcp"
                async with (
                    streamable_http_client(url, terminate_on_close=False) as (read, write, _),
                    ClientSession(read, write) as session,
                ):
                    await session.initialize()
                    yield session
            finally:
                server.should_exit = True
                await task


def tool_text(result: CallToolResult) -> str:
    (content,) = result.content
    assert isinstance(content, TextContent)
    return content.text


async def verify_mcp(mcp: APCoreMCP, notebook: Notebook, reviews: DemoReviewQueue) -> None:
    async with local_session(mcp) as session:
        tools = {tool.name: tool for tool in (await session.list_tools()).tools}
        assert tools[ARCHIVE].inputSchema["properties"]["note_id"]["type"] == "string"
        denied = await session.call_tool(RESTRICTED, {})
        assert denied.isError and "Access denied" in tool_text(denied)
        invalid = await session.call_tool(LOOKUP, {"note_id": ["invalid"]})
        assert invalid.isError and "validation" in tool_text(invalid).lower()

        arguments = {"note_id": "from-mcp"}
        pending = await session.call_tool(ARCHIVE, arguments)
        assert pending.isError and "Approval pending" in tool_text(pending)
        assert notebook.archived == ["from-code"]
        # The local review host gives the initiator the resume credential out of band.
        (approval_id,) = reviews.pending
        reviews.approve(approval_id)
        approved = await session.call_tool(ARCHIVE, {**arguments, "_approval_token": approval_id})
        assert not approved.isError, tool_text(approved)
        assert json.loads(tool_text(approved)) == {"archived": "from-mcp"}
        assert notebook.archived == ["from-code", "from-mcp"]


async def main() -> None:
    notebook = Notebook()
    reviews = DemoReviewQueue()
    audit: list[AuditEntry] = []
    client = build_app(notebook, reviews, audit)
    try:
        await verify_direct(client, notebook, reviews)
        await verify_mcp(build_mcp(client, reviews), notebook, reviews)
    finally:
        await client.shutdown()

    assert sum(entry.decision == "deny" and entry.target_id == RESTRICTED for entry in audit) == 2
    assert sum(entry.decision == "allow" and entry.target_id == ARCHIVE for entry in audit) == 4
    assert all(entry.trace_id for entry in audit)
    for entry in audit:
        print(entry.trace_id, entry.caller_id, entry.target_id, entry.decision)
    print("Direct calls and MCP both passed the governance checks.")


if __name__ == "__main__":
    asyncio.run(main())
