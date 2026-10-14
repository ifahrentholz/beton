//! `beton proxy ca rotate` gegen einen echten Daemon (PRX-002 AC3, AC4). Alle Tests nutzen den
//! Datei-Store (`secrets.backend: file`), nie den Keychain des Entwicklers.

#![allow(clippy::unwrap_used)]

mod common;

use std::path::Path;

use common::{Serve, beton, run, stderr, stdout};

fn passphrase_file(dir: &Path) -> String {
    let p = dir.join("passphrase");
    std::fs::write(&p, "test-passphrase\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    p.to_str().unwrap().to_owned()
}

fn file_daemon(pass_dir: &Path, unlocked: bool) -> Serve {
    let pass = passphrase_file(pass_dir);
    let mut env = vec![("BETON_CFG_SECRETS__BACKEND", "file")];
    if unlocked {
        env.push(("BETON_SECRETS_PASSPHRASE_FILE", pass.as_str()));
    }
    Serve::start_with(&env)
}

fn rotate_json(home: &Path) -> serde_json::Value {
    let o = run(beton(home).args(["--json", "proxy", "ca", "rotate"]));
    assert!(o.status.success(), "{}", stderr(&o));
    serde_json::from_str(&stdout(&o)).unwrap()
}

#[test]
fn prx_002_ac4_cli_rotate_creates_new_ca_and_audit_entries() {
    let pass_dir = tempfile::tempdir().unwrap();
    let serve = file_daemon(pass_dir.path(), true);
    let home = serve.home();

    let o = run(beton(home).args(["proxy", "ca", "rotate"]));
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stderr(&o).contains("beton egress CA"), "{}", stderr(&o));

    let first = rotate_json(home);
    let second = rotate_json(home);
    let fp = |v: &serde_json::Value| v["fingerprint_sha256"].as_str().unwrap().to_owned();
    assert_eq!(fp(&first).len(), 64);
    assert_ne!(fp(&first), fp(&second));
    assert!(
        first["subject"]
            .as_str()
            .unwrap()
            .starts_with("beton egress CA nod_")
    );
    assert!(first.get("key").is_none() && first.get("value").is_none());

    let o = run(beton(home).args(["--json", "audit", "list", "--action", "proxy.ca_rotated"]));
    assert!(o.status.success(), "{}", stderr(&o));
    let entries: Vec<serde_json::Value> = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(entries.len(), 3, "{entries:?}");
    assert_eq!(entries[0]["details"]["fingerprint_sha256"], fp(&second));
    let o = run(beton(home).args(["audit", "verify"]));
    assert!(o.status.success(), "{}", stderr(&o));

    // PRX-002 AC3: Der CA-Schlüssel liegt nirgends unter dem Datenverzeichnis im Klartext.
    assert_no_pem_keys(home);
}

#[test]
fn prx_002_cli_rotate_with_locked_store_fails_closed() {
    let pass_dir = tempfile::tempdir().unwrap();
    let serve = file_daemon(pass_dir.path(), false);
    let o = run(beton(serve.home()).args(["proxy", "ca", "rotate"]));
    assert_eq!(o.status.code(), Some(1), "{}", stderr(&o));
    assert!(stderr(&o).contains("secrets_locked"), "{}", stderr(&o));
}

fn assert_no_pem_keys(dir: &Path) {
    let mut stack = vec![dir.to_path_buf()];
    let mut files = 0;
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(bytes) = std::fs::read(&path) {
                files += 1;
                assert!(
                    !bytes.windows(11).any(|w| w == b"PRIVATE KEY"),
                    "PEM-Schlüssel im Klartext in {}",
                    path.display()
                );
            }
        }
    }
    assert!(files > 0);
}
