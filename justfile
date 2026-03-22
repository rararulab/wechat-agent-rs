# wechat-agent-rs

set dotenv-load := true

# Default: list available recipes
[private]
help:
    @just --list --unsorted

# ─── Environment ──────────────────────────────────────────────────────

# Print environment info
env:
    @echo "rustc: $(rustc --version)"
    @echo "cargo: $(cargo --version)"
    @echo "just:  $(just --version)"

# ─── Format ───────────────────────────────────────────────────────────

# Format all code
fmt:
    cargo +nightly fmt --all

# Check formatting (CI)
fmt-check:
    cargo +nightly fmt --all --check

# ─── Lint ─────────────────────────────────────────────────────────────

# Run clippy
clippy:
    cargo clippy --all-targets --all-features -- -D warnings

# Run all checks (compile)
check:
    cargo check --all-targets --all-features

# Run all lints (clippy + doc + deny)
lint: clippy
    cargo doc --no-deps --document-private-items 2>&1 | (! grep -E "^warning:" || (echo "Doc warnings found" && exit 1))
    cargo deny check

# ─── Test ─────────────────────────────────────────────────────────────

# Run tests
test *ARGS:
    cargo nextest run {{ ARGS }}

# ─── Pre-commit ──────────────────────────────────────────────────────

# Run all pre-commit checks
pre-commit: fmt-check lint test

# Install git hooks via prek
setup-hooks:
    prek install

# ─── Changelog & Release ─────────────────────────────────────────────

# Generate changelog (unreleased)
changelog:
    git cliff --unreleased --strip header

# Generate full changelog
changelog-all:
    git cliff

# Show what the next tag would be (based on conventional commits)
release-info:
    @echo "Latest tag: $(git describe --tags --abbrev=0 2>/dev/null || echo 'none')"
    @echo "Commits since tag: $(git rev-list $(git describe --tags --abbrev=0 2>/dev/null || git rev-list --max-parents=0 HEAD)..HEAD --count)"

# ─── Misc ─────────────────────────────────────────────────────────────

# Count lines of code
cloc:
    tokei .

# Clean build artifacts
clean:
    cargo clean
