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
    /// Session starten und im Terminal begleiten (mit `-p` nicht-interaktiv).
    Run(Box<RunArgs>),
    /// Gestoppte Session fortsetzen und anhängen.
    Resume(ResumeArgs),
    /// An eine laufende Session anhängen (Verlauf, dann live).
    Attach(AttachArgs),
    /// Web-UI im Browser öffnen (Anmeldung per Einmal-Link).
    Open(OpenArgs),
    /// Sessions verwalten.
    #[command(subcommand)]
    Session(SessionCommand),
    /// Lokalen Server starten (nur Loopback).
    Serve(ServeArgs),
    /// Erstkonfiguration: Harness-CLIs, Logins und lokale Modell-Server erkennen.
    Setup(SetupArgs),
    /// Chats von Claude Code und Codex übernehmen oder eine Exportdatei importieren.
    ///
    /// Ohne Argumente zeigt `import` die gefundenen Chats zur Auswahl (nur auf einem
    /// Terminal; sonst die Liste und Exit-Code 2). Die Vendor-Dateien liest nur der lokale
    /// Daemon, und er ändert sie nie.
    Import(ImportArgs),
    /// Session als JSONL exportieren (mit `--with-blobs` als .tar.zst mit Anhängen).
    Export(ExportArgs),
    /// Umgebung prüfen (ändert nichts); Exit 0 ok, 1 Warnungen, 2 Fehler.
    Doctor,
    /// Konfiguration lesen und schreiben.
    #[command(subcommand)]
    Config(ConfigCommand),
    /// Agent-Definitionen auflisten, anzeigen, prüfen und anlegen.
    #[command(subcommand)]
    Agent(AgentCommand),
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
    /// MCP-Relay für Harnesses: `mcp serve` (System-Tools) bzw. `mcp proxy --server <name>`;
    /// der Runner trägt es in die MCP-Konfiguration des Harness ein (HAR-009).
    #[command(name = "mcp", hide = true)]
    Mcp(McpArgs),
}

/// Argumente des MCP-Relays (`serve|proxy …`, siehe `beton_mcp::relay`).
#[derive(Debug, Args)]
pub struct McpArgs {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 0..)]
    pub args: Vec<String>,
}

#[derive(Debug, Args)]
pub struct RunArgs {
    /// Harness (`claude`, `codex`, `acp:<slug>`, `direct:<provider>`, `fake`) oder Agent:
    /// Name, Pfad (`./agents/x`) oder `builtin:<name>` (Default: `harnesses.default`, sonst
    /// `claude`).
    pub target: Option<String>,
    /// Prompt für den Skript-Modus; `-` liest ihn von stdin.
    #[arg(short = 'p', long = "prompt", value_name = "PROMPT")]
    pub prompt: Option<String>,
    /// Modell (sofern der Harness es unterstützt); mit `-c`/`--resume` ein Wechsel ab dem
    /// nächsten Turn.
    #[arg(long, value_name = "M")]
    pub model: Option<String>,
    /// Reasoning-Effort; eine Stufe, die der Harness nicht kennt, wird auf die nächstniedrigere
    /// gemappt (HAR-017).
    #[arg(long, value_enum, value_name = "LEVEL")]
    pub effort: Option<EffortArg>,
    /// Wie viel der Agent ohne Rückfrage darf (HAR-027). `yolo` startet nur mit Tool-Sandbox
    /// und Egress-Proxy.
    #[arg(long, value_enum, value_name = "MODE")]
    pub permission_mode: Option<PermissionModeArg>,
    /// Arbeitsverzeichnis der Session (Default: aktuelles Verzeichnis).
    #[arg(long, value_name = "DIR")]
    pub cwd: Option<std::path::PathBuf>,
    /// Zuletzt genutzte Session in diesem Verzeichnis fortsetzen.
    #[arg(short = 'c', long = "continue", conflicts_with = "resume")]
    pub continue_last: bool,
    /// Diese Session fortsetzen (ID, Präfix oder `last`).
    #[arg(long, value_name = "ID")]
    pub resume: Option<String>,
    /// Neue Session ab einem Event dieser Session abzweigen: `ID@SEQ`, ohne `@SEQ` ab dem
    /// Ende (SES-006). Mitten in einem Turn beginnt der Fork am vorherigen Turn-Ende.
    #[arg(
        long,
        value_name = "ID[@SEQ]",
        conflicts_with_all = ["continue_last", "resume", "worktree"]
    )]
    pub fork: Option<String>,
    /// Harness des Forks (wie TARGET; ein anderer Harness bekommt eine Übergabe, SES-007) bzw.
    /// eines Agents statt `executor.harness` (AGT-004).
    #[arg(long, value_name = "HARNESS")]
    pub harness: Option<String>,
    /// Dateien des Forks: neuer Worktree (Default im Git-Repository), gemeinsam oder leer.
    #[arg(long, value_enum, value_name = "MODE", requires = "fork")]
    pub workspace: Option<ForkWorkspaceArg>,
    /// Titel der neuen Session.
    #[arg(long, value_name = "T")]
    pub title: Option<String>,
    /// Session in einem eigenen `git worktree` mit eigenem Branch starten (Default-Branch
    /// `beton/<titel>-<id4>`; ein vorhandener Branch wird ausgecheckt).
    #[arg(
        long,
        value_name = "BRANCH",
        num_args = 0..=1,
        default_missing_value = "",
        conflicts_with_all = ["continue_last", "resume"]
    )]
    pub worktree: Option<String>,
    /// Base des Worktrees (Default: `origin/HEAD`, sonst der aktuelle Branch).
    #[arg(long, value_name = "BRANCH", requires = "worktree")]
    pub base: Option<String>,
    /// Session anlegen, ID ausgeben und nicht anhängen.
    #[arg(long, conflicts_with = "prompt")]
    pub detach: bool,
    /// Ausgabe im Skript-Modus.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text, value_name = "FORMAT")]
    pub output_format: OutputFormat,
    /// Verhalten bei Freigaben ohne Terminal: warten (Web-UI) oder sofort ablehnen.
    #[arg(long, value_enum, default_value_t = OnAsk::Wait, value_name = "MODE")]
    pub on_ask: OnAsk,
    /// Abbruch nach dieser Dauer, z. B. `90s`, `5m` (Skript-Modus).
    #[arg(long, value_name = "DUR", value_parser = parse_duration)]
    pub timeout: Option<std::time::Duration>,
    /// Szenario-Datei, nur für den Harness `fake` (HAR-026).
    #[arg(long, value_name = "FILE")]
    pub scenario: Option<std::path::PathBuf>,
    /// Parameter des Agents (AGT-010), mehrfach: `--param max_attempts=5`.
    #[arg(long = "param", value_name = "NAME=WERT")]
    pub params: Vec<String>,
}

/// Reasoning-Effort (HAR-017).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum EffortArg {
    Low,
    Medium,
    High,
    Xhigh,
}

impl EffortArg {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
        }
    }
}

/// Permission-Mode (HAR-027).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PermissionModeArg {
    /// Nur lesen und planen.
    Plan,
    /// Standard des Harness; Freigaben laut Policy.
    Default,
    /// Datei-Edits im Worktree ohne Rückfrage.
    #[value(name = "accept_edits")]
    AcceptEdits,
    /// Keine Rückfragen des Agents; nur mit Sandbox und Egress-Proxy.
    Yolo,
}

impl PermissionModeArg {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Default => "default",
            Self::AcceptEdits => "accept_edits",
            Self::Yolo => "yolo",
        }
    }
}

/// Workspace eines Forks (SES-006).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ForkWorkspaceArg {
    /// Neuer `git worktree` vom Stand der Quelle inkl. WIP-Snapshot.
    NewWorktree,
    /// Dieselben Dateien wie die Quelle.
    Shared,
    /// Leeres Verzeichnis, nur der Verlauf.
    Fresh,
}

impl ForkWorkspaceArg {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NewWorktree => "new_worktree",
            Self::Shared => "shared",
            Self::Fresh => "fresh",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    /// Nur der Antworttext.
    Text,
    /// Am Ende genau ein JSON-Objekt.
    Json,
    /// NDJSON der Events.
    StreamJson,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OnAsk {
    Wait,
    Deny,
}

/// `90s`, `5m`, `1h` oder Sekunden.
pub fn parse_duration(raw: &str) -> Result<std::time::Duration, String> {
    let raw = raw.trim();
    let (num, unit) = raw
        .find(|c: char| !c.is_ascii_digit())
        .map_or((raw, "s"), |i| raw.split_at(i));
    let n: u64 = num
        .parse()
        .map_err(|_| format!("`{raw}` ist keine Dauer (z. B. 90s, 5m)"))?;
    let secs = match unit {
        "s" | "" => n,
        "m" => n * 60,
        "h" => n * 3600,
        _ => return Err(format!("Einheit `{unit}` unbekannt (s, m, h)")),
    };
    Ok(std::time::Duration::from_secs(secs))
}

#[derive(Debug, Args)]
pub struct ResumeArgs {
    /// Session (ID, eindeutiges Präfix oder `last`).
    pub session: String,
}

#[derive(Debug, Args)]
pub struct AttachArgs {
    /// Session (ID, eindeutiges Präfix oder `last`).
    pub session: String,
    /// Nur zuschauen: keine Eingaben, keine Freigaben.
    #[arg(long)]
    pub read_only: bool,
}

#[derive(Debug, Args)]
pub struct SessionRefArgs {
    /// Session (ID, eindeutiges Präfix oder `last`).
    pub session: String,
}

#[derive(Debug, Args)]
pub struct DeleteArgs {
    /// Session (ID, eindeutiges Präfix oder `last`).
    pub session: String,
    /// Uncommittete Änderungen im Worktree: als WIP-Commit sichern oder verwerfen.
    #[arg(long, value_enum, value_name = "WAS")]
    pub uncommitted: Option<UncommittedChoice>,
    /// Branch mit ungepushten Commits: behalten oder löschen.
    #[arg(long, value_enum, value_name = "WAS")]
    pub branch: Option<BranchChoice>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum UncommittedChoice {
    /// WIP-Commit auf dem Branch.
    Commit,
    /// Änderungen verwerfen.
    Discard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BranchChoice {
    /// Branch behalten.
    Keep,
    /// Branch löschen.
    Delete,
}

#[derive(Debug, Args)]
pub struct RenameArgs {
    /// Session (ID, eindeutiges Präfix oder `last`).
    pub session: String,
    /// Neuer Titel.
    pub title: String,
}

#[derive(Debug, Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct SetupArgs {
    /// Keine Rückfragen; installiert und meldet nie etwas an.
    #[arg(long)]
    pub non_interactive: bool,
    /// Nur prüfen und ausgeben (mit `--json` maschinenlesbar).
    #[arg(long)]
    pub check: bool,
    #[command(subcommand)]
    pub command: Option<SetupCommand>,
}

#[derive(Debug, Subcommand)]
pub enum SetupCommand {
    /// ACP-Agents registrieren (HAR-008).
    #[command(subcommand)]
    Acp(SetupAcpCommand),
}

#[derive(Debug, Subcommand)]
pub enum SetupAcpCommand {
    /// ACP-Agent in `~/.beton/config.yaml` eintragen; danach ist er als `acp:<SLUG>` nutzbar.
    Add(SetupAcpAddArgs),
}

#[derive(Debug, Args)]
pub struct SetupAcpAddArgs {
    /// Kurzname des Agents (`[a-z0-9-]`), z. B. `mein-agent`.
    pub slug: String,
    /// Programm (Name in `PATH` oder Pfad).
    #[arg(long, value_name = "C")]
    pub command: String,
    /// Argument, mit dem der Agent ACP über stdio spricht (mehrfach möglich).
    #[arg(long = "arg", value_name = "A", allow_hyphen_values = true)]
    pub args: Vec<String>,
}

#[derive(Debug, Args)]
pub struct ImportArgs {
    /// Exportdatei von `beton export` (.jsonl oder .tar.zst); wird eine neue Session.
    #[arg(value_name = "FILE", conflicts_with_all = ["harness", "session", "last"])]
    pub file: Option<PathBuf>,
    /// Werkzeug, dessen Chats übernommen werden (Default: alle).
    #[arg(long, value_enum, value_name = "HARNESS")]
    pub harness: Option<ImportHarness>,
    /// Chat mit dieser Session-ID des Werkzeugs übernehmen (mehrfach möglich).
    #[arg(long = "session", value_name = "REF")]
    pub session: Vec<String>,
    /// Die N zuletzt geänderten Chats übernehmen.
    #[arg(
        long,
        value_name = "N",
        conflicts_with = "session",
        value_parser = clap::value_parser!(u32).range(1..=500)
    )]
    pub last: Option<u32>,
    /// Schon übernommene Chats bzw. schon importierte Dateien erneut als neue Session anlegen.
    #[arg(long)]
    pub force: bool,
}

/// Herkunft fremder Chats (SES-008).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ImportHarness {
    /// Claude Code (`~/.claude/projects`).
    Claude,
    /// Codex (`~/.codex/sessions`).
    Codex,
    /// Beide.
    All,
}

#[derive(Debug, Args)]
pub struct ExportArgs {
    /// Session (ID, eindeutiges Präfix oder `last`).
    pub session: String,
    /// Zieldatei (Default: stdout); wird nur für den Eigentümer lesbar angelegt.
    #[arg(short = 'o', long = "output", value_name = "FILE")]
    pub output: Option<PathBuf>,
    /// Mit Anhängen als .tar.zst (`session.jsonl` und `blobs/<sha256>`).
    #[arg(long)]
    pub with_blobs: bool,
    /// Rohausgaben der Harnesses (redigiert) einschließen, z. B. für Fehlerberichte.
    #[arg(long)]
    pub with_raw: bool,
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
    /// Details einer Session.
    Show(SessionRefArgs),
    /// Session umbenennen.
    Rename(RenameArgs),
    /// Session archivieren (Runner stoppt).
    Archive(SessionRefArgs),
    /// Archivierte Session wiederherstellen.
    Unarchive(SessionRefArgs),
    /// Session endgültig löschen (entfernt ihren Worktree kontrolliert).
    Delete(DeleteArgs),
    /// Laufenden Turn abbrechen.
    Interrupt(SessionRefArgs),
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
    /// Im Vordergrund laufen statt im Hintergrund.
    #[arg(long)]
    pub foreground: bool,
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
pub enum AgentCommand {
    /// Agents im Suchpfad: Projekt, User, Built-ins (Name, Quelle, Version, Harness, Pfad).
    List(AgentListArgs),
    /// Aufgelösten Agent mit Herkunft jedes Felds und Inhalts-Hash anzeigen.
    Show(AgentShowArgs),
    /// Agent gegen Schema und semantische Regeln prüfen; Exit 0 gültig, 1 Fehler.
    Validate(AgentValidateArgs),
    /// Gerüst unter `.beton/agents/<NAME>/` anlegen (mit `$schema`-Kommentar, Prompt, Skill).
    New(AgentNewArgs),
    /// JSON-Schema des Agent-Formats ausgeben.
    Schema,
}

#[derive(Debug, Args)]
pub struct AgentListArgs {
    /// Auch verschattete und ungültige Einträge zeigen.
    #[arg(long)]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct AgentShowArgs {
    /// Agent-Ref: Name, Pfad (`./agents/x`) oder `builtin:<name>`.
    #[arg(value_name = "REF")]
    pub reference: String,
}

#[derive(Debug, Args)]
pub struct AgentValidateArgs {
    /// Agent-Verzeichnis (oder dessen `agent.yaml`); sonst ein Agent-Ref.
    #[arg(value_name = "PATH")]
    pub path: String,
}

#[derive(Debug, Args)]
pub struct AgentNewArgs {
    /// Name des Agents (`a-z`, `0-9`, `-`).
    pub name: String,
    /// Vorhandenen Agent als Vorlage kopieren, z. B. `builtin:maestra`.
    #[arg(long, value_name = "REF")]
    pub from: Option<String>,
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
