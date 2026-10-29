use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

struct ScriptTestEnv {
    _dir: tempfile::TempDir,
    mock_bin: PathBuf,
    cert_dir: PathBuf,
}

impl ScriptTestEnv {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let mock_bin = dir.path().join("bin");
        let cert_dir = dir.path().join("certs");
        fs::create_dir(&mock_bin).expect("mock bin");
        write_mock_commands(&mock_bin);
        Self {
            _dir: dir,
            mock_bin,
            cert_dir,
        }
    }

    fn command(&self, script: &str) -> Command {
        let mut command = Command::new(repo_root().join(script));
        command.env("PATH", path_with_mock_bin(&self.mock_bin));
        command
    }
}

#[test]
fn bootstrap_generates_five_year_ca_and_server_certificate_with_configured_sans() {
    let env = ScriptTestEnv::new();

    let output = env
        .command("scripts/ct-sync-bootstrap-certs.sh")
        .args([
            "--cert-dir",
            env.cert_dir.to_str().expect("utf8 path"),
            "--server-name",
            "tasks.example.com",
            "--server-name",
            "192.0.2.10",
        ])
        .output()
        .expect("run bootstrap script");
    assert_success("bootstrap", &output);

    assert!(env.cert_dir.join("ca.key").is_file());
    assert!(env.cert_dir.join("ca.crt").is_file());
    assert!(env.cert_dir.join("server.key").is_file());
    assert!(env.cert_dir.join("server.crt").is_file());
    assert!(!env.cert_dir.join("server.csr").exists());

    #[cfg(unix)]
    {
        assert_eq!(file_mode(&env.cert_dir.join("ca.key")), 0o600);
        assert_eq!(file_mode(&env.cert_dir.join("server.key")), 0o640);
        assert_eq!(file_mode(&env.cert_dir.join("server.crt")), 0o644);
    }

    let ca_text = certificate_text(&env.cert_dir.join("ca.crt"));
    assert!(
        ca_text.contains("Subject: CN=couch-tasks sync CA")
            || ca_text.contains("Subject: CN = couch-tasks sync CA"),
        "{ca_text}"
    );
    assert!(ca_text.contains("CA:TRUE"), "{ca_text}");
    assert!(ca_text.contains("Certificate Sign"), "{ca_text}");

    let server_text = certificate_text(&env.cert_dir.join("server.crt"));
    assert!(
        server_text.contains("Subject: CN=couch-tasks sync server")
            || server_text.contains("Subject: CN = couch-tasks sync server"),
        "{server_text}"
    );
    assert!(
        server_text.contains("DNS:tasks.example.com"),
        "{server_text}"
    );
    assert!(
        server_text.contains("IP Address:192.0.2.10"),
        "{server_text}"
    );
    assert!(
        server_text.contains("TLS Web Server Authentication"),
        "{server_text}"
    );
    assert_valid_for_default_five_year_window(&env.cert_dir.join("server.crt"));
}

#[test]
fn bootstrap_rejects_partial_certificate_pairs() {
    let env = ScriptTestEnv::new();
    fs::create_dir(&env.cert_dir).expect("cert dir");
    fs::write(env.cert_dir.join("ca.key"), "not a real key").expect("ca key");

    let output = env
        .command("scripts/ct-sync-bootstrap-certs.sh")
        .args(["--cert-dir", env.cert_dir.to_str().expect("utf8 path")])
        .output()
        .expect("run bootstrap script");

    assert!(!output.status.success(), "bootstrap unexpectedly succeeded");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("found CA key without certificate"),
        "stderr: {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn bootstrap_rejects_symlinked_certificate_material() {
    let env = ScriptTestEnv::new();
    fs::create_dir(&env.cert_dir).expect("cert dir");
    let protected = env._dir.path().join("protected");
    fs::write(&protected, "secret").expect("protected file");
    fs::set_permissions(&protected, fs::Permissions::from_mode(0o600)).expect("chmod protected");
    fs::write(env.cert_dir.join("server.key"), "not a real key").expect("server key");
    std::os::unix::fs::symlink(&protected, env.cert_dir.join("server.crt"))
        .expect("server cert symlink");

    let output = env
        .command("scripts/ct-sync-bootstrap-certs.sh")
        .args(["--cert-dir", env.cert_dir.to_str().expect("utf8 path")])
        .output()
        .expect("run bootstrap script");

    assert!(!output.status.success(), "bootstrap unexpectedly succeeded");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("is not a regular file"), "stderr: {stderr}");
    assert_eq!(file_mode(&protected), 0o600);
    assert_eq!(fs::read_to_string(&protected).expect("read"), "secret");
}

#[cfg(unix)]
#[test]
fn bootstrap_rejects_cert_dir_under_writable_parent() {
    let env = ScriptTestEnv::new();
    let shared = env._dir.path().join("shared");
    fs::create_dir(&shared).expect("shared dir");
    fs::set_permissions(&shared, fs::Permissions::from_mode(0o777)).expect("chmod shared");
    let cert_dir = shared.join("certs");

    let output = env
        .command("scripts/ct-sync-bootstrap-certs.sh")
        .args(["--cert-dir", cert_dir.to_str().expect("utf8 path")])
        .output()
        .expect("run bootstrap script");

    assert!(!output.status.success(), "bootstrap unexpectedly succeeded");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("must be owned by root"), "stderr: {stderr}");
    assert!(!cert_dir.join("ca.key").exists());
}

#[cfg(unix)]
#[test]
fn client_cert_does_not_follow_existing_p12_symlink() {
    let env = ScriptTestEnv::new();
    assert_success(
        "bootstrap",
        &env.command("scripts/ct-sync-bootstrap-certs.sh")
            .args(["--cert-dir", env.cert_dir.to_str().expect("utf8 path")])
            .output()
            .expect("run bootstrap script"),
    );
    let protected = env._dir.path().join("protected");
    fs::write(&protected, "secret").expect("protected file");
    let p12_out = env.cert_dir.join("bundle.p12");
    std::os::unix::fs::symlink(&protected, &p12_out).expect("p12 symlink");
    let password_file = env._dir.path().join("p12-password.txt");
    fs::write(&password_file, "test-password\n").expect("write password file");

    let output = env
        .command("scripts/ct-sync-client-cert.sh")
        .args([
            "--cert-dir",
            env.cert_dir.to_str().expect("utf8 path"),
            "--cn",
            "Alice Example",
            "--p12-out",
            p12_out.to_str().expect("utf8 path"),
            "--p12-password-file",
            password_file.to_str().expect("utf8 path"),
        ])
        .output()
        .expect("run client cert script");

    assert!(
        !output.status.success(),
        "client cert unexpectedly succeeded"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("refusing to overwrite existing file"),
        "{stderr}"
    );
    assert_eq!(fs::read_to_string(&protected).expect("read"), "secret");
}

#[test]
fn client_cert_generates_sanitized_five_year_client_certificate() {
    let env = ScriptTestEnv::new();
    assert_success(
        "bootstrap",
        &env.command("scripts/ct-sync-bootstrap-certs.sh")
            .args(["--cert-dir", env.cert_dir.to_str().expect("utf8 path")])
            .output()
            .expect("run bootstrap script"),
    );

    let output = env
        .command("scripts/ct-sync-client-cert.sh")
        .args([
            "--cert-dir",
            env.cert_dir.to_str().expect("utf8 path"),
            "--cn",
            "Alice Example",
        ])
        .output()
        .expect("run client cert script");
    assert_success("client cert", &output);

    let client_key = env.cert_dir.join("client-alice-example.key");
    let client_cert = env.cert_dir.join("client-alice-example.crt");
    let client_csr = env.cert_dir.join("client-alice-example.csr");
    assert!(client_key.is_file());
    assert!(client_cert.is_file());
    assert!(client_csr.is_file());
    assert!(
        fs::read_dir(&env.cert_dir)
            .expect("read cert dir")
            .all(|entry| !entry
                .expect("dir entry")
                .file_name()
                .to_string_lossy()
                .starts_with(".ct-sync-client.")),
        "staging directory left behind"
    );

    #[cfg(unix)]
    {
        assert_eq!(file_mode(&client_key), 0o600);
        assert_eq!(file_mode(&client_cert), 0o644);
        assert_eq!(file_mode(&client_csr), 0o644);
    }

    let client_text = certificate_text(&client_cert);
    assert!(
        client_text.contains("Subject: CN=Alice Example")
            || client_text.contains("Subject: CN = Alice Example"),
        "{client_text}"
    );
    assert!(
        client_text.contains("TLS Web Client Authentication"),
        "{client_text}"
    );
    assert_valid_for_default_five_year_window(&client_cert);

    let second_output = env
        .command("scripts/ct-sync-client-cert.sh")
        .args([
            "--cert-dir",
            env.cert_dir.to_str().expect("utf8 path"),
            "--cn",
            "Alice Example",
        ])
        .output()
        .expect("run duplicate client cert script");
    assert!(
        !second_output.status.success(),
        "duplicate unexpectedly succeeded"
    );
    let stderr = String::from_utf8_lossy(&second_output.stderr);
    assert!(
        stderr.contains("refusing to overwrite existing file"),
        "{stderr}"
    );
}

#[test]
fn client_cert_can_generate_pkcs12_android_bundle() {
    let env = ScriptTestEnv::new();
    assert_success(
        "bootstrap",
        &env.command("scripts/ct-sync-bootstrap-certs.sh")
            .args(["--cert-dir", env.cert_dir.to_str().expect("utf8 path")])
            .output()
            .expect("run bootstrap script"),
    );
    let password_file = env._dir.path().join("p12-password.txt");
    fs::write(&password_file, "test-password\n").expect("write password file");

    let output = env
        .command("scripts/ct-sync-client-cert.sh")
        .args([
            "--cert-dir",
            env.cert_dir.to_str().expect("utf8 path"),
            "--cn",
            "Alice Example",
            "--p12",
            "--p12-password-file",
            password_file.to_str().expect("utf8 path"),
        ])
        .output()
        .expect("run client cert script");
    assert_success("client cert with p12", &output);

    let client_p12 = env.cert_dir.join("client-alice-example.p12");
    assert!(client_p12.is_file());

    #[cfg(unix)]
    assert_eq!(file_mode(&client_p12), 0o600);

    let output = Command::new("openssl")
        .args([
            "pkcs12",
            "-in",
            client_p12.to_str().expect("utf8 path"),
            "-nokeys",
            "-passin",
            &format!("file:{}", password_file.to_str().expect("utf8 path")),
        ])
        .output()
        .expect("inspect pkcs12 bundle");
    assert_success("openssl pkcs12 -nokeys", &output);
    let bundle_text = String::from_utf8_lossy(&output.stdout);
    assert!(bundle_text.contains("BEGIN CERTIFICATE"), "{bundle_text}");

    let second_output = env
        .command("scripts/ct-sync-client-cert.sh")
        .args([
            "--cert-dir",
            env.cert_dir.to_str().expect("utf8 path"),
            "--cn",
            "Alice Example",
            "--p12",
            "--p12-password-file",
            password_file.to_str().expect("utf8 path"),
        ])
        .output()
        .expect("run duplicate client cert script");
    assert!(
        !second_output.status.success(),
        "duplicate unexpectedly succeeded"
    );
    let stderr = String::from_utf8_lossy(&second_output.stderr);
    assert!(
        stderr.contains("refusing to overwrite existing file"),
        "{stderr}"
    );
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn path_with_mock_bin(mock_bin: &Path) -> String {
    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![mock_bin.to_path_buf()];
    paths.extend(std::env::split_paths(&old_path));
    std::env::join_paths(paths)
        .expect("join PATH")
        .into_string()
        .expect("utf8 PATH")
}

fn write_mock_commands(mock_bin: &Path) {
    write_executable(
        &mock_bin.join("id"),
        r#"#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == "-u" ]]; then
  if [[ "$#" -eq 1 ]]; then
    echo 0
  else
    echo 1000
  fi
else
  /usr/bin/id "$@"
fi
"#,
    );
    write_executable(
        &mock_bin.join("chown"),
        r#"#!/usr/bin/env bash
exit 0
"#,
    );
    // Report directories owned by the unprivileged test user as root-owned so the scripts'
    // trusted-directory checks run against real permission bits.
    let stat = real_command("stat");
    let uid = String::from_utf8(
        Command::new(real_command("id"))
            .arg("-u")
            .output()
            .expect("run id -u")
            .stdout,
    )
    .expect("utf8 uid");
    write_executable(
        &mock_bin.join("stat"),
        &format!(
            r#"#!/usr/bin/env bash
set -euo pipefail
'{stat}' "$@" | sed 's/^{uid} /0 /'
"#,
            stat = stat.display(),
            uid = uid.trim(),
        ),
    );
}

fn real_command(name: &str) -> PathBuf {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| panic!("{name} not found on PATH"))
}

fn write_executable(path: &Path, content: &str) {
    fs::write(path, content).expect("write mock command");
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("chmod mock command");
    }
}

#[cfg(unix)]
fn file_mode(path: &Path) -> u32 {
    fs::metadata(path).expect("metadata").permissions().mode() & 0o777
}

fn certificate_text(path: &Path) -> String {
    let output = Command::new("openssl")
        .args([
            "x509",
            "-in",
            path.to_str().expect("utf8 path"),
            "-noout",
            "-text",
        ])
        .output()
        .expect("inspect certificate");
    assert_success("openssl x509 -text", &output);
    String::from_utf8(output.stdout).expect("utf8 certificate text")
}

fn assert_valid_for_default_five_year_window(path: &Path) {
    let seconds_1824_days = (1824 * 24 * 60 * 60).to_string();
    let output = Command::new("openssl")
        .args([
            "x509",
            "-in",
            path.to_str().expect("utf8 path"),
            "-noout",
            "-checkend",
            &seconds_1824_days,
        ])
        .output()
        .expect("check certificate validity");
    assert_success("openssl x509 -checkend 1824 days", &output);

    let seconds_1826_days = (1826 * 24 * 60 * 60).to_string();
    let output = Command::new("openssl")
        .args([
            "x509",
            "-in",
            path.to_str().expect("utf8 path"),
            "-noout",
            "-checkend",
            &seconds_1826_days,
        ])
        .output()
        .expect("check certificate validity");
    assert!(
        !output.status.success(),
        "certificate should not be valid beyond the default five-year window"
    );
}

fn assert_success(label: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{label} failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
