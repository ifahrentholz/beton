//! Shell-Parsing für `tool.command`/`tool.argv` (POL-005).
//!
//! Best effort, aber konservativ: Das Kommando wird an `;`, `&&`, `||`, `|`, `&` und
//! Zeilenumbrüchen in Teilkommandos zerlegt; Quotes und Escapes werden aufgelöst,
//! Umleitungen als Pfade gesammelt. Inhalte von `$(…)`, Backticks, Prozess-Substitution und
//! `sh -c "…"` werden rekursiv mitgeparst und als eigene Teilkommandos angehängt.
//! Was sich nicht sicher bestimmen lässt (`eval`, Kommando-Substitution, Here-Docs, offene
//! Quotes, Variablen an Kommandoposition, Shell liest Skript von stdin), setzt
//! `parse_uncertain`; eingebaute Regeltypen werten das als `ask`.

/// Ergebnis des Parsens.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parsed {
    /// Alle Teilkommandos (Programm und Argumente), auch verschachtelte.
    pub argv: Vec<Vec<String>>,
    /// Ziele von Umleitungen (`> f`, `>> f`, `< f`, `2> f`).
    pub redirects: Vec<String>,
    /// Das Kommando ließ sich nicht sicher zerlegen.
    pub uncertain: bool,
}

/// Größte Länge, die geparst wird; längere Kommandos sind `uncertain`.
const MAX_LEN: usize = 64 * 1024;
/// Größte Verschachtelungstiefe (`sh -c "$(…)"`).
const MAX_DEPTH: usize = 4;

/// Shells, deren `-c`-Argument mitgeparst wird bzw. die ohne Skript von stdin lesen.
const SHELLS: [&str; 7] = ["sh", "bash", "zsh", "dash", "ksh", "fish", "ash"];

/// Zerlegt ein Shell-Kommando.
pub fn parse(command: &str) -> Parsed {
    let mut out = Parsed::default();
    if command.len() > MAX_LEN {
        out.uncertain = true;
        parse_into(
            &command[..floor_char_boundary(command, MAX_LEN)],
            0,
            &mut out,
        );
        return out;
    }
    parse_into(command, 0, &mut out);
    out
}

fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[derive(Debug, PartialEq)]
enum Tok {
    Word(String),
    /// `;`, `&&`, `||`, `|`, `&`, Zeilenumbruch, `(`, `)`
    Sep,
    /// Umleitung; das folgende Wort ist ihr Ziel (`None` bei `>&2`).
    Redirect {
        target_follows: bool,
    },
}

fn parse_into(command: &str, depth: usize, out: &mut Parsed) {
    if depth > MAX_DEPTH {
        out.uncertain = true;
        return;
    }
    let mut nested: Vec<String> = Vec::new();
    let tokens = tokenize(command, &mut nested, out);
    let mut current: Vec<String> = Vec::new();
    let mut expect_target = false;
    let flush = |current: &mut Vec<String>, out: &mut Parsed| {
        let cmd = strip_prefix_words(std::mem::take(current));
        if !cmd.is_empty() {
            inspect(&cmd, depth, out);
            out.argv.push(cmd);
        }
    };
    for tok in tokens {
        match tok {
            Tok::Word(w) if expect_target => {
                out.redirects.push(w);
                expect_target = false;
            }
            Tok::Word(w) => current.push(w),
            Tok::Redirect { target_follows } => expect_target = target_follows,
            Tok::Sep => {
                expect_target = false;
                flush(&mut current, out);
            }
        }
    }
    flush(&mut current, out);
    for inner in nested {
        parse_into(&inner, depth + 1, out);
    }
}

/// Entfernt Shell-Schlüsselwörter und Variablenzuweisungen am Anfang.
fn strip_prefix_words(mut cmd: Vec<String>) -> Vec<String> {
    const KEYWORDS: [&str; 12] = [
        "then", "do", "else", "elif", "if", "while", "until", "!", "{", "}", "time", "fi",
    ];
    let skip = cmd
        .iter()
        .take_while(|w| KEYWORDS.contains(&w.as_str()) || is_assignment(w))
        .count();
    cmd.drain(..skip);
    if cmd
        .iter()
        .all(|w| matches!(w.as_str(), "fi" | "done" | "esac" | "}"))
    {
        cmd.clear();
    }
    cmd
}

fn is_assignment(w: &str) -> bool {
    match w.split_once('=') {
        Some((name, _)) => {
            !name.is_empty()
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !name.starts_with(|c: char| c.is_ascii_digit())
        }
        None => false,
    }
}

/// Prüft ein Teilkommando auf unsichere Konstrukte und parst `sh -c "…"` mit.
fn inspect(cmd: &[String], depth: usize, out: &mut Parsed) {
    let program = basename(&cmd[0]);
    if cmd[0].starts_with('$') || program == "eval" || program == "source" || program == "." {
        out.uncertain = true;
    }
    if SHELLS.contains(&program) {
        match cmd.iter().position(|w| {
            w == "-c" || (w.starts_with('-') && !w.starts_with("--") && w.ends_with('c'))
        }) {
            Some(i) => match cmd.get(i + 1) {
                Some(script) => parse_into(script, depth + 1, out),
                None => out.uncertain = true,
            },
            // Ohne Skriptdatei liest die Shell Kommandos von stdin (`curl … | sh`).
            None if cmd[1..].iter().all(|w| w.starts_with('-')) => out.uncertain = true,
            None => {}
        }
    }
    if program == "eval" {
        parse_into(&cmd[1..].join(" "), depth + 1, out);
    }
}

pub(crate) fn basename(w: &str) -> &str {
    w.rsplit('/').next().unwrap_or(w)
}

/// Zerlegt in Wörter, Trenner und Umleitungen; Kommando-Substitutionen landen in `nested`.
fn tokenize(s: &str, nested: &mut Vec<String>, out: &mut Parsed) -> Vec<Tok> {
    let chars: Vec<char> = s.chars().collect();
    let mut toks = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut i = 0;
    let push_word = |word: &mut String, in_word: &mut bool, toks: &mut Vec<Tok>| {
        if *in_word {
            toks.push(Tok::Word(std::mem::take(word)));
            *in_word = false;
        }
    };
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' => push_word(&mut word, &mut in_word, &mut toks),
            '\n' | ';' | '(' | ')' => {
                push_word(&mut word, &mut in_word, &mut toks);
                toks.push(Tok::Sep);
            }
            '&' | '|' => {
                // `&>` / `&>>` ist eine Umleitung
                if c == '&' && chars.get(i + 1) == Some(&'>') {
                    push_word(&mut word, &mut in_word, &mut toks);
                    i += if chars.get(i + 2) == Some(&'>') { 3 } else { 2 };
                    toks.push(Tok::Redirect {
                        target_follows: true,
                    });
                    continue;
                }
                push_word(&mut word, &mut in_word, &mut toks);
                toks.push(Tok::Sep);
                if chars.get(i + 1) == Some(&c) || (c == '|' && chars.get(i + 1) == Some(&'&')) {
                    i += 1;
                }
            }
            '<' | '>' => {
                // Prozess-Substitution `<(…)` / `>(…)`
                if chars.get(i + 1) == Some(&'(') {
                    out.uncertain = true;
                    let (inner, end) = balanced(&chars, i + 2, '(', ')');
                    nested.push(inner);
                    i = end;
                    in_word = true;
                    continue;
                }
                // Here-Doc und Here-String
                if c == '<' && chars.get(i + 1) == Some(&'<') {
                    out.uncertain = true;
                    push_word(&mut word, &mut in_word, &mut toks);
                    toks.push(Tok::Redirect {
                        target_follows: false,
                    });
                    i += 2;
                    while matches!(chars.get(i), Some('<' | '-')) {
                        i += 1;
                    }
                    continue;
                }
                // Ein reines Ziffernwort davor ist der Dateideskriptor (`2>`).
                if in_word && word.chars().all(|d| d.is_ascii_digit()) {
                    word.clear();
                    in_word = false;
                } else {
                    push_word(&mut word, &mut in_word, &mut toks);
                }
                i += 1;
                while matches!(chars.get(i), Some('>' | '|')) {
                    i += 1;
                }
                // `>&2`, `2>&1`: Deskriptor statt Datei
                if chars.get(i) == Some(&'&') {
                    i += 1;
                    while chars
                        .get(i)
                        .is_some_and(|d| d.is_ascii_digit() || *d == '-')
                    {
                        i += 1;
                    }
                    toks.push(Tok::Redirect {
                        target_follows: false,
                    });
                } else {
                    toks.push(Tok::Redirect {
                        target_follows: true,
                    });
                }
                continue;
            }
            '\\' => {
                in_word = true;
                if let Some(n) = chars.get(i + 1) {
                    if *n != '\n' {
                        word.push(*n);
                    }
                    i += 1;
                }
            }
            '\'' => {
                in_word = true;
                match chars[i + 1..].iter().position(|x| *x == '\'') {
                    Some(len) => {
                        word.extend(&chars[i + 1..i + 1 + len]);
                        i += len + 1;
                    }
                    None => {
                        out.uncertain = true;
                        word.extend(&chars[i + 1..]);
                        i = chars.len();
                    }
                }
            }
            '"' => {
                in_word = true;
                i += 1;
                let mut closed = false;
                while i < chars.len() {
                    match chars[i] {
                        '"' => {
                            closed = true;
                            break;
                        }
                        '\\' if matches!(chars.get(i + 1), Some('"' | '\\' | '$' | '`')) => {
                            word.push(chars[i + 1]);
                            i += 1;
                        }
                        '$' if chars.get(i + 1) == Some(&'(') => {
                            out.uncertain = true;
                            let (inner, end) = balanced(&chars, i + 2, '(', ')');
                            word.push_str("$(");
                            word.push_str(&inner);
                            word.push(')');
                            nested.push(inner);
                            i = end;
                            continue;
                        }
                        '`' => {
                            out.uncertain = true;
                            let end = chars[i + 1..].iter().position(|x| *x == '`');
                            let stop = end.map_or(chars.len(), |e| i + 1 + e);
                            nested.push(chars[i + 1..stop].iter().collect());
                            i = stop;
                        }
                        other => word.push(other),
                    }
                    i += 1;
                }
                if !closed {
                    out.uncertain = true;
                }
            }
            '$' if chars.get(i + 1) == Some(&'(') => {
                out.uncertain = true;
                in_word = true;
                let (inner, end) = balanced(&chars, i + 2, '(', ')');
                word.push_str("$(");
                word.push_str(&inner);
                word.push(')');
                nested.push(inner);
                i = end;
                continue;
            }
            '`' => {
                out.uncertain = true;
                in_word = true;
                let end = chars[i + 1..].iter().position(|x| *x == '`');
                let stop = end.map_or(chars.len(), |e| i + 1 + e);
                nested.push(chars[i + 1..stop].iter().collect());
                i = stop;
            }
            other => {
                in_word = true;
                word.push(other);
            }
        }
        i += 1;
    }
    push_word(&mut word, &mut in_word, &mut toks);
    toks
}

/// Inhalt bis zur passenden schließenden Klammer ab `start`; liefert (Inhalt, Index nach `close`).
fn balanced(chars: &[char], start: usize, open: char, close: char) -> (String, usize) {
    let mut depth = 1;
    let mut i = start;
    while i < chars.len() {
        if chars[i] == open {
            depth += 1;
        } else if chars[i] == close {
            depth -= 1;
            if depth == 0 {
                return (chars[start..i].iter().collect(), i + 1);
            }
        }
        i += 1;
    }
    (
        chars[start.min(chars.len())..].iter().collect(),
        chars.len(),
    )
}

/// Programme, die ein anderes Kommando ausführen; ihre Optionen werden übersprungen.
/// Wert: Optionen, die ein Argument nehmen.
const WRAPPERS: [(&str, &[&str]); 13] = [
    (
        "sudo",
        &["-u", "-g", "-C", "-D", "-p", "-r", "-t", "-U", "-h"],
    ),
    ("doas", &["-u", "-C"]),
    ("env", &["-u", "-C", "-S", "--unset", "--chdir"]),
    ("command", &[]),
    ("builtin", &[]),
    ("exec", &["-a"]),
    ("nohup", &[]),
    ("time", &["-f", "-o"]),
    ("nice", &["-n"]),
    ("ionice", &["-c", "-n", "-p"]),
    ("timeout", &["-s", "-k", "--signal", "--kill-after"]),
    ("xargs", &["-n", "-L", "-P", "-I", "-s", "-d", "-E", "-a"]),
    ("stdbuf", &["-i", "-o", "-e"]),
];

/// Git-Optionen vor dem Unterkommando, die einen Wert nehmen (`git -C dir push`).
const GIT_VALUE_OPTIONS: [&str; 6] = [
    "-C",
    "-c",
    "--git-dir",
    "--work-tree",
    "--namespace",
    "--config-env",
];

/// Das eigentliche Kommando eines Teilkommandos (Wrapper wie `sudo`/`env` übersprungen).
fn effective<'a>(cmd: &'a [&'a str]) -> &'a [&'a str] {
    let mut i = 0;
    while i < cmd.len() {
        let w = cmd[i];
        if is_assignment(w) {
            i += 1;
            continue;
        }
        let Some((_, value_opts)) = WRAPPERS.iter().find(|(name, _)| *name == basename(w)) else {
            break;
        };
        let wrapper = basename(w);
        i += 1;
        while i < cmd.len() {
            let a = cmd[i];
            if a == "--" {
                i += 1;
                break;
            }
            if is_assignment(a) && wrapper == "env" {
                i += 1;
            } else if a.starts_with('-') {
                i += if value_opts.contains(&a) { 2 } else { 1 };
            } else {
                break;
            }
        }
        // `timeout 10 cmd`: die Dauer ist ein Positionsargument
        if wrapper == "timeout" && i < cmd.len() {
            i += 1;
        }
    }
    &cmd[i.min(cmd.len())..]
}

/// Datei-Kommandos, die alle ihre Pfad-Argumente schreiben bzw. löschen (POL-016).
const WRITES_ALL: [&str; 9] = [
    "tee", "touch", "mkdir", "rm", "rmdir", "mv", "truncate", "chmod", "chown",
];
/// Datei-Kommandos, die nur ihr letztes Argument schreiben (Ziel).
const WRITES_LAST: [&str; 3] = ["cp", "ln", "install"];

/// Argumente eines Teilkommandos, die es als Datei schreibt (best effort, POL-016): alle
/// Nicht-Options-Argumente von `tee`, `rm`, `mv` …, das letzte von `cp`, `ln`, `install`.
/// Ob ein Argument ein Pfad ist, entscheidet der Aufrufer.
pub(crate) fn written_args<'a>(cmd: &'a [&'a str]) -> Vec<&'a str> {
    let eff = effective(cmd);
    let Some(first) = eff.first() else {
        return Vec::new();
    };
    let program = basename(first);
    let args: Vec<&str> = eff[1..]
        .iter()
        .copied()
        .filter(|a| !a.starts_with('-'))
        .collect();
    if WRITES_ALL.contains(&program) {
        args
    } else if WRITES_LAST.contains(&program) {
        args.last().copied().into_iter().collect()
    } else {
        Vec::new()
    }
}

/// `shell.has_command(argv, cmd, sub?)`: Kommt `cmd` (Basisname) als Programm eines
/// Teilkommandos vor, optional mit Unterkommando `sub` als erstem Nicht-Options-Argument?
pub fn has_command(argv: &[Vec<&str>], cmd: &str, sub: Option<&str>) -> bool {
    argv.iter().any(|words| {
        let eff = effective(words);
        let Some(first) = eff.first() else {
            return false;
        };
        if basename(first) != cmd {
            return false;
        }
        let Some(sub) = sub else {
            return true;
        };
        let mut i = 1;
        while i < eff.len() {
            let a = eff[i];
            if a == "--" {
                return eff.get(i + 1) == Some(&sub);
            }
            if a.starts_with('-') && a.len() > 1 {
                let takes_value = cmd == "git" && GIT_VALUE_OPTIONS.contains(&a);
                i += if takes_value { 2 } else { 1 };
                continue;
            }
            return a == sub;
        }
        false
    })
}

/// `shell.has_flag(argv, flag)`: Kommt die Option in einem Teilkommando vor? Lange
/// Optionen auch als `--flag=wert`, kurze auch gebündelt (`-rf` enthält `-f`).
pub fn has_flag(argv: &[Vec<&str>], flag: &str) -> bool {
    let short = flag
        .strip_prefix('-')
        .filter(|f| f.len() == 1 && !f.starts_with('-'));
    argv.iter().flatten().any(|w| {
        if *w == flag {
            return true;
        }
        if flag.starts_with("--") && w.starts_with(&format!("{flag}=")) {
            return true;
        }
        match short {
            Some(letter) => {
                w.starts_with('-')
                    && !w.starts_with("--")
                    && w.len() > 2
                    && w[1..].chars().all(|c| c.is_ascii_alphabetic())
                    && w[1..].contains(letter)
            }
            None => false,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(p: &Parsed) -> Vec<Vec<&str>> {
        p.argv
            .iter()
            .map(|c| c.iter().map(String::as_str).collect())
            .collect()
    }

    #[test]
    fn splits_on_operators_and_resolves_quotes() {
        let p = parse(r#"echo 'a b' "c \"d\"" e\ f; ls | wc -l || true & sleep 1"#);
        assert_eq!(
            words(&p),
            vec![
                vec!["echo", "a b", "c \"d\"", "e f"],
                vec!["ls"],
                vec!["wc", "-l"],
                vec!["true"],
                vec!["sleep", "1"],
            ]
        );
        assert!(!p.uncertain);
    }

    #[test]
    fn redirects_are_collected_and_removed_from_argv() {
        let p = parse("echo x > /etc/hosts 2>/dev/null; cat < in.txt >> out.log 2>&1 &> all");
        assert_eq!(words(&p), vec![vec!["echo", "x"], vec!["cat"]]);
        assert_eq!(
            p.redirects,
            ["/etc/hosts", "/dev/null", "in.txt", "out.log", "all"]
        );
    }

    #[test]
    fn substitutions_are_parsed_and_uncertain() {
        let p = parse("echo $(git push --force) `rm -rf /`");
        assert!(p.uncertain);
        let w = words(&p);
        assert!(w.contains(&vec!["git", "push", "--force"]));
        assert!(w.contains(&vec!["rm", "-rf", "/"]));
    }

    #[test]
    fn nested_shell_c_is_parsed() {
        let p = parse("bash -c 'cd x && git push -f'");
        assert!(!p.uncertain);
        assert!(words(&p).contains(&vec!["git", "push", "-f"]));
        let p = parse("curl -s x | sh");
        assert!(p.uncertain);
        let p = parse("sh script.sh");
        assert!(!p.uncertain);
    }

    #[test]
    fn eval_heredoc_variables_and_open_quotes_are_uncertain() {
        for cmd in [
            "eval \"git push\"",
            "cat <<EOF\nrm -rf /\nEOF",
            "$CMD push",
            "echo 'offen",
            "echo \"offen",
            "diff <(ls a) <(ls b)",
            ". ./env.sh",
        ] {
            assert!(parse(cmd).uncertain, "{cmd}");
        }
        assert!(words(&parse("eval git push")).contains(&vec!["git", "push"]));
    }

    #[test]
    fn keywords_and_assignments_are_stripped() {
        let p = parse("if true; then FOO=1 git push; fi");
        assert!(words(&p).contains(&vec!["git", "push"]));
        assert!(!words(&p).iter().any(|c| c.first() == Some(&"fi")));
    }

    #[test]
    fn has_command_skips_wrappers_and_git_options() {
        let p = parse("sudo -u root env FOO=1 git -c a=b --git-dir=/x -C . push origin");
        let w = words(&p);
        assert!(has_command(&w, "git", Some("push")));
        assert!(has_command(&w, "git", None));
        assert!(!has_command(&w, "git", Some("origin")));
        let p = parse("timeout 10 /usr/bin/git push");
        assert!(has_command(&words(&p), "git", Some("push")));
        let p = parse("xargs -n 1 git push");
        assert!(has_command(&words(&p), "git", Some("push")));
        assert!(!has_command(&[vec!["git"]], "git", Some("push")));
        assert!(has_command(
            &[vec!["git", "--", "push"]],
            "git",
            Some("push")
        ));
    }

    #[test]
    fn has_flag_variants() {
        let w = vec![vec!["git", "push", "--force-with-lease=main", "-uf"]];
        assert!(has_flag(&w, "-f"));
        assert!(has_flag(&w, "-u"));
        assert!(has_flag(&w, "--force-with-lease"));
        assert!(!has_flag(&w, "--force"));
        assert!(!has_flag(&w, "-x"));
    }

    #[test]
    fn overlong_commands_are_uncertain() {
        let long = format!("echo {}", "ä".repeat(MAX_LEN));
        assert!(parse(&long).uncertain);
    }
}
