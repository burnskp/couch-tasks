use std::{env, fs, io::Write, process::Command};

use chrono::{DateTime, Local, TimeZone};
use tempfile::NamedTempFile;

use crate::{
    due::{self, DueDisplay},
    error::AppError,
    models::Task,
    storage::Storage,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditedTask {
    pub project: String,
    pub title: String,
    pub notes: String,
    pub due_at: Option<String>,
    pub remind: bool,
}

impl EditedTask {
    /// Save the edited fields to the task.
    pub fn apply(&self, storage: &Storage, id: i64) -> Result<Option<Task>, AppError> {
        let Some(task) =
            storage.update_task_details(id, &self.project, &self.title, &self.notes)?
        else {
            return Ok(None);
        };
        if task.due_at == self.due_at && task.remind == self.remind {
            return Ok(Some(task));
        }
        storage.set_task_due(id, self.due_at.as_deref(), self.remind)
    }
}

pub fn edit_task(task: &Task, display: &DueDisplay) -> Result<EditedTask, AppError> {
    let mut file = NamedTempFile::new()?;
    file.write_all(render_task(task, display).as_bytes())?;
    file.flush()?;

    let editor = env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
    let status = editor_command(&editor, file.path())
        .status()
        .map_err(|error| AppError::Editor(format!("failed to run editor: {error}")))?;

    if !status.success() {
        return Err(AppError::Editor(format!(
            "editor exited with status {}",
            status.code().unwrap_or(1)
        )));
    }

    let edited = fs::read_to_string(file.path())?;
    parse_edited_task(&edited, &Local::now())
}

pub fn render_task(task: &Task, display: &DueDisplay) -> String {
    format!(
        "Project: {}\nStatus: {}\nDue: {}\nRemind: {}\n\nTitle:\n{}\n\nNotes:\n{}\n",
        task.project_name,
        task.status.as_str(),
        task.due_at
            .as_deref()
            .map(|due_at| display.absolute(due_at))
            .unwrap_or_default(),
        if task.remind { "yes" } else { "no" },
        task.title,
        task.notes
    )
}

/// Parse an edited task; `Due:` accepts the same input as `ct due`, read in `now`'s zone.
pub fn parse_edited_task<Tz: TimeZone>(
    contents: &str,
    now: &DateTime<Tz>,
) -> Result<EditedTask, AppError> {
    let lines = contents.lines().collect::<Vec<_>>();

    let project = lines
        .iter()
        .find_map(|line| line.strip_prefix("Project:"))
        .map(str::trim)
        .filter(|project| !project.is_empty())
        .ok_or_else(|| AppError::User("Project name cannot be empty.".to_string()))?;

    let title_index = lines
        .iter()
        .position(|line| line.trim() == "Title:")
        .ok_or_else(|| AppError::User("Edited task is missing Title section.".to_string()))?;

    let header = &lines[..title_index];
    let due_at = header
        .iter()
        .find_map(|line| line.strip_prefix("Due:"))
        .ok_or_else(|| AppError::User("Edited task is missing Due line.".to_string()))?
        .trim();
    let due_at = if due_at.is_empty() {
        None
    } else {
        Some(due::parse_due(due_at, now)?)
    };
    let remind = match header
        .iter()
        .find_map(|line| line.strip_prefix("Remind:"))
        .map(str::trim)
    {
        Some("yes") => true,
        Some("no") => false,
        Some(_) => {
            return Err(AppError::User(
                "Remind must be \"yes\" or \"no\".".to_string(),
            ));
        }
        None => {
            return Err(AppError::User(
                "Edited task is missing Remind line.".to_string(),
            ));
        }
    };
    let remind = remind && due_at.is_some();
    let notes_index = lines
        .iter()
        .position(|line| line.trim() == "Notes:")
        .ok_or_else(|| AppError::User("Edited task is missing Notes section.".to_string()))?;

    if notes_index <= title_index {
        return Err(AppError::User(
            "Edited task has Notes before Title.".to_string(),
        ));
    }

    let title = trim_blank_lines(&lines[title_index + 1..notes_index]).join("\n");
    if title.trim().is_empty() {
        return Err(AppError::User("Task title cannot be empty.".to_string()));
    }
    if title.contains('\n') {
        return Err(AppError::User(
            "Task title must be a single line.".to_string(),
        ));
    }

    let notes = trim_blank_lines(&lines[notes_index + 1..]).join("\n");

    Ok(EditedTask {
        project: project.to_string(),
        title: title.trim().to_string(),
        notes,
        due_at,
        remind,
    })
}

fn editor_command(editor: &str, path: &std::path::Path) -> Command {
    let mut parts = editor.split_whitespace();
    let program = parts.next().unwrap_or("vi");
    let mut command = Command::new(program);
    command.args(parts);
    command.arg(path);
    command
}

fn trim_blank_lines<'a>(lines: &'a [&'a str]) -> &'a [&'a str] {
    let mut start = 0;
    let mut end = lines.len();

    while start < end && lines[start].trim().is_empty() {
        start += 1;
    }
    while end > start && lines[end - 1].trim().is_empty() {
        end -= 1;
    }

    &lines[start..end]
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use crate::{config::DueFormat, models::TaskStatus};

    use super::*;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 2, 12, 0, 0).unwrap()
    }

    fn parse(contents: &str) -> Result<EditedTask, AppError> {
        parse_edited_task(contents, &now())
    }

    #[test]
    fn parses_editable_task_format() {
        let edited = parse(
            "Project: Home\nStatus: pending\nDue:\nRemind: no\n\nTitle:\nfix garage light\n\nNotes:\nCheck breaker first.\n",
        )
        .unwrap();

        assert_eq!(edited.project, "Home");
        assert_eq!(edited.title, "fix garage light");
        assert_eq!(edited.notes, "Check breaker first.");
        assert_eq!(edited.due_at, None);
        assert!(!edited.remind);
    }

    #[test]
    fn renders_and_parses_due_and_remind() {
        let task = Task {
            id: 1,
            uuid: "uuid".to_string(),
            project_id: 1,
            project_name: "Home".to_string(),
            sort_order: 1,
            title: "pay rent".to_string(),
            notes: "Due: not a header".to_string(),
            status: TaskStatus::Pending,
            created_at: String::new(),
            updated_at: String::new(),
            completed_at: None,
            reviewed_at: String::new(),
            due_at: Some("2026-11-15T14:30:00Z".to_string()),
            remind: true,
        };
        let display = DueDisplay {
            format: DueFormat::Relative,
            now: now(),
            to_local: |due| due.naive_utc(),
        };
        let rendered = render_task(&task, &display);
        assert!(rendered.contains("Due: 2026-11-15 14:30\nRemind: yes\n"));

        let edited = parse(&rendered).unwrap();
        assert_eq!(edited.due_at.as_deref(), Some("2026-11-15T14:30:00Z"));
        assert!(edited.remind);

        let edited = parse(&rendered.replace("Due: 2026-11-15 14:30", "Due: 2w")).unwrap();
        assert_eq!(edited.due_at.as_deref(), Some("2026-11-16T09:00:00Z"));

        let edited = parse(&rendered.replace("Due: 2026-11-15 14:30", "Due:")).unwrap();
        assert_eq!(edited.due_at, None);
        assert!(!edited.remind);
    }

    #[test]
    fn rejects_invalid_due_and_remind() {
        let template = |due: &str, remind: &str| {
            format!("Project: Home\nDue: {due}\nRemind: {remind}\n\nTitle:\nx\n\nNotes:\n")
        };
        let error = parse(&template("soon", "no")).unwrap_err().to_string();
        assert!(error.contains("Invalid due date"));
        let error = parse(&template("15", "maybe")).unwrap_err().to_string();
        assert!(error.contains("Remind must be"));
        let error = parse("Project: Home\nRemind: no\n\nTitle:\nx\n\nNotes:\n")
            .unwrap_err()
            .to_string();
        assert!(error.contains("missing Due line"));
    }

    #[test]
    fn rejects_empty_title() {
        let error = parse("Project: Home\nDue:\nRemind: no\n\nTitle:\n\nNotes:\n").unwrap_err();
        assert_eq!(error.exit_code(), 1);
        assert!(error.to_string().contains("Task title cannot be empty"));
    }
}
