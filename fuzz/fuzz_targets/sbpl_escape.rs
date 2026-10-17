//! QA-009: SBPL-Pfad-Escaping des Seatbelt-Profils (SBX-007 AC3).
//!
//! Gefuzzt: `seatbelt::profile::regex_escape`, `glob_regex` und `render` mit einer
//! zufälligen `ResolvedPolicy` (Wurzeln, Masken, Dotfiles, Git, Sockets, Ports).
//!
//! Eingabe (Zeilen): `E <text>` (Escaping), `G <glob>` und `N <name>` (Glob gegen Namen),
//! `R`/`W`/`H`/`M`/`O`/`Y`/`U <pfad>` (Lese-, Schreib-, Dotfile-Wurzel, Maske, read-only,
//! ohne Symlinks, Unix-Socket), `K <wurzel>|<glob>|<0|1>` (Muster-Maske), `A <name>`
//! (erlaubtes Dotfile), `D <port>`, `X <port>` (Proxy), `C` (Keychain),
//! `T <dot_git>|<git_dir>|<common_dir>|<0|1>` (Git-Schutz).
//!
//! Invarianten:
//! - `^regex_escape(s)$` ist ein gültiger regulärer Ausdruck, der genau `s` trifft;
//! - `^glob_regex(g)$` trifft einen Namen ohne `/` genau dann, wenn eine Referenz-
//!   Implementierung von `*`/`?` ihn trifft;
//! - SBX-007 AC3: Der Profiltext hängt nicht vom Inhalt der Pfade ab, nur von ihrer
//!   Struktur (dieselbe Policy mit umbenannten Pfadsegmenten ergibt denselben Text); jede
//!   Referenz `(param "KEY")` nennt einen Parameter mit Schlüssel `[A-Z0-9_]+`, und jeder
//!   Parameter wird referenziert;
//! - die Dotfile- und Muster-Masken treffen, was sie sollen, und nichts neben der Wurzel
//!   (`/a` maskiert nichts in `/ab`).

#![no_main]

use std::path::{Component, Path, PathBuf};

use beton_sandbox::resolve::{GitProtection, HiddenDotfiles};
use beton_sandbox::seatbelt::profile::{Profile, glob_regex, regex_escape, render};
use beton_sandbox::{Mask, NetMode, ResolvedPolicy, Stage};
use libfuzzer_sys::fuzz_target;
use regex::Regex;

/// Referenz: `*` beliebig viele, `?` genau ein Zeichen außer `/`, sonst wörtlich
/// (dynamische Programmierung, damit viele `*` den Harness nicht selbst ausbremsen).
fn glob_ref(glob: &[char], name: &[char]) -> bool {
    let mut row = vec![false; name.len() + 1];
    row[0] = true;
    for g in glob {
        let mut next = vec![false; name.len() + 1];
        if *g == '*' {
            next[0] = row[0];
        }
        for (j, c) in name.iter().enumerate() {
            next[j + 1] = match g {
                '*' => row[j + 1] || (next[j] && *c != '/'),
                '?' => row[j] && *c != '/',
                g => row[j] && c == g,
            };
        }
        row = next;
    }
    row[name.len()]
}

fn anchored(re: &str) -> Regex {
    Regex::new(&format!("^{re}$")).unwrap_or_else(|e| panic!("ungültiger Ausdruck {re:?}: {e}"))
}

/// Pfade kommen aus der Auflösung (SBX-001) lexikalisch normalisiert an: ohne `//`, ohne
/// `.`-Segmente, ohne `/` am Ende. Sonderzeichen (`"`, `)`, Zeilenumbruch, …) bleiben.
fn lexical(raw: &str) -> PathBuf {
    Path::new(raw).components().collect()
}

/// Dieselbe Struktur mit anderem Inhalt: jedes Segment bekommt ein `Z` angehängt.
fn rename(p: &Path) -> PathBuf {
    p.components()
        .map(|c| match c {
            Component::Normal(s) => {
                let mut s = s.to_os_string();
                s.push("Z");
                s
            }
            other => other.as_os_str().to_os_string(),
        })
        .collect()
}

fn renamed(policy: &ResolvedPolicy) -> ResolvedPolicy {
    let all = |v: &[PathBuf]| v.iter().map(|p| rename(p)).collect::<Vec<_>>();
    let mut out = policy.clone();
    out.read_roots = all(&policy.read_roots);
    out.write_roots = all(&policy.write_roots);
    out.read_only = all(&policy.read_only);
    out.no_symlinks = all(&policy.no_symlinks);
    out.unix_sockets = all(&policy.unix_sockets);
    for h in &mut out.hidden {
        h.root = rename(&h.root);
    }
    for m in &mut out.masks {
        match m {
            Mask::Path { path, .. } => *path = rename(path),
            Mask::Pattern { root, .. } => *root = rename(root),
        }
    }
    if let Some(g) = &mut out.git {
        g.dot_git = rename(&g.dot_git);
        g.git_dir = rename(&g.git_dir);
        g.common_dir = rename(&g.common_dir);
    }
    out
}

fn params<'a>(profile: &'a Profile, prefix: &str) -> Vec<&'a str> {
    profile
        .params
        .iter()
        .filter(|(k, _)| {
            k.strip_prefix(prefix)
                .and_then(|n| n.strip_prefix('_'))
                .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
        })
        .map(|(_, v)| v.as_str())
        .collect()
}

fn check_keys(profile: &Profile) {
    let mut referenced = Vec::new();
    // Kommentare des Templates erklären `(param "KEY")` selbst.
    let code: Vec<&str> = profile
        .text
        .lines()
        .filter(|l| !l.trim_start().starts_with(';'))
        .collect();
    let code = code.join("\n");
    let mut rest = code.as_str();
    while let Some(i) = rest.find("(param \"") {
        rest = &rest[i + 8..];
        let end = rest.find('"').expect("Schlüssel ohne Ende");
        let key = &rest[..end];
        assert!(
            !key.is_empty()
                && key
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'),
            "ungültiger Schlüssel {key:?}"
        );
        assert!(
            profile.params.iter().any(|(k, _)| k == key),
            "Schlüssel {key} ohne Parameter"
        );
        referenced.push(key.to_owned());
    }
    for (k, _) in &profile.params {
        assert!(
            referenced.contains(k),
            "Parameter {k} wird nie referenziert"
        );
    }
}

fn root_str(root: &Path) -> String {
    root.to_str()
        .unwrap_or_default()
        .trim_end_matches('/')
        .to_owned()
}

fuzz_target!(|data: &[u8]| {
    let mut policy = ResolvedPolicy::unsandboxed(Stage::Tools);
    let mut globs = Vec::new();
    let mut names = Vec::new();
    let mut allow = Vec::new();
    for line in beton_fuzz::lines(data).into_iter().take(64) {
        let (tag, arg) = line.split_once(' ').unwrap_or((line.as_str(), ""));
        let path = lexical(arg);
        match tag {
            "E" => {
                let re = anchored(&regex_escape(arg));
                assert!(re.is_match(arg), "regex_escape trifft {arg:?} nicht");
                assert!(!re.is_match(&format!("{arg}x")) && !re.is_match(&format!("x{arg}")));
            }
            "G" => globs.push(arg.to_owned()),
            "N" if !arg.contains('/') => names.push(arg.to_owned()),
            "A" if !arg.is_empty() && !arg.contains('/') => allow.push(arg.to_owned()),
            "R" => policy.read_roots.push(path),
            "W" => policy.write_roots.push(path),
            "H" => policy.hidden.push(HiddenDotfiles {
                root: path,
                allow: allow.clone(),
                writable: policy.hidden.len() % 2 == 1,
            }),
            "M" => policy.masks.push(Mask::Path {
                path,
                source: "fuzz".into(),
            }),
            "K" => {
                let mut parts = arg.splitn(3, '|');
                let (Some(root), Some(glob)) = (parts.next(), parts.next()) else {
                    continue;
                };
                if glob.contains('/') {
                    continue;
                }
                policy.masks.push(Mask::Pattern {
                    root: lexical(root),
                    glob: glob.to_owned(),
                    any_depth: parts.next() == Some("1"),
                    source: "fuzz".into(),
                });
            }
            "O" => policy.read_only.push(path),
            "Y" => policy.no_symlinks.push(path),
            "U" => policy.unix_sockets.push(path),
            "D" => policy.dev_ports.extend(arg.parse::<u16>().ok()),
            "X" => {
                if let Ok(port) = arg.parse() {
                    policy.net = NetMode::ProxyOnly { port };
                }
            }
            "C" => {
                policy.keychain = true;
                policy.stage = Stage::Harness;
            }
            "T" => {
                let p: Vec<&str> = arg.splitn(4, '|').collect();
                if let [dot, dir, common, file] = p.as_slice() {
                    policy.git = Some(GitProtection {
                        dot_git: lexical(dot),
                        dot_git_is_file: *file == "1",
                        git_dir: lexical(dir),
                        common_dir: lexical(common),
                    });
                }
            }
            _ => {}
        }
    }

    for g in &globs {
        let re = anchored(&glob_regex(g));
        let g: Vec<char> = g.chars().collect();
        for n in &names {
            let chars: Vec<char> = n.chars().collect();
            assert_eq!(
                re.is_match(n),
                glob_ref(&g, &chars),
                "glob_regex({:?}) gegen {n:?}",
                g.iter().collect::<String>()
            );
        }
    }

    let Ok(profile) = render(&policy) else {
        return;
    };
    check_keys(&profile);
    if let Ok(other) = render(&renamed(&policy)) {
        assert_eq!(
            profile.text, other.text,
            "Profiltext hängt vom Inhalt der Pfade ab"
        );
    }

    for (h, re) in policy.hidden.iter().zip(params(&profile, "HIDDEN")) {
        let re = Regex::new(re).unwrap_or_else(|e| panic!("HIDDEN {re:?}: {e}"));
        let root = root_str(&h.root);
        assert!(
            re.is_match(&format!("{root}/.secret")),
            "Dotfile in {root:?} nicht maskiert"
        );
        assert!(re.is_match(&format!("{root}/.secret/x")));
        assert!(!re.is_match(&format!("{root}/visible")));
        assert!(
            !re.is_match(&format!("{root}x/.secret")),
            "Maske greift neben {root:?}"
        );
    }
    let patterns = policy.masks.iter().filter_map(|m| match m {
        Mask::Pattern { root, glob, .. } => Some((root, glob)),
        Mask::Path { .. } => None,
    });
    for ((root, glob), re) in patterns.zip(params(&profile, "MASKRE")) {
        let re = Regex::new(re).unwrap_or_else(|e| panic!("MASKRE {re:?}: {e}"));
        let root = root_str(root);
        let g: Vec<char> = glob.chars().collect();
        for n in &names {
            let chars: Vec<char> = n.chars().collect();
            let hit = re.is_match(&format!("{root}/{n}"));
            assert_eq!(
                hit,
                glob_ref(&g, &chars),
                "Maske {glob:?} unter {root:?} gegen {n:?}"
            );
            assert!(
                !re.is_match(&format!("{root}x/{n}")),
                "Maske greift neben {root:?}"
            );
        }
    }
});
