//! Coverage-Floor und Ratchet für sicherheitskritische Crates (QA-011).
//!
//! Liest den JSON-Export von `cargo llvm-cov --json`, berechnet die Zeilen-Coverage je
//! Crate (nur `crates/<name>/src/**`), prüft den Floor und den Ratchet gegen den Stand von
//! `main` (Toleranz 0,2 Punkte) und schreibt einen Bericht als Markdown (Job-Summary).

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde_json::Value;

/// Ratchet-Toleranz in Prozentpunkten.
pub const TOLERANCE: f64 = 0.2;

/// Gemessene Crates: Name und Floor (`None` = nur berichten, ohne Gate).
pub const CRATES: [(&str, Option<f64>); 4] = [
    ("beton-policy", Some(90.0)),
    // Gate ab WP-39, sobald das Crate gemessen wird.
    ("beton-sandbox", Some(80.0)),
    // Gate erst ab M4 (Should); bis dahin nur berichtet.
    ("beton-proxy", None),
    ("beton-secrets", None),
];

/// Zeilen-Coverage je Crate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lines {
    pub covered: u64,
    pub count: u64,
}

impl Lines {
    pub fn percent(&self) -> f64 {
        if self.count == 0 {
            100.0
        } else {
            self.covered as f64 * 100.0 / self.count as f64
        }
    }
}

/// Coverage je Crate aus dem llvm-cov-Export.
pub fn per_crate(export: &Value) -> Result<BTreeMap<String, Lines>> {
    let files = export
        .pointer("/data/0/files")
        .and_then(Value::as_array)
        .context("llvm-cov-Export ohne data[0].files")?;
    let mut out: BTreeMap<String, Lines> = BTreeMap::new();
    for f in files {
        let name = f["filename"]
            .as_str()
            .unwrap_or_default()
            .replace('\\', "/");
        let Some(rest) = name.split("/crates/").nth(1) else {
            continue;
        };
        let Some((krate, path)) = rest.split_once('/') else {
            continue;
        };
        if !path.starts_with("src/") {
            continue;
        }
        let lines = &f["summary"]["lines"];
        let entry = out.entry(krate.to_owned()).or_insert(Lines {
            covered: 0,
            count: 0,
        });
        entry.covered += lines["covered"].as_u64().unwrap_or(0);
        entry.count += lines["count"].as_u64().unwrap_or(0);
    }
    Ok(out)
}

/// Ergebnis je Crate.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub krate: &'static str,
    pub percent: Option<f64>,
    pub floor: Option<f64>,
    pub baseline: Option<f64>,
    pub problems: Vec<String>,
}

/// Prüft Floor und Ratchet. `present` nennt Crates, die es im Repo gibt; ein vorhandenes
/// Crate mit Gate, das nicht gemessen wurde, ist ein Fehler (fail closed).
pub fn evaluate(
    current: &BTreeMap<String, Lines>,
    baseline: Option<&BTreeMap<String, f64>>,
    present: &[&str],
) -> Vec<Row> {
    CRATES
        .iter()
        .filter(|(krate, _)| present.contains(krate))
        .map(|&(krate, floor)| {
            let percent = current.get(krate).map(Lines::percent);
            let base = baseline.and_then(|b| b.get(krate).copied());
            let mut problems = Vec::new();
            if let Some(floor) = floor {
                match percent {
                    None => problems.push("nicht gemessen".to_owned()),
                    Some(p) if p < floor => {
                        problems.push(format!("{p:.2} % unter dem Floor von {floor:.0} %"));
                    }
                    Some(_) => {}
                }
                if let (Some(p), Some(b)) = (percent, base)
                    && p < b - TOLERANCE
                {
                    problems.push(format!(
                        "Ratchet: {p:.2} % gegenüber {b:.2} % auf main (−{:.2} Punkte, Toleranz {TOLERANCE})",
                        b - p
                    ));
                }
            }
            Row {
                krate,
                percent,
                floor,
                baseline: base,
                problems,
            }
        })
        .collect()
}

/// Bericht als Markdown-Tabelle (Job-Summary, QA-011 AC2).
pub fn markdown(rows: &[Row]) -> String {
    let mut out = String::from(
        "### Coverage sicherheitskritischer Crates (QA-011)\n\n| Crate | Zeilen | Floor | main | Status |\n|---|---:|---:|---:|---|\n",
    );
    for r in rows {
        let pct = |v: Option<f64>| v.map_or("–".to_owned(), |p| format!("{p:.2} %"));
        let floor = r
            .floor
            .map_or("nur Bericht".to_owned(), |f| format!("{f:.0} %"));
        let status = if r.problems.is_empty() {
            "ok".to_owned()
        } else {
            format!("❌ {}", r.problems.join("; "))
        };
        out.push_str(&format!(
            "| `{}` | {} | {floor} | {} | {status} |\n",
            r.krate,
            pct(r.percent),
            pct(r.baseline)
        ));
    }
    out
}

/// Gemessene Werte als JSON (Basis für den Ratchet nachfolgender PRs).
pub fn summary_json(current: &BTreeMap<String, Lines>) -> Value {
    Value::Object(
        CRATES
            .iter()
            .filter_map(|(k, _)| {
                current
                    .get(*k)
                    .map(|l| ((*k).to_owned(), Value::from(l.percent())))
            })
            .collect(),
    )
}

/// Liest eine frühere Zusammenfassung (`{"beton-policy": 93.1, …}`).
pub fn parse_baseline(v: &Value) -> BTreeMap<String, f64> {
    v.as_object()
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_f64().map(|p| (k.clone(), p)))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn export(files: &[(&str, u64, u64)]) -> Value {
        json!({ "data": [{ "files": files.iter().map(|(name, covered, count)| json!({
            "filename": name,
            "summary": { "lines": { "covered": covered, "count": count } }
        })).collect::<Vec<_>>() }] })
    }

    #[test]
    fn qa_011_ac1_policy_below_90_percent_fails() {
        let current = per_crate(&export(&[
            ("/w/crates/beton-policy/src/a.rs", 80, 100),
            ("/w/crates/beton-policy/src/b.rs", 9, 10),
            ("/w/crates/beton-policy/tests/t.rs", 0, 500),
            ("/w/crates/beton-core/src/c.rs", 0, 100),
        ]))
        .unwrap();
        assert_eq!(
            current["beton-policy"],
            Lines {
                covered: 89,
                count: 110
            }
        );
        assert!(!current.contains_key("beton-core") || current["beton-core"].count == 100);
        let rows = evaluate(&current, None, &["beton-policy"]);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].problems[0].contains("Floor"), "{:?}", rows[0]);

        let ok = per_crate(&export(&[("/w/crates/beton-policy/src/a.rs", 91, 100)])).unwrap();
        assert!(
            evaluate(&ok, None, &["beton-policy"])[0]
                .problems
                .is_empty()
        );
    }

    #[test]
    fn qa_011_ac1_unmeasured_gated_crate_fails_closed() {
        let rows = evaluate(&BTreeMap::new(), None, &["beton-policy", "beton-secrets"]);
        assert_eq!(rows[0].problems, ["nicht gemessen"]);
        assert!(
            rows[1].problems.is_empty(),
            "beton-secrets wird nur berichtet"
        );
    }

    #[test]
    fn qa_011_ac2_report_lists_every_crate() {
        let current = per_crate(&export(&[
            ("/w/crates/beton-policy/src/a.rs", 95, 100),
            ("C:\\w\\crates\\beton-secrets\\src\\a.rs", 50, 100),
        ]))
        .unwrap();
        let baseline = BTreeMap::from([("beton-policy".to_owned(), 94.0)]);
        let rows = evaluate(
            &current,
            Some(&baseline),
            &["beton-policy", "beton-secrets"],
        );
        let md = markdown(&rows);
        assert!(
            md.contains("| `beton-policy` | 95.00 % | 90 % | 94.00 % | ok |"),
            "{md}"
        );
        assert!(
            md.contains("| `beton-secrets` | 50.00 % | nur Bericht | – | ok |"),
            "{md}"
        );
        assert_eq!(
            summary_json(&current),
            json!({ "beton-policy": 95.0, "beton-secrets": 50.0 })
        );
    }

    #[test]
    fn qa_011_ac3_ratchet_against_main() {
        let current =
            per_crate(&export(&[("/w/crates/beton-policy/src/a.rs", 937, 1000)])).unwrap();
        let tolerated = parse_baseline(&json!({ "beton-policy": 93.8 }));
        assert!(
            evaluate(&current, Some(&tolerated), &["beton-policy"])[0]
                .problems
                .is_empty()
        );
        let dropped = parse_baseline(&json!({ "beton-policy": 94.0 }));
        let rows = evaluate(&current, Some(&dropped), &["beton-policy"]);
        assert!(rows[0].problems[0].starts_with("Ratchet"), "{:?}", rows[0]);
        assert!(markdown(&rows).contains("❌"));
    }

    #[test]
    fn export_without_files_is_an_error() {
        assert!(per_crate(&json!({})).is_err());
        assert_eq!(
            Lines {
                covered: 0,
                count: 0
            }
            .percent(),
            100.0
        );
    }
}
