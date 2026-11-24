"""Unit + binding-path regression tests for OpenAI strict-mode compatibility detection.

Covers protocol-spec §5.12.5.

Cross-SDK feature-list parity lives in
``tests/conformance/test_openai_strict_compat.py``; this file covers the
Python-specific wiring (``BindingLoader`` on the ``auto_schema: strict`` path),
the error payload, and the false positives the previous ad-hoc detector in
``bindings.py`` produced.
"""

from __future__ import annotations

import copy
from pathlib import Path

import pytest

from apcore.bindings import BindingLoader
from apcore.errors import BindingStrictSchemaIncompatibleError
from apcore.registry import Registry
from apcore.schema.openai_strict import (
    assert_openai_strict_compatible,
    detect_openai_strict_incompatibilities,
)


class TestDetectOpenAiStrictIncompatibilities:
    def test_compatible_schema_yields_no_findings(self) -> None:
        assert (
            detect_openai_strict_incompatibilities(
                {
                    "type": "object",
                    "properties": {"a": {"type": "string"}, "n": {"type": "integer", "minimum": 0}},
                    "required": ["a", "n"],
                    "additionalProperties": False,
                }
            )
            == []
        )

    def test_nested_anyof_is_not_reported(self) -> None:
        """OpenAI supports anyOf below the root.

        Regression: the previous detector in ``bindings.py`` flagged every
        ``anyOf``, so the nullable wrapper Pydantic emits for ``str | None``
        made *every* optional field fail ``auto_schema: strict``.
        """
        assert (
            detect_openai_strict_incompatibilities(
                {
                    "type": "object",
                    "properties": {"note": {"anyOf": [{"type": "string"}, {"type": "null"}]}},
                }
            )
            == []
        )

    def test_root_anyof_is_reported(self) -> None:
        assert detect_openai_strict_incompatibilities({"anyOf": [{"type": "object"}]}) == ["$.anyOf"]

    def test_author_written_oneof_is_reported_and_not_rewritten(self) -> None:
        schema = {
            "type": "object",
            "properties": {"mode": {"oneOf": [{"type": "string"}, {"type": "integer"}]}},
        }
        snapshot = copy.deepcopy(schema)

        assert detect_openai_strict_incompatibilities(schema) == ["$.mode.oneOf"]
        # Rewriting oneOf -> anyOf would tell the LLM "both branches matching is
        # fine" while apcore's validator still raises SCHEMA_UNION_AMBIGUOUS.
        assert schema == snapshot

    @pytest.mark.parametrize(
        "fmt",
        ["date-time", "time", "date", "duration", "email", "hostname", "ipv4", "ipv6", "uuid"],
    )
    def test_supported_formats_are_not_reported(self, fmt: str) -> None:
        """Regression: the previous detector rejected date-time/date/time/email/uuid.

        All nine are listed as supported by OpenAI structured outputs, so a
        ``datetime`` field must not sink an ``auto_schema: strict`` binding.
        """
        assert (
            detect_openai_strict_incompatibilities(
                {"type": "object", "properties": {"v": {"type": "string", "format": fmt}}}
            )
            == []
        )

    @pytest.mark.parametrize("fmt", ["uri", "binary", "byte", "regex"])
    def test_unsupported_formats_are_reported(self, fmt: str) -> None:
        assert detect_openai_strict_incompatibilities(
            {"type": "object", "properties": {"v": {"type": "string", "format": fmt}}}
        ) == [f"$.v.format={fmt}"]

    def test_supported_numeric_and_array_constraints_are_not_reported(self) -> None:
        """Regression: minimum/maximum/multipleOf/minItems/maxItems/pattern are supported."""
        assert (
            detect_openai_strict_incompatibilities(
                {
                    "type": "object",
                    "properties": {
                        "n": {"type": "number", "minimum": 1, "maximum": 9, "multipleOf": 2},
                        "l": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": 3,
                            "items": {"type": "string", "pattern": "^a"},
                        },
                    },
                }
            )
            == []
        )

    def test_findings_are_sorted_and_deduplicated(self) -> None:
        assert detect_openai_strict_incompatibilities(
            {
                "type": "object",
                "properties": {
                    "zeta": {"type": "string", "minLength": 1},
                    "alpha": {"type": "string", "minLength": 1},
                },
            }
        ) == ["$.alpha.minLength", "$.zeta.minLength"]

    def test_non_mapping_schema_is_tolerated(self) -> None:
        assert detect_openai_strict_incompatibilities(True) == []  # type: ignore[arg-type]


class TestAssertOpenAiStrictCompatible:
    def test_compatible_schema_is_a_noop(self) -> None:
        assert_openai_strict_compatible({"type": "object", "properties": {}}, module_id="m")

    def test_raises_with_side_prefixed_features(self) -> None:
        with pytest.raises(BindingStrictSchemaIncompatibleError) as exc_info:
            assert_openai_strict_compatible(
                {"type": "object", "properties": {"s": {"type": "string", "minLength": 2}}},
                module_id="demo.mod",
                side="input",
                file_path="b.yaml",
            )

        err = exc_info.value
        assert err.code == "BINDING_STRICT_SCHEMA_INCOMPATIBLE"
        assert err.details["features_listed"] == ["input:$.s.minLength"]
        assert "binding 'demo.mod' uses auto_schema: strict" in str(err)
        assert "input:$.s.minLength" in str(err)
        assert "See protocol-spec §5.12.5" in str(err)


class TestBindingLoaderStrictEnforcement:
    """The ``auto_schema: strict`` binding path (protocol-spec §5.12.5)."""

    @staticmethod
    def _write(tmp_path: Path, target: str, auto_schema: str = "strict") -> str:
        f = tmp_path / "t.binding.yaml"
        f.write_text(
            f"bindings:\n  - module_id: strict.case\n" f"    target: {target}\n    auto_schema: {auto_schema}\n"
        )
        return str(f)

    def test_incompatible_inferred_schema_raises(self, tmp_path: Path) -> None:
        # `strict_incompatible_function` takes a `set[str]`, which Pydantic
        # renders with `uniqueItems: true` — not accepted by OpenAI.
        path = self._write(tmp_path, "binding_helpers:strict_incompatible_function")

        with pytest.raises(BindingStrictSchemaIncompatibleError) as exc_info:
            BindingLoader().load_bindings(path, Registry())

        assert exc_info.value.details["features_listed"] == ["input:$.tags.uniqueItems"]

    def test_optional_and_datetime_fields_pass_strict(self, tmp_path: Path) -> None:
        """Regression for the old detector's false positives.

        ``str | None`` renders as a nested ``anyOf`` and ``datetime`` as
        ``format: date-time``; OpenAI accepts both, so this binding must load.
        """
        path = self._write(tmp_path, "binding_helpers:strict_compatible_function")

        modules = BindingLoader().load_bindings(path, Registry())

        assert len(modules) == 1

    def test_permissive_mode_does_not_enforce(self, tmp_path: Path) -> None:
        path = self._write(tmp_path, "binding_helpers:strict_incompatible_function", auto_schema="permissive")

        assert len(BindingLoader().load_bindings(path, Registry())) == 1

    def test_implicit_auto_schema_does_not_enforce(self, tmp_path: Path) -> None:
        f = tmp_path / "t.binding.yaml"
        f.write_text(
            "bindings:\n  - module_id: strict.implicit\n" "    target: binding_helpers:strict_incompatible_function\n"
        )

        assert len(BindingLoader().load_bindings(str(f), Registry())) == 1
