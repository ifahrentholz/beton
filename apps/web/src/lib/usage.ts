import type { HarnessReportingView, SubscriptionWindowView, UsageItem } from '@beton/sdk'

/*
 * Verbrauch (USE-006) ohne React: Zeiträume, Formatierung, Zeilen der Aufschlüsselung, CSV,
 * Kennzahlen, Zeitreihe und Abo-Fenster. Alle Summen kommen aus `GET /v1/usage`; hier wird
 * nur zusammengefasst und formatiert (Subscription nie in Geld, ADR-0034).
 */

/** Kalendertag `YYYY-MM-DD` in der lokalen Zeitzone. */
export function localDate(d: Date): string {
  const p = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`
}

/** Versatz der lokalen Zeitzone, z. B. `+02:00` (Tagesgrenzen der API). */
export function tzOffset(d: Date = new Date()): string {
  const min = -d.getTimezoneOffset()
  const sign = min < 0 ? '-' : '+'
  const abs = Math.abs(min)
  return `${sign}${String(Math.floor(abs / 60)).padStart(2, '0')}:${String(abs % 60).padStart(2, '0')}`
}

function addDays(day: string, n: number): string {
  const [y, m, d] = day.split('-').map(Number) as [number, number, number]
  return localDate(new Date(y, m - 1, d + n))
}

/** Alle Tage von `from` bis `to`, beide einschließlich. */
export function daysBetween(from: string, to: string): string[] {
  const out: string[] = []
  for (let d = from; d <= to && out.length < 400; d = addDays(d, 1)) out.push(d)
  return out
}

export type PeriodId = 'today' | '7d' | '14d' | 'month' | 'custom'

/** Zeitraum der Seite; `from` und `to` sind Tage einschließlich. */
export interface Period {
  id: PeriodId
  label: string
  from: string
  to: string
}

/** Zeitraumwahl wie im Entwurf: Heute, 7 Tage, 14 Tage, laufender Monat. */
export function periods(now: Date): Period[] {
  const today = localDate(now)
  const month = new Date(now.getFullYear(), now.getMonth(), 1)
  return [
    { id: 'today', label: 'Heute', from: today, to: today },
    { id: '7d', label: '7 Tage', from: addDays(today, -6), to: today },
    { id: '14d', label: '14 Tage', from: addDays(today, -13), to: today },
    { id: 'month', label: now.toLocaleDateString('de-DE', { month: 'long' }), from: localDate(month), to: today },
  ]
}

/** „20.09.“ aus `2026-09-20`. */
export function shortDay(day: string): string {
  return `${day.slice(8, 10)}.${day.slice(5, 7)}.`
}

/** Eigener Zeitraum (Popover „Zeitraum …“): beide Tage einschließlich, kurz „20.09.–03.10.“. */
export function customPeriod(from: string, to: string): Period {
  return { id: 'custom', label: `${shortDay(from)}–${shortDay(to)}`, from, to }
}

// ------------------------------------------------------------------ Formatierung

/** Tokens kurz: `640 k`, `1,2 Mio.`, unter 1000 exakt. */
export function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toLocaleString('de-DE', { maximumFractionDigits: 1 })} Mio.`
  if (n >= 1000) return `${Math.round(n / 1000).toLocaleString('de-DE')} k`
  return n.toLocaleString('de-DE')
}

/** Betrag in USD, deutsch formatiert (`2,71 $`). */
export function dollars(v: number): string {
  return v.toLocaleString('de-DE', { style: 'currency', currency: 'USD' })
}

/** Tokens einer Gruppe für Kennzahlen und Zeitreihe: Eingabe plus Ausgabe (Cache separat). */
export function tokensOf(g: Pick<UsageItem, 'input_tokens' | 'output_tokens'>): number {
  return g.input_tokens + g.output_tokens
}

function dateOf(day: string): Date {
  const [y, m, d] = day.split('-').map(Number) as [number, number, number]
  return new Date(y, m - 1, d)
}

/** Achsenbeschriftung eines Tages, z. B. `So 20.`. */
export function dayShort(day: string): string {
  const d = dateOf(day)
  return `${d.toLocaleDateString('de-DE', { weekday: 'short' }).replace('.', '')} ${d.getDate()}.`
}

/** Zeilenname eines Tages, z. B. `Heute, Sa 3.10.` bzw. `Fr 2.10.`. */
export function dayLong(day: string, today: string): string {
  const d = dateOf(day)
  const text = `${d.toLocaleDateString('de-DE', { weekday: 'short' }).replace('.', '')} ${d.getDate()}.${d.getMonth() + 1}.`
  return day === today ? `Heute, ${text}` : text
}

// ------------------------------------------------------------------ Aufschlüsselung

export type Dim = 'day' | 'session' | 'project' | 'harness' | 'model' | 'auth' | 'user' | 'team'

export const DIMS: { id: Dim; label: string; slug: string; central?: boolean }[] = [
  { id: 'day', label: 'Tag', slug: 'tag' },
  { id: 'session', label: 'Session', slug: 'session' },
  { id: 'project', label: 'Projekt', slug: 'projekt' },
  { id: 'harness', label: 'Harness', slug: 'harness' },
  { id: 'model', label: 'Modell', slug: 'modell' },
  { id: 'auth', label: 'Anmeldung', slug: 'anmeldung' },
  { id: 'user', label: 'Person', slug: 'person', central: true },
  { id: 'team', label: 'Team', slug: 'team', central: true },
]

type GroupBy = 'day' | 'session' | 'project' | 'harness' | 'model' | 'auth_source'

/** `group_by` je Dimension; Session und Harness bringen ihre Badge-Angaben mit. */
export function groupByFor(dim: Dim): GroupBy[] {
  switch (dim) {
    case 'day':
      return ['day']
    case 'session':
      return ['session', 'harness']
    case 'project':
      return ['project']
    case 'harness':
      return ['harness', 'auth_source']
    case 'model':
      return ['model']
    default:
      return ['auth_source']
  }
}

/** Abrechnung einer Zeile: Subscription ohne Geld, Kosten, ohne Preis oder kostenlos. */
export type Billing = { kind: 'subscription' } | { kind: 'cost'; usd: number } | { kind: 'unpriced'; count: number } | { kind: 'free' }

export interface UsageRow {
  key: string
  name: string
  harnesses: string[]
  authSources: string[]
  turns: number
  input: number
  output: number
  cache: number
  billing: Billing
  /** Session-ID der Zeile (Dimension Session, öffnet die Session, USE-006 AC2). */
  sessionId?: string
  /** API-Äquivalent der Subscription-Nutzung in USD, nur informativ (USE-004 AC1, ADR-0034). */
  equivalentUsd?: number
}

const AUTH_LABEL: Record<string, string> = {
  vendor_cli: 'Subscription über CLI-Login',
  api_key: 'API-Key',
  gateway: 'Gateway',
  none: 'Lokal',
}

export function authLabel(source: string): string {
  return AUTH_LABEL[source] ?? source
}

function keyOf(dim: Dim, g: UsageItem): string {
  switch (dim) {
    case 'day':
      return g.day ?? ''
    case 'session':
      return g.session_id ?? ''
    case 'project':
      return g.project_id ?? ''
    case 'harness':
      return g.harness ?? ''
    case 'model':
      return g.model ?? ''
    default:
      return g.auth_source ?? ''
  }
}

function nameOf(dim: Dim, key: string, g: UsageItem, today: string): string {
  switch (dim) {
    case 'day':
      return dayLong(key, today)
    case 'session':
      return g.session_title || 'Ohne Titel'
    case 'project':
      return key || 'Ohne Projekt'
    case 'auth':
      return authLabel(key)
    default:
      return key
  }
}

/** Fasst die Gruppen der API zu einer Zeile je Wert der Dimension zusammen. */
export function rowsFor(dim: Dim, groups: UsageItem[], today: string): UsageRow[] {
  const rows = new Map<string, UsageRow & { costUsd: number; unpriced: number; subscription: boolean; equivalent: number | undefined }>()
  for (const g of groups) {
    const key = keyOf(dim, g)
    let r = rows.get(key)
    if (!r) {
      r = {
        key,
        name: nameOf(dim, key, g, today),
        harnesses: [],
        authSources: [],
        turns: 0,
        input: 0,
        output: 0,
        cache: 0,
        billing: { kind: 'free' },
        costUsd: 0,
        unpriced: 0,
        subscription: false,
        equivalent: undefined,
        ...(dim === 'session' && g.session_id ? { sessionId: g.session_id } : {}),
      }
      rows.set(key, r)
    }
    if (g.harness && !r.harnesses.includes(g.harness)) r.harnesses.push(g.harness)
    if (g.auth_source && !r.authSources.includes(g.auth_source)) r.authSources.push(g.auth_source)
    r.turns += g.turns
    r.input += g.input_tokens
    r.output += g.output_tokens
    r.cache += g.cache_read_tokens
    r.unpriced += g.unpriced_count
    if (g.billing === 'subscription') {
      r.subscription = true
      if (g.equivalent_usd !== null) r.equivalent = (r.equivalent ?? 0) + g.equivalent_usd
    } else r.costUsd += g.cost_usd ?? 0
  }
  const out: UsageRow[] = []
  for (const { costUsd, unpriced, subscription, equivalent, ...row } of rows.values()) {
    const billing: Billing =
      unpriced > 0
        ? { kind: 'unpriced', count: unpriced }
        : costUsd > 0
          ? { kind: 'cost', usd: costUsd }
          : subscription
            ? { kind: 'subscription' }
            : { kind: 'free' }
    out.push({ ...row, billing, ...(billing.kind === 'subscription' && equivalent !== undefined ? { equivalentUsd: equivalent } : {}) })
  }
  if (dim === 'day') return out.sort((a, b) => b.key.localeCompare(a.key))
  return out.sort((a, b) => b.input + b.output - (a.input + a.output) || a.name.localeCompare(b.name))
}

/** Abrechnung als Text (CSV). */
export function billingText(b: Billing): string {
  switch (b.kind) {
    case 'subscription':
      return 'Subscription'
    case 'cost':
      return dollars(b.usd)
    case 'unpriced':
      return 'ohne Preis'
    default:
      return 'kostenlos'
  }
}

/** Ein Feld nach RFC 4180. */
function cell(v: string | number): string {
  const s = String(v)
  return /[",\n\r]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s
}

/** CSV der sichtbaren Tabelle (USE-006 AC3): dieselben Spalten, Tokens als ganze Zahlen. */
export function toCsv(dim: Dim, rows: UsageRow[]): string {
  const label = DIMS.find((d) => d.id === dim)?.label ?? dim
  const lines = [[label, 'Turns', 'Eingabe', 'Ausgabe', 'Cache gelesen', 'Abrechnung']]
  for (const r of rows) lines.push([r.name, String(r.turns), String(r.input), String(r.output), String(r.cache), billingText(r.billing)])
  return lines.map((l) => l.map(cell).join(',')).join('\r\n') + '\r\n'
}

/** Dateiname mit den Filtern der Ansicht, z. B. `verbrauch_2026-09-20_2026-10-03_nach-session.csv`. */
export function csvFilename(dim: Dim, period: Pick<Period, 'from' | 'to'>): string {
  const slug = DIMS.find((d) => d.id === dim)?.slug ?? dim
  return `verbrauch_${period.from}_${period.to}_nach-${slug}.csv`
}

// ------------------------------------------------------------------ Kennzahlen

export interface Kpis {
  today: { tokens: number; turns: number }
  week: { tokens: number; turns: number }
  month: { tokens: number; days: number }
  /** Kosten der letzten 7 Tage (nur API-Key, Gateway, lokal). */
  cost7: number
}

/** Bereich der Kennzahlen-Abfrage: der frühere Beginn von 7 Tagen und Monat bis heute. */
export function kpiRange(now: Date): { from: string; to: string } {
  const today = localDate(now)
  const week = addDays(today, -6)
  const month = localDate(new Date(now.getFullYear(), now.getMonth(), 1))
  return { from: week < month ? week : month, to: today }
}

/** Kennzahlen aus `group_by=day` (heute, 7 Tage, Monat). */
export function kpis(dayGroups: UsageItem[], now: Date): Kpis {
  const today = localDate(now)
  const week = addDays(today, -6)
  const month = localDate(new Date(now.getFullYear(), now.getMonth(), 1)).slice(0, 7)
  const k: Kpis = { today: { tokens: 0, turns: 0 }, week: { tokens: 0, turns: 0 }, month: { tokens: 0, days: now.getDate() }, cost7: 0 }
  for (const g of dayGroups) {
    const day = g.day ?? ''
    const t = tokensOf(g)
    if (day === today) {
      k.today.tokens += t
      k.today.turns += g.turns
    }
    if (day >= week && day <= today) {
      k.week.tokens += t
      k.week.turns += g.turns
      if (g.billing === 'metered') k.cost7 += g.cost_usd ?? 0
    }
    if (day.slice(0, 7) === month) k.month.tokens += t
  }
  return k
}

// ------------------------------------------------------------------ Zeitreihe

export interface SeriesDay {
  day: string
  tokens: Record<string, number>
  total: number
  cost: number
}

/** Tokens je Tag und Harness sowie Kosten je Tag aus `group_by=day,harness`. */
export function series(groups: UsageItem[], days: string[]): { harnesses: string[]; days: SeriesDay[] } {
  const byDay = new Map(days.map((d) => [d, { day: d, tokens: {} as Record<string, number>, total: 0, cost: 0 }]))
  const totals = new Map<string, number>()
  for (const g of groups) {
    const d = byDay.get(g.day ?? '')
    if (!d || !g.harness) continue
    const t = tokensOf(g)
    d.tokens[g.harness] = (d.tokens[g.harness] ?? 0) + t
    d.total += t
    if (g.billing === 'metered') d.cost += g.cost_usd ?? 0
    totals.set(g.harness, (totals.get(g.harness) ?? 0) + t)
  }
  const harnesses = [...totals.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).map(([h]) => h)
  return { harnesses, days: [...byDay.values()] }
}

/** Achsenmaximum (1, 2, 2,5, 5 × 10ⁿ) und vier gleichmäßige Striche. */
export function axis(max: number): number[] {
  if (max <= 0) return [0, 1, 2, 3, 4]
  const rough = max / 4
  const pow = 10 ** Math.floor(Math.log10(rough))
  const step = [1, 2, 2.5, 5, 10].map((m) => m * pow).find((s) => s >= rough) ?? 10 * pow
  return [0, 1, 2, 3, 4].map((i) => i * step)
}

// ------------------------------------------------------------------ Abo-Fenster

export interface SubscriptionRow {
  harness: string
  windows: SubscriptionWindowView[]
  /** Der Harness meldet Abo-Fenster (USE-004 AC4); `false` = „nicht gemeldet“. */
  reportsLimits: boolean
  /** Alle gemeldeten Fenster sind abgelaufen: Das Kontingent ist zurückgesetzt. */
  expired: boolean
  todayTokens: number
  periodTokens: number
  /** API-Äquivalent von heute in USD (nur informativ); `null`, wenn keins gemeldet ist. */
  todayEquivalentUsd: number | null
}

const WINDOW_ORDER = ['5h', '7d']

/** Harnesses mit Subscription-Login im Zeitraum oder mit gemeldeten Abo-Fenstern. */
export function subscriptionRows(
  windows: SubscriptionWindowView[],
  reporting: HarnessReportingView[],
  dayHarnessGroups: UsageItem[],
  today: string,
): SubscriptionRow[] {
  const rows = new Map<string, SubscriptionRow>()
  const row = (harness: string) => {
    let r = rows.get(harness)
    if (!r) {
      r = {
        harness,
        windows: [],
        reportsLimits: reporting.find((x) => x.harness === harness)?.rate_limits ?? false,
        expired: false,
        todayTokens: 0,
        periodTokens: 0,
        todayEquivalentUsd: null,
      }
      rows.set(harness, r)
    }
    return r
  }
  for (const g of dayHarnessGroups) {
    if (g.billing !== 'subscription' || !g.harness) continue
    const r = row(g.harness)
    r.periodTokens += tokensOf(g)
    if (g.day === today) {
      r.todayTokens += tokensOf(g)
      if (g.equivalent_usd !== null) r.todayEquivalentUsd = (r.todayEquivalentUsd ?? 0) + g.equivalent_usd
    }
  }
  for (const w of windows) {
    const r = row(w.harness)
    r.reportsLimits = true
    if (!w.expired) r.windows.push(w)
  }
  for (const r of rows.values()) r.expired = r.windows.length === 0 && windows.some((w) => w.harness === r.harness && w.expired)
  const rank = (w: string) => (WINDOW_ORDER.includes(w) ? WINDOW_ORDER.indexOf(w) : WINDOW_ORDER.length)
  for (const r of rows.values()) r.windows.sort((a, b) => rank(a.window) - rank(b.window) || a.window.localeCompare(b.window))
  return [...rows.values()].sort((a, b) => b.periodTokens - a.periodTokens || a.harness.localeCompare(b.harness))
}

/** Bezeichnung eines Fensters wie im Entwurf. */
export function windowLabel(window: string): string {
  if (window === '5h') return '5-Stunden-Fenster'
  if (window === '7d') return 'Wochenlimit'
  return `Fenster ${window}`
}

const hhmm = (d: Date) => d.toLocaleTimeString('de-DE', { hour: '2-digit', minute: '2-digit' })

/** Reset in Nutzer-Zeitzone (USE-004 AC2): „setzt in 41 Min. zurück (14:46)“ bzw. „setzt Mo. 09:00 zurück“. */
export function resetText(resetsAt: string, now: Date): string {
  const at = new Date(resetsAt)
  const min = Math.max(0, Math.round((at.getTime() - now.getTime()) / 60_000))
  if (min < 24 * 60) {
    const h = Math.floor(min / 60)
    const m = min % 60
    const rel = h > 0 ? `${h} Std. ${m} Min.` : `${m} Min.`
    return `setzt in ${rel} zurück (${hhmm(at)})`
  }
  if (min < 6 * 24 * 60) return `setzt ${at.toLocaleDateString('de-DE', { weekday: 'short' }).replace('.', '')}. ${hhmm(at)} zurück`
  return `setzt ${at.getDate()}.${at.getMonth() + 1}. ${hhmm(at)} zurück`
}

/** Uhrzeit des Resets (für den Hinweis bei fast aufgebrauchtem Fenster). */
export function resetClock(resetsAt: string): string {
  return hhmm(new Date(resetsAt))
}
