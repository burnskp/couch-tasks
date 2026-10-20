use std::{fs, path::PathBuf};

#[test]
fn default_nix_package_contains_sync_server_and_certificate_helpers() {
    let flake = fs::read_to_string(repo_root().join("flake.nix")).expect("read flake.nix");

    assert!(
        flake.contains("cargoBuildFlags = [ \"--bins\" ];"),
        "default package should build both ct and ct-sync-server"
    );
    assert!(
        flake.contains("cargoInstallFlags = [ \"--bins\" ];"),
        "default package should install both ct and ct-sync-server"
    );
    assert!(
        flake.contains("postInstall = installSyncCertificateScripts + ''"),
        "default package should include NixOS activation certificate helpers"
    );
}

#[test]
fn sync_server_module_keeps_certificates_out_of_service_writable_state() {
    let flake = fs::read_to_string(repo_root().join("flake.nix")).expect("read flake.nix");

    assert!(
        flake.contains("stateDir = \"/var/lib/ct-sync\";"),
        "module should continue to use the documented state directory"
    );
    assert!(
        flake.contains("dbDir = \"${stateDir}/db\";"),
        "database files should live in a dedicated service-writable db directory"
    );
    assert!(
        flake.contains("databasePath = \"${dbDir}/tasks.db\";"),
        "ct-sync-server should store SQLite files under the db directory"
    );
    assert!(
        flake.contains("\"d ${stateDir} 0750 root ${group} - -\""),
        "state directory should be root-owned so the service cannot replace certs/"
    );
    assert!(
        flake.contains("\"d ${dbDir} 0750 ${user} ${group} - -\""),
        "only the db directory should be service-writable"
    );
    assert!(
        flake.contains("\"d ${certDir} 0750 root ${group} - -\""),
        "certificate directory should remain root-owned"
    );
    assert!(
        flake.contains("ReadWritePaths = [ dbDir ];"),
        "systemd sandbox should grant write access only to the database directory"
    );
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
