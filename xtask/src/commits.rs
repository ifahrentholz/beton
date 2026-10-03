//! Commit-Lint (QA-014) und DCO-Prüfung (QA-015).

use std::sync::LazyLock;

use regex::Regex;

use crate::spec::{PREFIXES, Spec, id_regex};

/// Ein Commit, wie ihn `git log` liefert.
#[derive(Debug, Clone)]
pub struct Commit {
    pub hash: String,
    pub author_name: String,
    pub author_email: String,
    pub message: String,
}

static SUBJECT: LazyLock<Regex> = LazyLock::new(|| {
    let ids = PREFIXES.join("|");
    Regex::new(&format!(
        r"^(feat|fix|perf|refactor|test|docs|build|ci|chore)(\([a-z0-9-]+\))?!?: ((?:{ids})-\d{{3}}(?:,\s?(?:{ids})-\d{{3}})* )?.+"
    ))
    .expect("gültige Regex")
});

static SIGNOFF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^Signed-off-by:\s*(.+?)\s*<([^>]+)>\s*$").expect("gültige Regex")
});

static IMPLEMENTS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^Implements:\s*(.+)$").expect("gültige Regex"));

/// Prüft einen Commit-Betreff bzw. PR-Titel und die `Implements:`-Zeilen des Bodys.
pub fn lint_message(message: &str, spec: &Spec) -> Vec<String> {
    let mut errors = Vec::new();
    let subject = message.lines().next().unwrap_or_default().trim();
    let Some(caps) = SUBJECT.captures(subject) else {
        errors.push(format!(
            "Betreff entspricht nicht `<type>(<scope>): <ID> <Beschreibung>`: `{subject}`"
        ));
        return errors;
    };
    if &caps[1] == "feat" && caps.get(3).is_none() {
        errors.push(format!(
            "`feat` braucht eine Feature-ID (z. B. `POL-003`): `{subject}`"
        ));
    }
    let id_re = id_regex();
    let mut referenced: Vec<&str> = caps
        .get(3)
        .map(|m| id_re.find_iter(m.as_str()).map(|m| m.as_str()).collect())
        .unwrap_or_default();
    for line in IMPLEMENTS.captures_iter(message) {
        let line = line.get(1).map(|m| m.as_str()).unwrap_or_default();
        referenced.extend(id_re.find_iter(line).map(|m| m.as_str()));
    }
    for id in referenced {
        if !spec.contains(id) {
            errors.push(format!("Feature-ID {id} existiert nicht in docs/spec"));
        }
    }
    errors
}

/// Prüft das DCO-Sign-off: Name und E-Mail müssen zum Autor passen.
pub fn check_dco(commit: &Commit) -> Option<String> {
    let short = &commit.hash[..commit.hash.len().min(10)];
    let signoffs: Vec<_> = SIGNOFF
        .captures_iter(&commit.message)
        .map(|c| (c[1].to_owned(), c[2].to_owned()))
        .collect();
    if signoffs.is_empty() {
        return Some(format!(
            "{short}: `Signed-off-by` fehlt (git commit -s; nachholen mit git rebase --signoff)"
        ));
    }
    let matches = signoffs.iter().any(|(name, email)| {
        name.trim() == commit.author_name.trim()
            && email
                .trim()
                .eq_ignore_ascii_case(commit.author_email.trim())
    });
    if matches {
        None
    } else {
        Some(format!(
            "{short}: `Signed-off-by` passt nicht zum Autor {} <{}>",
            commit.author_name, commit.author_email
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::tests::sample_spec;

    fn commit(message: &str, name: &str, email: &str) -> Commit {
        Commit {
            hash: "0123456789abcdef".into(),
            author_name: name.into(),
            author_email: email.into(),
            message: message.into(),
        }
    }

    #[test]
    fn qa_014_ac1_feat_without_feature_id_is_rejected() {
        let (_d, spec) = sample_spec();
        let errors = lint_message("feat(policy): hierarchische Auswertung", &spec);
        assert!(
            errors.iter().any(|e| e.contains("braucht eine Feature-ID")),
            "{errors:?}"
        );
        assert!(lint_message("feat(policy): POL-001 hierarchische Auswertung", &spec).is_empty());
    }

    #[test]
    fn qa_014_ac1_other_types_without_id_are_allowed_and_format_is_enforced() {
        let (_d, spec) = sample_spec();
        assert!(lint_message("docs: README ergänzt", &spec).is_empty());
        assert!(lint_message("chore(scripts): add generator", &spec).is_empty());
        assert!(!lint_message("Update README", &spec).is_empty());
        assert!(!lint_message("feature: POL-001 x", &spec).is_empty());
    }

    #[test]
    fn qa_014_ac2_unknown_feature_id_is_rejected() {
        let (_d, spec) = sample_spec();
        let errors = lint_message("feat(policy): POL-999 gibt es nicht", &spec);
        assert!(errors.iter().any(|e| e.contains("POL-999")), "{errors:?}");
        let errors = lint_message(
            "fix(policy): POL-001 Randfall\n\nImplements: POL-001 AC1, POL-998 AC2",
            &spec,
        );
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("POL-998"));
    }

    #[test]
    fn qa_015_ac1_commit_without_signoff_is_rejected() {
        let c = commit("docs: x", "Ada", "ada@example.com");
        assert!(check_dco(&c).unwrap().contains("fehlt"));
    }

    #[test]
    fn qa_015_ac2_signoff_with_different_email_is_rejected() {
        let c = commit(
            "docs: x\n\nSigned-off-by: Ada <other@example.com>",
            "Ada",
            "ada@example.com",
        );
        assert!(check_dco(&c).unwrap().contains("passt nicht"));
        let ok = commit(
            "docs: x\n\nCo-Authored-By: Bot <bot@example.com>\nSigned-off-by: Ada <ADA@example.com>",
            "Ada",
            "ada@example.com",
        );
        assert_eq!(check_dco(&ok), None);
    }
}
