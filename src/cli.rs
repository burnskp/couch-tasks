use clap::{Arg, ArgAction, Command};

use crate::{error::AppError, output::escape_terminal};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliCommand {
    Help,
    List {
        project: Option<String>,
        show_notes: bool,
        json: bool,
    },
    Completed {
        project: Option<String>,
        json: bool,
    },
    Projects {
        json: bool,
    },
    ClearCompleted {
        project: Option<String>,
    },
    Add {
        project: String,
        title: String,
    },
    Quick {
        title: String,
    },
    Done {
        ids: Vec<String>,
    },
    Undone {
        ids: Vec<String>,
    },
    Delete {
        ids: Vec<String>,
    },
    Move {
        id: String,
        project: String,
    },
    Update {
        id: String,
        project: Option<String>,
        title: Option<String>,
        notes: Option<String>,
    },
    Reorder {
        id: String,
        position: i64,
    },
    Edit {
        id: String,
    },
    Note {
        id: String,
        note: String,
    },
    Show {
        id: String,
    },
    Sync {
        url: Option<String>,
        cert: Option<String>,
        key: Option<String>,
        ca: Option<String>,
        verbose: bool,
    },
    CompleteProjects,
}

pub fn parse_env() -> Result<CliCommand, AppError> {
    parse_args(std::env::args().skip(1))
}

pub fn parse_args<I, S>(args: I) -> Result<CliCommand, AppError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<_>>();

    let Some(command) = args.first().map(String::as_str) else {
        return Ok(CliCommand::Help);
    };

    if command == "--help" || command == "-h" {
        return Ok(CliCommand::Help);
    }

    match command {
        "list" => parse_list(&args[1..]),
        "completed" => parse_completed(&args[1..]),
        "projects" => parse_projects(&args[1..]),
        "clear-completed" => parse_clear_completed(&args[1..]),
        "add" => parse_add(&args[1..]),
        "quick" => parse_quick(&args[1..]),
        "done" => parse_ids_command(&args[1..], "ct done <id...>", |ids| CliCommand::Done {
            ids,
        }),
        "undone" => parse_ids_command(&args[1..], "ct undone <id...>", |ids| CliCommand::Undone {
            ids,
        }),
        "delete" => parse_ids_command(&args[1..], "ct delete <id...>", |ids| CliCommand::Delete {
            ids,
        }),
        "move" => parse_move(&args[1..]),
        "update" => parse_update(&args[1..]),
        "reorder" => parse_reorder(&args[1..]),
        "edit" => parse_edit(&args[1..]),
        "note" => parse_note(&args[1..]),
        "show" => parse_show(&args[1..]),
        "sync" => parse_sync(&args[1..]),
        "_complete" => parse_complete(&args[1..]),
        unknown if unknown.starts_with('-') => Err(usage("ct [command]", "Invalid option")),
        _ if args.len() == 1 => Ok(CliCommand::List {
            project: Some(args[0].clone()),
            show_notes: false,
            json: false,
        }),
        unknown => Err(AppError::User(format!(
            "Unknown command: {}\nUsage: ct [command]",
            escape_terminal(unknown)
        ))),
    }
}

pub fn write_help(mut writer: impl std::io::Write) -> Result<(), AppError> {
    command().write_help(&mut writer).map_err(AppError::from)?;
    writeln!(writer).map_err(AppError::from)
}

pub fn command() -> Command {
    Command::new("ct")
        .about("Couch Tasks CLI")
        .disable_help_subcommand(true)
        .arg_required_else_help(false)
        .subcommand(
            Command::new("list")
                .about("List pending tasks")
                .arg(
                    Arg::new("notes")
                        .long("notes")
                        .help("Show notes under tasks")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Output pending tasks as JSON")
                        .action(ArgAction::SetTrue),
                )
                .arg(project_arg()),
        )
        .subcommand(
            Command::new("completed")
                .about("List completed tasks")
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Output completed tasks as JSON")
                        .action(ArgAction::SetTrue),
                )
                .arg(project_arg()),
        )
        .subcommand(
            Command::new("projects").about("List active projects").arg(
                Arg::new("json")
                    .long("json")
                    .help("Output active projects as JSON")
                    .action(ArgAction::SetTrue),
            ),
        )
        .subcommand(
            Command::new("clear-completed")
                .about("Delete completed tasks")
                .arg(project_arg()),
        )
        .subcommand(
            Command::new("add")
                .about("Add a task to a project")
                .arg(project_arg().required(true))
                .arg(task_words_arg().required(true)),
        )
        .subcommand(
            Command::new("quick")
                .about("Quick add a task to inbox")
                .arg(task_words_arg().required(true)),
        )
        .subcommand(
            Command::new("done")
                .about("Mark tasks done")
                .arg(ids_arg().required(true)),
        )
        .subcommand(
            Command::new("undone")
                .about("Restore completed tasks to pending")
                .arg(ids_arg().required(true)),
        )
        .subcommand(
            Command::new("delete")
                .about("Delete tasks")
                .arg(ids_arg().required(true)),
        )
        .subcommand(
            Command::new("move")
                .about("Move a task to another project")
                .arg(Arg::new("id").required(true))
                .arg(project_arg().required(true)),
        )
        .subcommand(
            Command::new("update")
                .about("Update task fields without opening an editor")
                .arg(Arg::new("id").required(true))
                .arg(Arg::new("project").long("project").num_args(1))
                .arg(Arg::new("title").long("title").num_args(1))
                .arg(Arg::new("notes").long("notes").num_args(1)),
        )
        .subcommand(
            Command::new("reorder")
                .about("Reorder a pending task within its project")
                .arg(Arg::new("id").required(true))
                .arg(Arg::new("position").required(true)),
        )
        .subcommand(
            Command::new("edit")
                .about("Edit a task in $EDITOR")
                .arg(Arg::new("id").required(true)),
        )
        .subcommand(
            Command::new("note")
                .about("Append a note to a task")
                .arg(Arg::new("id").required(true))
                .arg(
                    Arg::new("note")
                        .required(true)
                        .num_args(1..)
                        .action(ArgAction::Append),
                ),
        )
        .subcommand(
            Command::new("show")
                .about("Show task details")
                .arg(Arg::new("id").required(true)),
        )
        .subcommand(
            Command::new("sync")
                .about("Sync tasks")
                .arg(
                    Arg::new("verbose")
                        .short('v')
                        .long("verbose")
                        .help("Show the resulting sync checkpoint")
                        .action(ArgAction::SetTrue),
                )
                .arg(Arg::new("url").long("url").num_args(1))
                .arg(Arg::new("cert").long("cert").num_args(1))
                .arg(Arg::new("key").long("key").num_args(1))
                .arg(Arg::new("ca").long("ca").num_args(1)),
        )
        .subcommand(
            Command::new("_complete")
                .hide(true)
                .about("Completion helper")
                .subcommand(Command::new("projects")),
        )
}

fn parse_list(args: &[String]) -> Result<CliCommand, AppError> {
    let mut show_notes = false;
    let mut json = false;
    let mut project = None;

    for arg in args {
        if arg == "--notes" {
            show_notes = true;
        } else if arg == "--json" {
            json = true;
        } else if arg.starts_with('-') {
            return Err(usage(
                "ct list [--notes] [--json] [project]",
                "Invalid list option",
            ));
        } else if project.is_none() && !arg.trim().is_empty() {
            project = Some(arg.clone());
        } else {
            return Err(usage(
                "ct list [--notes] [--json] [project]",
                "Invalid list arguments",
            ));
        }
    }

    Ok(CliCommand::List {
        project,
        show_notes,
        json,
    })
}

fn parse_completed(args: &[String]) -> Result<CliCommand, AppError> {
    let mut json = false;
    let mut project = None;

    for arg in args {
        if arg == "--json" {
            json = true;
        } else if arg.starts_with('-') {
            return Err(usage(
                "ct completed [--json] [project]",
                "Invalid completed option",
            ));
        } else if project.is_none() && !arg.trim().is_empty() {
            project = Some(arg.clone());
        } else {
            return Err(usage(
                "ct completed [--json] [project]",
                "Invalid completed arguments",
            ));
        }
    }

    Ok(CliCommand::Completed { project, json })
}

fn parse_projects(args: &[String]) -> Result<CliCommand, AppError> {
    match args {
        [] => Ok(CliCommand::Projects { json: false }),
        [option] if option == "--json" => Ok(CliCommand::Projects { json: true }),
        [option] if option.starts_with('-') => {
            Err(usage("ct projects [--json]", "Invalid projects option"))
        }
        _ => Err(usage("ct projects [--json]", "Invalid projects arguments")),
    }
}

fn parse_clear_completed(args: &[String]) -> Result<CliCommand, AppError> {
    match args {
        [] => Ok(CliCommand::ClearCompleted { project: None }),
        [project] if !project.starts_with('-') => Ok(CliCommand::ClearCompleted {
            project: Some(require_non_empty(
                project,
                "ct clear-completed [project]",
                "Project name cannot be empty",
            )?),
        }),
        [_] => Err(usage(
            "ct clear-completed [project]",
            "Invalid clear-completed option",
        )),
        _ => Err(usage(
            "ct clear-completed [project]",
            "Invalid clear-completed arguments",
        )),
    }
}

fn parse_add(args: &[String]) -> Result<CliCommand, AppError> {
    if args.len() < 2 {
        return Err(usage("ct add <project> <task...>", "Missing add arguments"));
    }

    let project = require_non_empty(
        &args[0],
        "ct add <project> <task...>",
        "Project name cannot be empty",
    )?;
    let title = join_non_empty(
        &args[1..],
        "ct add <project> <task...>",
        "Task title cannot be empty",
    )?;
    Ok(CliCommand::Add { project, title })
}

fn parse_quick(args: &[String]) -> Result<CliCommand, AppError> {
    let title = join_non_empty(args, "ct quick <task...>", "Task title cannot be empty")?;
    Ok(CliCommand::Quick { title })
}

fn parse_ids_command(
    args: &[String],
    usage_text: &'static str,
    build: fn(Vec<String>) -> CliCommand,
) -> Result<CliCommand, AppError> {
    if args.is_empty() {
        return Err(usage(usage_text, "Missing task ID"));
    }

    Ok(build(args.to_vec()))
}

fn parse_move(args: &[String]) -> Result<CliCommand, AppError> {
    match args {
        [id, project] => Ok(CliCommand::Move {
            id: parse_task_selector(id, "ct move <id> <project>")?,
            project: require_non_empty(
                project,
                "ct move <id> <project>",
                "Project name cannot be empty",
            )?,
        }),
        _ => Err(usage("ct move <id> <project>", "Invalid move arguments")),
    }
}

fn parse_update(args: &[String]) -> Result<CliCommand, AppError> {
    let usage_text = "ct update <id> [--project <project>] [--title <title>] [--notes <notes>]";
    let Some(id) = args.first() else {
        return Err(usage(usage_text, "Missing task ID"));
    };

    let id = parse_task_selector(id, usage_text)?;
    let mut project = None;
    let mut title = None;
    let mut notes = None;
    let mut index = 1;

    while index < args.len() {
        let flag = args[index].as_str();
        let Some(value) = args.get(index + 1) else {
            return Err(usage(usage_text, &format!("Missing value for {flag}")));
        };

        match flag {
            "--project" => {
                if project.is_some() {
                    return Err(usage(usage_text, "Duplicate --project option"));
                }
                project = Some(require_non_empty(
                    value,
                    usage_text,
                    "Project name cannot be empty",
                )?);
            }
            "--title" => {
                if title.is_some() {
                    return Err(usage(usage_text, "Duplicate --title option"));
                }
                title = Some(require_non_empty(
                    value,
                    usage_text,
                    "Task title cannot be empty",
                )?);
            }
            "--notes" => {
                if notes.is_some() {
                    return Err(usage(usage_text, "Duplicate --notes option"));
                }
                notes = Some(value.to_string());
            }
            option if option.starts_with('-') => {
                return Err(usage(
                    usage_text,
                    &format!("Invalid update option: {option}"),
                ));
            }
            _ => return Err(usage(usage_text, "Invalid update arguments")),
        }

        index += 2;
    }

    if project.is_none() && title.is_none() && notes.is_none() {
        return Err(usage(usage_text, "No updates specified"));
    }

    Ok(CliCommand::Update {
        id,
        project,
        title,
        notes,
    })
}

fn parse_reorder(args: &[String]) -> Result<CliCommand, AppError> {
    match args {
        [id, position] => Ok(CliCommand::Reorder {
            id: parse_task_selector(id, "ct reorder <id> <position>")?,
            position: parse_id(position, "ct reorder <id> <position>")?,
        }),
        _ => Err(usage(
            "ct reorder <id> <position>",
            "Invalid reorder arguments",
        )),
    }
}

fn parse_edit(args: &[String]) -> Result<CliCommand, AppError> {
    match args {
        [id] => Ok(CliCommand::Edit {
            id: parse_task_selector(id, "ct edit <id>")?,
        }),
        _ => Err(usage("ct edit <id>", "Invalid edit arguments")),
    }
}

fn parse_note(args: &[String]) -> Result<CliCommand, AppError> {
    if args.len() < 2 {
        return Err(usage("ct note <id> <note...>", "Missing note arguments"));
    }

    Ok(CliCommand::Note {
        id: parse_task_selector(&args[0], "ct note <id> <note...>")?,
        note: join_non_empty(&args[1..], "ct note <id> <note...>", "Note cannot be empty")?,
    })
}

fn parse_show(args: &[String]) -> Result<CliCommand, AppError> {
    match args {
        [id] => Ok(CliCommand::Show {
            id: parse_task_selector(id, "ct show <id>")?,
        }),
        _ => Err(usage("ct show <id>", "Invalid show arguments")),
    }
}

fn parse_sync(args: &[String]) -> Result<CliCommand, AppError> {
    let usage_text =
        "ct sync [-v|--verbose] [--url <url>] [--cert <path>] [--key <path>] [--ca <path>]";
    let mut url = None;
    let mut cert = None;
    let mut key = None;
    let mut ca = None;
    let mut verbose = false;
    let mut index = 0;

    while index < args.len() {
        let flag = args[index].as_str();
        if matches!(flag, "-v" | "--verbose") {
            if verbose {
                return Err(usage(usage_text, "Duplicate verbose option"));
            }
            verbose = true;
            index += 1;
            continue;
        }

        let Some(value) = args.get(index + 1) else {
            return Err(usage(usage_text, &format!("Missing value for {flag}")));
        };

        match flag {
            "--url" => url = some_once(url, value, usage_text, "Duplicate --url option")?,
            "--cert" => cert = some_once(cert, value, usage_text, "Duplicate --cert option")?,
            "--key" => key = some_once(key, value, usage_text, "Duplicate --key option")?,
            "--ca" => ca = some_once(ca, value, usage_text, "Duplicate --ca option")?,
            option if option.starts_with('-') => {
                return Err(usage(usage_text, &format!("Invalid sync option: {option}")));
            }
            _ => return Err(usage(usage_text, "Invalid sync arguments")),
        }
        index += 2;
    }

    Ok(CliCommand::Sync {
        url,
        cert,
        key,
        ca,
        verbose,
    })
}

fn some_once(
    current: Option<String>,
    value: &str,
    usage_text: &'static str,
    duplicate_message: &str,
) -> Result<Option<String>, AppError> {
    if current.is_some() {
        Err(usage(usage_text, duplicate_message))
    } else if value.trim().is_empty() {
        Err(usage(usage_text, "Sync option value cannot be empty"))
    } else {
        Ok(Some(value.to_string()))
    }
}

fn parse_complete(args: &[String]) -> Result<CliCommand, AppError> {
    match args {
        [kind] if kind == "projects" => Ok(CliCommand::CompleteProjects),
        _ => Err(usage(
            "ct _complete projects",
            "Invalid completion helper arguments",
        )),
    }
}

fn parse_task_selector(value: &str, usage_text: &'static str) -> Result<String, AppError> {
    if value.trim().is_empty() {
        Err(usage(usage_text, "Task ID cannot be empty"))
    } else {
        Ok(value.to_string())
    }
}

fn parse_id(value: &str, usage_text: &'static str) -> Result<i64, AppError> {
    let id = value
        .parse::<i64>()
        .map_err(|_| usage(usage_text, &format!("Invalid task ID: {value}")))?;

    if id <= 0 {
        return Err(usage(usage_text, &format!("Invalid task ID: {value}")));
    }

    Ok(id)
}

fn require_non_empty(
    value: &str,
    usage_text: &'static str,
    message: &str,
) -> Result<String, AppError> {
    let value = value.trim();
    if value.is_empty() {
        Err(usage(usage_text, message))
    } else {
        Ok(value.to_string())
    }
}

fn join_non_empty(
    args: &[String],
    usage_text: &'static str,
    message: &str,
) -> Result<String, AppError> {
    let joined = args.join(" ");
    let joined = joined.trim();
    if joined.is_empty() {
        Err(usage(usage_text, message))
    } else {
        Ok(joined.to_string())
    }
}

fn usage(usage_text: &'static str, message: &str) -> AppError {
    AppError::User(format!("{}\nUsage: {usage_text}", escape_terminal(message)))
}

fn project_arg() -> Arg {
    Arg::new("project")
}

fn task_words_arg() -> Arg {
    Arg::new("task").num_args(1..).action(ArgAction::Append)
}

fn ids_arg() -> Arg {
    Arg::new("ids").num_args(1..).action(ArgAction::Append)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<CliCommand, AppError> {
        parse_args(args.iter().copied())
    }

    #[test]
    fn parses_empty_args_as_help() {
        assert_eq!(parse(&[]).unwrap(), CliCommand::Help);
    }

    #[test]
    fn parses_project_shortcut() {
        assert_eq!(
            parse(&["Home"]).unwrap(),
            CliCommand::List {
                project: Some("Home".to_string()),
                show_notes: false,
                json: false
            }
        );
    }

    #[test]
    fn parses_list_notes_flag() {
        assert_eq!(
            parse(&["list", "--notes", "Home"]).unwrap(),
            CliCommand::List {
                project: Some("Home".to_string()),
                show_notes: true,
                json: false
            }
        );
    }

    #[test]
    fn parses_list_json_flag() {
        assert_eq!(
            parse(&["list", "--json", "Home"]).unwrap(),
            CliCommand::List {
                project: Some("Home".to_string()),
                show_notes: false,
                json: true
            }
        );
    }

    #[test]
    fn parses_projects_json_flag() {
        assert_eq!(
            parse(&["projects", "--json"]).unwrap(),
            CliCommand::Projects { json: true }
        );
        assert_eq!(
            parse(&["projects"]).unwrap(),
            CliCommand::Projects { json: false }
        );
    }

    #[test]
    fn parses_sync_verbose_flags() {
        for verbose_flag in ["-v", "--verbose"] {
            assert_eq!(
                parse(&["sync", verbose_flag, "--url", "https://example.test"]).unwrap(),
                CliCommand::Sync {
                    url: Some("https://example.test".to_string()),
                    cert: None,
                    key: None,
                    ca: None,
                    verbose: true,
                }
            );
        }
    }

    #[test]
    fn parses_add_and_joins_task_words() {
        assert_eq!(
            parse(&["add", "Home", "fix", "garage", "light"]).unwrap(),
            CliCommand::Add {
                project: "Home".to_string(),
                title: "fix garage light".to_string()
            }
        );
    }

    #[test]
    fn parses_primary_commands() {
        assert_eq!(
            parse(&["quick", "buy", "milk"]).unwrap(),
            CliCommand::Quick {
                title: "buy milk".to_string()
            }
        );
        assert_eq!(
            parse(&["done", "7"]).unwrap(),
            CliCommand::Done {
                ids: vec!["7".to_string()]
            }
        );
        assert_eq!(
            parse(&["undone", "7", "8"]).unwrap(),
            CliCommand::Undone {
                ids: vec!["7".to_string(), "8".to_string()]
            }
        );
        assert_eq!(
            parse(&["delete", "8"]).unwrap(),
            CliCommand::Delete {
                ids: vec!["8".to_string()]
            }
        );
        assert_eq!(
            parse(&["show", "8"]).unwrap(),
            CliCommand::Show {
                id: "8".to_string()
            }
        );
        assert_eq!(
            parse(&["reorder", "8", "1"]).unwrap(),
            CliCommand::Reorder {
                id: "8".to_string(),
                position: 1
            }
        );
        assert_eq!(
            parse(&["completed", "--json", "Home"]).unwrap(),
            CliCommand::Completed {
                project: Some("Home".to_string()),
                json: true
            }
        );
        assert_eq!(
            parse(&["completed"]).unwrap(),
            CliCommand::Completed {
                project: None,
                json: false
            }
        );
        assert_eq!(
            parse(&["clear-completed", "Home"]).unwrap(),
            CliCommand::ClearCompleted {
                project: Some("Home".to_string())
            }
        );
        assert_eq!(
            parse(&["clear-completed"]).unwrap(),
            CliCommand::ClearCompleted { project: None }
        );
    }

    #[test]
    fn parses_update_fields() {
        assert_eq!(
            parse(&[
                "update",
                "7",
                "--project",
                "Home",
                "--title",
                "fix light",
                "--notes",
                "check breaker",
            ])
            .unwrap(),
            CliCommand::Update {
                id: "7".to_string(),
                project: Some("Home".to_string()),
                title: Some("fix light".to_string()),
                notes: Some("check breaker".to_string()),
            }
        );
    }

    #[test]
    fn parses_update_empty_notes() {
        assert_eq!(
            parse(&["update", "7", "--notes", ""]).unwrap(),
            CliCommand::Update {
                id: "7".to_string(),
                project: None,
                title: None,
                notes: Some(String::new()),
            }
        );
    }

    #[test]
    fn rejects_invalid_update_arguments() {
        let error = parse(&["update", "7"]).unwrap_err().to_string();
        assert!(error.contains("No updates specified"));

        let error = parse(&["update", "7", "--title", ""])
            .unwrap_err()
            .to_string();
        assert!(error.contains("Task title cannot be empty"));

        let error = parse(&["update", "7", "--project", ""])
            .unwrap_err()
            .to_string();
        assert!(error.contains("Project name cannot be empty"));

        let error = parse(&["update", "7", "--title", "one", "--title", "two"])
            .unwrap_err()
            .to_string();
        assert!(error.contains("Duplicate --title option"));
    }

    #[test]
    fn parses_completion_helper() {
        assert_eq!(
            parse(&["_complete", "projects"]).unwrap(),
            CliCommand::CompleteProjects
        );
    }

    #[test]
    fn rejects_missing_required_arguments_with_usage() {
        let error = parse(&["add", "Home"]).unwrap_err().to_string();
        assert!(error.contains("Usage: ct add <project> <task...>"));
    }
}
