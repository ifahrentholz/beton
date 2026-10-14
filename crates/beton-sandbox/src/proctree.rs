//! Nachfahren-Erfassung unter macOS (SBX-013): Seatbelt kennt keinen PID-Namespace, und ein
//! Prozess kann die Prozessgruppe (`setsid`) verlassen. Ein Hintergrund-Thread erfasst deshalb
//! regelmäßig alle Nachfahren des Wurzelprozesses (über die Eltern-PID) und alle Mitglieder
//! seiner Prozessgruppe, merkt sie sich mit Startzeit (gegen PID-Wiederverwendung) und beendet
//! sie beim Stoppen. Er meldet außerdem, wenn das `pids`-Limit erreicht ist (`RLIMIT_NPROC` zählt
//! unter macOS alle Prozesse des Benutzers).
//!
//! Restrisiko (dokumentiert): Ein Prozess, der per `setsid` und doppeltem `fork` zwischen zwei
//! Erfassungen verwaist, ist nicht mehr als Nachfahre erkennbar; er bleibt in der Sandbox
//! gefangen (das Profil wird vererbt), wird aber nur best effort beendet.
#![allow(unsafe_code, reason = "Prozessliste und Signale gibt es nur über libc")]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use beton_core::event::{SandboxStage, SandboxViolation, ViolationKind};

use crate::child::{TreeGuard, Violations};
use crate::registry::ViolationSink;

/// Abstand zum Limit, ab dem gemeldet wird (Erfassung ist nicht atomar).
const SLACK: u32 = 2;

#[derive(Debug, Clone, Copy)]
struct Proc {
    pid: i32,
    ppid: i32,
    pgid: i32,
    ruid: u32,
    start: (u64, u64),
}

fn list() -> Vec<Proc> {
    // SAFETY: proc_listallpids/proc_pidinfo schreiben in eigene, ausreichend große Puffer.
    unsafe {
        let n = libc::proc_listallpids(std::ptr::null_mut(), 0);
        if n <= 0 {
            return Vec::new();
        }
        let mut pids = vec![0 as libc::pid_t; n as usize + 64];
        let bytes = (pids.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int;
        let n = libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes);
        if n <= 0 {
            return Vec::new();
        }
        pids.truncate(n as usize);
        let mut out = Vec::with_capacity(pids.len());
        for pid in pids {
            let mut info: libc::proc_bsdinfo = std::mem::zeroed();
            let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
            let r = libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                size,
            );
            if r != size {
                continue;
            }
            out.push(Proc {
                pid,
                ppid: info.pbi_ppid as i32,
                pgid: info.pbi_pgid as i32,
                ruid: info.pbi_ruid,
                start: (info.pbi_start_tvsec, info.pbi_start_tvusec),
            });
        }
        out
    }
}

/// `proc_listpids`-Filter für die reale UID (`PROC_RUID_ONLY` aus `libproc.h`).
const PROC_RUID_ONLY: u32 = 5;

/// Prozesse mit realer UID `uid` laut der Liste des Kernels. Sie enthält auch Prozesse, deren
/// Daten `proc_pidinfo` nicht liefert (z. B. geschützte Prozesse); [`list`] sieht diese nicht.
fn listed_of_ruid(uid: u32) -> u64 {
    // SAFETY: proc_listpids schreibt höchstens `bytes` Bytes in den eigenen Puffer.
    unsafe {
        let n = libc::proc_listpids(PROC_RUID_ONLY, uid, std::ptr::null_mut(), 0);
        if n <= 0 {
            return 0;
        }
        let mut pids = vec![0 as libc::pid_t; n as usize / std::mem::size_of::<libc::pid_t>() + 64];
        let bytes = (pids.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int;
        let n = libc::proc_listpids(PROC_RUID_ONLY, uid, pids.as_mut_ptr().cast(), bytes);
        if n <= 0 {
            return 0;
        }
        pids.truncate(n as usize / std::mem::size_of::<libc::pid_t>());
        pids.iter().filter(|p| **p > 0).count() as u64
    }
}

/// Ist das `pids`-Limit erreicht? Der Kernel zählt für `RLIMIT_NPROC` alle Prozesse der realen
/// UID; genau sehen lässt sich diese Zahl nicht. Die lesbaren Prozesse (`visible`) zählen zu
/// wenig, wenn der Benutzer Prozesse hat, die `proc_pidinfo` nicht beschreibt; die Liste des
/// Kernels (`listed`) zählt eher etwas zu viel. Gemeldet wird, sobald eine der beiden das Limit
/// erreicht (#226).
fn limit_reached(visible: u64, listed: u64, limit: u64) -> bool {
    visible + u64::from(SLACK) >= limit || listed >= limit
}

/// Prozesse, die ein Baum mit `limits.pids = p` unter macOS starten darf: Andere Prozesse des
/// Benutzers können währenddessen enden und Platz freigeben, deshalb bleibt ein Abstand (ein
/// Viertel, höchstens 32). Strenger ist hier sicherer als ungenau.
pub(crate) fn budget(p: u32) -> u32 {
    p - (p / 4).min(32)
}

/// Anzahl der Prozesse eines Benutzers (für `RLIMIT_NPROC`, das unter macOS je Benutzer zählt).
pub(crate) fn count_of_uid(uid: u32) -> u64 {
    list().iter().filter(|p| p.ruid == uid).count() as u64
}

#[derive(Debug, Default)]
struct State {
    root: i32,
    known: BTreeMap<i32, (u64, u64)>,
}

impl State {
    /// Aktualisiert die bekannten Nachfahren; liefert ihre Anzahl und die Zahl der Prozesse
    /// von `uid`.
    fn scan_counting(&mut self, uid: Option<u32>) -> (usize, u64) {
        let procs = list();
        let of_uid = uid.map_or(0, |u| procs.iter().filter(|p| p.ruid == u).count() as u64);
        let by_pid: BTreeMap<i32, Proc> = procs.iter().map(|p| (p.pid, *p)).collect();
        self.known
            .retain(|pid, start| by_pid.get(pid).is_some_and(|p| p.start == *start));
        if let Some(r) = by_pid.get(&self.root) {
            self.known.entry(self.root).or_insert(r.start);
        }
        loop {
            let mut added = false;
            for p in &procs {
                if self.known.contains_key(&p.pid) {
                    continue;
                }
                if self.known.contains_key(&p.ppid) || p.pgid == self.root {
                    self.known.insert(p.pid, p.start);
                    added = true;
                }
            }
            if !added {
                break;
            }
        }
        (self.known.len(), of_uid)
    }

    fn scan(&mut self) -> usize {
        self.scan_counting(None).0
    }
}

/// Erfasst den Baum eines Seatbelt-Kindes und beendet ihn beim Stoppen.
pub(crate) struct Tracker {
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
}

impl Tracker {
    pub(crate) fn start(
        root: u32,
        stage: SandboxStage,
        nproc: Option<(u32, u64)>,
        violations: Violations,
        sink: Option<ViolationSink>,
    ) -> Self {
        let state = Arc::new(Mutex::new(State {
            root: root as i32,
            known: BTreeMap::new(),
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let (s, st) = (state.clone(), stop.clone());
        std::thread::spawn(move || {
            let mut reported = false;
            while !st.load(Ordering::Relaxed) {
                let (n, of_uid) = s
                    .lock()
                    .map(|mut g| g.scan_counting(nproc.map(|(u, _)| u)))
                    .unwrap_or((0, 0));
                if n == 0 {
                    break;
                }
                // Der Kernel prüft RLIMIT_NPROC gegen alle Prozesse des Benutzers; genau das
                // zählt hier.
                if let Some((uid, limit)) = nproc
                    && !reported
                    && limit_reached(of_uid, listed_of_ruid(uid), limit)
                {
                    reported = true;
                    let v = SandboxViolation {
                        stage,
                        kind: ViolationKind::Limit,
                        target: Some("pids".into()),
                        suppressed_count: 0,
                    };
                    if let Ok(mut l) = violations.lock() {
                        l.push(v.clone());
                    }
                    if let Some(sink) = &sink {
                        sink(v);
                    }
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        });
        Self { state, stop }
    }
}

impl TreeGuard for Tracker {
    /// Nicht erst auf den nächsten Durchlauf des Hintergrund-Threads warten: Was jetzt zum Baum
    /// gehört, ist danach bekannt, auch wenn es die Prozessgruppe verlassen hat.
    fn capture(&mut self) {
        if let Ok(mut s) = self.state.lock() {
            s.scan();
        }
    }

    fn kill_tree(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let Ok(mut s) = self.state.lock() else { return };
        for _ in 0..20 {
            let n = s.scan();
            // SAFETY: Signale an die Prozessgruppe und an erfasste Nachfahren (mit geprüfter
            // Startzeit, also keine wiederverwendeten PIDs).
            unsafe {
                libc::killpg(s.root, libc::SIGKILL);
                for pid in s.known.keys() {
                    libc::kill(*pid, libc::SIGKILL);
                }
            }
            if n == 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
mod tests {
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
}
