use crate::models::Task;

const ALT_ROW_BACKGROUND: &str = "\x1b[48;2;220;224;232m";
const RESET: &str = "\x1b[0m";
const NOTE_INDICATOR: &str = " [note]";

/// Escape untrusted single-line text without changing ordinary Unicode.
pub fn escape_terminal(text: &str) -> String {
    let mut output = String::new();
    for character in text.chars() {
        if character.is_control() {
            output.extend(character.escape_default());
        } else {
            output.push(character);
        }
    }
    output
}

/// Preserve line breaks in notes and diagnostics, escaping other controls.
pub fn escape_terminal_multiline(text: &str) -> String {
    text.split('\n')
        .map(escape_terminal)
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn format_task_list(tasks: &[Task], show_notes: bool) -> String {
    if tasks.is_empty() {
        return "No pending tasks.\n".to_string();
    }

    format_task_table(tasks, show_notes)
}

pub fn format_visible_task_list(tasks: &[Task], show_notes: bool) -> String {
    if tasks.is_empty() {
        return "No pending tasks.\n".to_string();
    }

    format_visible_task_table(tasks, show_notes)
}

pub fn format_project_task_list(project_name: &str, tasks: &[Task], show_notes: bool) -> String {
    if tasks.is_empty() {
        return format!("{} has no pending tasks.\n", escape_terminal(project_name));
    }

    format_task_table(tasks, show_notes)
}

pub fn format_visible_project_task_list(
    project_name: &str,
    tasks: &[Task],
    show_notes: bool,
) -> String {
    if tasks.is_empty() {
        return format!("{} has no pending tasks.\n", escape_terminal(project_name));
    }

    format_visible_task_table(tasks, show_notes)
}

pub fn format_completed_task_list(project_name: &str, tasks: &[Task]) -> String {
    if tasks.is_empty() {
        return format!(
            "{} has no completed tasks.\n",
            escape_terminal(project_name)
        );
    }

    format_completed_task_table(tasks)
}

pub fn format_completed_task_list_all(tasks: &[Task]) -> String {
    if tasks.is_empty() {
        return "No completed tasks.\n".to_string();
    }

    format_completed_task_table(tasks)
}

pub fn format_task_list_json(tasks: &[Task]) -> String {
    let mut output =
        serde_json::to_string_pretty(tasks).expect("serializing task list to json should not fail");
    output.push('\n');
    output
}

pub fn format_project_list(projects: &[String]) -> String {
    projects
        .iter()
        .map(|name| format!("{}\n", escape_terminal(name)))
        .collect()
}

pub fn format_project_list_json(projects: &[String]) -> String {
    let mut output = serde_json::to_string_pretty(projects)
        .expect("serializing project list to json should not fail");
    output.push('\n');
    output
}

pub fn format_task_details(task: &Task) -> String {
    format_task_details_with_id(task, &task.id.to_string())
}

pub fn format_task_details_with_id(task: &Task, id: &str) -> String {
    let mut output = format!(
        "{}  {}  {}\n",
        escape_terminal(id),
        escape_terminal(&task.project_name),
        escape_terminal(&task.title)
    );
    output.push_str(&format!(
        "Reviewed: {}\n",
        escape_terminal(format_date(&task.reviewed_at))
    ));

    if task.notes.trim().is_empty() {
        output.push_str("\nNotes:\nNo notes.\n");
    } else {
        output.push_str("\nNotes:\n");
        output.push_str(&escape_terminal_multiline(&task.notes));
        output.push('\n');
    }

    output
}

fn format_visible_task_table(tasks: &[Task], show_notes: bool) -> String {
    let visible_ids = (1_i64..)
        .take(tasks.len())
        .map(|id| id.to_string())
        .collect::<Vec<_>>();
    format_task_table_with_ids(tasks, &visible_ids, show_notes)
}

fn format_task_table(tasks: &[Task], show_notes: bool) -> String {
    let ids = tasks
        .iter()
        .map(|task| task.id.to_string())
        .collect::<Vec<_>>();
    format_task_table_with_ids(tasks, &ids, show_notes)
}

fn format_completed_task_table(tasks: &[Task]) -> String {
    let ids = tasks
        .iter()
        .map(|task| escape_terminal(&task.uuid))
        .collect::<Vec<_>>();
    format_task_table_with_ids(tasks, &ids, false)
}

fn format_task_table_with_ids(tasks: &[Task], ids: &[String], show_notes: bool) -> String {
    let id_width = max_width(1, ids.iter().map(|id| id.chars().count()));
    let projects = tasks
        .iter()
        .map(|task| escape_terminal(&task.project_name))
        .collect::<Vec<_>>();
    let project_width = max_width(1, projects.iter().map(|name| name.chars().count()));
    let note_indent = id_width + 2 + project_width + 2;

    let mut output = String::new();

    for (index, (task, id)) in tasks.iter().zip(ids).enumerate() {
        let mut line = format!(
            "{:<id_width$}  {:<project_width$}  {}",
            id,
            projects[index],
            escape_terminal(&task.title)
        );

        if !show_notes && has_notes(task) {
            line.push_str(NOTE_INDICATOR);
        }

        if index % 2 == 1 {
            output.push_str(ALT_ROW_BACKGROUND);
            output.push_str(&line);
            output.push_str(RESET);
            output.push('\n');
        } else {
            output.push_str(&line);
            output.push('\n');
        }

        if show_notes && has_notes(task) {
            for note_line in task.notes.split('\n') {
                output.push_str(&format!(
                    "{:<note_indent$}note: {}\n",
                    "",
                    escape_terminal(note_line),
                    note_indent = note_indent
                ));
            }
        }
    }

    output
}

/// Date portion of an RFC 3339 timestamp.
pub fn format_date(timestamp: &str) -> &str {
    timestamp.get(..10).unwrap_or(timestamp)
}

fn has_notes(task: &Task) -> bool {
    !task.notes.trim().is_empty()
}

fn max_width(base: usize, widths: impl Iterator<Item = usize>) -> usize {
    widths.fold(base, usize::max)
}

#[cfg(test)]
mod tests {
    use crate::models::TaskStatus;

    use super::*;

    fn task(id: i64, project_name: &str, title: &str) -> Task {
        Task {
            id,
            uuid: format!("uuid-{id}"),
            project_id: 1,
            project_name: project_name.to_string(),
            sort_order: id,
            title: title.to_string(),
            notes: String::new(),
            status: TaskStatus::Pending,
            created_at: String::new(),
            updated_at: String::new(),
            completed_at: None,
            reviewed_at: String::new(),
        }
    }

    #[test]
    fn escapes_controls_and_preserves_unicode_and_note_lines() {
        assert_eq!(
            escape_terminal("café\n\r\t\x1b[2J\x07\u{009b}"),
            "café\\n\\r\\t\\u{1b}[2J\\u{7}\\u{9b}"
        );
        assert_eq!(
            escape_terminal_multiline("a\r\nb\x1b[2J"),
            "a\\r\nb\\u{1b}[2J"
        );
        let tasks = vec![task(1, "x\x1b", "a\nb"), task(2, "y", "safe")];
        let output = format_task_list(&tasks, false);
        assert!(output.starts_with("1  x\\u{1b}  a\\nb\n"));
        assert!(output.contains("2  y        safe"));
    }

    #[test]
    fn formats_empty_list() {
        assert_eq!(format_task_list(&[], false), "No pending tasks.\n");
    }

    #[test]
    fn formats_task_table_with_alternating_truecolor_rows_and_note_indicator() {
        let mut noted = task(7, "Home", "fix garage light");
        noted.notes = "Check breaker first".to_string();
        let tasks = vec![
            task(12, "inbox", "buy milk"),
            noted,
            task(19, "Home", "clean gutters"),
        ];

        assert_eq!(
            format_task_list(&tasks, false),
            "12  inbox  buy milk\n\x1b[48;2;220;224;232m7   Home   fix garage light [note]\x1b[0m\n19  Home   clean gutters\n"
        );
    }

    #[test]
    fn formats_task_table_with_inline_notes() {
        let mut noted = task(7, "Home", "fix garage light");
        noted.notes = "Check breaker first".to_string();
        let tasks = vec![noted, task(19, "Away", "play hockey")];

        assert_eq!(
            format_task_list(&tasks, true),
            "7   Home  fix garage light\n          note: Check breaker first\n\x1b[48;2;220;224;232m19  Away  play hockey\x1b[0m\n"
        );
    }
}
