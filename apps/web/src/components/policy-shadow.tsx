import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import type { ShadowReport, ShadowRule } from '@beton/sdk'
import { Menu } from 'lucide-react'
import { client } from '@/lib/client'
import { ago } from '@/lib/format'
import { cn } from '@/lib/utils'
import { useLayout } from './layout-state'

/*
 * Shadow-Modus (POL-027 AC4) nach dem abgenommenen Entwurf (`design/prototype`, Screen
 * `policy-shadow`): Regeln im Schattenbetrieb mit ihren Treffern je Tag als „würde ablehnen“,
 * ohne Sessions zu stören. Abweichung vom Entwurf: „Scharf schalten“ fehlt, weil es einen
 * schreibenden Policy-Endpunkt braucht (eigenes Issue); die Ansicht steht unter
 * `/policies/shadow` statt in den Einstellungen, die es in der Web-UI noch nicht gibt.
 */

/** Abstand der Abfragen; neue Treffer erscheinen ohne Neuladen. */
export const SHADOW_POLL_MS = 5000

const SCOPES: Record<string, string> = { org: 'Org', team: 'Team', user: 'User', project: 'Projekt', agent: 'Agent' }

const VERDICTS: Record<string, string> = {
  deny: 'würde ablehnen',
  ask: 'würde nachfragen',
  modify: 'würde ändern',
  allow: 'würde erlauben',
  notify: 'würde melden',
}

/** Tageskopf: Wochentag, der letzte Tag heißt „heute“. */
export function dayLabel(day: string, last: boolean): string {
  if (last) return 'heute'
  const d = new Date(`${day}T12:00:00Z`)
  return d.toLocaleDateString('de-DE', { weekday: 'short', timeZone: 'UTC' }).replace('.', '')
}

/** Form und Text, nie nur Farbe: gestrichelte Raute für „würde ablehnen“. */
function WouldMark({ verdict }: { verdict: string }) {
  return <span className={cn('size-2 shrink-0 rotate-45 border border-dashed', verdict === 'deny' ? 'border-deny' : 'border-foreground/60')} aria-hidden />
}

function Bars({ rule, days, max }: { rule: ShadowRule; days: string[]; max: number }) {
  return (
    <>
      {rule.counts.map((n, i) => (
        <span
          key={days[i]}
          role="cell"
          className="flex h-7 flex-col items-center justify-end gap-0.5"
          title={`${n} Treffer am ${days[i]}`}
          data-testid="shadow-count"
        >
          <span className="w-4 bg-foreground/60" style={{ height: `${max ? (n / max) * 18 : 0}px` }} />
          <span className="text-[10px] text-muted-foreground tabular-nums">{n}</span>
        </span>
      ))}
    </>
  )
}

function Last({ rule }: { rule: ShadowRule }) {
  const last = rule.last
  if (!last)
    return (
      <span role="cell" className="text-[11px] text-muted-foreground">
        –
      </span>
    )
  return (
    <span role="cell" className="truncate font-mono text-[11px] text-muted-foreground" title={last.summary ?? undefined}>
      {last.summary ? `${last.summary} · ` : ''}
      <Link to="/s/$sessionId" params={{ sessionId: last.session_id }} className="underline-offset-2 hover:underline">
        {last.session_title || last.session_id}
      </Link>
      {` · ${ago(last.at)}`}
    </span>
  )
}

function Table({ report }: { report: ShadowReport }) {
  const max = Math.max(1, ...report.rules.flatMap((r) => r.counts))
  // Spaltenzahl hängt von den Tagen ab: als Style, nicht als (statische) Tailwind-Klasse.
  const cols = { gridTemplateColumns: `minmax(10rem,200px) minmax(8rem,160px) repeat(${report.days.length},44px) minmax(12rem,1fr)` }
  return (
    <div className="overflow-x-auto rounded-md border border-border bg-card">
      <div className="min-w-max" role="table" aria-label="Shadow-Treffer je Regel und Tag">
        <div className="grid items-end gap-2 border-b border-border bg-sunken px-3 py-1.5 text-[11px] text-muted-foreground" style={cols} role="row">
          <span role="columnheader">Regel</span>
          <span role="columnheader">Set</span>
          {report.days.map((d, i) => (
            <span key={d} role="columnheader" className="text-center" title={d}>
              {dayLabel(d, i === report.days.length - 1)}
            </span>
          ))}
          <span role="columnheader">Letzter Treffer</span>
        </div>
        {report.rules.map((r) => (
          <div
            key={`${r.scope}/${r.set}/${r.rule_id}/${r.verdict}`}
            className="grid items-center gap-2 border-b border-border px-3 py-2 text-[12px] last:border-b-0"
            style={cols}
            role="row"
            data-testid="shadow-rule"
          >
            <span role="cell" className="flex min-w-0 items-center gap-2">
              <WouldMark verdict={r.verdict} />
              <span className="min-w-0">
                <span className="block truncate font-mono">{r.rule_id}</span>
                <span className={cn('block text-[11px]', r.verdict === 'deny' ? 'text-deny' : 'text-muted-foreground')}>{VERDICTS[r.verdict] ?? `würde ${r.verdict}`}</span>
              </span>
            </span>
            <span role="cell" className="truncate text-muted-foreground">
              {r.set} ({SCOPES[r.scope] ?? r.scope})
            </span>
            <Bars rule={r} days={report.days} max={max} />
            <Last rule={r} />
          </div>
        ))}
      </div>
    </div>
  )
}

export function PolicyShadowView() {
  const toggleList = useLayout((s) => s.toggleList)
  const q = useQuery({ queryKey: ['policies', 'shadow'], queryFn: () => client.policies.shadow(7), refetchInterval: SHADOW_POLL_MS })
  const report = q.data
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex min-h-12 flex-wrap items-center gap-x-3 gap-y-1 border-b border-border px-3 py-2 sm:px-6">
        <button onClick={toggleList} className="flex size-8 items-center justify-center rounded-md hover:bg-accent lg:hidden" aria-label="Sessions anzeigen">
          <Menu className="size-4" />
        </button>
        <div className="min-w-0">
          <h1 className="type-wide text-xl font-[700]">Shadow-Modus</h1>
          <p className="text-[13px] text-muted-foreground">Neue Regeln gefahrlos ausprobieren: Sie werden ausgewertet und protokolliert, blockieren aber nichts.</p>
        </div>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto px-3 py-4 sm:px-6">
        {q.isError && (
          <div className="rounded-md border border-deny/40 bg-deny-soft px-3 py-2 text-[13px]" role="alert">
            Shadow-Treffer nicht geladen: {q.error instanceof Error ? q.error.message : 'unbekannter Fehler'}. Läuft der Daemon (<code>beton serve</code>)?
          </div>
        )}
        {q.isPending && <p className="text-[13px] text-muted-foreground">Lädt …</p>}
        {report && report.rules.length === 0 && (
          <div className="concrete-grain flex flex-col items-center gap-2 rounded-md border border-dashed border-border px-6 py-12 text-center" data-testid="shadow-empty">
            <p className="type-wide text-[15px] font-semibold">Keine Regel im Shadow-Modus</p>
            <p className="max-w-md text-[13px] text-muted-foreground">
              Setze in einer Policy <span className="font-mono">mode: dry_run</span> oder an einer Regel <span className="font-mono">dry_run: true</span>. Treffer
              erscheinen dann hier als „würde ablehnen“, ohne eine Session zu stören.
            </p>
          </div>
        )}
        {report && report.rules.length > 0 && (
          <>
            <Table report={report} />
            <p className="mt-3 text-[12px] text-muted-foreground">
              Shadow-Treffer stehen als <span className="font-mono">dry_run: true</span> im <span className="font-mono">policy.decision</span>-Ereignis und
              beeinflussen das Ergebnis nicht. Tage in UTC. Scharf schaltest du eine Regel, indem du in ihrer Datei <span className="font-mono">mode</span> auf
              <span className="font-mono"> enforce</span> setzt bzw. <span className="font-mono">dry_run</span> entfernst; prüfen kannst du sie vorher mit{' '}
              <span className="font-mono">beton policy test</span>.
            </p>
          </>
        )}
      </div>
    </div>
  )
}
