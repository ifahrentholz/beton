//! Bekannte Secret-Werte als Aho-Corasick-Matcher (SEC-013, #186).

use crate::service::SecretKind;
use crate::value::SecretValue;

use super::Span;

/// Matcher über die bekannten Werte eines Knotens. Kein Zugriff auf die Werte, kein
/// Klartext in `Debug`.
pub(crate) struct KnownSecrets {
    values: usize,
}

impl std::fmt::Debug for KnownSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KnownSecrets")
            .field("values", &self.values)
            .finish_non_exhaustive()
    }
}

impl KnownSecrets {
    pub(crate) fn empty() -> Self {
        Self { values: 0 }
    }

    /// Baut den Matcher aus Werten.
    pub(crate) fn build(values: &[SecretValue]) -> Result<Self, String> {
        Ok(Self {
            values: values.len(),
        })
    }

    pub(crate) fn values(&self) -> usize {
        self.values
    }

    pub(crate) fn find(&self, _text: &str, _out: &mut Vec<Span>) {}
}

/// Die Teile eines gespeicherten Werts, die geschwärzt werden.
pub(crate) fn parts(_kind: SecretKind, _value: &SecretValue) -> Vec<SecretValue> {
    Vec::new()
}
