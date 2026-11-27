"""Applications own contextual logging."""

from apcore.context import Context


def test_context_has_no_deprecated_logger_property() -> None:
    assert not hasattr(Context.create(), "logger")
