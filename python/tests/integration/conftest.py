"""Shared fixtures for integration tests."""

from __future__ import annotations

import sys

import pytest

from apcore.acl import ACL
from apcore.context import Context
from apcore.executor import Executor
from apcore.registry import Registry


# --- Module file templates ---

GREET_MODULE = """\
from pydantic import BaseModel


class GreetInput(BaseModel):
    name: str


class GreetOutput(BaseModel):
    message: str


class GreetModule:
    input_schema = GreetInput
    output_schema = GreetOutput
    description = "Greet module"
    tags = ["greet"]

    def execute(self, inputs, context=None):
        return {"message": f"Hello, {inputs['name']}!"}
"""

FAILING_MODULE = """\
from pydantic import BaseModel, ConfigDict


class FailingInput(BaseModel):
    model_config = ConfigDict(extra="allow")


class FailingOutput(BaseModel):
    model_config = ConfigDict(extra="allow")


class FailingModule:
    input_schema = FailingInput
    output_schema = FailingOutput
    description = "Failing module"
    tags = ["failing"]

    def execute(self, inputs, context=None):
        raise RuntimeError("module execution failed")
"""

ASYNC_GREET_MODULE = """\
from pydantic import BaseModel


class AsyncGreetInput(BaseModel):
    name: str


class AsyncGreetOutput(BaseModel):
    message: str


class AsyncGreetModule:
    input_schema = AsyncGreetInput
    output_schema = AsyncGreetOutput
    description = "Async greet module"
    tags = ["greet", "async"]

    async def execute(self, inputs, context=None):
        return {"message": f"Hello, {inputs['name']}!"}
"""

ACL_YAML_CONTENT = """\
rules:
  - callers: ["@external"]
    targets: ["greet"]
    effect: allow
    description: "Allow external callers to access greet"
  - callers: ["greet"]
    targets: ["async_greet"]
    effect: allow
    description: "Allow greet to call async_greet"
  - callers: ["@external"]
    targets: ["failing"]
    effect: allow
    description: "Allow external callers to access failing module"
  - callers: ["*"]
    targets: ["*"]
    effect: deny
    description: "Default deny all other access"
default_effect: deny
"""


# --- Fixtures ---


@pytest.fixture
def int_extensions_dir(tmp_path):
    """Create a temp directory with importable integration test module files."""
    ext_dir = tmp_path / "int_extensions"
    ext_dir.mkdir()

    (ext_dir / "greet.py").write_text(GREET_MODULE)
    (ext_dir / "failing.py").write_text(FAILING_MODULE)
    (ext_dir / "async_greet.py").write_text(ASYNC_GREET_MODULE)

    sys.path.insert(0, str(ext_dir))
    yield ext_dir
    sys.path.remove(str(ext_dir))
    for name in ["apcore_ext_greet", "apcore_ext_failing", "apcore_ext_async_greet"]:
        sys.modules.pop(name, None)


@pytest.fixture
def int_registry(int_extensions_dir):
    """Registry with discovered integration test modules."""
    registry = Registry(extensions_dir=str(int_extensions_dir))
    count = registry.discover()
    assert count >= 3
    return registry


@pytest.fixture
def int_executor(int_registry):
    """Executor with integration test registry, no ACL."""
    return Executor(registry=int_registry)


@pytest.fixture
def int_acl_executor(int_registry, tmp_path):
    """Executor with integration test registry and ACL rules."""
    acl_path = tmp_path / "acl.yaml"
    acl_path.write_text(ACL_YAML_CONTENT)
    acl = ACL.load(str(acl_path))
    return Executor(registry=int_registry, acl=acl)


@pytest.fixture
def int_context():
    """Fresh context for integration tests."""
    return Context.create()
