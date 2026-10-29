use std::{
    collections::HashSet,
    io::{BufRead, Write},
};

use crate::{
    config::ReviewMode,
    editor,
    error::AppError,
    models::{Project, Task},
    output::{self, escape_terminal},
    storage::Storage,
};

const TASK_PROMPT: &str = "[k]eep [d]one [x]delete [m]ove [e]dit [s]kip [q]uit [?]help > ";
const TASK_HELP: &str = "\
  k, Enter  keep the task and mark it reviewed
  d         mark the task done
  x         delete the task
  m         move the task to another project
  e         edit the task in $EDITOR
  s         skip the task without marking it reviewed
  q         quit the review
";
const LIST_PROMPT: &str = "review> ";
const LIST_HELP: &str = "\
  Enter, r          mark the remaining tasks reviewed and go to the next project
  k <n...>          keep tasks and mark them reviewed
  d <n...>          mark tasks done
  x <n...>          delete tasks
  m <n> <project>   move a task to another project
  e <n>             edit a task in $EDITOR
  s                 skip the rest of this project
  q                 quit the review
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    Next,
    Quit,
}

#[derive(Debug, Default)]
struct Summary {
    reviewed: usize,
    done: usize,
    deleted: usize,
    moved: usize,
}

pub fn run(
    storage: &Storage,
    mode: ReviewMode,
    cutoff: &str,
    input: impl BufRead,
    out: impl Write,
) -> Result<(), AppError> {
    Review {
        storage,
        cutoff,
        input,
        out,
        handled: HashSet::new(),
        summary: Summary::default(),
    }
    .run(mode)
}

struct Review<'a, R, W> {
    storage: &'a Storage,
    cutoff: &'a str,
    input: R,
    out: W,
    /// Tasks already handled this session, so moved or kept tasks are not shown twice.
    handled: HashSet<i64>,
    summary: Summary,
}

impl<R: BufRead, W: Write> Review<'_, R, W> {
    fn run(mut self, mode: ReviewMode) -> Result<(), AppError> {
        let projects = self.storage.review_projects(self.cutoff)?;
        if projects.is_empty() {
            writeln!(self.out, "Nothing to review.")?;
            return Ok(());
        }

        for project in projects {
            let flow = match mode {
                ReviewMode::Task => self.review_tasks(&project)?,
                ReviewMode::List => self.review_list(&project)?,
            };
            if flow == Flow::Quit {
                break;
            }
        }

        let Summary {
            reviewed,
            done,
            deleted,
            moved,
        } = self.summary;
        writeln!(
            self.out,
            "\nReviewed {reviewed}, done {done}, deleted {deleted}, moved {moved}."
        )?;
        Ok(())
    }

    fn pending_tasks(&self, project: &Project) -> Result<Vec<Task>, AppError> {
        Ok(self
            .storage
            .review_tasks_for_project(project.id, self.cutoff)?
            .into_iter()
            .filter(|task| !self.handled.contains(&task.id))
            .collect())
    }

    fn review_tasks(&mut self, project: &Project) -> Result<Flow, AppError> {
        let tasks = self.pending_tasks(project)?;
        if tasks.is_empty() {
            return Ok(Flow::Next);
        }

        let name = escape_terminal(&project.name);
        writeln!(self.out, "\n== {name} ({} to review) ==", tasks.len())?;
        for (index, task) in tasks.iter().enumerate() {
            writeln!(self.out, "\n{name} ({}/{})", index + 1, tasks.len())?;
            write!(self.out, "{}", format_task(task))?;
            if self.review_task(task)? == Flow::Quit {
                return Ok(Flow::Quit);
            }
        }
        Ok(Flow::Next)
    }

    fn review_task(&mut self, task: &Task) -> Result<Flow, AppError> {
        loop {
            let Some(line) = self.prompt(TASK_PROMPT)? else {
                return Ok(Flow::Quit);
            };
            match line.as_str() {
                "" | "k" => {
                    self.keep(task)?;
                    return Ok(Flow::Next);
                }
                "d" => {
                    self.done(task)?;
                    return Ok(Flow::Next);
                }
                "x" => {
                    let Some(answer) = self.prompt("Delete? [y/N] ")? else {
                        return Ok(Flow::Quit);
                    };
                    if answer.eq_ignore_ascii_case("y") {
                        self.delete(task)?;
                        return Ok(Flow::Next);
                    }
                }
                "m" => {
                    let Some(project) = self.prompt("Move to project: ")? else {
                        return Ok(Flow::Quit);
                    };
                    if !project.is_empty() && self.move_to(task, &project)? {
                        return Ok(Flow::Next);
                    }
                }
                "e" => {
                    if self.edit(task)? {
                        return Ok(Flow::Next);
                    }
                }
                "s" => return Ok(Flow::Next),
                "q" => return Ok(Flow::Quit),
                "?" => write!(self.out, "{TASK_HELP}")?,
                _ => {
                    writeln!(self.out, "Unknown choice: {}", escape_terminal(&line))?;
                    write!(self.out, "{TASK_HELP}")?;
                }
            }
        }
    }

    fn review_list(&mut self, project: &Project) -> Result<Flow, AppError> {
        loop {
            let tasks = self.pending_tasks(project)?;
            if tasks.is_empty() {
                return Ok(Flow::Next);
            }

            writeln!(self.out, "\n== {} ==", escape_terminal(&project.name))?;
            write!(
                self.out,
                "{}",
                output::format_visible_project_task_list(&project.name, &tasks, true)
            )?;

            let Some(line) = self.prompt(LIST_PROMPT)? else {
                return Ok(Flow::Quit);
            };
            let mut words = line.split_whitespace();
            let command = words.next().unwrap_or("");
            let args = words.collect::<Vec<_>>();

            match command {
                "" | "r" if args.is_empty() => {
                    for task in &tasks {
                        self.keep(task)?;
                    }
                    return Ok(Flow::Next);
                }
                "s" if args.is_empty() => return Ok(Flow::Next),
                "q" if args.is_empty() => return Ok(Flow::Quit),
                "?" if args.is_empty() => write!(self.out, "{LIST_HELP}")?,
                "k" | "d" | "x" => {
                    let Some(selected) = self.select(&tasks, &args)? else {
                        continue;
                    };
                    for task in selected {
                        match command {
                            "k" => self.keep(task)?,
                            "d" => self.done(task)?,
                            _ => self.delete(task)?,
                        }
                    }
                }
                "m" if args.len() >= 2 => {
                    if let Some(selected) = self.select(&tasks, &args[..1])? {
                        self.move_to(selected[0], &args[1..].join(" "))?;
                    }
                }
                "e" if args.len() == 1 => {
                    if let Some(selected) = self.select(&tasks, &args)? {
                        self.edit(selected[0])?;
                    }
                }
                _ => {
                    writeln!(self.out, "Unknown command: {}", escape_terminal(&line))?;
                    write!(self.out, "{LIST_HELP}")?;
                }
            }
        }
    }

    /// Resolve one-based list numbers, reporting every invalid number before acting.
    fn select<'t>(
        &mut self,
        tasks: &'t [Task],
        numbers: &[&str],
    ) -> Result<Option<Vec<&'t Task>>, AppError> {
        if numbers.is_empty() {
            writeln!(self.out, "Missing task number.")?;
            return Ok(None);
        }

        let mut selected = Vec::new();
        let mut invalid = Vec::new();
        for number in numbers {
            match number
                .parse::<usize>()
                .ok()
                .and_then(|n| n.checked_sub(1))
                .and_then(|index| tasks.get(index))
            {
                Some(task) if !selected.iter().any(|t: &&Task| t.id == task.id) => {
                    selected.push(task)
                }
                Some(_) => {}
                None => invalid.push(format!("Invalid task number: {}", escape_terminal(number))),
            }
        }

        if invalid.is_empty() {
            Ok(Some(selected))
        } else {
            writeln!(self.out, "{}", invalid.join("\n"))?;
            Ok(None)
        }
    }

    fn keep(&mut self, task: &Task) -> Result<(), AppError> {
        self.storage.mark_reviewed(task.id)?;
        self.handled.insert(task.id);
        self.summary.reviewed += 1;
        Ok(())
    }

    fn done(&mut self, task: &Task) -> Result<(), AppError> {
        self.storage.mark_done(task.id)?;
        self.handled.insert(task.id);
        self.summary.done += 1;
        writeln!(self.out, "Done: {}", escape_terminal(&task.title))?;
        Ok(())
    }

    fn delete(&mut self, task: &Task) -> Result<(), AppError> {
        self.storage.delete_task(task.id)?;
        self.handled.insert(task.id);
        self.summary.deleted += 1;
        writeln!(self.out, "Removed: {}", escape_terminal(&task.title))?;
        Ok(())
    }

    fn move_to(&mut self, task: &Task, project: &str) -> Result<bool, AppError> {
        let moved = match self.storage.move_task(task.id, project) {
            Ok(Some(moved)) => moved,
            Ok(None) => return Ok(false),
            Err(AppError::User(message)) => {
                writeln!(self.out, "{}", escape_terminal(&message))?;
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        self.storage.mark_reviewed(task.id)?;
        self.handled.insert(task.id);
        self.summary.moved += 1;
        writeln!(
            self.out,
            "Moved to {}: {}",
            escape_terminal(&moved.project_name),
            escape_terminal(&moved.title)
        )?;
        Ok(true)
    }

    fn edit(&mut self, task: &Task) -> Result<bool, AppError> {
        let edited = editor::edit_task(task).and_then(|edited| {
            self.storage
                .update_task_details(task.id, &edited.project, &edited.title, &edited.notes)
        });
        let updated = match edited {
            Ok(Some(updated)) => updated,
            Ok(None) => return Ok(false),
            Err(error @ (AppError::User(_) | AppError::Editor(_))) => {
                writeln!(self.out, "{}", escape_terminal(&error.to_string()))?;
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        self.keep(task)?;
        writeln!(self.out, "Updated: {}", escape_terminal(&updated.title))?;
        Ok(true)
    }

    fn prompt(&mut self, prompt: &str) -> Result<Option<String>, AppError> {
        write!(self.out, "{prompt}")?;
        self.out.flush()?;
        let mut line = String::new();
        if self.input.read_line(&mut line)? == 0 {
            writeln!(self.out)?;
            return Ok(None);
        }
        Ok(Some(line.trim().to_string()))
    }
}

fn format_task(task: &Task) -> String {
    let mut output = format!("  {}\n", escape_terminal(&task.title));
    for line in task.notes.lines() {
        output.push_str(&format!("    {}\n", escape_terminal(line)));
    }
    output.push_str(&format!(
        "  reviewed {}\n",
        escape_terminal(output::format_date(&task.reviewed_at))
    ));
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn storage() -> (tempfile::TempDir, Storage) {
        let dir = tempfile::tempdir().expect("temp dir");
        let storage = Storage::open(dir.path().join("tasks.db")).expect("open storage");
        (dir, storage)
    }

    fn review(storage: &Storage, mode: ReviewMode, cutoff: &str, input: &str) -> String {
        let mut out = Vec::new();
        run(storage, mode, cutoff, input.as_bytes(), &mut out).expect("review");
        String::from_utf8(out).expect("utf8 output")
    }

    const STALE: &str = "2000-01-01T00:00:00Z";

    fn add_stale(storage: &Storage, project: &str, title: &str) -> Task {
        let task = storage.add_task(project, title).expect("add task");
        storage
            .connection()
            .execute(
                "UPDATE tasks SET reviewed_at = ?1 WHERE id = ?2",
                rusqlite::params![STALE, task.id],
            )
            .expect("backdate task");
        storage.get_task(task.id).expect("get task").expect("task")
    }

    fn reviewed_at(storage: &Storage, task: &Task) -> String {
        storage
            .get_task(task.id)
            .expect("get task")
            .expect("task")
            .reviewed_at
    }

    const CUTOFF: &str = "2020-01-01T00:00:00Z";

    #[test]
    fn format_task_escapes_reviewed_at() {
        let (_dir, storage) = storage();
        let mut task = storage.add_task("Home", "task").expect("add");
        task.reviewed_at = "2026-01-0é\u{1b}]0;title\u{7}".to_string();

        let output = format_task(&task);

        assert!(!output.contains('\u{1b}'), "{output:?}");
        assert!(!output.contains('\u{7}'), "{output:?}");
    }

    #[test]
    fn nothing_to_review_when_inbox_is_empty_and_projects_are_fresh() {
        let (_dir, storage) = storage();
        storage.add_task("Home", "fresh").expect("add");

        let output = review(&storage, ReviewMode::Task, CUTOFF, "");

        assert_eq!(output, "Nothing to review.\n");
    }

    #[test]
    fn inbox_is_always_reviewed_and_stale_projects_follow() {
        let (_dir, storage) = storage();
        storage.add_task("inbox", "fresh inbox").expect("add");
        storage.add_task("Home", "fresh home").expect("add");
        let stale = add_stale(&storage, "Work", "stale work");

        let output = review(&storage, ReviewMode::Task, CUTOFF, "k\nk\n");

        assert!(output.contains("== inbox (1 to review) =="));
        assert!(output.contains("== Work (1 to review) =="));
        assert!(!output.contains("fresh home"));
        assert!(output.contains("Reviewed 2, done 0, deleted 0, moved 0."));
        assert_ne!(reviewed_at(&storage, &stale), STALE);
    }

    #[test]
    fn task_mode_handles_each_action() {
        let (_dir, storage) = storage();
        let done = storage.add_task("inbox", "finish me").expect("add");
        let deleted = storage.add_task("inbox", "delete me").expect("add");
        let kept = add_stale(&storage, "inbox", "keep me");
        let moved = storage.add_task("inbox", "move me").expect("add");
        let skipped = add_stale(&storage, "inbox", "skip me");

        let output = review(
            &storage,
            ReviewMode::Task,
            CUTOFF,
            "d\nx\nn\nx\ny\nbogus\nk\nm\n\nm\nHome\ns\n",
        );

        assert!(output.contains("Done: finish me"));
        assert!(output.contains("Removed: delete me"));
        assert!(output.contains("Unknown choice: bogus"));
        assert!(output.contains("Moved to Home: move me"));
        assert!(output.contains("Reviewed 1, done 1, deleted 1, moved 1."));
        assert!(
            storage
                .get_task(done.id)
                .unwrap()
                .unwrap()
                .completed_at
                .is_some()
        );
        assert!(storage.get_task(deleted.id).unwrap().is_none());
        assert_ne!(reviewed_at(&storage, &kept), STALE);
        assert_eq!(
            storage.get_task(moved.id).unwrap().unwrap().project_name,
            "Home"
        );
        assert_eq!(reviewed_at(&storage, &skipped), STALE);
    }

    #[test]
    fn task_mode_quits_on_q_and_eof() {
        let (_dir, storage) = storage();
        storage.add_task("inbox", "one").expect("add");
        storage.add_task("inbox", "two").expect("add");

        let output = review(&storage, ReviewMode::Task, CUTOFF, "q\n");
        assert!(!output.contains("(2/2)"));
        assert!(output.contains("Reviewed 0, done 0, deleted 0, moved 0."));

        let output = review(&storage, ReviewMode::Task, CUTOFF, "k\n");
        assert!(output.contains("(2/2)"));
        assert!(output.contains("Reviewed 1, done 0, deleted 0, moved 0."));
    }

    #[test]
    fn list_mode_handles_commands_and_marks_remaining_reviewed() {
        let (_dir, storage) = storage();
        let done = storage.add_task("inbox", "finish me").expect("add");
        let moved = storage.add_task("inbox", "move me").expect("add");
        let deleted = storage.add_task("inbox", "delete me").expect("add");
        let kept = add_stale(&storage, "inbox", "keep me");

        let output = review(
            &storage,
            ReviewMode::List,
            CUTOFF,
            "d 1 9\nd 1\nm 1 Weekend Projects\nx 1\n\n",
        );

        assert!(output.contains("Invalid task number: 9"));
        assert!(output.contains("Done: finish me"));
        assert!(output.contains("Moved to Weekend Projects: move me"));
        assert!(output.contains("Removed: delete me"));
        assert!(output.contains("Reviewed 1, done 1, deleted 1, moved 1."));
        assert!(
            storage
                .get_task(done.id)
                .unwrap()
                .unwrap()
                .completed_at
                .is_some()
        );
        assert_eq!(
            storage.get_task(moved.id).unwrap().unwrap().project_name,
            "Weekend Projects"
        );
        assert!(storage.get_task(deleted.id).unwrap().is_none());
        assert_ne!(reviewed_at(&storage, &kept), STALE);
    }

    #[test]
    fn list_mode_keep_removes_tasks_from_the_list_and_skip_leaves_rest() {
        let (_dir, storage) = storage();
        let kept = add_stale(&storage, "inbox", "keep me");
        let skipped = add_stale(&storage, "inbox", "skip me");

        let output = review(&storage, ReviewMode::List, CUTOFF, "k 1\ns\n");

        assert!(output.contains("Reviewed 1, done 0, deleted 0, moved 0."));
        assert_ne!(reviewed_at(&storage, &kept), STALE);
        assert_eq!(reviewed_at(&storage, &skipped), STALE);
    }

    #[test]
    fn moved_tasks_are_not_reviewed_again_in_later_projects() {
        let (_dir, storage) = storage();
        storage.add_task("inbox", "triage me").expect("add");
        add_stale(&storage, "Home", "stale");

        let output = review(
            &storage,
            ReviewMode::Task,
            "9999-01-01T00:00:00Z",
            "m\nHome\nk\n",
        );

        assert_eq!(output.matches("triage me").count(), 2);
        assert!(output.contains("== Home (1 to review) =="));
        assert!(output.contains("Reviewed 1, done 0, deleted 0, moved 1."));
    }
}
