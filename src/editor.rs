use std::{env, fs, io::Write, process::Command};

use tempfile::NamedTempFile;

use crate::{error::AppError, models::Task};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditedTask {
    pub project: String,
    pub title: String,
    pub notes: String,
}

pub fn edit_task(task: &Task) -> Result<EditedTask, AppError> {
    let mut file = NamedTempFile::new()?;
    file.write_all(render_task(task).as_bytes())?;
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
    parse_edited_task(&edited)
}

pub fn render_task(task: &Task) -> String {
    format!(
        "Project: {}\nStatus: {}\n\nTitle:\n{}\n\nNotes:\n{}\n",
        task.project_name,
        task.status.as_str(),
        task.title,
        task.notes
    )
}

pub fn parse_edited_task(contents: &str) -> Result<EditedTask, AppError> {
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
    use super::*;

    #[test]
    fn parses_editable_task_format() {
        let edited = parse_edited_task(
            "Project: Home\nStatus: pending\n\nTitle:\nfix garage light\n\nNotes:\nCheck breaker first.\n",
        )
        .unwrap();

        assert_eq!(edited.project, "Home");
        assert_eq!(edited.title, "fix garage light");
        assert_eq!(edited.notes, "Check breaker first.");
    }

    #[test]
    fn rejects_empty_title() {
        let error = parse_edited_task("Project: Home\n\nTitle:\n\nNotes:\n").unwrap_err();
        assert_eq!(error.exit_code(), 1);
        assert!(error.to_string().contains("Task title cannot be empty"));
    }
}
