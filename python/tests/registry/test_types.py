"""Tests for registry types: ModuleDescriptor, DiscoveredModule, DependencyInfo."""

from __future__ import annotations

from pathlib import Path

from apcore.module import ModuleAnnotations, ModuleExample
from apcore.registry.types import DependencyInfo, DiscoveredModule, ModuleDescriptor


# === ModuleDescriptor tests ===


class TestModuleDescriptor:
    def test_creates_with_required_fields(self) -> None:
        """ModuleDescriptor stores all required fields correctly."""
        desc = ModuleDescriptor(
            module_id="email.send",
            name="Send Email",
            description="Sends an email",
            documentation=None,
            input_schema={"type": "object", "properties": {"to": {"type": "string"}}},
            output_schema={
                "type": "object",
                "properties": {"sent": {"type": "boolean"}},
            },
        )
        assert desc.module_id == "email.send"
        assert desc.name == "Send Email"
        assert desc.description == "Sends an email"
        assert desc.input_schema == {
            "type": "object",
            "properties": {"to": {"type": "string"}},
        }
        assert desc.output_schema == {
            "type": "object",
            "properties": {"sent": {"type": "boolean"}},
        }

    def test_defaults_for_optional_fields(self) -> None:
        """Optional fields have correct default values."""
        desc = ModuleDescriptor(
            module_id="test.mod",
            name=None,
            description="Test",
            documentation=None,
            input_schema={},
            output_schema={},
        )
        assert desc.tags == []
        assert desc.examples == []
        assert desc.metadata == {}
        assert desc.version == "1.0.0"
        assert desc.annotations is None
        assert desc.documentation is None
        assert desc.name is None

    def test_stores_annotations(self) -> None:
        """ModuleDescriptor stores ModuleAnnotations correctly."""
        annot = ModuleAnnotations(readonly=True)
        desc = ModuleDescriptor(
            module_id="test.mod",
            name=None,
            description="Test",
            documentation=None,
            input_schema={},
            output_schema={},
            annotations=annot,
        )
        assert desc.annotations is annot
        assert desc.annotations.readonly is True

    def test_stores_documentation(self) -> None:
        """ModuleDescriptor stores documentation field."""
        desc = ModuleDescriptor(
            module_id="test.mod",
            name=None,
            description="Test",
            documentation="Extended docs here",
            input_schema={},
            output_schema={},
        )
        assert desc.documentation == "Extended docs here"

    def test_all_fields_populated(self) -> None:
        """ModuleDescriptor with all fields set stores everything correctly."""
        annot = ModuleAnnotations(destructive=True, requires_approval=True)
        example = ModuleExample(title="Example 1", inputs={"x": 1}, output={"y": 2})
        desc = ModuleDescriptor(
            module_id="my.module",
            name="My Module",
            description="Does things",
            documentation="Detailed docs",
            input_schema={"type": "object"},
            output_schema={"type": "object"},
            version="2.0.0",
            tags=["tag1", "tag2"],
            annotations=annot,
            examples=[example],
            metadata={"key": "value"},
        )
        assert desc.module_id == "my.module"
        assert desc.name == "My Module"
        assert desc.description == "Does things"
        assert desc.documentation == "Detailed docs"
        assert desc.version == "2.0.0"
        assert desc.tags == ["tag1", "tag2"]
        assert desc.annotations is annot
        assert desc.examples == [example]
        assert desc.metadata == {"key": "value"}

    def test_mutable_defaults_are_independent(self) -> None:
        """Each instance gets its own list/dict for mutable defaults."""
        desc1 = ModuleDescriptor(
            module_id="a",
            name=None,
            description="A",
            documentation=None,
            input_schema={},
            output_schema={},
        )
        desc2 = ModuleDescriptor(
            module_id="b",
            name=None,
            description="B",
            documentation=None,
            input_schema={},
            output_schema={},
        )
        desc1.tags.append("x")
        assert desc2.tags == []


# === DiscoveredModule tests ===


class TestDiscoveredModule:
    def test_stores_file_path_and_canonical_id(self) -> None:
        """DiscoveredModule stores file_path as Path and canonical_id as str."""
        dm = DiscoveredModule(file_path=Path("/some/path.py"), canonical_id="some.path")
        assert dm.file_path == Path("/some/path.py")
        assert dm.canonical_id == "some.path"

    def test_defaults_meta_path_and_namespace(self) -> None:
        """Optional fields default to None."""
        dm = DiscoveredModule(file_path=Path("/some/path.py"), canonical_id="some.path")
        assert dm.meta_path is None
        assert dm.namespace is None

    def test_with_meta_path_and_namespace(self) -> None:
        """DiscoveredModule stores meta_path and namespace when set."""
        dm = DiscoveredModule(
            file_path=Path("/some/path.py"),
            canonical_id="some.path",
            meta_path=Path("/some/path_meta.yaml"),
            namespace="mynamespace",
        )
        assert dm.meta_path == Path("/some/path_meta.yaml")
        assert dm.namespace == "mynamespace"


# === DependencyInfo tests ===


class TestDependencyInfo:
    def test_defaults(self) -> None:
        """DependencyInfo defaults optional=False, version=None."""
        dep = DependencyInfo(module_id="some.module")
        assert dep.module_id == "some.module"
        assert dep.optional is False
        assert dep.version is None

    def test_with_optional_and_version(self) -> None:
        """DependencyInfo stores optional and version when set."""
        dep = DependencyInfo(module_id="some.module", optional=True, version=">=1.0.0")
        assert dep.module_id == "some.module"
        assert dep.optional is True
        assert dep.version == ">=1.0.0"
