"""Decorated function example: add two numbers."""

from apcore.decorator import module


@module(description="Add two integers", tags=["math", "utility"])
def add(a: int, b: int) -> int:
    """Add two integers and return the sum."""
    return a + b
