//! Kommandobaum des Binaries `beton` (CLI-001).
//!
//! Verbindlich ist der Baum aus `docs/spec/08-clients.md` (Design). Angelegt sind nur
//! Kommandos, deren Features im aktuellen Meilenstein liegen; die übrigen folgen mit ihren
//! Arbeitspaketen.

use std::net::IpAddr;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

/// beton: Meta-Harness für KI-Coding-Agents.
#[derive(Debug, Parser)]
#[command(
    name = "beton",
    version,
    about = "Meta-Harness für KI-Coding-Agents",
    long_about = "Meta-Harness für KI-Coding-Agents: Sessions mit Claude Code und anderen \
                  Vendor-CLIs lokal starten, beobachten und steuern.",
    disable_help_subcommand = true,
    propagate_version = true
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,
    #[command(subcommand)]
    pub command: Command,
}

/// Globale Flags, gültig für jedes Kommando.
#[derive(Debug, Clone, Args)]
pub struct GlobalArgs {
    /// Server-URL statt des lokalen Daemons (Token aus `BETON_TOKEN`).
    #[arg(
        long,
        global = true,
        value_name = "URL",
        env = "BETON_SERVER",
        hide_env_values = true
    )]
    pub server: Option<String>,
    /// Maschinenlesbare Ausgabe (JSON) auf stdout.
    #[arg(long, global = true)]
    pub json: bool,
    /// Keine Fortschritts- und Hinweismeldungen auf stderr.
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    pub quiet: bool,
    /// Mehr Diagnose auf stderr (mehrfach: noch mehr).
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,
    /// Keine Farben (auch über `NO_COLOR`).
    #[arg(long, global = true)]
    pub no_color: bool,
    /// User-Konfiguration aus dieser Datei statt `~/.beton/config.yaml`.
    #[arg(long, global = true, value_name = "FILE")]
    pub config: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Web-UI im Browser öffnen (Anmeldung per Einmal-Link).
    Open(OpenArgs),
    /// Sessions verwalten.
    #[command(subcommand)]
    Session(SessionCommand),
    /// Lokalen Server starten (nur Loopback).
    Serve(ServeArgs),
    /// Konfiguration lesen und schreiben.
    #[command(subcommand)]
    Config(ConfigCommand),
    /// Lokale Anmeldung verwalten.
    #[command(subcommand)]
    Auth(AuthCommand),
    /// Wartung von Datenbank und Projektionen.
    #[command(subcommand)]
    Admin(AdminCommand),
    /// Shell-Vervollständigung ausgeben.
    Completion(CompletionArgs),
    /// Version ausgeben.
    Version,
    /// Werkzeuge für die Entwicklung von beton selbst.
    #[command(subcommand, hide = true)]
    Dev(DevCommand),
    /// Runner-Prozess einer Session (startet der Daemon, RUN-002).
    #[command(name = "__runner", hide = true)]
    Runner,
}

#[derive(Debug, Args)]
pub struct OpenArgs {
    /// Session, die direkt geöffnet wird.
    pub session: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum SessionCommand {
    /// Sessions auflisten.
    List(SessionListArgs),
}

#[derive(Debug, Args)]
pub struct SessionListArgs {
    /// Auch archivierte Sessions zeigen.
    #[arg(long)]
    pub archived: bool,
}

#[derive(Debug, Args)]
pub struct ServeArgs {
    /// Adresse, an die der Server bindet (nur Loopback, z. B. 127.0.0.1).
    #[arg(long, value_name = "ADDR")]
    pub bind: Option<IpAddr>,
    /// Port (Default 7420; 0 = beliebiger freier Port).
    #[arg(long, value_name = "N")]
    pub port: Option<u16>,
    /// Entwicklermodus: Fake-Harness verfügbar (HAR-026).
    #[arg(long)]
    pub dev: bool,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Effektiven Wert eines Schlüssels ausgeben.
    Get(ConfigKeyArgs),
    /// Wert setzen (Default: User-Konfiguration).
    Set(ConfigSetArgs),
    /// Wert entfernen.
    Unset(ConfigKeyArgs),
    /// Effektive Konfiguration mit Herkunft je Schlüssel.
    List,
    /// Konfigurationsdatei im Editor öffnen und danach prüfen.
    Edit(ConfigScopeArgs),
}

#[derive(Debug, Clone, Copy, Args)]
pub struct ConfigScopeArgs {
    /// User-Konfiguration (`~/.beton/config.yaml`).
    #[arg(long, conflicts_with = "project")]
    pub global: bool,
    /// Projekt-Konfiguration (`.beton/config.yaml`).
    #[arg(long)]
    pub project: bool,
}

#[derive(Debug, Args)]
pub struct ConfigKeyArgs {
    /// Schlüssel in Punktnotation, z. B. `events.store_raw`.
    pub key: String,
    #[command(flatten)]
    pub scope: ConfigScopeArgs,
}

#[derive(Debug, Args)]
pub struct ConfigSetArgs {
    /// Schlüssel in Punktnotation, z. B. `events.store_raw`.
    pub key: String,
    /// Wert als YAML, z. B. `false`, `7420` oder `[a, b]`.
    pub value: String,
    #[command(flatten)]
    pub scope: ConfigScopeArgs,
}

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Lokales Token erneuern; laufende Clients lesen es neu.
    #[command(name = "rotate-local")]
    RotateLocal,
}

#[derive(Debug, Subcommand)]
pub enum AdminCommand {
    /// Projektionen verwalten.
    #[command(subcommand)]
    Projections(ProjectionsCommand),
}

#[derive(Debug, Subcommand)]
pub enum ProjectionsCommand {
    /// Projektionen aus dem Event-Log neu aufbauen.
    Rebuild(RebuildArgs),
}

#[derive(Debug, Args)]
pub struct RebuildArgs {
    /// Nur diese Session.
    #[arg(long, value_name = "ID")]
    pub session: Option<String>,
}

#[derive(Debug, Args)]
pub struct CompletionArgs {
    pub shell: Shell,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    Powershell,
}

#[derive(Debug, Subcommand)]
pub enum DevCommand {
    /// Golden-Transcripts mit der echten Vendor-CLI aufnehmen (HAR-025).
    #[command(name = "record-golden")]
    RecordGolden(RecordGoldenArgs),
}

#[derive(Debug, Args)]
pub struct RecordGoldenArgs {
    /// Harness, z. B. `claude`.
    #[arg(long)]
    pub harness: String,
    /// Nur diese Szenarien (mehrfach möglich); ohne Angabe alle.
    #[arg(long)]
    pub scenario: Vec<String>,
    /// Zielverzeichnis (Default `crates/beton-harness-<harness>/tests/golden`).
    #[arg(long, value_name = "DIR")]
    pub out: Option<PathBuf>,
}
