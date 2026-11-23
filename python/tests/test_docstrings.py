"""Tests for apcore._docstrings module."""

from __future__ import annotations

from apcore._docstrings import parse_docstring


class TestParseDocstringGoogle:
    """Google-style docstring parsing."""

    def test_google_basic(self) -> None:
        def fn(a: int, b: str) -> None:
            """Do something.

            Args:
                a: First arg.
                b: Second arg.
            """

        desc, doc, params = parse_docstring(fn)
        assert desc == "Do something."
        assert params == {"a": "First arg.", "b": "Second arg."}

    def test_google_with_type_annotations(self) -> None:
        def fn(x: int) -> None:
            """Summary.

            Args:
                x (int): The value.
            """

        _, _, params = parse_docstring(fn)
        assert params == {"x": "The value."}

    def test_google_multiline_description(self) -> None:
        def fn(name: str) -> None:
            """Summary.

            Args:
                name: A really long
                    description that spans
                    multiple lines.
            """

        _, _, params = parse_docstring(fn)
        assert params["name"] == "A really long description that spans multiple lines."

    def test_google_arguments_keyword(self) -> None:
        def fn(x: int) -> None:
            """Summary.

            Arguments:
                x: The x value.
            """

        _, _, params = parse_docstring(fn)
        assert params == {"x": "The x value."}

    def test_google_parameters_keyword(self) -> None:
        def fn(x: int) -> None:
            """Summary.

            Parameters:
                x: The x value.
            """

        _, _, params = parse_docstring(fn)
        assert params == {"x": "The x value."}

    def test_google_stops_at_returns_section(self) -> None:
        def fn(a: int) -> None:
            """Summary.

            Args:
                a: Input value.

            Returns:
                The output.
            """

        _, _, params = parse_docstring(fn)
        assert params == {"a": "Input value."}


class TestParseDocstringNumpy:
    """NumPy-style docstring parsing."""

    def test_numpy_basic(self) -> None:
        def fn(x: int, y: float) -> None:
            """Summary line.

            Parameters
            ----------
            x : int
                The x value.
            y : float
                The y value.
            """

        desc, doc, params = parse_docstring(fn)
        assert desc == "Summary line."
        assert params == {"x": "The x value.", "y": "The y value."}

    def test_numpy_multiline(self) -> None:
        def fn(data: list) -> None:
            """Summary.

            Parameters
            ----------
            data : list
                A list of items
                to process.
            """

        _, _, params = parse_docstring(fn)
        assert params["data"] == "A list of items to process."


class TestParseDocstringSphinx:
    """Sphinx-style docstring parsing."""

    def test_sphinx_basic(self) -> None:
        def fn(a: int, b: str) -> None:
            """Summary.

            :param a: First arg.
            :param b: Second arg.
            """

        desc, doc, params = parse_docstring(fn)
        assert desc == "Summary."
        assert params == {"a": "First arg.", "b": "Second arg."}

    def test_sphinx_with_type(self) -> None:
        def fn(x: int) -> None:
            """Summary.

            :param int x: The value.
            """

        _, _, params = parse_docstring(fn)
        assert params == {"x": "The value."}


class TestParseDocstringDocumentation:
    """Test documentation (body text) extraction."""

    def test_body_text_extracted(self) -> None:
        def fn() -> None:
            """Summary.

            This is the body documentation
            that spans multiple lines.

            Args:
                (none)
            """

        _, doc, _ = parse_docstring(fn)
        assert doc is not None
        assert "body documentation" in doc

    def test_no_body_text(self) -> None:
        def fn() -> None:
            """Summary only."""

        _, doc, _ = parse_docstring(fn)
        assert doc is None

    def test_body_stops_at_section_header(self) -> None:
        def fn() -> None:
            """Summary.

            Body text here.

            Args:
                x: Something.
            """

        _, doc, _ = parse_docstring(fn)
        assert doc == "Body text here."


class TestParseDocstringEdgeCases:
    """Edge cases."""

    def test_no_docstring(self) -> None:
        def fn() -> None:
            pass

        desc, doc, params = parse_docstring(fn)
        assert desc is None
        assert doc is None
        assert params == {}

    def test_empty_docstring(self) -> None:
        def fn() -> None:
            """"""

        desc, doc, params = parse_docstring(fn)
        assert desc is None
        assert doc is None
        assert params == {}

    def test_summary_only_no_params(self) -> None:
        def fn() -> None:
            """Just a summary."""

        desc, doc, params = parse_docstring(fn)
        assert desc == "Just a summary."
        assert doc is None
        assert params == {}

    def test_no_args_section(self) -> None:
        def fn(x: int) -> None:
            """Summary.

            Returns:
                Something.
            """

        _, _, params = parse_docstring(fn)
        assert params == {}
