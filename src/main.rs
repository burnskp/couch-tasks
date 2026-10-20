use std::process::ExitCode;

use couch_tasks::{cli, commands, error::AppError};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let message = error.to_string();
            if !message.is_empty() {
                eprintln!(
                    "{}",
                    couch_tasks::output::escape_terminal_multiline(&message)
                );
            }
            ExitCode::from(error.exit_code())
        }
    }
}

fn run() -> Result<(), AppError> {
    let command = cli::parse_env()?;
    commands::dispatch(command)
}
