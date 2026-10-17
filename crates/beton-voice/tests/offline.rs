//! VOI-008 AC2: Während einer Transkription baut `beton-voice` keine Verbindung auf.
//!
//! Eigenes Test-Binary, damit kein anderer Test im selben Prozess Sockets öffnet. Ein
//! Beobachter-Thread prüft während Laden, Spracherkennung und Transkription mit der echten
//! whisper.cpp-Engine laufend die offenen Sockets des Prozesses; in der Linux-CI läuft der
//! Test zusätzlich in einem Netz-Namespace nur mit Loopback.

#![allow(clippy::unwrap_used)]

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use beton_voice::service::{FixedSource, ServiceConfig, VoiceService};
use beton_voice::whisper::WhisperEngine;
use beton_voice::{Acceleration, Lang, LanguageSetting};
use support::{Dims, synthetic_model, tone};

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn voi_008_ac2_transcription_opens_no_network_connections() {
    let model = synthetic_model(Dims::SMALL);
    // Kontrollgruppe: Die Messung erkennt einen geöffneten Socket.
    let baseline = support::open_sockets();
    let control = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    assert!(
        support::open_sockets().difference(&baseline).count() == 1,
        "die Socket-Messung erkennt keinen neuen Socket"
    );
    drop(control);
    let before = support::open_sockets();
    let stop = Arc::new(AtomicBool::new(false));
    let observer = {
        let stop = stop.clone();
        let before = before.clone();
        std::thread::spawn(move || {
            let mut seen = std::collections::BTreeSet::new();
            let mut rounds = 0u64;
            while !stop.load(Ordering::SeqCst) {
                for s in support::open_sockets() {
                    if !before.contains(&s) {
                        seen.insert(s);
                    }
                }
                rounds += 1;
                std::thread::sleep(Duration::from_micros(200));
            }
            (seen, rounds)
        })
    };
    let svc = VoiceService::new(
        Arc::new(WhisperEngine::new()),
        Arc::new(FixedSource(model.path.clone())),
        ServiceConfig {
            idle_unload: Duration::from_secs(600),
            max_concurrent: 1,
            // Erkennung und Transkription: beide Pfade der Engine.
            language: LanguageSetting::Auto,
            languages: vec![Lang::De, Lang::En],
            acceleration: Acceleration::Auto,
        },
    );
    for _ in 0..2 {
        let t = svc.transcribe(tone(2), |_| {}).await.unwrap();
        assert!(t.detected);
    }
    stop.store(true, Ordering::SeqCst);
    let (seen, rounds) = observer.join().unwrap();
    assert!(rounds > 10, "Beobachter lief nur {rounds} Runden");
    assert!(
        seen.is_empty(),
        "neue Sockets während der Transkription: {seen:?}"
    );
}

/// VOI-008 AC2: Netzwerk-Code gibt es in `beton-voice` nur im Download-Modul.
#[test]
fn voi_008_ac2_only_the_download_module_uses_the_network() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for entry in std::fs::read_dir(&src).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name().unwrap() == "download.rs" {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for needle in [
            "reqwest::Client",
            "reqwest::get",
            "TcpStream",
            "UdpSocket",
            "TcpListener",
            "hyper::",
            "download::pull",
        ] {
            assert!(
                !text.contains(needle),
                "{} nutzt {needle}; Netzzugriffe nur in download.rs (VOI-008)",
                path.display()
            );
        }
    }
}
