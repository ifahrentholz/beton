//! Lenkt die Git-Konfiguration einen Push ohne Refspec um (`session.push_redirected`,
//! POL-017, #293)? Der Runner liest die Dateien selbst, ohne `git` zu starten: die des Repos
//! (Common-Dir `config`, `config.worktree`), die globale (`~/.gitconfig`,
//! `$XDG_CONFIG_HOME/git/config`) und bekannte System-Dateien.
//!
//! Umlenkend sind `remote.<r>.push`, `remote.<r>.mirror` (außer `false`), `push.default`
//! `upstream`/`tracking`/`matching` sowie jede eingebundene Datei (`include.*`,
//! `includeIf.*`), weil der Runner Includes nicht auswertet. Eine Datei, die existiert, aber
//! nicht lesbar, zu groß oder nicht sicher zerlegbar ist, zählt ebenfalls (fail closed).

use std::path::{Path, PathBuf};

/// Größte Konfigurationsdatei, die gelesen wird.
const MAX_BYTES: u64 = 1 << 20;

/// System-Konfigurationen üblicher Installationen (Homebrew-Präfixe gehören dem User).
const SYSTEM_FILES: [&str; 3] = [
    "/etc/gitconfig",
    "/opt/homebrew/etc/gitconfig",
    "/usr/local/etc/gitconfig",
];

/// Prüft die Konfiguration für ein Repository, in dem `cwd` liegt. `home` ist das Home des
/// Users (für `~/.gitconfig`).
pub fn redirected(cwd: &Path, home: Option<&Path>) -> bool {
    let mut files: Vec<PathBuf> = Vec::new();
    if let Some((git_dir, common)) = git_dirs(cwd) {
        files.push(common.join("config"));
        files.push(git_dir.join("config.worktree"));
    }
    if let Some(home) = home {
        files.push(home.join(".gitconfig"));
        let xdg = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".config"));
        files.push(xdg.join("git/config"));
    }
    files.extend(SYSTEM_FILES.iter().map(PathBuf::from));
    files.iter().any(|f| file_redirects(f))
}

/// Git-Dir und Common-Dir des Repos von `cwd` (`.git` als Verzeichnis oder `gitdir:`-Datei,
/// `commondir` eines Worktrees).
fn git_dirs(cwd: &Path) -> Option<(PathBuf, PathBuf)> {
    let dot_git = cwd
        .ancestors()
        .map(|d| d.join(".git"))
        .find(|p| p.symlink_metadata().is_ok())?;
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else {
        let text = read_small(&dot_git).ok()?;
        let target = text.lines().next()?.strip_prefix("gitdir:")?.trim();
        dot_git.parent()?.join(target)
    };
    let common = match read_small(&git_dir.join("commondir")) {
        Ok(text) => git_dir.join(text.trim()),
        Err(_) => git_dir.clone(),
    };
    Some((git_dir, common))
}

fn read_small(path: &Path) -> std::io::Result<String> {
    use std::io::Read as _;
    let mut text = String::new();
    std::fs::File::open(path)?
        .take(MAX_BYTES + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > MAX_BYTES {
        return Err(std::io::Error::other("zu groß"));
    }
    Ok(text)
}

/// Fehlt die Datei, lenkt sie nichts um; sonst entscheidet der Inhalt (fail closed bei
/// Lese- oder Zerlegungsfehlern).
fn file_redirects(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Err(e) if missing(&e) => return false,
        Err(_) => return true,
        Ok(_) => {}
    }
    match read_small(path) {
        Ok(text) => text_redirects(&text),
        Err(_) => true,
    }
}

/// Wertet den Text einer Git-Konfiguration aus (Abschnitte `[name]`, `[name "sub"]`,
/// `[name.sub]`; Schlüssel ohne Groß-/Kleinschreibung).
pub fn text_redirects(text: &str) -> bool {
    let mut section = String::new();
    let mut lines = text.lines();
    while let Some(raw) = lines.next() {
        let mut line = raw.to_owned();
        // Fortsetzungszeilen.
        while line.ends_with('\\') {
            line.pop();
            match lines.next() {
                Some(next) => line.push_str(next),
                None => return true,
            }
        }
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            let Some(end) = rest.find(']') else {
                return true;
            };
            let header = &rest[..end];
            let name = header
                .split(|c: char| c.is_whitespace() || c == '.')
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            if name.is_empty() {
                return true;
            }
            if name == "include" || name == "includeif" {
                return true;
            }
            section = name;
            // Nach `]` kann direkt ein Schlüssel stehen (`[push] default = matching`).
            let after = rest[end + 1..].trim();
            if after.is_empty() || after.starts_with('#') || after.starts_with(';') {
                continue;
            }
            if entry_redirects(&section, after) {
                return true;
            }
            continue;
        }
        if section.is_empty() {
            return true;
        }
        if entry_redirects(&section, line) {
            return true;
        }
    }
    false
}

fn entry_redirects(section: &str, line: &str) -> bool {
    let (key, value) = match line.split_once('=') {
        Some((k, v)) => (k.trim(), v.trim()),
        None => (line, "true"),
    };
    let key = key.to_ascii_lowercase();
    if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return true;
    }
    let value = value
        .split(['#', ';'])
        .next()
        .unwrap_or_default()
        .trim()
        .trim_matches('"')
        .to_ascii_lowercase();
    match (section, key.as_str()) {
        ("remote", "push") => true,
        ("remote", "mirror") => !matches!(value.as_str(), "false" | "no" | "off" | "0"),
        ("push", "default") => matches!(value.as_str(), "upstream" | "tracking" | "matching"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::text_redirects;

    #[test]
    fn pol_017_push_config_text() {
        for (text, redirects) in [
            ("[core]\n\tbare = false\n", false),
            ("[push]\n\tdefault = simple\n", false),
            ("[push]\n\tdefault = current # Kommentar\n", false),
            ("[push] default = matching\n", true),
            ("[remote \"origin\"]\n\tmirror = false\n", false),
            ("[remote \"origin\"]\n\tmirror\n", true),
            ("[remote.origin]\n\tpush = x\n", true),
            ("[branch \"main\"]\n\tmerge = refs/heads/main\n", false),
            ("default = matching\n", true),
            ("[push]\n\tde\\\nfault = upstream\n", true),
            ("[includeIf \"onbranch:x\"]\n\tpath = c\n", true),
        ] {
            assert_eq!(text_redirects(text), redirects, "{text}");
        }
    }
}

/// Fehlt die Datei (auch, weil ein Teil des Pfads kein Verzeichnis ist)?
fn missing(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
    )
}
