//! Der Wert eines Secrets (SEC-001 AC1, AC4).
//!
//! [`SecretValue`] hat bewusst weder `Clone` noch `Serialize`; `Debug` und `Display` geben
//! `***` aus. Beim Drop wird der Puffer genullt (`zeroize`). Im Test-Build meldet ein
//! Drop-Hook, ob der Puffer beim Drop vollständig genullt war (AC4).

use std::fmt;

use zeroize::Zeroize;

/// Ein Secret-Wert im Speicher.
pub struct SecretValue(Vec<u8>);

impl SecretValue {
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Der Klartext. Nur für den unmittelbaren Gebrauch (Header setzen, Backend schreiben);
    /// nie loggen, nie in Events oder Antworten übernehmen.
    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<&str> for SecretValue {
    fn from(s: &str) -> Self {
        Self(s.as_bytes().to_vec())
    }
}

impl From<String> for SecretValue {
    fn from(s: String) -> Self {
        Self(s.into_bytes())
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl fmt::Display for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl Drop for SecretValue {
    fn drop(&mut self) {
        self.0.as_mut_slice().zeroize();
        #[cfg(test)]
        drop_hook::observe(&self.0);
        // Nullt auch die ungenutzte Kapazität und leert den Vektor.
        self.0.zeroize();
    }
}

/// Maskierte Darstellung für Ausgaben (HAR-016 AC3): bekanntes Präfix plus die letzten vier
/// Zeichen, z. B. `sk-…abcd`. Kurze oder nicht lesbare Werte ergeben nur `…`.
pub fn mask(value: &SecretValue) -> String {
    const MIN_LEN: usize = 16;
    let Ok(text) = std::str::from_utf8(value.expose()) else {
        return "…".into();
    };
    let chars: Vec<char> = text.chars().collect();
    if chars.len() < MIN_LEN || !chars.iter().all(char::is_ascii_graphic) {
        return "…".into();
    }
    let prefix: String = chars
        .iter()
        .take(5)
        .position(|c| *c == '-' || *c == '_')
        .map(|i| chars[..=i].iter().collect())
        .unwrap_or_default();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{prefix}…{tail}")
}

#[cfg(test)]
pub(crate) mod drop_hook {
    use std::cell::RefCell;

    #[derive(Debug, Clone, Copy)]
    pub(crate) struct Dropped {
        pub len: usize,
        pub all_zero: bool,
    }

    thread_local! {
        static SEEN: RefCell<Vec<Dropped>> = const { RefCell::new(Vec::new()) };
    }

    pub(crate) fn observe(buf: &[u8]) {
        SEEN.with(|s| {
            s.borrow_mut().push(Dropped {
                len: buf.len(),
                all_zero: buf.iter().all(|b| *b == 0),
            });
        });
    }

    pub(crate) fn reset() {
        SEEN.with(|s| s.borrow_mut().clear());
    }

    pub(crate) fn take() -> Vec<Dropped> {
        SEEN.with(|s| std::mem::take(&mut *s.borrow_mut()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Kompiliert nur, wenn `T` **kein** `serde::Serialize` implementiert: Mit Implementierung
    /// wären beide Blanket-Impls anwendbar und der Aufruf mehrdeutig (static_assertions-Trick).
    trait AmbiguousIfSerialize<A> {
        fn check() {}
    }
    impl<T: ?Sized> AmbiguousIfSerialize<()> for T {}
    impl<T: ?Sized + serde::Serialize> AmbiguousIfSerialize<u8> for T {}

    #[test]
    fn sec_001_ac1_secret_value_never_prints_or_serializes_plaintext() {
        <SecretValue as AmbiguousIfSerialize<_>>::check();
        let v = SecretValue::from("sk-proj-geheim-1234");
        assert_eq!(format!("{v:?}"), "***");
        assert_eq!(format!("{v}"), "***");
        assert_eq!(format!("{v:#?}"), "***");
        // Auch eingebettet in andere Debug-Ausgaben.
        assert_eq!(format!("{:?}", Some(&v)), "Some(***)");
    }

    #[test]
    fn sec_001_ac4_value_is_zeroed_on_drop() {
        drop_hook::reset();
        let v = SecretValue::from("ghp_marker_value_0123456789");
        assert_eq!(v.expose(), b"ghp_marker_value_0123456789");
        drop(v);
        let seen = drop_hook::take();
        assert_eq!(seen.len(), 1, "genau ein Drop beobachtet");
        assert_eq!(seen[0].len, 27);
        assert!(seen[0].all_zero, "Puffer beim Drop nicht genullt");
    }

    #[test]
    fn sec_001_ac4_replaced_value_is_zeroed_too() {
        drop_hook::reset();
        let mut v = SecretValue::from("first-value");
        assert_eq!(v.len(), 11);
        v = SecretValue::from_bytes(b"second".to_vec());
        assert_eq!(v.expose(), b"second");
        drop(v);
        let seen = drop_hook::take();
        assert_eq!(seen.len(), 2);
        assert!(seen.iter().all(|d| d.all_zero));
    }

    #[test]
    fn har_016_ac3_mask_shows_only_prefix_and_last_four() {
        assert_eq!(
            mask(&SecretValue::from("sk-abcdefghijklmnopqrstuvwxyzabcd")),
            "sk-…abcd"
        );
        assert_eq!(
            mask(&SecretValue::from("sk-proj-0123456789abcdefghijwxyz")),
            "sk-…wxyz"
        );
        assert_eq!(
            mask(&SecretValue::from("ghp_0123456789abcdefghijklmnopqrstuvwx")),
            "ghp_…uvwx"
        );
        // Ohne erkennbares Präfix nur die letzten vier Zeichen.
        assert_eq!(mask(&SecretValue::from("0123456789abcdefWXYZ")), "…WXYZ");
        // Kurze Werte verraten nichts.
        assert_eq!(mask(&SecretValue::from("sk-kurz")), "…");
        assert_eq!(mask(&SecretValue::from("")), "…");
        // Kein UTF-8: nichts zeigen.
        assert_eq!(mask(&SecretValue::from_bytes(vec![0xff; 40])), "…");
    }
}
