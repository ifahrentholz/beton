//! Verteilung von Events an verbundene Clients (PROTO-005).
//!
//! Dauerhafte Events werden erst nach dem Commit im Store veröffentlicht ([`EventService`]),
//! transiente direkt mit `tseq` aus dem Ringpuffer (PROTO-003). Wer zu spät kommt oder den
//! Broadcast verpasst, holt dauerhafte Events aus dem Store nach.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use beton_core::event::Event;
use beton_core::id::{OrgId, SessionId};
use beton_proto::transient::{BufferLimits, NotTransient, TransientBuffer};
use beton_store::Store;
use tokio::sync::broadcast;

/// Kapazität des Broadcasts je Session; wer weiter zurückliegt, holt aus dem Store nach.
const CHANNEL_CAPACITY: usize = 4096;

struct SessionChannel {
    tx: broadcast::Sender<Arc<Event>>,
    transient: TransientBuffer,
}

/// Live-Verteiler je Session.
#[derive(Default)]
pub struct Hub {
    sessions: Mutex<HashMap<SessionId, SessionChannel>>,
}

impl std::fmt::Debug for Hub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Hub")
    }
}

impl Hub {
    fn with<R>(&self, session: SessionId, f: impl FnOnce(&mut SessionChannel) -> R) -> R {
        let mut map = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        let ch = map.entry(session).or_insert_with(|| {
            let mut transient = TransientBuffer::new(BufferLimits::default());
            transient.start_epoch(1);
            SessionChannel {
                tx: broadcast::channel(CHANNEL_CAPACITY).0,
                transient,
            }
        });
        f(ch)
    }

    /// Abonniert eine Session; liefert zusätzlich den Snapshot laufender Deltas.
    pub fn subscribe(&self, session: SessionId) -> (broadcast::Receiver<Arc<Event>>, Vec<Event>) {
        self.with(session, |ch| (ch.tx.subscribe(), ch.transient.snapshot()))
    }

    /// Veröffentlicht frisch gespeicherte, dauerhafte Events (in `seq`-Reihenfolge).
    pub fn publish_durable(&self, session: SessionId, events: &[Event]) {
        self.with(session, |ch| {
            for e in events {
                ch.transient.observe_durable(e);
                let _ = ch.tx.send(Arc::new(e.clone()));
            }
        });
    }

    /// Veröffentlicht ein transientes Event; vergibt `tseq`.
    pub fn publish_transient(&self, event: Event) -> Result<Event, NotTransient> {
        self.with(event.session_id, |ch| {
            let stamped = ch.transient.push(event, Instant::now())?;
            let _ = ch.tx.send(Arc::new(stamped.clone()));
            Ok(stamped)
        })
    }

    /// Zahl der Abonnenten (Tests, Diagnose).
    pub fn subscribers(&self, session: SessionId) -> usize {
        self.with(session, |ch| ch.tx.receiver_count())
    }
}

/// Anhängen mit anschließender Verteilung.
#[derive(Debug, Clone)]
pub struct EventService {
    pub store: Store,
    pub hub: Arc<Hub>,
}

impl EventService {
    pub async fn append(
        &self,
        org: OrgId,
        session: SessionId,
        expected_head: u64,
        epoch: u64,
        events: Vec<Event>,
    ) -> beton_store::Result<Vec<Event>> {
        let written = self
            .store
            .append(org, session, expected_head, epoch, events)
            .await?;
        self.hub.publish_durable(session, &written);
        Ok(written)
    }
}
