//! Registry aller netzwirksamen Funktionen (QA-018, ADR-0033).
//!
//! beton läuft vollständig lokal. Jede Funktion, die eine Verbindung über Loopback hinaus
//! aufbaut oder aufbauen lässt, steht hier mit Zweck, Default und Offline-Alternative.
//! Einziger Default „an“ sind die Modell-Anbieter der Harnesses; alles andere ist aus oder
//! passiert nur auf ausdrückliche Nutzeraktion. Die Prüfung läuft zur Compile-Zeit: Ein
//! Eintrag ohne Zweck oder Offline-Alternative oder mit unerlaubtem Default bricht den Build.

/// Default einer netzwirksamen Funktion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetDefault {
    /// Aktiv ohne Zutun des Nutzers. Nur für Modell-Anbieter erlaubt.
    On,
    /// Aus, bis der Nutzer sie in der Konfiguration einschaltet.
    Off,
    /// Läuft nur, wenn der Nutzer die Aktion ausdrücklich auslöst.
    UserAction,
}

/// Eine netzwirksame Funktion.
#[derive(Debug, Clone, Copy)]
pub struct NetFunction {
    /// Stabiler Name, z. B. `harness.claude`.
    pub name: &'static str,
    /// Wozu die Verbindung dient.
    pub purpose: &'static str,
    /// Default nach der Installation.
    pub default: NetDefault,
    /// Wie dasselbe ohne Netz geht (oder was stattdessen passiert).
    pub offline_alternative: &'static str,
    /// Modell-Anbieter eines Harness (der einzige erlaubte Default „an“).
    pub model_provider: bool,
}

/// Alle netzwirksamen Funktionen von beton. Neue Funktionen mit Netzzugriff gehören hierher.
pub const NETWORK_FUNCTIONS: &[NetFunction] = &[
    NetFunction {
        name: "harness.claude",
        purpose: "Die Claude-Code-CLI spricht mit der Subscription des Nutzers beim Modell-Anbieter; beton selbst baut keine Verbindung auf (ADR-0005).",
        default: NetDefault::On,
        offline_alternative: "Fake-Harness (`beton run fake`) bzw. Fake-CLI über `BETON_CLAUDE_PATH`.",
        model_provider: true,
    },
    NetFunction {
        name: "server.bind",
        purpose: "`beton serve --bind <adresse>` macht Server und Web-UI außerhalb von Loopback erreichbar.",
        default: NetDefault::UserAction,
        offline_alternative: "Standard-Bindung an 127.0.0.1; Web-UI und CLI arbeiten lokal.",
        model_provider: false,
    },
];

/// Grund, aus dem ein Eintrag ungültig ist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryError {
    MissingName,
    MissingPurpose,
    MissingOfflineAlternative,
    /// Default „an“ ohne Modell-Anbieter zu sein.
    DefaultOn,
}

/// Prüft einen Eintrag (auch zur Compile-Zeit nutzbar).
pub const fn check(f: &NetFunction) -> Result<(), RegistryError> {
    if f.name.is_empty() {
        return Err(RegistryError::MissingName);
    }
    if f.purpose.is_empty() {
        return Err(RegistryError::MissingPurpose);
    }
    if f.offline_alternative.is_empty() {
        return Err(RegistryError::MissingOfflineAlternative);
    }
    if matches!(f.default, NetDefault::On) && !f.model_provider {
        return Err(RegistryError::DefaultOn);
    }
    Ok(())
}

/// Prüft alle Einträge; liefert den Index des ersten ungültigen.
pub const fn check_all(all: &[NetFunction]) -> Result<(), (usize, RegistryError)> {
    let mut i = 0;
    while i < all.len() {
        if let Err(e) = check(&all[i]) {
            return Err((i, e));
        }
        i += 1;
    }
    Ok(())
}

// Build-Gate (QA-018 AC3): ein ungültiger Eintrag ist ein Compile-Fehler.
const _: () = assert!(
    check_all(NETWORK_FUNCTIONS).is_ok(),
    "NETWORK_FUNCTIONS: jeder Eintrag braucht Name, Zweck und Offline-Alternative; Default „an“ nur für Modell-Anbieter"
);

#[cfg(test)]
mod tests {
    use super::*;

    const fn entry(default: NetDefault, model_provider: bool) -> NetFunction {
        NetFunction {
            name: "x",
            purpose: "y",
            default,
            offline_alternative: "z",
            model_provider,
        }
    }

    #[test]
    fn qa_018_ac3_registry_only_model_providers_default_on() {
        assert_eq!(check_all(NETWORK_FUNCTIONS), Ok(()));
        for f in NETWORK_FUNCTIONS {
            if !f.model_provider {
                assert_ne!(f.default, NetDefault::On, "{}", f.name);
            }
            assert!(!f.offline_alternative.trim().is_empty(), "{}", f.name);
        }
        let mut names: Vec<_> = NETWORK_FUNCTIONS.iter().map(|f| f.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), NETWORK_FUNCTIONS.len(), "Namen sind eindeutig");
    }

    #[test]
    fn qa_018_ac3_incomplete_entry_is_rejected() {
        assert_eq!(
            check(&entry(NetDefault::On, false)),
            Err(RegistryError::DefaultOn)
        );
        assert_eq!(check(&entry(NetDefault::On, true)), Ok(()));
        assert_eq!(check(&entry(NetDefault::Off, false)), Ok(()));
        let no_alt = NetFunction {
            offline_alternative: "",
            ..entry(NetDefault::UserAction, false)
        };
        assert_eq!(
            check(&no_alt),
            Err(RegistryError::MissingOfflineAlternative)
        );
        let no_purpose = NetFunction {
            purpose: "",
            ..entry(NetDefault::Off, false)
        };
        assert_eq!(
            check_all(&[entry(NetDefault::Off, false), no_purpose]),
            Err((1, RegistryError::MissingPurpose))
        );
    }
}
