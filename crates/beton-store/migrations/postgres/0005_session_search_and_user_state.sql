-- 0005_session_search_and_user_state: Gelesen-Stand und Anpinnen je User (SES-012) sowie
-- Volltextsuche über Titel und Nachrichten (SES-012 AC1).
-- Gleiche Semantik wie SQLite FTS5 (`unicode61 remove_diacritics 0`): Konfiguration `simple`
-- (keine Stammformen, keine Stoppwörter), jedes Suchwort als Präfix (`wort:*`), alle Wörter
-- in demselben Dokument (`&`). Der GIN-Index ist dialektspezifisch und nicht Teil der Parität.
-- Gegenstück: migrations/sqlite/0005_session_search_and_user_state.sql.

CREATE TABLE session_user_state (
    org_id TEXT NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions (id),
    user_id TEXT NOT NULL REFERENCES users (id),
    pinned BOOLEAN NOT NULL,
    read_seq BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (session_id, user_id)
);
CREATE INDEX session_user_state_changed ON session_user_state (org_id, user_id, updated_at);

CREATE TABLE search_docs (
    id BIGINT NOT NULL GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    org_id TEXT NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions (id),
    doc TEXT NOT NULL,
    body TEXT NOT NULL
);
CREATE UNIQUE INDEX search_docs_doc ON search_docs (session_id, doc);
CREATE INDEX search_docs_org ON search_docs (org_id, session_id);

CREATE INDEX search_docs_fts ON search_docs USING GIN (to_tsvector('simple', body));
