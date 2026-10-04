import { ChevronDown, FileCode2, Plus, RefreshCw } from 'lucide-react'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { Btn, Callout, PageHead, Premise, PrimaryButton, Scroll, Shell, Tag, Verdict, type VerdictKind } from '@/app/kit/policies'
import { layers, scopeLabel, type Layer } from './data'

function LayerBand({ layer, dimmed }: { layer: Layer; dimmed?: boolean }) {
  return (
    <div className={cn('flex border-b border-border last:border-b-0', dimmed && 'dimmed')}>
      <div className="w-44 shrink-0 border-r border-border bg-sunken/60 px-3 py-2.5">
        <div className="type-wide text-[13px] font-semibold">{scopeLabel[layer.scope]}</div>
        <div className="mt-0.5 text-[11px] leading-snug [overflow-wrap:anywhere] text-muted-foreground">{layer.origin}</div>
        {layer.server && !dimmed && <Tag mono={false} className="mt-1.5">nur lesend</Tag>}
      </div>
      <div className="min-w-0 flex-1">
        {dimmed ? (
          <div className="px-3 py-3 text-[12px] text-muted-foreground">
            Nur im Team-Betrieb. Ohne Team-Server gibt es diese Ebene nicht – lokal gelten User, Projekt und Agent.
          </div>
        ) : (
          layer.sets.map((set) => (
            <div key={set.name} className="border-b border-border/60 last:border-b-0">
              <div className="flex items-center gap-2 px-3 pt-2 pb-1">
                <FileCode2 className="size-3.5 text-muted-foreground" />
                <span className="text-[13px] font-medium">{set.name}</span>
                <span className="truncate font-mono text-[11px] text-muted-foreground">{set.file}</span>
                {set.shadow && <Tag mono={false}>Shadow-Modus</Tag>}
                <span className="ml-auto font-mono text-[11px] whitespace-nowrap text-muted-foreground">{set.version}</span>
              </div>
              <div className="pb-1.5">
                {set.rules.map((r) => (
                  <div key={r.id} className="grid grid-cols-[minmax(0,180px)_124px_minmax(0,1fr)_110px_44px] items-center gap-3 px-3 py-1 text-[12px] hover:bg-accent/50">
                    <span className="truncate font-mono">{r.id}</span>
                    <span className="flex items-center gap-1">
                      {r.type ? <Tag>{r.type}</Tag> : <Tag mono={false}>CEL</Tag>}
                      {r.milestone && <span className="text-[10px] text-muted-foreground">ab {r.milestone}</span>}
                    </span>
                    <span className="min-w-0 truncate text-muted-foreground" title={r.summary}>
                      <span className="mr-1.5 font-mono text-[11px] text-foreground/70">{r.phase}</span>
                      {r.summary}
                    </span>
                    <Verdict v={r.verdict} />
                    <span className="text-right text-[11px] text-muted-foreground tabular-nums" title="Treffer heute">
                      {r.hits ?? '–'}
                    </span>
                  </div>
                ))}
              </div>
            </div>
          ))
        )}
      </div>
    </div>
  )
}

const COLS: { v: VerdictKind; label: string }[] = [
  { v: 'allow', label: 'erlaubt' },
  { v: 'ask', label: 'fragt' },
  { v: 'deny', label: 'abgelehnt' },
]

/** „Strengere gewinnt“: jede Ebene setzt ihren Punkt, das Ergebnis liegt ganz rechts. */
function Strictness({ team }: { team: boolean }) {
  const rows: { scope: string; v: VerdictKind | null; rule: string }[] = [
    { scope: 'Org', v: null, rule: team ? 'keine Regel greift' : 'nur im Team-Betrieb' },
    { scope: 'Team', v: null, rule: team ? 'keine Regel greift' : 'nur im Team-Betrieb' },
    { scope: 'User', v: 'ask', rule: 'git-guard: push → ask' },
    { scope: 'Projekt', v: 'ask', rule: 'confirm-destructive-shell' },
    { scope: 'Agent', v: 'allow', rule: 'pr-fixer: allow-push' },
  ]
  const idx = (v: VerdictKind | null) => (v === 'allow' ? 0 : v === 'ask' ? 1 : v === 'deny' ? 2 : -1)
  const result = Math.max(...rows.map((r) => idx(r.v)))
  return (
    <F id={['POL-007', 'POL-008']} className="space-y-2">
      <div className="text-[12px] text-muted-foreground">
        Beispiel: <code className="font-mono text-foreground">git push origin feature/rate-limit</code> im Agent „pr-fixer“
      </div>
      <div className="grid grid-cols-[112px_repeat(3,1fr)] text-[11px]">
        <span />
        {COLS.map((c) => (
          <span key={c.v} className="pb-1 text-center text-muted-foreground">
            {c.label}
          </span>
        ))}
        {rows.map((r) => (
          <div key={r.scope} className="contents">
            <span className="flex h-10 flex-col justify-center leading-tight">
              <span className="text-[12px] font-medium">{r.scope}</span>
              <span className="truncate font-mono text-[10px] text-muted-foreground">{r.rule}</span>
            </span>
            {COLS.map((c, i) => (
              <span key={c.v} className="relative flex h-10 items-center justify-center border-l border-border">
                <span className="absolute inset-x-0 top-1/2 h-px bg-border" />
                {idx(r.v) === i && (
                  <span className="relative z-10 bg-background px-1" title={r.rule}>
                    <Verdict v={c.v} label="" />
                  </span>
                )}
              </span>
            ))}
          </div>
        ))}
        <span className="mt-1 flex h-9 items-center border-t-2 border-foreground text-[12px] font-semibold">Ergebnis</span>
        {COLS.map((c, i) => (
          <span key={c.v} className="mt-1 flex h-9 items-center justify-center border-t-2 border-l border-foreground border-l-border">
            {result === i && <Verdict v={c.v} className="font-semibold" />}
          </span>
        ))}
      </div>
      <ul className="space-y-1 text-[12px] text-muted-foreground">
        <li>Die strengste Entscheidung aller Ebenen gilt: abgelehnt vor fragt vor erlaubt.</li>
        <li>Ein „allow“ im Agent hebt kein „ask“ oder „deny“ aus Projekt oder User auf – tiefere Ebenen können nur verschärfen.</li>
        <li>Ein Default einer Ebene (z. B. „browser_navigate: deny“) wird nur durch ein „allow“ derselben Ebene aufgehoben.</li>
        {team && <li>Org und Team kommen vom Team-Server und können lokal nicht entfernt oder überschrieben werden.</li>}
      </ul>
    </F>
  )
}

export function PolicyOverview({ state }: { state: string }) {
  const team = state === 'team'
  return (
    <Shell nav="policies" connection={team ? 'server' : 'local'}>
      <PageHead
        title="Policies"
        sub="Regeln für alle Sessions auf diesem Rechner, nach Ebenen. Die strengere Entscheidung gewinnt."
        actions={
          <>
            <Btn>
              Effektiv für: Rate-Limiter für die Login-API <ChevronDown className="size-3.5" />
            </Btn>
            <PrimaryButton>
              <Plus className="size-3.5" /> Policy anlegen
            </PrimaryButton>
          </>
        }
      >
        <div className="mt-2 flex flex-wrap gap-x-5 gap-y-1">
          {team ? (
            <Premise kind="team">Org und Team vom Team-Server (Bundle v41, signiert) · lokal zwischengespeichert, gilt auch offline</Premise>
          ) : (
            <Premise>Lokal: ~/.beton/policies, .beton/policies im Repo, agent.yaml · kein Server nötig</Premise>
          )}
          <F id="POL-001" as="span">
            <span className="inline-flex items-center gap-1.5 text-[12px] text-muted-foreground">
              <RefreshCw className="size-3.5" /> Dateien werden beobachtet · zuletzt neu geladen 14:02:11
            </span>
          </F>
        </div>
      </PageHead>
      <Scroll>
        <div className="flex gap-6 px-6 py-4">
          <div className="min-w-0 flex-1 space-y-3">
            {state === 'reload-error' && (
              <F id="POL-001">
                <Callout
                  tone="deny"
                  title="Änderung an git-and-budget.yaml nicht übernommen"
                  actions={
                    <>
                      <Btn>Im Editor öffnen</Btn>
                      <Btn tone="quiet">Ausblenden</Btn>
                    </>
                  }
                >
                  <p>
                    Zeile 20, Spalte 11: <code className="font-mono">session.cost_usd &gt; "5"</code> vergleicht eine Zahl mit
                    einem Text. Die letzte gültige Version (sha256:9c1e…7a) bleibt aktiv – laufende Sessions sind nicht betroffen.
                  </p>
                </Callout>
              </F>
            )}
            <F
              id={['POL-006', 'POL-011', 'POL-012', 'POL-013', 'POL-014', 'POL-015', 'POL-016', 'POL-017', 'POL-018', 'POL-019', 'POL-020', 'POL-028']}
              className="overflow-hidden rounded-md border border-border bg-card"
            >
              <div className="flex border-b border-border bg-sunken py-1.5 text-[11px] text-muted-foreground">
                <span className="w-44 shrink-0 px-3">Ebene</span>
                <div className="grid flex-1 grid-cols-[minmax(0,180px)_124px_minmax(0,1fr)_110px_44px] gap-3 px-3">
                  <span>Regel</span>
                  <span>Typ</span>
                  <span>Phase · Inhalt</span>
                  <span>Aktion</span>
                  <span className="text-right">heute</span>
                </div>
              </div>
              {layers.map((l) => (
                <F key={l.scope} id={l.server ? ['POL-008', 'SYNC-006'] : 'POL-001'} badge="top-right">
                  <LayerBand layer={l} dimmed={!team && !!l.server} />
                </F>
              ))}
            </F>
            <p className="text-[12px] text-muted-foreground">
              Reihenfolge der Auswertung: Org → Team → User → Projekt → Agent; innerhalb einer Ebene Set-Name alphabetisch, dann
              Deklarationsreihenfolge. Rauten markieren Ablehnungen, Dreiecke Rückfragen, Kreise Erlaubnisse.
            </p>
          </div>
          <aside className="w-[300px] shrink-0">
            <h3 className="type-wide mb-2 text-[13px] font-semibold">Strengere gewinnt</h3>
            <Strictness team={team} />
          </aside>
        </div>
      </Scroll>
    </Shell>
  )
}
