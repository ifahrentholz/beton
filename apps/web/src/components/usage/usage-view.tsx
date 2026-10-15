import { useMemo, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Link, useNavigate } from '@tanstack/react-router'
import { Download, Info } from 'lucide-react'
import { client } from '@/lib/client'
import { cn } from '@/lib/utils'
import {
  authLabel,
  csvFilename,
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

function BillingCell({ row }: { row: UsageRow }) {
  const b = row.billing
  switch (b.kind) {
    case 'subscription':
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

function SubscriptionLine({ row, rows, periodLabel, now }: { row: SubscriptionRow; rows: SubscriptionRow[]; periodLabel: string; now: Date }) {
  const high = row.windows.find((w) => w.used_pct >= 90)
  const other = rows.find((r) => r.harness !== row.harness && r.windows.every((w) => w.used_pct < 90))
  const cli = LOCAL_CLI[row.harness]
  return (
    <div className={cn('grid grid-cols-1 gap-4 px-4 py-3 sm:grid-cols-[230px_1fr]', row.windows.length === 0 && 'items-center', high && 'bg-warn-soft/60')} data-testid="subscription-row">
      <div>
        <HarnessBadge harness={row.harness} />
        {cli && <div className="mt-0.5 text-[11px] text-muted-foreground">angemeldet über {cli}</div>}
        {row.windows.length > 0 && <div className="mt-1 text-[11px] text-muted-foreground tabular-nums">heute {fmtTokens(row.todayTokens)} Tokens</div>}
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
      ) : (
        <div className="text-[12px] text-muted-foreground">
          {/* USE-004 AC4: ohne Rate-Limit-Daten „nicht gemeldet“ statt 0. */}
          Kontingent nicht gemeldet – {harnessName(row.harness)} liefert keine Rate-Limit-Daten. {fmtTokens(row.periodTokens)} Tokens in {periodLabel}.
        </div>
      )}
    </div>
  )
}

export function UsageView() {
  const navigate = useNavigate()
  const now = useMemo(() => new Date(), [])
  const today = localDate(now)
  const tz = tzOffset(now)
  const options = useMemo(() => periods(now), [now])
  const [periodId, setPeriodId] = useState<PeriodId>('14d')
  const period = options.find((p) => p.id === periodId) ?? options[2]!
  const [dim, setDim] = useState<Dim>('session')
  const [stack, setStack] = useState<'tokens' | 'cost'>('tokens')
  const range = kpiRange(now)

  const common = { refetchInterval: USAGE_REFRESH_MS, staleTime: 2_000 }
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
  const periodLabel = period.id === 'today' ? 'heute' : period.id === 'month' ? period.label : `${days.length} Tagen`
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
          <Segmented label="Zeitraum" value={period.id} onChange={(v) => setPeriodId(v as PeriodId)} options={options.map((p) => ({ id: p.id, label: p.label }))} />
          <button
            type="button"
            onClick={() => download(filename, toCsv(effectiveDim, rows))}
            disabled={!tableQuery.data}
            className="inline-flex h-8 items-center gap-1.5 rounded-md border border-border px-3 text-[13px] hover:bg-accent disabled:opacity-45"
            title={filename}
          >
            <Download className="size-3.5" /> CSV exportieren
          </button>
        </div>

        {failed && (
          <div className="mt-4 border border-deny/40 bg-deny-soft px-4 py-1.5 text-[12px]" role="alert">
            {failed instanceof Error ? failed.message : 'Verbrauch nicht verfügbar'}
          </div>
        )}

        {k && (
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

        {chartQuery.data && subs.length > 0 && (
          <section className="mt-6" aria-label="Subscriptions">
            <div className="flex flex-wrap items-baseline gap-3">
              <h2 className="text-[14px] font-semibold">Subscriptions</h2>
              <span className="text-[12px] text-muted-foreground">Kontingente, wie die CLIs sie melden – kein Geldbetrag</span>
            </div>
            <div className="mt-2 divide-y divide-border rounded-md border border-border">
              {subs.map((r) => (
                <SubscriptionLine key={r.harness} row={r} rows={subs} periodLabel={periodLabel} now={now} />
              ))}
            </div>
          </section>
        )}

        {chartQuery.data && (
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

        {tableQuery.data && (
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
                        <BillingCell row={r} />
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </section>
        )}

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
      </div>
    </div>
  )
}
