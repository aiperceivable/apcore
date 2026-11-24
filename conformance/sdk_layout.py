"""Resolve SDK checkouts from the monorepo or an older sibling layout."""

from pathlib import Path


def sdk_dir(root: Path, legacy_name: str) -> Path:
    """Find a language SDK under the given root."""
    member = root / legacy_name.removeprefix("apcore-")
    if member.is_dir():
        return member
    return root / legacy_name
