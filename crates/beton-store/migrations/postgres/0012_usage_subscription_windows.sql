-- 0012_usage_subscription_windows: Abo-Fenster der Vendor-Logins (USE-004). Je Session,
-- Anbieter und Fenster der zuletzt gemeldete Stand aus `usage.subscription` (genutzter Anteil,
-- Reset-Zeitpunkt), dazu der Harness der Session. Usage-Ansichten zeigen je Harness, Anbieter
-- und Fenster die jüngste Meldung über alle Sessions. Fensternamen sind vereinheitlicht
-- (`5h`, `7d`, auch für ältere Events mit `five_hour` bzw. `300m`).
-- Projektion aus dem Event-Log, je Session neu aufbaubar; ältere Sessions füllt
-- `beton admin projections rebuild`. Folgt auf 0011_inbox_policy_state (WP-37).
-- Gegenstück: migrations/sqlite/0012_usage_subscription_windows.sql.

CREATE TABLE usage_subscription_windows (
    org_id TEXT NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions (id),
    harness TEXT NOT NULL,
    vendor TEXT NOT NULL,
    window_name TEXT NOT NULL,
    used_pct DOUBLE PRECISION NOT NULL,
    resets_at TIMESTAMPTZ NOT NULL,
    seq BIGINT NOT NULL,
    ts TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (session_id, vendor, window_name)
);
CREATE INDEX usage_subscription_windows_latest
    ON usage_subscription_windows (org_id, harness, vendor, window_name, ts);
