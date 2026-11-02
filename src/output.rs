use crate::{
    due::DueDisplay,
    models::{Task, TaskStatus},
};

const ALT_ROW_BACKGROUND: &str = "\x1b[48;2;220;224;232m";
const RESET: &str = "\x1b[0m";
const OVERDUE_FOREGROUND: &str = "\x1b[31m";
const DEFAULT_FOREGROUND: &str = "\x1b[39m";
const NOTE_INDICATOR: &str = " [note]";
const REMIND_INDICATOR: &str = " [remind]";
const REMIND_COLUMN: &str = "[remind]";

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

pub fn format_task_list(tasks: &[Task], show_notes: bool, due: &DueDisplay) -> String {
    if tasks.is_empty() {
        return "No pending tasks.\n".to_string();
    }

    format_task_table(tasks, show_notes, due)
}

pub fn format_visible_task_list(tasks: &[Task], show_notes: bool, due: &DueDisplay) -> String {
    if tasks.is_empty() {
        return "No pending tasks.\n".to_string();
    }

    format_visible_task_table(tasks, show_notes, due)
}

pub fn format_project_task_list(
    project_name: &str,
    tasks: &[Task],
    show_notes: bool,
    due: &DueDisplay,
) -> String {
    if tasks.is_empty() {
        return format!("{} has no pending tasks.\n", escape_terminal(project_name));
    }

    format_task_table(tasks, show_notes, due)
}

pub fn format_visible_project_task_list(
    project_name: &str,
    tasks: &[Task],
    show_notes: bool,
    due: &DueDisplay,
) -> String {
    if tasks.is_empty() {
        return format!("{} has no pending tasks.\n", escape_terminal(project_name));
    }

    format_visible_task_table(tasks, show_notes, due)
}

pub fn format_visible_due_task_list(tasks: &[Task], show_notes: bool, due: &DueDisplay) -> String {
    if tasks.is_empty() {
        return "No pending tasks with due dates.\n".to_string();
    }

    format_due_task_table(tasks, show_notes, due)
}

pub fn format_visible_project_due_task_list(
    project_name: &str,
    tasks: &[Task],
    show_notes: bool,
    due: &DueDisplay,
) -> String {
    if tasks.is_empty() {
        return format!(
            "{} has no pending tasks with due dates.\n",
            escape_terminal(project_name)
        );
    }

    format_due_task_table(tasks, show_notes, due)
}

pub fn format_completed_task_list(project_name: &str, tasks: &[Task], due: &DueDisplay) -> String {
    if tasks.is_empty() {
        return format!(
            "{} has no completed tasks.\n",
            escape_terminal(project_name)
        );
    }

    format_completed_task_table(tasks, due)
}

pub fn format_completed_task_list_all(tasks: &[Task], due: &DueDisplay) -> String {
    if tasks.is_empty() {
        return "No completed tasks.\n".to_string();
    }

    format_completed_task_table(tasks, due)
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

pub fn format_task_details(task: &Task, due: &DueDisplay) -> String {
    format_task_details_with_id(task, &task.id.to_string(), due)
}

pub fn format_task_details_with_id(task: &Task, id: &str, due: &DueDisplay) -> String {
    let mut output = format!(
        "{}  {}  {}\n",
        escape_terminal(id),
        escape_terminal(&task.project_name),
        escape_terminal(&task.title)
    );
    if let Some(due_at) = &task.due_at {
        output.push_str(&format!(
            "Due: {}\nRemind: {}\n",
            due.format_both(due_at, task.status == TaskStatus::Pending),
            if task.remind { "yes" } else { "no" }
        ));
    }
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

/// ` [due …]` and ` [remind]` tags for a task, with overdue pending due dates in red.
pub fn format_due_tags(task: &Task, due: &DueDisplay) -> String {
    let Some(due_at) = &task.due_at else {
        return String::new();
    };
    let overdue_applies = task.status == TaskStatus::Pending;
    let text = due.format(due_at, overdue_applies);
    let mut output = if overdue_applies && due.is_overdue(due_at) {
        format!(" [due {OVERDUE_FOREGROUND}{text}{DEFAULT_FOREGROUND}]")
    } else {
        format!(" [due {text}]")
    };
    if task.remind {
        output.push_str(REMIND_INDICATOR);
    }
    output
}

fn format_visible_task_table(tasks: &[Task], show_notes: bool, due: &DueDisplay) -> String {
    format_task_table_with_ids(tasks, &visible_ids(tasks), show_notes, due)
}

fn format_task_table(tasks: &[Task], show_notes: bool, due: &DueDisplay) -> String {
    let ids = tasks
        .iter()
        .map(|task| task.id.to_string())
        .collect::<Vec<_>>();
    format_task_table_with_ids(tasks, &ids, show_notes, due)
}

fn format_completed_task_table(tasks: &[Task], due: &DueDisplay) -> String {
    let ids = tasks
        .iter()
        .map(|task| escape_terminal(&task.uuid))
        .collect::<Vec<_>>();
    format_task_table_with_ids(tasks, &ids, false, due)
}

fn visible_ids(tasks: &[Task]) -> Vec<String> {
    (1_i64..)
        .take(tasks.len())
        .map(|id| id.to_string())
        .collect()
}

fn format_task_table_with_ids(
    tasks: &[Task],
    ids: &[String],
    show_notes: bool,
    due: &DueDisplay,
) -> String {
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
        line.push_str(&format_due_tags(task, due));

        if !show_notes && has_notes(task) {
            line.push_str(NOTE_INDICATOR);
        }

        push_row(&mut output, index, &line);
        if show_notes {
            push_notes(&mut output, task, note_indent);
        }
    }

    output
}

/// Flat due-date table: ID, due, reminder, project, title.
fn format_due_task_table(tasks: &[Task], show_notes: bool, due: &DueDisplay) -> String {
    let ids = visible_ids(tasks);
    let id_width = max_width(1, ids.iter().map(|id| id.chars().count()));
    let due_texts = tasks
        .iter()
        .map(|task| {
            task.due_at
                .as_deref()
                .map(|due_at| due.format(due_at, true))
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();
    let due_width = max_width(1, due_texts.iter().map(|text| text.chars().count()));
    let remind_width = REMIND_COLUMN.len();
    let projects = tasks
        .iter()
        .map(|task| escape_terminal(&task.project_name))
        .collect::<Vec<_>>();
    let project_width = max_width(1, projects.iter().map(|name| name.chars().count()));
    let note_indent = id_width + 2 + due_width + 2 + remind_width + 2 + project_width + 2;

    let mut output = String::new();

    for (index, task) in tasks.iter().enumerate() {
        let padded_due = format!("{:<due_width$}", due_texts[index]);
        let due_column = if task
            .due_at
            .as_deref()
            .is_some_and(|due_at| due.is_overdue(due_at))
        {
            let (text, padding) = padded_due.split_at(due_texts[index].len());
            format!("{OVERDUE_FOREGROUND}{text}{DEFAULT_FOREGROUND}{padding}")
        } else {
            padded_due
        };
        let mut line = format!(
            "{:<id_width$}  {}  {:<remind_width$}  {:<project_width$}  {}",
            ids[index],
            due_column,
            if task.remind { REMIND_COLUMN } else { "" },
            projects[index],
            escape_terminal(&task.title)
        );

        if !show_notes && has_notes(task) {
            line.push_str(NOTE_INDICATOR);
        }

        push_row(&mut output, index, &line);
        if show_notes {
            push_notes(&mut output, task, note_indent);
        }
    }

    output
}

fn push_row(output: &mut String, index: usize, line: &str) {
    if index % 2 == 1 {
        output.push_str(ALT_ROW_BACKGROUND);
        output.push_str(line);
        output.push_str(RESET);
    } else {
        output.push_str(line);
    }
    output.push('\n');
}

fn push_notes(output: &mut String, task: &Task, note_indent: usize) {
    if !has_notes(task) {
        return;
    }
    for note_line in task.notes.split('\n') {
        output.push_str(&format!(
            "{:<note_indent$}note: {}\n",
            "",
            escape_terminal(note_line),
            note_indent = note_indent
        ));
    }
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
    use chrono::{TimeZone, Utc};

    use crate::config::DueFormat;

    use super::*;

    fn display(format: DueFormat) -> DueDisplay {
        DueDisplay {
            format,
            now: Utc.with_ymd_and_hms(2026, 11, 2, 12, 0, 0).unwrap(),
            to_local: |due| due.naive_utc(),
        }
    }

    fn absolute() -> DueDisplay {
        display(DueFormat::Absolute)
    }

    fn due_task(id: i64, project_name: &str, title: &str, due_at: &str, remind: bool) -> Task {
        let mut task = task(id, project_name, title);
        task.due_at = Some(due_at.to_string());
        task.remind = remind;
        task
    }

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
            due_at: None,
            remind: false,
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
        let output = format_task_list(&tasks, false, &absolute());
        assert!(output.starts_with("1  x\\u{1b}  a\\nb\n"));
        assert!(output.contains("2  y        safe"));
    }

    #[test]
    fn formats_empty_list() {
        assert_eq!(
            format_task_list(&[], false, &absolute()),
            "No pending tasks.\n"
        );
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
            format_task_list(&tasks, false, &absolute()),
            "12  inbox  buy milk\n\x1b[48;2;220;224;232m7   Home   fix garage light [note]\x1b[0m\n19  Home   clean gutters\n"
        );
    }

    #[test]
    fn formats_task_table_with_inline_notes() {
        let mut noted = task(7, "Home", "fix garage light");
        noted.notes = "Check breaker first".to_string();
        let tasks = vec![noted, task(19, "Away", "play hockey")];

        assert_eq!(
            format_task_list(&tasks, true, &absolute()),
            "7   Home  fix garage light\n          note: Check breaker first\n\x1b[48;2;220;224;232m19  Away  play hockey\x1b[0m\n"
        );
    }

    #[test]
    fn task_table_shows_due_and_remind_tags_with_red_overdue_dates() {
        let tasks = vec![
            due_task(1, "Home", "pay rent", "2026-11-15T09:00:00Z", true),
            due_task(2, "Home", "renew passport", "2026-10-30T09:00:00Z", false),
        ];

        assert_eq!(
            format_task_list(&tasks, false, &absolute()),
            "1  Home  pay rent [due 2026-11-15 09:00] [remind]\n\x1b[48;2;220;224;232m2  Home  renew passport [due \x1b[31m!2026-10-30 09:00\x1b[39m]\x1b[0m\n"
        );
        assert_eq!(
            format_task_list(&tasks[..1], false, &display(DueFormat::Relative)),
            "1  Home  pay rent [due 1w5d] [remind]\n"
        );
    }

    #[test]
    fn completed_tasks_are_never_marked_overdue() {
        let mut done = due_task(1, "Home", "renew passport", "2026-10-30T09:00:00Z", false);
        done.status = TaskStatus::Done;

        assert_eq!(
            format_completed_task_list_all(&[done], &absolute()),
            "uuid-1  Home  renew passport [due 2026-10-30 09:00]\n"
        );
    }

    #[test]
    fn due_table_has_due_remind_project_and_title_columns() {
        let mut noted = due_task(9, "inbox", "call bank", "2026-11-02T15:00:00Z", false);
        noted.notes = "Ask about fees".to_string();
        let tasks = vec![
            due_task(4, "Home", "renew passport", "2026-10-30T09:00:00Z", true),
            noted,
        ];

        assert_eq!(
            format_visible_due_task_list(&tasks, false, &absolute()),
            "1  \x1b[31m!2026-10-30 09:00\x1b[39m  [remind]  Home   renew passport\n\x1b[48;2;220;224;232m2  2026-11-02 15:00             inbox  call bank [note]\x1b[0m\n"
        );
        assert_eq!(
            format_visible_due_task_list(&tasks, true, &display(DueFormat::Relative)),
            "1  \x1b[31m!3d\x1b[39m  [remind]  Home   renew passport\n\x1b[48;2;220;224;232m2  3h             inbox  call bank\x1b[0m\n                         note: Ask about fees\n"
        );
        assert_eq!(
            format_visible_due_task_list(&[], false, &absolute()),
            "No pending tasks with due dates.\n"
        );
    }

    #[test]
    fn task_details_show_absolute_and_relative_due() {
        let task = due_task(3, "Home", "pay rent", "2026-11-04T15:30:00Z", true);
        let output = format_task_details_with_id(&task, "3", &absolute());

        assert!(output.contains("Due: 2026-11-04 15:30 (2d3h)\nRemind: yes\n"));
        let output = format_task_details_with_id(&self::task(4, "Home", "x"), "4", &absolute());
        assert!(!output.contains("Due:"));
    }
}
