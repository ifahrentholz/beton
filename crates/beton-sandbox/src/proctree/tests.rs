//! Tests der Nachfahren-Erfassung (SBX-013).

#![allow(clippy::unwrap_used, clippy::disallowed_methods)]

use super::*;
use crate::command::{CommandSpec, Stdio};

/// Prozesse, deren Kommandozeile `marker` enthält.
fn processes_with(marker: &str) -> Vec<u32> {
    let out = std::process::Command::new("pgrep")
        .args(["-f", marker])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect()
}

/// SBX-013 AC1 (#226): Gemessen auf einem Entwickler-Mac, an dem die Meldung fehlte. Die
/// lesbaren Prozesse des Benutzers blieben 3 unter dem Limit, weil der Kernel Prozesse
/// mitzählt, deren Daten `proc_pidinfo` nicht liefert; die Kernel-Liste der Prozesse mit
/// dieser realen UID lag dagegen darüber.
#[test]
fn sbx_013_ac1_limit_counts_processes_the_kernel_lists_but_does_not_describe() {
    // Der Fall aus #226: lesbar 724, Kernel-Liste 732, Limit 727.
    assert!(limit_reached(724, 732, 727));
    // Ohne verborgene Prozesse wie bisher mit Abstand `SLACK`.
    assert!(limit_reached(725, 725, 727));
    assert!(!limit_reached(724, 724, 727));
    // Weit unter dem Limit meldet keine der beiden Zählungen.
    assert!(!limit_reached(700, 708, 727));
}

/// SBX-013 AC2 (#224): Ein per `setsid` abgelöster Nachfahre endet mit dem Stoppen, auch wenn
/// der Hintergrund-Thread ihn noch nicht erfasst hat (unter Last läuft er seltener als alle
/// 25 ms). Der Tracker hier hat keinen Hintergrund-Thread: Erfasst wird nur beim Stoppen,
/// und das muss geschehen, solange die Wurzel noch lebt.
#[tokio::test]
async fn sbx_013_ac2_stop_finds_setsid_descendants_without_a_background_scan() {
    let marker = format!("1001.{}", std::process::id());
    let script = format!(
        "perl -e 'use POSIX; POSIX::setsid(); exec \"sleep\", \"{marker}\"' & sleep {marker}"
    );
    let spec = CommandSpec::new("/bin/sh").stdio(Stdio::Null, Stdio::Null, Stdio::Null);
    let mut cmd = crate::child::command(std::path::Path::new("/bin/sh"), &spec);
    cmd.args(["-c", &script]);
    let child = crate::child::spawn(cmd).unwrap();
    let root = child.id().unwrap();
    let tracker = Tracker {
        state: Arc::new(Mutex::new(State {
            root: root as i32,
            known: BTreeMap::new(),
        })),
        stop: Arc::new(AtomicBool::new(true)),
    };
    let mut child = child.with_guard(Box::new(tracker), Violations::default());
    let start = std::time::Instant::now();
    while processes_with(&format!("sleep {marker}")).len() < 2 {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "Prozesse starten nicht"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    child.start_kill().unwrap();
    let _ = child.wait().await;
    let start = std::time::Instant::now();
    loop {
        let left = processes_with(&marker);
        if left.is_empty() {
            break;
        }
        if start.elapsed() > Duration::from_secs(2) {
            // Aufräumen, damit der Test keinen Prozess hinterlässt.
            for pid in &left {
                // SAFETY: Signal an einen eben gefundenen Testprozess.
                unsafe { libc::kill(*pid as i32, libc::SIGKILL) };
            }
            panic!("übrig: {left:?}");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
