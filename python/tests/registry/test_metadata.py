"""Tests for metadata and ID map loading."""

from __future__ import annotations

import logging
from pathlib import Path

import pytest
import yaml

from apcore.errors import ConfigError, ConfigNotFoundError
from apcore.registry.metadata import (
    load_id_map,
    load_metadata,
    merge_module_metadata,
    parse_dependencies,
)


# === load_metadata() ===


class TestLoadMetadata:
    def test_valid_yaml_all_fields(self, tmp_path: Path) -> None:
        """Valid YAML with all fields returns parsed dict."""
        meta = tmp_path / "mod_meta.yaml"
        meta.write_text(
            yaml.dump(
                {
                    "description": "Test module",
                    "tags": ["tag1", "tag2"],
                    "version": "2.0.0",
                    "dependencies": [{"module_id": "foo.bar"}],
                    "entry_point": "mod:MyModule",
                }
            )
        )
        result = load_metadata(meta)
        assert result["description"] == "Test module"
        assert result["tags"] == ["tag1", "tag2"]
        assert result["version"] == "2.0.0"

    def test_nonexistent_file_returns_empty(self, tmp_path: Path) -> None:
        """Non-existent file returns empty dict."""
        assert load_metadata(tmp_path / "nonexistent_meta.yaml") == {}

    def test_invalid_yaml_raises(self, tmp_path: Path) -> None:
        """Invalid YAML content raises ConfigError."""
        meta = tmp_path / "bad_meta.yaml"
        meta.write_text("{{invalid yaml:")
        with pytest.raises(ConfigError):
            load_metadata(meta)

    def test_partial_fields(self, tmp_path: Path) -> None:
        """YAML with only some fields returns partial dict."""
        meta = tmp_path / "partial_meta.yaml"
        meta.write_text(yaml.dump({"description": "partial", "tags": ["t1"]}))
        result = load_metadata(meta)
        assert result["description"] == "partial"
        assert result["tags"] == ["t1"]
        assert "version" not in result

    def test_empty_file_returns_empty(self, tmp_path: Path) -> None:
        """Empty file returns empty dict."""
        meta = tmp_path / "empty_meta.yaml"
        meta.write_text("")
        assert load_metadata(meta) == {}


# === parse_dependencies() ===


class TestParseDependencies:
    def test_list_of_dicts(self) -> None:
        """List of dicts returns DependencyInfo objects."""
        result = parse_dependencies([{"module_id": "foo.bar"}, {"module_id": "baz.qux"}])
        assert len(result) == 2
        assert result[0].module_id == "foo.bar"
        assert result[0].optional is False
        assert result[0].version is None

    def test_optional_true(self) -> None:
        """Dependency with optional=True."""
        result = parse_dependencies([{"module_id": "foo", "optional": True}])
        assert result[0].optional is True

    def test_version_constraint(self) -> None:
        """Dependency with version constraint."""
        result = parse_dependencies([{"module_id": "foo", "version": ">=1.0.0"}])
        assert result[0].version == ">=1.0.0"

    def test_empty_list(self) -> None:
        """Empty list returns empty list."""
        assert parse_dependencies([]) == []


class TestParseDependenciesMalformed:
    """A malformed ``dependencies:`` MUST degrade, never abort ``discover()``.

    The key comes from hand-written filesystem YAML and no schema describes
    it, so any shape can arrive. A truthy scalar used to slip past the
    falsiness guard and make the ``for dep in deps_raw`` loop iterate
    *characters*, raising a bare ``AttributeError`` that aborted the whole
    discovery pass rather than skipping the one bad module. Both peers guard
    it and return ``[]`` — apcore-typescript with ``Array.isArray``,
    apcore-rust with ``.as_array()``.
    """

    @pytest.mark.parametrize("scalar", ["foo", 42, 3.5, True])
    def test_scalar_returns_empty_without_raising(self, scalar: object) -> None:
        assert parse_dependencies(scalar) == []

    def test_mapping_returns_empty_without_raising(self) -> None:
        """A mapping is the most common YAML slip (``dependencies: {module_id: x}``)."""
        assert parse_dependencies({"module_id": "foo.bar"}) == []

    def test_scalar_logs_a_warning(self, caplog: pytest.LogCaptureFixture) -> None:
        with caplog.at_level(logging.WARNING, logger="apcore.registry.metadata"):
            parse_dependencies("foo")
        assert any("must be a list" in msg for msg in caplog.messages)

    def test_list_of_non_mappings_skips_the_bad_entries(self, caplog: pytest.LogCaptureFixture) -> None:
        with caplog.at_level(logging.WARNING, logger="apcore.registry.metadata"):
            result = parse_dependencies(["foo", 42, None, {"module_id": "good.one"}])
        assert [d.module_id for d in result] == ["good.one"]
        assert any("must be a mapping" in msg for msg in caplog.messages)

    def test_none_returns_empty(self) -> None:
        assert parse_dependencies(None) == []


# === merge_module_metadata() ===


class TestMergeModuleMetadata:
    def test_yaml_overrides_code(self) -> None:
        """YAML values override code attributes."""

        class MockModule:
            description = "code desc"
            tags = ["code_tag"]
            version = "1.0.0"

        meta = {"description": "yaml desc", "tags": ["yaml_tag"], "version": "2.0.0"}
        result = merge_module_metadata(MockModule, meta)
        assert result["description"] == "yaml desc"
        assert result["tags"] == ["yaml_tag"]
        assert result["version"] == "2.0.0"

    def test_code_attrs_when_yaml_empty(self) -> None:
        """Code attributes used when YAML is empty."""

        class MockModule:
            description = "code desc"
            tags = ["t1"]
            version = "1.0.0"

        result = merge_module_metadata(MockModule, {})
        assert result["description"] == "code desc"
        assert result["tags"] == ["t1"]
        assert result["version"] == "1.0.0"

    def test_defaults_for_missing(self) -> None:
        """Minimal class with empty meta gives defaults."""

        class MockModule:
            description = "d"

        result = merge_module_metadata(MockModule, {})
        assert result["description"] == "d"
        assert result["tags"] == []
        assert result["version"] == "1.0.0"

    def test_metadata_dicts_merged(self) -> None:
        """Nested metadata dicts are shallow-merged (YAML wins on conflicts)."""

        class MockModule:
            description = "d"
            metadata = {"key1": "val1", "shared": "code"}

        meta = {"metadata": {"key2": "val2", "shared": "yaml"}}
        result = merge_module_metadata(MockModule, meta)
        assert result["metadata"]["key1"] == "val1"
        assert result["metadata"]["key2"] == "val2"
        assert result["metadata"]["shared"] == "yaml"

    def test_annotations_field_level_merge(self) -> None:
        """Spec §4.13: YAML annotations merge over code annotations field-by-field.

        A YAML annotation that flips one flag must NOT blow away unrelated
        flags set on the code-level annotations dataclass.
        """
        from apcore.module import ModuleAnnotations

        class MockModule:
            description = "d"
            annotations = ModuleAnnotations(readonly=True, idempotent=True)

        # YAML only overrides `destructive`; readonly/idempotent must survive.
        meta = {"annotations": {"destructive": True}}
        result = merge_module_metadata(MockModule, meta)
        assert isinstance(result["annotations"], ModuleAnnotations)
        assert result["annotations"].destructive is True
        assert result["annotations"].readonly is True
        assert result["annotations"].idempotent is True

    def test_annotations_yaml_only(self) -> None:
        """YAML annotations are honored when no code annotations exist."""
        from apcore.module import ModuleAnnotations

        class MockModule:
            description = "d"

        result = merge_module_metadata(MockModule, {"annotations": {"readonly": True}})
        assert isinstance(result["annotations"], ModuleAnnotations)
        assert result["annotations"].readonly is True

    def test_annotations_none_when_neither_provided(self) -> None:
        """Result.annotations is None when neither code nor YAML defines them."""

        class MockModule:
            description = "d"

        result = merge_module_metadata(MockModule, {})
        assert result["annotations"] is None

    def test_examples_yaml_overrides_code(self) -> None:
        """Spec §4.13: YAML examples take full priority over code examples."""
        from apcore.module import ModuleExample

        class MockModule:
            description = "d"
            examples = [ModuleExample(title="from_code", inputs={}, output={})]

        meta = {"examples": [{"title": "from_yaml", "inputs": {"x": 1}, "output": {"y": 2}}]}
        result = merge_module_metadata(MockModule, meta)
        assert len(result["examples"]) == 1
        assert result["examples"][0].title == "from_yaml"
        assert result["examples"][0].inputs == {"x": 1}


# === load_id_map() ===


class TestLoadIdMap:
    def test_valid_id_map(self, tmp_path: Path) -> None:
        """Valid ID map returns dict keyed by file path."""
        f = tmp_path / "id_map.yaml"
        f.write_text(
            yaml.dump(
                {
                    "mappings": [
                        {
                            "file": "extensions/email/send.py",
                            "id": "email.send",
                            "class": "SendModule",
                        },
                        {"file": "extensions/legacy/old.py", "id": "legacy.old"},
                    ]
                }
            )
        )
        result = load_id_map(f)
        assert len(result) == 2
        assert result["extensions/email/send.py"]["id"] == "email.send"
        assert result["extensions/email/send.py"] == {"id": "email.send"}

    def test_class_field_is_not_carried(self, tmp_path: Path) -> None:
        """An entry carries `file` and `id` only (D-138); a stray `class` is not read."""
        f = tmp_path / "id_map.yaml"
        f.write_text(yaml.dump({"mappings": [{"file": "mod.py", "id": "m", "class": "MyClass"}]}))
        result = load_id_map(f)
        assert result["mod.py"] == {"id": "m"}

    def test_nonexistent_raises(self, tmp_path: Path) -> None:
        """Non-existent file raises ConfigNotFoundError."""
        with pytest.raises(ConfigNotFoundError):
            load_id_map(tmp_path / "missing.yaml")

    def test_invalid_yaml_raises(self, tmp_path: Path) -> None:
        """Invalid YAML raises ConfigError."""
        f = tmp_path / "bad.yaml"
        f.write_text("{{invalid")
        with pytest.raises(ConfigError):
            load_id_map(f)

    def test_entry_without_class_field(self, tmp_path: Path) -> None:
        """An entry maps its file to its id."""
        f = tmp_path / "id_map.yaml"
        f.write_text(yaml.dump({"mappings": [{"file": "mod.py", "id": "my.mod"}]}))
        result = load_id_map(f)
        assert result["mod.py"] == {"id": "my.mod"}
