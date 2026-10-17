//! Tests der Nachfahren-Erfassung (SBX-013).

#![allow(clippy::unwrap_used, clippy::disallowed_methods)]

use super::*;
use crate::child::SandboxedChild;
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

/// Startet `sh` mit einem per `setsid` abgelösten Nachfahren und einem Tracker ohne
/// Hintergrund-Thread (wie unter Last, wenn er noch nicht gelaufen ist). Liefert Kind und
/// Markierung, sobald beide `sleep` laufen.
async fn tree_with_setsid_descendant(tag: &str) -> (SandboxedChild, String) {
    let marker = format!("{tag}.{}", std::process::id());
    let script = format!(
        "perl -e 'use POSIX; POSIX::setsid(); exec \"sleep\", \"{marker}\"' & sleep {marker}"
    );
    let spec = CommandSpec::new("/bin/sh").stdio(Stdio::Null, Stdio::Null, Stdio::Null);
    let mut cmd = crate::child::command(std::path::Path::new("/bin/sh"), &spec).unwrap();
    cmd.args(["-c", &script]);
    let child = crate::child::spawn(cmd, false).unwrap();
    let root = child.id().unwrap();
    let tracker = Tracker {
        state: Arc::new(Mutex::new(State {
            root: root as i32,
            ..State::default()
        })),
        stop: Arc::new(AtomicBool::new(true)),
    };
    let child = child.with_guard(Box::new(tracker), Violations::default());
    let start = std::time::Instant::now();
    while processes_with(&format!("sleep {marker}")).len() < 2 {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "Prozesse starten nicht"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    (child, marker)
}

/// Wartet bis zu 2 s, bis kein Prozess mit `marker` mehr läuft; räumt sonst auf und scheitert.
async fn assert_gone(marker: &str) {
    let start = std::time::Instant::now();
    loop {
        let left = processes_with(marker);
        if left.is_empty() {
            return;
        }
        if start.elapsed() > Duration::from_secs(2) {
            for pid in &left {
                // SAFETY: Signal an einen eben gefundenen Testprozess.
                unsafe { libc::kill(*pid as i32, libc::SIGKILL) };
            }
            panic!("übrig: {left:?}");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// SBX-013 AC2 (#224): Ein per `setsid` abgelöster Nachfahre endet mit dem Stoppen, auch wenn
/// der Hintergrund-Thread ihn noch nicht erfasst hat (unter Last läuft er seltener als alle
/// 25 ms). Erfasst wird nur beim Stoppen, und das muss geschehen, solange die Wurzel noch lebt.
#[tokio::test]
async fn sbx_013_ac2_stop_finds_setsid_descendants_without_a_background_scan() {
    let (mut child, marker) = tree_with_setsid_descendant("1001").await;
    child.start_kill().unwrap();
    let _ = child.wait().await;
    assert_gone(&marker).await;
}

/// SBX-013 AC2 (Security-Review zu #224): Auch der geordnete Weg (SIGTERM an den Baum, danach
/// Fallenlassen) erfasst den Baum, bevor die Wurzel das Signal bekommt.
#[tokio::test]
async fn sbx_013_ac2_signal_then_drop_finds_setsid_descendants() {
    let (mut child, marker) = tree_with_setsid_descendant("1002").await;
    child.signal_tree(libc::SIGTERM).unwrap();
    let _ = child.wait().await;
    drop(child);
    assert_gone(&marker).await;
}

/// SBX-013 AC2 (#244): Ein Prozess, der beim Stoppen laufend Kinder startet (jedes per
/// `setsid` abgelöst), verliert keines: Zwischen Erfassung und SIGKILL entstandene Kinder
/// hingen sonst nach dem Tod des Elternprozesses unerkannt an launchd. Der Baum wird vor dem
/// Beenden eingefroren (SIGSTOP), erfasst, bis nichts Neues hinzukommt, und dann beendet. Das
/// Zeitfenster ist kurz; mehrere Durchläufe machen den Fall verlässlich.
#[tokio::test]
async fn sbx_013_ac2_stop_keeps_children_forked_while_stopping() {
    for round in 0..4 {
        let marker = format!("10{round}4.{}", std::process::id());
        let script = format!(
            "use POSIX; for (1..4000) {{ my $p = fork(); if (defined $p && $p == 0) {{ \
             POSIX::setsid(); exec \"sleep\", \"{marker}\"; }} select(undef, undef, undef, 0.0005); }} \
             sleep 60"
        );
        let spec = CommandSpec::new("/usr/bin/perl").stdio(Stdio::Null, Stdio::Null, Stdio::Null);
        let mut cmd = crate::child::command(std::path::Path::new("/usr/bin/perl"), &spec).unwrap();
        cmd.args(["-e", &script]);
        let child = crate::child::spawn(cmd, false).unwrap();
        let root = child.id().unwrap();
        let tracker = Tracker::start(root, SandboxStage::Tools, None, Violations::default(), None);
        let mut child = child.with_guard(Box::new(tracker), Violations::default());
        let start = std::time::Instant::now();
        while processes_with(&format!("sleep {marker}")).len() < 40 {
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "Kinder starten nicht"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        child.start_kill().unwrap();
        let _ = child.wait().await;
        drop(child);
        assert_gone(&marker).await;
    }
}

/// #244: Ein Signal an den Baum friert ihn nur kurz ein; danach laufen alle Prozesse weiter
/// (hier SIGWINCH, das sie ignorieren), auch der per `setsid` abgelöste.
#[tokio::test]
async fn sbx_013_signal_tree_leaves_no_process_stopped() {
    let (mut child, marker) = tree_with_setsid_descendant("1005").await;
    child.signal_tree(libc::SIGWINCH).unwrap();
    for pid in processes_with(&format!("sleep {marker}")) {
        let out = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        let stat = String::from_utf8_lossy(&out.stdout);
        assert!(!stat.contains('T'), "{pid} angehalten: {stat}");
    }
    child.start_kill().unwrap();
    let _ = child.wait().await;
    drop(child);
    assert_gone(&marker).await;
}
