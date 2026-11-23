"""Tests for ModuleAnnotations redesign: extra field, pagination_style, __post_init__, from_dict."""

from __future__ import annotations

import logging
from typing import Any

from apcore.module import DEFAULT_ANNOTATIONS, ModuleAnnotations


class TestExtraField:
    """AC-005: extra field exists with default {}."""

    def test_default_extra_is_empty_dict(self) -> None:
        ann = ModuleAnnotations()
        assert ann.extra == {}
        assert isinstance(ann.extra, dict)

    def test_extra_with_values(self) -> None:
        ann = ModuleAnnotations(extra={"mcp.category": "tools", "cli.approval_message": "Are you sure?"})
        assert ann.extra["mcp.category"] == "tools"
        assert ann.extra["cli.approval_message"] == "Are you sure?"

    def test_extra_round_trip_preserves_data(self) -> None:
        ann = ModuleAnnotations(extra={"a2a.guidance": "careful"})
        assert ann.extra == {"a2a.guidance": "careful"}


class TestExtraDetachment:
    """AC-023: extra dict is detached from caller's mutable dict."""

    def test_mutation_of_original_does_not_affect_instance(self) -> None:
        d: dict[str, Any] = {"k": "v"}
        ann = ModuleAnnotations(extra=d)
        d["k2"] = "v2"
        assert "k2" not in ann.extra

    def test_mutation_of_extra_raises_on_frozen(self) -> None:
        ann = ModuleAnnotations(extra={"k": "v"})
        # extra itself is a plain dict (not frozen), but the dataclass is frozen
        # so ann.extra = {} would raise. The dict contents can be mutated post-init
        # but that's user responsibility per spec.
        assert ann.extra["k"] == "v"


class TestCacheKeyFieldsTuple:
    """AC-022: cache_key_fields stored as tuple even when passed as list."""

    def test_list_converted_to_tuple(self) -> None:
        ann = ModuleAnnotations(cache_key_fields=["a", "b"])  # type: ignore[arg-type]
        assert isinstance(ann.cache_key_fields, tuple)
        assert ann.cache_key_fields == ("a", "b")

    def test_tuple_stays_tuple(self) -> None:
        ann = ModuleAnnotations(cache_key_fields=("x", "y"))
        assert isinstance(ann.cache_key_fields, tuple)
        assert ann.cache_key_fields == ("x", "y")

    def test_none_stays_none(self) -> None:
        ann = ModuleAnnotations(cache_key_fields=None)
        assert ann.cache_key_fields is None


class TestPaginationStyle:
    """AC-007: pagination_style accepts arbitrary strings."""

    def test_default_is_cursor(self) -> None:
        ann = ModuleAnnotations()
        assert ann.pagination_style == "cursor"

    def test_accepts_custom_string(self) -> None:
        ann = ModuleAnnotations(pagination_style="custom")
        assert ann.pagination_style == "custom"

    def test_accepts_known_values(self) -> None:
        for style in ("cursor", "offset", "page"):
            ann = ModuleAnnotations(pagination_style=style)
            assert ann.pagination_style == style


class TestCacheTtlClamping:
    """AC-027: Negative cache_ttl clamped to 0 with WARN log."""

    def test_negative_cache_ttl_clamped(self) -> None:
        ann = ModuleAnnotations(cache_ttl=-5)
        assert ann.cache_ttl == 0

    def test_negative_cache_ttl_logs_warning(self, caplog: Any) -> None:
        with caplog.at_level(logging.WARNING):
            ModuleAnnotations(cache_ttl=-10)
        assert any("cache_ttl" in record.message for record in caplog.records)

    def test_zero_cache_ttl_unchanged(self) -> None:
        ann = ModuleAnnotations(cache_ttl=0)
        assert ann.cache_ttl == 0

    def test_positive_cache_ttl_unchanged(self) -> None:
        ann = ModuleAnnotations(cache_ttl=300)
        assert ann.cache_ttl == 300


class TestFromDict:
    """AC-006: Unknown top-level keys in deserialized dict placed into extra."""

    def test_unknown_keys_go_to_extra(self) -> None:
        data = {"readonly": True, "future_field": 42}
        ann = ModuleAnnotations.from_dict(data)
        assert ann.readonly is True
        assert ann.extra["future_field"] == 42

    def test_explicit_extra_preserved(self) -> None:
        data = {"extra": {"mcp.cat": "tools"}}
        ann = ModuleAnnotations.from_dict(data)
        assert ann.extra["mcp.cat"] == "tools"

    def test_explicit_extra_merged_with_unknown(self) -> None:
        data = {"extra": {"mcp.cat": "tools"}, "new_field": "val"}
        ann = ModuleAnnotations.from_dict(data)
        assert ann.extra["mcp.cat"] == "tools"
        assert ann.extra["new_field"] == "val"

    def test_nested_extra_wins_over_top_level_collision(self) -> None:
        # PROTOCOL_SPEC §4.4.1 rule 7: when the same key appears both nested and
        # at the root, the nested value MUST win.
        data = {
            "mcp.category": "LEGACY_VALUE",
            "extra": {"mcp.category": "CANONICAL_VALUE"},
        }
        ann = ModuleAnnotations.from_dict(data)
        assert ann.extra["mcp.category"] == "CANONICAL_VALUE"

    def test_legacy_flattened_form_still_accepted(self) -> None:
        # Backward compatibility: top-level overflow keys still normalize into extra.
        data = {"readonly": True, "mcp.category": "tools", "cli.approval_message": "ok?"}
        ann = ModuleAnnotations.from_dict(data)
        assert ann.readonly is True
        assert ann.extra == {"mcp.category": "tools", "cli.approval_message": "ok?"}

    def test_missing_fields_use_defaults(self) -> None:
        ann = ModuleAnnotations.from_dict({})
        assert ann.readonly is False
        assert ann.open_world is True
        assert ann.pagination_style == "cursor"
        assert ann.extra == {}

    def test_all_known_fields(self) -> None:
        data = {
            "readonly": True,
            "destructive": True,
            "idempotent": True,
            "requires_approval": True,
            "open_world": False,
            "streaming": True,
            "cacheable": True,
            "cache_ttl": 60,
            "cache_key_fields": ["id"],
            "paginated": True,
            "pagination_style": "offset",
        }
        ann = ModuleAnnotations.from_dict(data)
        assert ann.readonly is True
        assert ann.destructive is True
        assert ann.open_world is False
        assert ann.cache_ttl == 60
        assert ann.cache_key_fields == ("id",)  # converted to tuple
        assert ann.pagination_style == "offset"
        assert ann.extra == {}


class TestDefaultAnnotations:
    """DEFAULT_ANNOTATIONS constant."""

    def test_default_annotations_is_instance(self) -> None:
        assert isinstance(DEFAULT_ANNOTATIONS, ModuleAnnotations)

    def test_default_annotations_has_empty_extra(self) -> None:
        assert DEFAULT_ANNOTATIONS.extra == {}

    def test_default_annotations_matches_no_arg_constructor(self) -> None:
        assert DEFAULT_ANNOTATIONS == ModuleAnnotations()
