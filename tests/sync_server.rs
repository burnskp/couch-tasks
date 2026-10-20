use assert_cmd::prelude::*;
use predicates::prelude::*;
use reqwest::{Certificate, Identity, blocking::Client};
use std::{
    fs,
    net::TcpListener,
    path::Path,
    process::{Child, Command},
    thread,
    time::Duration,
};

struct ServerProcess {
    child: Child,
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn sync_server_requires_mtls_and_accepts_valid_client_certificate() {
    let dir = tempfile::tempdir().expect("temp dir");
    generate_certificates(dir.path());
    let port = free_port();
    let url = format!("https://localhost:{port}");
    let server_db = dir.path().join("server.db");
    let client_db = dir.path().join("client.db");

    let server = start_server(dir.path(), port, &server_db);
    wait_for_server(&url, dir.path());

    Command::cargo_bin("ct")
        .expect("ct binary")
        .env("CT_DB", &client_db)
        .args(["quick", "write", "sync", "test"])
        .assert()
        .success();

    Command::cargo_bin("ct")
        .expect("ct binary")
        .env("CT_DB", &client_db)
        .args([
            "sync",
            "--url",
            &url,
            "--cert",
            &dir.path().join("client.crt").display().to_string(),
            "--key",
            &dir.path().join("client.key").display().to_string(),
            "--ca",
            &dir.path().join("ca.crt").display().to_string(),
        ])
        .assert()
        .success()
        .stdout("");

    Command::cargo_bin("ct")
        .expect("ct binary")
        .env("CT_DB", &client_db)
        .args([
            "sync",
            "-v",
            "--url",
            &url,
            "--cert",
            &dir.path().join("client.crt").display().to_string(),
            "--key",
            &dir.path().join("client.key").display().to_string(),
            "--ca",
            &dir.path().join("ca.crt").display().to_string(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Synced to checkpoint"));

    let mismatched_url = format!("https://127.0.0.1:{port}");
    Command::cargo_bin("ct")
        .expect("ct binary")
        .env("CT_DB", &client_db)
        .args([
            "sync",
            "--url",
            &mismatched_url,
            "--cert",
            &dir.path().join("client.crt").display().to_string(),
            "--key",
            &dir.path().join("client.key").display().to_string(),
            "--ca",
            &dir.path().join("ca.crt").display().to_string(),
        ])
        .assert()
        .code(4)
        .stderr(predicate::str::contains("Caused by:"))
        .stderr(predicate::str::contains("certificate"));

    let unauthenticated = Client::builder()
        .add_root_certificate(
            Certificate::from_pem(&fs::read(dir.path().join("ca.crt")).unwrap()).unwrap(),
        )
        .build()
        .unwrap()
        .get(format!("{url}/healthz"))
        .send();
    assert!(unauthenticated.is_err());

    let untrusted = tempfile::tempdir().unwrap();
    generate_certificates(untrusted.path());
    fs::copy(dir.path().join("ca.crt"), untrusted.path().join("ca.crt")).unwrap();
    sync_command(untrusted.path(), &client_db, &url)
        .assert()
        .code(4)
        .stderr(predicate::str::contains("Sync error"));

    drop(server);
}

// A one-request mTLS endpoint lets tests pause between accepting a snapshot
// and sending the response, without sleeps or timing-dependent mutations.
fn serve_one_tls_request(
    dir: &Path,
    handler: impl FnOnce(couch_tasks::sync_protocol::SyncRequest) -> String + Send + 'static,
) -> (String, thread::JoinHandle<()>) {
    use rustls::{
        RootCertStore,
        pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
    };
    use std::{
        io::{Read, Write},
        sync::Arc,
    };

    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let certs = CertificateDer::pem_file_iter(dir.join("server.crt"))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let key = PrivateKeyDer::from_pem_file(dir.join("server.key")).unwrap();
    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from_pem_file(dir.join("ca.crt")).unwrap())
        .unwrap();
    let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(roots))
        .build()
        .unwrap();
    let config = rustls::ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(certs, key)
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "https://localhost:{}",
        listener.local_addr().unwrap().port()
    );
    listener.set_nonblocking(true).unwrap();
    let worker = thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let socket = loop {
            match listener.accept() {
                Ok((socket, _)) => break socket,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "client never connected"
                    );
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("accept: {error}"),
            }
        };
        socket
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        socket
            .set_write_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let connection = rustls::ServerConnection::new(Arc::new(config)).unwrap();
        let mut stream = rustls::StreamOwned::new(connection, socket);
        let mut headers = Vec::new();
        while !headers.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            headers.push(byte[0]);
            assert!(headers.len() < 16_384);
        }
        let headers = String::from_utf8(headers).unwrap();
        assert!(headers.starts_with("POST /sync HTTP/1.1"));
        let length: usize = headers
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .unwrap()
            .1
            .trim()
            .parse()
            .unwrap();
        let mut body = vec![0; length];
        stream.read_exact(&mut body).unwrap();
        let response = handler(serde_json::from_slice(&body).unwrap());
        stream.write_all(response.as_bytes()).unwrap();
        stream.flush().unwrap();
    });
    (url, worker)
}

fn sync_command(dir: &Path, db: &Path, url: &str) -> Command {
    let mut command = Command::cargo_bin("ct").unwrap();
    command.env("CT_DB", db).env("XDG_CONFIG_HOME", dir).args([
        "sync",
        "--url",
        url,
        "--cert",
        dir.join("client.crt").to_str().unwrap(),
        "--key",
        dir.join("client.key").to_str().unwrap(),
        "--ca",
        dir.join("ca.crt").to_str().unwrap(),
    ]);
    command
}

fn json_response(value: &impl serde::Serialize) -> String {
    let body = serde_json::to_string(value).unwrap();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

#[test]
fn http_sync_is_rejected_before_connecting_or_reading_certificates() {
    let dir = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    // No certificate files exist: HTTPS validation must happen first.
    sync_command(dir.path(), &dir.path().join("client.db"), &url)
        .assert()
        .code(1)
        .stderr(predicate::str::contains("must use HTTPS"));
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn sync_never_follows_redirects() {
    let dir = tempfile::tempdir().unwrap();
    generate_certificates(dir.path());
    for status in [301, 302, 303, 307, 308] {
        for scheme in ["http", "https"] {
            let destination = TcpListener::bind("127.0.0.1:0").unwrap();
            destination.set_nonblocking(true).unwrap();
            let location = format!("{scheme}://{}/sync", destination.local_addr().unwrap());
            let (url, worker) = serve_one_tls_request(dir.path(), move |_| {
                format!(
                    "HTTP/1.1 {status} Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
            });
            sync_command(dir.path(), &dir.path().join("client.db"), &url)
                .assert()
                .code(4)
                .stderr(predicate::str::contains(format!(
                    "server returned {status}"
                )));
            worker.join().unwrap();
            assert_eq!(
                destination.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
        }
    }
}

#[test]
fn edit_during_mtls_sync_is_preserved_and_explicit_retry_succeeds() {
    use couch_tasks::storage::Storage;
    let dir = tempfile::tempdir().unwrap();
    generate_certificates(dir.path());
    let client_path = dir.path().join("client.db");
    let server_path = dir.path().join("server.db");
    let client = Storage::open(client_path.clone()).unwrap();
    let task = client.add_task("Work", "original").unwrap();
    let concurrent_path = client_path.clone();
    let remote_path = server_path.clone();
    let (changed_tx, changed_rx) = std::sync::mpsc::channel();
    let (url, worker) = serve_one_tls_request(dir.path(), move |request| {
        let server = Storage::open(remote_path).unwrap();
        let response = server.apply_server_sync(&request).unwrap();
        let other = Storage::open(concurrent_path).unwrap();
        other
            .update_task_details(task.id, "Work", "edited while syncing", "new notes")
            .unwrap();
        changed_tx
            .send(other.export_sync_request().unwrap())
            .unwrap();
        json_response(&response)
    });
    sync_command(dir.path(), &client_path, &url)
        .assert()
        .code(4)
        .stderr(predicate::str::contains("Local data changed during sync"));
    worker.join().unwrap();
    assert_eq!(
        client.export_sync_request().unwrap(),
        changed_rx.recv().unwrap()
    );

    let (retry_url, retry_worker) = serve_one_tls_request(dir.path(), move |request| {
        let server = Storage::open(server_path).unwrap();
        json_response(&server.apply_server_sync(&request).unwrap())
    });
    sync_command(dir.path(), &client_path, &retry_url)
        .assert()
        .success();
    retry_worker.join().unwrap();
    let current = client.export_sync_request().unwrap();
    assert_eq!(current.tasks[0].title, "edited while syncing");
    assert_eq!(current.tasks[0].notes, "new notes");
    assert!(!current.tasks[0].sync_dirty);
    assert!(current.checkpoint > 0);
}

fn start_server(dir: &Path, port: u16, db: &Path) -> ServerProcess {
    start_server_with_limit(dir, port, db, None, None)
}

fn start_server_with_limit(
    dir: &Path,
    port: u16,
    db: &Path,
    flag: Option<&str>,
    env_limit: Option<&str>,
) -> ServerProcess {
    let mut command = Command::cargo_bin("ct-sync-server").expect("server binary");
    command.env_remove("CT_SYNC_MAX_REQUEST_BYTES");
    if let Some(limit) = flag {
        command.args(["--max-request-bytes", limit]);
    }
    if let Some(limit) = env_limit {
        command.env("CT_SYNC_MAX_REQUEST_BYTES", limit);
    }
    let child = command
        .env("RUST_LOG", "off")
        .args([
            "--bind",
            &format!("127.0.0.1:{port}"),
            "--db",
            &db.display().to_string(),
            "--cert",
            &dir.join("server.crt").display().to_string(),
            "--key",
            &dir.join("server.key").display().to_string(),
            "--client-ca",
            &dir.join("ca.crt").display().to_string(),
        ])
        .spawn()
        .expect("start server");
    ServerProcess { child }
}

fn authenticated_client(dir: &Path) -> Client {
    let mut identity_pem = fs::read(dir.join("client.crt")).expect("client cert");
    identity_pem.extend_from_slice(&fs::read(dir.join("client.key")).expect("client key"));
    Client::builder()
        .identity(Identity::from_pem(&identity_pem).expect("identity"))
        .add_root_certificate(
            Certificate::from_pem(&fs::read(dir.join("ca.crt")).unwrap()).unwrap(),
        )
        .build()
        .expect("client")
}

fn wait_for_server(url: &str, dir: &Path) {
    let client = authenticated_client(dir);
    for _ in 0..50 {
        if client
            .get(format!("{url}/healthz"))
            .send()
            .and_then(|response| response.error_for_status())
            .is_ok()
        {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("server did not become ready");
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind free port")
        .local_addr()
        .expect("local addr")
        .port()
}

fn generate_certificates(dir: &Path) {
    run_openssl(
        dir,
        &[
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-keyout",
            "ca.key",
            "-out",
            "ca.crt",
            "-subj",
            "/CN=Couch Tasks Test CA",
            "-days",
            "1",
        ],
    );
    run_openssl(
        dir,
        &[
            "req",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-keyout",
            "server.key",
            "-out",
            "server.csr",
            "-subj",
            "/CN=localhost",
            "-addext",
            "subjectAltName=DNS:localhost",
        ],
    );
    run_openssl(
        dir,
        &[
            "x509",
            "-req",
            "-in",
            "server.csr",
            "-CA",
            "ca.crt",
            "-CAkey",
            "ca.key",
            "-CAcreateserial",
            "-out",
            "server.crt",
            "-days",
            "1",
            "-copy_extensions",
            "copy",
        ],
    );
    // Explicit extensions ensure a v3 client certificate on OpenSSL 3.0 too.
    // Without them, its default v1 certificate is rejected by Rustls.
    run_openssl(
        dir,
        &[
            "req",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-keyout",
            "client.key",
            "-out",
            "client.csr",
            "-subj",
            "/CN=couch-tasks-test-client",
            "-addext",
            "basicConstraints=critical,CA:FALSE",
            "-addext",
            "keyUsage=critical,digitalSignature,keyEncipherment",
            "-addext",
            "extendedKeyUsage=clientAuth",
        ],
    );
    run_openssl(
        dir,
        &[
            "x509",
            "-req",
            "-in",
            "client.csr",
            "-CA",
            "ca.crt",
            "-CAkey",
            "ca.key",
            "-CAcreateserial",
            "-out",
            "client.crt",
            "-days",
            "1",
            "-copy_extensions",
            "copy",
        ],
    );
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

#[test]
fn large_snapshots_sync_and_configured_limits_fail_atomically_then_recover() {
    use couch_tasks::{
        storage::Storage,
        sync_client::{self, SyncClientConfig},
    };
    let dir = tempfile::tempdir().unwrap();
    generate_certificates(dir.path());
    let server_db = dir.path().join("server.db");
    let client = Storage::open(dir.path().join("client.db")).unwrap();
    let title = "large snapshot\x1b]0;test\x07";
    let task = client.add_task("inbox", title).unwrap();
    let notes = format!("{}\nraw\rnotes\x1b[2J", "x".repeat(2100 * 1024));
    client
        .update_task_details(task.id, "inbox", title, &notes)
        .unwrap();
    let port = free_port();
    let mut config = SyncClientConfig {
        url: format!("https://localhost:{port}"),
        cert: dir.path().join("client.crt"),
        key: dir.path().join("client.key"),
        ca: dir.path().join("ca.crt"),
    };
    let server = start_server(dir.path(), port, &server_db);
    wait_for_server(&config.url, dir.path());
    sync_client::sync(&client, &config).unwrap();
    let stored = Storage::open(server_db.clone()).unwrap();
    assert_eq!(
        stored.get_task_by_uuid(&task.uuid).unwrap().unwrap().notes,
        notes
    );
    let other_client = Storage::open(dir.path().join("other-client.db")).unwrap();
    sync_client::sync(&other_client, &config).unwrap();
    let received = other_client.get_task_by_uuid(&task.uuid).unwrap().unwrap();
    assert_eq!(received.title, title);
    assert_eq!(received.notes, notes);
    drop(server);

    client.append_note(task.id, "unsynced change").unwrap();
    let before_client = client.export_sync_request().unwrap();
    let before_server = stored.export_sync_request().unwrap();
    let port = free_port();
    config.url = format!("https://localhost:{port}");
    let server = start_server_with_limit(dir.path(), port, &server_db, None, Some("1024"));
    wait_for_server(&config.url, dir.path());
    let error = sync_client::sync(&client, &config).unwrap_err();
    assert_eq!(error.exit_code(), 4);
    let error = error.to_string();
    for detail in [
        "413",
        "1024 bytes",
        "--max-request-bytes",
        "CT_SYNC_MAX_REQUEST_BYTES",
        "maxRequestBytes",
    ] {
        assert!(error.contains(detail), "{error}");
    }
    assert_eq!(client.export_sync_request().unwrap(), before_client);
    assert_eq!(stored.export_sync_request().unwrap(), before_server);
    drop(server);

    let port = free_port();
    config.url = format!("https://localhost:{port}");
    // The flag overrides even an invalid environment setting.
    let _server = start_server_with_limit(
        dir.path(),
        port,
        &server_db,
        Some("4194304"),
        Some("invalid"),
    );
    wait_for_server(&config.url, dir.path());
    sync_client::sync(&client, &config).unwrap();
    assert!(!client.export_sync_request().unwrap().tasks[0].sync_dirty);
    assert!(
        stored
            .get_task_by_uuid(&task.uuid)
            .unwrap()
            .unwrap()
            .notes
            .ends_with("unsynced change")
    );
}

#[test]
fn server_rejects_invalid_request_limits() {
    for invalid in ["0", "-1", "invalid", "184467440737095516160"] {
        for from_env in [false, true] {
            let mut command = Command::cargo_bin("ct-sync-server").unwrap();
            command.env_remove("CT_SYNC_MAX_REQUEST_BYTES");
            if from_env {
                command.env("CT_SYNC_MAX_REQUEST_BYTES", invalid);
            } else {
                command.args(["--max-request-bytes", invalid]);
            }
            command.assert().code(1).stderr(predicate::str::contains(
                "expected a positive integer byte count",
            ));
        }
    }
}

#[test]
fn request_limit_counts_bytes_for_fixed_length_and_chunked_uploads() {
    let dir = tempfile::tempdir().unwrap();
    generate_certificates(dir.path());
    let port = free_port();
    let url = format!("https://localhost:{port}");
    let _server = start_server_with_limit(
        dir.path(),
        port,
        &dir.path().join("server.db"),
        Some("1024"),
        None,
    );
    wait_for_server(&url, dir.path());
    let client = authenticated_client(dir.path());
    let mut payload = br#"{"checkpoint":0,"projects":[],"tasks":[],"tombstones":[]}"#.to_vec();
    payload.resize(1024, b' ');
    for chunked in [false, true] {
        for oversized in [false, true] {
            let mut bytes = payload.clone();
            if oversized {
                bytes.push(b' ');
            }
            let body = if chunked {
                reqwest::blocking::Body::new(std::io::Cursor::new(bytes))
            } else {
                reqwest::blocking::Body::from(bytes)
            };
            let response = client
                .post(format!("{url}/sync"))
                .header("Content-Type", "application/json")
                .body(body)
                .send()
                .unwrap();
            if oversized {
                assert_eq!(response.status(), reqwest::StatusCode::PAYLOAD_TOO_LARGE);
                assert!(response.text().unwrap().contains("1024 bytes"));
            } else {
                assert_eq!(response.status(), reqwest::StatusCode::OK);
            }
        }
    }
}
