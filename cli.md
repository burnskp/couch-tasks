# Using ct

For installation, see the [README](README.md). Run `ct` or `ct --help` for the
command summary.

## Create tasks and projects

```sh
ct quick buy milk
ct add Home fix garage light
ct add "Weekend Projects" paint porch
ct projects
```

`quick` adds to `inbox`. Projects are created when first used. Names are
case-insensitive: `Home` and `home` select the same project, keeping the casing
from its creation. Quote project names containing spaces. `ct projects` lists
`inbox` and projects with at least one pending or completed task.

## Find and select tasks

```sh
ct list
ct list Home
ct Home
ct list --notes
```

`list` shows pending tasks, grouped by project with `inbox` first. `[note]`
indicates a task has notes; `--notes` expands them below the task. Use
`ct list <project>` for project names that match command names.

Each non-JSON list assigns numeric IDs starting at 1. Task commands use the
IDs from that most recent list, including when it was filtered to one project.
Run `ct list` before selecting numbers, and use the numbers actually displayed.

You can also pass a task UUID to any command that accepts a task ID. UUIDs are
stable and appear in completed lists and JSON output. JSON lists and completed
lists do not change the numeric IDs. For automation, see [scripts and widgets](task-usage.md).

The examples below use sample IDs; substitute IDs from your list.

## Complete, restore, and delete

```sh
ct done 1
ct done 2 3
ct completed
ct completed Home
```

To restore a completed task, copy its UUID from `ct completed`:

```sh
ct undone <uuid>
```

Restoring places the task at the end of its project's pending list. To permanently
remove a pending or completed task, use `ct delete <id-or-uuid>`. Both `undone`
and `delete` accept multiple IDs. Batch commands leave all selected tasks
unchanged if any selection is invalid for that operation.

Delete completed tasks in one project or everywhere:

```sh
ct clear-completed Home
ct clear-completed
```

Deletion also propagates to other devices when you sync.

## Move and order tasks

```sh
ct move 2 "Weekend Projects"
ct list "Weekend Projects"
ct reorder 1 2
```

`reorder <id> <position>` places a pending task at a one-based position within
its current project. The destination position must exist in that project's
pending list. Reordering completed tasks is not supported.

## Notes and editing

```sh
ct note 2 Check the breaker first
ct show 2
ct update 2 --title "replace garage light"
ct update 2 --project Home --notes "Buy a replacement bulb."
ct update 2 --notes ""
```

`note` appends a note. `update --notes` replaces all notes; an empty string clears
them. `update` needs at least one of `--title`, `--project`, or `--notes` and
leaves omitted fields unchanged.

For longer edits, run `ct edit <id>`. It opens `$EDITOR`, or `vi` if unset:

```text
Project: Home
Status: pending

Title:
fix garage light

Notes:
Check the breaker first.
```

Edit the project, single-line title, or notes, keeping the section headings.
Save and close the editor to apply the changes. Use `done` and `undone` to change
status; editing the `Status` line does not change it.

## Sync between devices

After [configuring sync](docs/sync.md), run `ct sync` on each device to send and
receive changes. Sync is manual. Use `ct sync --verbose` to print a success
message; ordinary successful sync is silent.
