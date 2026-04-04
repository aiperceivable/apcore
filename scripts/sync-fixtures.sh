#!/usr/bin/env bash
# sync-fixtures.sh — Push canonical conformance fixtures to SDK repos.
#
# Lives in the apcore spec repo. Copies conformance/fixtures/*.json into
# each target SDK repo so that their CI can run without the spec repo.
#
# Usage:
#   ./scripts/sync-fixtures.sh                     # sync all sibling SDK repos
#   ./scripts/sync-fixtures.sh ../apcore-python     # sync one specific repo
#   ./scripts/sync-fixtures.sh --check              # verify all siblings are up to date
#
# Target layout per language:
#   Python      → tests/conformance/fixtures/
#   TypeScript  → tests/conformance/fixtures/
#   Rust        → tests/conformance/fixtures/
#
# SDK repos are discovered as siblings: ../apcore-{python,typescript,rust}

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SPEC_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SOURCE="$SPEC_ROOT/conformance/fixtures"

if [ ! -d "$SOURCE" ]; then
    echo "Error: $SOURCE not found. Run from the apcore spec repo root." >&2
    exit 1
fi

# All SDK languages use the same relative path for vendored fixtures.
FIXTURE_REL="tests/conformance/fixtures"

CHECK_MODE=false
if [ "${1:-}" = "--check" ]; then
    CHECK_MODE=true
    shift
fi

# --- Discover target SDK repos ---
discover_sdks() {
    local targets=()
    for lang in python typescript rust; do
        local candidate="$SPEC_ROOT/../apcore-$lang"
        if [ -d "$candidate" ]; then
            targets+=("$(cd "$candidate" && pwd)")
        fi
    done
    echo "${targets[@]}"
}

# --- Sync one repo ---
sync_one() {
    local target_repo="$1"
    local repo_name
    repo_name="$(basename "$target_repo")"
    local dest="$target_repo/$FIXTURE_REL"

    mkdir -p "$dest"
    cp "$SOURCE"/*.json "$dest/"

    # Check for changes
    if command -v git &>/dev/null && git -C "$target_repo" rev-parse --git-dir &>/dev/null 2>&1; then
        local diff
        diff="$(git -C "$target_repo" diff --stat -- "$FIXTURE_REL/" 2>/dev/null || true)"
        local untracked
        untracked="$(git -C "$target_repo" ls-files --others --exclude-standard -- "$FIXTURE_REL/" 2>/dev/null || true)"

        if [ -n "$diff" ] || [ -n "$untracked" ]; then
            echo "  $repo_name: updated"
            [ -n "$diff" ] && echo "$diff" | sed 's/^/    /'
            [ -n "$untracked" ] && echo "$untracked" | sed 's/^/    [new] /'
            return 1  # has changes
        else
            echo "  $repo_name: up to date"
            return 0
        fi
    else
        echo "  $repo_name: synced (not a git repo, cannot diff)"
        return 0
    fi
}

# --- Check mode: verify without modifying ---
check_one() {
    local target_repo="$1"
    local repo_name
    repo_name="$(basename "$target_repo")"
    local dest="$target_repo/$FIXTURE_REL"

    if [ ! -d "$dest" ]; then
        echo "  $repo_name: MISSING (no $FIXTURE_REL/ directory)"
        return 1
    fi

    # Compare each fixture file
    local stale=false
    for src_file in "$SOURCE"/*.json; do
        local fname
        fname="$(basename "$src_file")"
        local dest_file="$dest/$fname"

        if [ ! -f "$dest_file" ]; then
            echo "  $repo_name: MISSING $fname"
            stale=true
        elif ! diff -q "$src_file" "$dest_file" &>/dev/null; then
            echo "  $repo_name: STALE  $fname"
            stale=true
        fi
    done

    if [ "$stale" = true ]; then
        return 1
    else
        echo "  $repo_name: up to date"
        return 0
    fi
}

# --- Main ---
echo "Source: $SOURCE"
echo ""

# Determine targets
if [ $# -gt 0 ]; then
    TARGETS=("$@")
else
    read -ra TARGETS <<< "$(discover_sdks)"
fi

if [ ${#TARGETS[@]} -eq 0 ]; then
    echo "No SDK repos found as siblings of $SPEC_ROOT" >&2
    echo "Usage: $0 [path/to/sdk-repo ...]" >&2
    exit 1
fi

HAS_CHANGES=false

for target in "${TARGETS[@]}"; do
    if [ ! -d "$target" ]; then
        echo "  Warning: $target does not exist, skipping" >&2
        continue
    fi

    if [ "$CHECK_MODE" = true ]; then
        check_one "$target" || HAS_CHANGES=true
    else
        sync_one "$target" || HAS_CHANGES=true
    fi
done

echo ""
if [ "$CHECK_MODE" = true ]; then
    if [ "$HAS_CHANGES" = true ]; then
        echo "Some SDK repos have stale fixtures. Run: $0"
        exit 1
    else
        echo "All SDK repos are up to date."
    fi
else
    if [ "$HAS_CHANGES" = true ]; then
        echo "Done. Review changes and commit in each SDK repo."
    else
        echo "All SDK repos already up to date."
    fi
fi
