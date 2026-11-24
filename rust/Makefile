.PHONY: setup check check-ci check-chars fmt-check lint lint-full build test build-examples fmt clean

# One-time dev environment setup
setup:
	@echo "Installing apdev-rs..."
	@command -v apdev-rs >/dev/null 2>&1 || cargo install apdev-rs
	@echo "Installing git pre-commit hook..."
	@mkdir -p .git/hooks
	@cp .githooks/pre-commit .git/hooks/pre-commit
	@chmod +x .git/hooks/pre-commit
	@echo "Done! Development environment is ready."

# Fast local check (pre-commit). Lints lib+bins only — test code is still
# COMPILED (and any error caught) by `test`, just not clippy-linted here. The
# full clippy --all-targets pass lives in `check-ci` / `lint-full`.
check: fmt-check lint check-chars build test build-examples

# Full check mirroring CI: clippy over all targets incl. tests + examples.
check-ci: fmt-check lint-full check-chars build test build-examples

check-chars:
	apdev-rs check-chars src/

fmt-check:
	cargo fmt --all -- --check

# Fast lint: lib + bins only. Skips re-type-checking all 122 test binaries
# (which `test` compiles anyway) — the single biggest make-check speedup.
lint:
	cargo clippy --lib --bins --all-features -- -D warnings

# Full lint: every target (lib, bins, tests, examples). Slower; use in CI.
lint-full:
	cargo clippy --all-targets --all-features -- -D warnings

build:
	cargo build --all-features

# Most integration tests compile into ONE binary (tests/it.rs, autotests=false)
# for fast builds; the few files that touch process-global Config/env state stay
# as separate binaries (see Cargo.toml [[test]] entries) so `cargo test` keeps
# per-process isolation for them.
test:
	cargo test --all-features

build-examples:
	cargo build --examples

fmt:
	cargo fmt --all

clean:
	cargo clean
