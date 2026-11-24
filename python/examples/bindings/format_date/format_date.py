"""Target function for the format_date YAML binding."""

from datetime import datetime


def format_date_string(date_string: str, output_format: str) -> dict:
    """Parse a date string and reformat it.

    Args:
        date_string: Input date string (ISO 8601 format, e.g. "2024-01-15").
        output_format: strftime format string (e.g. "%B %d, %Y").

    Returns:
        Dict with "formatted" key containing the reformatted date.
    """
    dt = datetime.fromisoformat(date_string)
    return {"formatted": dt.strftime(output_format)}
