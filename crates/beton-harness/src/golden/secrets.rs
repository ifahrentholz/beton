//! Secret-Scrubbing für Aufnahmen (HAR-025 AC2) und Secret-Scan über `**/golden/**`
//! (QA-003 AC2).
//!
//! Zwei Stufen: [`scrub`] ersetzt bekannte Secret-Werte, wohlgeformte Keys bekannter
//! Anbieter und `bt_cred_*`-Platzhalter. [`find_secrets`] sucht danach mit breiteren Mustern;
//! findet es noch etwas, wird die Aufnahme verworfen statt eingecheckt.

use std::sync::LazyLock;

use regex::Regex;

/// Wohlgeformte Keys, die das Scrubbing sicher ersetzt.
static SCRUB: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (
            r"sk-ant-(?:api|admin)\d{2}-[A-Za-z0-9_-]{80,120}",
            "<key:anthropic>",
        ),
        (r"sk-proj-[A-Za-z0-9_-]{40,200}", "<key:openai>"),
        (r"gh[pousr]_[A-Za-z0-9]{36,255}", "<key:github>"),
        (r"github_pat_[A-Za-z0-9_]{60,255}", "<key:github>"),
        (r"glpat-[A-Za-z0-9_-]{20,}", "<key:gitlab>"),
        (r"bt_cred_[A-Za-z0-9_]+", "<bt_cred>"),
    ]
    .into_iter()
    .filter_map(|(p, r)| Regex::new(p).ok().map(|re| (re, r)))
    .collect()
});

/// Breite Muster: was danach noch passt, gilt als Leck.
static DETECT: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (r"sk-ant-[A-Za-z0-9_-]{8,}", "Anthropic-Key (sk-ant-…)"),
        (r"sk-[A-Za-z0-9_-]{20,}", "API-Key (sk-…)"),
        (r"gh[pousr]_[A-Za-z0-9]{20,}", "GitHub-Token"),
        (r"github_pat_[A-Za-z0-9_]{20,}", "GitHub-Token"),
        (r"glpat-[A-Za-z0-9_-]{20,}", "GitLab-Token"),
        (r"xox[abpr]-[A-Za-z0-9-]{10,}", "Slack-Token"),
        (r"AKIA[0-9A-Z]{16}", "AWS-Access-Key"),
        (r"-----BEGIN [A-Z ]*PRIVATE KEY-----", "privater Schlüssel"),
    ]
    .into_iter()
    .filter_map(|(p, r)| Regex::new(p).ok().map(|re| (re, r)))
    .collect()
});

/// Ersetzt bekannte Secrets (exakte Werte zuerst) und wohlgeformte Keys.
pub fn scrub(text: &str, known_secrets: &[String]) -> String {
    let mut out = text.to_owned();
    for (i, secret) in known_secrets.iter().enumerate() {
        if secret.len() >= 8 {
            out = out.replace(secret, &format!("<secret:{}>", i + 1));
        }
    }
    for (re, replacement) in SCRUB.iter() {
        out = re.replace_all(&out, *replacement).into_owned();
    }
    out
}

/// Ein verdächtiger Fund.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub kind: &'static str,
    /// Zeile (1-basiert).
    pub line: usize,
    /// Gekürzter Ausschnitt, nie der ganze Wert.
    pub excerpt: String,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Zeile {}: {} ({})", self.line, self.kind, self.excerpt)
    }
}

/// Sucht nach Secrets, die das Scrubbing nicht erwischt hat.
pub fn find_secrets(text: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        // Überlappende Treffer (z. B. `sk-ant-…` auch als `sk-…`) nur einmal melden.
        let mut covered: Vec<std::ops::Range<usize>> = Vec::new();
        for (re, kind) in DETECT.iter() {
            for m in re.find_iter(line) {
                if covered
                    .iter()
                    .any(|r| r.start < m.end() && m.start() < r.end)
                {
                    continue;
                }
                covered.push(m.range());
                let excerpt: String = m.as_str().chars().take(10).collect();
                out.push(Finding {
                    kind,
                    line: n + 1,
                    excerpt: format!("{excerpt}…"),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anthropic_key() -> String {
        format!("sk-ant-api03-{}AA", "x".repeat(93))
    }

    #[test]
    fn well_formed_keys_and_placeholders_are_scrubbed() {
        let text = format!(
            "{{\"key\":\"{}\",\"cred\":\"bt_cred_github_1\",\"tok\":\"ghp_{}\"}}",
            anthropic_key(),
            "a".repeat(36)
        );
        let clean = scrub(&text, &[]);
        assert!(clean.contains("<key:anthropic>"));
        assert!(clean.contains("<bt_cred>"));
        assert!(clean.contains("<key:github>"));
        assert!(
            find_secrets(&clean).is_empty(),
            "{:?}",
            find_secrets(&clean)
        );
    }

    #[test]
    fn har_025_ac2_unknown_key_formats_survive_scrubbing_and_are_found() {
        for leaked in [
            "sk-ant-oat01-kurzaberecht",
            &format!("sk-{}", "Z".repeat(24)),
        ] {
            let clean = scrub(&format!("{{\"x\":\"{leaked}\"}}"), &[]);
            let findings = find_secrets(&clean);
            assert!(!findings.is_empty(), "{leaked} nicht gefunden");
            assert!(
                !findings[0].to_string().contains(leaked),
                "Fund zeigt den Wert"
            );
        }
    }

    #[test]
    fn known_secret_values_are_replaced() {
        let secret = "mein-geheimes-passwort";
        let clean = scrub(&format!("export X={secret}"), &[secret.to_owned()]);
        assert_eq!(clean, "export X=<secret:1>");
    }
}
