-- 0013_policy_snapshots_shadow: Set-Snapshots für Explain (POL-027 AC2) und Projektion der
-- Shadow-Treffer (POL-027 AC4).
-- Folgt auf 0012_usage_subscription_windows (WP-44).
-- Gegenstück: migrations/postgres/0013_policy_snapshots_shadow.sql.

-- Zusammensetzung eines effektiven Policy-Sets je Session und `policy_set_hash`; die Inhalte
-- der Sets liegen inhaltsadressiert im Blob-Store der Session (Version = SHA-256 des Inhalts).
CREATE TABLE policy_set_snapshots (
    org_id TEXT NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions (id),
    policy_set_hash TEXT NOT NULL,
    manifest TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (org_id, session_id, policy_set_hash)
);

-- Treffer von Regeln im Shadow-Betrieb (`dry_run: true` in `policy.decision.matched`) je
-- Session, Tag (UTC bis zur Einstellung `user.timezone`, #195), Regel und Verdikt. Projektion
-- aus dem Event-Log, je Session neu aufbaubar.
CREATE TABLE policy_shadow_daily (
    org_id TEXT NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions (id),
    day TEXT NOT NULL,
    scope TEXT NOT NULL,
    set_name TEXT NOT NULL,
    rule_id TEXT NOT NULL,
    verdict TEXT NOT NULL,
    hits INTEGER NOT NULL,
    last_seq INTEGER NOT NULL,
    last_at TEXT NOT NULL,
    PRIMARY KEY (org_id, session_id, day, scope, set_name, rule_id, verdict)
);
CREATE INDEX policy_shadow_daily_day ON policy_shadow_daily (org_id, day);
