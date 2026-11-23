"""Minimal example module: greet a user by name."""

from pydantic import BaseModel


class GreetInput(BaseModel):
    """Input schema for the greet module."""

    name: str


class GreetOutput(BaseModel):
    """Output schema for the greet module."""

    message: str


class GreetModule:
    """A simple greeting module.

    Demonstrates the minimal duck-typed module interface:
    - input_schema (Pydantic BaseModel subclass)
    - output_schema (Pydantic BaseModel subclass)
    - execute(inputs, context) method
    - description string
    """

    input_schema = GreetInput
    output_schema = GreetOutput
    description = "Greet a user by name"

    def execute(self, inputs: dict, context) -> dict:
        """Execute the greeting."""
        name = inputs["name"]
        return {"message": f"Hello, {name}!"}
