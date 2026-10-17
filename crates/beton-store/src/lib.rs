//! Persistenz: Event-Log, Projektionen und Blob-Store auf SQLite (DATA-001 … DATA-008).
//!
//! Spec: `docs/spec/06-data-sync-protocol.md`. Lokal liegt alles unter `~/.beton/`:
//! `beton.db` (SQLite, WAL) und `blobs/sha256/…`. Postgres und S3 (DATA-004, DATA-007)
//! kommen in M4 hinter dieselbe [`Store`]-Schnittstelle.
//!
//! Jede Abfrage ist auf eine Org beschränkt (DATA-001 AC2): Alle Methoden nehmen die
//! `OrgId` als ersten Parameter, und jedes SQL filtert nach `org_id`.

pub mod audit;
#[cfg(test)]
mod audit_tests;
mod blobs;
mod error;
mod events;
pub mod export;
mod idempotency;
mod imports;
mod inbox;
#[cfg(test)]
mod inbox_tests;
mod listing;
#[cfg(test)]
mod listing_tests;
mod migrate;
mod policy;
#[cfg(test)]
mod policy_tests;
mod projections;
mod redaction;
pub mod schema;
mod secrets_meta;
#[cfg(test)]
mod secrets_meta_tests;
mod sessions;
#[cfg(test)]
mod tests;
mod usage;
#[cfg(test)]
mod usage_report_tests;
#[cfg(test)]
mod usage_tests;
mod user_settings;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

pub use crate::audit::{
    AUDIT_USAGE_WINDOW, AuditEntry, AuditPage, AuditQuery, AuditVerification, NewAuditEntry,
    UsageKey,
};
pub use crate::blobs::{BlobStore, DEFAULT_MAX_BLOB_BYTES, GC_GRACE, GcReport};
pub use crate::error::{Error, Result};
pub use crate::export::{
    ExportFile, ExportHeader, ExportOptions, FileImportOutcome, FileImportRecord, FileImportStatus,
    ImportedFrom, InvalidExport, InvalidKind, ParsedExport, parse_export,
};
pub use crate::idempotency::{IDEMPOTENCY_TTL, StoredResponse};
pub use crate::imports::ImportKey;
pub use crate::inbox::{InboxQuery, InboxRecord};
pub use crate::listing::{SessionFilter, SessionScope, SessionView, fts_query};
pub use crate::migrate::SCHEMA_VERSION;
pub use crate::policy::{MAX_SNAPSHOT_SETS, MAX_SNAPSHOT_SOURCE, ShadowHits};
pub use crate::projections::{ApprovalRecord, UsageDaily, UsageRecord, UsageScope, UsageTotals};
pub use crate::redaction::{MANUAL_MARKER, ManualRedaction, ManualRedactionOutcome};
pub use crate::secrets_meta::SecretMetaRecord;
pub use crate::sessions::{
    DeleteAuthority, LocalIdentity, NewSession, SessionRecord, SessionWorktree, Tombstone,
    TreeEntry,
};
pub use crate::usage::{
    Billing, HarnessReporting, SubscriptionWindow, UsageDimension, UsageGroup, UsageQuery,
    UsageReport,
};

/// Redaction-Hook (SEC-013, PROTO-001 AC3): schwärzt alle Strings eines JSON-Werts an Ort
/// und Stelle, bevor Payloads, `raw` und Live-Events persistiert oder verteilt werden.
///
/// Die Pipeline selbst (bekannte Werte, Detektoren) liegt in `beton-secrets`; der Daemon setzt
/// sie über [`StoreOptions::redactor`]. [`NoRedaction`] ist nur der Default für Tests und
/// Werkzeuge ohne Secret-Store.
pub trait ValueRedactor: Send + Sync {
    /// `Ok(true)`, wenn sich etwas geändert hat; `Err`, wenn die Pipeline nicht bereit ist
    /// (fail closed: der Aufrufer persistiert und sendet dann nichts).
    fn redact(&self, value: &mut Value) -> std::result::Result<bool, RedactionUnavailable>;
}

/// Die Redaction-Pipeline ist nicht bereit (SEC-013, fail closed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RedactionUnavailable;

/// Redigiert nichts (Tests, Werkzeuge ohne Secret-Store).
#[derive(Debug, Default)]
pub struct NoRedaction;

impl ValueRedactor for NoRedaction {
    fn redact(&self, _value: &mut Value) -> std::result::Result<bool, RedactionUnavailable> {
        Ok(false)
    }
}

/// Einstellungen des Stores.
#[derive(Clone)]
pub struct StoreOptions {
    /// `events.store_raw`: Original-Payloads der Harnesses speichern (Default `true`).
    pub store_raw: bool,
    /// Redaction vor Persistenz und Verteilung (SEC-013).
    pub redactor: Arc<dyn ValueRedactor>,
    /// Maximale Blob-Größe (DATA-006, Default 512 MiB).
    pub max_blob_bytes: u64,
    /// Bewertet jedes `cost.delta` beim Append (USE-001, USE-003): Katalog und Preis-Overrides.
    pub pricer: Arc<beton_core::pricing::Pricer>,
}

impl Default for StoreOptions {
    fn default() -> Self {
        Self {
            store_raw: true,
            redactor: Arc::new(NoRedaction),
            max_blob_bytes: DEFAULT_MAX_BLOB_BYTES,
            pricer: Arc::new(beton_core::pricing::Pricer::default()),
        }
    }
}

impl std::fmt::Debug for StoreOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoreOptions")
            .field("store_raw", &self.store_raw)
            .field("max_blob_bytes", &self.max_blob_bytes)
            .field("pricing_version", &self.pricer.catalog().version())
            .field("price_overrides", &self.pricer.overrides().len())
            .finish_non_exhaustive()
    }
}

/// Zugriff auf Datenbank und Blob-Store eines Knotens.
#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
    blobs: BlobStore,
    options: StoreOptions,
    /// Schreibsperre des Prozesses (#287), siehe [`Store::write_guard`].
    writer: Arc<tokio::sync::Mutex<()>>,
}

/// Wie lange ein Schreiber höchstens auf die Schreibsperre des Prozesses wartet; entspricht
/// `busy_timeout` (DATA-003).
const WRITE_LOCK_TIMEOUT: Duration = Duration::from_millis(5000);

/// Schreib-Transaktion mit gehaltener Schreibsperre des Prozesses (siehe [`Store::write_tx`]).
/// Ohne `commit` rollt sie beim Drop zurück und gibt die Sperre frei.
pub(crate) struct WriteTx {
    // Reihenfolge: erst die Transaktion, dann die Sperre freigeben.
    tx: sqlx::Transaction<'static, sqlx::Sqlite>,
    _guard: tokio::sync::OwnedMutexGuard<()>,
}

impl WriteTx {
    pub(crate) async fn commit(self) -> Result<()> {
        Ok(self.tx.commit().await?)
    }
}

impl std::ops::Deref for WriteTx {
    type Target = sqlx::SqliteConnection;
    fn deref(&self) -> &Self::Target {
        &self.tx
    }
}

impl std::ops::DerefMut for WriteTx {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.tx
    }
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("blobs", &self.blobs)
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

impl Store {
    /// Bewertung beim Append: Katalog und Preis-Overrides (USE-002, USE-003).
    pub fn pricer(&self) -> &beton_core::pricing::Pricer {
        &self.options.pricer
    }

    /// Öffnet (bzw. erzeugt) den Store im Datenverzeichnis, z. B. `~/.beton`:
    /// `beton.db` plus `blobs/`. Migrationen laufen dabei unter Lock (DATA-003).
    pub async fn open(data_dir: &Path, options: StoreOptions) -> Result<Self> {
        Self::open_at(&data_dir.join("beton.db"), &data_dir.join("blobs"), options).await
    }

    pub async fn open_at(db_path: &Path, blob_dir: &Path, options: StoreOptions) -> Result<Self> {
        migrate::run(db_path, migrate::MIGRATIONS).await?;
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(connect_options(db_path))
            .await?;
        let blobs = BlobStore::open(blob_dir, options.max_blob_bytes)?;
        Ok(Self {
            pool,
            blobs,
            options,
            writer: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    /// Schreibsperre des Prozesses (#287): Alle Schreibzugriffe dieses Stores laufen
    /// nacheinander in Ankunftsreihenfolge (`tokio::sync::Mutex` ist fair, FIFO). SQLite
    /// selbst kennt keine Warteschlange: Wartende pollen im Busy-Handler (Schlafphasen bis
    /// 100 ms), und ein Dauerschreiber, der die Sperre direkt nach dem Commit wieder nimmt,
    /// gewinnt fast immer. Andere Prozesse auf derselben Datei regelt weiter `busy_timeout`.
    ///
    /// Wartet höchstens [`WRITE_LOCK_TIMEOUT`] (wie `busy_timeout`); danach ein
    /// Datenbankfehler wie bei `SQLITE_BUSY`.
    pub(crate) async fn write_guard(&self) -> Result<tokio::sync::OwnedMutexGuard<()>> {
        tokio::time::timeout(WRITE_LOCK_TIMEOUT, self.writer.clone().lock_owned())
            .await
            .map_err(|_| Error::Db(sqlx::Error::PoolTimedOut))
    }

    /// Schreib-Transaktion unter der Schreibsperre des Prozesses ([`Store::write_guard`]) mit
    /// `BEGIN IMMEDIATE`: nimmt die SQLite-Schreibsperre sofort. Bei `DEFERRED` scheitert ein
    /// Lesen-dann-Schreiben im WAL-Modus sofort mit `SQLITE_BUSY`, sobald ein anderer
    /// Schreiber dazwischen committet; der Busy-Timeout greift dort nicht.
    pub(crate) async fn write_tx(&self) -> Result<WriteTx> {
        // Erst die Sperre, dann die Verbindung: Wartende belegen keine Pool-Verbindung.
        let guard = self.write_guard().await?;
        let tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        Ok(WriteTx { tx, _guard: guard })
    }

    pub fn blobs(&self) -> &BlobStore {
        &self.blobs
    }

    /// Schließt alle Verbindungen (z. B. vor dem Beenden des Daemons).
    pub async fn close(&self) {
        self.pool.close().await;
    }
}

/// Verbindungsparameter laut DATA-003: WAL, `synchronous=FULL`, `busy_timeout=5000`,
/// Fremdschlüssel aktiv.
pub(crate) fn connect_options(db_path: &Path) -> SqliteConnectOptions {
    SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Full)
        .busy_timeout(Duration::from_millis(5000))
        .foreign_keys(true)
}

/// SQLite-Integritätsprüfung (`PRAGMA quick_check`) ohne Schreibzugriff und ohne Migrationen
/// (OBS-005). Liefert die gemeldeten Probleme; leer heißt in Ordnung.
pub async fn quick_check(db_path: &Path) -> Result<Vec<String>> {
    use sqlx::{ConnectOptions as _, Row as _};
    // Ohne `-wal` läuft kein Schreiber: `immutable` liest dann, ohne `-shm`/`-wal` anzulegen.
    // Mit `-wal` (Daemon aktiv) liest es schreibgeschützt über die vorhandenen Dateien.
    let mut wal = db_path.as_os_str().to_owned();
    wal.push("-wal");
    let idle = !Path::new(&wal).exists();
    let mut conn = SqliteConnectOptions::new()
        .filename(db_path)
        .read_only(true)
        .immutable(idle)
        .create_if_missing(false)
        .connect()
        .await?;
    let rows = sqlx::query("PRAGMA quick_check")
        .fetch_all(&mut conn)
        .await?;
    let _ = sqlx::Connection::close(conn).await;
    Ok(rows
        .iter()
        .filter_map(|r| r.try_get::<String, _>(0).ok())
        .filter(|s| s != "ok")
        .collect())
}

/// Worktree-Pfade aller Sessions (SES-016 AC3: `beton doctor` meldet Verzeichnisse ohne
/// Session). Liest wie [`quick_check`] schreibgeschützt und ohne Migrationen.
pub async fn worktree_paths(db_path: &Path) -> Result<Vec<String>> {
    use sqlx::ConnectOptions as _;
    let mut wal = db_path.as_os_str().to_owned();
    wal.push("-wal");
    let idle = !Path::new(&wal).exists();
    let mut conn = SqliteConnectOptions::new()
        .filename(db_path)
        .read_only(true)
        .immutable(idle)
        .create_if_missing(false)
        .connect()
        .await?;
    let paths: Vec<String> =
        sqlx::query_scalar("SELECT worktree_path FROM sessions WHERE worktree_path IS NOT NULL")
            .fetch_all(&mut conn)
            .await?;
    let _ = sqlx::Connection::close(conn).await;
    Ok(paths)
}

/// Standard-Datenverzeichnis: `$BETON_HOME`, sonst `~/.beton` (bzw. `%USERPROFILE%\.beton`).
pub fn default_data_dir() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("BETON_HOME").filter(|h| !h.is_empty()) {
        return Some(PathBuf::from(home));
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|home| PathBuf::from(home).join(".beton"))
}

#[cfg(test)]
pub(crate) mod testutil {
    use beton_core::id::OrgId;

    use super::*;

    pub(crate) struct TestStore {
        pub store: Store,
        pub dir: tempfile::TempDir,
        pub local: LocalIdentity,
    }

    pub(crate) async fn store_with(options: StoreOptions) -> TestStore {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path(), options).await.unwrap();
        let local = store.ensure_local().await.unwrap();
        TestStore { store, dir, local }
    }

    pub(crate) async fn store() -> TestStore {
        store_with(StoreOptions::default()).await
    }

    pub(crate) fn org(t: &TestStore) -> OrgId {
        t.local.org
    }
}

#[cfg(test)]
mod quick_check_tests {
    use super::*;

    #[tokio::test]
    async fn obs_005_quick_check_creates_no_side_files() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path(), StoreOptions::default())
            .await
            .unwrap();
        store.ensure_local().await.unwrap();
        store.close().await;
        let db = dir.path().join("beton.db");
        let names = || {
            let mut v: Vec<String> = std::fs::read_dir(dir.path())
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            v.sort();
            v
        };
        let before = names();
        assert!(quick_check(&db).await.unwrap().is_empty());
        assert_eq!(names(), before);
    }
}
