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
Due: 2026-11-15 09:00
Remind: no

Title:
fix garage light

Notes:
Check the breaker first.
```

Edit the project, due date, reminder, single-line title, or notes, keeping the
section headings.
Save and close the editor to apply the changes. Use `done` and `undone` to change
status; editing the `Status` line does not change it.

## Due dates and reminders

```sh
ct quick --due 4d pay rent
ct add Home fix garage light --due 11-15 --remind
ct due 2 2026-11-20 14:30
ct due 2 3 --clear
ct remind 2 on
ct update 2 --due 1w --no-remind
ct list --due
```

Due dates accept these forms, each optionally followed by a 24-hour `HH:MM`
time:

| Form         | Meaning                                         |
| ------------ | ----------------------------------------------- |
| `YYYY-MM-DD` | That date                                       |
| `YY-MM-DD`   | That date in 20YY                               |
| `MM-DD`      | That month and day this year                    |
| `DD`         | That day this month                             |
| `<n>w<n>d`   | Weeks and/or days from today: `4d`, `1w`, `2w3d` |

Without a time, the due time is 09:00. Dates and times are read and shown in
your local timezone. Dates in the past are accepted and show as overdue. A local
time skipped by a daylight saving change is an error; a repeated one uses the
earlier time.

`ct due <id...> <date>` treats the last argument as the date, or the last two
when the final one is a time, so `ct due 3 15` sets task 3 due on the 15th.
`--clear` removes the due date. `update --due ""` also clears it.

A reminder can only be set on a task with a due date, and clearing a due date
turns its reminder off. `ct remind <id...> on|off` changes several tasks at once
and changes none if any selected task has no due date. Reminders are stored and
synced; this version of `ct` does not send notifications.

`ct list` shows `[due …]` and `[remind]` after task titles. `ct list --due`
shows only pending tasks with due dates, soonest first, in columns for the due
date, reminder, project, and title; add a project name to filter it. It assigns
numeric IDs like `ct list`. Overdue dates are shown in red with a `!` prefix.
`ct show` displays the due date with the time remaining, and `ct edit` includes
`Due:` and `Remind: yes|no` lines that accept the same date forms. Completed
tasks keep their due date and reminder.

Due dates are shown as `YYYY-MM-DD HH:MM` by default. To show the time remaining
instead, pass `--relative` to `ct list`, set `CT_DUE_FORMAT=relative`, or add a
`due` section to the config file:

```json
{
  "due": {
    "format": "relative"
  }
}
```

The format is chosen from `--absolute`/`--relative`, then `CT_DUE_FORMAT`, then
the config file, falling back to `absolute`. It also applies to `ct completed`
and `ct review`. Relative times are rounded down:

| Time remaining | Shown as            | Example        |
| -------------- | ------------------- | -------------- |
| 7 days or more | weeks and days      | `2w3d`         |
| 3 to 7 days    | days                | `4d`           |
| 1 to 3 days    | days and hours      | `2d5h`         |
| Under 1 day    | hours and minutes   | `5h30m`, `45m` |

Overdue tasks show how long ago they were due, such as `!2d5h`.

## Review tasks

```sh
ct review
ct review --days 14
ct review --list
```

`review` walks through pending tasks project by project, `inbox` first. Every
task records when it was last reviewed, starting from when it was created. A
project's tasks come up for review once they have not been reviewed for 7 days
(change this with `--days N`; `--days 0` reviews everything). Every pending
`inbox` task is always included, regardless of when it was last reviewed.

By default, review shows one task at a time. Type a key and press Enter:

| Key          | Action                                        |
| ------------ | --------------------------------------------- |
| `k` or Enter | Keep the task and mark it reviewed            |
| `d`          | Mark the task done                            |
| `x`          | Delete the task (asks for confirmation)       |
| `m`          | Move the task; prompts for the project name   |
| `e`          | Edit the task in `$EDITOR`                    |
| `s`          | Skip the task without marking it reviewed     |
| `q`          | Quit the review                               |

With `--list`, each project is shown as a numbered list and the `review>` prompt
accepts commands. Numbers refer to that list, which is reprinted after each
change; they do not affect the IDs from `ct list`.

| Command           | Action                                                  |
| ----------------- | ------------------------------------------------------- |
| Enter or `r`      | Mark the remaining tasks reviewed and go to next project |
| `k <n...>`        | Keep tasks and mark them reviewed                       |
| `d <n...>`        | Mark tasks done                                         |
| `x <n...>`        | Delete tasks                                            |
| `m <n> <project>` | Move a task to another project                          |
| `e <n>`           | Edit a task in `$EDITOR`                                |
| `s`               | Skip the rest of this project                           |
| `q`               | Quit the review                                         |

Moved and edited tasks are marked reviewed. Quitting or reaching end of input
keeps the changes made so far and leaves unreviewed tasks for the next review.
`ct show` displays a task's last review date.

To make list mode the default, set `CT_REVIEW_MODE=list` or add a `review`
section to `~/.config/couch-tasks/config.json` (or
`$XDG_CONFIG_HOME/couch-tasks/config.json`):

```json
{
  "review": {
    "mode": "list"
  }
}
```

The mode is chosen from `--tasks`/`--list`, then `CT_REVIEW_MODE`, then the
config file, falling back to `task`. Review dates sync between devices.

## Sync between devices

After [configuring sync](docs/sync.md), run `ct sync` on each device to send and
receive changes. Sync is manual. Use `ct sync --verbose` to print a success
message; ordinary successful sync is silent.
