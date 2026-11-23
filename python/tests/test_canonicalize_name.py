"""Public bare-name canonicalization contract shared by all three SDKs."""

from __future__ import annotations

import json
from dataclasses import FrozenInstanceError, asdict
import pytest

from apcore import CanonicalNameError, CanonicalNameResult, canonicalize_name, normalize_to_canonical_id


@pytest.mark.parametrize(
    ("name", "canonical", "error"),
    [
        ("getHTTPResponse", "get_http_response", None),
        ("HTTPServer", "http_server", None),
        ("HTTP_SERVER", "http_server", None),
        ("user__profile", "user__profile", None),
        ("User___Profile", "user___profile", None),
        ("user-profile", "user_profile", None),
        ("  *** Hello, World!!! \t", "hello_world", None),
        ("a-- . / b", "a_b", None),
        ("a_--_b", "a___b", None),
        ("foo.bar::Baz", "foo_bar_baz", None),
        ("system", "system", None),
        ("external", "external", None),
        ("a\x00b", "a_b", None),
        ("\x00A\x7f", "a", None),
        ("", None, "empty_name"),
        (" \t\r\n", None, "empty_name"),
        ("---!", None, "empty_name"),
        ("_user", None, "invalid_start"),
        (".._user..", None, "invalid_start"),
        ("__", None, "invalid_start"),
        ("123Hello", None, "invalid_start"),
        ("Über", None, "non_ascii"),
        ("\u00a0hello", None, "non_ascii"),
        ("hello\u2003", None, "non_ascii"),
        ("😀", None, "non_ascii"),
        ("\ud800", None, "non_ascii"),
        ("a" * 192, "a" * 192, None),
        ("a" * 193, None, "name_too_long"),
        ("aA" * 96, None, "name_too_long"),
        ("1" + "a" * 193, None, "invalid_start"),
        ("!" * 200 + "Valid" + "!" * 200, "valid", None),
    ],
)
def test_canonicalize_name_preserves_original_and_reports_a_structured_result(
    name: str,
    canonical: str | None,
    error: str | None,
) -> None:
    """A bare name is either one valid segment or an explicit diagnostic."""
    result = canonicalize_name(name)
    assert isinstance(result, CanonicalNameResult)
    assert result.original_name == name
    assert result.canonical_name == canonical
    assert (result.error.value if result.error is not None else None) == error
    assert (result.error is None) is (result.canonical_name is not None)


def test_canonicalize_name_types_are_exported_from_root_and_utils() -> None:
    from apcore.utils import CanonicalNameError as UtilityError
    from apcore.utils import CanonicalNameResult as UtilityResult
    from apcore.utils import canonicalize_name as utility_function
    from apcore.utils.normalize import canonicalize_name as normalization_function

    assert UtilityError is CanonicalNameError
    assert UtilityResult is CanonicalNameResult
    assert utility_function is normalization_function is canonicalize_name
    assert {error.value for error in CanonicalNameError} == {
        "empty_name",
        "non_ascii",
        "invalid_start",
        "name_too_long",
    }


def test_canonicalize_name_result_is_frozen_and_serializable() -> None:
    result = canonicalize_name("_bad")
    with pytest.raises(FrozenInstanceError):
        setattr(result, "canonical_name", "bad")
    assert json.loads(json.dumps(asdict(result))) == {
        "original_name": "_bad",
        "canonical_name": None,
        "error": "invalid_start",
    }


@pytest.mark.parametrize("code", range(128))
def test_canonicalize_name_accepts_every_ascii_character_without_throwing(code: int) -> None:
    name = "before" + chr(code) + "after"
    result = canonicalize_name(name)
    assert result.original_name == name
    assert result.error is None
    assert result.canonical_name is not None
    assert all(char.isascii() and (char.isalnum() or char == "_") for char in result.canonical_name)
    assert canonicalize_name(result.canonical_name).canonical_name == result.canonical_name


def test_canonicalize_name_does_not_change_a02_module_id_normalization() -> None:
    assert canonicalize_name("my-module").canonical_name == "my_module"
    with pytest.raises(ValueError, match="does not conform"):
        normalize_to_canonical_id("my-module", "python")
    assert normalize_to_canonical_id("User__Profile", "python") == "user__profile"
