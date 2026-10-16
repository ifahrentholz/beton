//! Generator des Seatbelt-Profils (SBX-007): festes, versioniertes Template plus Parameter.
//!
//! Kein Pfad und kein Muster gelangt als Text ins SBPL. Jede generierte Zeile besteht aus
//! festen Bausteinen und einem generierten Schlüssel (`[A-Z0-9_]+`); die Werte gehen als
//! `-D KEY=VALUE` an `sandbox-exec` und werden im Profil nur über `(param "KEY")` gelesen.
//! Damit kann ein Workspace-Name mit `"`, `)` oder Zeilenumbruch das Profil nicht verändern
//! (AC3). Reguläre Ausdrücke für Muster-Masken entstehen aus maskierten Pfaden
//! ([`regex_escape`]); sie sind ebenfalls nur Parameterwerte.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::error::SandboxError;
use crate::resolve::{Mask, NetMode, ResolvedPolicy, Stage};

/// Version des Templates (erscheint in Probe und `explain`).
pub const PROFILE_VERSION: u32 = 3;

/// Das Template.
pub const TEMPLATE: &str = include_str!("profile.v3.sbpl");

/// Höchstgröße von Profil plus Parametern (SBX-007).
pub const MAX_PROFILE_BYTES: usize = 256 * 1024;

/// Mach-Services des Keychains (nur Stufe 1 mit `keychain: true`).
pub const KEYCHAIN_SERVICES: [&str; 2] = ["com.apple.SecurityServer", "com.apple.securityd"];

/// Profiltext und Parameter für einen Start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub text: String,
    pub params: Vec<(String, String)>,
}

impl Profile {
    /// Gesamtgröße (Text, Schlüssel, Werte).
    pub fn size(&self) -> usize {
        self.text.len()
            + self
                .params
                .iter()
                .map(|(k, v)| k.len() + v.len() + 1)
                .sum::<usize>()
    }
}

/// Maskiert alle Sonderzeichen regulärer Ausdrücke (POSIX ERE) mit `\`.
pub fn regex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if "\\^$.|?*+()[]{}".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Ein Pfadsegment-Muster (`*`, `?`) als regulärer Ausdruck ohne `/`.
pub fn glob_regex(glob: &str) -> String {
    let mut out = String::new();
    for c in glob.chars() {
        match c {
            '*' => out.push_str("[^/]*"),
            '?' => out.push_str("[^/]"),
            c => out.push_str(&regex_escape(&c.to_string())),
        }
    }
    out
}

fn utf8(p: &Path) -> Result<String, SandboxError> {
    p.to_str().map(str::to_owned).ok_or_else(|| {
        SandboxError::invalid("paths", format!("{} ist kein gültiges UTF-8", p.display()))
    })
}

/// Regex für einen Pfad samt Inhalt unter `root`.
fn root_regex(root: &Path) -> Result<String, SandboxError> {
    Ok(format!(
        "^{}",
        regex_escape(utf8(root)?.trim_end_matches('/'))
    ))
}

struct Builder {
    params: Vec<(String, String)>,
}

impl Builder {
    fn param(&mut self, prefix: &str, value: String) -> String {
        let key = format!("{prefix}_{}", self.params.len());
        self.params.push((key.clone(), value));
        key
    }

    fn path(&mut self, prefix: &str, p: &Path) -> Result<String, SandboxError> {
        let v = utf8(p)?;
        Ok(self.param(prefix, v))
    }
}

fn within_any(p: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|r| p.starts_with(r))
}

/// Erzeugt Profil und Parameter für `policy`.
pub fn render(policy: &ResolvedPolicy) -> Result<Profile, SandboxError> {
    render_for(policy, None)
}

/// Wie [`render`], für einen Start am Pseudo-Terminal mit dem Slave `tty` (SBX-017).
pub fn render_for(policy: &ResolvedPolicy, tty: Option<&Path>) -> Result<Profile, SandboxError> {
    let mut b = Builder { params: Vec::new() };
    let hidden_roots: Vec<&PathBuf> = policy.hidden.iter().map(|h| &h.root).collect();
    let is_grant = |p: &PathBuf| hidden_roots.contains(&p);

    let mut grant_lines = Vec::new();
    let mut root_lines = Vec::new();
    for root in &policy.read_roots {
        let key = b.path("READ", root)?;
        let line = format!("(allow file-read* (subpath (param \"{key}\")))");
        if is_grant(root) {
            grant_lines.push(line);
        } else {
            root_lines.push(line);
        }
    }
    for root in &policy.write_roots {
        let key = b.path("WRITE", root)?;
        let line = format!("(allow file-read* file-write* (subpath (param \"{key}\")))");
        if is_grant(root) {
            grant_lines.push(line);
        } else {
            root_lines.push(line);
        }
    }

    let mut hidden_lines = Vec::new();
    for h in &policy.hidden {
        let re = format!("{}/\\.[^/]+(/.*)?$", root_regex(&h.root)?);
        let key = b.param("HIDDEN", re);
        hidden_lines.push(format!(
            "(deny file-read* file-write* (regex (param \"{key}\")))"
        ));
        for name in &h.allow {
            let key = b.path("UNHIDE", &h.root.join(name))?;
            let ops = if h.writable {
                "file-read* file-write*"
            } else {
                "file-read*"
            };
            hidden_lines.push(format!("(allow {ops} (subpath (param \"{key}\")))"));
        }
    }

    let mut ancestors = BTreeSet::new();
    for root in policy.read_roots.iter().chain(&policy.write_roots) {
        for a in root.ancestors().skip(1) {
            if a.parent().is_some() {
                ancestors.insert(a.to_path_buf());
            }
        }
    }
    let mut ancestor_lines = Vec::new();
    for a in &ancestors {
        let key = b.path("ANC", a)?;
        ancestor_lines.push(format!(
            "(allow file-read-metadata (literal (param \"{key}\")))"
        ));
    }

    let mut mask_lines = Vec::new();
    let mut keep = BTreeSet::new();
    for m in &policy.masks {
        match m {
            Mask::Path { path, .. } => {
                let key = b.path("MASK", path)?;
                mask_lines.push(format!(
                    "(deny file-read* file-write* (subpath (param \"{key}\")))"
                ));
                // Eine von beton verwaltete Wurzel unter einer Maske (Worktree unter
                // `~/.beton/worktrees`, SES-015) bleibt freigegeben, samt den Metadaten ihrer
                // Vorfahren innerhalb der Maske; sonst nichts von der Maske.
                for (roots, ops, prefix) in [
                    (&policy.read_roots, "file-read*", "READ"),
                    (&policy.write_roots, "file-read* file-write*", "WRITE"),
                ] {
                    for root in roots.iter().filter(|r| r.starts_with(path) && *r != path) {
                        let key = b.path(prefix, root)?;
                        mask_lines.push(format!("(allow {ops} (subpath (param \"{key}\")))"));
                        for a in root.ancestors().skip(1) {
                            if !a.starts_with(path) {
                                break;
                            }
                            let key = b.path("ANC", a)?;
                            mask_lines.push(format!(
                                "(allow file-read-metadata (literal (param \"{key}\")))"
                            ));
                        }
                    }
                }
                for a in path.ancestors().skip(1) {
                    if within_any(a, &policy.write_roots) {
                        keep.insert(a.to_path_buf());
                    }
                }
            }
            Mask::Pattern {
                root,
                glob,
                any_depth,
                ..
            } => {
                let middle = if *any_depth { "/(.*/)?" } else { "/" };
                let re = format!("{}{middle}{}(/.*)?$", root_regex(root)?, glob_regex(glob));
                let key = b.param("MASKRE", re);
                mask_lines.push(format!(
                    "(deny file-read* file-write* (regex (param \"{key}\")))"
                ));
                if !any_depth {
                    for a in root.ancestors() {
                        if within_any(a, &policy.write_roots) {
                            keep.insert(a.to_path_buf());
                        }
                    }
                }
            }
        }
    }
    let mut keep_lines = Vec::new();
    for a in &keep {
        let key = b.path("KEEP", a)?;
        keep_lines.push(format!(
            "(deny file-write-unlink (literal (param \"{key}\")))"
        ));
    }

    let mut git_lines = Vec::new();
    if let Some(g) = &policy.git {
        let mut dirs = vec![g.common_dir.clone()];
        if g.git_dir != g.common_dir {
            dirs.push(g.git_dir.clone());
        }
        for d in &dirs {
            let hooks = b.path("GIT_HOOKS", &d.join("hooks"))?;
            let config = b.path("GIT_CONFIG", &d.join("config"))?;
            git_lines.push(format!(
                "(deny file-write* (subpath (param \"{hooks}\")) (literal (param \"{config}\")))"
            ));
            // `commondir`/`gitdir` lenken git auf ein anderes Git-Verzeichnis (#209); ein
            // literal/regex-Deny sperrt auch das Anlegen (Symlink, Hardlink, `mkdir`, `mv`).
            let re = format!(
                "{}/(commondir|gitdir|modules/(.+/)?(hooks(/.*)?|config|commondir|gitdir)|\
                 worktrees/[^/]+/(config\\.worktree|commondir|gitdir))$",
                root_regex(d)?
            );
            let key = b.param("GIT_RE", re);
            git_lines.push(format!("(deny file-write* (regex (param \"{key}\")))"));
            // Einträge unter `worktrees` weder anlegen noch wegbewegen: Sonst ließe sich ein
            // vorbereitetes Verzeichnis samt `commondir` an ihre Stelle umbenennen.
            let re = format!("{}/worktrees(/[^/]+)?$", root_regex(d)?);
            let key = b.param("GIT_WT", re);
            git_lines.push(format!(
                "(deny file-write-create file-write-unlink (regex (param \"{key}\")))"
            ));
            // `hooks` und `config` selbst nicht austauschen (Umbenennen des Elternteils).
            let keep = b.path("GIT_DIR", d)?;
            git_lines.push(format!(
                "(deny file-write-unlink (literal (param \"{keep}\")))"
            ));
        }
        let dot = b.path("DOT_GIT", &g.dot_git)?;
        if g.dot_git_is_file {
            git_lines.push(format!("(deny file-write* (literal (param \"{dot}\")))"));
        } else {
            git_lines.push(format!(
                "(deny file-write-unlink (literal (param \"{dot}\")))"
            ));
        }
    }

    let mut ro_lines = Vec::new();
    for d in &policy.read_only {
        let key = b.path("READ_ONLY", d)?;
        ro_lines.push(format!("(deny file-write* (subpath (param \"{key}\")))"));
    }
    let mut nosym_lines = Vec::new();
    for d in &policy.no_symlinks {
        let key = b.path("NO_SYMLINKS", d)?;
        // Auch keine Verzeichnisse: Ein in Stufe 2 vorbereitetes Verzeichnis mit Symlinks
        // ließe sich sonst als Ganzes in den Austausch umbenennen (Unterverzeichnisse legt
        // Claude in Stufe 1 an).
        nosym_lines.push(format!(
            "(deny file-write-create (require-all (subpath (param \"{key}\")) (vnode-type SYMLINK)))"
        ));
        nosym_lines.push(format!(
            "(deny file-write-create (require-all (subpath (param \"{key}\")) (vnode-type DIRECTORY)))"
        ));
    }

    let mut keychain_lines = Vec::new();
    if policy.keychain && policy.stage == Stage::Harness {
        let names: Vec<String> = KEYCHAIN_SERVICES
            .iter()
            .map(|s| format!("(global-name \"{s}\")"))
            .collect();
        keychain_lines.push(format!(
            "(allow mach-lookup {} (global-name-prefix \"com.apple.securityd.\"))",
            names.join(" ")
        ));
        keychain_lines.push(
            "(allow file-read* (subpath \"/Library/Keychains\") (subpath \"/private/var/db/mds\"))"
                .to_owned(),
        );
        keychain_lines.push(
            "(allow ipc-posix-shm* (ipc-posix-name-prefix \"com.apple.AppleDatabaseChanged\"))"
                .to_owned(),
        );
    }

    let mut net_lines = Vec::new();
    if let NetMode::ProxyOnly { port } = policy.net {
        let key = b.param("PROXY_PORT", port.to_string());
        net_lines.push(format!(
            "(allow network-outbound (remote ip (string-append \"localhost:\" (param \"{key}\"))))"
        ));
    }

    let mut unix_lines = Vec::new();
    for sock in &policy.unix_sockets {
        let key = b.path("SOCK", sock)?;
        unix_lines.push(format!(
            "(allow network-outbound (remote unix-socket (path-literal (param \"{key}\"))))"
        ));
    }
    // Laufzeitverzeichnis der Stufe 1 (#206): jeder Socket darin, auch in Unterverzeichnissen
    // (Hook-Server, MCP-Hub); beton legt dort nur eigene Sockets an.
    for dir in &policy.unix_socket_dirs {
        let key = b.path("SOCKDIR", dir)?;
        unix_lines.push(format!(
            "(allow network-outbound (remote unix-socket (subpath (param \"{key}\"))))"
        ));
    }
    let mut dev_lines = Vec::new();
    for port in &policy.dev_ports {
        let key = b.param("DEV_PORT", port.to_string());
        let ip = format!("(string-append \"localhost:\" (param \"{key}\"))");
        dev_lines.push(format!("(allow network-bind (local ip {ip}))"));
        dev_lines.push(format!("(allow network-inbound (local ip {ip}))"));
        dev_lines.push(format!("(allow network-outbound (remote ip {ip}))"));
    }

    let mut tty_lines = Vec::new();
    if let Some(slave) = tty {
        let key = b.path("TTY", slave)?;
        tty_lines.push(format!(
            "(allow file-read* file-write-data file-ioctl (literal (param \"{key}\")) (literal \"/dev/tty\"))"
        ));
    }

    let sections: [(&str, &Vec<String>); 15] = [
        ("GRANT_ROOTS", &grant_lines),
        ("HIDDEN", &hidden_lines),
        ("ROOTS", &root_lines),
        ("ANCESTORS", &ancestor_lines),
        ("MASKS", &mask_lines),
        ("KEEP", &keep_lines),
        ("GIT", &git_lines),
        ("READ_ONLY", &ro_lines),
        ("NO_SYMLINKS", &nosym_lines),
        ("KEYCHAIN", &keychain_lines),
        ("NETWORK", &net_lines),
        ("UNIX_SOCKETS", &unix_lines),
        ("DEV_PORTS", &dev_lines),
        ("TTY", &tty_lines),
        ("", &Vec::new()),
    ];
    let mut text = String::with_capacity(TEMPLATE.len() + 4096);
    for line in TEMPLATE.lines() {
        if let Some(name) = line.strip_prefix(";;@") {
            let lines = sections
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, l)| *l)
                .ok_or_else(|| {
                    SandboxError::spawn(format!("Template: unbekannte Markierung {name}"))
                })?;
            for l in lines {
                text.push_str(l);
                text.push('\n');
            }
            continue;
        }
        text.push_str(line);
        text.push('\n');
    }
    let profile = Profile {
        text,
        params: b.params,
    };
    if profile.size() > MAX_PROFILE_BYTES {
        return Err(SandboxError::invalid(
            "sandbox",
            format!(
                "Seatbelt-Profil wäre {} Bytes groß (höchstens {MAX_PROFILE_BYTES})",
                profile.size()
            ),
        ));
    }
    Ok(profile)
}
