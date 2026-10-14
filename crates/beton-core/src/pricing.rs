//! Kostenberechnung für `cost.delta` (USE-001, USE-002, USE-003).
//!
//! Spec: `docs/spec/11-platform-features.md`, Abschnitt „Kostenberechnung“. Für jedes
//! `cost.delta` mit `auth_source` ≠ `vendor_cli` gilt in dieser Reihenfolge:
//! 1. **Preis-Override** (USE-003), erster passender in Deklarationsreihenfolge → `override`;
//! 2. **vom Harness gemeldete Kosten** (`cost_micro` im Event) → `harness`;
//! 3. **Preis-Katalog** (USE-002) → `catalog` mit `pricing_version`;
//! 4. sonst `none` („ohne Preis“, `cost_micro` leer).
//!
//! Bei `vendor_cli` (Subscription, ADR-0034) gibt es nie einen Geldbetrag
//! (`cost_source: subscription`); das API-Äquivalent laut Katalog steht informativ in
//! `equivalent_micro`. Fehlen Cache-Raten, gilt `cache_read = 0,1 × input` und
//! `cache_write = 1,25 × input`. Gerechnet wird in Mikro-USD als Ganzzahl.
//!
//! Der [`Pricer`] bewertet ein Event genau einmal, beim Append im Server; das Ergebnis steht
//! im Event-Log. Ein Katalog-Update ändert deshalb keine bestehenden Records, und ein Rebuild
//! der Projektion ergibt dieselben Summen (USE-001 AC2, USE-002 AC1).

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, OnceLock};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::event::{AuthSource, CostDelta, PriceSource};

/// Der mit beton ausgelieferte Katalog (TOML).
pub const EMBEDDED_CATALOG: &str = include_str!("../pricing/catalog.toml");

/// Mikro-USD je US-Dollar.
const MICRO: f64 = 1_000_000.0;

// ---------------------------------------------------------------------------
// Katalog (USE-002)
// ---------------------------------------------------------------------------

/// Ein Modell im Preis-Katalog; Preise in USD je 1 Mio. Tokens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CatalogModel {
    /// Kanonische Modell-ID.
    pub id: String,
    /// Weitere IDs desselben Modells, z. B. datierte Snapshots.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_per_mtok: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_per_mtok: Option<f64>,
    /// Ab wann der Preis gilt (`YYYY-MM-DD`).
    pub valid_from: String,
}

impl CatalogModel {
    pub fn rates(&self) -> Rates {
        Rates::new(
            self.input_per_mtok,
            self.output_per_mtok,
            self.cache_read_per_mtok,
            self.cache_write_per_mtok,
        )
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogFile {
    catalog_version: String,
    currency: String,
    #[serde(default, rename = "model")]
    models: Vec<CatalogModel>,
}

/// Ungültiger Katalog: alle gefundenen Probleme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogError(pub Vec<String>);

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Preis-Katalog ungültig: {}", self.0.join("; "))
    }
}

impl std::error::Error for CatalogError {}

/// Geprüfter, versionierter Preis-Katalog.
#[derive(Debug, Clone, PartialEq)]
pub struct Catalog {
    version: String,
    models: Vec<CatalogModel>,
    /// ID bzw. Alias → Index in `models`.
    index: HashMap<String, usize>,
}

impl Catalog {
    /// Liest und prüft einen Katalog (USE-002 AC3): Schema, keine doppelten IDs oder Aliase,
    /// Preise endlich und ≥ 0, `valid_from` als Datum gesetzt, Währung USD.
    pub fn parse(text: &str) -> Result<Self, CatalogError> {
        let file: CatalogFile =
            toml::from_str(text).map_err(|e| CatalogError(vec![e.message().to_owned()]))?;
        let mut problems = Vec::new();
        if file.catalog_version.trim().is_empty() {
            problems.push("catalog_version fehlt".to_owned());
        }
        if file.currency != "USD" {
            problems.push(format!("currency muss USD sein, nicht {}", file.currency));
        }
        let mut index = HashMap::new();
        for (i, m) in file.models.iter().enumerate() {
            let at = if m.id.is_empty() {
                format!("model[{i}]")
            } else {
                m.id.clone()
            };
            if m.id.trim().is_empty() {
                problems.push(format!("{at}: id fehlt"));
            }
            if m.provider.trim().is_empty() {
                problems.push(format!("{at}: provider fehlt"));
            }
            for (name, value) in [
                ("input_per_mtok", Some(m.input_per_mtok)),
                ("output_per_mtok", Some(m.output_per_mtok)),
                ("cache_read_per_mtok", m.cache_read_per_mtok),
                ("cache_write_per_mtok", m.cache_write_per_mtok),
            ] {
                if let Some(v) = value
                    && !valid_price(v)
                {
                    problems.push(format!("{at}: {name} muss eine Zahl ≥ 0 sein ({v})"));
                }
            }
            if m.context_window == Some(0) {
                problems.push(format!("{at}: context_window muss > 0 sein"));
            }
            if !is_date(&m.valid_from) {
                problems.push(format!(
                    "{at}: valid_from muss ein Datum YYYY-MM-DD sein ({:?})",
                    m.valid_from
                ));
            }
            for name in std::iter::once(&m.id).chain(&m.aliases) {
                if let Some(prev) = index.insert(name.clone(), i) {
                    problems.push(format!(
                        "{name}: doppelt (bei {} und {at})",
                        file.models[prev].id
                    ));
                }
            }
        }
        if problems.is_empty() {
            Ok(Self {
                version: file.catalog_version,
                models: file.models,
                index,
            })
        } else {
            Err(CatalogError(problems))
        }
    }

    /// Der eingebettete Katalog. Seine Gültigkeit sichert ein Test in der CI (USE-002 AC3).
    pub fn embedded() -> Arc<Catalog> {
        static EMBEDDED: OnceLock<Arc<Catalog>> = OnceLock::new();
        EMBEDDED
            .get_or_init(|| {
                Arc::new(Catalog::parse(EMBEDDED_CATALOG).unwrap_or_else(|e| {
                    // Unerreichbar, solange der CI-Test grün ist; ohne Katalog gibt es nur
                    // Overrides und Harness-Kosten.
                    warn_invalid_catalog(&e);
                    Catalog::empty()
                }))
            })
            .clone()
    }

    /// Katalog ohne Einträge (Tests, Fallback).
    pub fn empty() -> Self {
        Self {
            version: String::new(),
            models: Vec::new(),
            index: HashMap::new(),
        }
    }

    /// `catalog_version`, z. B. `2026-10-14`.
    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn models(&self) -> &[CatalogModel] {
        &self.models
    }

    /// Eintrag zu einer Modell-ID oder einem Alias (USE-002 AC2).
    pub fn resolve(&self, model: &str) -> Option<&CatalogModel> {
        self.index.get(model).map(|&i| &self.models[i])
    }
}

fn warn_invalid_catalog(e: &CatalogError) {
    // beton-core hat bewusst keine Logging-Abhängigkeit.
    eprintln!("beton: {e}");
}

fn valid_price(v: f64) -> bool {
    v.is_finite() && v >= 0.0
}

fn is_date(s: &str) -> bool {
    let format = time::macros::format_description!("[year]-[month]-[day]");
    time::Date::parse(s, &format).is_ok()
}

// ---------------------------------------------------------------------------
// Raten
// ---------------------------------------------------------------------------

/// Tokens eines `cost.delta`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl From<&CostDelta> for Tokens {
    fn from(c: &CostDelta) -> Self {
        Self {
            input: c.input_tokens,
            output: c.output_tokens,
            cache_read: c.cache_read_tokens,
            cache_write: c.cache_write_tokens,
        }
    }
}

/// Wirksame Preise in Mikro-USD je 1 Mio. Tokens (fehlende Cache-Raten aufgefüllt).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Rates {
    pub input_micro: i64,
    pub output_micro: i64,
    pub cache_read_micro: i64,
    pub cache_write_micro: i64,
    /// Die Cache-Raten sind geschätzt (0,1 × bzw. 1,25 × input).
    pub cache_estimated: bool,
}

impl Rates {
    /// Raten aus USD je 1 Mio. Tokens; fehlende Cache-Raten nach USE-003 AC2.
    pub fn new(input: f64, output: f64, cache_read: Option<f64>, cache_write: Option<f64>) -> Self {
        let input_micro = to_micro(input);
        Self {
            input_micro,
            output_micro: to_micro(output),
            cache_read_micro: cache_read.map_or((input_micro + 5) / 10, to_micro),
            cache_write_micro: cache_write.map_or((input_micro * 5 + 2) / 4, to_micro),
            cache_estimated: cache_read.is_none() || cache_write.is_none(),
        }
    }

    /// Kostenlos (`free: true`).
    pub fn free() -> Self {
        Self {
            input_micro: 0,
            output_micro: 0,
            cache_read_micro: 0,
            cache_write_micro: 0,
            cache_estimated: false,
        }
    }

    /// Kosten in Mikro-USD, einmal am Ende kaufmännisch gerundet.
    pub fn cost_micro(&self, t: Tokens) -> i64 {
        let sum = i128::from(t.input) * i128::from(self.input_micro)
            + i128::from(t.output) * i128::from(self.output_micro)
            + i128::from(t.cache_read) * i128::from(self.cache_read_micro)
            + i128::from(t.cache_write) * i128::from(self.cache_write_micro);
        i64::try_from((sum + 500_000) / 1_000_000).unwrap_or(i64::MAX)
    }
}

fn to_micro(usd: f64) -> i64 {
    if valid_price(usd) {
        (usd * MICRO).round() as i64
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// Overrides (USE-003)
// ---------------------------------------------------------------------------

/// Worauf ein Override passt: Glob (`*`, `?`) auf Provider und/oder Modell.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OverrideMatch {
    /// Provider, z. B. `litellm-intern` (Name unter `providers:`), `anthropic`, `openai`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Modell-ID, z. B. `llama-3.3-70b*`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Eigener Preis für Gateway- und Self-hosted-Modelle (`pricing.overrides[]`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PriceOverride {
    #[serde(rename = "match")]
    pub matcher: OverrideMatch,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_per_mtok: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_per_mtok: Option<f64>,
    /// Optional; sonst 0,1 × `input_per_mtok`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_per_mtok: Option<f64>,
    /// Optional; sonst 1,25 × `input_per_mtok`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_per_mtok: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    /// Kostenlos, z. B. lokales Ollama; dann keine Preise angeben.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub free: bool,
}

impl PriceOverride {
    /// Prüft einen Override; der Fehler nennt den Grund.
    pub fn validate(&self) -> Result<(), String> {
        if self.matcher.provider.is_none() && self.matcher.model.is_none() {
            return Err("match braucht provider oder model".into());
        }
        let prices = [
            ("input_per_mtok", self.input_per_mtok),
            ("output_per_mtok", self.output_per_mtok),
            ("cache_read_per_mtok", self.cache_read_per_mtok),
            ("cache_write_per_mtok", self.cache_write_per_mtok),
        ];
        if self.free {
            if let Some((name, _)) = prices.iter().find(|(_, v)| v.is_some()) {
                return Err(format!("free: true verträgt keine Preise ({name})"));
            }
            return Ok(());
        }
        for (name, v) in prices {
            if let Some(v) = v
                && !valid_price(v)
            {
                return Err(format!("{name} muss eine Zahl ≥ 0 sein"));
            }
        }
        if self.input_per_mtok.is_none() || self.output_per_mtok.is_none() {
            return Err("input_per_mtok und output_per_mtok fehlen (oder free: true)".into());
        }
        Ok(())
    }

    /// Passt der Override auf Provider und Modell?
    pub fn matches(&self, provider: Option<&str>, model: &str) -> bool {
        self.mismatch(provider, model).is_none()
    }

    /// Warum der Override nicht passt (für die Herleitung), `None` bei Treffer.
    fn mismatch(&self, provider: Option<&str>, model: &str) -> Option<String> {
        if let Some(p) = &self.matcher.provider {
            match provider {
                Some(actual) if glob(p, actual) => {}
                Some(actual) => return Some(format!("Provider {actual} passt nicht auf {p}")),
                None => return Some(format!("nur für Provider {p}")),
            }
        }
        if let Some(m) = &self.matcher.model
            && !glob(m, model)
        {
            return Some(format!("Modell passt nicht auf {m}"));
        }
        None
    }

    pub fn rates(&self) -> Rates {
        if self.free {
            return Rates::free();
        }
        Rates::new(
            self.input_per_mtok.unwrap_or(0.0),
            self.output_per_mtok.unwrap_or(0.0),
            self.cache_read_per_mtok,
            self.cache_write_per_mtok,
        )
    }

    /// Kurzform für Anzeigen, z. B. `litellm-intern / llama-3.3-70b*`.
    pub fn label(&self) -> String {
        format!(
            "{} / {}",
            self.matcher.provider.as_deref().unwrap_or("*"),
            self.matcher.model.as_deref().unwrap_or("*")
        )
    }
}

/// Glob mit `*` (beliebig viele Zeichen) und `?` (genau ein Zeichen).
pub fn glob(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

/// Provider eines Harness für den Override-Abgleich: `direct:<name>` → `<name>`,
/// `acp:<slug>` → `<slug>`, `claude` → `anthropic`, `codex` → `openai`, sonst die Harness-ID.
pub fn provider_of(harness: &str) -> &str {
    if let Some(name) = harness
        .strip_prefix("direct:")
        .or_else(|| harness.strip_prefix("acp:"))
    {
        return name;
    }
    match harness {
        "claude" => "anthropic",
        "codex" => "openai",
        other => other,
    }
}

// ---------------------------------------------------------------------------
// Bewertung
// ---------------------------------------------------------------------------

/// Bewertet `cost.delta`-Events: Overrides vor Harness-Kosten vor Katalog.
#[derive(Debug, Clone)]
pub struct Pricer {
    catalog: Arc<Catalog>,
    overrides: Vec<PriceOverride>,
}

impl Default for Pricer {
    /// Eingebetteter Katalog, keine Overrides.
    fn default() -> Self {
        Self::new(Catalog::embedded(), Vec::new())
    }
}

impl Pricer {
    /// Overrides in Deklarationsreihenfolge; ungültige sollte der Aufrufer vorher aussortieren
    /// ([`PriceOverride::validate`]).
    pub fn new(catalog: Arc<Catalog>, overrides: Vec<PriceOverride>) -> Self {
        Self { catalog, overrides }
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    pub fn overrides(&self) -> &[PriceOverride] {
        &self.overrides
    }

    fn first_override(
        &self,
        provider: Option<&str>,
        model: &str,
    ) -> Option<(usize, &PriceOverride)> {
        self.overrides
            .iter()
            .enumerate()
            .find(|(_, o)| o.matches(provider, model))
    }

    /// Bewertet ein `cost.delta` (setzt `cost_source`, `cost_micro`, `pricing_version`,
    /// `equivalent_micro`). Schon bewertete Events bleiben unverändert: Historische Kosten
    /// werden nie neu bewertet (USE-002 AC1).
    pub fn price(&self, c: &mut CostDelta) {
        if c.cost_source.is_some() {
            return;
        }
        let tokens = Tokens::from(&*c);
        let entry = self.catalog.resolve(&c.model);
        if c.auth_source == AuthSource::VendorCli {
            // Subscription: nie Geld (ADR-0034); das Äquivalent ist nur informativ.
            c.cost_micro = None;
            c.cost_source = Some(PriceSource::Subscription);
            c.equivalent_micro = entry.map(|m| m.rates().cost_micro(tokens));
            c.pricing_version = entry.map(|_| self.catalog.version.clone());
            return;
        }
        if let Some((_, o)) = self.first_override(Some(provider_of(&c.harness)), &c.model) {
            c.cost_micro = Some(o.rates().cost_micro(tokens));
            c.cost_source = Some(PriceSource::Override);
        } else if c.cost_micro.is_some() {
            c.cost_source = Some(PriceSource::Harness);
        } else if let Some(m) = entry {
            c.cost_micro = Some(m.rates().cost_micro(tokens));
            c.cost_source = Some(PriceSource::Catalog);
            c.pricing_version = Some(self.catalog.version.clone());
        } else {
            c.cost_source = Some(PriceSource::None);
        }
    }

    /// Herleitung des Preises für ein Modell (`beton usage pricing explain`, USE-003 AC3).
    pub fn explain(&self, provider: Option<&str>, model: &str) -> Explanation {
        let mut steps = Vec::new();
        let winner = self.first_override(provider, model).map(|(i, _)| i);
        for (i, o) in self.overrides.iter().enumerate() {
            let (outcome, detail) = match o.mismatch(provider, model) {
                Some(reason) => (StepOutcome::NoMatch, reason),
                None if Some(i) == winner => (StepOutcome::Used, "passt".to_owned()),
                None => (
                    StepOutcome::Shadowed,
                    format!("passt, aber #{} steht davor", winner.map_or(0, |w| w + 1)),
                ),
            };
            steps.push(ExplainStep {
                source: PriceSource::Override,
                position: Some(i + 1),
                label: o.label(),
                outcome,
                detail,
            });
        }
        steps.push(ExplainStep {
            source: PriceSource::Harness,
            position: None,
            label: "vom Harness gemeldete Kosten".into(),
            outcome: if winner.is_some() {
                StepOutcome::Skipped
            } else {
                StepOutcome::Runtime
            },
            detail: if winner.is_some() {
                "nicht geprüft".into()
            } else {
                "nur wenn der Harness Kosten meldet (z. B. Claude mit API-Key)".into()
            },
        });
        let entry = self.catalog.resolve(model);
        steps.push(ExplainStep {
            source: PriceSource::Catalog,
            position: None,
            label: format!("Katalog {}", self.catalog.version),
            outcome: match (entry, winner) {
                (None, _) => StepOutcome::NoEntry,
                (Some(_), Some(_)) => StepOutcome::Skipped,
                (Some(_), None) => StepOutcome::Used,
            },
            detail: match entry {
                Some(m) if m.id == model => format!("Eintrag {}", m.id),
                Some(m) => format!("Alias von {}", m.id),
                None => "kein Eintrag".into(),
            },
        });
        let (result, rates, pricing_version) = match (winner, entry) {
            (Some(i), _) => (PriceSource::Override, Some(self.overrides[i].rates()), None),
            (None, Some(m)) => (
                PriceSource::Catalog,
                Some(m.rates()),
                Some(self.catalog.version.clone()),
            ),
            (None, None) => (PriceSource::None, None, None),
        };
        Explanation {
            provider: provider.map(str::to_owned),
            model: model.to_owned(),
            steps,
            cost_source: result,
            rates,
            pricing_version,
        }
    }
}

/// Ergebnis eines Prüfschritts der Herleitung.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StepOutcome {
    /// Diese Quelle bestimmt den Preis.
    Used,
    /// Passt, aber eine frühere Quelle gewinnt.
    Shadowed,
    NoMatch,
    NoEntry,
    /// Nicht geprüft, weil eine frühere Quelle gewinnt.
    Skipped,
    /// Entscheidet sich erst zur Laufzeit (Harness-Kosten).
    Runtime,
}

/// Ein Schritt der Herleitung.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ExplainStep {
    pub source: PriceSource,
    /// Position des Overrides (ab 1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<usize>,
    pub label: String,
    pub outcome: StepOutcome,
    pub detail: String,
}

/// Herleitung des Preises für Provider und Modell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Explanation {
    pub provider: Option<String>,
    pub model: String,
    pub steps: Vec<ExplainStep>,
    /// Preisquelle, wenn der Harness keine Kosten meldet.
    pub cost_source: PriceSource,
    pub rates: Option<Rates>,
    pub pricing_version: Option<String>,
}

// ---------------------------------------------------------------------------
// Normalisierung für Projektionen (USE-001)
// ---------------------------------------------------------------------------

/// Preisquelle eines Events; Events ohne `cost_source` (vor M2 bzw. importiert) werden wie
/// beim Bewerten eingestuft, ohne neu zu rechnen.
pub fn effective_source(c: &CostDelta) -> PriceSource {
    c.cost_source
        .unwrap_or(if c.auth_source == AuthSource::VendorCli {
            PriceSource::Subscription
        } else if c.cost_micro.is_some() {
            PriceSource::Harness
        } else {
            PriceSource::None
        })
}

/// Geldbetrag eines Events für Summen: bei `vendor_cli` nie (USE-001 AC3).
pub fn money_micro(c: &CostDelta) -> Option<i64> {
    if c.auth_source == AuthSource::VendorCli {
        None
    } else {
        c.cost_micro
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// Das Override-Beispiel aus der Spec (11-platform-features.md, „Preis-Overrides“).
    fn design_overrides() -> Vec<PriceOverride> {
        let v: Vec<PriceOverride> = serde_json::from_value(serde_json::json!([
            {
                "match": { "provider": "litellm-intern", "model": "llama-3.3-70b*" },
                "input_per_mtok": 0.40,
                "output_per_mtok": 0.80,
                "cache_read_per_mtok": 0.04,
                "cache_write_per_mtok": 0.50,
                "context_window": 131072
            },
            { "match": { "provider": "ollama-local" }, "free": true }
        ]))
        .unwrap();
        for o in &v {
            o.validate().unwrap();
        }
        v
    }

    fn delta(harness: &str, model: &str, auth: AuthSource) -> CostDelta {
        CostDelta {
            harness: harness.into(),
            model: model.into(),
            currency: "USD".into(),
            auth_source: auth,
            ..CostDelta::default()
        }
    }

    #[test]
    fn use_002_ac3_embedded_catalog_is_valid() {
        let catalog = Catalog::parse(EMBEDDED_CATALOG).unwrap_or_else(|e| panic!("{e}"));
        assert!(!catalog.version().is_empty());
        assert!(catalog.models().len() >= 10, "Katalog ist nicht leer");
        for m in catalog.models() {
            assert!(
                m.input_per_mtok >= 0.0 && m.output_per_mtok >= 0.0,
                "{}",
                m.id
            );
            assert!(is_date(&m.valid_from), "{}", m.id);
        }
        assert_eq!(Catalog::embedded().version(), catalog.version());
    }

    #[test]
    fn use_002_ac3_validation_rejects_duplicates_negative_prices_and_missing_valid_from() {
        let bad = r#"
catalog_version = "x"
currency = "USD"
[[model]]
id = "a"
aliases = ["b"]
provider = "p"
input_per_mtok = -1.0
output_per_mtok = 1.0
valid_from = "2026-01-01"
[[model]]
id = "b"
provider = "p"
input_per_mtok = 1.0
output_per_mtok = 1.0
valid_from = ""
"#;
        let err = Catalog::parse(bad).unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("a: input_per_mtok muss eine Zahl ≥ 0 sein"),
            "{text}"
        );
        assert!(text.contains("b: doppelt"), "{text}");
        assert!(text.contains("b: valid_from muss ein Datum"), "{text}");

        // Schema: fehlendes Pflichtfeld und unbekannte Felder.
        let missing = "catalog_version = \"x\"\ncurrency = \"USD\"\n[[model]]\nid = \"a\"\nprovider = \"p\"\ninput_per_mtok = 1.0\noutput_per_mtok = 1.0\n";
        assert!(
            Catalog::parse(missing)
                .unwrap_err()
                .to_string()
                .contains("valid_from")
        );
        let unknown = "catalog_version = \"x\"\ncurrency = \"USD\"\nfoo = 1\n";
        assert!(Catalog::parse(unknown).is_err());
        let eur = "catalog_version = \"x\"\ncurrency = \"EUR\"\n";
        assert!(Catalog::parse(eur).unwrap_err().to_string().contains("USD"));
    }

    #[test]
    fn use_002_ac2_every_alias_resolves_to_its_canonical_model() {
        let catalog = Catalog::embedded();
        for m in catalog.models() {
            assert_eq!(catalog.resolve(&m.id).unwrap().id, m.id);
            for alias in &m.aliases {
                assert_eq!(
                    catalog.resolve(alias).unwrap().id,
                    m.id,
                    "Alias {alias} gehört zu {}",
                    m.id
                );
            }
        }
        assert_eq!(
            catalog.resolve("claude-sonnet-4-5-20250929").unwrap().id,
            "claude-sonnet-4-5"
        );
        assert!(catalog.resolve("gibt-es-nicht").is_none());
    }

    #[test]
    fn use_002_ac2_dated_alias_is_priced_like_the_canonical_model() {
        let pricer = Pricer::default();
        let mut a = delta("codex", "gpt-5", AuthSource::ApiKey);
        a.input_tokens = 1_000_000;
        let mut b = a.clone();
        b.model = "gpt-5-2025-08-07".into();
        pricer.price(&mut a);
        pricer.price(&mut b);
        assert_eq!(a.cost_micro, Some(1_250_000));
        assert_eq!(a.cost_micro, b.cost_micro);
        assert_eq!(b.cost_source, Some(PriceSource::Catalog));
    }

    #[test]
    fn use_003_ac1_design_override_prices_one_million_input_tokens_at_forty_cents() {
        let pricer = Pricer::new(Catalog::embedded(), design_overrides());
        let mut c = delta(
            "direct:litellm-intern",
            "llama-3.3-70b-instruct",
            AuthSource::Gateway,
        );
        c.input_tokens = 1_000_000;
        pricer.price(&mut c);
        assert_eq!(c.cost_micro, Some(400_000), "0,40 USD");
        assert_eq!(c.cost_source, Some(PriceSource::Override));
        assert_eq!(c.pricing_version, None);

        let mut free = delta("direct:ollama-local", "qwen3", AuthSource::Gateway);
        free.input_tokens = 5_000;
        pricer.price(&mut free);
        assert_eq!(
            (free.cost_micro, free.cost_source),
            (Some(0), Some(PriceSource::Override))
        );
    }

    #[test]
    fn use_003_ac2_missing_cache_rates_fall_back_to_input_multiples() {
        let o = PriceOverride {
            matcher: OverrideMatch {
                provider: Some("gw".into()),
                model: None,
            },
            input_per_mtok: Some(0.40),
            output_per_mtok: Some(0.80),
            ..PriceOverride::default()
        };
        let r = o.rates();
        assert_eq!(r.cache_read_micro, 40_000, "0,1 × input");
        assert_eq!(r.cache_write_micro, 500_000, "1,25 × input");
        assert!(r.cache_estimated);
        let pricer = Pricer::new(Arc::new(Catalog::empty()), vec![o]);
        let mut c = delta("direct:gw", "m", AuthSource::Gateway);
        c.cache_read_tokens = 1_000_000;
        c.cache_write_tokens = 1_000_000;
        pricer.price(&mut c);
        assert_eq!(c.cost_micro, Some(540_000));
        // Katalogeinträge ohne Cache-Raten nutzen denselben Fallback.
        assert_eq!(
            Rates::new(3.0, 15.0, None, None).cache_write_micro,
            3_750_000
        );
        assert_eq!(Rates::new(3.0, 15.0, None, None).cache_read_micro, 300_000);
    }

    #[test]
    fn use_003_ac3_first_matching_override_wins_and_explain_shows_why() {
        let mut overrides = design_overrides();
        overrides.insert(
            0,
            PriceOverride {
                matcher: OverrideMatch {
                    provider: None,
                    model: Some("llama-3.3-70b-instruct".into()),
                },
                input_per_mtok: Some(1.0),
                output_per_mtok: Some(2.0),
                ..PriceOverride::default()
            },
        );
        overrides.push(PriceOverride {
            matcher: OverrideMatch {
                provider: None,
                model: Some("llama-*".into()),
            },
            input_per_mtok: Some(9.0),
            output_per_mtok: Some(9.0),
            ..PriceOverride::default()
        });
        let pricer = Pricer::new(Catalog::embedded(), overrides);
        let mut c = delta(
            "direct:litellm-intern",
            "llama-3.3-70b-instruct",
            AuthSource::Gateway,
        );
        c.input_tokens = 1_000_000;
        pricer.price(&mut c);
        assert_eq!(c.cost_micro, Some(1_000_000), "erster Treffer (#1) gewinnt");

        let e = pricer.explain(Some("litellm-intern"), "llama-3.3-70b-instruct");
        let outcomes: Vec<_> = e.steps.iter().map(|s| (s.position, s.outcome)).collect();
        assert_eq!(
            outcomes,
            vec![
                (Some(1), StepOutcome::Used),
                (Some(2), StepOutcome::Shadowed),
                (Some(3), StepOutcome::NoMatch),
                (Some(4), StepOutcome::Shadowed),
                (None, StepOutcome::Skipped),
                (None, StepOutcome::NoEntry),
            ]
        );
        assert_eq!(e.steps[1].detail, "passt, aber #1 steht davor");
        assert_eq!(e.cost_source, PriceSource::Override);
        assert_eq!(e.rates.unwrap().input_micro, 1_000_000);

        // Ohne Treffer: Katalog.
        let e = pricer.explain(None, "claude-sonnet-4-5-20250929");
        assert_eq!(e.cost_source, PriceSource::Catalog);
        assert_eq!(e.steps.last().unwrap().outcome, StepOutcome::Used);
        assert_eq!(
            e.steps.last().unwrap().detail,
            "Alias von claude-sonnet-4-5"
        );
        assert_eq!(
            e.pricing_version.as_deref(),
            Some(Catalog::embedded().version())
        );
    }

    #[test]
    fn use_001_design_order_override_then_harness_then_catalog_then_none() {
        let o = PriceOverride {
            matcher: OverrideMatch {
                provider: Some("anthropic".into()),
                model: Some("claude-sonnet-4-5".into()),
            },
            input_per_mtok: Some(1.0),
            output_per_mtok: Some(1.0),
            ..PriceOverride::default()
        };
        let pricer = Pricer::new(Catalog::embedded(), vec![o]);
        let mut reported = delta("claude", "claude-sonnet-4-5", AuthSource::ApiKey);
        reported.cost_micro = Some(7);
        reported.input_tokens = 1_000_000;
        pricer.price(&mut reported);
        assert_eq!(
            (reported.cost_source, reported.cost_micro),
            (Some(PriceSource::Override), Some(1_000_000))
        );

        let mut harness = delta("claude", "claude-haiku-4-5", AuthSource::ApiKey);
        harness.cost_micro = Some(7);
        pricer.price(&mut harness);
        assert_eq!(
            (
                harness.cost_source,
                harness.cost_micro,
                harness.pricing_version
            ),
            (Some(PriceSource::Harness), Some(7), None)
        );

        let mut catalog = delta("codex", "gpt-5-codex", AuthSource::ApiKey);
        catalog.input_tokens = 1_000;
        catalog.output_tokens = 1_000;
        catalog.cache_read_tokens = 10_000;
        pricer.price(&mut catalog);
        // 1000 × 1,25 + 1000 × 10 + 10 000 × 0,125 = 12 500 Mikro-USD
        assert_eq!(catalog.cost_micro, Some(12_500));
        assert_eq!(catalog.cost_source, Some(PriceSource::Catalog));
        assert_eq!(
            catalog.pricing_version.as_deref(),
            Some(Catalog::embedded().version())
        );

        let mut none = delta("fake", "fake-mini", AuthSource::None);
        none.input_tokens = 1_200;
        pricer.price(&mut none);
        assert_eq!(
            (none.cost_source, none.cost_micro),
            (Some(PriceSource::None), None)
        );
    }

    #[test]
    fn har_006_ac3_codex_with_api_key_is_priced_from_the_catalog() {
        // Codex meldet nur Tokens (cost_micro leer); mit API-Key kommt der Preis aus dem Katalog,
        // mit Subscription nur das informative Äquivalent (USE-004 folgt mit WP-44).
        let pricer = Pricer::default();
        let mut api = delta("codex", "gpt-5.3-codex", AuthSource::ApiKey);
        api.source = crate::event::CostSource::Estimated;
        (api.input_tokens, api.output_tokens, api.cache_read_tokens) = (10_000, 1_000, 50_000);
        let mut sub = api.clone();
        sub.auth_source = AuthSource::VendorCli;
        pricer.price(&mut api);
        pricer.price(&mut sub);
        // 10 000 × 1,75 + 1 000 × 14 + 50 000 × 0,175 = 40 250 Mikro-USD
        assert_eq!(
            (api.cost_source, api.cost_micro),
            (Some(PriceSource::Catalog), Some(40_250))
        );
        assert_eq!((sub.cost_micro, sub.equivalent_micro), (None, Some(40_250)));
    }

    #[test]
    fn use_001_ac3_subscription_never_gets_money_only_an_equivalent() {
        let pricer = Pricer::default();
        let mut c = delta("claude", "claude-opus-5-5", AuthSource::VendorCli);
        c.input_tokens = 1_000_000;
        c.cost_micro = Some(123); // falsch gemeldet: wird verworfen
        pricer.price(&mut c);
        assert_eq!(c.cost_micro, None);
        assert_eq!(c.cost_source, Some(PriceSource::Subscription));
        assert_eq!(c.equivalent_micro, Some(4_000_000));
        assert_eq!(money_micro(&c), None);

        let legacy = CostDelta {
            cost_micro: Some(5),
            auth_source: AuthSource::VendorCli,
            ..CostDelta::default()
        };
        assert_eq!(money_micro(&legacy), None);
        assert_eq!(effective_source(&legacy), PriceSource::Subscription);
    }

    #[test]
    fn use_002_ac1_priced_events_are_never_repriced() {
        let mut c = delta("codex", "gpt-5", AuthSource::ApiKey);
        c.input_tokens = 1_000_000;
        Pricer::default().price(&mut c);
        let before = c.clone();
        let newer = Catalog::parse(
            &EMBEDDED_CATALOG
                .replace("catalog_version = \"", "catalog_version = \"9")
                .replace("input_per_mtok = 1.25", "input_per_mtok = 99.0"),
        )
        .unwrap();
        Pricer::new(Arc::new(newer), Vec::new()).price(&mut c);
        assert_eq!(c, before);
    }

    #[test]
    fn override_validation() {
        let mut o = PriceOverride::default();
        assert!(o.validate().unwrap_err().contains("match"));
        o.matcher.model = Some("x".into());
        assert!(o.validate().unwrap_err().contains("input_per_mtok"));
        o.free = true;
        assert!(o.validate().is_ok());
        o.input_per_mtok = Some(1.0);
        assert!(o.validate().unwrap_err().contains("free"));
        o.free = false;
        o.output_per_mtok = Some(-1.0);
        assert!(o.validate().unwrap_err().contains("output_per_mtok"));
    }

    #[test]
    fn glob_matching() {
        assert!(glob("llama-3.3-70b*", "llama-3.3-70b-instruct"));
        assert!(glob("*", ""));
        assert!(glob("a?c", "abc"));
        assert!(glob("*-mini", "gpt-5-mini"));
        assert!(!glob("llama*", "qwen"));
        assert!(!glob("a?c", "ac"));
        assert!(glob("a*b*c", "aXbYbZc"));
    }

    #[test]
    fn providers_of_harnesses() {
        assert_eq!(provider_of("direct:litellm-intern"), "litellm-intern");
        assert_eq!(provider_of("acp:gemini"), "gemini");
        assert_eq!(provider_of("claude"), "anthropic");
        assert_eq!(provider_of("codex"), "openai");
        assert_eq!(provider_of("fake"), "fake");
    }

    #[test]
    fn rounding_happens_once_at_the_end() {
        let r = Rates::new(0.000_001, 0.0, Some(0.0), Some(0.0));
        assert_eq!(
            r.cost_micro(Tokens {
                input: 499_999,
                ..Tokens::default()
            }),
            0
        );
        assert_eq!(
            r.cost_micro(Tokens {
                input: 500_000,
                ..Tokens::default()
            }),
            1
        );
        assert_eq!(
            Rates::new(1e9, 0.0, None, None).cost_micro(Tokens {
                input: u64::MAX,
                ..Tokens::default()
            }),
            i64::MAX
        );
    }
}
