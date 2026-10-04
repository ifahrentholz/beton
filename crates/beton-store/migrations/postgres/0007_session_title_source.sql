-- 0007_session_title_source: Herkunft des Session-Titels (SES-010, UX-009). Ein vom User
-- gesetzter Titel wird nie durch einen generierten ersetzt; die UI zeigt die Herkunft an.
-- Werte: '' (kein Titel), 'user', 'generated', 'harness'.
-- Gegenstück: migrations/sqlite/0007_session_title_source.sql.

ALTER TABLE sessions ADD COLUMN title_source TEXT NOT NULL DEFAULT '';
