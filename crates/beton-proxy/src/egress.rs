//! Events und Zusammenfassung des Egress (PRX-009).
//!
//! - Jede Ablehnung einer Instanz geht als `egress.blocked` an ihre [`EgressEvents`] (Methode,
//!   Host, Pfad ohne Query, Grund, Regel-Hinweis), nie mit Header-Werten, Cookies oder
//!   Query-Werten.
//! - Erlaubter Verkehr wird je Session in [`Traffic`] gezählt (Host → Requests, Bytes);
//!   [`spawn_summaries`] meldet ihn höchstens einmal je Periode als `egress.summary` und nur,
//!   wenn es Verkehr gab.

use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use beton_core::event::{EgressBlocked, EgressHost, EgressSummary};
use bytes::Bytes;
use hyper::body::{Body, Frame, SizeHint};

/// Empfänger der Egress-Events einer Instanz (im Runner: das Session-Log).
pub trait EgressEvents: Send + Sync {
    fn blocked(&self, event: EgressBlocked);

    /// Zusammenfassung des erlaubten Verkehrs (über [`spawn_summaries`]).
    fn summary(&self, _summary: EgressSummary) {}
}

/// Erlaubter Verkehr einer Session je Host; mehrere Instanzen können sich einen Zähler teilen.
#[derive(Clone, Default)]
pub struct Traffic(Arc<Mutex<BTreeMap<String, (u64, u64)>>>);

impl std::fmt::Debug for Traffic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Traffic")
    }
}

impl Traffic {
    pub(crate) fn record(&self, host: &str, requests: u64, bytes: u64) {
        if let Ok(mut m) = self.0.lock() {
            let e = m.entry(host.to_owned()).or_default();
            e.0 += requests;
            e.1 += bytes;
        }
    }

    /// Bisheriger Verkehr; danach ist der Zähler leer.
    pub fn take(&self) -> Vec<EgressHost> {
        let Ok(mut m) = self.0.lock() else {
            return Vec::new();
        };
        std::mem::take(&mut *m)
            .into_iter()
            .map(|(host, (requests, bytes))| EgressHost {
                host,
                requests,
                bytes,
            })
            .collect()
    }
}

/// Meldet den Verkehr je `period` als `egress.summary` (nur bei Verkehr, PRX-009 AC3).
pub fn spawn_summaries<F>(
    traffic: Traffic,
    period: Duration,
    emit: F,
) -> tokio::task::JoinHandle<()>
where
    F: Fn(EgressSummary) + Send + 'static,
{
    tokio::spawn(async move {
        let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let hosts = traffic.take();
            if !hosts.is_empty() {
                emit(EgressSummary {
                    window_s: u32::try_from(period.as_secs()).unwrap_or(u32::MAX),
                    hosts,
                });
            }
        }
    })
}

/// Zählt die Bytes eines Bodys und meldet sie am Ende (bzw. beim Fallenlassen) an [`Traffic`].
pub(crate) struct Counted<B> {
    inner: B,
    report: Option<(Traffic, String, u64)>,
    bytes: u64,
}

impl<B> Counted<B> {
    /// `requests`: wie viele Requests dieser Body zählt (1 für die Antwort, 0 für den Request).
    pub(crate) fn new(inner: B, traffic: Option<Traffic>, host: &str, requests: u64) -> Self {
        Self {
            inner,
            report: traffic.map(|t| (t, host.to_owned(), requests)),
            bytes: 0,
        }
    }

    /// Bytes, die außerhalb von `poll_frame` gelesen wurden (gepufferte Antworten).
    pub(crate) fn add(&mut self, bytes: usize) {
        self.bytes += bytes as u64;
    }

    fn finish(&mut self) {
        if let Some((t, host, requests)) = self.report.take() {
            t.record(&host, requests, self.bytes);
        }
    }
}

impl Counted<()> {
    /// Übernimmt Zähler und bisherige Bytes für einen Body.
    pub(crate) fn around<C>(mut self, inner: C) -> Counted<C> {
        Counted {
            inner,
            report: self.report.take(),
            bytes: self.bytes,
        }
    }
}

impl<B> Drop for Counted<B> {
    fn drop(&mut self) {
        self.finish();
    }
}

impl<B> Body for Counted<B>
where
    B: Body<Data = Bytes> + Unpin,
{
    type Data = Bytes;
    type Error = B::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        let this = self.get_mut();
        let polled = Pin::new(&mut this.inner).poll_frame(cx);
        match &polled {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(d) = frame.data_ref() {
                    this.bytes += d.len() as u64;
                }
            }
            Poll::Ready(None) => this.finish(),
            _ => {}
        }
        polled
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn prx_009_ac3_summary_at_most_once_per_minute_and_only_with_traffic() {
        let traffic = Traffic::default();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let task = spawn_summaries(traffic.clone(), Duration::from_secs(60), move |s| {
            let _ = tx.send((tokio::time::Instant::now(), s));
        });
        let start = tokio::time::Instant::now();
        // Ohne Verkehr keine Zusammenfassung.
        tokio::time::sleep(Duration::from_secs(150)).await;
        assert!(rx.try_recv().is_err());
        // Mehrere Requests innerhalb einer Minute ergeben eine Zusammenfassung.
        traffic.record("api.test", 1, 10);
        traffic.record("api.test", 1, 5);
        traffic.record("b.test", 1, 1);
        tokio::time::sleep(Duration::from_secs(40)).await;
        let (at, s) = rx.try_recv().unwrap();
        assert_eq!(at - start, Duration::from_secs(180));
        assert_eq!(s.window_s, 60);
        assert_eq!(
            s.hosts,
            vec![
                EgressHost {
                    host: "api.test".into(),
                    requests: 2,
                    bytes: 15
                },
                EgressHost {
                    host: "b.test".into(),
                    requests: 1,
                    bytes: 1
                }
            ]
        );
        assert!(rx.try_recv().is_err(), "höchstens eine je Minute");
        // Weiterer Verkehr erscheint frühestens eine Minute später.
        traffic.record("api.test", 1, 1);
        tokio::time::sleep(Duration::from_secs(30)).await;
        assert!(rx.try_recv().is_err(), "höchstens eine je Minute");
        tokio::time::sleep(Duration::from_secs(30)).await;
        let (second, s) = rx.try_recv().unwrap();
        assert_eq!(second - at, Duration::from_secs(60));
        assert_eq!(s.hosts.len(), 1);
        assert_eq!(s.hosts[0].requests, 1);
        tokio::time::sleep(Duration::from_secs(300)).await;
        assert!(rx.try_recv().is_err(), "nur bei Verkehr");
        task.abort();
    }
}
