//! `beton secrets` (SEC-003, SEC-004) und `beton audit` (SEC-012).
//!
//! Werte werden nur eingegeben – verdeckt am Terminal, über stdin oder aus einer Datei – und
//! nie angezeigt. Ein Wert als Argument wird abgelehnt, bevor irgendetwas passiert (er stünde
//! sonst in der Shell-History und in `ps`).

use std::io::{IsTerminal as _, Read as _, Write as _};

use anyhow::Context as _;
use beton_secrets::SecretRef;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::cli::{
    AuditCommand, AuditExportArgs, AuditListArgs, ProxyCaCommand, ProxyCommand, SecretKindArg,
    SecretsAddArgs, SecretsCommand, SecretsRotateArgs, SecretsUnlockArgs, ValueSource,
};
use crate::commands::{Ctx, print_json};
use crate::exit::{CliError, CliResult, Exit};

/// Hinweis bei einem Wert als Argument (SEC-004 AC2); nennt den Wert nicht.
pub const ARGUMENT_REJECTED: &str = "Secret-Werte werden nie als Argument angenommen (sie \
     landeten in der Shell-History und in `ps`). Stattdessen über stdin \
     (`… | beton secrets add <ziel> --stdin`), mit `--from-file DATEI` oder verdeckt am Terminal \
     (ohne Optionen). Nichts wurde gespeichert.";

/// Kurznamen für Provider.
fn provider_alias(name: &str) -> &str {
    match name {
        "github" => "github.com",
        "gitlab" => "gitlab.com",
        other => other,
    }
}

/// SecretRef aus Ziel (Provider oder Ref), Account und Name.
pub fn resolve_ref(
    target: &str,
    account: Option<&str>,
    name: Option<&str>,
) -> CliResult<SecretRef> {
    if target.starts_with("secret://") {
        if account.is_some() || name.is_some() {
            return Err(CliError::usage(
                "--account und --name nur mit einem Provider, nicht mit einer vollständigen Ref",
            ));
        }
        return target
            .parse()
            .map_err(|e: beton_secrets::Error| CliError::usage(e));
    }
    SecretRef::user(provider_alias(target), account.unwrap_or("default"), name)
        .map_err(CliError::usage)
}

fn trim_newline(mut bytes: Zeroizing<Vec<u8>>) -> Zeroizing<Vec<u8>> {
    while bytes.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
        bytes.pop();
    }
    bytes
}

/// Liest eine Zeile vom Terminal ohne Echo.
#[cfg(unix)]
fn prompt_hidden(prompt: &str) -> CliResult<Zeroizing<Vec<u8>>> {
    use rustix::termios::{LocalModes, OptionalActions, tcgetattr, tcsetattr};

    struct Restore(rustix::termios::Termios);
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = tcsetattr(std::io::stdin(), OptionalActions::Now, &self.0);
        }
    }
    eprint!("{prompt}: ");
    let _ = std::io::stderr().flush();
    let original = tcgetattr(std::io::stdin()).context("Terminal")?;
    let mut hidden = original.clone();
    hidden.local_modes.remove(LocalModes::ECHO);
    hidden.local_modes.insert(LocalModes::ECHONL);
    tcsetattr(std::io::stdin(), OptionalActions::Now, &hidden).context("Terminal")?;
    let _restore = Restore(original);
    let mut line = Zeroizing::new(String::new());
    std::io::stdin().read_line(&mut line).context("stdin")?;
    Ok(trim_newline(Zeroizing::new(line.as_bytes().to_vec())))
}

#[cfg(not(unix))]
fn prompt_hidden(_prompt: &str) -> CliResult<Zeroizing<Vec<u8>>> {
    Err(CliError::usage(
        "Verdeckte Eingabe ist auf diesem System noch nicht verfügbar; Wert über `--stdin` oder `--from-file` übergeben",
    ))
}

/// Liest einen Wert aus der gewählten Quelle; leer ist ein Fehler.
fn read_value(source: &ValueSource, prompt: &str) -> CliResult<Zeroizing<Vec<u8>>> {
    if source.has_argument_value() {
        return Err(CliError::usage(ARGUMENT_REJECTED));
    }
    let bytes = if let Some(path) = &source.from_file {
        trim_newline(Zeroizing::new(
            std::fs::read(path).with_context(|| format!("{} lesen", path.display()))?,
        ))
    } else if source.stdin || !std::io::stdin().is_terminal() {
        let mut buf = Zeroizing::new(Vec::new());
        std::io::stdin().read_to_end(&mut buf).context("stdin")?;
        trim_newline(buf)
    } else {
        prompt_hidden(prompt)?
    };
    if bytes.is_empty() {
        return Err(CliError::usage(
            "Der Wert ist leer; nichts wurde gespeichert",
        ));
    }
    Ok(bytes)
}

fn as_text(bytes: &[u8]) -> CliResult<&str> {
    std::str::from_utf8(bytes).map_err(|_| CliError::usage("Der Wert ist kein UTF-8-Text"))
}

pub async fn run(ctx: &Ctx, cmd: SecretsCommand) -> CliResult {
    match cmd {
        SecretsCommand::Add(args) => add(ctx, args).await,
        SecretsCommand::List => list(ctx).await,
        SecretsCommand::Rm(args) => {
            let r = resolve_ref(&args.secret_ref, None, None)?;
            let client = crate::daemon::ensure_running(ctx).await?;
            client.delete_secret(&r.to_string()).await?;
            done(
                ctx,
                json!({ "secret_ref": r.to_string(), "deleted": true }),
                &format!("{r} entfernt"),
            )
        }
        SecretsCommand::Rotate(args) => rotate(ctx, args).await,
        SecretsCommand::Test(args) => test(ctx, args).await,
        SecretsCommand::Unlock(args) => unlock(ctx, args).await,
    }
}

fn done(ctx: &Ctx, json: Value, text: &str) -> CliResult {
    if ctx.global.json {
        print_json(&json)
    } else {
        ctx.note(text);
        Ok(())
    }
}

async fn add(ctx: &Ctx, args: SecretsAddArgs) -> CliResult {
    // Zuerst: Wert als Argument ablehnen, noch vor jeder anderen Prüfung (SEC-004 AC2).
    if args.source.has_argument_value() {
        return Err(CliError::usage(ARGUMENT_REJECTED));
    }
    let target = match &args.host {
        Some(host) => {
            let valid = !host.is_empty()
                && host
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
            if !valid || args.target.starts_with("secret://") {
                return Err(CliError::usage(
                    "--host erwartet einen Hostnamen (z. B. github.com) und einen Provider statt einer Ref",
                ));
            }
            host.to_ascii_lowercase()
        }
        None => args.target.clone(),
    };
    let r = resolve_ref(&target, args.account.as_deref(), args.name.as_deref())?;
    let (kind, value) = match args.kind {
        SecretKindArg::Token => ("token", read_value(&args.source, "Wert")?),
        SecretKindArg::Basic => {
            let username = args
                .username
                .clone()
                .ok_or_else(|| CliError::usage("--type basic braucht --username"))?;
            let password = read_value(&args.source, "Passwort")?;
            let json = json!({ "username": username, "password": as_text(&password)? });
            ("basic", Zeroizing::new(json.to_string().into_bytes()))
        }
        SecretKindArg::Oauth => ("oauth", read_value(&args.source, "OAuth-JSON")?),
    };
    let client = crate::daemon::ensure_running(ctx).await?;
    let meta = client
        .create_secret(&r.to_string(), kind, as_text(&value)?)
        .await?;
    drop(value);
    let backend = meta["backend"].as_str().unwrap_or_default().to_owned();
    done(ctx, meta, &format!("{r} angelegt ({kind}, {backend})"))
}

/// `beton secrets test` (SEC-004, SEC-011 AC3): Prüfung beim Provider über den Proxy-Pfad des
/// Daemons; Exit 1, wenn der Provider ablehnt oder Scopes fehlen.
async fn test(ctx: &Ctx, args: crate::cli::SecretsTestArgs) -> CliResult {
    let r = resolve_ref(&args.secret_ref, None, None)?;
    let client = crate::daemon::ensure_running(ctx).await?;
    let report = client
        .test_secret(&r.to_string(), args.kind.as_deref())
        .await?;
    let ok = report["ok"].as_bool() == Some(true);
    if ctx.global.json {
        print_json(&report)?;
    } else {
        let list = |k: &str| {
            report[k]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| "–".into())
        };
        println!(
            "{r}: {} (HTTP {}, {})",
            if ok { "gültig" } else { "Problem" },
            report["status"],
            report["provider"].as_str().unwrap_or("?")
        );
        println!("  Scopes: {}", list("scopes"));
        println!("  Fehlende Scopes: {}", list("missing"));
        if let Some(hint) = report["hint"].as_str() {
            println!("  {hint}");
        }
    }
    if ok {
        Ok(())
    } else {
        Err(CliError::new(
            Exit::General,
            anyhow::anyhow!("Secret-Prüfung nicht bestanden"),
        ))
    }
}

async fn rotate(ctx: &Ctx, args: SecretsRotateArgs) -> CliResult {
    if args.source.has_argument_value() {
        return Err(CliError::usage(ARGUMENT_REJECTED));
    }
    let r = resolve_ref(&args.secret_ref, None, None)?;
    let value = read_value(&args.source, "Neuer Wert")?;
    let client = crate::daemon::ensure_running(ctx).await?;
    let meta = client
        .rotate_secret(&r.to_string(), as_text(&value)?)
        .await?;
    drop(value);
    done(ctx, meta, &format!("{r}: neuer Wert gespeichert"))
}

async fn unlock(ctx: &Ctx, args: SecretsUnlockArgs) -> CliResult {
    let (old, new) = if args.stdin || !std::io::stdin().is_terminal() {
        let mut buf = Zeroizing::new(String::new());
        std::io::stdin().read_to_string(&mut buf).context("stdin")?;
        let mut lines = buf.lines();
        let first = Zeroizing::new(lines.next().unwrap_or_default().as_bytes().to_vec());
        let second = args
            .change_passphrase
            .then(|| Zeroizing::new(lines.next().unwrap_or_default().as_bytes().to_vec()));
        (first, second)
    } else if args.change_passphrase {
        let old = prompt_hidden("Alte Passphrase")?;
        let new = prompt_hidden("Neue Passphrase")?;
        let again = prompt_hidden("Neue Passphrase wiederholen")?;
        if *new != *again {
            return Err(CliError::usage(
                "Die neuen Passphrasen stimmen nicht überein",
            ));
        }
        (old, Some(new))
    } else {
        (prompt_hidden("Passphrase")?, None)
    };
    if old.is_empty() || new.as_ref().is_some_and(|n| n.is_empty()) {
        return Err(CliError::usage("Leere Passphrase"));
    }
    let client = crate::daemon::ensure_running(ctx).await?;
    let new_text = match &new {
        Some(n) => Some(as_text(n)?),
        None => None,
    };
    client.unlock_secrets(as_text(&old)?, new_text).await?;
    let text = if new.is_some() {
        "Passphrase gewechselt; der Datei-Store ist neu verschlüsselt"
    } else {
        "Datei-Store entsperrt"
    };
    done(ctx, json!({ "unlocked": true }), text)
}

async fn list(ctx: &Ctx) -> CliResult {
    let client = crate::daemon::ensure_running(ctx).await?;
    let result = client.list_secrets().await?;
    if ctx.global.json {
        return print_json(&result);
    }
    if result["store"]["locked"].as_bool() == Some(true) {
        ctx.note(format!(
            "Secret-Store ({}) gesperrt: beton secrets unlock",
            result["store"]["backend"].as_str().unwrap_or("?")
        ));
    }
    let items = result["items"].as_array().cloned().unwrap_or_default();
    if items.is_empty() {
        ctx.note("Keine Secrets. Neues Secret: beton secrets add <provider>");
        return Ok(());
    }
    let text = |v: &Value, k: &str| v[k].as_str().unwrap_or_default().to_owned();
    let rows: Vec<Vec<String>> = items
        .iter()
        .map(|m| {
            vec![
                text(m, "secret_ref"),
                text(m, "kind"),
                text(m, "provider"),
                text(m, "account"),
                m["last_used_at"]
                    .as_str()
                    .map_or_else(|| "nie".to_owned(), str::to_owned),
            ]
        })
        .collect();
    print_table(
        &["REF", "TYP", "PROVIDER", "ACCOUNT", "ZULETZT GENUTZT"],
        &rows,
    )
}

fn print_table(header: &[&str], rows: &[Vec<String>]) -> CliResult {
    let widths: Vec<usize> = (0..header.len())
        .map(|i| {
            rows.iter()
                .map(|r| r[i].chars().count())
                .chain([header[i].chars().count()])
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut out = std::io::stdout().lock();
    let mut line = |cells: Vec<&str>| -> std::io::Result<()> {
        let last = cells.len() - 1;
        for (i, c) in cells.iter().enumerate() {
            if i == last {
                writeln!(out, "{c}")?;
            } else {
                write!(out, "{c:w$}  ", w = widths[i])?;
            }
        }
        Ok(())
    };
    line(header.to_vec()).context("stdout")?;
    for r in rows {
        line(r.iter().map(String::as_str).collect()).context("stdout")?;
    }
    Ok(())
}

/// `beton proxy ca rotate` (PRX-002 AC4): neue beton-CA über den Daemon; zeigt nur Subject
/// und Fingerabdruck.
pub async fn proxy(ctx: &Ctx, cmd: ProxyCommand) -> CliResult {
    let ProxyCommand::Ca(ProxyCaCommand::Rotate) = cmd;
    let client = crate::daemon::ensure_running(ctx).await?;
    let info = client.rotate_proxy_ca().await?;
    let text = format!(
        "Neue {} (SHA-256 {}, gültig bis {}); neue Sessions nutzen sie, laufende behalten \
         ihre bis zum Neustart",
        info["subject"].as_str().unwrap_or("?"),
        info["fingerprint_sha256"].as_str().unwrap_or("?"),
        info["not_after"].as_str().unwrap_or("?"),
    );
    done(ctx, info, &text)
}

pub async fn audit(ctx: &Ctx, cmd: AuditCommand) -> CliResult {
    let client = crate::daemon::ensure_running(ctx).await?;
    match cmd {
        AuditCommand::List(args) => audit_list(ctx, &client, args).await,
        AuditCommand::Export(args) => audit_export(&client, args).await,
        AuditCommand::Verify => audit_verify(ctx, &client).await,
    }
}

async fn audit_list(ctx: &Ctx, client: &beton_sdk::Client, args: AuditListArgs) -> CliResult {
    let page = client
        .audit_page(
            None,
            args.limit,
            args.action.as_deref(),
            args.kind.as_deref(),
        )
        .await?;
    if ctx.global.json {
        return print_json(&Value::Array(page.items));
    }
    if page.items.is_empty() {
        ctx.note("Keine Audit-Einträge");
        return Ok(());
    }
    let text = |v: &Value, k: &str| v[k].as_str().unwrap_or("-").to_owned();
    let rows: Vec<Vec<String>> = page
        .items
        .iter()
        .map(|e| {
            vec![
                e["seq"].to_string(),
                text(e, "at"),
                text(e, "action"),
                text(e, "target_id"),
                text(e, "session_id"),
                text(e, "host_id"),
                e["count"].to_string(),
            ]
        })
        .collect();
    print_table(
        &["SEQ", "ZEIT", "AKTION", "ZIEL", "SESSION", "HOST", "ANZAHL"],
        &rows,
    )
}

async fn audit_export(client: &beton_sdk::Client, args: AuditExportArgs) -> CliResult {
    let mut all = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page = client
            .audit_page(cursor.as_deref(), 200, None, None)
            .await?;
        all.extend(page.items);
        match page.next_cursor {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    all.reverse();
    let mut text = String::new();
    for e in &all {
        text.push_str(&serde_json::to_string(e).context("JSON")?);
        text.push('\n');
    }
    match args.output {
        Some(path) => {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                options.mode(0o600);
            }
            let mut f = options
                .open(&path)
                .with_context(|| format!("{} anlegen", path.display()))?;
            f.write_all(text.as_bytes())
                .with_context(|| format!("{} schreiben", path.display()))?;
            Ok(())
        }
        None => {
            std::io::stdout()
                .lock()
                .write_all(text.as_bytes())
                .context("stdout")?;
            Ok(())
        }
    }
}

async fn audit_verify(ctx: &Ctx, client: &beton_sdk::Client) -> CliResult {
    let v = client.audit_verify().await?;
    let ok = v["ok"].as_bool() == Some(true);
    if ctx.global.json {
        print_json(&v)?;
    } else if ok {
        println!("Audit-Log intakt ({} Einträge)", v["entries"]);
    } else {
        println!(
            "Audit-Log ungültig ab Position {}: {}",
            v["first_invalid"],
            v["reason"].as_str().unwrap_or("?")
        );
    }
    if ok {
        Ok(())
    } else {
        Err(CliError::new(
            Exit::General,
            anyhow::anyhow!("Hash-Kette des Audit-Logs ist nicht intakt"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sec_004_targets_resolve_to_refs() {
        assert_eq!(
            resolve_ref("github", Some("ifahrentholz"), None)
                .unwrap()
                .to_string(),
            "secret://user/github.com/ifahrentholz"
        );
        assert_eq!(
            resolve_ref("openai", None, None).unwrap().to_string(),
            "secret://user/openai/default"
        );
        assert_eq!(
            resolve_ref("secret://org/npm/registry", None, None)
                .unwrap()
                .to_string(),
            "secret://org/npm/registry"
        );
        assert!(resolve_ref("secret://org/npm/registry", Some("x"), None).is_err());
        assert!(resolve_ref("Not A Provider", None, None).is_err());
    }
}
