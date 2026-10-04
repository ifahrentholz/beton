-- 0004_session_search_and_user_state: Gelesen-Stand und Anpinnen je User (SES-012) sowie
-- Volltextsuche über Titel und Nachrichten (SES-012 AC1).
-- `search_docs` ist in beiden Dialekten gleich; nur der Volltext-Index ist dialektspezifisch
-- (hier FTS5 mit externem Inhalt, in Postgres ein GIN-Index über `to_tsvector('simple', …)`)
-- und gehört nicht zur Parität (DATA-003 AC1).
-- Gegenstück: migrations/postgres/0004_session_search_and_user_state.sql.

CREATE TABLE session_user_state (
    org_id TEXT NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions (id),
    user_id TEXT NOT NULL REFERENCES users (id),
    pinned INTEGER NOT NULL,
    read_seq INTEGER NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (session_id, user_id)
);
CREATE INDEX session_user_state_changed ON session_user_state (org_id, user_id, updated_at);

-- Projektion (DATA-005): ein Dokument je Titel (`title`) und Nachricht (`msg:<seq>`).
CREATE TABLE search_docs (
    id INTEGER NOT NULL PRIMARY KEY,
    org_id TEXT NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions (id),
    doc TEXT NOT NULL,
    body TEXT NOT NULL
);
CREATE UNIQUE INDEX search_docs_doc ON search_docs (session_id, doc);
CREATE INDEX search_docs_org ON search_docs (org_id, session_id);

CREATE VIRTUAL TABLE search_fts USING fts5 (
    body,
    content = 'search_docs',
    content_rowid = 'id',
    tokenize = 'unicode61 remove_diacritics 0'
);
