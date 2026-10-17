//! SEC-013 AC3: Vergleich des Redaction-Benchmarks mit der Basis, die derselbe CI-Job auf dem
//! Stand von `main` gemessen hat (#307).

use super::{baseline_json, baseline_value, load_baseline, verdict};

#[test]
fn sec_013_ac3_baseline_is_the_value_measured_on_main() {
    assert_eq!(baseline_value(r#"{"mb_per_s":197.0}"#), Ok(197.0));
}

#[test]
fn sec_013_ac3_baseline_written_by_the_previous_benchmark_is_accepted() {
    // Der Benchmark auf `main` vor #307 schreibt zusätzlich eine Messreihe (`history`). Maßgeblich
    // ist allein seine eigene Messung im selben Job, nicht die Reihe.
    assert_eq!(
        baseline_value(r#"{"mb_per_s":197.0,"history":[197.0]}"#),
        Ok(197.0)
    );
    assert_eq!(
        baseline_value(r#"{"mb_per_s":197.0,"history":[300.1,202.4,197.0]}"#),
        Ok(197.0)
    );
}

#[test]
fn sec_013_ac3_invalid_baseline_is_rejected() {
    for text in [
        "",
        "kein json",
        "{}",
        "[]",
        r#"{"history":[200.0]}"#,
        r#"{"mb_per_s":"200"}"#,
        r#"{"mb_per_s":null}"#,
        r#"{"mb_per_s":0}"#,
        r#"{"mb_per_s":-5}"#,
    ] {
        assert!(baseline_value(text).is_err(), "{text:?} wurde akzeptiert");
    }
}

#[test]
fn sec_013_ac3_regression_over_20_percent_against_main_fails() {
    // Toleranz 20 %: 0,8 × 205 = 164.
    assert_eq!(verdict(205.0, Some(205.0)), Ok(()));
    assert_eq!(verdict(300.0, Some(205.0)), Ok(()));
    assert_eq!(verdict(164.0, Some(205.0)), Ok(()));
    let err = verdict(163.9, Some(205.0)).unwrap_err();
    assert!(err.contains("Regression"), "{err}");
    assert!(err.contains("205.0"), "{err}");
}

#[test]
fn sec_013_ac3_floor_of_50_mb_per_s_always_applies() {
    // Ohne Basis (lokal, Push auf main) gilt nur der Floor …
    assert_eq!(verdict(50.0, None), Ok(()));
    assert!(verdict(49.9, None).is_err());
    // … mit Basis zusätzlich, auch wenn main selbst schon langsam ist.
    assert!(verdict(49.9, Some(55.0)).is_err());
    assert!(verdict(49.9, Some(49.0)).is_err());
}

#[test]
fn sec_013_ac3_missing_baseline_file_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    // Ohne angeforderte Basis: kein Vergleich.
    assert_eq!(load_baseline(None), Ok(None));
    // Angefordert, aber nicht gemessen (Basis-Lauf gescheitert): kein stilles Überspringen.
    let missing = dir.path().join("redaction-base.json");
    let err = load_baseline(Some(&missing)).unwrap_err();
    assert!(err.contains("redaction-base.json"), "{err}");
    // Vorhanden, aber kaputt: ebenfalls ein Fehler.
    std::fs::write(&missing, "{}").unwrap();
    assert!(load_baseline(Some(&missing)).is_err());
    std::fs::write(&missing, r#"{"mb_per_s":201.5}"#).unwrap();
    assert_eq!(load_baseline(Some(&missing)), Ok(Some(201.5)));
}

#[test]
fn sec_013_ac3_written_baseline_round_trips() {
    let text = baseline_json(202.4);
    assert_eq!(baseline_value(&text), Ok(202.4));
}
