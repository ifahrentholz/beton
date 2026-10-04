-- 0007_session_file_imports: Sessions, die aus einer beton-Exportdatei importiert wurden
-- (DATA-010). Je importierter Session ein Eintrag mit Herkunft (`imported_from`): Session der
-- Quelle, Export-Zeitpunkt und SHA-256 der `session.jsonl`. Über den Hash ist der Import
-- idempotent; mit `force` entsteht eine weitere Session mit demselben Hash. Der Eintrag
-- markiert die Session außerdem als nicht auf diesem Host ausführbar (Pfade aus der Datei).
-- Beim Löschen der Session wird er mitgelöscht.
-- Gegenstück: migrations/postgres/0007_session_file_imports.sql.

CREATE TABLE session_file_imports (
    session_id TEXT NOT NULL PRIMARY KEY REFERENCES sessions (id),
    org_id TEXT NOT NULL,
    export_sha256 TEXT NOT NULL,
    source_session_id TEXT NOT NULL,
    exported_at TEXT NOT NULL,
    imported_at TEXT NOT NULL
);
CREATE INDEX session_file_imports_hash ON session_file_imports (org_id, export_sha256);
