//! `beton secrets` und `beton audit` (SEC-002, SEC-003, SEC-004, SEC-012) gegen einen echten
//! Daemon. Ohne `BETON_TEST_KEYCHAIN=1` nutzen alle Tests den Datei-Store
//! (`secrets.backend: file`), damit nie der Keychain des Entwicklers berührt wird.

#![allow(
    clippy::disallowed_methods,
    reason = "Testaufbau startet Prozesse direkt (SBX-001 gilt für Harness und Tools)"
)]
#![allow(clippy::unwrap_used)]

mod common;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use common::{Serve, beton, run, stderr, stdout};

const VALUE: &str = "MARKERsk-proj-Zq9xW3vB7nT1kL5mP0aR";

/// Passphrase-Datei (0600) für den Headless-Betrieb.
fn passphrase_file(dir: &Path) -> PathBuf {
    let p = dir.join("passphrase");
    std::fs::write(&p, "test-passphrase\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    p
}

struct Daemon {
    serve: Serve,
    _pass_dir: tempfile::TempDir,
}

fn file_daemon(unlocked: bool) -> Daemon {
    let pass_dir = tempfile::tempdir().unwrap();
    let pass = passphrase_file(pass_dir.path());
    let pass = pass.to_str().unwrap().to_owned();
    let mut env = vec![("BETON_CFG_SECRETS__BACKEND", "file")];
    if unlocked {
        env.push(("BETON_SECRETS_PASSPHRASE_FILE", pass.as_str()));
    }
    Daemon {
        serve: Serve::start_with(&env),
        _pass_dir: pass_dir,
    }
}

fn with_stdin(cmd: &mut Command, input: &str) -> Output {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn assert_no_value(o: &Output) {
    let text = format!("{}{}", stdout(o), stderr(o));
    for part in ["MARKER", "Zq9x", "0aR", "sk-proj"] {
        assert!(
            !text.contains(part),
            "Wert(teil) `{part}` in der Ausgabe: {text}"
        );
    }
}

#[test]
fn sec_004_ac2_value_as_argument_is_rejected() {
    let home = tempfile::tempdir().unwrap();
    for args in [
        vec!["secrets", "add", "github", VALUE],
        vec!["secrets", "add", "github", "--value", VALUE],
        vec!["secrets", "add", "github", &format!("--token={VALUE}")],
        vec![
            "secrets",
            "rotate",
            "secret://user/github.com/default",
            VALUE,
        ],
    ] {
        let o = run(beton(home.path()).args(&args));
        assert_eq!(o.status.code(), Some(2), "{args:?}: {}", stderr(&o));
        assert!(
            stderr(&o).contains("stdin"),
            "Hinweis auf stdin: {}",
            stderr(&o)
        );
        assert_no_value(&o);
    }
    // Abgelehnt, bevor ein Daemon gefragt wird: nichts angelegt.
    assert!(!home.path().join("beton.db").exists());
}

#[test]
fn sec_004_ac1_add_list_rotate_rm_without_values() {
    let d = file_daemon(true);
    let home = d.serve.home();
    let o = with_stdin(
        beton(home).args(["secrets", "add", "openai", "--stdin"]),
        &format!("{VALUE}\n"),
    );
    assert!(o.status.success(), "{}", stderr(&o));
    assert_no_value(&o);
    assert!(
        stderr(&o).contains("secret://user/openai/default"),
        "{}",
        stderr(&o)
    );

    let o = run(beton(home).args(["secrets", "list"]));
    assert!(o.status.success(), "{}", stderr(&o));
    assert_no_value(&o);
    let out = stdout(&o);
    for col in ["REF", "TYP", "PROVIDER", "ACCOUNT", "ZULETZT GENUTZT"] {
        assert!(out.contains(col), "Spalte {col}: {out}");
    }
    assert!(out.contains("secret://user/openai/default"), "{out}");
    assert!(out.contains("token"), "{out}");
    assert!(out.contains("nie"), "noch nie genutzt: {out}");

    let o = run(beton(home).args(["--json", "secrets", "list"]));
    assert_no_value(&o);
    let json: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(json["items"][0]["provider"], "openai");
    assert_eq!(json["store"]["backend"], "file");

    // Werte mit Account und Name, aus einer Datei.
    let file = d.serve.work.path().join("token.txt");
    std::fs::write(&file, format!("{VALUE}-zwei")).unwrap();
    let o = run(beton(home).args([
        "secrets",
        "add",
        "github",
        "--account",
        "ifahrentholz",
        "--from-file",
        file.to_str().unwrap(),
    ]));
    assert!(o.status.success(), "{}", stderr(&o));
    assert_no_value(&o);

    let o = with_stdin(
        beton(home).args([
            "secrets",
            "rotate",
            "secret://user/openai/default",
            "--stdin",
        ]),
        "neuer-wert",
    );
    assert!(o.status.success(), "{}", stderr(&o));

    let o = run(beton(home).args(["secrets", "rm", "secret://user/openai/default"]));
    assert!(o.status.success(), "{}", stderr(&o));
    let o = run(beton(home).args(["secrets", "rm", "secret://user/openai/default"]));
    assert_eq!(o.status.code(), Some(1), "{}", stderr(&o));
    let o = run(beton(home).args(["--json", "secrets", "list"]));
    let json: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(json["items"].as_array().unwrap().len(), 1);

    // SEC-002 AC1 sinngemäß für den Datei-Store: kein Klartext unter dem Datenverzeichnis.
    assert_no_plaintext_under(home);
}

fn assert_no_plaintext_under(dir: &Path) {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(bytes) = std::fs::read(&path) {
                assert!(
                    !bytes.windows(6).any(|w| w == b"MARKER"),
                    "Klartext in {}",
                    path.display()
                );
            }
        }
    }
}

#[test]
fn sec_003_locked_store_is_unlocked_via_cli() {
    let d = file_daemon(false);
    let home = d.serve.home();
    let o = with_stdin(
        beton(home).args(["secrets", "add", "openai", "--stdin"]),
        "wert",
    );
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("secrets_locked"), "{}", stderr(&o));
    assert!(
        stderr(&o).contains("beton secrets unlock"),
        "{}",
        stderr(&o)
    );

    let o = with_stdin(
        beton(home).args(["secrets", "unlock", "--stdin"]),
        "pass-1\n",
    );
    assert!(o.status.success(), "{}", stderr(&o));
    let o = with_stdin(
        beton(home).args(["secrets", "add", "openai", "--stdin"]),
        "wert",
    );
    assert!(o.status.success(), "{}", stderr(&o));

    // Passphrase-Wechsel: alte und neue Passphrase zeilenweise auf stdin.
    let o = with_stdin(
        beton(home).args(["secrets", "unlock", "--change-passphrase", "--stdin"]),
        "pass-1\npass-2\n",
    );
    assert!(o.status.success(), "{}", stderr(&o));
    let o = with_stdin(
        beton(home).args(["secrets", "unlock", "--stdin"]),
        "pass-1\n",
    );
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("secrets_locked"), "{}", stderr(&o));
    assert!(!stderr(&o).contains("pass-1"));
}

#[tokio::test]
async fn sec_012_ac1_audit_list_export_verify() {
    let d = file_daemon(true);
    let home = d.serve.home().to_path_buf();
    for account in ["a", "b"] {
        let o = with_stdin(
            beton(&home).args(["secrets", "add", "npm", "--account", account, "--stdin"]),
            VALUE,
        );
        assert!(o.status.success(), "{}", stderr(&o));
    }
    let o = run(beton(&home).args(["audit", "list"]));
    assert!(o.status.success(), "{}", stderr(&o));
    assert_no_value(&o);
    assert!(stdout(&o).contains("secret.created"), "{}", stdout(&o));

    let o = run(beton(&home).args(["audit", "export"]));
    assert!(o.status.success(), "{}", stderr(&o));
    assert_no_value(&o);
    let lines: Vec<serde_json::Value> = stdout(&o)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(lines.len() >= 3, "{lines:?}");
    // Älteste zuerst, mit Hash-Kette.
    assert_eq!(lines[0]["seq"], 1);
    assert_eq!(lines[1]["prev_hash"], lines[0]["hash"]);

    let o = run(beton(&home).args(["audit", "verify"]));
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stdout(&o).contains("intakt"), "{}", stdout(&o));

    // Nachträglich in der DB geänderte Zeile: verify nennt die erste ungültige Position.
    let db = home.join("beton.db");
    let mut conn = <sqlx::SqliteConnection as sqlx::Connection>::connect(&format!(
        "sqlite://{}",
        db.display()
    ))
    .await
    .unwrap();
    sqlx::query("UPDATE audit_entries SET target_id = 'secret://user/npm/x' WHERE seq = 2")
        .execute(&mut conn)
        .await
        .unwrap();
    let o = run(beton(&home).args(["audit", "verify"]));
    assert_eq!(o.status.code(), Some(1), "{}", stdout(&o));
    assert!(stdout(&o).contains("Position 2"), "{}", stdout(&o));
    let o = run(beton(&home).args(["--json", "audit", "verify"]));
    let json: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(json["ok"], false);
    assert_eq!(json["first_invalid"], 2);
}

#[tokio::test]
async fn prx_009_ac4_credential_audit_entries_via_audit_list_kind_credential() {
    let d = file_daemon(true);
    let home = d.serve.home().to_path_buf();
    let o = with_stdin(
        beton(&home).args(["secrets", "add", "github", "--stdin"]),
        VALUE,
    );
    assert!(o.status.success(), "{}", stderr(&o));
    // Ein abgelaufenes Nutzungsfenster des Proxys (wie `credential.used` aus dem Runner) und
    // ein Fehlgebrauch.
    let db = home.join("beton.db");
    let mut conn = <sqlx::SqliteConnection as sqlx::Connection>::connect(&format!(
        "sqlite://{}",
        db.display()
    ))
    .await
    .unwrap();
    let (org, actor): (String, String) =
        sqlx::query_as("SELECT org_id, actor_id FROM audit_entries LIMIT 1")
            .fetch_one(&mut conn)
            .await
            .unwrap();
    sqlx::query(
        "INSERT INTO audit_usage_windows (org_id, secret_ref, session_id, host_id, actor_id, \
         purpose, window_start, last_at, count) VALUES (?, 'credential:github', '', \
         'api.github.com', ?, 'proxy', '2026-01-01T00:00:00Z', '2026-01-01T00:01:00Z', 42)",
    )
    .bind(&org)
    .bind(&actor)
    .execute(&mut conn)
    .await
    .unwrap();
    drop(conn);
    let o = run(beton(&home).args(["--json", "audit", "list", "--kind", "credential"]));
    assert!(o.status.success(), "{}", stderr(&o));
    assert_no_value(&o);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0]["action"], "credential.used");
    assert_eq!(items[0]["target_kind"], "credential");
    assert_eq!(items[0]["target_id"], "github");
    assert_eq!(items[0]["host_id"], "api.github.com");
    assert_eq!(items[0]["count"], 42);
    // Ohne Filter auch die Secret-Einträge.
    let o = run(beton(&home).args(["audit", "list"]));
    assert!(stdout(&o).contains("secret.created") && stdout(&o).contains("credential.used"));
    let o = run(beton(&home).args(["audit", "list", "--kind", "credential"]));
    assert!(!stdout(&o).contains("secret.created"), "{}", stdout(&o));
}

/// SEC-002 AC1 und AC3 mit echtem Keychain: nur mit `BETON_TEST_KEYCHAIN=1` (macOS-CI mit
/// temporärem Keychain als Standard); lokal übersprungen.
#[test]
fn sec_002_ac1_ac3_value_lives_in_the_keychain_and_rm_removes_it() {
    if std::env::var("BETON_TEST_KEYCHAIN").as_deref() != Ok("1") {
        eprintln!("übersprungen: BETON_TEST_KEYCHAIN=1 fehlt (nur CI mit temporärem Keychain)");
        return;
    }
    let serve = Serve::start_with(&[("BETON_CFG_SECRETS__BACKEND", "keychain")]);
    let home = serve.home();
    let account = format!("ci-{}", std::process::id());
    let r = format!("secret://user/github.com/{account}");
    let o = with_stdin(
        beton(home).args(["secrets", "add", "github", "--account", &account, "--stdin"]),
        VALUE,
    );
    assert!(o.status.success(), "{}", stderr(&o));
    let o = run(beton(home).args(["--json", "secrets", "list"]));
    let json: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(json["items"][0]["backend"], "keychain");
    if let Some(found) = in_temp_keychain(&r) {
        assert!(found, "Wert liegt im Keychain");
    }
    assert_no_plaintext_under(home);

    let o = run(beton(home).args(["secrets", "rm", &r]));
    assert!(o.status.success(), "{}", stderr(&o));
    if let Some(found) = in_temp_keychain(&r) {
        assert!(!found, "Keychain-Eintrag entfernt");
    }
    let o = run(beton(home).args(["--json", "secrets", "list"]));
    let json: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert!(
        json["items"].as_array().unwrap().is_empty(),
        "Metadaten entfernt"
    );
}

fn in_temp_keychain(account: &str) -> Option<bool> {
    let path = std::env::var("BETON_TEST_KEYCHAIN_PATH").ok()?;
    let status = Command::new("security")
        .args(["find-generic-password", "-s", "beton", "-a", account, &path])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    Some(status.success())
}
