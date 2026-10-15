// Generiert von `cargo xtask codegen` aus den Rust-Typen. Nicht von Hand ändern.
import type { HarnessReportingView } from "./HarnessReportingView";
import type { SubscriptionWindowView } from "./SubscriptionWindowView";
import type { UsageItem } from "./UsageItem";
import type { UsageTotals } from "./UsageTotals";

/**
 * Antwort von `GET /v1/usage` und Ausgabe von `beton usage --json` (USE-005 AC1, USE-007
 * AC1; Schema `schemas/v1/usage.schema.json`).
 */
export type UsageReportView = { 
/**
 * Beginn (inklusiv), RFC 3339 UTC.
 */
from: string, 
/**
 * Ende (exklusiv), RFC 3339 UTC.
 */
to: string, 
/**
 * Zeitzone der Tagesgrenzen, z. B. `Europe/Berlin` oder `+02:00`.
 */
tz: string, group_by: Array<string>, 
/**
 * Version des eingebauten Preis-Katalogs (USE-002); Records tragen ihre eigene.
 */
pricing_version: string, 
/**
 * Eine Gruppe je Kombination der `group_by`-Werte und Abrechnungsart (kein Paging:
 * eine Auswertung, keine Liste von Ressourcen).
 */
groups: Array<UsageItem>, totals: UsageTotals, subscriptions: Array<SubscriptionWindowView>, reporting: Array<HarnessReportingView>, };
