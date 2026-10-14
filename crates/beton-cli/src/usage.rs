//! `beton usage pricing explain` (USE-003 AC3): Herleitung des Preises für ein Modell.
//!
//! Läuft ohne Daemon: eingebetteter Katalog (USE-002) plus Preis-Overrides der
//! User-Konfiguration (`pricing.overrides`, `providers.*.models[].pricing`). Im zentralen
//! Betrieb (`--server`) gelten nur die Overrides des Servers; die lokalen werden ignoriert
//! (USE-003 AC4). Die übrigen `usage`-Kommandos kommen mit USE-007.

use std::io::Write as _;

use beton_core::event::PriceSource;
use beton_core::pricing::{Catalog, Explanation, Pricer, Rates, StepOutcome};
use serde::Serialize;

use crate::cli::{PricingCommand, UsageCommand};
use crate::commands::Ctx;
use crate::config::Settings;
use crate::exit::{CliError, CliResult, Exit};

/// Hinweis, wenn lokale Overrides im zentralen Betrieb nicht gelten (USE-003 AC4).
pub const CENTRAL_NOTE: &str = "Zentraler Betrieb (--server): Es gelten nur die Preis-Overrides des \
     Servers; die lokalen Einträge unter pricing.overrides und providers.*.models[].pricing \
     werden ignoriert.";

/// Ausgabe von `beton usage pricing explain --json`.
#[derive(Debug, Clone, Serialize)]
pub struct ExplainReport {
    #[serde(flatten)]
    pub explanation: Explanation,
    /// `true` im zentralen Betrieb: lokale Overrides sind nicht berücksichtigt.
    pub central: bool,
    /// Zahl der ignorierten lokalen Overrides (nur zentral).
    pub ignored_overrides: usize,
}

/// Herleitung aus der Konfiguration; `central` blendet lokale Overrides aus.
pub fn explain(
    settings: &Settings,
    central: bool,
    provider: Option<&str>,
    model: &str,
) -> ExplainReport {
    let local = settings.price_overrides();
    let (overrides, ignored) = if central {
        (Vec::new(), local.len())
    } else {
        (local, 0)
    };
    let pricer = Pricer::new(Catalog::embedded(), overrides);
    ExplainReport {
        explanation: pricer.explain(provider, model),
        central,
        ignored_overrides: ignored,
    }
}

pub fn run(ctx: &Ctx, cmd: UsageCommand) -> CliResult {
    let UsageCommand::Pricing(PricingCommand::Explain(args)) = cmd;
    let layers = ctx.layers()?;
    let settings = layers
        .daemon_settings()
        .map_err(|e| CliError::new(Exit::General, e))?;
    let central = ctx.global.server.is_some();
    let report = explain(&settings, central, args.provider.as_deref(), &args.model);
    if central {
        ctx.note(CENTRAL_NOTE);
    }
    let mut out = std::io::stdout().lock();
    if ctx.global.json {
        serde_json::to_writer_pretty(&mut out, &report)
            .map_err(|e| CliError::new(Exit::General, e))?;
        writeln!(out).map_err(|e| CliError::new(Exit::General, e))?;
    } else {
        render(&report, &mut out).map_err(|e| CliError::new(Exit::General, e))?;
    }
    Ok(())
}

/// Betrag in USD wie in der UI, z. B. `0,40 $`.
fn usd(micro: i64) -> String {
    let s = format!("{:.6}", micro as f64 / 1_000_000.0);
    let s = s.trim_end_matches('0');
    let s = match s.split_once('.') {
        Some((int, frac)) if frac.len() < 2 => format!("{int}.{frac:0<2}"),
        _ => s.to_owned(),
    };
    format!("{} $", s.replace('.', ","))
}

fn source_name(s: PriceSource) -> &'static str {
    match s {
        PriceSource::Override => "eigener Preis (override)",
        PriceSource::Harness => "vom Harness gemeldet (harness)",
        PriceSource::Catalog => "Preis-Katalog (catalog)",
        PriceSource::None => "ohne Preis (none)",
        PriceSource::Subscription => "Subscription (subscription)",
    }
}

fn rates_line(r: &Rates) -> String {
    let estimated = if r.cache_estimated {
        " (Cache geschätzt: 0,1 × bzw. 1,25 × Eingabe)"
    } else {
        ""
    };
    format!(
        "Preise je 1 Mio. Tokens: Eingabe {}, Ausgabe {}, Cache lesen {}, Cache schreiben {}{estimated}",
        usd(r.input_micro),
        usd(r.output_micro),
        usd(r.cache_read_micro),
        usd(r.cache_write_micro),
    )
}

/// Text für das Terminal: ein Schritt je Zeile in Prüfreihenfolge.
pub fn render(report: &ExplainReport, out: &mut dyn std::io::Write) -> std::io::Result<()> {
    let e = &report.explanation;
    let subject = match &e.provider {
        Some(p) => format!("{p}/{}", e.model),
        None => e.model.clone(),
    };
    writeln!(out, "Herleitung für {subject}")?;
    for (i, step) in e.steps.iter().enumerate() {
        let what = match (step.source, step.position) {
            (PriceSource::Override, Some(n)) => format!("Eigener Preis #{n} ({})", step.label),
            _ => step.label.clone(),
        };
        let verdict = match step.outcome {
            StepOutcome::Used => format!("{} → wird verwendet", step.detail),
            _ => step.detail.clone(),
        };
        writeln!(out, "  {}. {what}: {verdict}", i + 1)?;
    }
    if report.central {
        writeln!(
            out,
            "  ({} lokale Overrides ignoriert: zentraler Betrieb)",
            report.ignored_overrides
        )?;
    }
    match &e.rates {
        Some(r) => {
            writeln!(out, "{}", rates_line(r))?;
            let example = r.cost_micro(beton_core::pricing::Tokens {
                input: 1_000_000,
                ..Default::default()
            });
            writeln!(
                out,
                "1.000.000 Eingabe-Tokens = {} · Quelle: {}",
                usd(example),
                source_name(e.cost_source)
            )?;
        }
        None => writeln!(
            out,
            "Quelle: {} – Turns zählen nicht in Budgets; eigenen Preis unter pricing.overrides anlegen",
            source_name(e.cost_source)
        )?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    use serde_json::json;

    fn settings(value: serde_json::Value) -> Settings {
        serde_json::from_value(value).unwrap()
    }

    /// Das Override-Beispiel aus der Spec (11-platform-features.md, „Preis-Overrides“).
    fn design() -> Settings {
        settings(json!({"pricing": {"overrides": [
            {
                "match": {"provider": "litellm-intern", "model": "llama-3.3-70b*"},
                "input_per_mtok": 0.40,
                "output_per_mtok": 0.80,
                "cache_read_per_mtok": 0.04,
                "cache_write_per_mtok": 0.50,
                "context_window": 131072
            },
            {"match": {"provider": "ollama-local"}, "free": true}
        ]}}))
    }

    #[test]
    fn use_003_ac3_explain_shows_the_derivation_step_by_step() {
        let report = explain(
            &design(),
            false,
            Some("litellm-intern"),
            "llama-3.3-70b-instruct",
        );
        let mut buf = Vec::new();
        render(&report, &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        let version = Catalog::embedded().version().to_owned();
        assert_eq!(
            text,
            format!(
                "Herleitung für litellm-intern/llama-3.3-70b-instruct\n\
                 \x20 1. Eigener Preis #1 (litellm-intern / llama-3.3-70b*): passt → wird verwendet\n\
                 \x20 2. Eigener Preis #2 (ollama-local / *): Provider litellm-intern passt nicht auf ollama-local\n\
                 \x20 3. vom Harness gemeldete Kosten: nicht geprüft\n\
                 \x20 4. Katalog {version}: kein Eintrag\n\
                 Preise je 1 Mio. Tokens: Eingabe 0,40 $, Ausgabe 0,80 $, Cache lesen 0,04 $, Cache schreiben 0,50 $\n\
                 1.000.000 Eingabe-Tokens = 0,40 $ · Quelle: eigener Preis (override)\n"
            )
        );
    }

    #[test]
    fn use_003_ac4_central_mode_ignores_local_overrides() {
        let report = explain(
            &design(),
            true,
            Some("litellm-intern"),
            "llama-3.3-70b-instruct",
        );
        assert_eq!(report.ignored_overrides, 2);
        assert_eq!(report.explanation.cost_source, PriceSource::None);
        assert!(
            report
                .explanation
                .steps
                .iter()
                .all(|s| s.source != PriceSource::Override)
        );
    }

    #[test]
    fn use_003_provider_prices_are_overrides_after_pricing_overrides() {
        let s = settings(json!({
            "pricing": {"overrides": [
                {"match": {"model": "qwen/*"}, "input_per_mtok": 1.0, "output_per_mtok": 1.0}
            ]},
            "providers": {"openrouter": {
                "kind": "openai",
                "base_url": "https://openrouter.ai/api/v1",
                "api_key_env": "OPENROUTER_API_KEY",
                "models": [{
                    "id": "qwen/qwen3-coder",
                    "pricing": {"input_per_mtok": 0.4, "output_per_mtok": 1.6}
                }]
            }}
        }));
        let overrides = s.price_overrides();
        assert_eq!(overrides.len(), 2);
        assert_eq!(overrides[1].label(), "openrouter / qwen/qwen3-coder");
        let report = explain(&s, false, Some("openrouter"), "qwen/qwen3-coder");
        assert_eq!(report.explanation.steps[0].outcome, StepOutcome::Used);
        assert_eq!(report.explanation.steps[1].outcome, StepOutcome::Shadowed);
    }

    #[test]
    fn unpriced_model_hints_at_an_override() {
        let report = explain(&Settings::default(), false, None, "mistral-large-2511");
        let mut buf = Vec::new();
        render(&report, &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(text.contains("ohne Preis (none)"), "{text}");
        assert!(text.contains("pricing.overrides"), "{text}");
    }

    #[test]
    fn usd_formatting() {
        assert_eq!(usd(400_000), "0,40 $");
        assert_eq!(usd(1_250_000), "1,25 $");
        assert_eq!(usd(75_000), "0,075 $");
        assert_eq!(usd(0), "0,00 $");
        assert_eq!(usd(10_000_000), "10,00 $");
    }
}
