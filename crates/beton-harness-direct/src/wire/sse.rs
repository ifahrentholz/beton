//! Inkrementeller Parser für Server-Sent Events (`text/event-stream`).

/// Ein SSE-Ereignis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

/// Höchstgröße eines einzelnen Ereignisses; darüber gilt der Strom als kaputt.
pub const MAX_EVENT_BYTES: usize = 8 * 1024 * 1024;

/// Sammelt Bytes und liefert vollständige Ereignisse.
#[derive(Debug, Default)]
pub struct SseParser {
    buf: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("SSE-Ereignis größer als {MAX_EVENT_BYTES} Bytes")]
pub struct TooLarge;

impl SseParser {
    /// Nimmt ein Stück des Stroms und liefert alle darin abgeschlossenen Ereignisse.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<SseEvent>, TooLarge> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(pos) = self.buf.iter().position(|b| *b == b'\n') {
            let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8_lossy(&line).into_owned();
            if line.is_empty() {
                if !self.data.is_empty() || self.event.is_some() {
                    out.push(SseEvent {
                        event: self.event.take(),
                        data: std::mem::take(&mut self.data).join("\n"),
                    });
                }
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            let (field, value) = line.split_once(':').unwrap_or((&line, ""));
            let value = value.strip_prefix(' ').unwrap_or(value);
            match field {
                "event" => self.event = Some(value.to_owned()),
                "data" => self.data.push(value.to_owned()),
                _ => {}
            }
        }
        let pending: usize = self.buf.len() + self.data.iter().map(String::len).sum::<usize>();
        if pending > MAX_EVENT_BYTES {
            return Err(TooLarge);
        }
        Ok(out)
    }

    /// Ende des Stroms: ein letztes Ereignis ohne Leerzeile abschließen.
    pub fn finish(&mut self) -> Option<SseEvent> {
        if !self.buf.is_empty() {
            let _ = self.push(b"\n");
        }
        (!self.data.is_empty()).then(|| SseEvent {
            event: self.event.take(),
            data: std::mem::take(&mut self.data).join("\n"),
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn parses_split_events() {
        let mut p = SseParser::default();
        assert!(
            p.push(b"event: message_start\r\ndata: {\"a\"")
                .unwrap()
                .is_empty()
        );
        let out = p
            .push(b":1}\r\n\r\n: ping\n\ndata: x\ndata: y\n\n")
            .unwrap();
        assert_eq!(
            out,
            vec![
                SseEvent {
                    event: Some("message_start".into()),
                    data: "{\"a\":1}".into()
                },
                SseEvent {
                    event: None,
                    data: "x\ny".into()
                }
            ]
        );
        p.push(b"data: [DONE]").unwrap();
        assert_eq!(p.finish().unwrap().data, "[DONE]");
    }
}
