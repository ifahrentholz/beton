//! Interne Feature-Flags (UX-007, ADR-0035).
//!
//! Unfertige Funktionen liegen hinter einem [`FeatureFlag`]. Ein Flag hat einen Reifegrad
//! ([`FlagStatus`]): `experimental` und `beta` sind nur aktiv, wenn jemand sie ausdrücklich
//! einschaltet (Config `features: [..]`, Env `BETON_FEATURES=a,b`); `stable` ist immer an;
//! `removed` wird ignoriert und als „entfernt“ gemeldet. Unbekannte Namen erzeugen eine
//! Warnung, nie einen Startabbruch.
//!
//! Die Auflösung ([`FeatureSet::resolve`]) arbeitet auf einem Katalog ([`FlagInfo`]), damit
//! sie mit eigenen Katalogen testbar ist; das Produkt nutzt [`CATALOG`].

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Umgebungsvariable mit kommagetrennten Flags, z. B. `BETON_FEATURES=voice,browser`.
pub const ENV_VAR: &str = "BETON_FEATURES";

/// Reifegrad eines Flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum FlagStatus {
    /// Unfertig; nur nach ausdrücklicher Aktivierung erreichbar.
    Experimental,
    /// Weitgehend fertig; weiterhin nur nach Aktivierung.
    Beta,
    /// Fertig und immer an; das Flag bleibt nur, damit alte Konfigurationen gültig bleiben.
    Stable,
    /// Funktion entfernt; Nennungen werden ignoriert und als „entfernt“ gemeldet.
    Removed,
}

impl FlagStatus {
    /// Deutsches Wort für Ausgaben (`beton doctor`, Einstellungen).
    pub fn label(self) -> &'static str {
        match self {
            Self::Experimental => "experimentell",
            Self::Beta => "Beta",
            Self::Stable => "stabil",
            Self::Removed => "entfernt",
        }
    }
}

/// Katalogeintrag eines Flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlagInfo {
    /// Name in Config und Env, `snake_case`.
    pub id: &'static str,
    pub status: FlagStatus,
    /// Kurzbeschreibung für Menschen.
    pub title: &'static str,
}

macro_rules! feature_flags {
    ($($(#[$m:meta])* $variant:ident = $id:literal, $status:ident, $title:literal;)*) => {
        /// Alle Feature-Flags des Produkts (UX-007). Konvention: ADR-0035.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS)]
        #[serde(rename_all = "snake_case")]
        pub enum FeatureFlag {
            $($(#[$m])* $variant,)*
        }

        /// Katalog aller Flags des Produkts.
        pub const CATALOG: &[FlagInfo] = &[
            $(FlagInfo { id: $id, status: FlagStatus::$status, title: $title },)*
        ];

        impl FeatureFlag {
            pub const ALL: &'static [FeatureFlag] = &[$(Self::$variant,)*];

            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $id,)* }
            }

            pub fn info(self) -> FlagInfo {
                match self {
                    $(Self::$variant => FlagInfo { id: $id, status: FlagStatus::$status, title: $title },)*
                }
            }
        }

        impl FromStr for FeatureFlag {
            type Err = UnknownFlag;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $($id => Ok(Self::$variant),)*
                    other => Err(UnknownFlag(other.to_owned())),
                }
            }
        }
    };
}

feature_flags! {
    /// Fake-Harness ohne `--dev` (HAR-026 AC3), z. B. für Demos und E2E gegen Release-Builds.
    /// `--dev` schaltet ihn ebenfalls ein.
    FakeHarness = "fake_harness", Experimental, "Fake-Harness für Demos und Tests";
}

impl fmt::Display for FeatureFlag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unbekanntes Feature-Flag `{0}`")]
pub struct UnknownFlag(pub String);

/// Woher die Aktivierung eines Flags stammt.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema, TS,
)]
#[serde(rename_all = "snake_case")]
pub enum FlagSource {
    /// `features:` in der Benutzer-Konfiguration.
    Config,
    /// Umgebungsvariable `BETON_FEATURES`.
    Env,
    /// Entwicklermodus (`beton serve --dev`).
    Dev,
    /// Reifegrad `stable`: immer an.
    Stable,
}

impl FlagSource {
    /// Für Ausgaben, z. B. „BETON_FEATURES (Umgebung)“.
    pub fn label(self) -> &'static str {
        match self {
            Self::Config => "Konfiguration",
            Self::Env => "BETON_FEATURES (Umgebung)",
            Self::Dev => "--dev",
            Self::Stable => "immer an",
        }
    }
}

/// Zerlegt den Wert von `BETON_FEATURES` (kommagetrennt, Leerzeichen erlaubt).
pub fn parse_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Aufgelöste Flags eines Prozesses: aktiv, unbekannt, entfernt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FeatureSet {
    active: BTreeMap<String, FlagSource>,
    unknown: Vec<(String, FlagSource)>,
    removed: Vec<(String, FlagSource)>,
    /// Bekannte IDs des Katalogs (für Vorschläge bei Tippfehlern).
    known: Vec<&'static str>,
}

impl FeatureSet {
    /// Löst Anfragen gegen einen Katalog auf. Spätere Quellen überschreiben die Herkunft
    /// früherer (Env nach Config).
    pub fn resolve<'a>(
        catalog: &[FlagInfo],
        requests: impl IntoIterator<Item = (FlagSource, &'a str)>,
    ) -> Self {
        let mut set = Self {
            known: catalog
                .iter()
                .filter(|f| f.status != FlagStatus::Removed)
                .map(|f| f.id)
                .collect(),
            ..Self::default()
        };
        for info in catalog.iter().filter(|f| f.status == FlagStatus::Stable) {
            set.active.insert(info.id.to_owned(), FlagSource::Stable);
        }
        for (source, raw) in requests {
            let id = raw.trim();
            if id.is_empty() {
                continue;
            }
            match catalog.iter().find(|f| f.id == id) {
                None => push_unique(&mut set.unknown, id, source),
                Some(info) => match info.status {
                    FlagStatus::Removed => push_unique(&mut set.removed, id, source),
                    FlagStatus::Stable => {}
                    FlagStatus::Experimental | FlagStatus::Beta => {
                        set.active.insert(id.to_owned(), source);
                    }
                },
            }
        }
        set
    }

    /// Produktkatalog aus Config (`features:`) und dem Wert von `BETON_FEATURES`.
    pub fn from_sources(config: &[String], env: Option<&str>) -> Self {
        let env = env.map(parse_list).unwrap_or_default();
        Self::resolve(
            CATALOG,
            config
                .iter()
                .map(|s| (FlagSource::Config, s.as_str()))
                .chain(env.iter().map(|s| (FlagSource::Env, s.as_str()))),
        )
    }

    /// Schaltet ein Flag zusätzlich ein (z. B. `fake_harness` durch `--dev`).
    pub fn activate(&mut self, flag: FeatureFlag, source: FlagSource) {
        if flag.info().status != FlagStatus::Removed {
            self.active
                .entry(flag.as_str().to_owned())
                .or_insert(source);
        }
    }

    pub fn is_active(&self, flag: FeatureFlag) -> bool {
        self.is_active_id(flag.as_str())
    }

    pub fn is_active_id(&self, id: &str) -> bool {
        self.active.contains_key(id)
    }

    /// Aktive Flags mit Herkunft, nach Name sortiert.
    pub fn active(&self) -> impl Iterator<Item = (&str, FlagSource)> {
        self.active.iter().map(|(k, v)| (k.as_str(), *v))
    }

    /// Namen der aktiven Flags (für `GET /v1/info`).
    pub fn active_ids(&self) -> Vec<String> {
        self.active.keys().cloned().collect()
    }

    pub fn unknown(&self) -> &[(String, FlagSource)] {
        &self.unknown
    }

    pub fn removed(&self) -> &[(String, FlagSource)] {
        &self.removed
    }

    /// Ähnlichster bekannter Name für einen Tippfehler (Abstand höchstens 2).
    pub fn suggestion(&self, unknown: &str) -> Option<&'static str> {
        self.known
            .iter()
            .map(|k| (distance(k, unknown), *k))
            .filter(|(d, _)| *d <= 2)
            .min()
            .map(|(_, k)| k)
    }

    /// Unbekannte und entfernte Nennungen, in dieser Reihenfolge.
    pub fn problems(&self) -> Vec<FlagProblem> {
        let unknown = self.unknown.iter().map(|(id, source)| FlagProblem {
            id: id.clone(),
            source: *source,
            removed: false,
            suggestion: self.suggestion(id),
        });
        let removed = self.removed.iter().map(|(id, source)| FlagProblem {
            id: id.clone(),
            source: *source,
            removed: true,
            suggestion: None,
        });
        unknown.chain(removed).collect()
    }

    /// Warnungen für das Log: unbekannte (mit Vorschlag) und entfernte Flags.
    pub fn warnings(&self) -> Vec<String> {
        self.problems()
            .iter()
            .map(|p| match p.suggestion {
                Some(s) => format!("{} (meintest du „{s}“?)", p.message()),
                None => p.message(),
            })
            .collect()
    }
}

/// Ein Flag, das genannt, aber nicht eingeschaltet wurde.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlagProblem {
    pub id: String,
    pub source: FlagSource,
    /// `true`: Reifegrad `removed`; sonst unbekannt.
    pub removed: bool,
    /// Ähnlicher bekannter Name bei einem Tippfehler.
    pub suggestion: Option<&'static str>,
}

impl FlagProblem {
    /// Z. B. „BETON_FEATURES enthält unbekanntes Flag „brwoser“ – ignoriert“.
    pub fn message(&self) -> String {
        if self.removed {
            format!(
                "{} nennt „{}“: entfernt, wird ignoriert",
                origin(self.source),
                self.id
            )
        } else {
            format!(
                "{} enthält unbekanntes Flag „{}“ – ignoriert",
                origin(self.source),
                self.id
            )
        }
    }
}

fn origin(source: FlagSource) -> &'static str {
    match source {
        FlagSource::Env => ENV_VAR,
        FlagSource::Config => "features: (Konfiguration)",
        FlagSource::Dev => "--dev",
        FlagSource::Stable => "Katalog",
    }
}

fn push_unique(list: &mut Vec<(String, FlagSource)>, id: &str, source: FlagSource) {
    if !list.iter().any(|(i, _)| i == id) {
        list.push((id.to_owned(), source));
    }
}

/// Levenshtein-Abstand (kurze Namen, daher ohne Optimierung).
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST: &[FlagInfo] = &[
        FlagInfo {
            id: "browser",
            status: FlagStatus::Beta,
            title: "Browser",
        },
        FlagInfo {
            id: "voice",
            status: FlagStatus::Experimental,
            title: "Sprache",
        },
        FlagInfo {
            id: "score_view",
            status: FlagStatus::Stable,
            title: "Partitur",
        },
        FlagInfo {
            id: "legacy_pty",
            status: FlagStatus::Removed,
            title: "Altes PTY",
        },
    ];

    #[test]
    fn ux_007_ac1_experimental_flag_is_off_without_explicit_activation() {
        let none = FeatureSet::resolve(TEST, []);
        assert!(!none.is_active_id("voice"));
        assert!(!none.is_active_id("browser"));
        assert!(none.is_active_id("score_view"), "stable ist immer an");
        let on = FeatureSet::resolve(TEST, [(FlagSource::Env, "voice")]);
        assert!(on.is_active_id("voice"));
        assert_eq!(
            on.active().collect::<Vec<_>>(),
            vec![
                ("score_view", FlagSource::Stable),
                ("voice", FlagSource::Env)
            ]
        );
        // Produktkatalog: `fake_harness` ist experimentell und ohne Aktivierung aus.
        assert!(!FeatureSet::from_sources(&[], None).is_active(FeatureFlag::FakeHarness));
        assert!(
            FeatureSet::from_sources(&[], Some("fake_harness")).is_active(FeatureFlag::FakeHarness)
        );
    }

    #[test]
    fn ux_007_ac2_unknown_flag_warns_and_suggests() {
        let set = FeatureSet::resolve(
            TEST,
            [
                (FlagSource::Env, "unknown_flag"),
                (FlagSource::Env, "brwoser"),
            ],
        );
        assert_eq!(set.unknown().len(), 2);
        let w = set.warnings();
        assert!(
            w[0].contains("BETON_FEATURES") && w[0].contains("unknown_flag"),
            "{w:?}"
        );
        assert!(w[1].contains("meintest du „browser“"), "{w:?}");
        assert!(!set.is_active_id("unknown_flag"));
    }

    #[test]
    fn ux_007_ac3_removed_flags_are_ignored_and_reported() {
        let set = FeatureSet::resolve(TEST, [(FlagSource::Config, "legacy_pty")]);
        assert!(!set.is_active_id("legacy_pty"));
        assert_eq!(
            set.removed(),
            &[("legacy_pty".to_owned(), FlagSource::Config)]
        );
        assert!(set.warnings()[0].contains("entfernt"));
        assert_eq!(FlagStatus::Removed.label(), "entfernt");
        // Entfernte Namen werden nicht als Vorschlag angeboten.
        assert_eq!(set.suggestion("legacy_ptx"), None);
    }

    #[test]
    fn env_list_is_comma_separated_and_trimmed() {
        assert_eq!(parse_list(" voice , browser,,"), vec!["voice", "browser"]);
        let set = FeatureSet::from_sources(&["fake_harness".into()], Some("fake_harness"));
        assert_eq!(set.active().count(), 1);
        assert_eq!(
            set.active().next(),
            Some(("fake_harness", FlagSource::Env)),
            "Env überschreibt die Herkunft"
        );
    }

    #[test]
    fn catalog_and_enum_agree() {
        assert_eq!(CATALOG.len(), FeatureFlag::ALL.len());
        for f in FeatureFlag::ALL {
            assert_eq!(f.as_str().parse::<FeatureFlag>(), Ok(*f));
            assert!(CATALOG.contains(&f.info()));
        }
    }
}
