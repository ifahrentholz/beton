//! SEC-013 AC3: Basis des Redaction-Benchmarks als Median der letzten main-Messungen (#296).

use super::{advance, baseline_history, baseline_json, check_regression, median};

#[test]
fn sec_013_ac3_old_baseline_without_history_counts_as_one_value() {
    assert_eq!(
        baseline_history(r#"{"mb_per_s":300.1}"#).unwrap(),
        vec![300.1]
    );
    assert_eq!(
        median(&baseline_history(r#"{"mb_per_s":300.1}"#).unwrap()),
        Some(300.1)
    );
}

#[test]
fn sec_013_ac3_history_wins_over_current_value() {
    let text = r#"{"mb_per_s":210.0,"history":[202.4,300.1,210.0]}"#;
    assert_eq!(baseline_history(text).unwrap(), vec![202.4, 300.1, 210.0]);
}

#[test]
fn sec_013_ac3_reading_keeps_last_five_values() {
    let text = r#"{"mb_per_s":6.0,"history":[1,2,3,4,5,6]}"#;
    assert_eq!(
        baseline_history(text).unwrap(),
        vec![2.0, 3.0, 4.0, 5.0, 6.0]
    );
}

#[test]
fn sec_013_ac3_invalid_baseline_is_rejected() {
    for text in [
        "",
        "kein json",
        "{}",
        "[]",
        r#"{"mb_per_s":"200"}"#,
        r#"{"mb_per_s":0}"#,
        r#"{"mb_per_s":-5}"#,
        r#"{"history":[]}"#,
        r#"{"history":200}"#,
        r#"{"history":[200,"x"]}"#,
        r#"{"history":[200,0]}"#,
        r#"{"mb_per_s":200,"history":[]}"#,
    ] {
        assert!(baseline_history(text).is_err(), "{text:?} wurde akzeptiert");
    }
}

#[test]
fn sec_013_ac3_median_of_odd_and_even_series() {
    assert_eq!(median(&[]), None);
    assert_eq!(median(&[300.1]), Some(300.1));
    assert_eq!(median(&[200.0, 300.0]), Some(250.0));
    assert_eq!(median(&[300.1, 202.4, 210.0]), Some(210.0));
    assert_eq!(median(&[202.4, 300.1, 205.0, 199.0, 210.0]), Some(205.0));
    assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), Some(2.5));
}

#[test]
fn sec_013_ac3_single_outlier_does_not_set_the_baseline() {
    // Issue #296: Eine Messung mit 300 MB/s zwischen sonst ~200 MB/s.
    let history = [202.4, 300.1, 205.0, 199.0, 210.0];
    assert_eq!(check_regression(203.7, &history), Ok(205.0));
    // Toleranz unverändert 20 %: 0,8 × 205 = 164.
    assert!(check_regression(164.0, &history).is_ok());
    assert!(check_regression(163.9, &history).is_err());
}

#[test]
fn sec_013_ac3_regression_against_old_baseline_still_fails() {
    // Alte Basis-Datei: Median aus genau einem Wert, keine Abschwächung.
    let history = baseline_history(r#"{"mb_per_s":250.0}"#).unwrap();
    assert!(check_regression(200.0, &history).is_ok());
    assert!(check_regression(199.9, &history).is_err());
    assert!(check_regression(100.0, &[]).is_err());
}

#[test]
fn sec_013_ac3_recording_appends_and_keeps_last_five() {
    assert_eq!(advance(&[], 200.0), vec![200.0]);
    assert_eq!(advance(&[300.1], 202.4), vec![300.1, 202.4]);
    assert_eq!(
        advance(&[1.0, 2.0, 3.0, 4.0, 5.0], 6.0),
        vec![2.0, 3.0, 4.0, 5.0, 6.0]
    );
}

#[test]
fn sec_013_ac3_written_baseline_round_trips() {
    let history = advance(&baseline_history(r#"{"mb_per_s":300.1}"#).unwrap(), 202.4);
    let text = baseline_json(202.4, &history);
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["mb_per_s"].as_f64(), Some(202.4));
    assert_eq!(baseline_history(&text).unwrap(), vec![300.1, 202.4]);
}
