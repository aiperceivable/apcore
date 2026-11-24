"""Tests for example modules in the examples/ directory."""

from __future__ import annotations

import importlib.util
import pathlib

import yaml
from pydantic import BaseModel

from apcore.context import Context
from apcore.module import ModuleAnnotations, ModuleExample

PROJECT_ROOT = pathlib.Path(__file__).resolve().parent.parent.parent


def _load_example_module(relative_path: str):
    """Load a Python module from a path relative to PROJECT_ROOT using importlib."""
    full_path = PROJECT_ROOT / relative_path
    spec = importlib.util.spec_from_file_location(full_path.stem, str(full_path))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


# --- GreetModule Tests ---


class TestGreetModule:
    """Test: GreetModule instantiable with required attributes."""

    def test_greet_module_has_input_schema(self):
        mod = _load_example_module("examples/modules/greet.py")
        instance = mod.GreetModule()
        assert hasattr(instance, "input_schema")
        assert issubclass(instance.input_schema, BaseModel)

    def test_greet_module_has_output_schema(self):
        mod = _load_example_module("examples/modules/greet.py")
        instance = mod.GreetModule()
        assert hasattr(instance, "output_schema")
        assert issubclass(instance.output_schema, BaseModel)

    def test_greet_module_has_description(self):
        mod = _load_example_module("examples/modules/greet.py")
        instance = mod.GreetModule()
        assert isinstance(instance.description, str)
        assert len(instance.description) > 0

    def test_greet_module_execute_returns_correct_greeting(self):
        mod = _load_example_module("examples/modules/greet.py")
        instance = mod.GreetModule()
        ctx = Context.create()
        result = instance.execute({"name": "Alice"}, ctx)
        assert result == {"message": "Hello, Alice!"}


# --- SendEmailModule Tests ---


class TestSendEmailModule:
    """Test: SendEmailModule has annotations, tags, version, examples."""

    def test_send_email_has_annotations(self):
        mod = _load_example_module("examples/modules/send_email.py")
        instance = mod.SendEmailModule()
        assert isinstance(instance.annotations, ModuleAnnotations)
        assert instance.annotations.destructive is True
        assert instance.annotations.idempotent is False

    def test_send_email_has_tags(self):
        mod = _load_example_module("examples/modules/send_email.py")
        instance = mod.SendEmailModule()
        assert isinstance(instance.tags, list)
        assert len(instance.tags) > 0

    def test_send_email_has_version(self):
        mod = _load_example_module("examples/modules/send_email.py")
        instance = mod.SendEmailModule()
        assert isinstance(instance.version, str)

    def test_send_email_has_examples(self):
        mod = _load_example_module("examples/modules/send_email.py")
        instance = mod.SendEmailModule()
        assert isinstance(instance.examples, list)
        assert len(instance.examples) > 0
        assert isinstance(instance.examples[0], ModuleExample)

    def test_send_email_input_schema_has_sensitive_field(self):
        mod = _load_example_module("examples/modules/send_email.py")
        schema = mod.SendEmailInput.model_json_schema()
        api_key_props = schema["properties"]["api_key"]
        assert api_key_props.get("x-sensitive") is True

    def test_send_email_execute_returns_status(self):
        mod = _load_example_module("examples/modules/send_email.py")
        instance = mod.SendEmailModule()
        ctx = Context.create()
        child = ctx.child("send_email")
        result = instance.execute(
            {
                "to": "test@example.com",
                "subject": "Hi",
                "body": "Hello",
                "api_key": "sk-test",
            },
            child,
        )
        assert result["status"] == "sent"
        assert "message_id" in result


# --- GetUserModule Tests ---


class TestGetUserModule:
    """Test: GetUserModule has readonly and idempotent annotations."""

    def test_get_user_has_readonly_annotation(self):
        mod = _load_example_module("examples/modules/get_user.py")
        instance = mod.GetUserModule()
        assert instance.annotations.readonly is True

    def test_get_user_has_idempotent_annotation(self):
        mod = _load_example_module("examples/modules/get_user.py")
        instance = mod.GetUserModule()
        assert instance.annotations.idempotent is True

    def test_get_user_execute_returns_user_data(self):
        mod = _load_example_module("examples/modules/get_user.py")
        instance = mod.GetUserModule()
        ctx = Context.create()
        result = instance.execute({"user_id": "user-1"}, ctx)
        assert result == {"id": "user-1", "name": "Alice", "email": "alice@example.com"}


# --- Decorated Add Tests ---


class TestDecoratedAdd:
    """Test: decorated add function has apcore_module attribute."""

    def test_add_has_apcore_module_attribute(self):
        mod = _load_example_module("examples/modules/decorated_add.py")
        assert hasattr(mod.add, "apcore_module")

    def test_add_module_produces_correct_sum(self):
        mod = _load_example_module("examples/modules/decorated_add.py")
        fm = mod.add.apcore_module
        ctx = Context.create()
        result = fm.execute({"a": 2, "b": 3}, ctx)
        assert result == {"result": 5}


# --- Format Date Binding Tests ---


class TestFormatDateBinding:
    """Test: binding.yaml is valid YAML with required fields."""

    def test_binding_yaml_is_valid(self):
        binding_path = PROJECT_ROOT / "examples" / "bindings" / "format_date" / "format_date.binding.yaml"
        data = yaml.safe_load(binding_path.read_text())
        assert "bindings" in data
        assert isinstance(data["bindings"], list)
        assert len(data["bindings"]) >= 1
        entry = data["bindings"][0]
        assert "module_id" in entry
        assert "target" in entry

    def test_format_date_function_formats_dates(self):
        mod = _load_example_module("examples/bindings/format_date/format_date.py")
        result = mod.format_date_string("2024-01-15", "%B %d, %Y")
        assert result == {"formatted": "January 15, 2024"}


class TestExamplesReadmeAccuracy:
    """The README must describe ``examples/modules/`` as it actually is.

    ``examples/README.md`` claimed the files under ``modules/`` were "imported by
    the examples above". No example script imports them — this file is their only
    automated consumer — so a reader following the README looked for a wiring
    that does not exist.
    """

    EXAMPLES_DIR = PROJECT_ROOT / "examples"
    MODULES_DIR = EXAMPLES_DIR / "modules"

    def test_no_example_script_imports_the_modules_package(self) -> None:
        stems = {p.stem for p in self.MODULES_DIR.glob("*.py") if p.stem != "__init__"}
        offenders: list[str] = []
        for script in self.EXAMPLES_DIR.rglob("*.py"):
            if self.MODULES_DIR in script.parents:
                continue
            source = script.read_text()
            for stem in stems:
                if f"from modules.{stem}" in source or f"import modules.{stem}" in source:
                    offenders.append(f"{script.name} imports modules.{stem}")
        assert offenders == [], (
            "examples/README.md says no example imports modules/; update the README " f"if that changed: {offenders}"
        )

    def test_every_reference_module_is_exercised_here(self) -> None:
        # This file is the documented consumer, so every reference module must
        # actually be loaded by it.
        source = pathlib.Path(__file__).read_text()
        missing = [
            p.name
            for p in sorted(self.MODULES_DIR.glob("*.py"))
            if p.stem != "__init__" and f"examples/modules/{p.name}" not in source
        ]
        assert missing == [], f"reference module(s) with no test here: {missing}"


class TestReadmeDocumentationLink:
    """mkdocs emits directory URLs, so ``getting-started.html`` is a 404.

    ``mkdocs.yml`` in the spec repo does not set ``use_directory_urls: false``,
    which means ``docs/getting-started.md`` publishes as ``getting-started/``.
    The README's primary documentation link pointed at the ``.html`` form.
    """

    def test_documentation_link_uses_the_directory_url(self) -> None:
        readme = (PROJECT_ROOT / "README.md").read_text()
        assert "apcore/getting-started.html" not in readme
        assert "https://aiperceivable.github.io/apcore/getting-started/" in readme
