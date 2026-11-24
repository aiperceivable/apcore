"""Tests for normalize_to_canonical_id (Algorithm A02)."""

from __future__ import annotations

import pytest

from apcore.utils.normalize import normalize_to_canonical_id


class TestNormalizeToCanonicalId:
    """Cross-language ID normalization per PROTOCOL_SPEC §2.2."""

    # --- Python ---
    def test_python_already_canonical(self) -> None:
        assert normalize_to_canonical_id("api.handler", "python") == "api.handler"

    def test_python_pascal_case(self) -> None:
        assert normalize_to_canonical_id("Api.HttpHandler", "python") == "api.http_handler"

    def test_python_camel_case(self) -> None:
        assert normalize_to_canonical_id("api.httpHandler", "python") == "api.http_handler"

    # --- Rust ---
    def test_rust_double_colon(self) -> None:
        assert normalize_to_canonical_id("executor::validator::db_params", "rust") == "executor.validator.db_params"

    def test_rust_pascal_case(self) -> None:
        assert normalize_to_canonical_id("executor::DbValidator", "rust") == "executor.db_validator"

    # --- Go ---
    def test_go_pascal_case(self) -> None:
        assert normalize_to_canonical_id("Api.HttpJsonParser", "go") == "api.http_json_parser"

    # --- Java ---
    def test_java_package_style(self) -> None:
        assert normalize_to_canonical_id("com.example.HttpService", "java") == "com.example.http_service"

    # --- TypeScript ---
    def test_typescript_camel_case(self) -> None:
        assert normalize_to_canonical_id("api.httpHandler", "typescript") == "api.http_handler"

    # --- Acronym handling ---
    def test_acronym_http_json_parser(self) -> None:
        assert normalize_to_canonical_id("HttpJsonParser", "python") == "http_json_parser"

    def test_acronym_html_parser(self) -> None:
        assert normalize_to_canonical_id("HTMLParser", "python") == "html_parser"

    def test_acronym_get_db_url(self) -> None:
        assert normalize_to_canonical_id("getDBUrl", "python") == "get_db_url"

    def test_already_snake_case(self) -> None:
        assert normalize_to_canonical_id("my_module.handler", "python") == "my_module.handler"

    # --- Edge cases ---
    def test_single_segment(self) -> None:
        assert normalize_to_canonical_id("handler", "python") == "handler"

    def test_single_segment_pascal(self) -> None:
        assert normalize_to_canonical_id("Handler", "python") == "handler"

    # --- Error cases ---
    def test_unsupported_language(self) -> None:
        with pytest.raises(ValueError, match="Unsupported language"):
            normalize_to_canonical_id("foo.bar", "csharp")

    def test_invalid_result_raises(self) -> None:
        with pytest.raises(ValueError, match="does not conform"):
            normalize_to_canonical_id("123Invalid", "python")

    # --- Roundtrip: already-canonical IDs are idempotent ---
    @pytest.mark.parametrize("lang", ["python", "go", "java", "typescript"])
    def test_idempotent_for_canonical_input(self, lang: str) -> None:
        canonical = "api.handler.task_submit"
        assert normalize_to_canonical_id(canonical, lang) == canonical
