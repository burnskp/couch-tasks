# Couch Tasks

Couch Tasks is a local-first task manager for the terminal. Use `ct` to organize
tasks into projects, add notes, and track completed work. An optional sync server
lets you share your tasks between devices.

## Install

Install Git and the stable Rust toolchain (including Cargo), then run:

```sh
git clone https://github.com/burnskp/couch-tasks.git
cd couch-tasks
cargo install --path .
```

This installs `ct` and `ct-sync-server`. Make sure Cargo's binary directory
(usually `~/.cargo/bin`) is on your `PATH`.

To run without installing, use `cargo run -- <command>` from the checkout.
For Nix, Home Manager, and NixOS installation, see [Nix setup](docs/nix.md).

## Quick start

```sh
ct quick buy milk
ct add Home fix garage light
ct list
```

On a new database, the list shows:

```text
1  inbox  buy milk
2  Home   fix garage light
```

Use the number from your own list to act on a task:

```sh
ct note 2 Check the breaker first
ct show 2
ct done 1
ct list
```

Numbers refer to the most recent non-JSON `ct list` and are reassigned whenever
you list tasks again. Completed tasks are available through `ct completed`;
use their UUIDs to restore or delete them. See [using ct](cli.md) for the full
workflow.

## Your data

The database is created on first use. Its location is chosen in this order:

1. `CT_DB`, when set.
2. `$XDG_DATA_HOME/couch-tasks/tasks.db`.
3. `~/.local/share/couch-tasks/tasks.db`.

To try commands with a separate database:

```sh
CT_DB=/tmp/couch-tasks-demo.db ct quick try Couch Tasks
CT_DB=/tmp/couch-tasks-demo.db ct list
```

Tasks stay local unless you configure and run [sync](docs/sync.md).

## Zsh completion

From the checkout, link the completion file into your completion directory:

```zsh
mkdir -p ~/.zsh/completions
ln -sf "$PWD/completions/_ct" ~/.zsh/completions/_ct
```

Add these lines to `~/.zshrc`, placing the `fpath` setting before any existing
`compinit` call:

```zsh
fpath=(~/.zsh/completions $fpath)
autoload -Uz compinit
compinit
```

Start a new shell. Completion suggests commands, `inbox`, and projects with pending or
completed tasks. Keep the checkout in place when using the symlink. The Nix
package installs the completion file automatically.

## Guides

- [Using ct](cli.md): projects, tasks, notes, editing, and completed work.
- [Scripts and widgets](task-usage.md): JSON output and UUID-based commands.
- [Sync setup](docs/sync.md): connect devices, run a server, and troubleshoot.
- [Nix setup](docs/nix.md): install with Nix or Home Manager and host on NixOS.

## License

[MIT](LICENSE).
