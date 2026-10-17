//! Binärkanäle auf dem Server (PROTO-007): Vermittlung zwischen einer Quelle (PTY eines
//! Runners, Browser, Audio-Senke) und den Clients, die den Kanal geöffnet haben.
//!
//! Je Quelle gibt es ein [`Relay`]. Flusskontrolle ist kreditbasiert und durchgängig:
//! - Der Runner sendet höchstens so viele Bytes, wie der Server ihm gewährt
//!   ([`ChannelBackend::credit`]).
//! - Der Server leitet ein Frame erst weiter, wenn **jeder** Zuschauer genug Kredit hat, und
//!   gibt dem Runner den Kredit erst danach zurück. Ein langsamer Client bremst so den Sender:
//!   Das PTY-Lesen im Runner pausiert, der Speicher des Servers bleibt beim Fenster des Runners
//!   (PROTO-007 AC1). Ohne Zuschauer kommt der Kredit sofort zurück.
//! - Ein Zuschauer, der länger als [`STALL_TIMEOUT`] keinen Kredit gewährt, während Frames
//!   warten, verliert den Kanal (`channel_closed {reason: "stalled"}`), damit ein hängender
//!   Client das Terminal nicht für alle anhält.
//! - Neue Zuschauer eines Terminals bekommen zuerst ein Keyframe (Scrollback und Bildschirm,
//!   `KEYFRAME`, letztes Fragment `FIN`), danach die laufenden Frames.
//! - Browser-Frames zählen in Frames: höchstens [`BROWSER_MAX_UNACKED`] unbestätigt; kommt ein
//!   neues, während zwei ausstehen, ersetzt es das wartende (nur das neueste zählt, AC3).
//!
//! Terminal-Bytes, Browser-Frames und Audio werden hier nie gespeichert (DATA-011).

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use beton_core::id::SessionId;
use beton_proto::channel::{
    BROWSER_MAX_UNACKED, ChannelAccess, ChannelKind, DEFAULT_WINDOW, Frame, flags,
};

/// So lange darf ein Zuschauer den Sender aufhalten, ohne Kredit zu gewähren.
pub const STALL_TIMEOUT: Duration = Duration::from_secs(30);

/// Fenster, das der Server einem Runner je Kanal gewährt (Bytes in Fahrt bzw. wartend).
pub const RUNNER_WINDOW: u64 = DEFAULT_WINDOW;

/// Gegenstelle einer Quelle: Kredit, Keyframes und Eingaben gehen dorthin.
pub trait ChannelBackend: Send + Sync {
    /// Kredit für weitere Bytes des Senders.
    fn credit(&self, bytes: u64);
    /// Ein Keyframe anfordern (neuer Zuschauer eines Terminals).
    fn keyframe(&self);
    /// Eingabe eines Clients (`INPUT`); `false`, wenn die Quelle keine annimmt.
    fn input(&self, payload: Vec<u8>) -> bool;
}

/// Höchstens so viele Bytes stehen je Client in seiner Ausgangswarteschlange, auch wenn er
/// mehr Kredit gewährt (der Server bleibt beschränkt, wenn TCP stockt).
pub const CLIENT_BACKLOG: usize = 1024 * 1024;

/// Ausgang zu einem Client.
pub trait ClientSink: Send + Sync {
    /// Ein Binärframe senden (mit der Kanal-ID des Clients).
    fn frame(&self, frame: Frame) -> bool;
    /// Kanal beendet.
    fn closed(&self, channel_id: u32, reason: &str);
    /// Bytes, die zu diesem Client noch nicht gesendet sind.
    fn backlog(&self) -> usize {
        0
    }
}

/// Kennung eines Zuschauers innerhalb eines Relays.
pub type SubId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Wartet auf den Anfang eines Keyframes.
    Awaiting,
    /// Bekommt gerade die Fragmente eines Keyframes.
    Receiving,
    /// Hat den Zustand und liest laufend mit.
    Synced,
}

struct Sub {
    sink: Arc<dyn ClientSink>,
    channel_id: u32,
    /// Kredit des Clients (Bytes bzw. Frames).
    window: u64,
    /// Browser: gesendete, noch nicht bestätigte Frames.
    outstanding: u32,
    /// Browser: das neueste wartende Frame.
    pending: Option<Vec<Frame>>,
    sync: Phase,
}

struct State {
    backend: Option<Arc<dyn ChannelBackend>>,
    queue: VecDeque<Frame>,
    queued_bytes: usize,
    subs: HashMap<SubId, Sub>,
    next_sub: SubId,
    /// Ein Keyframe ist angefordert und noch nicht angekommen.
    keyframe_requested: bool,
    /// Gerade laufen Fragmente eines Keyframes.
    in_keyframe: bool,
    /// Browser: Fragmente des laufenden Bildes.
    partial: Vec<Frame>,
    /// Seit wann das vorderste Frame auf Kredit eines Zuschauers wartet.
    blocked_since: Option<Instant>,
    closed: bool,
}

/// Vermittlung einer Quelle an ihre Zuschauer.
pub struct Relay {
    pub session: SessionId,
    pub kind: ChannelKind,
    pub target: String,
    state: Mutex<State>,
    /// Zähler des Hubs für Bytes in Warteschlangen (Messung, PROTO-007 AC1).
    buffered: Arc<AtomicUsize>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Relay {
    /// Bytes, die gerade auf Zuschauer warten.
    pub fn queued_bytes(&self) -> usize {
        lock(&self.state).queued_bytes
    }

    /// Anzahl der Zuschauer.
    pub fn subscribers(&self) -> usize {
        lock(&self.state).subs.len()
    }

    /// Neue Gegenstelle (z. B. nach einem Reconnect des Runners): Startkredit, und alle
    /// Zuschauer bekommen ein neues Keyframe.
    pub fn attach_backend(&self, backend: Arc<dyn ChannelBackend>) {
        let mut st = lock(&self.state);
        let dropped = st.queued_bytes;
        st.queue.clear();
        st.queued_bytes = 0;
        st.partial.clear();
        st.in_keyframe = false;
        st.blocked_since = None;
        self.buffered.fetch_sub(dropped, Ordering::Relaxed);
        if self.kind == ChannelKind::Terminal {
            for s in st.subs.values_mut() {
                s.sync = Phase::Awaiting;
            }
        }
        backend.credit(RUNNER_WINDOW);
        st.keyframe_requested = false;
        let awaiting = st.subs.values().any(|s| s.sync == Phase::Awaiting);
        if awaiting && self.kind == ChannelKind::Terminal {
            st.keyframe_requested = true;
            backend.keyframe();
        }
        st.backend = Some(backend);
    }

    /// Zuschauer anmelden. Terminals: Er wartet auf ein Keyframe, das hier angefordert wird.
    pub fn subscribe(&self, sink: Arc<dyn ClientSink>, channel_id: u32) -> SubId {
        let mut st = lock(&self.state);
        let id = st.next_sub;
        st.next_sub += 1;
        let sync = if self.kind == ChannelKind::Terminal {
            Phase::Awaiting
        } else {
            Phase::Synced
        };
        st.subs.insert(
            id,
            Sub {
                sink,
                channel_id,
                window: 0,
                outstanding: 0,
                pending: None,
                sync,
            },
        );
        if sync == Phase::Awaiting && !st.keyframe_requested {
            st.keyframe_requested = true;
            if let Some(b) = &st.backend {
                b.keyframe();
            }
        }
        id
    }

    /// Zuschauer abmelden; wartende Frames gehen an die übrigen weiter.
    pub fn unsubscribe(&self, sub: SubId) {
        let mut st = lock(&self.state);
        st.subs.remove(&sub);
        self.pump(&mut st);
    }

    /// Kredit eines Zuschauers (Bytes bzw. bestätigte Frames).
    pub fn credit(&self, sub: SubId, amount: u64) {
        let mut st = lock(&self.state);
        if self.kind.counts_frames() {
            let Some(s) = st.subs.get_mut(&sub) else {
                return;
            };
            s.outstanding = s
                .outstanding
                .saturating_sub(u32::try_from(amount).unwrap_or(u32::MAX));
            if s.outstanding < BROWSER_MAX_UNACKED
                && let Some(frames) = s.pending.take()
            {
                s.outstanding += 1;
                for f in frames {
                    s.sink.frame(Frame::new(s.channel_id, f.flags, f.payload));
                }
            }
            return;
        }
        if let Some(s) = st.subs.get_mut(&sub) {
            s.window = s.window.saturating_add(amount);
        }
        self.pump(&mut st);
    }

    /// Eingabe eines Zuschauers an die Quelle.
    pub fn input(&self, payload: Vec<u8>) -> bool {
        let backend = lock(&self.state).backend.clone();
        backend.is_some_and(|b| b.input(payload))
    }

    /// Ein Frame der Quelle.
    pub fn push(&self, frame: Frame) {
        let mut st = lock(&self.state);
        if st.closed {
            return;
        }
        if self.kind.counts_frames() {
            self.push_image(&mut st, frame);
            return;
        }
        st.queued_bytes += frame.payload.len();
        self.buffered
            .fetch_add(frame.payload.len(), Ordering::Relaxed);
        st.queue.push_back(frame);
        self.pump(&mut st);
    }

    /// Browser: Bild aus Fragmenten zusammensetzen und je Zuschauer höchstens zwei ausstehend.
    fn push_image(&self, st: &mut State, frame: Frame) {
        let fin = frame.has(flags::FIN);
        st.partial.push(frame);
        if !fin {
            return;
        }
        let image = std::mem::take(&mut st.partial);
        for s in st.subs.values_mut() {
            if s.outstanding < BROWSER_MAX_UNACKED {
                s.outstanding += 1;
                for f in &image {
                    s.sink
                        .frame(Frame::new(s.channel_id, f.flags, f.payload.clone()));
                }
            } else {
                // Nur das neueste zählt: ein älteres wartendes Bild entfällt.
                s.pending = Some(image.clone());
            }
        }
    }

    /// Leitet wartende Frames weiter, soweit alle Empfänger Kredit haben.
    fn pump(&self, st: &mut State) {
        while let Some(front) = st.queue.front() {
            let len = front.payload.len() as u64;
            let keyframe = front.has(flags::KEYFRAME);
            let fin = front.has(flags::FIN);
            if keyframe && !st.in_keyframe {
                // Anfang eines Keyframes: Alle Wartenden lesen ab hier.
                for s in st.subs.values_mut() {
                    if s.sync == Phase::Awaiting {
                        s.sync = Phase::Receiving;
                    }
                }
                st.in_keyframe = true;
                st.keyframe_requested = false;
            }
            let wanted = |s: &Sub| {
                if keyframe {
                    s.sync == Phase::Receiving
                } else {
                    s.sync == Phase::Synced
                }
            };
            let blocked = |s: &Sub| {
                wanted(s) && (s.window < len || s.sink.backlog() + len as usize > CLIENT_BACKLOG)
            };
            if st.subs.values().any(blocked) {
                st.blocked_since.get_or_insert_with(Instant::now);
                return;
            }
            st.blocked_since = None;
            let Some(frame) = st.queue.pop_front() else {
                return;
            };
            st.queued_bytes -= frame.payload.len();
            self.buffered
                .fetch_sub(frame.payload.len(), Ordering::Relaxed);
            for s in st.subs.values_mut().filter(|s| wanted(s)) {
                s.window -= len;
                s.sink
                    .frame(Frame::new(s.channel_id, frame.flags, frame.payload.clone()));
            }
            if keyframe && fin {
                st.in_keyframe = false;
                for s in st.subs.values_mut() {
                    if s.sync == Phase::Receiving {
                        s.sync = Phase::Synced;
                    }
                }
                // Wer während des Keyframes kam, braucht ein eigenes.
                if st.subs.values().any(|s| s.sync == Phase::Awaiting) && !st.keyframe_requested {
                    st.keyframe_requested = true;
                    if let Some(b) = &st.backend {
                        b.keyframe();
                    }
                }
            }
            if let Some(b) = &st.backend {
                b.credit(len);
            }
        }
    }

    /// Erneut weiterleiten, z. B. wenn die Warteschlange eines Clients leer gelaufen ist.
    pub fn kick(&self) {
        let mut st = lock(&self.state);
        self.pump(&mut st);
    }

    /// Entfernt Zuschauer, die den Sender länger als `timeout` aufhalten.
    fn evict_stalled(&self, timeout: Duration) {
        let mut st = lock(&self.state);
        self.pump(&mut st);
        let Some(since) = st.blocked_since else {
            return;
        };
        if since.elapsed() < timeout {
            return;
        }
        let Some(front) = st.queue.front() else {
            return;
        };
        let len = front.payload.len() as u64;
        let keyframe = front.has(flags::KEYFRAME);
        let stalled: Vec<SubId> = st
            .subs
            .iter()
            .filter(|(_, s)| {
                (if keyframe {
                    s.sync == Phase::Receiving
                } else {
                    s.sync == Phase::Synced
                }) && (s.window < len || s.sink.backlog() + len as usize > CLIENT_BACKLOG)
            })
            .map(|(id, _)| *id)
            .collect();
        for id in stalled {
            if let Some(s) = st.subs.remove(&id) {
                tracing::info!(session = %self.session, kind = self.kind.as_str(), "Zuschauer ohne Kredit getrennt");
                s.sink.closed(s.channel_id, "stalled");
            }
        }
        st.blocked_since = None;
        self.pump(&mut st);
    }

    /// Quelle beendet: Zuschauer bekommen `channel_closed`.
    fn close(&self, reason: &str) {
        let mut st = lock(&self.state);
        st.closed = true;
        let dropped = st.queued_bytes;
        st.queue.clear();
        st.queued_bytes = 0;
        self.buffered.fetch_sub(dropped, Ordering::Relaxed);
        for (_, s) in st.subs.drain() {
            s.sink.closed(s.channel_id, reason);
        }
        st.backend = None;
    }
}

type Key = (SessionId, ChannelKind, String);

/// Alle Kanäle dieses Knotens.
pub struct ChannelHub {
    relays: Mutex<HashMap<Key, Arc<Relay>>>,
    buffered: Arc<AtomicUsize>,
    /// Bytes in den Ausgangswarteschlangen der Clients (Messung, PROTO-007 AC1).
    in_flight: Arc<AtomicUsize>,
    stall_timeout: Mutex<Duration>,
    next_client_channel: AtomicU64,
}

impl Default for ChannelHub {
    fn default() -> Self {
        Self {
            relays: Mutex::default(),
            buffered: Arc::default(),
            in_flight: Arc::default(),
            stall_timeout: Mutex::new(STALL_TIMEOUT),
            next_client_channel: AtomicU64::new(0),
        }
    }
}

impl std::fmt::Debug for ChannelHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChannelHub")
            .field("relays", &lock(&self.relays).len())
            .finish()
    }
}

impl ChannelHub {
    /// Meldet eine Quelle an (bzw. erneut, z. B. nach einem Reconnect des Runners).
    pub fn register(
        self: &Arc<Self>,
        session: SessionId,
        kind: ChannelKind,
        target: &str,
        backend: Arc<dyn ChannelBackend>,
    ) -> Arc<Relay> {
        let key = (session, kind, target.to_owned());
        let (relay, fresh) = {
            let mut relays = lock(&self.relays);
            match relays.get(&key) {
                Some(r) if !lock(&r.state).closed => (r.clone(), false),
                _ => {
                    let r = Arc::new(Relay {
                        session,
                        kind,
                        target: target.to_owned(),
                        state: Mutex::new(State {
                            backend: None,
                            queue: VecDeque::new(),
                            queued_bytes: 0,
                            subs: HashMap::new(),
                            next_sub: 1,
                            keyframe_requested: false,
                            in_keyframe: false,
                            partial: Vec::new(),
                            blocked_since: None,
                            closed: false,
                        }),
                        buffered: self.buffered.clone(),
                    });
                    relays.insert(key, r.clone());
                    (r, true)
                }
            }
        };
        relay.attach_backend(backend);
        if fresh {
            let weak = Arc::downgrade(&relay);
            let hub = Arc::downgrade(self);
            tokio::spawn(watch_stalls(weak, hub));
        }
        relay
    }

    /// Quelle beendet.
    pub fn close(&self, session: SessionId, kind: ChannelKind, target: &str, reason: &str) {
        let relay = lock(&self.relays).remove(&(session, kind, target.to_owned()));
        if let Some(r) = relay {
            r.close(reason);
        }
    }

    /// Alle Quellen einer Session beenden (Runner getrennt).
    pub fn close_session(&self, session: SessionId, reason: &str) {
        let gone: Vec<Arc<Relay>> = {
            let mut relays = lock(&self.relays);
            let keys: Vec<Key> = relays
                .keys()
                .filter(|(s, _, _)| *s == session)
                .cloned()
                .collect();
            keys.into_iter().filter_map(|k| relays.remove(&k)).collect()
        };
        for r in gone {
            r.close(reason);
        }
    }

    pub fn get(&self, session: SessionId, kind: ChannelKind, target: &str) -> Option<Arc<Relay>> {
        lock(&self.relays)
            .get(&(session, kind, target.to_owned()))
            .cloned()
    }

    /// Nächste Kanal-ID für einen Client (je Knoten eindeutig).
    pub fn next_client_channel(&self) -> u32 {
        let n = self.next_client_channel.fetch_add(1, Ordering::Relaxed) + 1;
        u32::try_from(n % u64::from(u32::MAX)).unwrap_or(1).max(1)
    }

    /// Bytes, die der Server gerade für Kanäle hält: wartend in Relays und in den
    /// Ausgangswarteschlangen der Clients (PROTO-007 AC1).
    pub fn buffered_bytes(&self) -> usize {
        self.buffered.load(Ordering::Relaxed) + self.in_flight.load(Ordering::Relaxed)
    }

    /// Zähler für Bytes in Ausgangswarteschlangen (die WS-Verbindung zählt hoch und runter).
    pub fn in_flight(&self) -> Arc<AtomicUsize> {
        self.in_flight.clone()
    }

    /// Wartezeit, bis ein Zuschauer ohne Kredit den Kanal verliert (Tests).
    pub fn set_stall_timeout(&self, timeout: Duration) {
        *lock(&self.stall_timeout) = timeout;
    }

    fn stall_timeout(&self) -> Duration {
        *lock(&self.stall_timeout)
    }

    /// Offene Terminal-Kanäle einer Session (Runner gilt dann nicht als im Leerlauf).
    pub fn open_terminals(&self, session: SessionId) -> usize {
        lock(&self.relays)
            .keys()
            .filter(|(s, k, _)| *s == session && *k == ChannelKind::Terminal)
            .count()
    }
}

async fn watch_stalls(relay: Weak<Relay>, hub: Weak<ChannelHub>) {
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    loop {
        tick.tick().await;
        let (Some(r), Some(h)) = (relay.upgrade(), hub.upgrade()) else {
            return;
        };
        if lock(&r.state).closed {
            return;
        }
        r.evict_stalled(h.stall_timeout());
    }
}

/// Darf ein Client mit `access` Eingaben senden?
pub fn may_input(access: ChannelAccess, kind: ChannelKind) -> bool {
    access == ChannelAccess::Drive && kind != ChannelKind::Browser
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Backend {
        credit: AtomicU64,
        keyframes: AtomicUsize,
        input: Mutex<Vec<u8>>,
    }

    impl ChannelBackend for Backend {
        fn credit(&self, bytes: u64) {
            self.credit.fetch_add(bytes, Ordering::Relaxed);
        }
        fn keyframe(&self) {
            self.keyframes.fetch_add(1, Ordering::Relaxed);
        }
        fn input(&self, payload: Vec<u8>) -> bool {
            lock(&self.input).extend(payload);
            true
        }
    }

    #[derive(Default)]
    struct Client {
        frames: Mutex<Vec<Frame>>,
        closed: Mutex<Vec<String>>,
    }

    impl ClientSink for Client {
        fn frame(&self, frame: Frame) -> bool {
            lock(&self.frames).push(frame);
            true
        }
        fn closed(&self, _channel_id: u32, reason: &str) {
            lock(&self.closed).push(reason.into());
        }
    }

    fn data(n: usize) -> Frame {
        Frame::new(1, flags::FIN, vec![b'x'; n])
    }

    fn keyframe(text: &str) -> Frame {
        Frame::new(1, flags::KEYFRAME | flags::FIN, text.as_bytes().to_vec())
    }

    fn setup(kind: ChannelKind) -> (Arc<ChannelHub>, Arc<Relay>, Arc<Backend>) {
        let hub = Arc::new(ChannelHub::default());
        let backend = Arc::new(Backend::default());
        let relay = hub.register(SessionId::new(), kind, "term_1", backend.clone());
        (hub, relay, backend)
    }

    #[tokio::test]
    async fn proto_007_without_viewers_credit_returns_immediately() {
        let (_hub, relay, backend) = setup(ChannelKind::Terminal);
        assert_eq!(backend.credit.load(Ordering::Relaxed), RUNNER_WINDOW);
        relay.push(data(1000));
        assert_eq!(backend.credit.load(Ordering::Relaxed), RUNNER_WINDOW + 1000);
        assert_eq!(relay.queued_bytes(), 0);
    }

    #[tokio::test]
    async fn proto_007_ac1_slow_viewer_holds_back_frames_and_runner_credit() {
        let (hub, relay, backend) = setup(ChannelKind::Terminal);
        let client = Arc::new(Client::default());
        let sub = relay.subscribe(client.clone(), 7);
        assert_eq!(backend.keyframes.load(Ordering::Relaxed), 1);
        relay.credit(sub, 100);
        // Vor dem Keyframe ist der Zuschauer nicht synchron: laufende Frames gehen an ihm
        // vorbei, ohne den Sender aufzuhalten.
        relay.push(data(50));
        assert!(lock(&client.frames).is_empty());
        relay.push(keyframe("K"));
        relay.push(data(60));
        relay.push(data(60));
        let got = lock(&client.frames).clone();
        assert_eq!(got.len(), 2, "Keyframe und ein Frame im Kredit von 100");
        assert!(got[0].has(flags::KEYFRAME));
        assert_eq!(got[0].channel_id, 7, "Kanal-ID des Clients");
        // Das dritte wartet, der Runner bekommt dafür keinen Kredit zurück.
        assert_eq!(relay.queued_bytes(), 60);
        assert_eq!(hub.buffered_bytes(), 60);
        let before = backend.credit.load(Ordering::Relaxed);
        relay.credit(sub, 1000);
        assert_eq!(lock(&client.frames).len(), 3);
        assert_eq!(backend.credit.load(Ordering::Relaxed), before + 60);
        assert_eq!(hub.buffered_bytes(), 0);
    }

    #[tokio::test]
    async fn proto_007_viewers_joining_during_a_keyframe_get_their_own() {
        let (_hub, relay, backend) = setup(ChannelKind::Terminal);
        let a = Arc::new(Client::default());
        let sa = relay.subscribe(a.clone(), 1);
        relay.credit(sa, 10_000);
        relay.push(Frame::new(1, flags::KEYFRAME, b"K1a".to_vec()));
        let b = Arc::new(Client::default());
        let sb = relay.subscribe(b.clone(), 2);
        relay.credit(sb, 10_000);
        relay.push(Frame::new(1, flags::KEYFRAME | flags::FIN, b"K1b".to_vec()));
        assert_eq!(lock(&a.frames).len(), 2);
        assert!(lock(&b.frames).is_empty(), "kein halbes Keyframe");
        assert_eq!(backend.keyframes.load(Ordering::Relaxed), 2);
        relay.push(keyframe("K2"));
        relay.push(data(5));
        assert_eq!(lock(&b.frames).len(), 2);
        assert_eq!(lock(&a.frames).len(), 3, "A bekommt K2 nicht noch einmal");
    }

    #[tokio::test]
    async fn proto_007_stalled_viewer_loses_the_channel() {
        let (_hub, relay, _backend) = setup(ChannelKind::Terminal);
        let slow = Arc::new(Client::default());
        let s = relay.subscribe(slow.clone(), 1);
        relay.credit(s, 10);
        relay.push(keyframe("K"));
        relay.push(data(100));
        assert_eq!(relay.queued_bytes(), 100);
        relay.evict_stalled(Duration::ZERO);
        assert_eq!(lock(&slow.closed).as_slice(), ["stalled"]);
        assert_eq!(relay.queued_bytes(), 0, "ohne Zuschauer weitergeleitet");
    }

    #[tokio::test]
    async fn proto_007_ac3_at_most_two_unacked_browser_frames_newest_wins() {
        let (_hub, relay, _backend) = setup(ChannelKind::Browser);
        let c = Arc::new(Client::default());
        let s = relay.subscribe(c.clone(), 3);
        for i in 0..10u8 {
            relay.push(Frame::new(1, flags::KEYFRAME | flags::FIN, vec![i]));
        }
        let got: Vec<u8> = lock(&c.frames).iter().map(|f| f.payload[0]).collect();
        assert_eq!(got, [0, 1]);
        relay.credit(s, 1);
        let got: Vec<u8> = lock(&c.frames).iter().map(|f| f.payload[0]).collect();
        assert_eq!(got, [0, 1, 9], "nur das neueste wartende");
    }

    #[tokio::test]
    async fn proto_007_closing_a_source_closes_its_viewers() {
        let (hub, relay, _backend) = setup(ChannelKind::Terminal);
        let c = Arc::new(Client::default());
        relay.subscribe(c.clone(), 1);
        hub.close(relay.session, ChannelKind::Terminal, "term_1", "closed");
        assert_eq!(lock(&c.closed).as_slice(), ["closed"]);
        assert!(
            hub.get(relay.session, ChannelKind::Terminal, "term_1")
                .is_none()
        );
    }

    #[test]
    fn proto_007_ac2_only_drive_sends_input() {
        assert!(may_input(ChannelAccess::Drive, ChannelKind::Terminal));
        assert!(!may_input(ChannelAccess::Read, ChannelKind::Terminal));
        assert!(!may_input(ChannelAccess::Drive, ChannelKind::Browser));
    }
}
