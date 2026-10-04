//! Read-only SSE-Strom einer Session für Skripte (PROTO-012, API-003).
//!
//! `GET /v1/sessions/{id}/events/stream` liefert `text/event-stream`: je dauerhaftem Event
//! `id: <seq>`, `event: <type>`, `data: <Envelope>`. Fortsetzen über `Last-Event-ID` (gewinnt,
//! wie beim Reconnect eines `EventSource`) oder `?from_seq=`; ohne beides ab `seq 1`.
//! Transiente Events nur mit `?transient=true` und ohne `id:`-Zeile. Ohne Ereignisse kommt alle
//! 15 s der Kommentar `: hb`.
//!
//! Über den Strom ist keine Aktion möglich; die Route kennt nur `GET`. Authentisierung und
//! Origin-Prüfung übernimmt die Sicherheitsschicht wie beim WebSocket ([`crate::security`]).
//! Wie der WebSocket-Weiterleiter abonniert der Strom erst den Hub und liefert dann aus dem
//! Store nach; Lücken und verpasste Broadcasts schließt er aus dem Store.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Extension;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use beton_core::event::Event;
use beton_core::id::SessionId;
use serde::Deserialize;
use tokio::sync::{broadcast, mpsc};
use utoipa::IntoParams;

use crate::app::AppState;
use crate::extract::ApiQuery;
use crate::problem::{ApiResult, Problem, ProblemCode};
use crate::security::Authenticated;

/// Einstellungen des SSE-Stroms.
#[derive(Debug, Clone, Copy)]
pub struct SseConfig {
    /// Abstand der Heartbeat-Kommentare ohne Ereignisse (PROTO-012 AC3: 15 s).
    pub heartbeat: Duration,
    /// Events je Store-Abfrage beim Nachliefern.
    pub page: u32,
    /// Gepufferte Frames je Verbindung; ist der Puffer voll, wartet das Nachliefern.
    pub buffer: usize,
}

impl Default for SseConfig {
    fn default() -> Self {
        Self {
            heartbeat: Duration::from_secs(15),
            page: 500,
            buffer: 256,
        }
    }
}

/// Text des Heartbeat-Kommentars (`: hb`).
pub const HEARTBEAT: &str = "hb";

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct StreamQuery {
    /// Events ab `seq > from_seq` (Default 0: alles). `Last-Event-ID` hat Vorrang.
    pub from_seq: Option<String>,
    /// `true`: auch transiente Events (Deltas), ohne `id:`-Zeile.
    pub transient: Option<String>,
}

fn invalid(detail: &str) -> Problem {
    Problem::new(ProblemCode::ValidationFailed).detail(detail.to_owned())
}

fn parse_seq(raw: &str, what: &str) -> Result<u64, Problem> {
    raw.trim()
        .parse()
        .map_err(|_| invalid(&format!("{what} muss eine nicht-negative Zahl sein")))
}

/// Startpunkt: `Last-Event-ID` vor `from_seq`, sonst 0.
fn start_seq(headers: &HeaderMap, q: &StreamQuery) -> Result<u64, Problem> {
    if let Some(v) = headers.get("last-event-id") {
        let text = v
            .to_str()
            .map_err(|_| invalid("Last-Event-ID ist ungültig"))?;
        return parse_seq(text, "Last-Event-ID");
    }
    match q.from_seq.as_deref().filter(|s| !s.is_empty()) {
        Some(s) => parse_seq(s, "from_seq"),
        None => Ok(0),
    }
}

fn flag(raw: Option<&str>) -> Result<bool, Problem> {
    match raw {
        None | Some("" | "false" | "0") => Ok(false),
        Some("true" | "1") => Ok(true),
        Some(_) => Err(invalid("transient muss true oder false sein")),
    }
}

/// Read-only Event-Strom einer Session als Server-Sent Events (PROTO-012, API-003).
#[utoipa::path(get, path = "/v1/sessions/{id}/events/stream", tag = "events",
    params(("id" = String, Path),
           ("Last-Event-ID" = Option<String>, Header, description = "Letzte empfangene `seq`; der Strom setzt danach fort und hat Vorrang vor `from_seq`"),
           StreamQuery),
    responses((status = 200, description = "`text/event-stream`: je Event `id: <seq>`, `event: <type>`, `data: <Envelope>`; Heartbeat `: hb` alle 15 s", content_type = "text/event-stream", body = String),
              (status = 404, description = "Session unbekannt", body = Problem, content_type = "application/problem+json"),
              (status = 409, description = "`seq_ahead`: Startpunkt hinter `head_seq`", body = Problem, content_type = "application/problem+json")))]
pub async fn stream_events(
    State(state): State<AppState>,
    Extension(auth): Extension<Authenticated>,
    Path(id): Path<String>,
    headers: HeaderMap,
    ApiQuery(q): ApiQuery<StreamQuery>,
) -> ApiResult<Response> {
    let session: SessionId = id
        .parse()
        .map_err(|_| Problem::new(ProblemCode::NotFound).detail(format!("Session {id}")))?;
    let from = start_seq(&headers, &q)?;
    let transient = flag(q.transient.as_deref())?;
    let record = state.store.session(state.local.org, session).await?;
    if !state.runtime.authorizer.can_read(auth, &record) {
        return Err(Problem::new(ProblemCode::Forbidden));
    }
    if from > record.head_seq {
        return Err(Problem::new(ProblemCode::SeqAhead).detail(format!(
            "Startpunkt {from} > head_seq {}; neu ab 0 verbinden",
            record.head_seq
        )));
    }
    let cfg = state.runtime.sse;
    // Erst abonnieren, dann nachliefern: so geht zwischen Replay und Live nichts verloren.
    let (rx, _) = state.runtime.hub.subscribe(session);
    let (tx, out) = mpsc::channel::<SseEvent>(cfg.buffer);
    let pump = Pump {
        state: state.clone(),
        session,
        transient,
        last: from,
        tx,
        cfg,
    };
    tokio::spawn(pump.run(rx));
    let stream = futures_util::stream::unfold(out, |mut out| async move {
        out.recv().await.map(|e| (Ok::<_, Infallible>(e), out))
    });
    let mut res = Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(cfg.heartbeat).text(HEARTBEAT))
        .into_response();
    // Proxies sollen den Strom nicht puffern.
    res.headers_mut().insert(
        "x-accel-buffering",
        axum::http::HeaderValue::from_static("no"),
    );
    Ok(res)
}

/// Liefert Events einer Session an eine SSE-Verbindung.
struct Pump {
    state: AppState,
    session: SessionId,
    transient: bool,
    /// Höchste gesendete dauerhafte `seq`.
    last: u64,
    tx: mpsc::Sender<SseEvent>,
    cfg: SseConfig,
}

/// Verbindung beendet (Client weg oder Server fährt herunter).
struct Closed;

fn frame(e: &Event) -> SseEvent {
    let data = serde_json::to_string(e).unwrap_or_else(|_| "{}".into());
    let f = SseEvent::default().event(e.type_name()).data(data);
    if e.transient {
        f
    } else {
        f.id(e.seq.to_string())
    }
}

impl Pump {
    async fn send(&self, e: &Event) -> Result<(), Closed> {
        self.tx.send(frame(e)).await.map_err(|_| Closed)
    }

    /// Dauerhafte Events aus dem Store bis `head`.
    async fn catch_up(&mut self, head: u64) -> Result<(), Closed> {
        let org = self.state.local.org;
        while self.last < head {
            let page = self
                .state
                .store
                .events(org, self.session, self.last, self.cfg.page)
                .await
                .map_err(|_| Closed)?;
            let before = self.last;
            for e in page {
                if e.seq > head {
                    break;
                }
                self.send(&e).await?;
                self.last = e.seq;
            }
            if self.last == before {
                break;
            }
        }
        Ok(())
    }

    async fn head(&self) -> Result<u64, Closed> {
        self.state
            .store
            .session(self.state.local.org, self.session)
            .await
            .map(|s| s.head_seq)
            .map_err(|_| Closed)
    }

    async fn run(mut self, mut rx: broadcast::Receiver<Arc<Event>>) {
        let _ = self.serve(&mut rx).await;
    }

    async fn serve(&mut self, rx: &mut broadcast::Receiver<Arc<Event>>) -> Result<(), Closed> {
        let head = self.head().await?;
        self.catch_up(head).await?;
        let mut shutdown = self.state.runtime.shutdown.clone();
        loop {
            let next = tokio::select! {
                r = rx.recv() => r,
                () = self.tx.closed() => return Err(Closed),
                _ = shutdown.wait_for(|s| *s) => return Err(Closed),
            };
            match next {
                Ok(e) if e.transient => {
                    if self.transient {
                        self.send(&e).await?;
                    }
                }
                Ok(e) => {
                    if e.seq <= self.last {
                        continue;
                    }
                    if e.seq > self.last + 1 {
                        // Lücke (paralleles Anhängen): aus dem Store schließen.
                        self.catch_up(e.seq - 1).await?;
                    }
                    self.send(&e).await?;
                    self.last = e.seq;
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // Broadcast verpasst: dauerhafte Events aus dem Store nachholen.
                    let head = self.head().await?;
                    self.catch_up(head).await?;
                }
                Err(broadcast::error::RecvError::Closed) => return Err(Closed),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proto_012_last_event_id_wins_over_from_seq() {
        let mut h = HeaderMap::new();
        let q = StreamQuery {
            from_seq: Some("0".into()),
            transient: None,
        };
        assert_eq!(start_seq(&h, &q).unwrap(), 0);
        h.insert("last-event-id", "42".parse().unwrap());
        assert_eq!(start_seq(&h, &q).unwrap(), 42);
        assert_eq!(
            start_seq(&HeaderMap::new(), &StreamQuery::default()).unwrap(),
            0
        );
    }

    #[test]
    fn proto_012_ac3_default_heartbeat_is_15_s() {
        assert_eq!(SseConfig::default().heartbeat, Duration::from_secs(15));
    }

    #[test]
    fn proto_012_transient_frames_carry_no_id() {
        let mut e = Event::new(
            SessionId::new(),
            7,
            beton_core::event::Actor::default(),
            beton_core::event::EventPayload::Notice(beton_core::event::Notice::default()),
        );
        let durable = format!("{:?}", frame(&e));
        assert!(durable.contains("id: 7"), "{durable}");
        e.transient = true;
        let transient = format!("{:?}", frame(&e));
        assert!(!transient.contains("id: 7"), "{transient}");
    }
}
