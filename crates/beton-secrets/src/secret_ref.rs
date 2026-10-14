//! SecretRefs (SEC-001): `secret://<user|team:<slug>|org>/<provider>/<account>[#name]`.
//!
//! Provider sind Hostnamen (`github.com`, `gitlab.example.com`), Kurznamen (`openai`,
//! `anthropic`, `npm`) oder `custom:<name>`. Lokal gilt `org = local`, `user = usr_local`;
//! die Bindung an Org und User übernimmt der Store, nicht die Ref.

use std::fmt;
use std::str::FromStr;

use crate::error::Error;

const PREFIX: &str = "secret://";
/// Höchstlänge einer Ref (Keychain-Accounts sind begrenzt).
pub const MAX_REF_LEN: usize = 256;

/// Geltungsbereich eines Secrets.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SecretScope {
    User,
    Team(String),
    Org,
}

impl SecretScope {
    /// `user`, `team` oder `org` (ohne Slug).
    pub fn kind(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Team(_) => "team",
            Self::Org => "org",
        }
    }
}

impl fmt::Display for SecretScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::User => f.write_str("user"),
            Self::Team(slug) => write!(f, "team:{slug}"),
            Self::Org => f.write_str("org"),
        }
    }
}

/// Adresse eines Secrets; enthält nie den Wert.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SecretRef {
    pub scope: SecretScope,
    pub provider: String,
    pub account: String,
    pub name: Option<String>,
}

fn slug_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn provider_ok(s: &str) -> bool {
    if let Some(name) = s.strip_prefix("custom:") {
        return slug_ok(name);
    }
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
}

fn account_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'@' | b'+'))
}

fn name_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

impl SecretRef {
    /// Ref im Bereich `user`, z. B. `secret://user/openai/default`.
    pub fn user(provider: &str, account: &str, name: Option<&str>) -> Result<Self, Error> {
        let r = Self {
            scope: SecretScope::User,
            provider: provider.to_owned(),
            account: account.to_owned(),
            name: name.map(str::to_owned),
        };
        r.to_string().parse()
    }
}

impl fmt::Display for SecretRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{PREFIX}{}/{}/{}",
            self.scope, self.provider, self.account
        )?;
        if let Some(name) = &self.name {
            write!(f, "#{name}")?;
        }
        Ok(())
    }
}

impl FromStr for SecretRef {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let invalid = |why: &str| {
            Error::InvalidRef(format!(
                "{why} (erwartet secret://<user|team:<slug>|org>/<provider>/<account>[#name])"
            ))
        };
        if s.len() > MAX_REF_LEN {
            return Err(invalid("Ref zu lang"));
        }
        let rest = s
            .strip_prefix(PREFIX)
            .ok_or_else(|| invalid("Präfix secret:// fehlt"))?;
        let (path, name) = match rest.split_once('#') {
            Some((p, n)) => {
                if !name_ok(n) {
                    return Err(invalid("ungültiger Name"));
                }
                (p, Some(n.to_owned()))
            }
            None => (rest, None),
        };
        let parts: Vec<&str> = path.split('/').collect();
        let [scope, provider, account] = parts.as_slice() else {
            return Err(invalid("falsche Anzahl Segmente"));
        };
        let scope = match *scope {
            "user" => SecretScope::User,
            "org" => SecretScope::Org,
            other => match other.strip_prefix("team:") {
                Some(slug) if slug_ok(slug) => SecretScope::Team(slug.to_owned()),
                _ => return Err(invalid("unbekannter Bereich")),
            },
        };
        if !provider_ok(provider) {
            return Err(invalid("ungültiger Provider"));
        }
        if !account_ok(account) {
            return Err(invalid("ungültiger Account"));
        }
        Ok(Self {
            scope,
            provider: (*provider).to_owned(),
            account: (*account).to_owned(),
            name,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sec_001_parses_and_prints_refs() {
        for text in [
            "secret://user/github.com/ifahrentholz",
            "secret://user/openai/default",
            "secret://team:platform/gitlab.example.com/deploy-bot#ci",
            "secret://org/npm/registry",
            "secret://user/custom:hetzner/api-token",
            "secret://user/anthropic/me@example.com",
        ] {
            let r: SecretRef = text.parse().unwrap();
            assert_eq!(r.to_string(), text);
        }
        let r: SecretRef = "secret://team:platform/gitlab.example.com/deploy-bot#ci"
            .parse()
            .unwrap();
        assert_eq!(r.scope, SecretScope::Team("platform".into()));
        assert_eq!(r.provider, "gitlab.example.com");
        assert_eq!(r.account, "deploy-bot");
        assert_eq!(r.name.as_deref(), Some("ci"));
    }

    #[test]
    fn sec_001_rejects_malformed_refs() {
        for text in [
            "",
            "github.com/x",
            "secret://",
            "secret://user/github.com",
            "secret://user//x",
            "secret://user/github.com/",
            "secret://admin/github.com/x",
            "secret://team:/github.com/x",
            "secret://team:Plat form/github.com/x",
            "secret://user/github.com/a/b",
            "secret://user/Git Hub/x",
            "secret://user/github.com/x#",
            "secret://user/github.com/x#a#b",
            "secret://user/github.com/x y",
            "secret://user/github.com/x\n",
            "secret://user/custom:/x",
        ] {
            assert!(
                text.parse::<SecretRef>().is_err(),
                "{text:?} sollte ungültig sein"
            );
        }
        let long = format!("secret://user/openai/{}", "a".repeat(300));
        assert!(long.parse::<SecretRef>().is_err());
    }

    #[test]
    fn sec_001_builds_a_user_ref_for_a_provider() {
        let r = SecretRef::user("openai", "default", None).unwrap();
        assert_eq!(r.to_string(), "secret://user/openai/default");
        assert!(SecretRef::user("open ai", "default", None).is_err());
    }
}
