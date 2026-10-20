use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use reqwest::{Certificate, Identity, blocking::Client};
use serde::Deserialize;

use crate::{error::AppError, storage::Storage};

const CONFIG_FILE_NAME: &str = "config.json";
const DEFAULT_CLIENT_CERT: &str = "client.crt";
const DEFAULT_CLIENT_KEY: &str = "client.key";
const DEFAULT_SERVER_CA: &str = "server-ca.crt";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncClientConfig {
    pub url: String,
    pub cert: PathBuf,
    pub key: PathBuf,
    pub ca: PathBuf,
}

impl SyncClientConfig {
    pub fn from_env() -> Result<Self, AppError> {
        Self::resolve(None, None, None, None)
    }

    pub fn resolve(
        url: Option<String>,
        cert: Option<String>,
        key: Option<String>,
        ca: Option<String>,
    ) -> Result<Self, AppError> {
        let home_dir = dirs::home_dir();
        let config_path =
            client_config_path_from(env::var_os("XDG_CONFIG_HOME").as_deref(), home_dir.clone())?;
        let data_dir =
            client_data_dir_from(env::var_os("XDG_DATA_HOME").as_deref(), home_dir.clone())?;
        let file_config = ClientConfigFile::load_optional(&config_path)?;
        Self::resolve_from(
            SyncConfigOverrides { url, cert, key, ca },
            SyncConfigEnv::current(),
            file_config.sync,
            &config_path,
            &data_dir,
            home_dir.as_deref(),
        )
    }

    fn resolve_from(
        overrides: SyncConfigOverrides,
        env: SyncConfigEnv,
        file: Option<SyncConfigFile>,
        config_path: &Path,
        data_dir: &Path,
        home_dir: Option<&Path>,
    ) -> Result<Self, AppError> {
        let file = file.unwrap_or_default();
        let url = first_non_empty([overrides.url, env.url, file.url]).ok_or_else(|| {
            AppError::User(format!(
                "Missing sync URL: pass --url, set CT_SYNC_URL, or configure sync.url in {}",
                config_path.display()
            ))
        })?;
        let cert = first_non_empty([overrides.cert, env.cert, file.cert])
            .map(|path| expand_tilde(&path, home_dir))
            .transpose()?
            .unwrap_or_else(|| data_dir.join(DEFAULT_CLIENT_CERT));
        let key = first_non_empty([overrides.key, env.key, file.key])
            .map(|path| expand_tilde(&path, home_dir))
            .transpose()?
            .unwrap_or_else(|| data_dir.join(DEFAULT_CLIENT_KEY));
        let ca = first_non_empty([overrides.ca, env.ca, file.ca])
            .map(|path| expand_tilde(&path, home_dir))
            .transpose()?
            .unwrap_or_else(|| data_dir.join(DEFAULT_SERVER_CA));

        Ok(Self { url, cert, key, ca })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
struct ClientConfigFile {
    #[serde(default)]
    sync: Option<SyncConfigFile>,
}

impl ClientConfigFile {
    fn load_optional(path: &Path) -> Result<Self, AppError> {
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
struct SyncConfigFile {
    url: Option<String>,
    cert: Option<String>,
    key: Option<String>,
    ca: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SyncConfigOverrides {
    url: Option<String>,
    cert: Option<String>,
    key: Option<String>,
    ca: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SyncConfigEnv {
    url: Option<String>,
    cert: Option<String>,
    key: Option<String>,
    ca: Option<String>,
}

impl SyncConfigEnv {
    fn current() -> Self {
        Self {
            url: env::var("CT_SYNC_URL").ok(),
            cert: env::var("CT_SYNC_CERT").ok(),
            key: env::var("CT_SYNC_KEY").ok(),
            ca: env::var("CT_SYNC_CA").ok(),
        }
    }
}

pub fn sync(storage: &Storage, config: &SyncClientConfig) -> Result<i64, AppError> {
    let url = sync_url(&config.url)?;
    let client = build_client(config)?;
    let request = storage.export_sync_request()?;
    let response = client.post(url).json(&request).send().map_err(sync_error)?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().map_err(sync_error)?;
        return Err(server_response_error(status, &body));
    }
    let response = response.json().map_err(sync_error)?;

    storage.apply_sync_response(&request, &response)?;
    Ok(response.checkpoint)
}

fn sync_url(base: &str) -> Result<reqwest::Url, AppError> {
    let url = reqwest::Url::parse(base)
        .map_err(|_| AppError::User("Invalid sync URL: expected an HTTPS URL.".to_string()))?;
    if url.scheme() != "https" || url.host_str().is_none() {
        return Err(AppError::User("Sync URL must use HTTPS.".to_string()));
    }
    let mut endpoint = url;
    endpoint.set_path(&format!("{}/sync", endpoint.path().trim_end_matches('/')));
    Ok(endpoint)
}

fn server_response_error(status: reqwest::StatusCode, body: &str) -> AppError {
    let body = body.trim();
    AppError::Sync(if body.is_empty() {
        format!("server returned {status}")
    } else {
        format!("server returned {status}: {body}")
    })
}

fn build_client(config: &SyncClientConfig) -> Result<Client, AppError> {
    let mut identity_pem = read_sync_file(&config.cert, "client certificate")?;
    identity_pem.extend_from_slice(&read_sync_file(&config.key, "client private key")?);
    let identity = Identity::from_pem(&identity_pem).map_err(sync_error)?;
    let ca = Certificate::from_pem(&read_sync_file(&config.ca, "server CA certificate")?)
        .map_err(sync_error)?;

    Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .identity(identity)
        .add_root_certificate(ca)
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(sync_error)
}

fn read_sync_file(path: &Path, description: &str) -> Result<Vec<u8>, AppError> {
    fs::read(path).map_err(|error| {
        AppError::Sync(format!(
            "could not read {description} {}: {error}",
            path.display()
        ))
    })
}

fn sync_error(error: impl Error) -> AppError {
    let mut message = error.to_string();
    let mut source = error.source();

    if source.is_some() {
        message.push_str("\nCaused by:");
    }
    while let Some(cause) = source {
        message.push_str("\n  - ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }

    AppError::Sync(message)
}

fn client_config_path_from(
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

fn client_data_dir_from(
    xdg_data_home: Option<&OsStr>,
    home_dir: Option<PathBuf>,
) -> Result<PathBuf, AppError> {
    if let Some(path) = xdg_data_home.filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path).join("couch-tasks"));
    }

    let home = home_dir.ok_or_else(|| {
        AppError::User("could not determine home directory for client data path".to_string())
    })?;
    Ok(home.join(".local").join("share").join("couch-tasks"))
}

fn first_non_empty(values: [Option<String>; 3]) -> Option<String> {
    values
        .into_iter()
        .flatten()
        .find(|value| !value.trim().is_empty())
}

fn expand_tilde(path: &str, home_dir: Option<&Path>) -> Result<PathBuf, AppError> {
    if path == "~" {
        return home_dir.map(Path::to_path_buf).ok_or_else(|| {
            AppError::User(format!("could not expand home directory in path: {path}"))
        });
    }

    if let Some(rest) = path.strip_prefix("~/") {
        return home_dir.map(|home| home.join(rest)).ok_or_else(|| {
            AppError::User(format!("could not expand home directory in path: {path}"))
        });
    }

    Ok(PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use std::{error::Error, fmt};

    use super::*;

    #[test]
    fn sync_urls_require_https_and_preserve_base_paths() {
        for url in [
            "http://localhost:8443",
            "ftp://example.com",
            "not a url",
            "/relative",
        ] {
            assert!(matches!(sync_url(url), Err(AppError::User(_))), "{url}");
        }
        assert_eq!(
            sync_url("https://example.com/base/").unwrap().as_str(),
            "https://example.com/base/sync"
        );
    }

    #[derive(Debug)]
    struct TestError {
        message: &'static str,
        source: Option<Box<TestError>>,
    }

    impl fmt::Display for TestError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(self.message)
        }
    }

    impl Error for TestError {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            self.source
                .as_deref()
                .map(|error| error as &(dyn Error + 'static))
        }
    }

    #[test]
    fn sync_errors_include_the_full_cause_chain() {
        let error = TestError {
            message: "error sending request for url (https://example.test/sync)",
            source: Some(Box::new(TestError {
                message: "client error (Connect)",
                source: Some(Box::new(TestError {
                    message: "invalid peer certificate: certificate not valid for this IP address",
                    source: None,
                })),
            })),
        };

        assert_eq!(
            sync_error(error).to_string(),
            "Sync error: error sending request for url (https://example.test/sync)\nCaused by:\n  - client error (Connect)\n  - invalid peer certificate: certificate not valid for this IP address"
        );
    }

    #[test]
    fn server_errors_preserve_actionable_response_bodies() {
        let task_uuid = "a24ef625-90c3-44bc-844b-b0e77809ae25";
        let body = format!(
            "task {task_uuid} is orphaned; run `ct update {task_uuid} --project <valid-project>` or `ct delete {task_uuid}`\n"
        );

        let error = server_response_error(reqwest::StatusCode::CONFLICT, &body);

        assert_eq!(
            error.to_string(),
            format!("Sync error: server returned 409 Conflict: {}", body.trim())
        );
    }

    #[test]
    fn missing_config_uses_default_cert_paths_and_requires_url() {
        let home = PathBuf::from("/home/alice");
        let config_path = home
            .join(".config")
            .join("couch-tasks")
            .join(CONFIG_FILE_NAME);
        let data_dir = home.join(".local").join("share").join("couch-tasks");

        let error = SyncClientConfig::resolve_from(
            SyncConfigOverrides::default(),
            SyncConfigEnv::default(),
            None,
            &config_path,
            &data_dir,
            Some(&home),
        )
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "Missing sync URL: pass --url, set CT_SYNC_URL, or configure sync.url in /home/alice/.config/couch-tasks/config.json"
        );

        let config = SyncClientConfig::resolve_from(
            SyncConfigOverrides {
                url: Some("https://tasks.example.com".to_string()),
                ..SyncConfigOverrides::default()
            },
            SyncConfigEnv::default(),
            None,
            &config_path,
            &data_dir,
            Some(&home),
        )
        .expect("resolve config");

        assert_eq!(config.cert, data_dir.join(DEFAULT_CLIENT_CERT));
        assert_eq!(config.key, data_dir.join(DEFAULT_CLIENT_KEY));
        assert_eq!(config.ca, data_dir.join(DEFAULT_SERVER_CA));
    }

    #[test]
    fn config_file_supplies_url_and_cert_paths() {
        let home = PathBuf::from("/home/alice");
        let data_dir = home.join(".local").join("share").join("couch-tasks");
        let config = SyncClientConfig::resolve_from(
            SyncConfigOverrides::default(),
            SyncConfigEnv::default(),
            Some(SyncConfigFile {
                url: Some("https://config.example.com".to_string()),
                cert: Some("~/.local/share/couch-tasks/alice.crt".to_string()),
                key: Some("/run/secrets/alice.key".to_string()),
                ca: Some("ca.pem".to_string()),
            }),
            &home
                .join(".config")
                .join("couch-tasks")
                .join(CONFIG_FILE_NAME),
            &data_dir,
            Some(&home),
        )
        .expect("resolve config");

        assert_eq!(config.url, "https://config.example.com");
        assert_eq!(
            config.cert,
            home.join(".local")
                .join("share")
                .join("couch-tasks")
                .join("alice.crt")
        );
        assert_eq!(config.key, PathBuf::from("/run/secrets/alice.key"));
        assert_eq!(config.ca, PathBuf::from("ca.pem"));
    }

    #[test]
    fn precedence_is_cli_then_env_then_config_then_defaults() {
        let home = PathBuf::from("/home/alice");
        let data_dir = home.join(".local").join("share").join("couch-tasks");
        let config = SyncClientConfig::resolve_from(
            SyncConfigOverrides {
                url: Some("https://flag.example.com".to_string()),
                cert: None,
                key: Some("flag.key".to_string()),
                ca: None,
            },
            SyncConfigEnv {
                url: Some("https://env.example.com".to_string()),
                cert: Some("env.crt".to_string()),
                key: Some("env.key".to_string()),
                ca: None,
            },
            Some(SyncConfigFile {
                url: Some("https://config.example.com".to_string()),
                cert: Some("config.crt".to_string()),
                key: Some("config.key".to_string()),
                ca: Some("config-ca.crt".to_string()),
            }),
            &home
                .join(".config")
                .join("couch-tasks")
                .join(CONFIG_FILE_NAME),
            &data_dir,
            Some(&home),
        )
        .expect("resolve config");

        assert_eq!(config.url, "https://flag.example.com");
        assert_eq!(config.cert, PathBuf::from("env.crt"));
        assert_eq!(config.key, PathBuf::from("flag.key"));
        assert_eq!(config.ca, PathBuf::from("config-ca.crt"));
    }

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
    fn path_helpers_use_xdg_then_home_fallbacks() {
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
        assert_eq!(
            client_data_dir_from(Some(OsStr::new("/tmp/data")), Some(home.clone()))
                .expect("data path"),
            PathBuf::from("/tmp/data").join("couch-tasks")
        );
        assert_eq!(
            client_data_dir_from(None, Some(home.clone())).expect("data path"),
            home.join(".local").join("share").join("couch-tasks")
        );
    }
}
