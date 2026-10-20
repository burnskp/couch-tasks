set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

# List available tasks.
default:
    @just --list

# Format Rust sources.
fmt:
    cargo fmt --all

# Check Rust formatting without changing files.
fmt-check:
    cargo fmt --all -- --check

# Run clippy with warnings denied.
lint:
    cargo clippy --all-targets --all-features -- -D warnings

# Run the test suite.
test:
    cargo test --all-targets --all-features

# Syntax-check zsh completion, if zsh is available.
completion-check:
    if command -v zsh >/dev/null; then zsh -n completions/_ct; else echo "zsh not found; skipping completion syntax check"; fi

# Run the full local validation suite.
check: fmt-check lint test completion-check

# Remove build artifacts.
clean:
    cargo clean

# Audit locked dependencies, including informational security warnings.
audit:
    cargo audit --deny warnings
