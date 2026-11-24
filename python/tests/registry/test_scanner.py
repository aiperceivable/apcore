"""Tests for the directory scanner: scan_extensions() and scan_multi_root()."""

from __future__ import annotations

import os
from pathlib import Path
from unittest.mock import patch

import pytest

from apcore.errors import ConfigError, ConfigNotFoundError
from apcore.registry.scanner import scan_extensions, scan_multi_root


# === scan_extensions() basic scanning ===


class TestScanExtensionsBasic:
    def test_empty_directory(self, tmp_path: Path) -> None:
        """Empty directory returns empty list."""
        assert scan_extensions(tmp_path) == []

    def test_single_py_file(self, tmp_path: Path) -> None:
        """Single .py file returns one DiscoveredModule with correct fields."""
        (tmp_path / "hello.py").write_text("")
        result = scan_extensions(tmp_path)
        assert len(result) == 1
        assert result[0].canonical_id == "hello"
        assert result[0].file_path == tmp_path / "hello.py"
        assert result[0].meta_path is None
        assert result[0].namespace is None

    def test_nested_directories(self, tmp_path: Path) -> None:
        """Nested directories generate dot-separated canonical_ids."""
        (tmp_path / "sub").mkdir()
        (tmp_path / "sub" / "module.py").write_text("")
        result = scan_extensions(tmp_path)
        assert len(result) == 1
        assert result[0].canonical_id == "sub.module"

    def test_multiple_files_at_different_depths(self, tmp_path: Path) -> None:
        """Multiple .py files at different depths all discovered."""
        (tmp_path / "a.py").write_text("")
        (tmp_path / "b").mkdir()
        (tmp_path / "b" / "c.py").write_text("")
        (tmp_path / "b" / "d").mkdir()
        (tmp_path / "b" / "d" / "e.py").write_text("")
        result = scan_extensions(tmp_path)
        ids = {m.canonical_id for m in result}
        assert ids == {"a", "b.c", "b.d.e"}


# === scan_extensions() ignore patterns ===


class TestScanExtensionsIgnore:
    def test_underscore_prefix_skipped(self, tmp_path: Path) -> None:
        """Files starting with _ are skipped."""
        (tmp_path / "_private.py").write_text("")
        assert scan_extensions(tmp_path) == []

    def test_dot_prefix_skipped(self, tmp_path: Path) -> None:
        """Files starting with . are skipped."""
        (tmp_path / ".hidden.py").write_text("")
        assert scan_extensions(tmp_path) == []

    def test_pycache_skipped(self, tmp_path: Path) -> None:
        """__pycache__ directories are skipped entirely."""
        (tmp_path / "__pycache__").mkdir()
        (tmp_path / "__pycache__" / "cached.py").write_text("")
        assert scan_extensions(tmp_path) == []

    def test_node_modules_skipped(self, tmp_path: Path) -> None:
        """node_modules directories are skipped."""
        (tmp_path / "node_modules").mkdir()
        (tmp_path / "node_modules" / "something.py").write_text("")
        assert scan_extensions(tmp_path) == []

    def test_pyc_files_skipped(self, tmp_path: Path) -> None:
        """.pyc files are skipped."""
        (tmp_path / "compiled.pyc").write_text("")
        assert scan_extensions(tmp_path) == []

    def test_init_py_skipped(self, tmp_path: Path) -> None:
        """__init__.py files are skipped (starts with _)."""
        (tmp_path / "__init__.py").write_text("")
        assert scan_extensions(tmp_path) == []


# === scan_extensions() depth and errors ===


class TestScanExtensionsDepthAndErrors:
    def test_max_depth_1(self, tmp_path: Path) -> None:
        """max_depth=1 only scans root level."""
        (tmp_path / "root_mod.py").write_text("")
        (tmp_path / "sub").mkdir()
        (tmp_path / "sub" / "deep_mod.py").write_text("")
        result = scan_extensions(tmp_path, max_depth=1)
        ids = {m.canonical_id for m in result}
        assert ids == {"root_mod"}

    def test_depth_exceeded_skipped(self, tmp_path: Path) -> None:
        """Directories beyond max_depth are skipped."""
        (tmp_path / "a").mkdir()
        (tmp_path / "a" / "b").mkdir()
        (tmp_path / "a" / "b" / "c").mkdir()
        (tmp_path / "a" / "b" / "c" / "d.py").write_text("")
        result = scan_extensions(tmp_path, max_depth=2)
        ids = {m.canonical_id for m in result}
        assert "a.b.c.d" not in ids

    def test_nonexistent_root_raises(self, tmp_path: Path) -> None:
        """Non-existent root raises ConfigNotFoundError."""
        with pytest.raises(ConfigNotFoundError):
            scan_extensions(tmp_path / "nonexistent")

    def test_permission_error_continues(self, tmp_path: Path) -> None:
        """PermissionError on subdirectory is caught, scanning continues."""
        (tmp_path / "good.py").write_text("")
        (tmp_path / "forbidden").mkdir()
        (tmp_path / "forbidden" / "secret.py").write_text("")

        original_scandir = os.scandir

        def mock_scandir(path):
            if str(path).endswith("forbidden"):
                raise PermissionError("Access denied")
            return original_scandir(path)

        with patch("os.scandir", side_effect=mock_scandir):
            result = scan_extensions(tmp_path)

        ids = {m.canonical_id for m in result}
        assert "good" in ids


# === scan_extensions() metadata companion files ===


class TestScanExtensionsMeta:
    def test_meta_yaml_detected(self, tmp_path: Path) -> None:
        """file.py with file_meta.yaml sets meta_path."""
        (tmp_path / "task.py").write_text("")
        (tmp_path / "task_meta.yaml").write_text("")
        result = scan_extensions(tmp_path)
        assert result[0].meta_path == tmp_path / "task_meta.yaml"

    def test_no_meta_yaml(self, tmp_path: Path) -> None:
        """file.py without companion meta has meta_path=None."""
        (tmp_path / "task.py").write_text("")
        result = scan_extensions(tmp_path)
        assert result[0].meta_path is None


# === scan_extensions() symlinks ===


class TestScanExtensionsSymlinks:
    def test_follow_symlinks_false_skips(self, tmp_path: Path) -> None:
        """follow_symlinks=False skips symlinked directories."""
        real_dir = tmp_path / "real"
        real_dir.mkdir()
        (real_dir / "mod.py").write_text("")
        link_dir = tmp_path / "link"
        link_dir.symlink_to(real_dir)
        result = scan_extensions(tmp_path, follow_symlinks=False)
        ids = {m.canonical_id for m in result}
        assert "real.mod" in ids
        assert "link.mod" not in ids

    def test_follow_symlinks_true_records_the_target_once(self, tmp_path: Path) -> None:
        """follow_symlinks=True follows an aliased directory WITHOUT duplicating it.

        This asserted ``"link.mod" in ids`` until spec v1.56.0 — it was pinning a
        duplicate. One directory reached by two paths produced two modules with
        different IDs, and step 4's ``detect_id_conflicts`` cannot catch that
        precisely because the IDs differ, so the same module was registered and
        executed under two names. D-127 keys visited-directory tracking on the
        canonical real path, which both removes the duplicate and terminates a
        cycle.
        """
        real_dir = tmp_path / "real"
        real_dir.mkdir()
        (real_dir / "mod.py").write_text("")
        link_dir = tmp_path / "link"
        link_dir.symlink_to(real_dir)
        result = scan_extensions(tmp_path, follow_symlinks=True)
        ids = {m.canonical_id for m in result}
        assert ids == {"real.mod"}, f"the target is recorded once, under its real path; got {ids}"

    def test_symlink_cycle_detected(self, tmp_path: Path) -> None:
        """Symlink cycle detected and skipped without infinite recursion."""
        a_dir = tmp_path / "a"
        a_dir.mkdir()
        (a_dir / "mod.py").write_text("")
        (a_dir / "loop").symlink_to(a_dir)
        result = scan_extensions(tmp_path, follow_symlinks=True)
        ids = {m.canonical_id for m in result}
        assert "a.mod" in ids

    def test_symlink_escaping_root_refused(self, tmp_path: Path) -> None:
        """Regression: follow_symlinks=True must not traverse symlinks escaping root.

        Construct an extension root with a symlink to a sibling directory.
        Prior to the fix, scanner would walk the escape target; now it
        logs and refuses.
        """
        root = tmp_path / "ext_root"
        root.mkdir()
        (root / "ours.py").write_text("")
        # Outside the root
        outside = tmp_path / "sibling"
        outside.mkdir()
        (outside / "escape.py").write_text("")
        # Symlink into outside
        (root / "escape_link").symlink_to(outside)

        result = scan_extensions(root, follow_symlinks=True)
        ids = {m.canonical_id for m in result}
        assert "ours" in ids
        assert not any("escape" in i for i in ids), "scanner walked into symlink escaping extension root"

    def test_symlinked_file_escaping_root_refused(self, tmp_path: Path) -> None:
        """A symlinked FILE whose target escapes the root must not be discovered.

        The directory case above was guarded; this one was not. The confinement
        check lived inside the ``is_dir`` branch, so a symlink to a .py file
        outside the root reached ``elif is_file`` unchecked, was discovered, and
        was then imported and executed by the registry -- code execution from
        outside the declared trust boundary, which is precisely what the check's
        own comment says it exists to prevent.
        """
        root = tmp_path / "ext_root"
        root.mkdir()
        (root / "ours.py").write_text("")
        outside = tmp_path / "outside"
        outside.mkdir()
        (outside / "evil.py").write_text("")

        (root / "linked_evil.py").symlink_to(outside / "evil.py")

        result = scan_extensions(root, follow_symlinks=True)
        ids = {m.canonical_id for m in result}
        assert "ours" in ids
        assert (
            "linked_evil" not in ids
        ), "scanner discovered a symlinked file whose target is outside the extension root"

    def test_symlinked_file_inside_root_still_followed(self, tmp_path: Path) -> None:
        """Confinement must not cost the feature: an in-root symlinked file still resolves.

        Guards this fix against over-correction -- refusing every symlinked file
        would silence the escape and break deliberate aliasing at the same time.
        """
        root = tmp_path / "ext_root"
        root.mkdir()
        sub = root / "sub"
        sub.mkdir()
        (sub / "real.py").write_text("")

        (root / "alias.py").symlink_to(sub / "real.py")

        result = scan_extensions(root, follow_symlinks=True)
        ids = {m.canonical_id for m in result}
        # D-127: the target is recorded ONCE, under the ID derived from its
        # canonical real path. This asserted `"alias" in ids` until spec
        # v1.56.0, which pinned one file becoming two modules — and made the
        # registered ID depend on which path the traversal reached first, i.e.
        # on directory iteration order.
        assert ids == {"sub.real"}, f"the alias resolves to the target, it is not a second module; got {ids}"
        assert all(
            m.file_path.name == "real.py" for m in result
        ), "the loader must open the target, not the alias — the alias is a name, not the module"

    def test_symlinked_file_not_followed_when_disabled(self, tmp_path: Path) -> None:
        """follow_symlinks=False skips symlinked files, as it does directories."""
        root = tmp_path / "ext_root"
        root.mkdir()
        sub = root / "sub"
        sub.mkdir()
        (sub / "real.py").write_text("")
        (root / "alias.py").symlink_to(sub / "real.py")

        result = scan_extensions(root, follow_symlinks=False)
        ids = {m.canonical_id for m in result}
        assert "sub.real" in ids
        assert "alias" not in ids


# === scan_multi_root() ===


class TestScanMultiRoot:
    def test_two_roots_with_namespaces(self, tmp_path: Path) -> None:
        """Two roots with different namespaces prefix IDs correctly."""
        root_a = tmp_path / "root_a"
        root_b = tmp_path / "root_b"
        root_a.mkdir()
        root_b.mkdir()
        (root_a / "mod.py").write_text("")
        (root_b / "mod.py").write_text("")
        result = scan_multi_root(
            [
                {"root": str(root_a), "namespace": "ns_a"},
                {"root": str(root_b), "namespace": "ns_b"},
            ]
        )
        ids = {m.canonical_id for m in result}
        assert ids == {"ns_a.mod", "ns_b.mod"}

    def test_auto_derive_namespace(self, tmp_path: Path) -> None:
        """Auto-derive namespace from directory name."""
        ext_dir = tmp_path / "my_extensions"
        ext_dir.mkdir()
        (ext_dir / "mod.py").write_text("")
        result = scan_multi_root([{"root": str(ext_dir)}])
        assert result[0].canonical_id == "my_extensions.mod"
        assert result[0].namespace == "my_extensions"

    def test_duplicate_namespaces_raises(self) -> None:
        """Duplicate namespaces raise ConfigError."""
        with pytest.raises(ConfigError):
            scan_multi_root(
                [
                    {"root": "/tmp/a", "namespace": "same"},
                    {"root": "/tmp/b", "namespace": "same"},
                ]
            )

    def test_explicit_namespace_used(self, tmp_path: Path) -> None:
        """Explicit namespace in config dict is used."""
        ext_dir = tmp_path / "dir"
        ext_dir.mkdir()
        (ext_dir / "mod.py").write_text("")
        result = scan_multi_root([{"root": str(ext_dir), "namespace": "custom"}])
        assert result[0].canonical_id == "custom.mod"
        assert result[0].namespace == "custom"
