use std::io;

use uuid::Uuid;

use crate::{
    cli,
    cli::CliCommand,
    config::ReviewMode,
    editor,
    error::AppError,
    models::{Task, TaskStatus},
    output, review,
    storage::{self, Storage},
    sync_client::{self, SyncClientConfig},
};

pub fn dispatch(command: CliCommand) -> Result<(), AppError> {
    match command {
        CliCommand::Help => cli::write_help(io::stdout()),
        CliCommand::List {
            project,
            show_notes,
            json,
        } => {
            let storage = Storage::open_default()?;
            list(&storage, project, show_notes, json)
        }
        CliCommand::Completed { project, json } => {
            let storage = Storage::open_default()?;
            completed(&storage, project.as_deref(), json)
        }
        CliCommand::Projects { json } => {
            let storage = Storage::open_default()?;
            projects(&storage, json)
        }
        CliCommand::ClearCompleted { project } => {
            let storage = Storage::open_default()?;
            clear_completed(&storage, project.as_deref())
        }
        CliCommand::Add { project, title } => {
            let storage = Storage::open_default()?;
            add(&storage, &project, &title)
        }
        CliCommand::Quick { title } => {
            let storage = Storage::open_default()?;
            add(&storage, "inbox", &title)
        }
        CliCommand::Done { ids } => {
            let storage = Storage::open_default()?;
            mark_done(&storage, &ids)
        }
        CliCommand::Undone { ids } => {
            let storage = Storage::open_default()?;
            mark_undone(&storage, &ids)
        }
        CliCommand::Delete { ids } => {
            let storage = Storage::open_default()?;
            delete(&storage, &ids)
        }
        CliCommand::Move { id, project } => {
            let storage = Storage::open_default()?;
            move_task(&storage, id, &project)
        }
        CliCommand::Update {
            id,
            project,
            title,
            notes,
        } => {
            let storage = Storage::open_default()?;
            update(&storage, id, project, title, notes)
        }
        CliCommand::Reorder { id, position } => {
            let storage = Storage::open_default()?;
            reorder_task(&storage, id, position)
        }
        CliCommand::Note { id, note } => {
            let storage = Storage::open_default()?;
            append_note(&storage, id, &note)
        }
        CliCommand::Edit { id } => {
            let storage = Storage::open_default()?;
            edit(&storage, id)
        }
        CliCommand::Show { id } => {
            let storage = Storage::open_default()?;
            show(&storage, id)
        }
        CliCommand::Review { days, mode } => {
            let mode = ReviewMode::resolve(mode)?;
            let cutoff = storage::review_cutoff(days.unwrap_or(DEFAULT_REVIEW_DAYS))?;
            let storage = Storage::open_default()?;
            review::run(
                &storage,
                mode,
                &cutoff,
                io::stdin().lock(),
                io::stdout().lock(),
            )
        }
        CliCommand::Sync {
            url,
            cert,
            key,
            ca,
            verbose,
        } => {
            let storage = Storage::open_default()?;
            sync_command(&storage, url, cert, key, ca, verbose)
        }
        CliCommand::CompleteProjects => {
            let storage = Storage::open_default()?;
            complete_projects(&storage)
        }
    }
}

const DEFAULT_REVIEW_DAYS: i64 = 7;

fn add(storage: &Storage, project: &str, title: &str) -> Result<(), AppError> {
    let task = storage.add_task(project, title)?;
    println!(
        "Added to {}: {}",
        output::escape_terminal(&task.project_name),
        output::escape_terminal(&task.title)
    );
    Ok(())
}

fn list(
    storage: &Storage,
    project: Option<String>,
    show_notes: bool,
    json: bool,
) -> Result<(), AppError> {
    match project {
        Some(project) => {
            let (project, tasks) = storage
                .list_pending_tasks_for_project(&project)?
                .ok_or_else(|| {
                    AppError::User(format!(
                        "Project not found: {}",
                        output::escape_terminal(&project)
                    ))
                })?;
            if json {
                print!("{}", output::format_task_list_json(&tasks));
            } else {
                storage.save_visible_id_mapping(&tasks)?;
                print!(
                    "{}",
                    output::format_visible_project_task_list(&project.name, &tasks, show_notes)
                );
            }
        }
        None => {
            let tasks = storage.list_pending_tasks()?;
            if json {
                print!("{}", output::format_task_list_json(&tasks));
            } else {
                storage.save_visible_id_mapping(&tasks)?;
                print!("{}", output::format_visible_task_list(&tasks, show_notes));
            }
        }
    }

    Ok(())
}

fn completed(storage: &Storage, project: Option<&str>, json: bool) -> Result<(), AppError> {
    let tasks = match project {
        Some(project) => {
            let (project, tasks) = storage
                .list_completed_tasks_for_project(project)?
                .ok_or_else(|| {
                    AppError::User(format!(
                        "Project not found: {}",
                        output::escape_terminal(project)
                    ))
                })?;
            if json {
                print!("{}", output::format_task_list_json(&tasks));
            } else {
                print!(
                    "{}",
                    output::format_completed_task_list(&project.name, &tasks)
                );
            }
            return Ok(());
        }
        None => storage.list_completed_tasks()?,
    };

    if json {
        print!("{}", output::format_task_list_json(&tasks));
    } else {
        print!("{}", output::format_completed_task_list_all(&tasks));
    }

    Ok(())
}

fn projects(storage: &Storage, json: bool) -> Result<(), AppError> {
    let names = storage.active_project_names()?;
    if json {
        print!("{}", output::format_project_list_json(&names));
    } else {
        print!("{}", output::format_project_list(&names));
    }
    Ok(())
}

fn clear_completed(storage: &Storage, project: Option<&str>) -> Result<(), AppError> {
    let count = storage.clear_completed_tasks(project)?;
    match project {
        Some(project) => println!(
            "Cleared {count} completed tasks from {}.",
            output::escape_terminal(project)
        ),
        None => println!("Cleared {count} completed tasks."),
    }
    Ok(())
}

fn mark_done(storage: &Storage, ids: &[String]) -> Result<(), AppError> {
    let tasks = resolve_tasks(storage, ids)?;
    require_status(&tasks, TaskStatus::Pending, "pending")?;

    for resolved in tasks {
        storage.mark_done(resolved.task.id)?.ok_or_else(|| {
            AppError::StorageMessage(format!(
                "validated task disappeared before update: {}",
                resolved.task.id
            ))
        })?;
        println!(
            "Done {}: {}",
            resolved.label,
            output::escape_terminal(&resolved.task.title)
        );
    }

    Ok(())
}

fn mark_undone(storage: &Storage, ids: &[String]) -> Result<(), AppError> {
    let tasks = resolve_tasks(storage, ids)?;
    require_status(&tasks, TaskStatus::Done, "completed")?;

    for resolved in tasks {
        storage.mark_undone(resolved.task.id)?.ok_or_else(|| {
            AppError::StorageMessage(format!(
                "validated task disappeared before update: {}",
                resolved.task.id
            ))
        })?;
        println!(
            "Undone {}: {}",
            resolved.label,
            output::escape_terminal(&resolved.task.title)
        );
    }

    Ok(())
}

fn delete(storage: &Storage, ids: &[String]) -> Result<(), AppError> {
    let tasks = resolve_tasks(storage, ids)?;

    for resolved in tasks {
        storage.delete_task(resolved.task.id)?.ok_or_else(|| {
            AppError::StorageMessage(format!(
                "validated task disappeared before delete: {}",
                resolved.task.id
            ))
        })?;
        println!(
            "Removed {}: {}",
            resolved.label,
            output::escape_terminal(&resolved.task.title)
        );
    }

    Ok(())
}

const NO_VISIBLE_ID_MAPPING_ERROR: &str = "No task list has been generated yet; run ct list first.";

#[derive(Debug)]
struct ResolvedTask {
    label: String,
    task: Task,
}

#[derive(Debug)]
enum TaskSelector {
    VisibleId(i64),
    Uuid(String),
}

impl TaskSelector {
    fn label(&self) -> String {
        match self {
            Self::VisibleId(id) => id.to_string(),
            Self::Uuid(uuid) => uuid.clone(),
        }
    }
}

fn resolve_tasks(storage: &Storage, ids: &[String]) -> Result<Vec<ResolvedTask>, AppError> {
    let mut mapping = None;
    let mut tasks = Vec::new();
    let mut errors = Vec::new();

    for value in ids {
        match parse_task_selector(value) {
            Some(selector) => match resolve_task_selector(storage, &mut mapping, &selector)? {
                Some(task) => tasks.push(ResolvedTask {
                    label: selector.label(),
                    task,
                }),
                None => errors.push(format!("Task not found: {}", selector.label())),
            },
            None => errors.push(format!(
                "Invalid task ID: {}",
                output::escape_terminal(value)
            )),
        }
    }

    if errors.is_empty() {
        Ok(tasks)
    } else {
        Err(AppError::User(errors.join("\n")))
    }
}

fn resolve_task(storage: &Storage, value: &str) -> Result<ResolvedTask, AppError> {
    let selector = parse_task_selector(value).ok_or_else(|| {
        AppError::User(format!(
            "Invalid task ID: {}",
            output::escape_terminal(value)
        ))
    })?;
    let mut mapping = None;
    let task = resolve_task_selector(storage, &mut mapping, &selector)?
        .ok_or_else(|| AppError::User(format!("Task not found: {}", selector.label())))?;
    Ok(ResolvedTask {
        label: selector.label(),
        task,
    })
}

fn load_visible_id_mapping(storage: &Storage) -> Result<Vec<String>, AppError> {
    storage
        .load_visible_id_mapping()?
        .ok_or_else(|| AppError::User(NO_VISIBLE_ID_MAPPING_ERROR.to_string()))
}

fn resolve_visible_task(
    storage: &Storage,
    mapping: &[String],
    visible_id: i64,
) -> Result<Option<Task>, AppError> {
    let Some(index) = visible_id
        .checked_sub(1)
        .and_then(|id| usize::try_from(id).ok())
    else {
        return Ok(None);
    };
    let Some(uuid) = mapping.get(index) else {
        return Ok(None);
    };
    storage.get_task_by_uuid(uuid)
}

fn resolve_task_selector(
    storage: &Storage,
    mapping: &mut Option<Vec<String>>,
    selector: &TaskSelector,
) -> Result<Option<Task>, AppError> {
    match selector {
        TaskSelector::VisibleId(visible_id) => {
            if mapping.is_none() {
                *mapping = Some(load_visible_id_mapping(storage)?);
            }
            resolve_visible_task(storage, mapping.as_deref().unwrap_or(&[]), *visible_id)
        }
        TaskSelector::Uuid(uuid) => storage.get_task_by_uuid(uuid),
    }
}

fn require_status(
    tasks: &[ResolvedTask],
    expected: TaskStatus,
    expected_name: &str,
) -> Result<(), AppError> {
    let errors = tasks
        .iter()
        .filter(|resolved| resolved.task.status != expected)
        .map(|resolved| format!("Task is not {expected_name}: {}", resolved.label))
        .collect::<Vec<_>>();

    if errors.is_empty() {
        Ok(())
    } else {
        Err(AppError::User(errors.join("\n")))
    }
}

fn parse_task_selector(value: &str) -> Option<TaskSelector> {
    parse_id(value).map(TaskSelector::VisibleId).or_else(|| {
        Uuid::parse_str(value)
            .ok()
            .map(|uuid| TaskSelector::Uuid(uuid.to_string()))
    })
}

fn parse_id(value: &str) -> Option<i64> {
    let id = value.parse::<i64>().ok()?;
    (id > 0).then_some(id)
}

fn move_task(storage: &Storage, id: String, project: &str) -> Result<(), AppError> {
    let resolved = resolve_task(storage, &id)?;
    match storage.move_task(resolved.task.id, project)? {
        Some(task) => {
            println!(
                "Moved {} to {}: {}",
                resolved.label,
                output::escape_terminal(&task.project_name),
                output::escape_terminal(&task.title)
            );
            Ok(())
        }
        None => Err(AppError::User(format!(
            "Task not found: {}",
            resolved.label
        ))),
    }
}

fn update(
    storage: &Storage,
    id: String,
    project: Option<String>,
    title: Option<String>,
    notes: Option<String>,
) -> Result<(), AppError> {
    let resolved = resolve_task(storage, &id)?;
    let project = project.unwrap_or(resolved.task.project_name);
    let title = title.unwrap_or(resolved.task.title);
    let notes = notes.unwrap_or(resolved.task.notes);

    let task = storage
        .update_task_details(resolved.task.id, &project, &title, &notes)?
        .ok_or_else(|| AppError::User(format!("Task not found: {}", resolved.label)))?;
    println!(
        "Updated {}: {}",
        resolved.label,
        output::escape_terminal(&task.title)
    );
    Ok(())
}

fn reorder_task(storage: &Storage, id: String, position: i64) -> Result<(), AppError> {
    let resolved = resolve_task(storage, &id)?;
    if resolved.task.status != TaskStatus::Pending {
        return Err(AppError::User(format!(
            "Task is not pending and cannot be reordered: {}",
            resolved.label
        )));
    }
    match storage.reorder_task(resolved.task.id, position)? {
        Some(task) => {
            println!(
                "Reordered {} in {} to position {}: {}",
                resolved.label,
                output::escape_terminal(&task.project_name),
                task.sort_order,
                output::escape_terminal(&task.title)
            );
            Ok(())
        }
        None => Err(AppError::User(format!(
            "Task not found: {}",
            resolved.label
        ))),
    }
}

fn append_note(storage: &Storage, id: String, note: &str) -> Result<(), AppError> {
    let resolved = resolve_task(storage, &id)?;
    match storage.append_note(resolved.task.id, note)? {
        Some(task) => {
            println!(
                "Noted {}: {}",
                resolved.label,
                output::escape_terminal(&task.title)
            );
            Ok(())
        }
        None => Err(AppError::User(format!(
            "Task not found: {}",
            resolved.label
        ))),
    }
}

fn edit(storage: &Storage, id: String) -> Result<(), AppError> {
    let resolved = resolve_task(storage, &id)?;
    let edited = editor::edit_task(&resolved.task)?;
    storage
        .update_task_details(
            resolved.task.id,
            &edited.project,
            &edited.title,
            &edited.notes,
        )?
        .ok_or_else(|| AppError::User(format!("Task not found: {}", resolved.label)))?;
    Ok(())
}

fn show(storage: &Storage, id: String) -> Result<(), AppError> {
    let resolved = resolve_task(storage, &id)?;
    print!(
        "{}",
        output::format_task_details_with_id(&resolved.task, &resolved.label)
    );
    Ok(())
}

fn sync_command(
    storage: &Storage,
    url: Option<String>,
    cert: Option<String>,
    key: Option<String>,
    ca: Option<String>,
    verbose: bool,
) -> Result<(), AppError> {
    let config = SyncClientConfig::resolve(url, cert, key, ca)?;
    let checkpoint = sync_client::sync(storage, &config)?;
    if verbose {
        println!("Synced to checkpoint {checkpoint}.");
    }
    Ok(())
}

fn complete_projects(storage: &Storage) -> Result<(), AppError> {
    for name in storage.active_project_names()? {
        println!("{name}");
    }
    Ok(())
}
