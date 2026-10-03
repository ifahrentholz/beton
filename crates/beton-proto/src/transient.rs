//! Transiente Events und Delta-Ringpuffer (PROTO-003).
//!
//! `*.delta`, `presence.updated` und Kanal-Frames landen nie im Session-Log. Der Home-Knoten
//! hält pro Session einen Ringpuffer (10 000 Einträge oder 5 min) und den bisher
//! angesammelten Text laufender Nachrichten und Tool-Ausgaben. Ein Client, der mitten in einer
//! Nachricht neu verbindet, bekommt nach den dauerhaften Events einen Snapshot: je laufender
//! Nachricht ein Delta mit dem vollen bisherigen Text und `snapshot: true`. Mit dem passenden
//! `*.completed` (dauerhaft) ist der Zustand vollständig im Log und der Puffer räumt auf.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use beton_core::event::{Event, EventPayload, OutputStream, TextDelta, ToolCallOutputDelta};

/// Grenzen des Ringpuffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferLimits {
    pub max_entries: usize,
    pub max_age: Duration,
}

impl Default for BufferLimits {
    fn default() -> Self {
        Self {
            max_entries: 10_000,
            max_age: Duration::from_secs(5 * 60),
        }
    }
}

/// Schlüssel eines laufenden, angesammelten Textstroms.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum StreamKey {
    Message(String),
    Reasoning(String),
    ToolOutput(String, StreamSide),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum StreamSide {
    Stdout,
    Stderr,
}

impl From<OutputStream> for StreamSide {
    fn from(s: OutputStream) -> Self {
        match s {
            OutputStream::Stdout => Self::Stdout,
            OutputStream::Stderr => Self::Stderr,
        }
    }
}

/// Angesammelter Zustand eines laufenden Textstroms.
#[derive(Debug, Clone)]
struct Accumulated {
    text: String,
    /// Envelope des letzten Deltas als Vorlage für den Snapshot.
    template: Event,
}

/// Ringpuffer transienter Events einer Session auf dem Home-Knoten.
#[derive(Debug)]
pub struct TransientBuffer {
    limits: BufferLimits,
    epoch: u64,
    next_tseq: u64,
    ring: VecDeque<(Instant, Event)>,
    streams: BTreeMap<StreamKey, Accumulated>,
}

impl TransientBuffer {
    pub fn new(limits: BufferLimits) -> Self {
        Self {
            limits,
            epoch: 0,
            next_tseq: 1,
            ring: VecDeque::new(),
            streams: BTreeMap::new(),
        }
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Neue Epoch (z. B. nach Ownership-Wechsel): `tseq` beginnt wieder bei 1, Puffer und
    /// angesammelte Ströme werden verworfen.
    pub fn start_epoch(&mut self, epoch: u64) {
        self.epoch = epoch;
        self.next_tseq = 1;
        self.ring.clear();
        self.streams.clear();
    }

    /// Nimmt ein transientes Event auf, vergibt `tseq` und setzt `transient: true`.
    /// Gibt das gestempelte Event zurück, wie es an Clients geht. Dauerhafte Events werden
    /// zurückgewiesen; die gehören ins Log (siehe [`Self::observe_durable`]).
    pub fn push(&mut self, mut event: Event, now: Instant) -> Result<Event, NotTransient> {
        if !event.body.is_transient() {
            return Err(NotTransient(event.type_name()));
        }
        event.transient = true;
        event.tseq = Some(self.next_tseq);
        self.next_tseq += 1;
        self.accumulate(&event);
        self.ring.push_back((now, event.clone()));
        self.evict(now);
        Ok(event)
    }

    /// Meldet ein dauerhaftes Event; ein `*.completed` schließt den zugehörigen Strom ab.
    pub fn observe_durable(&mut self, event: &Event) {
        match &event.body {
            EventPayload::MessageCompleted(m) => {
                self.streams
                    .remove(&StreamKey::Message(m.message_id.clone()));
            }
            EventPayload::ReasoningCompleted(r) => {
                self.streams
                    .remove(&StreamKey::Reasoning(r.message_id.clone()));
            }
            EventPayload::ToolCallCompleted(t) => {
                self.streams.remove(&StreamKey::ToolOutput(
                    t.call_id.clone(),
                    StreamSide::Stdout,
                ));
                self.streams.remove(&StreamKey::ToolOutput(
                    t.call_id.clone(),
                    StreamSide::Stderr,
                ));
            }
            _ => {}
        }
    }

    /// Transiente Events mit `tseq` > `after`, soweit noch im Puffer.
    pub fn since(&self, after: u64) -> Vec<Event> {
        self.ring
            .iter()
            .filter(|(_, e)| e.tseq.is_some_and(|t| t > after))
            .map(|(_, e)| e.clone())
            .collect()
    }

    /// Snapshot laufender Ströme für einen (re)connectenden Client: je Strom ein Delta mit
    /// dem vollen bisherigen Text und `snapshot: true`. Die `tseq` ist die zuletzt vergebene.
    pub fn snapshot(&self) -> Vec<Event> {
        let tseq = self.next_tseq.saturating_sub(1);
        self.streams
            .iter()
            .map(|(key, acc)| {
                let mut e = acc.template.clone();
                e.tseq = Some(tseq);
                e.body = match key {
                    StreamKey::Message(id) => EventPayload::MessageDelta(TextDelta {
                        message_id: id.clone(),
                        text: acc.text.clone(),
                        snapshot: true,
                    }),
                    StreamKey::Reasoning(id) => EventPayload::ReasoningDelta(TextDelta {
                        message_id: id.clone(),
                        text: acc.text.clone(),
                        snapshot: true,
                    }),
                    StreamKey::ToolOutput(id, side) => {
                        EventPayload::ToolCallOutputDelta(ToolCallOutputDelta {
                            call_id: id.clone(),
                            stream: match side {
                                StreamSide::Stdout => OutputStream::Stdout,
                                StreamSide::Stderr => OutputStream::Stderr,
                            },
                            text: acc.text.clone(),
                            snapshot: true,
                        })
                    }
                };
                e
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.ring.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }

    fn accumulate(&mut self, event: &Event) {
        let (key, text, snapshot) = match &event.body {
            EventPayload::MessageDelta(d) => (
                StreamKey::Message(d.message_id.clone()),
                &d.text,
                d.snapshot,
            ),
            EventPayload::ReasoningDelta(d) => (
                StreamKey::Reasoning(d.message_id.clone()),
                &d.text,
                d.snapshot,
            ),
            EventPayload::ToolCallOutputDelta(d) => (
                StreamKey::ToolOutput(d.call_id.clone(), d.stream.into()),
                &d.text,
                d.snapshot,
            ),
            _ => return,
        };
        let entry = self.streams.entry(key).or_insert_with(|| Accumulated {
            text: String::new(),
            template: event.clone(),
        });
        if snapshot {
            entry.text.clone_from(text);
        } else {
            entry.text.push_str(text);
        }
        entry.template = event.clone();
    }

    fn evict(&mut self, now: Instant) {
        while self.ring.len() > self.limits.max_entries {
            self.ring.pop_front();
        }
        while self
            .ring
            .front()
            .is_some_and(|(t, _)| now.duration_since(*t) > self.limits.max_age)
        {
            self.ring.pop_front();
        }
    }
}

impl Default for TransientBuffer {
    fn default() -> Self {
        Self::new(BufferLimits::default())
    }
}

/// Ein dauerhaftes Event wurde dem transienten Puffer übergeben.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotTransient(pub &'static str);

impl std::fmt::Display for NotTransient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "`{}` ist kein transientes Event", self.0)
    }
}

impl std::error::Error for NotTransient {}

/// Client-seitige Sicht auf transiente Events einer Session (PROTO-003 AC3):
/// verwirft Events mit `tseq` ≤ der zuletzt gesehenen und setzt Text aus Deltas und
/// Snapshots zusammen.
#[derive(Debug, Default)]
pub struct TransientView {
    last_tseq: u64,
    epoch: u64,
    texts: BTreeMap<String, String>,
}

impl TransientView {
    /// Übernimmt ein transientes Event; `false`, wenn es verworfen wurde.
    pub fn accept(&mut self, event: &Event, epoch: u64) -> bool {
        if epoch != self.epoch {
            self.epoch = epoch;
            self.last_tseq = 0;
            self.texts.clear();
        }
        let Some(tseq) = event.tseq else { return false };
        let snapshot = matches!(&event.body, EventPayload::MessageDelta(d) if d.snapshot);
        // Ein Snapshot trägt die zuletzt vergebene tseq und ersetzt den Stand; sonst gilt
        // strikte Monotonie.
        if !(tseq > self.last_tseq || (snapshot && tseq >= self.last_tseq)) {
            return false;
        }
        self.last_tseq = tseq;
        if let EventPayload::MessageDelta(d) = &event.body {
            let text = self.texts.entry(d.message_id.clone()).or_default();
            if d.snapshot {
                text.clone_from(&d.text);
            } else {
                text.push_str(&d.text);
            }
        }
        true
    }

    /// Bisher angezeigter Text einer Nachricht.
    pub fn message_text(&self, message_id: &str) -> Option<&str> {
        self.texts.get(message_id).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use beton_core::event::{Actor, MessageCompleted, MessageRole, Persistence};
    use beton_core::id::SessionId;
    use proptest::prelude::*;

    use super::*;

    fn delta(session: SessionId, msg: &str, text: &str) -> Event {
        Event::new(
            session,
            10,
            Actor::Agent {
                id: None,
                harness: "fake".into(),
                agent_ref: None,
            },
            EventPayload::MessageDelta(TextDelta {
                message_id: msg.into(),
                text: text.into(),
                snapshot: false,
            }),
        )
    }

    fn completed(session: SessionId, msg: &str) -> Event {
        Event::new(
            session,
            11,
            Actor::Agent {
                id: None,
                harness: "fake".into(),
                agent_ref: None,
            },
            EventPayload::MessageCompleted(MessageCompleted {
                message_id: msg.into(),
                role: MessageRole::Assistant,
                content: vec![],
                author: None,
            }),
        )
    }

    #[test]
    fn proto_003_ac1_only_the_completed_event_is_durable() {
        let session = SessionId::new();
        let mut buffer = TransientBuffer::default();
        let now = Instant::now();
        let mut stream: Vec<Event> = (0..1_000)
            .map(|i| delta(session, "m1", &format!("{i} ")))
            .collect();
        stream.push(completed(session, "m1"));

        let mut durable = Vec::new();
        for e in stream {
            if e.body.persistence() == Persistence::Durable {
                buffer.observe_durable(&e);
                durable.push(e);
            } else {
                buffer.push(e, now).unwrap();
            }
        }
        assert_eq!(durable.len(), 1);
        assert_eq!(durable[0].type_name(), "message.completed");
        assert!(
            buffer.snapshot().is_empty(),
            "abgeschlossene Nachricht bleibt nicht im Snapshot"
        );
        assert!(buffer.push(completed(session, "m2"), now).is_err());
    }

    #[test]
    fn proto_003_ac2_reconnecting_client_sees_same_text() {
        let session = SessionId::new();
        let mut buffer = TransientBuffer::default();
        let now = Instant::now();
        let mut always_on = TransientView::default();
        let parts = ["Die ", "Login-Route ", "ist ", "jetzt ", "begrenzt."];

        let mut late_joiner = TransientView::default();
        for (i, p) in parts.iter().enumerate() {
            let e = buffer.push(delta(session, "m1", p), now).unwrap();
            assert!(always_on.accept(&e, 0));
            if i == 2 {
                // Client verbindet sich hier neu: bekommt den Snapshot, danach live.
                for s in buffer.snapshot() {
                    assert!(late_joiner.accept(&s, 0));
                }
            } else if i > 2 {
                assert!(late_joiner.accept(&e, 0));
            }
        }
        assert_eq!(
            always_on.message_text("m1"),
            Some("Die Login-Route ist jetzt begrenzt.")
        );
        assert_eq!(late_joiner.message_text("m1"), always_on.message_text("m1"));
    }

    #[test]
    fn proto_003_ac3_client_discards_old_or_duplicate_tseq() {
        let session = SessionId::new();
        let mut buffer = TransientBuffer::default();
        let now = Instant::now();
        let a = buffer.push(delta(session, "m1", "a"), now).unwrap();
        let b = buffer.push(delta(session, "m1", "b"), now).unwrap();
        let mut view = TransientView::default();
        assert!(view.accept(&a, 0));
        assert!(view.accept(&b, 0));
        assert!(!view.accept(&a, 0), "ältere tseq wird verworfen");
        assert!(!view.accept(&b, 0), "Duplikat wird verworfen");
        assert_eq!(view.message_text("m1"), Some("ab"));
    }

    #[test]
    fn proto_003_ac3_tseq_restarts_per_epoch() {
        let session = SessionId::new();
        let mut buffer = TransientBuffer::default();
        let now = Instant::now();
        buffer.push(delta(session, "m1", "a"), now).unwrap();
        buffer.start_epoch(2);
        let e = buffer.push(delta(session, "m1", "b"), now).unwrap();
        assert_eq!(e.tseq, Some(1));
        let mut view = TransientView::default();
        assert!(view.accept(&e, 2));
    }

    #[test]
    fn proto_003_ring_respects_entry_and_age_limits() {
        let session = SessionId::new();
        let mut buffer = TransientBuffer::new(BufferLimits {
            max_entries: 3,
            max_age: Duration::from_secs(300),
        });
        let t0 = Instant::now();
        for i in 0..5 {
            buffer
                .push(delta(session, "m1", &i.to_string()), t0)
                .unwrap();
        }
        assert_eq!(buffer.len(), 3);
        assert_eq!(buffer.since(0).first().and_then(|e| e.tseq), Some(3));
        buffer
            .push(delta(session, "m1", "spät"), t0 + Duration::from_secs(301))
            .unwrap();
        assert_eq!(buffer.len(), 1, "Einträge älter als 5 min fallen heraus");
        // Der angesammelte Text bleibt trotz Ringgrenze vollständig.
        let snap = buffer.snapshot();
        assert!(matches!(&snap[0].body, EventPayload::MessageDelta(d) if d.text == "01234spät"));
    }

    proptest! {
        #[test]
        fn proto_003_ac3_tseq_strictly_increasing(n in 1usize..200) {
            let session = SessionId::new();
            let mut buffer = TransientBuffer::default();
            let now = Instant::now();
            let mut last = 0;
            for _ in 0..n {
                let e = buffer.push(delta(session, "m", "x"), now).unwrap();
                let t = e.tseq.unwrap();
                prop_assert!(t > last);
                last = t;
            }
        }
    }
}
