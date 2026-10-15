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

/// Unterkommando und dessen Argumente, wenn das Teilkommando `git` aufruft.
fn invocation<'a>(cmd: &'a [&'a str]) -> Option<(&'a str, &'a [&'a str])> {
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
    Some((sub, &eff[i + 1..]))
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
        let dst = match spec.split_once(':') {
            Some(("", dst)) => {
                p.delete = true;
                dst
            }
            Some((src, "")) => src,
            Some((_, dst)) => dst,
            None => spec,
        };
        let dst = dst.strip_prefix("refs/heads/").unwrap_or(dst);
        p.targets.push(dst.to_owned());
    }
    if positional.len() <= 1 && !all && !tags {
        // Ohne Refspec: der aktuelle Branch (`session.branch`).
        p.targets.push("HEAD".into());
    }
    p
}

/// Operationen eines Teilkommandos: `push`, `force_push`, `delete_branch`, `merge`,
/// `history_rewrite`.
fn ops_of(cmd: &[&str], out: &mut Vec<&'static str>) {
    let Some((sub, args)) = invocation(cmd) else {
        return;
    };
    let mut add = |op: &'static str| {
        if !out.contains(&op) {
            out.push(op);
        }
    };
    let has = |names: &[&str]| options(args).any(|a| names.contains(&a));
    match sub {
        "push" => {
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

/// `shell.git_push_targets(argv)`: Ziel-Branches aller `git push` (`refs/heads/` entfernt);
/// `HEAD` für den aktuellen Branch (ohne Refspec), `*` für `--all`/`--mirror`.
pub fn git_push_targets(argv: &[Vec<&str>]) -> Vec<String> {
    let mut out = Vec::new();
    for cmd in argv {
        if let Some(("push", args)) = invocation(cmd) {
            out.extend(push(args).targets);
        }
    }
    out
}
