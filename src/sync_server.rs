use std::{
    future::{Future, poll_fn},
    io::{self, ErrorKind},
    net::SocketAddr,
    path::PathBuf,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use axum::{
    Extension, Json, Router,
    body::{Body, HttpBody},
    extract::{DefaultBodyLimit, State, rejection::JsonRejection},
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use axum_server::accept::Accept;
use rustls::server::WebPkiClientVerifier;
use rustls::{
    RootCertStore, ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
};
use tokio::{net::TcpStream, sync::Mutex, time::timeout};
use tokio_rustls::{TlsAcceptor, server::TlsStream};
use tower_service::Service;
use tracing::{debug, error, info};
use x509_parser::prelude::FromDer;

use crate::{
    error::AppError,
    storage::Storage,
    sync_protocol::{SyncRequest, SyncResponse},
};

pub const DEFAULT_MAX_REQUEST_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncServerConfig {
    pub bind: SocketAddr,
    pub database: PathBuf,
    pub cert: PathBuf,
    pub key: PathBuf,
    pub client_ca: PathBuf,
    pub max_request_bytes: usize,
}

#[derive(Clone)]
struct AppState {
    storage: Arc<Mutex<Storage>>,
    max_request_bytes: usize,
}

#[derive(Debug, Clone)]
struct ClientIdentity {
    peer_addr: SocketAddr,
    tls_cn: Option<String>,
}

#[derive(Clone)]
struct ClientIdentityService<S> {
    inner: S,
    identity: ClientIdentity,
}

impl<S, Body> Service<Request<Body>> for ClientIdentityService<S>
where
    S: Service<Request<Body>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut request: Request<Body>) -> Self::Future {
        request.extensions_mut().insert(self.identity.clone());
        self.inner.call(request)
    }
}

#[derive(Clone)]
struct SyncTlsAcceptor {
    config: Arc<ServerConfig>,
    handshake_timeout: Duration,
}

impl SyncTlsAcceptor {
    fn new(config: Arc<ServerConfig>) -> Self {
        Self {
            config,
            handshake_timeout: Duration::from_secs(10),
        }
    }
}

impl<S> Accept<TcpStream, S> for SyncTlsAcceptor
where
    S: Send + 'static,
{
    type Stream = TlsStream<TcpStream>;
    type Service = ClientIdentityService<S>;
    type Future =
        Pin<Box<dyn Future<Output = io::Result<(Self::Stream, Self::Service)>> + Send + 'static>>;

    fn accept(&self, stream: TcpStream, service: S) -> Self::Future {
        let config = self.config.clone();
        let handshake_timeout = self.handshake_timeout;
        Box::pin(async move {
            let peer_addr = stream.peer_addr()?;
            let stream = timeout(handshake_timeout, TlsAcceptor::from(config).accept(stream))
                .await
                .map_err(|error| io::Error::new(ErrorKind::TimedOut, error))??;
            let tls_cn = client_tls_cn(&stream);
            let identity = ClientIdentity { peer_addr, tls_cn };

            Ok((
                stream,
                ClientIdentityService {
                    inner: service,
                    identity,
                },
            ))
        })
    }
}

pub async fn serve(config: SyncServerConfig) -> Result<(), AppError> {
    if config.max_request_bytes == 0 {
        return Err(AppError::User(
            "max request bytes must be greater than zero".to_string(),
        ));
    }
    info!(database = %config.database.display(), "opening sync server database");
    let storage = Storage::open(config.database.clone()).map_err(|error| {
        error!(database = %config.database.display(), error = %error, "failed to open sync server database");
        error
    })?;
    let state = AppState {
        storage: Arc::new(Mutex::new(storage)),
        max_request_bytes: config.max_request_bytes,
    };
    let router = Router::new()
        .route("/healthz", get(healthz))
        .route(
            "/sync",
            post(sync)
                .layer(DefaultBodyLimit::max(config.max_request_bytes))
                .layer(middleware::from_fn_with_state(
                    state.clone(),
                    buffer_sync_request,
                )),
        )
        .with_state(state);
    let tls = rustls_config(&config)?;

    info!(bind = %config.bind, "starting sync server");
    axum_server::bind(config.bind)
        .acceptor(SyncTlsAcceptor::new(tls))
        .serve(router.into_make_service())
        .await
        .map_err(|error| {
            error!(error = %error, "sync server stopped with error");
            AppError::Sync(error.to_string())
        })
}

fn request_limit_message(limit: usize) -> String {
    format!(
        "sync snapshot exceeds the configured request limit of {limit} bytes; increase --max-request-bytes or CT_SYNC_MAX_REQUEST_BYTES on the server (NixOS: services.couch-tasks-sync-server.maxRequestBytes), restart the service, and retry sync. Larger limits increase potential memory use."
    )
}

// Finish reading oversized uploads without retaining their contents. Closing an HTTP/1
// connection while the client is still uploading can hide the 413 behind a broken pipe.
// Bound the whole read, including draining, so slow or endless uploads cannot hold it open.
async fn buffer_sync_request(
    State(state): State<AppState>,
    Extension(identity): Extension<ClientIdentity>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let (parts, mut body) = request.into_parts();
    let mut oversized = false;
    let buffered = timeout(Duration::from_secs(30), async {
        let mut bytes = Vec::new();
        while let Some(frame) = poll_fn(|cx| Pin::new(&mut body).poll_frame(cx)).await {
            let frame = frame?;
            if let Ok(data) = frame.into_data() {
                if !oversized && data.len() <= state.max_request_bytes - bytes.len() {
                    bytes.extend_from_slice(&data);
                } else {
                    oversized = true;
                    bytes = Vec::new();
                }
            }
        }
        Ok::<_, axum::Error>(bytes)
    })
    .await;

    if oversized {
        info!(
            client_ip = %identity.peer_addr.ip(),
            client_tls_cn = identity.tls_cn.as_deref().unwrap_or("unknown"),
            classification = "payload_too_large",
            max_request_bytes = state.max_request_bytes,
            "sync request rejected"
        );
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            request_limit_message(state.max_request_bytes),
        )
            .into_response();
    }
    match buffered {
        Ok(Ok(bytes)) => {
            next.run(Request::from_parts(parts, Body::from(bytes)))
                .await
        }
        Ok(Err(_)) => (StatusCode::BAD_REQUEST, "could not read sync request body").into_response(),
        Err(_) => (
            StatusCode::REQUEST_TIMEOUT,
            "timed out reading sync request body",
        )
            .into_response(),
    }
}

async fn healthz() -> &'static str {
    "ok\n"
}

async fn sync(
    State(state): State<AppState>,
    Extension(identity): Extension<ClientIdentity>,
    payload: Result<Json<SyncRequest>, JsonRejection>,
) -> Result<Json<SyncResponse>, (StatusCode, String)> {
    let Json(request) = payload.map_err(|error| {
        let status = error.status();
        // Parser and storage errors can contain user text. Keep details in the
        // authenticated response, not in operational logs.
        info!(
            client_ip = %identity.peer_addr.ip(),
            client_addr = %identity.peer_addr,
            client_tls_cn = identity.tls_cn.as_deref().unwrap_or("unknown"),
            classification = "bad_request",
            "sync request rejected"
        );
        let message = if status == StatusCode::PAYLOAD_TOO_LARGE {
            request_limit_message(state.max_request_bytes)
        } else {
            format!("invalid sync request: {error}")
        };
        (status, message)
    })?;
    let started = Instant::now();
    let project_count = request.projects.len();
    let task_count = request.tasks.len();
    let tombstone_count = request.tombstones.len();
    let client_tls_cn = identity.tls_cn.as_deref().unwrap_or("unknown");
    info!(
        client_ip = %identity.peer_addr.ip(),
        client_addr = %identity.peer_addr,
        client_tls_cn,
        checkpoint = request.checkpoint,
        project_count, task_count, tombstone_count, "sync request received"
    );

    let storage = state.storage.lock().await;
    match storage.apply_server_sync(&request) {
        Ok(response) => {
            info!(
                client_ip = %identity.peer_addr.ip(),
                client_addr = %identity.peer_addr,
                client_tls_cn,
                checkpoint = response.checkpoint,
                project_count = response.projects.len(),
                task_count = response.tasks.len(),
                tombstone_count = response.tombstones.len(),
                elapsed_ms = started.elapsed().as_millis(),
                "sync request completed"
            );
            Ok(Json(response))
        }
        Err(error) => {
            let (status, classification) = match error {
                AppError::SyncBadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
                AppError::SyncConflict(_) => (StatusCode::CONFLICT, "conflict"),
                _ => (StatusCode::INTERNAL_SERVER_ERROR, "storage_failure"),
            };
            if status == StatusCode::INTERNAL_SERVER_ERROR {
                error!(
                    client_ip = %identity.peer_addr.ip(),
                    client_addr = %identity.peer_addr,
                    client_tls_cn,
                    checkpoint = request.checkpoint,
                    project_count,
                    task_count,
                    tombstone_count,
                    elapsed_ms = started.elapsed().as_millis(),
                    classification,
                    "sync request failed"
                );
            } else {
                info!(
                    client_ip = %identity.peer_addr.ip(),
                    client_addr = %identity.peer_addr,
                    client_tls_cn,
                    checkpoint = request.checkpoint,
                    project_count,
                    task_count,
                    tombstone_count,
                    elapsed_ms = started.elapsed().as_millis(),
                    classification,
                    "sync request rejected"
                );
            }
            Err((status, error.to_string()))
        }
    }
}

fn rustls_config(config: &SyncServerConfig) -> Result<Arc<ServerConfig>, AppError> {
    info!("loading sync server TLS configuration");
    debug!(
        cert = %config.cert.display(),
        key = %config.key.display(),
        client_ca = %config.client_ca.display(),
        "loading TLS files"
    );

    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let cert_chain = load_certs(&config.cert)?;
    let key = load_private_key(&config.key)?;
    let mut roots = RootCertStore::empty();
    let client_ca_certs = load_certs(&config.client_ca)?;
    for cert in client_ca_certs {
        roots.add(cert).map_err(|error| {
            error!(error = %error, "failed to add client CA certificate");
            AppError::Sync(error.to_string())
        })?;
    }
    let verifier = WebPkiClientVerifier::builder(Arc::new(roots))
        .build()
        .map_err(|error| {
            error!(error = %error, "failed to build client certificate verifier");
            AppError::Sync(error.to_string())
        })?;
    let server_config = ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(cert_chain, key)
        .map_err(|error| {
            error!(error = %error, "failed to build TLS server configuration");
            AppError::Sync(error.to_string())
        })?;
    info!("loaded sync server TLS configuration");
    Ok(Arc::new(server_config))
}

fn client_tls_cn(stream: &TlsStream<TcpStream>) -> Option<String> {
    let (_, connection) = stream.get_ref();
    let cert = connection.peer_certificates()?.first()?;
    certificate_common_name(cert)
}

fn certificate_common_name(cert: &CertificateDer<'_>) -> Option<String> {
    let (_, cert) = x509_parser::certificate::X509Certificate::from_der(cert.as_ref()).ok()?;
    cert.subject()
        .iter_common_name()
        .next()
        .and_then(|name| name.as_str().ok())
        .map(ToString::to_string)
}

fn load_certs(path: &PathBuf) -> Result<Vec<CertificateDer<'static>>, AppError> {
    CertificateDer::pem_file_iter(path)
        .map_err(|error| AppError::Sync(error.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| AppError::Sync(error.to_string()))
}

fn load_private_key(path: &PathBuf) -> Result<PrivateKeyDer<'static>, AppError> {
    PrivateKeyDer::from_pem_file(path).map_err(|error| {
        AppError::Sync(format!(
            "could not load private key {}: {error}",
            path.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, path::Path, process::Command, sync::Arc};

    use axum::{
        Extension, Json,
        body::Body,
        extract::{FromRequest, State},
        http::{Request, StatusCode},
    };
    use tokio::sync::Mutex;
    use uuid::Uuid;

    use crate::{storage::Storage, sync_protocol::SyncTask};

    use super::{AppState, ClientIdentity, certificate_common_name, load_certs, sync};

    #[test]
    fn certificate_common_name_reads_client_certificate_cn() {
        let dir = tempfile::tempdir().expect("temp dir");
        run_openssl(
            dir.path(),
            &[
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-keyout",
                "client.key",
                "-out",
                "client.crt",
                "-subj",
                "/CN=couch-tasks-test-client",
                "-days",
                "1",
            ],
        );

        let certs = load_certs(&dir.path().join("client.crt")).expect("load certs");

        assert_eq!(
            certificate_common_name(certs.first().expect("certificate")),
            Some("couch-tasks-test-client".to_string())
        );
    }

    #[tokio::test]
    async fn sync_handler_classifies_bad_requests_and_conflicts() {
        let dir = tempfile::tempdir().expect("temp dir");
        let storage = Storage::open(dir.path().join("server.db")).expect("open server");
        let mut bad_request = storage.export_sync_request().expect("export request");
        bad_request.checkpoint = -1;
        let state = AppState {
            max_request_bytes: super::DEFAULT_MAX_REQUEST_BYTES,
            storage: Arc::new(Mutex::new(storage)),
        };
        let identity = test_identity();
        let (status, body) = sync(
            State(state.clone()),
            Extension(identity.clone()),
            Ok(Json(bad_request)),
        )
        .await
        .expect_err("negative checkpoint is rejected");
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("checkpoint"));

        let mut orphan = state
            .storage
            .lock()
            .await
            .export_sync_request()
            .expect("export orphan request");
        let task_uuid = Uuid::new_v4().to_string();
        orphan.tasks.push(SyncTask {
            uuid: task_uuid.clone(),
            project_uuid: "missing-project".to_string(),
            sort_order: 1,
            title: "not logged by the handler".to_string(),
            notes: String::new(),
            status: "pending".to_string(),
            created_at: "2026-09-04T00:00:00Z".to_string(),
            updated_at: "2026-09-04T00:00:00Z".to_string(),
            completed_at: None,
            sync_revision: 0,
            sync_dirty: true,
            reviewed_at: "2026-09-04T00:00:00Z".to_string(),
        });
        let (status, body) = sync(State(state), Extension(identity), Ok(Json(orphan)))
            .await
            .expect_err("orphan is rejected");
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(body.contains(&task_uuid));
        assert!(body.contains("ct update"));
        assert!(body.contains("ct delete"));
    }

    #[tokio::test]
    async fn rejected_sync_logs_omit_user_text() {
        let dir = tempfile::tempdir().expect("temp dir");
        let log_path = dir.path().join("server.log");
        let log_file = std::fs::File::create(&log_path).expect("create log");
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || log_file.try_clone().expect("clone log file"))
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        let storage = Storage::open(dir.path().join("server.db")).expect("open server");
        let mut request = storage.export_sync_request().expect("export snapshot");
        let private_text = "confidential-project-marker";
        request.projects[0].name = private_text.to_string();
        request.projects[0].name_normalized = private_text.to_string();
        let mut duplicate = request.projects[0].clone();
        duplicate.uuid = Uuid::new_v4().to_string();
        request.projects.push(duplicate);
        let state = AppState {
            max_request_bytes: super::DEFAULT_MAX_REQUEST_BYTES,
            storage: Arc::new(Mutex::new(storage)),
        };
        let (status, body) = sync(
            State(state.clone()),
            Extension(test_identity()),
            Ok(Json(request)),
        )
        .await
        .expect_err("duplicate project name");
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains(private_text));

        let request = Request::post("/sync")
            .header("content-type", "application/json")
            .body(Body::from(format!(r#"{{"checkpoint":"{private_text}"}}"#)))
            .expect("build malformed request");
        let rejection = Json::<crate::sync_protocol::SyncRequest>::from_request(request, &())
            .await
            .expect_err("checkpoint must be a number");
        let (_, body) = sync(State(state), Extension(test_identity()), Err(rejection))
            .await
            .expect_err("malformed JSON payload");
        assert!(body.contains(private_text));

        let logs = std::fs::read_to_string(log_path).expect("read logs");
        assert_eq!(logs.matches("sync request rejected").count(), 2);
        assert!(logs.contains("bad_request"));
        assert!(!logs.contains(private_text), "{logs}");
    }

    #[tokio::test]
    async fn sync_handler_preserves_json_rejection_status() {
        let dir = tempfile::tempdir().expect("temp dir");
        let state = AppState {
            max_request_bytes: super::DEFAULT_MAX_REQUEST_BYTES,
            storage: Arc::new(Mutex::new(
                Storage::open(dir.path().join("server.db")).expect("open server"),
            )),
        };
        let request = Request::post("/sync")
            .body(Body::from("{}"))
            .expect("build request");
        let rejection = Json::<crate::sync_protocol::SyncRequest>::from_request(request, &())
            .await
            .expect_err("missing content type is rejected");
        assert_eq!(rejection.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

        let (status, body) = sync(State(state), Extension(test_identity()), Err(rejection))
            .await
            .expect_err("JSON rejection is returned");

        assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert!(body.starts_with("invalid sync request:"));
    }

    fn test_identity() -> ClientIdentity {
        ClientIdentity {
            peer_addr: "127.0.0.1:12345"
                .parse::<SocketAddr>()
                .expect("peer address"),
            tls_cn: Some("test-client".to_string()),
        }
    }

    fn run_openssl(dir: &Path, args: &[&str]) {
        let output = Command::new("openssl")
            .current_dir(dir)
            .args(args)
            .output()
            .expect("run openssl");
        assert!(
            output.status.success(),
            "openssl failed\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
