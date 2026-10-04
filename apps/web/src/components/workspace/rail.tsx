import { useEffect, useMemo, useRef, useState, type CSSProperties } from 'react'
import type { Capabilities, ChangeScope, Event, SessionWorktree } from '@beton/sdk'
import { X } from 'lucide-react'
import { badgeCount, lastFsSeq, RAIL_MIN, TAB_WIDTH, visibleTabs, workspaceVersion, type RailTab, type WorkspaceSearch } from '@/lib/workspace'
import { cn } from '@/lib/utils'
import { initSeen, markSeen, setTab, useSessionWorkspace, useWorkspace } from '@/store/workspace'
import { AgentsPanel } from './agents'
import { ChangesPanel } from './changes'
import { FilesPanel } from './files'

/** Tab-Leiste der Rail (ohne Router- und Server-Abhängigkeiten, für Tests). */
export function RailTabs({
  tabs,
  active,
  badges,
  onSelect,
  onClose,
}: {
  tabs: { id: RailTab; label: string }[]
  active: RailTab
  badges: Partial<Record<RailTab, number>>
  onSelect: (t: RailTab) => void
  onClose?: () => void
}) {
  return (
    <div role="tablist" aria-label="Workspace" className="flex h-12 shrink-0 items-end gap-0.5 overflow-x-auto border-b border-border px-2">
      {tabs.map((t, i) => {
        const n = badges[t.id] ?? 0
        return (
          <button
            key={t.id}
            role="tab"
            aria-selected={t.id === active}
            title={`${t.label} (Alt+${i + 1})`}
            onClick={() => onSelect(t.id)}
            data-tab={t.id}
            className={cn(
              '-mb-px inline-flex items-center gap-1.5 border-b-2 px-2 pb-2 text-[12px] whitespace-nowrap',
              t.id === active ? 'border-foreground font-medium' : 'border-transparent text-muted-foreground hover:text-foreground',
            )}
          >
            {t.label}
            {n > 0 && (
              <span
                data-testid={`badge-${t.id}`}
                className="min-w-4 rounded-sm bg-foreground px-1 text-center text-[10px] leading-4 font-semibold text-background tabular-nums"
                aria-label={`${n} ${n === 1 ? 'Datei' : 'Dateien'} geändert`}
              >
                {n}
              </span>
            )}
          </button>
        )
      })}
      {onClose && (
        <button onClick={onClose} className="mb-2 ml-auto flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent lg:hidden" aria-label="Workspace schließen">
          <X className="size-4" />
        </button>
      )}
    </div>
  )
}

/**
 * Version des Workspace, beruhigt: Beim Nachladen des Logs (Replay) springt sie mehrfach;
 * Baum, Änderungen und Diffs sollen dann nur einmal neu laden.
 */
function useSettled(value: number, ms = 150): number {
  const [v, setV] = useState(value)
  useEffect(() => {
    const t = setTimeout(() => setV(value), ms)
    return () => clearTimeout(t)
  }, [value, ms])
  return v
}

/** Platz, der für Liste, Navigation und Chat mindestens bleiben soll. */
const KEEP = 48 + 256 + 420

/**
 * Rechte Workspace-Rail einer Session (WEB-008): Tabs Dateien, Änderungen und Agents (WEB-012), Change-Badge
 * bei `fs.changed` (AC1), nicht verfügbare Tabs ausgeblendet (AC2). Breite per Ziehen am
 * linken Rand, Tabs mit Alt+1 … wechselbar. Unter 1024 px als Vollbild.
 */
export function WorkspaceRail({
  sessionId,
  events,
  capabilities,
  worktree,
  readSeq,
  search,
  onAnchor,
  copyLink,
}: {
  sessionId: string
  events: readonly Event[] | undefined
  capabilities?: Capabilities | undefined
  worktree?: SessionWorktree | undefined
  /** Gelesen-Stand aus der Session-Liste; `undefined`, solange sie nicht geladen ist. */
  readSeq: number | undefined
  search: WorkspaceSearch
  onAnchor: (p: { scope: ChangeScope; turn?: string | undefined; file: string; line: string }) => void
  copyLink: (p: { scope: ChangeScope; turn?: string | undefined; file: string; line: string }) => Promise<void>
}) {
  const ws = useSessionWorkspace(sessionId)
  const railOpen = useWorkspace((s) => s.railOpen)
  const railSheet = useWorkspace((s) => s.railSheet)
  const railWidth = useWorkspace((s) => s.railWidth)
  const tabs = useMemo(() => visibleTabs({ capabilities, role: 'owner' }), [capabilities])
  const tab = tabs.some((t) => t.id === ws.tab) ? ws.tab : (tabs[0]?.id ?? 'changes')
  const fsSeq = lastFsSeq(events)
  const version = useSettled(workspaceVersion(events))

  // Badges: bis zum Gelesen-Stand gilt alles als gesehen; der aktive Tab ist immer gesehen.
  useEffect(() => {
    if (readSeq !== undefined) initSeen(sessionId, readSeq)
  }, [sessionId, readSeq])
  useEffect(() => {
    if (fsSeq > 0) markSeen(sessionId, tab, fsSeq)
  }, [sessionId, tab, fsSeq])
  const badges: Partial<Record<RailTab, number>> = {}
  for (const t of ['files', 'changes'] as const) {
    if (t !== tab && ws.seen[t] !== undefined) badges[t] = badgeCount(events, ws.seen[t]!)
  }

  // Geteilter Link öffnet die Änderungen.
  useEffect(() => {
    if (search.tab) setTab(sessionId, search.tab)
  }, [sessionId, search.tab])

  // Alt+1 … wechselt die Tabs.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.altKey || e.metaKey || e.ctrlKey) return
      const n = /^Digit(\d)$/.exec(e.code)?.[1]
      const t = n ? tabs[Number(n) - 1] : undefined
      if (!t) return
      e.preventDefault()
      setTab(sessionId, t.id)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [sessionId, tabs])

  const width = Math.max(RAIL_MIN, Math.min(railWidth ?? TAB_WIDTH[tab] ?? 420, typeof window === 'undefined' ? 9999 : window.innerWidth - KEEP))
  const drag = useRef<{ x: number; w: number } | undefined>(undefined)

  return (
    <aside
      aria-label="Workspace"
      data-testid="workspace-rail"
      style={{ '--rail-w': `${width}px` } as CSSProperties}
      className={cn(
        'relative min-h-0 shrink-0 flex-col border-l border-border bg-background',
        railSheet ? 'fixed inset-0 z-40 flex lg:static lg:z-auto' : 'hidden',
        railOpen ? 'lg:flex lg:w-[var(--rail-w)]' : 'lg:hidden',
      )}
    >
      <div
        role="separator"
        aria-orientation="vertical"
        aria-label="Breite der Workspace-Leiste"
        title="Ziehen ändert die Breite, Doppelklick setzt sie zurück"
        onDoubleClick={() => useWorkspace.getState().setRailWidth(null)}
        onPointerDown={(e) => {
          drag.current = { x: e.clientX, w: width }
          e.currentTarget.setPointerCapture(e.pointerId)
        }}
        onPointerMove={(e) => {
          if (!drag.current) return
          const w = Math.max(RAIL_MIN, Math.min(drag.current.w + drag.current.x - e.clientX, window.innerWidth - KEEP))
          useWorkspace.getState().setRailWidth(Math.round(w))
        }}
        onPointerUp={() => (drag.current = undefined)}
        className="absolute inset-y-0 -left-1 z-20 hidden w-2 cursor-col-resize hover:bg-border/60 lg:block"
      />
      <RailTabs tabs={tabs} active={tab} badges={badges} onSelect={(t) => setTab(sessionId, t)} onClose={() => useWorkspace.getState().setRailSheet(false)} />
      <div className="min-h-0 flex-1 overflow-hidden" role="tabpanel">
        {tab === 'files' && <FilesPanel sessionId={sessionId} version={version} worktree={worktree} />}
        {tab === 'agents' && <AgentsPanel sessionId={sessionId} events={events} />}
        {tab === 'changes' && (
          <ChangesPanel
            sessionId={sessionId}
            events={events}
            version={version}
            worktree={worktree}
            initial={{ scope: search.scope, turn: search.turn, anchor: search.file && search.line ? { file: search.file, line: search.line } : undefined }}
            onAnchor={onAnchor}
            copyLink={copyLink}
          />
        )}
      </div>
    </aside>
  )
}
