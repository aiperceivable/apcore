"""Readonly example module: look up a user by ID."""

from pydantic import BaseModel

from apcore.module import ModuleAnnotations


class GetUserInput(BaseModel):
    """Input schema for get_user module."""

    user_id: str


class GetUserOutput(BaseModel):
    """Output schema for get_user module."""

    id: str
    name: str
    email: str


class GetUserModule:
    """Look up a user by ID.

    Demonstrates readonly and idempotent annotations.
    This module only reads data -- no side effects.
    """

    input_schema = GetUserInput
    output_schema = GetUserOutput
    description = "Get user details by ID"
    annotations = ModuleAnnotations(
        readonly=True,
        idempotent=True,
    )

    _users = {
        "user-1": {"id": "user-1", "name": "Alice", "email": "alice@example.com"},
        "user-2": {"id": "user-2", "name": "Bob", "email": "bob@example.com"},
    }

    def execute(self, inputs: dict, context) -> dict:
        """Look up a user from the simulated database."""
        user_id = inputs["user_id"]
        user = self._users.get(user_id)
        if user is None:
            return {"id": user_id, "name": "Unknown", "email": "unknown@example.com"}
        return dict(user)
