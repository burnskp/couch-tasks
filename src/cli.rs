use clap::{Arg, ArgAction, Command};

use crate::{
    config::{DueFormat, ReviewMode},
    due,
    error::AppError,
    output::escape_terminal,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliCommand {
    Help,
    List {
        project: Option<String>,
        show_notes: bool,
        json: bool,
        due_only: bool,
        due_format: Option<DueFormat>,
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
        due: Option<String>,
        remind: bool,
    },
    Quick {
        title: String,
        due: Option<String>,
        remind: bool,
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
        due: Option<String>,
        remind: Option<bool>,
    },
    Due {
        ids: Vec<String>,
        /// `None` clears the due date.
        date: Option<String>,
    },
    Remind {
        ids: Vec<String>,
        on: bool,
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
    Review {
        days: Option<i64>,
        mode: Option<ReviewMode>,
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
        "due" => parse_due(&args[1..]),
        "remind" => parse_remind(&args[1..]),
        "reorder" => parse_reorder(&args[1..]),
        "edit" => parse_edit(&args[1..]),
        "note" => parse_note(&args[1..]),
        "show" => parse_show(&args[1..]),
        "review" => parse_review(&args[1..]),
        "sync" => parse_sync(&args[1..]),
        "_complete" => parse_complete(&args[1..]),
        unknown if unknown.starts_with('-') => Err(usage("ct [command]", "Invalid option")),
        _ if args.len() == 1 => Ok(CliCommand::List {
            project: Some(args[0].clone()),
            show_notes: false,
            json: false,
            due_only: false,
            due_format: None,
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
                .arg(
                    Arg::new("due")
                        .long("due")
                        .help("List only tasks with due dates, soonest first")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("absolute")
                        .long("absolute")
                        .help("Show due dates as local date and time")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("relative")
                        .long("relative")
                        .help("Show due dates as time remaining")
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
                .arg(due_option_arg())
                .arg(remind_flag_arg())
                .arg(project_arg().required(true))
                .arg(task_words_arg().required(true)),
        )
        .subcommand(
            Command::new("quick")
                .about("Quick add a task to inbox")
                .arg(due_option_arg())
                .arg(remind_flag_arg())
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
                .arg(Arg::new("notes").long("notes").num_args(1))
                .arg(due_option_arg().help("Set the due date; an empty value clears it"))
                .arg(remind_flag_arg())
                .arg(
                    Arg::new("no-remind")
                        .long("no-remind")
                        .help("Turn the reminder off")
                        .action(ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("due")
                .about("Set or clear due dates")
                .arg(ids_arg().required(true))
                .arg(Arg::new("date").help(DUE_DATE_HELP))
                .arg(
                    Arg::new("clear")
                        .long("clear")
                        .help("Clear the due date and reminder")
                        .action(ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("remind")
                .about("Turn reminders on or off")
                .arg(ids_arg().required(true))
                .arg(Arg::new("state").required(true).value_parser(["on", "off"])),
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
            Command::new("review")
                .about("Review tasks project by project")
                .arg(
                    Arg::new("days")
                        .long("days")
                        .num_args(1)
                        .help("Review tasks not reviewed in this many days [default: 7]"),
                )
                .arg(
                    Arg::new("tasks")
                        .long("tasks")
                        .help("Review one task at a time")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("list")
                        .long("list")
                        .help("Review each project as a numbered list")
                        .action(ArgAction::SetTrue),
                ),
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
    let usage_text = "ct list [--notes] [--json] [--due] [--absolute | --relative] [project]";
    let mut show_notes = false;
    let mut json = false;
    let mut due_only = false;
    let mut due_format = None;
    let mut project = None;

    for arg in args {
        if arg == "--notes" {
            show_notes = true;
        } else if arg == "--json" {
            json = true;
        } else if arg == "--due" {
            due_only = true;
        } else if arg == "--absolute" || arg == "--relative" {
            if due_format.is_some() {
                return Err(usage(
                    usage_text,
                    "Choose only one of --absolute or --relative",
                ));
            }
            due_format = Some(if arg == "--absolute" {
                DueFormat::Absolute
            } else {
                DueFormat::Relative
            });
        } else if arg.starts_with('-') {
            return Err(usage(usage_text, "Invalid list option"));
        } else if project.is_none() && !arg.trim().is_empty() {
            project = Some(arg.clone());
        } else {
            return Err(usage(usage_text, "Invalid list arguments"));
        }
    }

    Ok(CliCommand::List {
        project,
        show_notes,
        json,
        due_only,
        due_format,
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
    let usage_text = "ct add [--due <date>] [--remind] <project> <task...>";
    let (words, due, remind) = parse_new_task_options(args, usage_text)?;
    if words.len() < 2 {
        return Err(usage(usage_text, "Missing add arguments"));
    }

    let project = require_non_empty(&words[0], usage_text, "Project name cannot be empty")?;
    let title = join_non_empty(&words[1..], usage_text, "Task title cannot be empty")?;
    Ok(CliCommand::Add {
        project,
        title,
        due,
        remind,
    })
}

fn parse_quick(args: &[String]) -> Result<CliCommand, AppError> {
    let usage_text = "ct quick [--due <date>] [--remind] <task...>";
    let (words, due, remind) = parse_new_task_options(args, usage_text)?;
    let title = join_non_empty(&words, usage_text, "Task title cannot be empty")?;
    Ok(CliCommand::Quick { title, due, remind })
}

/// Split `--due <date>` and `--remind` out of the words of a new task.
fn parse_new_task_options(
    args: &[String],
    usage_text: &'static str,
) -> Result<(Vec<String>, Option<String>, bool), AppError> {
    let mut words = Vec::new();
    let mut due = None;
    let mut remind = false;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--due" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(usage(usage_text, "Missing value for --due"));
                };
                due = some_due_once(due, value, usage_text)?;
                index += 1;
            }
            "--remind" => remind = true,
            word => words.push(word.to_string()),
        }
        index += 1;
    }

    if remind && due.is_none() {
        return Err(usage(usage_text, "--remind requires --due"));
    }
    Ok((words, due, remind))
}

fn some_due_once(
    current: Option<String>,
    value: &str,
    usage_text: &'static str,
) -> Result<Option<String>, AppError> {
    if current.is_some() {
        Err(usage(usage_text, "Duplicate --due option"))
    } else if value.trim().is_empty() {
        Err(usage(usage_text, "Due date cannot be empty"))
    } else {
        Ok(Some(value.to_string()))
    }
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
    let usage_text = "ct update <id> [--project <project>] [--title <title>] [--notes <notes>] [--due <date>] [--remind | --no-remind]";
    let Some(id) = args.first() else {
        return Err(usage(usage_text, "Missing task ID"));
    };

    let id = parse_task_selector(id, usage_text)?;
    let mut project = None;
    let mut title = None;
    let mut notes = None;
    let mut due = None;
    let mut remind = None;
    let mut index = 1;

    while index < args.len() {
        let flag = args[index].as_str();
        if flag == "--remind" || flag == "--no-remind" {
            if remind.is_some() {
                return Err(usage(
                    usage_text,
                    "Choose only one of --remind or --no-remind",
                ));
            }
            remind = Some(flag == "--remind");
            index += 1;
            continue;
        }

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
            "--due" => {
                if due.is_some() {
                    return Err(usage(usage_text, "Duplicate --due option"));
                }
                due = Some(value.trim().to_string());
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

    if project.is_none() && title.is_none() && notes.is_none() && due.is_none() && remind.is_none()
    {
        return Err(usage(usage_text, "No updates specified"));
    }
    if remind == Some(true) && due.as_deref() == Some("") {
        return Err(usage(
            usage_text,
            "--remind cannot be used when clearing --due",
        ));
    }

    Ok(CliCommand::Update {
        id,
        project,
        title,
        notes,
        due,
        remind,
    })
}

fn parse_due(args: &[String]) -> Result<CliCommand, AppError> {
    let usage_text = "ct due <id...> <date> [HH:MM] | ct due <id...> --clear";
    let clear = args.iter().any(|arg| arg == "--clear");
    let mut positional = args
        .iter()
        .filter(|arg| arg.as_str() != "--clear")
        .cloned()
        .collect::<Vec<_>>();

    if clear {
        if args.iter().filter(|arg| arg.as_str() == "--clear").count() > 1 {
            return Err(usage(usage_text, "Duplicate --clear option"));
        }
        if positional.is_empty() {
            return Err(usage(usage_text, "Missing task ID"));
        }
        return Ok(CliCommand::Due {
            ids: positional,
            date: None,
        });
    }

    // The date is the last argument, or the last two when it ends with a time.
    let date_len = match positional.as_slice() {
        [.., _, last] if due::is_due_time(last) && positional.len() >= 3 => 2,
        _ => 1,
    };
    if positional.len() <= date_len {
        return Err(usage(usage_text, "Missing task ID or due date"));
    }
    let date = positional.split_off(positional.len() - date_len).join(" ");
    if date.trim().is_empty() {
        return Err(usage(usage_text, "Due date cannot be empty"));
    }

    Ok(CliCommand::Due {
        ids: positional,
        date: Some(date),
    })
}

fn parse_remind(args: &[String]) -> Result<CliCommand, AppError> {
    let usage_text = "ct remind <id...> on|off";
    let Some((state, ids)) = args.split_last() else {
        return Err(usage(usage_text, "Missing task ID"));
    };
    let on = match state.as_str() {
        "on" => true,
        "off" => false,
        _ => return Err(usage(usage_text, "Expected on or off after the task IDs")),
    };
    if ids.is_empty() {
        return Err(usage(usage_text, "Missing task ID"));
    }

    Ok(CliCommand::Remind {
        ids: ids.to_vec(),
        on,
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

fn parse_review(args: &[String]) -> Result<CliCommand, AppError> {
    let usage_text = "ct review [--days <days>] [--tasks | --list]";
    let mut days = None;
    let mut mode = None;
    let mut index = 0;

    while index < args.len() {
        let flag = args[index].as_str();
        match flag {
            "--tasks" | "--list" => {
                if mode.is_some() {
                    return Err(usage(usage_text, "Choose only one of --tasks or --list"));
                }
                mode = Some(if flag == "--tasks" {
                    ReviewMode::Task
                } else {
                    ReviewMode::List
                });
            }
            "--days" => {
                if days.is_some() {
                    return Err(usage(usage_text, "Duplicate --days option"));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(usage(usage_text, "Missing value for --days"));
                };
                let value = value
                    .parse::<i64>()
                    .ok()
                    .filter(|days| *days >= 0)
                    .ok_or_else(|| usage(usage_text, &format!("Invalid review days: {value}")))?;
                days = Some(value);
                index += 1;
            }
            option if option.starts_with('-') => {
                return Err(usage(
                    usage_text,
                    &format!("Invalid review option: {option}"),
                ));
            }
            _ => return Err(usage(usage_text, "Invalid review arguments")),
        }
        index += 1;
    }

    Ok(CliCommand::Review { days, mode })
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

const DUE_DATE_HELP: &str =
    "YYYY-MM-DD, YY-MM-DD, MM-DD, DD, or <n>w<n>d, optionally followed by HH:MM";

fn due_option_arg() -> Arg {
    Arg::new("due")
        .long("due")
        .num_args(1)
        .value_name("date")
        .help(DUE_DATE_HELP)
}

fn remind_flag_arg() -> Arg {
    Arg::new("remind")
        .long("remind")
        .help("Turn the reminder on (requires a due date)")
        .action(ArgAction::SetTrue)
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
                json: false,
                due_only: false,
                due_format: None,
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
                json: false,
                due_only: false,
                due_format: None,
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
                json: true,
                due_only: false,
                due_format: None,
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
                title: "fix garage light".to_string(),
                due: None,
                remind: false,
            }
        );
    }

    #[test]
    fn parses_primary_commands() {
        assert_eq!(
            parse(&["quick", "buy", "milk"]).unwrap(),
            CliCommand::Quick {
                title: "buy milk".to_string(),
                due: None,
                remind: false,
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
                due: None,
                remind: None,
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
                due: None,
                remind: None,
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
    fn parses_review_options() {
        assert_eq!(
            parse(&["review"]).unwrap(),
            CliCommand::Review {
                days: None,
                mode: None
            }
        );
        assert_eq!(
            parse(&["review", "--days", "0", "--list"]).unwrap(),
            CliCommand::Review {
                days: Some(0),
                mode: Some(ReviewMode::List)
            }
        );
        assert_eq!(
            parse(&["review", "--tasks", "--days", "14"]).unwrap(),
            CliCommand::Review {
                days: Some(14),
                mode: Some(ReviewMode::Task)
            }
        );
    }

    #[test]
    fn rejects_invalid_review_options() {
        let error = parse(&["review", "--tasks", "--list"])
            .unwrap_err()
            .to_string();
        assert!(error.contains("Choose only one of --tasks or --list"));

        let error = parse(&["review", "--days", "-1"]).unwrap_err().to_string();
        assert!(error.contains("Invalid review days: -1"));

        let error = parse(&["review", "--days"]).unwrap_err().to_string();
        assert!(error.contains("Missing value for --days"));

        let error = parse(&["review", "Home"]).unwrap_err().to_string();
        assert!(error.contains("Usage: ct review"));
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
        assert!(error.contains("Usage: ct add [--due <date>] [--remind] <project> <task...>"));
    }

    #[test]
    fn parses_list_due_and_format_flags() {
        assert_eq!(
            parse(&["list", "--due", "--relative", "Home"]).unwrap(),
            CliCommand::List {
                project: Some("Home".to_string()),
                show_notes: false,
                json: false,
                due_only: true,
                due_format: Some(DueFormat::Relative),
            }
        );
        let error = parse(&["list", "--absolute", "--relative"])
            .unwrap_err()
            .to_string();
        assert!(error.contains("Choose only one of --absolute or --relative"));
    }

    #[test]
    fn parses_due_and_remind_options_on_new_tasks() {
        assert_eq!(
            parse(&[
                "add", "--due", "15 14:30", "Home", "pay", "rent", "--remind"
            ])
            .unwrap(),
            CliCommand::Add {
                project: "Home".to_string(),
                title: "pay rent".to_string(),
                due: Some("15 14:30".to_string()),
                remind: true,
            }
        );
        assert_eq!(
            parse(&["quick", "buy", "milk", "--due", "2d"]).unwrap(),
            CliCommand::Quick {
                title: "buy milk".to_string(),
                due: Some("2d".to_string()),
                remind: false,
            }
        );

        let error = parse(&["quick", "--remind", "buy", "milk"])
            .unwrap_err()
            .to_string();
        assert!(error.contains("--remind requires --due"));
        let error = parse(&["quick", "buy", "--due"]).unwrap_err().to_string();
        assert!(error.contains("Missing value for --due"));
        let error = parse(&["quick", "--due", "1d", "--due", "2d", "x"])
            .unwrap_err()
            .to_string();
        assert!(error.contains("Duplicate --due option"));
    }

    #[test]
    fn parses_update_due_and_remind() {
        assert_eq!(
            parse(&["update", "7", "--due", "4d", "--remind"]).unwrap(),
            CliCommand::Update {
                id: "7".to_string(),
                project: None,
                title: None,
                notes: None,
                due: Some("4d".to_string()),
                remind: Some(true),
            }
        );
        assert_eq!(
            parse(&["update", "7", "--no-remind"]).unwrap(),
            CliCommand::Update {
                id: "7".to_string(),
                project: None,
                title: None,
                notes: None,
                due: None,
                remind: Some(false),
            }
        );
        assert_eq!(
            parse(&["update", "7", "--due", ""]).unwrap(),
            CliCommand::Update {
                id: "7".to_string(),
                project: None,
                title: None,
                notes: None,
                due: Some(String::new()),
                remind: None,
            }
        );

        let error = parse(&["update", "7", "--remind", "--no-remind"])
            .unwrap_err()
            .to_string();
        assert!(error.contains("Choose only one of --remind or --no-remind"));
        let error = parse(&["update", "7", "--due", "", "--remind"])
            .unwrap_err()
            .to_string();
        assert!(error.contains("--remind cannot be used when clearing --due"));
    }

    #[test]
    fn parses_due_command_with_trailing_date_and_optional_time() {
        assert_eq!(
            parse(&["due", "3", "15"]).unwrap(),
            CliCommand::Due {
                ids: vec!["3".to_string()],
                date: Some("15".to_string()),
            }
        );
        assert_eq!(
            parse(&["due", "1", "2", "15", "14:30"]).unwrap(),
            CliCommand::Due {
                ids: vec!["1".to_string(), "2".to_string()],
                date: Some("15 14:30".to_string()),
            }
        );
        assert_eq!(
            parse(&["due", "3", "15 14:30"]).unwrap(),
            CliCommand::Due {
                ids: vec!["3".to_string()],
                date: Some("15 14:30".to_string()),
            }
        );
        assert_eq!(
            parse(&["due", "3", "4", "--clear"]).unwrap(),
            CliCommand::Due {
                ids: vec!["3".to_string(), "4".to_string()],
                date: None,
            }
        );

        for args in [&["due"][..], &["due", "3"], &["due", "--clear"]] {
            let error = parse(args).unwrap_err().to_string();
            assert!(error.contains("Usage: ct due"), "{args:?}: {error}");
        }
    }

    #[test]
    fn parses_remind_command() {
        assert_eq!(
            parse(&["remind", "3", "4", "on"]).unwrap(),
            CliCommand::Remind {
                ids: vec!["3".to_string(), "4".to_string()],
                on: true,
            }
        );
        assert_eq!(
            parse(&["remind", "3", "off"]).unwrap(),
            CliCommand::Remind {
                ids: vec!["3".to_string()],
                on: false,
            }
        );
        let error = parse(&["remind", "3"]).unwrap_err().to_string();
        assert!(error.contains("Expected on or off"));
        let error = parse(&["remind", "on"]).unwrap_err().to_string();
        assert!(error.contains("Missing task ID"));
    }
}
