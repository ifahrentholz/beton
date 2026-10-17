//! Git-Operationen in Shell-Kommandos (POL-017, `git_guard`): `shell.git_ops(argv)` und
//! `shell.git_push_targets(argv)`.
//!
//! Jedes Teilkommando wird für sich betrachtet (Wrapper wie `sudo`/`env` übersprungen, Optionen
//! vor dem Unterkommando wie `-C <dir>` und `-c key=val` ebenfalls). Die Erkennung ist best
//! effort; Kommandos, die sich nicht sicher zerlegen lassen, meldet `tool.parse_uncertain`
//! (POL-005), und `git_guard` fragt dann nach.

use super::shell::{GIT_VALUE_OPTIONS, basename, effective};

/// Optionen von `git push`, die einen Wert nehmen.
const PUSH_VALUE_OPTIONS: [&str; 5] = ["-o", "--push-option", "--repo", "--receive-pack", "--exec"];

/// Ein `git`-Aufruf: Optionen vor dem Unterkommando, Unterkommando, dessen Argumente.
struct Invocation<'a> {
    global: &'a [&'a str],
    sub: &'a str,
    args: &'a [&'a str],
}

/// Der `git`-Aufruf eines Teilkommandos.
fn git<'a>(cmd: &'a [&'a str]) -> Option<Invocation<'a>> {
    let eff = effective(cmd);
    if basename(eff.first()?) != "git" {
        return None;
    }
    let mut i = 1;
    while i < eff.len() {
        let a = eff[i];
        if a == "--" {
            i += 1;
            break;
        }
        if a.starts_with('-') && a.len() > 1 {
            i += if GIT_VALUE_OPTIONS.contains(&a) { 2 } else { 1 };
            continue;
        }
        break;
    }
    let sub = *eff.get(i)?;
    Some(Invocation {
        global: &eff[1..i.min(eff.len())],
        sub,
        args: &eff[i + 1..],
    })
}

/// Unterkommando und dessen Argumente, wenn das Teilkommando `git` aufruft.
fn invocation<'a>(cmd: &'a [&'a str]) -> Option<(&'a str, &'a [&'a str])> {
    git(cmd).map(|g| (g.sub, g.args))
}

/// Unterkommandos, die pushen (`send-pack` ist der Push ohne Porcelain).
fn is_push(sub: &str) -> bool {
    matches!(sub, "push" | "send-pack")
}

/// Optionen vor dem Unterkommando, die ein anderes Repository bzw. einen anderen Worktree
/// wählen.
const RELOCATING: [&str; 3] = ["-C", "--git-dir", "--work-tree"];

/// Umgebungsvariablen, die ein anderes Repository bzw. einen anderen Worktree wählen.
const RELOCATING_ENV: [&str; 3] = ["GIT_DIR=", "GIT_WORK_TREE=", "GIT_COMMON_DIR="];

/// Konfiguration, die bestimmt, wohin gepusht wird (`-c remote.*`, `push.*`, `branch.*`).
fn push_config(global: &[&str]) -> bool {
    let affects = |kv: &str| {
        let key = kv.to_ascii_lowercase();
        ["remote.", "push.", "branch."]
            .iter()
            .any(|p| key.starts_with(p))
    };
    let mut i = 0;
    while i < global.len() {
        let a = global[i];
        if (a == "-c" || a == "--config-env") && global.get(i + 1).is_some_and(|v| affects(v)) {
            return true;
        }
        if let Some(v) = a.strip_prefix("--config-env=")
            && affects(v)
        {
            return true;
        }
        i += 1;
    }
    false
}

/// Läuft ein Git-Aufruf möglicherweise nicht im Arbeitsverzeichnis des Calls (`git -C`,
/// `--git-dir`, `--work-tree`, `GIT_DIR=` …, ein `cd`/`pushd` davor)? Dann sagt
/// `session.branch` nichts über seinen Branch.
pub fn git_elsewhere(argv: &[Vec<&str>]) -> bool {
    if argv
        .iter()
        .flatten()
        .any(|w| RELOCATING_ENV.iter().any(|e| w.starts_with(e)))
    {
        return true;
    }
    let mut moved = false;
    for cmd in argv {
        let first = effective(cmd).first().map(|w| basename(w));
        if matches!(first, Some("cd" | "pushd" | "popd")) {
            moved = true;
            continue;
        }
        if let Some(g) = git(cmd) {
            let relocated = g.global.iter().any(|a| {
                RELOCATING.contains(a)
                    || a.starts_with("--git-dir=")
                    || a.starts_with("--work-tree=")
                    || (a.starts_with("-C") && a.len() > 2)
            });
            if moved || relocated {
                return true;
            }
        }
    }
    false
}

/// Options-Argumente bis `--`.
fn options<'a>(args: &'a [&'a str]) -> impl Iterator<Item = &'a str> {
    args.iter()
        .copied()
        .take_while(|a| *a != "--")
        .filter(|a| a.starts_with('-') && a.len() > 1)
}

/// `--long`, `--long=…` oder eine kurze Option (auch gebündelt, `-uf`).
fn flag(a: &str, long: &str, short: &[char]) -> bool {
    if a == long || a.strip_prefix(long).is_some_and(|r| r.starts_with('=')) {
        return true;
    }
    a.starts_with('-')
        && !a.starts_with("--")
        && a.len() > 1
        && a[1..].chars().all(|c| c.is_ascii_alphabetic())
        && a[1..].chars().any(|c| short.contains(&c))
}

/// Ergebnis der Analyse eines `git push`.
#[derive(Default)]
struct Push {
    force: bool,
    delete: bool,
    targets: Vec<String>,
    /// Ziele aus Refspecs der Form `src:dst`.
    explicit: Vec<String>,
}

fn push(args: &[&str]) -> Push {
    let mut p = Push::default();
    let mut all = false;
    let mut tags = false;
    let mut positional: Vec<&str> = Vec::new();
    let mut options_done = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i];
        i += 1;
        if !options_done && a == "--" {
            options_done = true;
            continue;
        }
        if options_done || !a.starts_with('-') || a.len() == 1 {
            positional.push(a);
            continue;
        }
        if PUSH_VALUE_OPTIONS.contains(&a) {
            i += 1;
            continue;
        }
        if flag(a, "--force", &['f']) || a.starts_with("--force-with-lease") || a == "--mirror" {
            p.force = true;
        }
        if a == "--mirror" || a == "--all" || a == "--branches" {
            all = true;
        }
        if flag(a, "--delete", &['d']) {
            p.delete = true;
        }
        if a == "--tags" {
            tags = true;
        }
    }
    if all {
        p.targets.push("*".into());
    }
    // Erstes Positionsargument: Remote; danach Refspecs.
    for spec in positional.iter().skip(1) {
        let spec = match spec.strip_prefix('+') {
            Some(rest) => {
                p.force = true;
                rest
            }
            None => spec,
        };
        let (dst, explicit) = match spec.split_once(':') {
            Some(("", dst)) => {
                p.delete = true;
                (dst, true)
            }
            Some((src, "")) => (src, false),
            Some((_, dst)) => (dst, true),
            None => (spec, false),
        };
        let target = branch_target(dst);
        if explicit {
            p.explicit.push(target.clone());
        }
        p.targets.push(target);
    }
    if positional.len() <= 1 && !all && !tags {
        // Ohne Refspec: der aktuelle Branch (`session.branch`).
        p.targets.push("HEAD".into());
    }
    p
}

/// Ziel-Branch einer Refspec: `refs/heads/` entfernt, Tags (`refs/tags/…`) unverändert; alles
/// ohne eindeutigen Branch-Namen (`*`, `@`, `heads/…`, andere `refs/…`, Zeichen wie `~`, `^`)
/// wird `*` (geschützt, Review #285 M1).
fn branch_target(dst: &str) -> String {
    if dst == "HEAD" {
        return dst.to_owned();
    }
    if dst.starts_with("refs/tags/") {
        return dst.to_owned();
    }
    let name = match dst.strip_prefix("refs/heads/") {
        Some(rest) => rest,
        None if dst.starts_with("refs/") || dst.starts_with("heads/") => return "*".into(),
        None => dst,
    };
    let valid = !name.is_empty()
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._/-".contains(c))
        && !name.contains("..");
    if valid { name.to_owned() } else { "*".into() }
}

/// Operationen eines Teilkommandos: `push`, `force_push`, `delete_branch`, `merge`,
/// `history_rewrite`.
fn ops_of(cmd: &[&str], out: &mut Vec<&'static str>) {
    let Some(g) = git(cmd) else {
        return;
    };
    let (sub, args) = (g.sub, g.args);
    let mut add = |op: &'static str| {
        if !out.contains(&op) {
            out.push(op);
        }
    };
    let has = |names: &[&str]| options(args).any(|a| names.contains(&a));
    // Konfiguration, die das Push-Ziel ändert: nicht sicher beurteilbar.
    if push_config(g.global) {
        add("uncertain");
    }
    match sub {
        "push" | "send-pack" => {
            let p = push(args);
            add("push");
            if p.force {
                add("force_push");
            }
            if p.delete {
                add("delete_branch");
            }
        }
        "merge" if !has(&["--abort", "--quit"]) => add("merge"),
        "branch" if options(args).any(|a| flag(a, "--delete", &['d', 'D'])) => {
            add("delete_branch");
        }
        "reset" if has(&["--hard"]) => add("history_rewrite"),
        "rebase" if !has(&["--abort", "--quit", "--edit-todo", "--show-current-patch"]) => {
            add("history_rewrite");
        }
        "filter-branch" | "filter-repo" => add("history_rewrite"),
        "commit" if has(&["--amend"]) => add("history_rewrite"),
        _ => {}
    }
}

/// `shell.git_ops(argv)`: Git-Operationen aller Teilkommandos (ohne Doppelte, in Reihenfolge).
pub fn git_ops(argv: &[Vec<&str>]) -> Vec<&'static str> {
    let mut out = Vec::new();
    for cmd in argv {
        ops_of(cmd, &mut out);
    }
    out
}

/// `shell.git_push_targets(argv, elsewhere?)`: Ziel-Branches aller `git push` bzw.
/// `git send-pack` (`refs/heads/` entfernt); `HEAD` für den aktuellen Branch (ohne Refspec),
/// `*` für `--all`/`--mirror` und unklare Ziele. Mit `elsewhere` (der Push läuft womöglich in
/// einem anderen Repository, siehe [`git_elsewhere`]) zählen nur Ziele der Form `src:dst`;
/// alles andere wird `*`.
pub fn git_push_targets(argv: &[Vec<&str>], elsewhere: bool) -> Vec<String> {
    let mut out = Vec::new();
    for cmd in argv {
        if let Some((sub, args)) = invocation(cmd)
            && is_push(sub)
        {
            let p = push(args);
            if elsewhere {
                let explicit = p.explicit;
                out.extend(p.targets.into_iter().map(|t| {
                    if t != "*" && explicit.contains(&t) {
                        t
                    } else {
                        "*".into()
                    }
                }));
            } else {
                out.extend(p.targets);
            }
        }
    }
    out
}
