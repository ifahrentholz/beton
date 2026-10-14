//! Eingebettete Migrationen mit Lock und Backup (DATA-003).
//!
//! - Die SQL-Dateien liegen unter `migrations/{sqlite,postgres}/NNNN_<name>.sql` und sind
//!   per `include_str!` ins Binary eingebettet.
//! - Ein exklusiver Datei-Lock (`<db>.lock`) verhindert, dass zwei gleichzeitig startende
//!   Daemons doppelt migrieren.
//! - Vor jeder Migration einer bestehenden Datenbank entsteht `<db>.bak-<version>`.
//! - Fehlende Versionen werden auch unterhalb des aktuellen Stands nachgezogen (Lücken durch
//!   reservierte Nummern paralleler Arbeitspakete).
//! - Down-Migrationen gibt es nicht; eine Datenbank mit neuerer Version wird abgelehnt
//!   (`schema_too_new`).

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use fs4::FileExt;
use sqlx::{Connection, Row, SqliteConnection};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy)]
pub(crate) struct Migration {
    pub version: i64,
    pub name: &'static str,
    pub sql: &'static str,
}

pub(crate) const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "init",
        sql: include_str!("../migrations/sqlite/0001_init.sql"),
    },
    Migration {
        version: 2,
        name: "idempotency",
        sql: include_str!("../migrations/sqlite/0002_idempotency.sql"),
    },
    Migration {
        version: 3,
        name: "approvals_per_session",
        sql: include_str!("../migrations/sqlite/0003_approvals_per_session.sql"),
    },
    Migration {
        version: 4,
        name: "session_worktree",
        sql: include_str!("../migrations/sqlite/0004_session_worktree.sql"),
    },
    Migration {
        version: 5,
        name: "session_search_and_user_state",
        sql: include_str!("../migrations/sqlite/0005_session_search_and_user_state.sql"),
    },
    Migration {
        version: 6,
        name: "session_imports",
        sql: include_str!("../migrations/sqlite/0006_session_imports.sql"),
    },
    Migration {
        version: 7,
        name: "session_file_imports",
        sql: include_str!("../migrations/sqlite/0007_session_file_imports.sql"),
    },
    Migration {
        version: 8,
        name: "session_title_source",
        sql: include_str!("../migrations/sqlite/0008_session_title_source.sql"),
    },
    // 0009 ist für WP-34 (Usage) reserviert; siehe `migrate` zu Lücken.
    Migration {
        version: 10,
        name: "secrets_audit",
        sql: include_str!("../migrations/sqlite/0010_secrets_audit.sql"),
    },
];

/// Schema-Version, die dieses Binary erwartet.
pub const SCHEMA_VERSION: i64 = 10;

const CREATE_BOOKKEEPING: &str = "CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER NOT NULL PRIMARY KEY,
    name TEXT NOT NULL,
    applied_at TEXT NOT NULL
)";

/// Bringt die Datenbank unter `db_path` auf den Stand von `migrations`.
pub(crate) async fn run(db_path: &Path, migrations: &[Migration]) -> Result<()> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _lock = lock(db_path).await?;
    let mut conn = SqliteConnection::connect_with(&crate::connect_options(db_path)).await?;
    let result = migrate(&mut conn, db_path, migrations).await;
    conn.close().await?;
    result
}

async fn migrate(
    conn: &mut SqliteConnection,
    db_path: &Path,
    migrations: &[Migration],
) -> Result<()> {
    sqlx::raw_sql(CREATE_BOOKKEEPING)
        .execute(&mut *conn)
        .await?;
    let current: i64 = sqlx::query("SELECT COALESCE(MAX(version), 0) FROM schema_migrations")
        .fetch_one(&mut *conn)
        .await?
        .try_get(0)?;
    let latest = migrations.last().map_or(0, |m| m.version);
    if current > latest {
        return Err(Error::SchemaTooNew {
            db: current,
            binary: latest,
        });
    }
    // Angewendet wird jede noch fehlende Version, nicht nur die oberhalb des Stands: Parallele
    // Arbeitspakete reservieren Nummern, und eine niedrigere kann nach einer höheren gemergt
    // werden. Migrationen mit Lücke dürfen deshalb nicht voneinander abhängen.
    let applied: Vec<i64> = sqlx::query_scalar("SELECT version FROM schema_migrations")
        .fetch_all(&mut *conn)
        .await?;
    let mut version = current;
    for m in migrations.iter().filter(|m| !applied.contains(&m.version)) {
        if version > 0 {
            backup(conn, db_path, version).await?;
        }
        tracing::info!(
            version = m.version,
            name = m.name,
            "Migration wird angewendet"
        );
        let mut tx = conn.begin().await?;
        sqlx::raw_sql(m.sql).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO schema_migrations (version, name, applied_at) VALUES (?, ?, ?)")
            .bind(m.version)
            .bind(m.name)
            .bind(beton_core::time::Timestamp::now().to_string())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        version = version.max(m.version);
    }
    Ok(())
}

/// Konsistente Kopie der Datenbank vor einer Migration: `<db>.bak-<version>`.
async fn backup(conn: &mut SqliteConnection, db_path: &Path, version: i64) -> Result<()> {
    let target = sibling(db_path, &format!(".bak-{version}"));
    if target.exists() {
        std::fs::remove_file(&target)?;
    }
    let target_str = target
        .to_str()
        .ok_or_else(|| Error::corrupt("Backup-Pfad ist kein UTF-8"))?;
    sqlx::query("VACUUM INTO ?")
        .bind(target_str)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

fn sibling(db_path: &Path, suffix: &str) -> PathBuf {
    let mut name = db_path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    db_path.with_file_name(name)
}

/// Exklusiver Lock auf `<db>.lock`; wird beim Drop der Datei freigegeben.
async fn lock(db_path: &Path) -> Result<File> {
    let path = sibling(db_path, ".lock");
    tokio::task::spawn_blocking(move || -> Result<File> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)?;
        FileExt::lock(&file)?;
        Ok(file)
    })
    .await
    .map_err(|e| Error::corrupt(format!("Lock-Task abgebrochen: {e}")))?
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use super::*;

    /// Eine Migration nach der neuesten (simuliert ein neueres Binary).
    const TEST_NEXT: Migration = Migration {
        version: SCHEMA_VERSION + 1,
        name: "test_extra",
        sql: "CREATE TABLE test_extra (org_id TEXT NOT NULL, id TEXT NOT NULL PRIMARY KEY);",
    };

    async fn versions(db: &Path) -> Vec<i64> {
        let mut conn = SqliteConnection::connect_with(&crate::connect_options(db))
            .await
            .unwrap();
        sqlx::query("SELECT version FROM schema_migrations ORDER BY version")
            .fetch_all(&mut conn)
            .await
            .unwrap()
            .iter()
            .map(|r| r.get(0))
            .collect()
    }

    #[test]
    fn migration_files_are_embedded_in_order() {
        let latest = MIGRATIONS.last().unwrap().version;
        assert_eq!(latest, SCHEMA_VERSION);
        // Streng aufsteigend; Lücken sind erlaubt, solange parallele Arbeitspakete Nummern
        // reserviert haben (die fehlende Migration wird später nachgezogen).
        for pair in MIGRATIONS.windows(2) {
            assert!(pair[0].version < pair[1].version, "{pair:?}");
        }
        assert_eq!(MIGRATIONS[0].version, 1);
    }

    /// Eine Migration, die erst nach einer höheren Nummer gemergt wird (reservierte Nummer
    /// eines parallelen Arbeitspakets), wird trotzdem nachgezogen (DATA-003).
    #[tokio::test]
    async fn data_003_late_migration_below_head_is_applied() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("beton.db");
        let first = Migration {
            version: 1,
            name: "a",
            sql: "CREATE TABLE a (id TEXT NOT NULL PRIMARY KEY);",
        };
        let late = Migration {
            version: 2,
            name: "b",
            sql: "CREATE TABLE b (id TEXT NOT NULL PRIMARY KEY);",
        };
        let head = Migration {
            version: 3,
            name: "c",
            sql: "CREATE TABLE c (id TEXT NOT NULL PRIMARY KEY);",
        };
        run(&db, &[first, head]).await.unwrap();
        assert_eq!(versions(&db).await, vec![1, 3]);
        run(&db, &[first, late, head]).await.unwrap();
        assert_eq!(versions(&db).await, vec![1, 2, 3]);
        // Erneut: nichts mehr zu tun.
        run(&db, &[first, late, head]).await.unwrap();
        assert_eq!(versions(&db).await, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn data_003_ac2_older_binary_refuses_newer_schema() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("beton.db");
        let newer: Vec<Migration> = MIGRATIONS.iter().copied().chain([TEST_NEXT]).collect();
        run(&db, &newer).await.unwrap();

        let err = run(&db, MIGRATIONS).await.unwrap_err();
        assert_eq!(err.code(), "schema_too_new");
        assert!(
            matches!(err, Error::SchemaTooNew { db, binary } if db == SCHEMA_VERSION + 1 && binary == SCHEMA_VERSION),
            "{err}"
        );
        // Auch der Store selbst startet nicht.
        let err = crate::Store::open(dir.path(), crate::StoreOptions::default())
            .await
            .unwrap_err();
        assert_eq!(err.code(), "schema_too_new");
    }

    #[tokio::test]
    async fn data_003_backup_is_written_before_each_migration() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("beton.db");
        run(&db, &MIGRATIONS[..1]).await.unwrap();
        assert!(!dir.path().join("beton.db.bak-0").exists());

        run(&db, MIGRATIONS).await.unwrap();
        let backup = dir.path().join("beton.db.bak-1");
        assert!(backup.exists());
        assert_eq!(versions(&backup).await, vec![1]);
        assert_eq!(
            versions(&db).await,
            MIGRATIONS.iter().map(|m| m.version).collect::<Vec<_>>()
        );
    }

    #[test]
    fn data_003_ac3_concurrent_daemons_migrate_once() {
        for _ in 0..10 {
            let dir = tempfile::tempdir().unwrap();
            let db = dir.path().join("beton.db");
            let barrier = Arc::new(Barrier::new(4));
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    let db = db.clone();
                    let barrier = barrier.clone();
                    std::thread::spawn(move || {
                        let rt = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .unwrap();
                        barrier.wait();
                        let all: Vec<Migration> =
                            MIGRATIONS.iter().copied().chain([TEST_NEXT]).collect();
                        rt.block_on(run(&db, &all))
                    })
                })
                .collect();
            for h in handles {
                h.join().unwrap().unwrap();
            }
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            assert_eq!(
                rt.block_on(versions(&db)),
                MIGRATIONS
                    .iter()
                    .map(|m| m.version)
                    .chain([SCHEMA_VERSION + 1])
                    .collect::<Vec<_>>()
            );
        }
    }
}
