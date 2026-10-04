import type { ReactNode } from 'react'
import { ArrowUp, ChevronDown, GitBranch, GitFork, Mic, Paperclip, Share2, Square } from 'lucide-react'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { harnesses, quotas, sampleContext, type ContextUsage, type HarnessId, type Quota } from '@/mock/data'
import { HarnessBadge, StatusMark } from './harness'
import type { SessionStatus } from '@/mock/data'

/** Kopfzeile einer Session: Titel, Harness, Branch, Kontext, Verbrauch, Aktionen. */
export function SessionHeader({
  title,
  harness,
  model,
  status,
  branch,
  extra,
  context = sampleContext,
  quota = quotas[harness],
}: {
  title: string
  harness: HarnessId
  model?: string
  status: SessionStatus
  branch?: string
  extra?: ReactNode
  /** Belegtes Kontextfenster; Standard: Beispielwerte. */
  context?: ContextUsage
  /** Subscription-Fenster; Standard: Kontingent des Harness aus den Beispieldaten, `null` blendet es aus. */
  quota?: Quota | null
}) {
  const ctxPct = Math.round((context.used / context.window) * 100)
  return (
    <div className="@container flex h-12 shrink-0 items-center gap-3 border-b border-border px-4">
      <StatusMark status={status} />
      <h2 className="min-w-[8rem] flex-1 truncate text-[15px] font-semibold" title={title}>{title}</h2>
      <HarnessBadge id={harness} className="shrink-0 whitespace-nowrap @3xl:hidden" />
      <HarnessBadge id={harness} model={model ?? harnesses[harness].models[0]} className="hidden shrink-0 whitespace-nowrap @3xl:inline-flex" />
      {branch && (
        <span className="hidden shrink-0 items-center gap-1 font-mono text-[11px] whitespace-nowrap text-muted-foreground @5xl:inline-flex">
          <GitBranch className="size-3" />
          {branch}
        </span>
      )}
      <div className="ml-auto flex min-w-0 shrink items-center gap-3 overflow-hidden whitespace-nowrap">
        {extra}
        <F id={['USE-008', 'HAR-021']} badge="bottom-left" className="hidden @2xl:block">
          <span className="flex items-center gap-1.5 text-[11px] text-muted-foreground" title={`Kontext: ${context.used.toLocaleString('de-DE')} von ${context.window.toLocaleString('de-DE')} Tokens`}>
            <span className="h-1.5 w-16 overflow-hidden rounded-full bg-muted">
              <span className="block h-full bg-foreground/70" style={{ width: `${ctxPct}%` }} />
            </span>
            {ctxPct} % Kontext
          </span>
        </F>
        {quota && (
          <F id="USE-004" badge="bottom-left" className="hidden @4xl:block">
            {quota.windowUsedPct === undefined ? (
              <span className="text-[11px] text-muted-foreground" title="Der Harness meldet kein Subscription-Fenster">
                {quota.label}: nicht gemeldet
              </span>
            ) : (
              <span className="text-[11px] text-muted-foreground" title={quota.resetsIn ? `Subscription-Fenster setzt in ${quota.resetsIn} zurück` : undefined}>
                {quota.label}: {quota.windowUsedPct} % genutzt
              </span>
            )}
          </F>
        )}
        <button className="inline-flex h-7 items-center gap-1 rounded-md border border-border px-2 text-xs hover:bg-accent">
          <GitFork className="size-3.5" /> Fork
        </button>
        <button className="inline-flex h-7 items-center gap-1 rounded-md border border-border px-2 text-xs hover:bg-accent">
          <Share2 className="size-3.5" /> Teilen
        </button>
      </div>
    </div>
  )
}

function Picker({ label, value }: { label: string; value: ReactNode }) {
  return (
    <button className="inline-flex h-7 items-center gap-1 rounded-md px-2 text-xs text-muted-foreground hover:bg-accent hover:text-foreground" aria-label={label}>
      {value}
      <ChevronDown className="size-3" />
    </button>
  )
}

/** Eingabe mit Pickern für Harness, Modell, Effort und Permission-Mode. */
export function Composer({
  harness = 'claude',
  running,
  placeholder = 'Nachricht an den Agent – @ für Dateien, / für Befehle',
  queued,
  draft,
}: {
  harness?: HarnessId
  running?: boolean
  placeholder?: string
  queued?: string[]
  draft?: string
}) {
  const h = harnesses[harness]
  return (
    <F id={['WEB-004', 'CLI-002']} className="shrink-0 border-t border-border p-3" badge="top-right">
      {queued && queued.length > 0 && (
        <F id={['SES-004', 'WEB-005']} className="mb-2 flex flex-col gap-1">
          {queued.map((q, i) => (
            <div key={i} className="flex items-center gap-2 rounded-md border border-dashed border-border px-2 py-1 text-[12px]">
              <span className="text-muted-foreground">Eingereiht {i + 1}</span>
              <span className="truncate">{q}</span>
              <span className="ml-auto flex gap-1">
                <button className="rounded-sm px-1.5 text-[11px] hover:bg-accent">Jetzt lenken</button>
                <button className="rounded-sm px-1.5 text-[11px] hover:bg-accent">Bearbeiten</button>
                <button className="rounded-sm px-1.5 text-[11px] hover:bg-accent">Entfernen</button>
              </span>
            </div>
          ))}
        </F>
      )}
      <div className="rounded-md border border-input bg-card focus-within:outline-2 focus-within:outline-ring">
        <div className={cn('min-h-[52px] px-3 pt-2.5 text-[14px]', !draft && 'text-muted-foreground')}>{draft ?? placeholder}</div>
        <div className="flex items-center gap-0.5 px-1.5 pb-1.5">
          <button className="flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-accent" aria-label="Datei anhängen">
            <Paperclip className="size-4" />
          </button>
          <Picker label="Harness" value={<HarnessBadge id={harness} />} />
          <Picker label="Modell" value={<span className="font-mono">{h.models[0]}</span>} />
          <Picker label="Effort" value="Effort: hoch" />
          <Picker label="Permission-Mode" value="Fragen bei Schreibzugriff" />
          <div className="ml-auto flex items-center gap-1">
            <F id={['VOI-007', 'VOI-004']} as="span" badge="top-right">
              <button className="flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-accent" aria-label="Diktieren (gedrückt halten)">
                <Mic className="size-4" />
              </button>
            </F>
            {running ? (
              <F id="SES-005" as="span" badge="top-right">
                <button className="flex h-7 items-center gap-1 rounded-md border border-foreground/40 px-2 text-xs" aria-label="Unterbrechen">
                  <Square className="size-3 fill-current" /> Stopp
                </button>
              </F>
            ) : null}
            <button className="flex size-7 items-center justify-center rounded-md bg-foreground text-background" aria-label={running ? 'Einreihen' : 'Senden'}>
              <ArrowUp className="size-4" />
            </button>
          </div>
        </div>
      </div>
    </F>
  )
}

export type RailTab = 'files' | 'changes' | 'terminal' | 'browser' | 'agents' | 'pr' | 'side' | 'comments'
const RAIL_TABS: { id: RailTab; label: string }[] = [
  { id: 'files', label: 'Dateien' },
  { id: 'changes', label: 'Änderungen' },
  { id: 'terminal', label: 'Terminal' },
  { id: 'browser', label: 'Browser' },
  { id: 'agents', label: 'Agents' },
  { id: 'pr', label: 'PR' },
  { id: 'side', label: 'Side-Chats' },
  { id: 'comments', label: 'Kommentare' },
]

/** Rechte Workspace-Leiste mit Tabs. */
export function WorkspaceRail({ active, width = 'w-[420px]', children }: { active: RailTab; width?: string; children: ReactNode }) {
  return (
    <F id="WEB-008" className={cn('flex shrink-0 flex-col border-l border-border bg-background', width)} badge="top-right">
      <div role="tablist" className="flex h-12 shrink-0 items-end gap-0.5 overflow-x-auto border-b border-border px-2">
        {RAIL_TABS.map((t) => (
          <button
            key={t.id}
            role="tab"
            aria-selected={t.id === active}
            className={cn(
              '-mb-px border-b-2 px-2 pb-2 text-[12px] whitespace-nowrap',
              t.id === active ? 'border-foreground font-medium' : 'border-transparent text-muted-foreground hover:text-foreground',
            )}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div className="min-h-0 flex-1 overflow-auto">{children}</div>
    </F>
  )
}
