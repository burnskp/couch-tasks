# AGENTS.md

Guidance for coding agents working in this repository.

## Formatting

Format Rust sources with:

```sh
just fmt
```

Equivalent Cargo command:

```sh
cargo fmt --all
```

Check formatting without modifying files:

```sh
just fmt-check
```

## Linting

Run Clippy with warnings denied:

```sh
just lint
```

Equivalent Cargo command:

```sh
cargo clippy --all-targets --all-features -- -D warnings
```

## Tests

Run the full Rust test suite:

```sh
just test
```

Equivalent Cargo command:

```sh
cargo test --all-targets --all-features
```

## Full local validation

Before handing off changes, prefer running:

```sh
just check
```

This runs formatting checks, Clippy, tests, and zsh completion syntax checks when `zsh` is available.

## Compatibility and legacy policy

When modifying code, keep the implementation focused on the current desired behavior. If a task removes or replaces a feature, do not add migration support, compatibility shims, fallback paths, or legacy code for the removed behavior unless the user explicitly asks for it.

In particular:

- Do not preserve old code paths only for backward compatibility.
- Do not add database or data migrations unless requested.
- Do not keep deprecated flags, commands, formats, or aliases when the task is to remove them.
- Prefer deleting obsolete code and tests over maintaining legacy behavior.
