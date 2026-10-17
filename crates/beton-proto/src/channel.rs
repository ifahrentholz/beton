//! Binärkanäle im selben WebSocket (PROTO-007): Terminal-Bytes, Browser-Frames, Audio.
//!
//! Ein Binärframe ist `[ver u8][channel_id u32 BE][flags u8][payload]`. Kanäle öffnet der
//! Client mit `open_channel` (Autorisierung je Art), beendet werden sie mit `close_channel`
//! bzw. `channel_closed`. Die Flusskontrolle ist kreditbasiert: Der Empfänger gewährt
//! `credit {channel_id, bytes|frames}`, der Sender überschreitet das Fenster nie. Dasselbe
//! Framing gilt im Tunnel zwischen Runner und Server (dort mit den Kanal-IDs des Runners).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Version des Binärframings.
pub const FRAME_VERSION: u8 = 1;
/// Länge des Kopfes: Version, Kanal-ID, Flags.
pub const HEADER_LEN: usize = 6;
/// Größte Nutzlast eines Frames; größere Nachrichten (Keyframes, Bilder) gehen in Fragmenten,
/// das letzte trägt `FIN`.
pub const MAX_PAYLOAD: usize = 64 * 1024;
/// Startfenster je Richtung in Bytes (Terminal, Audio), siehe `channel_opened.window`.
pub const DEFAULT_WINDOW: u64 = 256 * 1024;
/// Browser-Frames: höchstens so viele unbestätigt; ältere verwirft der Sender (PROTO-007).
pub const BROWSER_MAX_UNACKED: u32 = 2;

/// Flags eines Binärframes.
pub mod flags {
    /// Letztes Fragment einer Nachricht (Terminal-Ausgabe: jedes Frame).
    pub const FIN: u8 = 0x01;
    /// Vollständiger Zustand (Terminal: Bildschirm samt Scrollback), ab dem ein neuer
    /// Empfänger weiterliest; Browser: Bild ohne Bezug auf frühere.
    pub const KEYFRAME: u8 = 0x02;
    /// Eingabe vom Client (Terminal-Tastatur, Audio des Mikrofons).
    pub const INPUT: u8 = 0x04;
    pub const ALL: u8 = FIN | KEYFRAME | INPUT;
}

/// Art eines Kanals.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS,
)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    /// PTY eines Workspace-Terminals (SES-019); `ref` ist die `terminal_id`.
    Terminal,
    /// Screencast des eingebetteten Browsers (BRW-005); `ref` ist die `browser_id`.
    Browser,
    /// Audio des Mikrofons zur Spracheingabe (VOI-004), Client → Server.
    Audio,
}

impl ChannelKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::Browser => "browser",
            Self::Audio => "audio",
        }
    }

    /// Fluss in Frames statt Bytes (Browser).
    pub fn counts_frames(self) -> bool {
        self == Self::Browser
    }
}

/// Was ein Client auf einem Kanal darf. Lokal hat der Owner immer `drive`; Freigaben mit
/// Rollen (`view`, `comment_approve`, COL-001) bilden ab M4 auf `read` ab.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS,
)]
#[serde(rename_all = "snake_case")]
pub enum ChannelAccess {
    /// Nur lesen: Frames mit `INPUT` werden verworfen und mit `nack 403` beantwortet.
    Read,
    /// Lesen und Eingaben senden.
    Drive,
}

/// Ein Binärframe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub channel_id: u32,
    pub flags: u8,
    pub payload: Vec<u8>,
}

/// Fehler beim Lesen eines Binärframes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    #[error("Binärframe kürzer als der Kopf ({0} Bytes)")]
    TooShort(usize),
    #[error("unbekannte Version des Binärframings: {0}")]
    Version(u8),
    #[error("unbekannte Flags im Binärframe: {0:#04x}")]
    Flags(u8),
    #[error("Nutzlast größer als {MAX_PAYLOAD} Bytes: {0}")]
    TooLarge(usize),
}

impl Frame {
    pub fn new(channel_id: u32, flags: u8, payload: Vec<u8>) -> Self {
        Self {
            channel_id,
            flags,
            payload,
        }
    }

    pub fn has(&self, flag: u8) -> bool {
        self.flags & flag == flag
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + self.payload.len());
        out.push(FRAME_VERSION);
        out.extend_from_slice(&self.channel_id.to_be_bytes());
        out.push(self.flags);
        out.extend_from_slice(&self.payload);
        out
    }

    /// Liest ein Frame; unbekannte Versionen und Flags sind Fehler (fail closed).
    pub fn decode(bytes: &[u8]) -> Result<Self, FrameError> {
        if bytes.len() < HEADER_LEN {
            return Err(FrameError::TooShort(bytes.len()));
        }
        if bytes[0] != FRAME_VERSION {
            return Err(FrameError::Version(bytes[0]));
        }
        let flags = bytes[5];
        if flags & !flags::ALL != 0 {
            return Err(FrameError::Flags(flags));
        }
        let payload = &bytes[HEADER_LEN..];
        if payload.len() > MAX_PAYLOAD {
            return Err(FrameError::TooLarge(payload.len()));
        }
        Ok(Self {
            channel_id: u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]),
            flags,
            payload: payload.to_vec(),
        })
    }

    /// Zerlegt eine Nachricht in Frames von höchstens [`MAX_PAYLOAD`] Bytes; das letzte trägt
    /// `FIN`, alle tragen `extra` (z. B. `KEYFRAME`). Eine leere Nachricht ist ein leeres Frame.
    pub fn fragments(channel_id: u32, extra: u8, message: &[u8]) -> Vec<Frame> {
        if message.is_empty() {
            return vec![Frame::new(channel_id, extra | flags::FIN, Vec::new())];
        }
        let chunks: Vec<&[u8]> = message.chunks(MAX_PAYLOAD).collect();
        let last = chunks.len() - 1;
        chunks
            .into_iter()
            .enumerate()
            .map(|(i, c)| {
                let fin = if i == last { flags::FIN } else { 0 };
                Frame::new(channel_id, extra | fin, c.to_vec())
            })
            .collect()
    }
}

/// Bildformat eines Browser-Frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
    Jpeg,
    Webp,
}

/// Metadaten eines Browser-Frames: `u16 meta_len ‖ JSON {w, h, ts, format} ‖ Bilddaten`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct BrowserFrameMeta {
    pub w: u32,
    pub h: u32,
    /// Zeitpunkt der Aufnahme (Millisekunden seit Unix-Epoche).
    pub ts: u64,
    pub format: ImageFormat,
}

impl BrowserFrameMeta {
    /// Nutzlast eines Browser-Frames.
    pub fn payload(&self, image: &[u8]) -> Vec<u8> {
        let meta = serde_json::to_vec(self).unwrap_or_default();
        let len = u16::try_from(meta.len()).unwrap_or(u16::MAX);
        let mut out = Vec::with_capacity(2 + meta.len() + image.len());
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&meta[..usize::from(len)]);
        out.extend_from_slice(image);
        out
    }

    /// Liest Metadaten und Bild aus einer Nutzlast.
    pub fn parse(payload: &[u8]) -> Option<(Self, &[u8])> {
        let len = usize::from(u16::from_be_bytes([*payload.first()?, *payload.get(1)?]));
        let meta = payload.get(2..2 + len)?;
        Some((serde_json::from_slice(meta).ok()?, payload.get(2 + len..)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn proto_007_frame_layout_is_version_channel_flags_payload() {
        let f = Frame::new(5, flags::INPUT, b"ls\r".to_vec());
        let bytes = f.encode();
        assert_eq!(bytes, [1, 0, 0, 0, 5, 0x04, b'l', b's', b'\r']);
        assert_eq!(Frame::decode(&bytes).unwrap(), f);
        // Big Endian für die Kanal-ID.
        let f = Frame::new(0x0102_0304, flags::FIN, Vec::new());
        assert_eq!(&f.encode()[1..5], &[1, 2, 3, 4]);
    }

    #[test]
    fn proto_007_unknown_versions_flags_and_short_frames_are_rejected() {
        assert_eq!(Frame::decode(&[1, 0, 0]), Err(FrameError::TooShort(3)));
        assert_eq!(
            Frame::decode(&[2, 0, 0, 0, 1, 0]),
            Err(FrameError::Version(2))
        );
        assert_eq!(
            Frame::decode(&[1, 0, 0, 0, 1, 0x80]),
            Err(FrameError::Flags(0x80))
        );
        let mut big = Frame::new(1, 0, vec![0; MAX_PAYLOAD]).encode();
        assert!(Frame::decode(&big).is_ok());
        big.push(0);
        assert_eq!(
            Frame::decode(&big),
            Err(FrameError::TooLarge(MAX_PAYLOAD + 1))
        );
    }

    #[test]
    fn proto_007_messages_split_into_fragments_with_fin_on_the_last() {
        let msg = vec![7u8; MAX_PAYLOAD * 2 + 10];
        let parts = Frame::fragments(3, flags::KEYFRAME, &msg);
        assert_eq!(parts.len(), 3);
        assert!(parts.iter().all(|p| p.has(flags::KEYFRAME)));
        assert!(!parts[0].has(flags::FIN) && !parts[1].has(flags::FIN));
        assert!(parts[2].has(flags::FIN));
        let joined: Vec<u8> = parts.iter().flat_map(|p| p.payload.clone()).collect();
        assert_eq!(joined, msg);
        let empty = Frame::fragments(3, flags::KEYFRAME, &[]);
        assert_eq!(empty.len(), 1);
        assert!(empty[0].has(flags::FIN | flags::KEYFRAME));
    }

    #[test]
    fn proto_007_browser_payload_carries_meta_and_image() {
        let meta = BrowserFrameMeta {
            w: 1280,
            h: 720,
            ts: 1,
            format: ImageFormat::Jpeg,
        };
        let payload = meta.payload(&[0xff, 0xd8, 0xff]);
        let (back, image) = BrowserFrameMeta::parse(&payload).unwrap();
        assert_eq!(back, meta);
        assert_eq!(image, [0xff, 0xd8, 0xff]);
        assert!(BrowserFrameMeta::parse(&[0, 9, b'{']).is_none());
    }

    proptest! {
        #[test]
        fn proto_007_frames_roundtrip(id in any::<u32>(), fl in 0u8..8, payload in proptest::collection::vec(any::<u8>(), 0..512)) {
            let f = Frame::new(id, fl, payload);
            prop_assert_eq!(Frame::decode(&f.encode()).unwrap(), f);
        }
    }
}
