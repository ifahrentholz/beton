//! Das stdio-Relay, das ein Harness als MCP-Server startet (HAR-009):
//!
//! ```text
//! beton mcp serve                 --session <id> --socket <pfad> --token-file <pfad>
//! beton mcp proxy --server <name> --session <id> --socket <pfad> --token-file <pfad>
//! ```
//!
//! Es liest das Token aus der Datei (nie aus argv oder Env), meldet sich beim Relay-Hub des
//! Runners an und reicht danach stdin/stdout unverändert durch.

use std::path::PathBuf;
use std::process::ExitCode;

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

use crate::config::SYSTEM_SERVER;
use crate::hub::{Handshake, HandshakeReply, RELAY_VERSION, read_line};

/// Argumente eines Relays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayArgs {
    pub server: String,
    pub session: String,
    pub socket: PathBuf,
    pub token_file: PathBuf,
}

/// Fehler eines Relays (ohne Token-Inhalt).
#[derive(Debug, thiserror::Error)]
pub enum RelayError {
    #[error("Aufruf: {0}")]
    Usage(String),
    #[error("Token-Datei {0}: {1}")]
    Token(String, std::io::Error),
    #[error("Relay-Hub {0} nicht erreichbar: {1}")]
    Connect(String, std::io::Error),
    #[error("vom Relay-Hub abgelehnt: {0}")]
    Rejected(String),
    #[error("E/A: {0}")]
    Io(#[from] std::io::Error),
}

/// Liest die Argumente nach `mcp`: `serve …` oder `proxy --server <name> …`.
pub fn parse_args(args: &[String]) -> Result<RelayArgs, RelayError> {
    let usage = || {
        RelayError::Usage(
            "beton mcp serve|proxy [--server <name>] --session <id> --socket <pfad> --token-file <pfad>"
                .into(),
        )
    };
    let (mode, rest) = args.split_first().ok_or_else(usage)?;
    let mut server = None;
    let mut session = None;
    let mut socket = None;
    let mut token_file = None;
    let mut it = rest.iter();
    while let Some(flag) = it.next() {
        let value = it.next().cloned().ok_or_else(usage)?;
        match flag.as_str() {
            "--server" => server = Some(value),
            "--session" => session = Some(value),
            "--socket" => socket = Some(PathBuf::from(value)),
            "--token-file" => token_file = Some(PathBuf::from(value)),
            _ => return Err(usage()),
        }
    }
    let server = match mode.as_str() {
        "serve" if server.is_none() => SYSTEM_SERVER.to_owned(),
        "proxy" => server.ok_or_else(usage)?,
        _ => return Err(usage()),
    };
    Ok(RelayArgs {
        server,
        session: session.ok_or_else(usage)?,
        socket: socket.ok_or_else(usage)?,
        token_file: token_file.ok_or_else(usage)?,
    })
}

/// Verbindet sich mit dem Hub und reicht `input`/`output` durch, bis eine Seite endet.
#[cfg(unix)]
pub async fn run<I, O>(args: &RelayArgs, input: I, mut output: O) -> Result<(), RelayError>
where
    I: AsyncRead + Unpin,
    O: AsyncWrite + Unpin,
{
    let token = std::fs::read_to_string(&args.token_file)
        .map_err(|e| RelayError::Token(args.token_file.display().to_string(), e))?;
    let stream = tokio::net::UnixStream::connect(&args.socket)
        .await
        .map_err(|e| RelayError::Connect(args.socket.display().to_string(), e))?;
    let (r, mut w) = stream.into_split();
    let mut r = BufReader::new(r);
    let hello = Handshake {
        relay: RELAY_VERSION,
        session_id: args.session.clone(),
        token: token.trim().to_owned(),
        server: args.server.clone(),
    };
    let mut line = serde_json::to_vec(&hello).map_err(std::io::Error::other)?;
    line.push(b'\n');
    w.write_all(&line).await?;
    w.flush().await?;
    let reply = read_line(&mut r, 4096).await?.unwrap_or_default();
    let reply: HandshakeReply = serde_json::from_str(&reply)
        .map_err(|_| RelayError::Rejected("ungültige Antwort".into()))?;
    if !reply.ok {
        return Err(RelayError::Rejected(reply.error.unwrap_or_default()));
    }
    let mut input = input;
    let up = async {
        let _ = tokio::io::copy(&mut input, &mut w).await;
        let _ = w.shutdown().await;
    };
    let down = async {
        let _ = tokio::io::copy(&mut r, &mut output).await;
        let _ = output.flush().await;
    };
    // Endet der Hub (Session vorbei), endet auch das Relay.
    tokio::select! {
        () = down => {}
        () = async { up.await; std::future::pending::<()>().await } => {}
    }
    Ok(())
}

/// Einstieg für `beton mcp …` und das Test-Binary des Runners.
pub async fn main(args: &[String]) -> ExitCode {
    let parsed = match parse_args(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("beton mcp: {e}");
            return ExitCode::from(2);
        }
    };
    #[cfg(unix)]
    {
        match run(&parsed, tokio::io::stdin(), tokio::io::stdout()).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("beton mcp: {e}");
                ExitCode::FAILURE
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = parsed;
        eprintln!("beton mcp: das Relay braucht Unix-Sockets (Windows folgt mit Named Pipes)");
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }

    #[test]
    fn parses_serve_and_proxy() {
        let a = parse_args(&s(&[
            "serve",
            "--session",
            "ses_1",
            "--socket",
            "/r/m.sock",
            "--token-file",
            "/r/m.token",
        ]))
        .unwrap();
        assert_eq!(a.server, "beton");
        let p = parse_args(&s(&[
            "proxy",
            "--server",
            "gh",
            "--session",
            "ses_1",
            "--socket",
            "/s",
            "--token-file",
            "/t",
        ]))
        .unwrap();
        assert_eq!(p.server, "gh");
        assert!(
            parse_args(&s(&[
                "proxy",
                "--session",
                "x",
                "--socket",
                "/s",
                "--token-file",
                "/t"
            ]))
            .is_err()
        );
        assert!(
            parse_args(&s(&[
                "serve",
                "--server",
                "gh",
                "--session",
                "x",
                "--socket",
                "/s",
                "--token-file",
                "/t"
            ]))
            .is_err()
        );
        assert!(parse_args(&s(&["serve", "--token", "geheim"])).is_err());
    }
}
