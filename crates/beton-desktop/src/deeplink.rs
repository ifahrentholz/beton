//! Deep-Links `beton://` (DESK-006).
//!
//! - `beton://local/s/<session_id>[?seq=N][&comment=<id>]` öffnet die Session in der laufenden
//!   App (Single-Instance), mit `seq` an Event N (AC1, AC3).
//! - Ein Link auf einen anderen Host oder `beton://pair?…` braucht Server-Profile (DESK-003,
//!   ab M4): Er führt nie zu einer Verbindung, bevor der User zugestimmt hat (AC2). Bis dahin
//!   zeigt die App einen Hinweis ([`DeepLink::confirmation`]), öffnet ihn nicht und baut keine
//!   Verbindung auf.
//! - Ein Link öffnet nur Ansichten. Er entscheidet keine Freigabe, sendet nichts und startet
//!   nichts; alles andere (unbekannte Pfade, Parameter mit Aktionen) wird abgelehnt.

use beton_core::id::SessionId;

/// Das einzige Profil bis M4.
pub const LOCAL_PROFILE: &str = "local";

/// Was ein Link öffnen soll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepLink {
    /// Session im Profil `local`, optional an Event `seq` bzw. Kommentar `comment`.
    Session {
        session_id: SessionId,
        seq: Option<u64>,
        comment: Option<String>,
    },
    /// Host ohne Profil: nur nach Bestätigung (ab M4), vorher kein Netzwerkzugriff.
    UnknownServer { host: String },
    /// Koppeln mit einem anderen Rechner (ab M4), nur nach Bestätigung.
    Pair { server: String, code: String },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeepLinkError {
    #[error("kein beton://-Link")]
    Scheme,
    #[error("Pfad `{0}` gibt es nicht")]
    Path(String),
    #[error("Parameter `{0}` ist ungültig")]
    Param(String),
}

fn decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = s.get(i + 1..i + 3)?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn query_pairs(query: &str) -> Result<Vec<(String, String)>, DeepLinkError> {
    query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            match (decode(k), decode(v)) {
                (Some(k), Some(v)) => Ok((k, v)),
                _ => Err(DeepLinkError::Param(p.to_owned())),
            }
        })
        .collect()
}

/// Liest einen `beton://`-Link.
pub fn parse(url: &str) -> Result<DeepLink, DeepLinkError> {
    let rest = url
        .trim()
        .strip_prefix("beton://")
        .or_else(|| url.trim().strip_prefix("BETON://"))
        .ok_or(DeepLinkError::Scheme)?;
    let rest = rest.split('#').next().unwrap_or_default();
    let (before_query, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (host, path) = before_query.split_once('/').unwrap_or((before_query, ""));
    let host = host.to_ascii_lowercase();
    let params = query_pairs(query)?;
    let param = |name: &str| {
        params
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    };
    if host == "pair" {
        let (Some(server), Some(code)) = (param("server"), param("code")) else {
            return Err(DeepLinkError::Param("server, code".into()));
        };
        return Ok(DeepLink::Pair { server, code });
    }
    if host.is_empty() {
        return Err(DeepLinkError::Path(before_query.to_owned()));
    }
    if host != LOCAL_PROFILE {
        return Ok(DeepLink::UnknownServer { host });
    }
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let [kind, id] = segments.as_slice() else {
        return Err(DeepLinkError::Path(path.to_owned()));
    };
    if *kind != "s" {
        return Err(DeepLinkError::Path(path.to_owned()));
    }
    let session_id: SessionId = id
        .parse()
        .map_err(|_| DeepLinkError::Path(path.to_owned()))?;
    for (k, _) in &params {
        if k != "seq" && k != "comment" {
            return Err(DeepLinkError::Param(k.clone()));
        }
    }
    let seq = match param("seq") {
        Some(s) => Some(
            s.parse::<u64>()
                .map_err(|_| DeepLinkError::Param("seq".into()))?,
        ),
        None => None,
    };
    let comment = param("comment");
    if comment.as_deref().is_some_and(|c| {
        c.is_empty()
            || c.len() > 64
            || !c
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    }) {
        return Err(DeepLinkError::Param("comment".into()));
    }
    Ok(DeepLink::Session {
        session_id,
        seq,
        comment,
    })
}

/// Wozu ein Link erst die Zustimmung des Users braucht (DESK-006 AC2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationKind {
    /// Session auf einem Server ohne Profil.
    Server,
    /// Koppeln mit einem anderen Rechner.
    Pair,
}

/// Inhalt des Dialogs, den ein Link auf einen anderen Server auslöst. Bis zur Zustimmung
/// (ab M4: Profil anlegen bzw. koppeln) baut die App keine Verbindung auf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirmation {
    pub kind: ConfirmationKind,
    /// Der Server, z. B. `beton.team.example.com`.
    pub host: String,
    /// Der Link zur Anzeige (höchstens 200 Zeichen).
    pub link: String,
}

fn host_of(server: &str) -> String {
    let rest = server.split_once("://").map_or(server, |(_, r)| r);
    rest.split(['/', '?', '#'])
        .next()
        .unwrap_or(rest)
        .to_owned()
}

impl DeepLink {
    /// Dialog vor dem Öffnen; `None` für Links, die nur eine lokale Ansicht öffnen.
    pub fn confirmation(&self, raw: &str) -> Option<Confirmation> {
        let (kind, host) = match self {
            Self::Session { .. } => return None,
            Self::UnknownServer { host } => (ConfirmationKind::Server, host.clone()),
            Self::Pair { server, .. } => (ConfirmationKind::Pair, host_of(server)),
        };
        let link: String = raw.trim().chars().take(200).collect();
        Some(Confirmation { kind, host, link })
    }

    /// Route der Web-UI, die der Link öffnet; `None`, wenn erst der User zustimmen muss.
    /// `opened=link` lässt die Web-UI den Hinweis „Geöffnet über beton://…“ zeigen.
    pub fn route(&self) -> Option<String> {
        match self {
            Self::Session {
                session_id,
                seq,
                comment,
            } => {
                let mut q = vec!["opened=link".to_owned()];
                if let Some(seq) = seq {
                    q.insert(0, format!("seq={seq}"));
                }
                if let Some(c) = comment {
                    q.push(format!("comment={c}"));
                }
                Some(format!("/s/{session_id}?{}", q.join("&")))
            }
            Self::UnknownServer { .. } | Self::Pair { .. } => None,
        }
    }

    /// Die Session, die der Link öffnet.
    pub fn session(&self) -> Option<SessionId> {
        match self {
            Self::Session { session_id, .. } => Some(*session_id),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sid() -> SessionId {
        "ses_01JB8Y2D0M3K4J5H6G7F8E9D0C".parse().unwrap()
    }

    #[test]
    fn desk_006_ac1_session_link_opens_the_session_route() {
        let link = parse("beton://local/s/ses_01JB8Y2D0M3K4J5H6G7F8E9D0C").unwrap();
        assert_eq!(link.session(), Some(sid()));
        assert_eq!(
            link.route().unwrap(),
            "/s/ses_01JB8Y2D0M3K4J5H6G7F8E9D0C?opened=link"
        );
        // Abschließender Schrägstrich und Großschreibung des Hosts sind egal.
        assert_eq!(
            parse("beton://LOCAL/s/ses_01JB8Y2D0M3K4J5H6G7F8E9D0C/").unwrap(),
            link
        );
    }

    #[test]
    fn desk_006_ac3_seq_and_comment_anchor_end_up_in_the_route() {
        let link =
            parse("beton://local/s/ses_01JB8Y2D0M3K4J5H6G7F8E9D0C?seq=214&comment=c_42").unwrap();
        assert_eq!(
            link,
            DeepLink::Session {
                session_id: sid(),
                seq: Some(214),
                comment: Some("c_42".into())
            }
        );
        assert_eq!(
            link.route().unwrap(),
            "/s/ses_01JB8Y2D0M3K4J5H6G7F8E9D0C?seq=214&opened=link&comment=c_42"
        );
        assert!(parse("beton://local/s/ses_01JB8Y2D0M3K4J5H6G7F8E9D0C?seq=-1").is_err());
        assert!(
            parse("beton://local/s/ses_01JB8Y2D0M3K4J5H6G7F8E9D0C?comment=%3Cscript%3E").is_err()
        );
    }

    #[test]
    fn desk_006_ac2_unknown_host_needs_confirmation_and_has_no_route() {
        let link = parse("beton://beton.team.example.com/s/ses_9a1b?comment=c_42").unwrap();
        assert_eq!(
            link,
            DeepLink::UnknownServer {
                host: "beton.team.example.com".into()
            }
        );
        assert_eq!(link.route(), None, "keine Verbindung vor Zustimmung");
        let pair =
            parse("beton://pair?server=https%3A%2F%2Fingo-laptop.tail5c2e.ts.net&code=K7Q-4MX")
                .unwrap();
        assert_eq!(
            pair,
            DeepLink::Pair {
                server: "https://ingo-laptop.tail5c2e.ts.net".into(),
                code: "K7Q-4MX".into()
            }
        );
        assert_eq!(pair.route(), None);
    }

    #[test]
    fn desk_006_ac2_server_links_ask_first_and_name_the_server() {
        let raw = "beton://beton.team.example.com/s/ses_9a1b?comment=c_42";
        let c = parse(raw).unwrap().confirmation(raw).unwrap();
        assert_eq!(c.kind, ConfirmationKind::Server);
        assert_eq!(c.host, "beton.team.example.com");
        assert_eq!(c.link, raw);
        let raw = "beton://pair?server=https%3A%2F%2Fingo-laptop.tail5c2e.ts.net&code=K7Q-4MX";
        let c = parse(raw).unwrap().confirmation(raw).unwrap();
        assert_eq!(c.kind, ConfirmationKind::Pair);
        assert_eq!(c.host, "ingo-laptop.tail5c2e.ts.net");
        let local = "beton://local/s/ses_01JB8Y2D0M3K4J5H6G7F8E9D0C";
        assert_eq!(
            parse(local).unwrap().confirmation(local),
            None,
            "lokale Links öffnen direkt"
        );
        let long = format!("beton://x.example/{}", "a".repeat(500));
        assert!(
            parse(&long)
                .unwrap()
                .confirmation(&long)
                .unwrap()
                .link
                .chars()
                .count()
                <= 200
        );
    }

    #[test]
    fn desk_006_links_open_views_only_and_never_trigger_actions() {
        for bad in [
            "https://local/s/ses_01JB8Y2D0M3K4J5H6G7F8E9D0C",
            "beton://local/s/ses_01JB8Y2D0M3K4J5H6G7F8E9D0C?approve=apr_1",
            "beton://local/s/ses_01JB8Y2D0M3K4J5H6G7F8E9D0C?input=rm%20-rf",
            "beton://local/s/nicht-eine-id",
            "beton://local/approvals/apr_1/approve",
            "beton://local/settings",
            "beton:///s/ses_01JB8Y2D0M3K4J5H6G7F8E9D0C",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
}
