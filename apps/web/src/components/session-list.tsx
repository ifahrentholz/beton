import { useMemo, useRef, useState } from 'react'
import { Link } from '@tanstack/react-router'
import { useVirtualizer } from '@tanstack/react-virtual'
import { Archive, Plus, Search } from 'lucide-react'
import { ago } from '@/lib/format'
import { cn } from '@/lib/utils'
import { sortSessions, useSessions } from '@/store/sessions'
import { harnessName, listStatus, StatusMark, VoiceDot, voiceOf } from './harness'
import { useLayout } from './layout-state'

/** Linke Spalte (WEB-003): Sessions nach Aktivität, Suche, Archiv; virtualisiert. */
export function SessionList({ active, onNew }: { active?: string | undefined; onNew: () => void }) {
  const byId = useSessions((s) => s.byId)
  const loaded = useSessions((s) => s.loaded)
  const [query, setQuery] = useState('')
  const [archived, setArchived] = useState(false)
  const closeList = useLayout((s) => s.closeList)
  const list = useMemo(() => {
    const q = query.trim().toLowerCase()
    return sortSessions(Object.values(byId)).filter(
      (s) => s.archived === archived && (!q || s.title.toLowerCase().includes(q) || s.id.includes(q)),
    )
  }, [byId, query, archived])
  const scrollRef = useRef<HTMLDivElement>(null)
  const virtualizer = useVirtualizer({
    count: list.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 48,
    overscan: 12,
    getItemKey: (i) => list[i]?.id ?? i,
  })
  return (
    <nav aria-label="Sessions" className="flex h-full w-full flex-col bg-sidebar">
      <div className="flex items-center gap-1 p-2">
        <label className="flex h-8 flex-1 items-center gap-2 rounded-md border border-input bg-card px-2 text-xs text-muted-foreground">
          <Search className="size-3.5 shrink-0" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Sessions durchsuchen"
            aria-label="Sessions durchsuchen"
            className="min-w-0 flex-1 bg-transparent text-foreground outline-none placeholder:text-muted-foreground"
          />
        </label>
        <button onClick={onNew} title="Neue Session" aria-label="Neue Session" className="flex size-8 items-center justify-center rounded-md bg-foreground text-background">
          <Plus className="size-4" />
        </button>
      </div>
      <div ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto pb-2" data-testid="session-list">
        <div className="relative" style={{ height: virtualizer.getTotalSize() }}>
          {virtualizer.getVirtualItems().map((v) => {
            const s = list[v.index]
            if (!s) return null
            return (
              <Link
                key={v.key}
                to="/s/$sessionId"
                params={{ sessionId: s.id }}
                onClick={closeList}
                data-testid="session-row"
                data-session={s.id}
                className={cn(
                  'absolute left-0 flex h-12 w-full items-center gap-2 border-l-2 px-3',
                  s.id === active ? 'border-signal bg-accent' : 'border-transparent hover:bg-accent/60',
                )}
                style={{ transform: `translateY(${v.start}px)` }}
              >
                <StatusMark status={listStatus(s.status)} />
                <div className="min-w-0 flex-1">
                  <div className="truncate text-[13px]">{s.title || 'Neue Session'}</div>
                  <div className="flex items-center gap-1.5 overflow-hidden text-[11px] whitespace-nowrap text-muted-foreground">
                    <VoiceDot voice={voiceOf(s.harness)} className="size-1.5" />
                    <span>{harnessName(s.harness)}</span>
                    <span className="ml-auto shrink-0 pl-1">{ago(s.last_activity_at)}</span>
                  </div>
                </div>
              </Link>
            )
          })}
        </div>
        {loaded && list.length === 0 && (
          <p className="px-3 py-4 text-[12px] text-muted-foreground">
            {query ? 'Keine Session passt zur Suche.' : archived ? 'Keine archivierten Sessions.' : 'Noch keine Sessions. Starte eine mit dem Plus oder `beton run`.'}
          </p>
        )}
      </div>
      <button
        onClick={() => setArchived((v) => !v)}
        className="flex items-center gap-2 border-t border-border px-3 py-2 text-[12px] text-muted-foreground hover:text-foreground"
        aria-pressed={archived}
      >
        <Archive className="size-3.5" /> {archived ? 'Aktive Sessions' : 'Archiv'}
      </button>
    </nav>
  )
}
