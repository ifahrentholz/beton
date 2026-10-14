//! Secret-Werte nur im Speicher. Ausschließlich für Tests; nie im Daemon.

use std::collections::BTreeMap;
use std::sync::Mutex;

use super::{Backend, BackendKind};
use crate::error::{Error, Result};
use crate::{SecretRef, SecretValue};

#[derive(Debug, Default)]
pub struct MemoryBackend {
    values: Mutex<BTreeMap<String, SecretValue>>,
}

impl MemoryBackend {
    fn values(&self) -> Result<std::sync::MutexGuard<'_, BTreeMap<String, SecretValue>>> {
        self.values
            .lock()
            .map_err(|_| Error::Unavailable("Speicher-Backend vergiftet".into()))
    }
}

impl Backend for MemoryBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Memory
    }

    fn put(&self, r: &SecretRef, value: &SecretValue) -> Result<()> {
        self.values()?.insert(
            r.to_string(),
            SecretValue::from_bytes(value.expose().to_vec()),
        );
        Ok(())
    }

    fn get(&self, r: &SecretRef) -> Result<SecretValue> {
        self.values()?
            .get(&r.to_string())
            .map(|v| SecretValue::from_bytes(v.expose().to_vec()))
            .ok_or_else(|| Error::NotFound(r.to_string()))
    }

    fn delete(&self, r: &SecretRef) -> Result<bool> {
        Ok(self.values()?.remove(&r.to_string()).is_some())
    }
}
