import { useMemo, useRef, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Link, useNavigate } from '@tanstack/react-router'
import { Download, Info } from 'lucide-react'
import { client } from '@/lib/client'
import { useT } from '@/lib/i18n'
import { cn } from '@/lib/utils'
import {
  authLabel,
  csvFilename,
  customPeriod,
  DIMS,
  dollars,
  fmtTokens,
  groupByFor,
  kpiRange,
  kpis,
  localDate,
  periods,
  resetClock,
  resetText,
  rowsFor,
  series,
  subscriptionRows,
  toCsv,
  tzOffset,
  windowLabel,
  daysBetween,
  type Dim,
  type PeriodId,
  type SubscriptionRow,
  type UsageRow,
} from '@/lib/usage'
import { HarnessBadge, harnessName, VoiceDot, voiceOf } from '../harness'
import { rich } from '../kbd'
import { Switch } from '../settings/kit'
import { Kpi, QuotaBar, Segmented, StackedBars } from './parts'

/*
 * Verbrauch (USE-006) nach dem abgenommenen Entwurf `usage-overview`: Zeitraum, Kennzahlen,
 * Subscriptions mit Abo-Fenstern (USE-004, nie Geld), Verlauf gestapelt nach Harness und die
 * Aufschlüsselung mit CSV-Export. Alle Zahlen aus `GET /v1/usage`.
 */

/** Wie oft die Seite neue Turns nachlädt. */
export const USAGE_REFRESH_MS = 10_000

const LOCAL_CLI: Record<string, string> = { claude: 'claude-CLI', codex: 'codex-CLI' }

function download(name: string, text: string) {
  const url = URL.createObjectURL(new Blob([text], { type: 'text/csv;charset=utf-8' }))
  const a = document.createElement('a')
  a.href = url
  a.download = name
  document.body.append(a)
  a.click()
  a.remove()
  setTimeout(() => URL.revokeObjectURL(url), 0)
}

/** API-Äquivalent einer Subscription, nur informativ (USE-004 AC1, ADR-0034): nie als Kosten. */
function Equivalent({ usd, version }: { usd: number; version: string }) {
  const t = useT()
  return (
    <div className="text-[11px] text-muted-foreground tabular-nums" title={t('usage.equivalentTitle', { version })}>
      {t('usage.equivalent', { amount: dollars(usd) })}
    </div>
  )
}

function BillingCell({ row, equivalent }: { row: UsageRow; equivalent: string | undefined }) {
  const t = useT()
  const b = row.billing
  switch (b.kind) {
    case 'subscription':
      if (equivalent && row.equivalentUsd !== undefined) {
        return (
          <span className="text-muted-foreground tabular-nums" title={t('usage.equivalentTitle', { version: equivalent })}>
            Subscription · <span className="text-[11px]">{t('usage.equivalentShort', { amount: dollars(row.equivalentUsd) })}</span>
          </span>
        )
      }
      return <span className="text-muted-foreground">Subscription</span>
    case 'cost':
      return <span className="tabular-nums">{dollars(b.usd)}</span>
    case 'unpriced':
      // USE-006 AC4: Turns ohne Preis mit Weg zu den eigenen Preisen.
      return (
        <Link to="/usage/pricing" className="text-[12px] underline underline-offset-2" data-testid="unpriced-link">
          ohne Preis – eigenen Preis anlegen
        </Link>
      )
    default:
      return <span className="text-muted-foreground">kostenlos</span>
  }
}

function RowName({ dim, row }: { dim: Dim; row: UsageRow }) {
  if (dim === 'harness') return <HarnessBadge harness={row.key} />
  if (dim === 'model') return <span className="font-mono text-[12px]">{row.name}</span>
  if (dim === 'project') return <span className={cn(row.key && 'font-mono text-[12px]')}>{row.name}</span>
  return <>{row.name}</>
}

function RowSub({ dim, row }: { dim: Dim; row: UsageRow }) {
  if (dim === 'session' && row.harnesses.length > 0) {
    return (
      <span className="flex flex-wrap gap-2">
        {row.harnesses.map((h) => (
          <HarnessBadge key={h} harness={h} />
        ))}
      </span>
    )
  }
  if (dim === 'harness' && row.authSources.length > 0) {
    return <span className="text-[11px] text-muted-foreground">{row.authSources.map(authLabel).join(', ')}</span>
  }
  return null
}

function SubscriptionLine({
  row,
  rows,
  periodLabel,
  now,
  equivalent,
}: {
  row: SubscriptionRow
  rows: SubscriptionRow[]
  periodLabel: string
  now: Date
  /** Katalogversion, wenn das API-Äquivalent eingeblendet ist. */
  equivalent: string | undefined
}) {
  const t = useT()
  const high = row.windows.find((w) => w.used_pct >= 90)
  const other = rows.find((r) => r.harness !== row.harness && r.windows.every((w) => w.used_pct < 90))
  const cli = LOCAL_CLI[row.harness]
  const reported = row.windows.length > 0 || row.expired
  // Ohne Nutzung heute ist das Äquivalent 0; ohne Preis bleibt es weg.
  const todayEq = row.todayEquivalentUsd ?? (row.todayTokens === 0 ? 0 : null)
  return (
    <div className={cn('grid grid-cols-1 gap-4 px-4 py-3 sm:grid-cols-[230px_1fr]', row.windows.length === 0 && 'items-center', high && 'bg-warn-soft/60')} data-testid="subscription-row">
      <div>
        <HarnessBadge harness={row.harness} />
        {cli && <div className="mt-0.5 text-[11px] text-muted-foreground">angemeldet über {cli}</div>}
        {reported && <div className="mt-1 text-[11px] text-muted-foreground tabular-nums">heute {fmtTokens(row.todayTokens)} Tokens</div>}
        {reported && equivalent && todayEq !== null && <Equivalent usd={todayEq} version={equivalent} />}
      </div>
      {row.windows.length > 0 ? (
        <div>
          {row.windows.map((w) => (
            <QuotaBar key={w.window} label={windowLabel(w.window)} pct={w.used_pct} reset={resetText(w.resets_at, now)} warn />
          ))}
          {high && (
            <p className="mt-1 text-[12px]">
              Neue Turns mit {harnessName(row.harness)} können gleich abgelehnt werden.{' '}
              {other
                ? `Du kannst mit ${harnessName(other.harness)} weiterarbeiten (Session forken) oder bis ${resetClock(high.resets_at)} warten.`
                : `Du kannst bis ${resetClock(high.resets_at)} warten.`}{' '}
              Ein Hinweis liegt in der Inbox.
            </p>
          )}
        </div>
      ) : row.expired ? (
        // Reset-Zeitpunkt vorbei (Entwurf `usage-overview`, Zustand `window-expired`): kein alter Balken.
        <div className="text-[12px] text-muted-foreground">{t('usage.windowReset', { tokens: fmtTokens(row.periodTokens), period: periodLabel })}</div>
      ) : (
        <div className="text-[12px] text-muted-foreground">
          {/* USE-004 AC4: ohne Rate-Limit-Daten „nicht gemeldet“ statt 0. */}
          Kontingent nicht gemeldet – {harnessName(row.harness)} liefert keine Rate-Limit-Daten. {fmtTokens(row.periodTokens)} Tokens in {periodLabel}.
        </div>
      )}
    </div>
  )
}

/** Popover „Eigener Zeitraum“ an der Zeitraumwahl (USE-006, Entwurf `usage-overview`, Zustand `custom-range`). */
function RangePopover({
  initial,
  max,
  onCancel,
  onApply,
}: {
  initial: { from: string; to: string }
  max: string
  onCancel: () => void
  onApply: (r: { from: string; to: string }) => void
}) {
  const t = useT()
  const [from, setFrom] = useState(initial.from)
  const [to, setTo] = useState(initial.to)
  const swapped = !!from && !!to && from > to
  const invalid = !from || !to || swapped
  const field = 'h-8 w-full rounded-md border border-input bg-card px-2 text-[13px] text-foreground tabular-nums'
  return (
    <div
      role="dialog"
      aria-labelledby="usage-range-title"
      className="absolute top-full right-0 z-20 mt-1 w-[300px] rounded-md border border-border bg-popover p-3 text-[13px] text-popover-foreground shadow-lg"
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          e.stopPropagation()
          onCancel()
        }
      }}
    >
      <div id="usage-range-title" className="font-semibold">
        {t('usage.rangeTitle')}
      </div>
      <div className="mt-2 grid grid-cols-2 gap-2">
        <label className="flex flex-col gap-1 text-[12px] text-muted-foreground">
          {t('usage.from')}
          <input autoFocus type="date" value={from} max={max} onChange={(e) => setFrom(e.target.value)} className={field} />
        </label>
        <label className="flex flex-col gap-1 text-[12px] text-muted-foreground">
          {t('usage.to')}
          <input type="date" value={to} max={max} onChange={(e) => setTo(e.target.value)} className={field} />
        </label>
      </div>
      <p className="mt-2 text-[12px] text-muted-foreground">{t('usage.rangeHint')}</p>
      {swapped && <p className="mt-1 text-[12px] text-deny">{t('usage.rangeInvalid')}</p>}
      <div className="mt-3 flex justify-end gap-2">
        <button type="button" onClick={onCancel} className="inline-flex h-8 items-center rounded-md border border-border px-3 text-[13px] hover:bg-accent">
          {t('usage.cancel')}
        </button>
        <button
          type="button"
          disabled={invalid}
          onClick={() => onApply({ from, to })}
          className="inline-flex h-8 items-center rounded-md border border-foreground bg-foreground px-3 text-[13px] font-semibold text-background disabled:cursor-not-allowed disabled:opacity-50"
        >
          {t('usage.apply')}
        </button>
      </div>
    </div>
  )
}

/** Fehler der Abfrage: ohne Antwort vom Daemon dessen Hinweis, sonst die Meldung des Servers. */
function UsageError({ error, onRetry, compact }: { error: unknown; onRetry: () => void; compact?: boolean }) {
  const t = useT()
  const detail = (error as { problem?: { detail?: string; title?: string } }).problem
  const title = <strong className="font-semibold">{t('usage.unavailable')}</strong>
  const text = detail
    ? rich(t('usage.failed'), { title, detail: detail.detail ?? detail.title ?? '' })
    : rich(t('usage.daemonDown'), { title, cmd: <code className="font-mono text-[12px]">beton serve</code> })
  return (
    <div role="alert" className={cn('rounded-md border border-deny/40 bg-deny-soft p-3 text-[13px]', compact ? 'mt-4' : 'mt-5')}>
      <p className="flex items-center gap-2">
        <span aria-hidden className="size-2 shrink-0 rotate-45 bg-deny" />
        <span>{text}</span>
      </p>
      <button type="button" onClick={onRetry} className="mt-2 ml-4 inline-flex h-8 items-center rounded-md border border-border bg-card px-3 text-[13px] hover:bg-accent">
        {t('usage.retry')}
      </button>
    </div>
  )
}

export function UsageView() {
  const navigate = useNavigate()
  const now = useMemo(() => new Date(), [])
  const today = localDate(now)
  const tz = tzOffset(now)
  const t = useT()
  const options = useMemo(() => periods(now), [now])
  const [periodId, setPeriodId] = useState<PeriodId>('14d')
  // Eigener Zeitraum: zuletzt angewandt; `fixed` ist der vorige feste Zeitraum (Abbrechen).
  const [custom, setCustom] = useState<{ from: string; to: string } | null>(null)
  const [fixed, setFixed] = useState<PeriodId>('14d')
  const [rangeOpen, setRangeOpen] = useState(false)
  const rangeAnchor = useRef<HTMLDivElement>(null)
  const period =
    periodId === 'custom' && custom ? customPeriod(custom.from, custom.to) : (options.find((p) => p.id === (periodId === 'custom' ? fixed : periodId)) ?? options[2]!)
  const [dim, setDim] = useState<Dim>('session')
  const [stack, setStack] = useState<'tokens' | 'cost'>('tokens')
  const [equivalent, setEquivalent] = useState(false)
  const range = kpiRange(now)
  const closeRange = () => {
    setRangeOpen(false)
    // Fokus zurück an die Zeitraumwahl.
    requestAnimationFrame(() => rangeAnchor.current?.querySelector<HTMLElement>('[role="radio"][aria-checked="true"]')?.focus())
  }

  // Beim Wechsel des Zeitraums bleiben die alten Zahlen stehen, bis die neuen da sind.
  const common = { refetchInterval: USAGE_REFRESH_MS, staleTime: 2_000, placeholderData: <T,>(prev: T | undefined) => prev }
  const kpiQuery = useQuery({
    queryKey: ['usage', 'kpi', range.from, range.to, tz],
    queryFn: () => client.usage.report({ from: range.from, to: range.to, tz, groupBy: ['day'] }),
    ...common,
  })
  const chartQuery = useQuery({
    queryKey: ['usage', 'chart', period.from, period.to, tz],
    queryFn: () => client.usage.report({ from: period.from, to: period.to, tz, groupBy: ['day', 'harness'] }),
    ...common,
  })
  const tableQuery = useQuery({
    queryKey: ['usage', 'table', dim, period.from, period.to, tz],
    queryFn: () => client.usage.report({ from: period.from, to: period.to, tz, groupBy: groupByFor(dim) }),
    ...common,
  })

  const k = kpiQuery.data ? kpis(kpiQuery.data.groups, now) : undefined
  const days = useMemo(() => daysBetween(period.from, period.to), [period.from, period.to])
  const chart = useMemo(() => series(chartQuery.data?.groups ?? [], days), [chartQuery.data, days])
  const rows = useMemo(() => rowsFor(dim, tableQuery.data?.groups ?? [], today), [dim, tableQuery.data, today])
  const subs = useMemo(
    () => (chartQuery.data ? subscriptionRows(chartQuery.data.subscriptions, chartQuery.data.reporting, chartQuery.data.groups, today) : []),
    [chartQuery.data, today],
  )
  const hasCost = chart.days.some((d) => d.cost > 0)
  const mode = hasCost ? stack : 'tokens'
  const info = useQuery({ queryKey: ['info'], queryFn: () => client.info(), staleTime: 60_000 })
  const failed = kpiQuery.error ?? chartQuery.error ?? tableQuery.error
  const loaded = kpiQuery.data && chartQuery.data && tableQuery.data
  const retry = () => {
    void kpiQuery.refetch()
    void chartQuery.refetch()
    void tableQuery.refetch()
  }
  const periodLabel = period.id === 'today' ? 'heute' : period.id === 'month' ? period.label : `${days.length} Tagen`
  const catalog = chartQuery.data?.pricing_version ?? ''
  const shownEquivalent = equivalent ? catalog || '–' : undefined
  const empty = loaded && kpiQuery.data.groups.length === 0 && chartQuery.data.groups.length === 0 && chartQuery.data.subscriptions.length === 0

  if (empty) {
    return (
      <div className="concrete-grain flex flex-1 flex-col items-center justify-center gap-2 p-8 text-center">
        <p className="type-wide text-xl font-[700]">Noch kein Verbrauch</p>
        <p className="max-w-md text-sm text-muted-foreground">
          Sobald eine Session läuft, siehst du hier Tokens und die Kontingente deiner Subscriptions. Kosten in Dollar erscheinen nur, wenn du
          einen API-Key oder ein Gateway nutzt.
        </p>
      </div>
    )
  }

  const effectiveDim = DIMS.find((d) => d.id === dim)?.central ? 'session' : dim
  const filename = csvFilename(effectiveDim, period)

  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto max-w-6xl px-4 py-6 sm:px-8">
        <div className="flex flex-wrap items-center gap-3 border-b border-border pb-4">
          <h1 className="type-wide mr-auto text-xl font-[700]">Verbrauch</h1>
          <div className="relative" ref={rangeAnchor}>
            <Segmented
              label="Zeitraum"
              value={periodId}
              onChange={(v) => {
                if (v === 'custom') {
                  setPeriodId('custom')
                  setRangeOpen(true)
                  return
                }
                setPeriodId(v as PeriodId)
                setFixed(v as PeriodId)
                setRangeOpen(false)
              }}
              options={[
                ...options.map((p) => ({ id: p.id, label: p.label })),
                custom
                  ? { id: 'custom', label: customPeriod(custom.from, custom.to).label, title: t('usage.rangeTooltip', { range: customPeriod(custom.from, custom.to).label }) }
                  : { id: 'custom', label: t('usage.custom') },
              ]}
            />
            {rangeOpen && (
              <RangePopover
                initial={custom ?? { from: period.from, to: period.to }}
                max={today}
                onCancel={() => {
                  closeRange()
                  if (!custom) setPeriodId(fixed)
                }}
                onApply={(r) => {
                  setCustom(r)
                  closeRange()
                }}
              />
            )}
          </div>
          <button
            type="button"
            onClick={() => download(filename, toCsv(effectiveDim, rows))}
            disabled={!tableQuery.data}
            className="inline-flex h-8 items-center gap-1.5 rounded-md border border-border px-3 text-[13px] enabled:hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
            title={tableQuery.data ? filename : t('usage.csvWait')}
          >
            <Download className="size-3.5" /> CSV exportieren
          </button>
        </div>

        {/* Ohne Daten: Fehler mit „Erneut laden“ statt der Seite; danach als Hinweis über den Zahlen. */}
        {failed && <UsageError error={failed} onRetry={retry} compact={!!loaded} />}

        {!loaded && !failed && (
          // Lädt (Entwurf `usage-overview`, Zustand `loading`): ruhige Platzhalter statt Skeletons.
          <>
            <div className="mt-5 grid grid-cols-2 gap-4 sm:grid-cols-4" data-testid="usage-kpis" aria-busy="true">
              {['Heute', 'Letzte 7 Tage', options[3]!.label, 'Kosten'].map((label) => (
                <Kpi key={label} label={label} value={<span className="text-muted-foreground">–</span>} />
              ))}
            </div>
            <p role="status" className="mt-5 text-[13px] text-muted-foreground">
              {t('usage.loading')}
            </p>
          </>
        )}

        {loaded && k && (
          <div className="mt-5 grid grid-cols-2 gap-4 sm:grid-cols-4" data-testid="usage-kpis">
            <Kpi label="Heute" value={fmtTokens(k.today.tokens)} sub={`Tokens · ${k.today.turns} Turns`} />
            <Kpi label="Letzte 7 Tage" value={fmtTokens(k.week.tokens)} sub={`Tokens · ${k.week.turns} Turns`} />
            <Kpi label={options[3]!.label} value={fmtTokens(k.month.tokens)} sub={`Tokens · ${k.month.days} ${k.month.days === 1 ? 'Tag' : 'Tage'}`} />
            {k.cost7 > 0 ? (
              <Kpi label="Kosten 7 Tage" value={dollars(k.cost7)} sub="nur API-Key & Gateway" />
            ) : (
              <Kpi label="Kosten" value="keine" sub="alle Sessions über Subscriptions oder lokal" />
            )}
          </div>
        )}

        {loaded && subs.length > 0 && (
          <section className="mt-6" aria-label="Subscriptions">
            <div className="flex flex-wrap items-baseline gap-3">
              <h2 className="text-[14px] font-semibold">Subscriptions</h2>
              <span className="text-[12px] text-muted-foreground">Kontingente, wie die CLIs sie melden – kein Geldbetrag</span>
            </div>
            <div className="mt-2 divide-y divide-border rounded-md border border-border">
              {subs.map((r) => (
                <SubscriptionLine key={r.harness} row={r} rows={subs} periodLabel={periodLabel} now={now} equivalent={shownEquivalent} />
              ))}
            </div>
            <label className="mt-2 flex items-center gap-2 text-[12px] text-muted-foreground">
              <Switch checked={equivalent} onChange={setEquivalent} label={t('usage.equivalentToggle')} />
              {t('usage.equivalentLabel')}
            </label>
          </section>
        )}

        {loaded && (
          <section className="mt-7" aria-label="Verlauf">
            <div className="flex flex-wrap items-center gap-3">
              <h2 className="text-[14px] font-semibold">Verlauf</h2>
              <Segmented
                label="Größe"
                value={mode}
                onChange={(v) => setStack(v as 'tokens' | 'cost')}
                options={[
                  { id: 'tokens', label: 'Tokens nach Harness' },
                  { id: 'cost', label: 'Kosten (API & Gateway)', disabled: !hasCost, title: hasCost ? undefined : 'Keine Sessions mit API-Key oder Gateway' },
                ]}
              />
              <div className="ml-auto flex flex-wrap gap-4 text-[12px]">
                {chart.harnesses.map((h) => (
                  <span key={h} className="inline-flex items-center gap-1.5">
                    <VoiceDot voice={voiceOf(h)} className="size-2.5 rounded-[2px]" />
                    {harnessName(h)}
                  </span>
                ))}
              </div>
            </div>
            <div className="mt-3">
              <StackedBars mode={mode} days={chart.days} harnesses={chart.harnesses} today={today} />
            </div>
          </section>
        )}

        {loaded && (
          <section className="mt-7" aria-label="Aufschlüsselung">
            <div className="flex flex-wrap items-center gap-3">
              <h2 className="text-[14px] font-semibold">Aufschlüsselung</h2>
              <Segmented
                label="Gruppieren nach"
                value={effectiveDim}
                onChange={(v) => setDim(v as Dim)}
                options={DIMS.map((d) => ({
                  id: d.id,
                  label: d.label,
                  disabled: !!d.central,
                  title: d.central ? 'Nur mit Team-Server (ab M4); lokal gibt es nur dich' : undefined,
                }))}
              />
            </div>
            <div className="mt-2 overflow-x-auto rounded-md border border-border">
              <table className="w-full text-[13px]" data-testid="usage-table">
                <thead className="bg-sunken text-[12px] text-muted-foreground">
                  <tr>
                    <th className="py-2 pl-3 text-left font-medium">{DIMS.find((d) => d.id === effectiveDim)?.label}</th>
                    <th className="px-2 text-right font-medium">Turns</th>
                    <th className="px-2 text-right font-medium">Eingabe</th>
                    <th className="px-2 text-right font-medium">Ausgabe</th>
                    <th className="px-2 text-right font-medium">Cache gelesen</th>
                    <th className="py-2 pr-3 pl-6 text-left font-medium">Abrechnung</th>
                  </tr>
                </thead>
                <tbody className="type-narrow">
                  {rows.map((r) => (
                    <tr
                      key={r.key}
                      data-key={r.key}
                      className={cn('border-t border-border align-top', r.sessionId && 'cursor-pointer hover:bg-accent/50')}
                      onClick={
                        r.sessionId
                          ? (e) => {
                              // Links in der Zeile (Session, „ohne Preis“) führen selbst.
                              if ((e.target as HTMLElement).closest('a, button')) return
                              void navigate({ to: '/s/$sessionId', params: { sessionId: r.sessionId! } })
                            }
                          : undefined
                      }
                    >
                      <td className="py-2 pl-3">
                        {r.sessionId ? (
                          // USE-006 AC2: Klick auf eine Session öffnet sie (auch per Tastatur).
                          <Link to="/s/$sessionId" params={{ sessionId: r.sessionId }} className="font-sans hover:underline">
                            {r.name}
                          </Link>
                        ) : (
                          <div className="font-sans">
                            <RowName dim={effectiveDim} row={r} />
                          </div>
                        )}
                        <div className="mt-0.5 empty:hidden">
                          <RowSub dim={effectiveDim} row={r} />
                        </div>
                      </td>
                      <td className="px-2 py-2 text-right tabular-nums">{r.turns}</td>
                      <td className="px-2 py-2 text-right tabular-nums">{fmtTokens(r.input)}</td>
                      <td className="px-2 py-2 text-right tabular-nums">{fmtTokens(r.output)}</td>
                      <td className="px-2 py-2 text-right tabular-nums">{fmtTokens(r.cache)}</td>
                      <td className="py-2 pr-3 pl-6 text-[12px]">
                        <BillingCell row={r} equivalent={shownEquivalent} />
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </section>
        )}

        {loaded && (
          <div className="mt-4 flex flex-wrap items-center gap-x-4 gap-y-1 border-t border-border pt-3 text-[11px] text-muted-foreground">
            <span className="inline-flex items-center gap-1">
              <Info className="size-3" /> Preise aus dem mitgelieferten Katalog{' '}
              <span className="font-mono">{chartQuery.data?.pricing_version ?? '…'}</span>
              {info.data ? ` (mit beton ${info.data.version}, kein Online-Abruf)` : ' (kein Online-Abruf)'}
            </span>
            <span>Beträge in USD</span>
            <Link to="/usage/pricing" className="underline underline-offset-2">
              Preise & eigene Preise
            </Link>
          </div>
        )}
      </div>
    </div>
  )
}
