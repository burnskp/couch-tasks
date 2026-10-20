use std::{env, net::SocketAddr, path::PathBuf, process::ExitCode};

use couch_tasks::{
    error::AppError,
    sync_server::{self, DEFAULT_MAX_REQUEST_BYTES, SyncServerConfig},
};
use tracing::error;
use tracing_subscriber::{EnvFilter, fmt};

const DEFAULT_LOG_FILTER: &str = "warn,couch_tasks::sync_server=info,ct_sync_server=info";

#[tokio::main]
async fn main() -> ExitCode {
    init_logging();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let message = couch_tasks::output::escape_terminal_multiline(&error.to_string());
            if !message.is_empty() {
                error!(error = %message, "sync server failed");
                eprintln!("{message}");
            }
            ExitCode::from(error.exit_code())
        }
    }
}

fn init_logging() {
    fmt()
        .with_env_filter(log_filter())
        .with_writer(std::io::stdout)
        .with_ansi(false)
        .init();
}

fn log_filter() -> EnvFilter {
    EnvFilter::new(log_filter_directive(env::var("RUST_LOG")))
}

fn log_filter_directive(rust_log: Result<String, env::VarError>) -> String {
    rust_log
        .ok()
        .filter(|value| !value.trim().is_empty())
        .filter(|value| EnvFilter::try_new(value).is_ok())
        .unwrap_or_else(|| DEFAULT_LOG_FILTER.to_string())
}

async fn run() -> Result<(), AppError> {
    let config = parse_config()?;
    sync_server::serve(config).await
}

fn parse_config() -> Result<SyncServerConfig, AppError> {
    let mut bind = env::var("CT_SYNC_BIND").unwrap_or_else(|_| "127.0.0.1:8443".to_string());
    let mut database = env::var("CT_SYNC_DB").unwrap_or_else(|_| "sync-server.db".to_string());
    let mut cert = env::var("CT_SYNC_SERVER_CERT").ok();
    let mut key = env::var("CT_SYNC_SERVER_KEY").ok();
    let mut client_ca = env::var("CT_SYNC_CLIENT_CA").ok();
    let mut max_request_bytes = env::var("CT_SYNC_MAX_REQUEST_BYTES").ok();

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        let Some(value) = args.next() else {
            return Err(AppError::User(format!("Missing value for {arg}")));
        };
        match arg.as_str() {
            "--bind" => bind = value,
            "--db" => database = value,
            "--cert" => cert = Some(value),
            "--key" => key = Some(value),
            "--client-ca" => client_ca = Some(value),
            "--max-request-bytes" => max_request_bytes = Some(value),
            _ => return Err(AppError::User(format!("Unknown option: {arg}"))),
        }
    }

    let max_request_bytes = parse_max_request_bytes(max_request_bytes.as_deref())?;

    Ok(SyncServerConfig {
        max_request_bytes,
        bind: bind
            .parse::<SocketAddr>()
            .map_err(|_| AppError::User(format!("Invalid bind address: {bind}")))?,
        database: PathBuf::from(database),
        cert: required_path(cert, "--cert or CT_SYNC_SERVER_CERT")?,
        key: required_path(key, "--key or CT_SYNC_SERVER_KEY")?,
        client_ca: required_path(client_ca, "--client-ca or CT_SYNC_CLIENT_CA")?,
    })
}

fn parse_max_request_bytes(value: Option<&str>) -> Result<usize, AppError> {
    match value {
        None => Ok(DEFAULT_MAX_REQUEST_BYTES),
        Some(value) => value.parse::<usize>().ok().filter(|bytes| *bytes > 0)
            .ok_or_else(|| AppError::User(format!(
                "Invalid --max-request-bytes or CT_SYNC_MAX_REQUEST_BYTES: {value:?}; expected a positive integer byte count"
            ))),
    }
}

fn required_path(value: Option<String>, name: &str) -> Result<PathBuf, AppError> {
    value
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| AppError::User(format!("Missing required option: {name}")))
}

#[cfg(test)]
mod tests {
    use std::env::VarError;

    use super::{DEFAULT_LOG_FILTER, log_filter_directive};

    #[test]
    fn request_limit_defaults_and_validation() {
        assert_eq!(
            super::parse_max_request_bytes(None).unwrap(),
            16 * 1024 * 1024
        );
        assert_eq!(super::parse_max_request_bytes(Some("4096")).unwrap(), 4096);
        for invalid in ["0", "-1", "", "16MiB", "1.5", "184467440737095516160"] {
            assert!(
                super::parse_max_request_bytes(Some(invalid)).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn log_filter_uses_rust_log_when_set() {
        assert_eq!(
            log_filter_directive(Ok("couch_tasks::sync_server=debug".to_string())),
            "couch_tasks::sync_server=debug"
        );
    }

    #[test]
    fn log_filter_uses_default_when_rust_log_is_missing_empty_or_invalid() {
        assert_eq!(
            log_filter_directive(Err(VarError::NotPresent)),
            DEFAULT_LOG_FILTER
        );
        assert_eq!(
            log_filter_directive(Ok("  ".to_string())),
            DEFAULT_LOG_FILTER
        );
        assert_eq!(
            log_filter_directive(Ok("couch_tasks::sync_server=notalevel".to_string())),
            DEFAULT_LOG_FILTER
        );
    }
}
