use std::{
    env,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::{error::AppError, sync_client::SyncConfigFile};

pub(crate) const CONFIG_FILE_NAME: &str = "config.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub(crate) struct ClientConfigFile {
    #[serde(default)]
    pub(crate) sync: Option<SyncConfigFile>,
    #[serde(default)]
    pub(crate) review: Option<ReviewConfigFile>,
}

impl ClientConfigFile {
    pub(crate) fn load_optional(path: &Path) -> Result<Self, AppError> {
        match fs::read_to_string(path) {
            Ok(contents) => serde_json::from_str(&contents).map_err(|error| {
                AppError::User(format!("Invalid client config {}: {error}", path.display()))
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(AppError::User(format!(
                "Could not read client config {}: {error}",
                path.display()
            ))),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub(crate) struct ReviewConfigFile {
    pub(crate) mode: Option<String>,
}

pub(crate) fn client_config_path() -> Result<PathBuf, AppError> {
    client_config_path_from(env::var_os("XDG_CONFIG_HOME").as_deref(), dirs::home_dir())
}

pub(crate) fn client_config_path_from(
    xdg_config_home: Option<&OsStr>,
    home_dir: Option<PathBuf>,
) -> Result<PathBuf, AppError> {
    if let Some(path) = xdg_config_home.filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path)
            .join("couch-tasks")
            .join(CONFIG_FILE_NAME));
    }

    let home = home_dir.ok_or_else(|| {
        AppError::User("could not determine home directory for client config path".to_string())
    })?;
    Ok(home
        .join(".config")
        .join("couch-tasks")
        .join(CONFIG_FILE_NAME))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ReviewMode {
    #[default]
    Task,
    List,
}

impl ReviewMode {
    pub fn resolve(flag: Option<ReviewMode>) -> Result<Self, AppError> {
        if let Some(mode) = flag {
            return Ok(mode);
        }
        let config_path = client_config_path()?;
        let file_config = ClientConfigFile::load_optional(&config_path)?;
        Self::resolve_from(
            None,
            env::var("CT_REVIEW_MODE").ok(),
            file_config.review.and_then(|review| review.mode),
            &config_path,
        )
    }

    fn resolve_from(
        flag: Option<ReviewMode>,
        env: Option<String>,
        file: Option<String>,
        config_path: &Path,
    ) -> Result<Self, AppError> {
        if let Some(mode) = flag {
            return Ok(mode);
        }
        if let Some(value) = env.filter(|value| !value.trim().is_empty()) {
            return Self::parse(&value).ok_or_else(|| {
                AppError::User(format!(
                    "Invalid CT_REVIEW_MODE: {value:?} (expected \"task\" or \"list\")"
                ))
            });
        }
        if let Some(value) = file.filter(|value| !value.trim().is_empty()) {
            return Self::parse(&value).ok_or_else(|| {
                AppError::User(format!(
                    "Invalid review.mode in {}: {value:?} (expected \"task\" or \"list\")",
                    config_path.display()
                ))
            });
        }
        Ok(Self::default())
    }

    fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "task" => Some(Self::Task),
            "list" => Some(Self::List),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_config_file_is_user_error() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(CONFIG_FILE_NAME);
        fs::write(&path, "not json").expect("write config");

        let error = ClientConfigFile::load_optional(&path).unwrap_err();

        assert!(error.to_string().contains("Invalid client config"));
        assert!(
            error
                .to_string()
                .contains(path.to_str().expect("utf8 path"))
        );
    }

    #[test]
    fn config_file_parses_review_section() {
        let config: ClientConfigFile =
            serde_json::from_str(r#"{"review": {"mode": "list"}}"#).expect("parse config");
        assert_eq!(
            config.review,
            Some(ReviewConfigFile {
                mode: Some("list".to_string())
            })
        );
        assert_eq!(config.sync, None);
    }

    #[test]
    fn config_path_uses_xdg_then_home_fallback() {
        let home = PathBuf::from("/home/alice");

        assert_eq!(
            client_config_path_from(Some(OsStr::new("/tmp/config")), Some(home.clone()))
                .expect("config path"),
            PathBuf::from("/tmp/config")
                .join("couch-tasks")
                .join(CONFIG_FILE_NAME)
        );
        assert_eq!(
            client_config_path_from(None, Some(home.clone())).expect("config path"),
            home.join(".config")
                .join("couch-tasks")
                .join(CONFIG_FILE_NAME)
        );
    }

    #[test]
    fn review_mode_precedence_is_flag_then_env_then_config_then_task() {
        let path = PathBuf::from("/home/alice/.config/couch-tasks/config.json");
        let resolve = |flag, env: Option<&str>, file: Option<&str>| {
            ReviewMode::resolve_from(
                flag,
                env.map(str::to_string),
                file.map(str::to_string),
                &path,
            )
        };

        assert_eq!(resolve(None, None, None).unwrap(), ReviewMode::Task);
        assert_eq!(resolve(None, None, Some("list")).unwrap(), ReviewMode::List);
        assert_eq!(
            resolve(None, Some("task"), Some("list")).unwrap(),
            ReviewMode::Task
        );
        assert_eq!(
            resolve(Some(ReviewMode::List), Some("task"), Some("bogus")).unwrap(),
            ReviewMode::List
        );
        assert_eq!(
            resolve(None, Some(""), Some("list")).unwrap(),
            ReviewMode::List
        );
    }

    #[test]
    fn invalid_review_mode_names_its_source() {
        let path = PathBuf::from("/home/alice/.config/couch-tasks/config.json");

        let error = ReviewMode::resolve_from(None, Some("grid".to_string()), None, &path)
            .unwrap_err()
            .to_string();
        assert!(error.contains("CT_REVIEW_MODE"));

        let error = ReviewMode::resolve_from(None, None, Some("grid".to_string()), &path)
            .unwrap_err()
            .to_string();
        assert!(error.contains("review.mode in /home/alice/.config/couch-tasks/config.json"));
    }
}
