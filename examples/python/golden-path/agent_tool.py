"""Expose the same governed archive operation to an in-process Agents SDK agent."""

from agents import FunctionTool, function_tool
from apcore import BoundModule
from app import ArchiveResult


def archive_tool(archive: BoundModule[[str], ArchiveResult]) -> FunctionTool:
    @function_tool
    async def archive_note(note_id: str) -> str:
        """Archive a note by its ID."""
        result = await archive.invoke_async(note_id)
        return result.model_dump_json()

    return archive_note
