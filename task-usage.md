# Using ct from scripts and widgets

Use JSON output to read tasks and UUIDs to change them. This works without a
prior interactive list and avoids numeric IDs changing underneath your script.
See [using ct](cli.md) for task workflows.

## Read tasks and projects

```sh
ct list --json
ct list --json Home
ct completed --json
ct completed --json Home
ct projects --json
```

Task lists return JSON arrays, including `[]` when there are no matching tasks.
An unknown project is an error. `ct projects --json` returns an array of project
names, always including `inbox` plus projects with pending or completed tasks,
for example `["inbox", "Home"]`.

Each task includes these fields:

| Field | Use |
| --- | --- |
| `uuid` | Stable task selector for commands |
| `project_name` | Display name and project filter |
| `title`, `notes` | Task text; absent notes are an empty string |
| `status` | `pending` or `completed` |
| `sort_order` | One-based position for pending tasks within their project |
| `created_at`, `updated_at` | Timestamp strings |
| `completed_at` | Completion timestamp, or `null` for pending tasks |
| `id`, `project_id` | Internal metadata; do not use these as command selectors |

Pending tasks are ordered by project (`inbox` first, then names) and position
within the project. JSON output does not update the interactive numeric IDs.

## Change tasks safely

Pass arguments as arrays when invoking `ct` from an application. For example,
in Python 3:

```python
import json
import subprocess


def ct(*args):
    return subprocess.run(
        ["ct", *args], check=True, capture_output=True, text=True
    ).stdout


ct("quick", "buy milk")
tasks = json.loads(ct("list", "--json", "inbox"))
task_uuid = tasks[-1]["uuid"]
ct("update", task_uuid, "--title", "buy oat milk", "--notes", "")
ct("done", task_uuid)
ct("undone", task_uuid)
```

Use `update` for non-interactive edits; `edit` opens an editor. Pass UUIDs to
`move`, `reorder`, `note`, `show`, and `delete` in the same way. Reorder positions
are relative to the task's project, so use a single-project view for drag ordering.
Refresh task and project lists after successful changes or sync.

## Output and errors

Successful JSON reads write only JSON to stdout. Most mutation commands print
confirmation text; do not parse that text as JSON. Successful `ct sync` is silent
unless `--verbose` is supplied. Errors are written to stderr.

| Exit code | Meaning |
| --- | --- |
| `0` | Success |
| `1` | Invalid arguments, missing task/project, or another usage error |
| `2` | Storage error |
| `3` | Editor error |
| `4` | Sync error |

Surface stderr to users when a command fails. Sync errors need different remedies;
see [sync troubleshooting](docs/sync.md#troubleshooting) before adding retries.
